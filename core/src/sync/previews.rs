//! Partial-fetch previews for rows a list is showing.
//!
//! Independent of body downloads: each account has its own single-session lane
//! and never touches the body manager's gate, so opening a message is never
//! queued behind previews. A request replaces what has not started yet (the
//! list has scrolled on); a running batch finishes. Transient failures leave a
//! row `missing`, only parsed-but-empty results are recorded as `failed`.
//! Service is always locked before the lane state.
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU64, Ordering},
    },
};

use tokio::{runtime::Handle, task::AbortHandle};

use crate::{
    AppError, ErrorCode,
    accounts::{AccountService, sync_plan::SyncPlan},
    events::{CoreEvent, EmailPreviewItem, EmailPreviews, Events},
    imap::{
        self, ImapError, PREVIEW_BATCH, PREVIEW_BATCH_BYTES, PREVIEW_FALLBACK_BYTES, PartialFetch,
    },
    mail::{
        PreviewState,
        preview::{PreviewText, extract},
    },
    storage::{bodies::BodyIdentity, previews::PreviewOutcome},
};

pub(crate) struct PreviewManager {
    shared: Arc<Shared>,
    runtime: Handle,
}

struct Shared {
    service: Weak<Mutex<AccountService>>,
    events: Events,
    sequence: AtomicU64,
    state: Mutex<State>,
    #[cfg(test)]
    connector: Option<
        tokio::sync::mpsc::UnboundedSender<
            tokio::sync::oneshot::Sender<Result<imap::Session, ImapError>>,
        >,
    >,
}

#[derive(Default)]
struct State {
    /// Newest request applied; older ones that lose the race are dropped.
    applied: u64,
    lanes: HashMap<String, Lane>,
}

#[derive(Default)]
struct Lane {
    wanted: VecDeque<BodyIdentity>,
    /// Present while this account's one preview task runs.
    worker: Option<AbortHandle>,
}

fn internal() -> AppError {
    AppError::new(ErrorCode::Internal)
}

impl PreviewManager {
    pub(crate) fn new(
        service: &Arc<Mutex<AccountService>>,
        runtime: Handle,
        events: Events,
    ) -> Self {
        Self {
            shared: Arc::new(Shared {
                service: Arc::downgrade(service),
                events,
                sequence: AtomicU64::new(0),
                state: Mutex::new(State::default()),
                #[cfg(test)]
                connector: None,
            }),
            runtime,
        }
    }

    /// `ids` is everything the list wants right now. Returns at once; results
    /// arrive as [`CoreEvent::Previews`]. Ids that already have a preview, or
    /// no longer exist, are dropped, and every account's not-yet-started work
    /// is replaced by what this request leaves for it.
    pub(crate) fn request(&self, ids: Vec<String>) {
        let sequence = self.shared.sequence.fetch_add(1, Ordering::SeqCst) + 1;
        // Upgrade before handing off, so a closing core counts this reference.
        let Some(service) = self.shared.service.upgrade() else {
            return;
        };
        let shared = self.shared.clone();
        let runtime = self.runtime.clone();
        self.runtime.spawn_blocking(move || {
            let _ = apply(&shared, &runtime, &service, sequence, &ids);
        });
    }

    /// Caller holds service after a successful account update: connection
    /// settings changed, so queued and running work belongs to the old ones.
    pub(crate) fn restart(&self, account_id: &str) {
        self.retire(account_id);
    }

    /// Caller holds the service lock after deletion.
    pub(crate) fn retire(&self, account_id: &str) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(lane) = state.lanes.remove(account_id)
            && let Some(worker) = lane.worker
        {
            worker.abort();
        }
    }

    /// Stops every lane; the core is closing. Returns the stopped tasks so the
    /// caller can wait for them to finish.
    pub(crate) fn shutdown(&self) -> Vec<AbortHandle> {
        let mut stopped = Vec::new();
        if let Ok(mut state) = self.shared.state.lock() {
            for (_, lane) in state.lanes.drain() {
                if let Some(worker) = lane.worker {
                    worker.abort();
                    stopped.push(worker);
                }
            }
        }
        stopped
    }
}

impl Drop for PreviewManager {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn apply(
    shared: &Arc<Shared>,
    runtime: &Handle,
    service: &Arc<Mutex<AccountService>>,
    sequence: u64,
    ids: &[String],
) -> Result<(), AppError> {
    let service = service.lock().map_err(|_| internal())?;
    let targets = service.preview_targets(ids)?;
    let mut state = shared.state.lock().map_err(|_| internal())?;
    if sequence <= state.applied {
        return Ok(());
    }
    state.applied = sequence;
    let mut grouped: HashMap<String, VecDeque<BodyIdentity>> = HashMap::new();
    for identity in targets {
        grouped
            .entry(identity.account_id.clone())
            .or_default()
            .push_back(identity);
    }
    for (account_id, lane) in &mut state.lanes {
        lane.wanted = grouped.remove(account_id).unwrap_or_default();
    }
    for (account_id, wanted) in grouped {
        state.lanes.entry(account_id).or_default().wanted = wanted;
    }
    let idle: Vec<String> = state
        .lanes
        .iter()
        .filter(|(_, lane)| lane.wanted.is_empty() && lane.worker.is_none())
        .map(|(account_id, _)| account_id.clone())
        .collect();
    for account_id in idle {
        state.lanes.remove(&account_id);
    }
    for (account_id, lane) in &mut state.lanes {
        if lane.worker.is_none() && !lane.wanted.is_empty() {
            let job = runtime.spawn(run(shared.clone(), account_id.clone()));
            lane.worker = Some(job.abort_handle());
        }
    }
    Ok(())
}

/// Whether the stored size says the message needs its own request: its text
/// may exceed the fallback cap on a server that ignores the partial range.
/// What the message adds to a batch's byte total.
fn batch_weight(identity: &BodyIdentity) -> u64 {
    identity.size.map_or(PREVIEW_FALLBACK_BYTES, |size| {
        size.min(PREVIEW_FALLBACK_BYTES)
    })
}

fn is_large(identity: &BodyIdentity) -> bool {
    identity
        .size
        .is_some_and(|size| size > PREVIEW_FALLBACK_BYTES)
}

/// Takes the next folder batch, or retires the worker in the same critical
/// section so a concurrent request cannot slip in between.
fn next_batch(shared: &Shared, account_id: &str) -> Option<Vec<BodyIdentity>> {
    let mut state = shared
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let lane = state.lanes.get_mut(account_id)?;
    let Some(first) = lane.wanted.pop_front() else {
        state.lanes.remove(account_id);
        return None;
    };
    let mut batch = vec![first];
    let mut rest = VecDeque::new();
    // A large message is fetched alone, and is never added to another batch,
    // so one that cannot be truncated server-side fails only itself.
    let solo = is_large(&batch[0]);
    // The first message is always taken; the rest must fit the byte cap.
    let mut bytes = batch_weight(&batch[0]);
    while let Some(identity) = lane.wanted.pop_front() {
        let same_folder = identity.folder_id == batch[0].folder_id
            && identity.uid_validity == batch[0].uid_validity;
        if !solo
            && same_folder
            && !is_large(&identity)
            && batch.len() < PREVIEW_BATCH
            && bytes + batch_weight(&identity) <= PREVIEW_BATCH_BYTES
        {
            bytes += batch_weight(&identity);
            batch.push(identity);
        } else {
            rest.push_back(identity);
        }
    }
    lane.wanted = rest;
    Some(batch)
}

/// Gives up on everything queued for the account after a transient failure,
/// so an unreachable server is not retried batch after batch. Returns the ids
/// that were still wanted and now got no attempt.
fn stop(shared: &Shared, account_id: &str) -> Vec<String> {
    let mut state = shared
        .state
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    state
        .lanes
        .remove(account_id)
        .map(|lane| {
            lane.wanted
                .into_iter()
                .map(|identity| identity.message_id)
                .collect()
        })
        .unwrap_or_default()
}

/// Tells the list these ids ended an attempt without a preview, so it can stop
/// showing them as loading. Nothing is stored; they may be requested again.
fn announce_missing(shared: &Shared, ids: impl IntoIterator<Item = String>) {
    let items: Vec<EmailPreviewItem> = ids
        .into_iter()
        .map(|email_id| EmailPreviewItem {
            email_id,
            preview: String::new(),
            preview_state: PreviewState::Missing,
        })
        .collect();
    if !items.is_empty() {
        shared
            .events
            .emit(&CoreEvent::Previews(EmailPreviews { items }));
    }
}

async fn run(shared: Arc<Shared>, account_id: String) {
    while let Some(batch) = next_batch(&shared, &account_id) {
        if process(&shared, &account_id, &batch).await.is_err() {
            let dropped = stop(&shared, &account_id);
            announce_missing(
                &shared,
                batch
                    .into_iter()
                    .map(|identity| identity.message_id)
                    .chain(dropped),
            );
            return;
        }
    }
}

async fn with_service<T: Send + 'static>(
    shared: &Arc<Shared>,
    action: impl FnOnce(&mut AccountService) -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    let service = shared.service.upgrade().ok_or_else(internal)?;
    tokio::task::spawn_blocking(move || {
        let mut service = service.lock().map_err(|_| internal())?;
        action(&mut service)
    })
    .await
    .map_err(|_| internal())?
}

/// One `UID FETCH` for `batch` (one folder) on a fresh session. An empty
/// result means the UID validity changed.
async fn fetch_batch(
    shared: &Shared,
    plan: &SyncPlan,
    batch: &[BodyIdentity],
) -> Result<Vec<(u32, PartialFetch)>, ImapError> {
    let uids: Vec<u32> = batch.iter().map(|identity| identity.uid).collect();
    let sizes: Vec<Option<u64>> = batch.iter().map(|identity| identity.size).collect();
    let mut session = connect(shared, plan).await?;
    let mailbox = session.examine(&batch[0].remote_name).await?;
    if mailbox.uid_validity != batch[0].uid_validity {
        return Ok(Vec::new());
    }
    session.fetch_previews(&uids, &sizes).await
}

/// `Err` means a transient problem: nothing was written, rows stay `missing`
/// and the caller announces them. Rows of the batch without a result (UID
/// validity changed, absent from the reply) are announced here.
async fn process(
    shared: &Arc<Shared>,
    account_id: &str,
    batch: &[BodyIdentity],
) -> Result<(), AppError> {
    let account = account_id.to_owned();
    let Ok(plan) = with_service(shared, move |service| Ok(service.sync_plan(&account))).await?
    else {
        // Disabled or credentials unavailable: not an account to connect to.
        return Err(internal());
    };
    let fetched = fetch_batch(shared, &plan, batch).await;
    let outcomes: Vec<PreviewOutcome> = match fetched {
        Ok(fetched) => fetched
            .into_iter()
            .filter_map(|(uid, part)| {
                let identity = batch.iter().find(|identity| identity.uid == uid)?;
                Some(PreviewOutcome {
                    identity: identity.clone(),
                    preview: preview_text(part),
                })
            })
            .collect(),
        // The reply outgrew the budget derived from the stored sizes: the
        // server ignored the partial range for something big. A lone message
        // is recorded as failed (more attempts cannot succeed). In a larger
        // batch the sizes were wrong, so which message is not known: each is
        // retried alone, and only the one that is too large fails.
        Err(ImapError::TooLarge) => {
            let mut outcomes = Vec::with_capacity(batch.len());
            for identity in batch {
                let alone = std::slice::from_ref(identity);
                let fetched = if batch.len() == 1 {
                    Err(ImapError::TooLarge)
                } else {
                    fetch_batch(shared, &plan, alone).await
                };
                let preview = match fetched {
                    Ok(fetched) => match fetched.into_iter().next() {
                        Some((_, part)) => preview_text(part),
                        // Gone, or the UID validity changed.
                        None => continue,
                    },
                    Err(ImapError::TooLarge) => None,
                    Err(_) => return Err(internal()),
                };
                outcomes.push(PreviewOutcome {
                    identity: identity.clone(),
                    preview,
                });
            }
            outcomes
        }
        Err(_) => return Err(internal()),
    };
    let written = if outcomes.is_empty() {
        Vec::new()
    } else {
        with_service(shared, move |service| service.preview_commit(&outcomes)).await?
    };
    let unresolved: Vec<String> = batch
        .iter()
        .filter(|identity| {
            !written
                .iter()
                .any(|row| row.message_id == identity.message_id)
        })
        .map(|identity| identity.message_id.clone())
        .collect();
    if !written.is_empty() {
        shared.events.emit(&CoreEvent::Previews(EmailPreviews {
            items: written
                .into_iter()
                .map(|written| EmailPreviewItem {
                    email_id: written.message_id,
                    preview: written.preview,
                    preview_state: written.state,
                })
                .collect(),
        }));
    }
    announce_missing(shared, unresolved);
    Ok(())
}

/// `None` is recorded as `failed`.
fn preview_text(part: PartialFetch) -> Option<String> {
    match part {
        PartialFetch::Data { header, text } => match extract(&header, &text) {
            PreviewText::Text(text) => Some(text),
            PreviewText::Empty => None,
        },
        PartialFetch::Oversized => None,
    }
}

async fn connect(shared: &Shared, plan: &SyncPlan) -> Result<imap::Session, ImapError> {
    #[cfg(test)]
    if let Some(connector) = &shared.connector {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        connector
            .send(sender)
            .map_err(|_| ImapError::Protocol("test connector stopped".into()))?;
        return receiver
            .await
            .map_err(|_| ImapError::Protocol("test connector dropped".into()))?;
    }
    let _ = shared;
    imap::open_body_session(&plan.route, &plan.login).await
}

#[cfg(test)]
mod tests;
