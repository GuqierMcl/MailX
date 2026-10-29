//! On-demand body work is independent of periodic header synchronization.
//! All short database operations acquire service before flights. Cancellation
//! removes the flight under those same locks, including for queued blocking work.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};

use tokio::{
    runtime::Handle,
    sync::{Semaphore, watch},
    task::AbortHandle,
};
use uuid::Uuid;

use crate::{
    AppError, ErrorCode,
    accounts::{AccountService, sync_plan::PlanError},
    imap::{self, ImapError},
    mail::{
        BodyFailureCode, EmailDetail,
        render::{self, MAX_MESSAGE_BYTES, SANITIZER_VERSION},
    },
    storage::bodies::{BlobStore, BodyIdentity, CachedBody},
};

type Outcome = Result<(), AppError>;
type Waiter = watch::Receiver<Option<Outcome>>;

#[derive(Clone)]
struct Token {
    id: Uuid,
    identity: BodyIdentity,
}

struct Flight {
    token: Token,
    cached: bool,
    result: watch::Sender<Option<Outcome>>,
    abort: AbortHandle,
}

#[derive(Default)]
struct Flights {
    messages: HashMap<String, Flight>,
    accounts: HashMap<String, Arc<Semaphore>>,
}

struct Shared {
    service: Weak<Mutex<AccountService>>,
    flights: Mutex<Flights>,
    #[cfg(test)]
    connector: Option<
        tokio::sync::mpsc::UnboundedSender<
            tokio::sync::oneshot::Sender<Result<imap::Session, ImapError>>,
        >,
    >,
}

pub(crate) struct BodyManager {
    shared: Arc<Shared>,
    runtime: Handle,
}

fn internal() -> AppError {
    AppError::new(ErrorCode::Internal)
}
fn stale() -> AppError {
    AppError::new(ErrorCode::NotFound)
}

impl BodyManager {
    pub(crate) fn new(service: &Arc<Mutex<AccountService>>, runtime: Handle) -> Self {
        Self {
            shared: Arc::new(Shared {
                service: Arc::downgrade(service),
                flights: Mutex::new(Flights::default()),
                #[cfg(test)]
                connector: None,
            }),
            runtime,
        }
    }

    pub(crate) async fn fetch(&self, id: String) -> Result<EmailDetail, AppError> {
        self.ensure(id.clone(), false).await?;
        // Upgrade before handing off, so a closing core counts this reference.
        let service = self.shared.service.upgrade().ok_or_else(internal)?;
        self.runtime
            .spawn_blocking(move || {
                let service = service.lock().map_err(|_| internal())?;
                service.get_email(&id)
            })
            .await
            .map_err(|_| internal())?
    }

    /// Protocol reads can rebuild an older sanitizer result but never start a
    /// missing message's network download merely because a URL was requested.
    pub(crate) async fn refresh(&self, id: String) -> Outcome {
        self.ensure(id, true).await
    }

    async fn ensure(&self, id: String, cached_only: bool) -> Outcome {
        let shared = self.shared.clone();
        let runtime = self.runtime.clone();
        let mut waiter = self
            .runtime
            .spawn_blocking(move || register(shared, runtime, &id, cached_only))
            .await
            .map_err(|_| internal())??;
        loop {
            let result = waiter.borrow_and_update().clone();
            if let Some(result) = result {
                result?;
                break;
            }
            waiter.changed().await.map_err(|_| internal())?;
        }
        Ok(())
    }

    /// Caller holds service after a successful account update. No new flight or
    /// blocking commit can race the reset, and failed account updates do not
    /// cancel a previously valid download.
    pub(crate) fn restart(
        &self,
        account_id: &str,
        service: &mut AccountService,
    ) -> Result<(), AppError> {
        let mut flights = self.shared.flights.lock().map_err(|_| internal())?;
        let ids: Vec<_> = flights
            .messages
            .iter()
            .filter(|(_, f)| f.token.identity.account_id == account_id)
            .map(|(id, _)| id.clone())
            .collect();
        let mut failure = None;
        for id in ids {
            let Some(flight) = flights.messages.remove(&id) else {
                continue;
            };
            flight.abort.abort();
            // The current identity may have a new revision. Reset only the
            // exact message registered by the removed old flight; a new flight
            // cannot begin while service and flights are held.
            match service.body_identity(&id) {
                Ok(identity) => {
                    if let Err(error) = service.body_reset(&identity) {
                        failure = Some(error);
                    }
                }
                Err(error) if error.code == ErrorCode::NotFound => {}
                Err(error) => failure = Some(error),
            }
            flight
                .result
                .send_replace(Some(Err(AppError::new(ErrorCode::Conflict))));
        }
        failure.map_or(Ok(()), Err)
    }

    /// Caller holds the service lock after deletion. No new flight can be
    /// registered, and already queued completions observe the missing account.
    pub(crate) fn retire(&self, account_id: &str) {
        let mut flights = self
            .shared
            .flights
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        flights.messages.retain(|_, flight| {
            if flight.token.identity.account_id != account_id {
                return true;
            }
            flight.abort.abort();
            flight.result.send_replace(Some(Err(stale())));
            false
        });
    }
}

impl BodyManager {
    /// Stops every body download; the core is closing. Returns the stopped
    /// tasks so the caller can wait for them to finish.
    pub(crate) fn shutdown(&self) -> Vec<AbortHandle> {
        let mut stopped = Vec::new();
        if let Ok(mut flights) = self.shared.flights.lock() {
            for (_, flight) in flights.messages.drain() {
                flight.abort.abort();
                flight.result.send_replace(Some(Err(internal())));
                stopped.push(flight.abort);
            }
        }
        stopped
    }
}

impl Drop for BodyManager {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn register(
    shared: Arc<Shared>,
    runtime: Handle,
    id: &str,
    cached_only: bool,
) -> Result<Waiter, AppError> {
    let service_arc = shared.service.upgrade().ok_or_else(internal)?;
    let mut service = service_arc.lock().map_err(|_| internal())?;
    let identity = service.body_identity(id)?;
    let mut flights = shared.flights.lock().map_err(|_| internal())?;
    if let Some(flight) = flights.messages.get(id)
        && flight.token.identity.same_message(&identity)
    {
        if cached_only && !flight.cached {
            return Err(stale());
        }
        return Ok(flight.result.subscribe());
    }
    if let Some(flight) = flights.messages.remove(id) {
        flight.abort.abort();
        flight.result.send_replace(Some(Err(stale())));
        service.body_reset(&identity)?;
    }
    let cached = service.body_cache(&identity)?;
    if cached_only && cached.is_none() {
        return Err(stale());
    }
    if cached
        .as_ref()
        .is_some_and(|body| body.sanitizer_version == SANITIZER_VERSION)
    {
        let (tx, rx) = watch::channel(Some(Ok(())));
        drop(tx);
        return Ok(rx);
    }
    if cached.is_some() && !service.body_rebuild(&identity)? {
        return Err(stale());
    }
    if cached.is_none() && !service.body_begin(&identity)? {
        // Only a previous cancelled/panicked flight can leave fetching without
        // a registered worker. Repair it under both locks before restarting.
        service.body_reset(&identity)?;
        if !service.body_begin(&identity)? {
            return Err(stale());
        }
    }
    let blobs = service.body_blobs();
    let gate = flights
        .accounts
        .entry(identity.account_id.clone())
        .or_insert_with(|| Arc::new(Semaphore::new(1)))
        .clone();
    let token = Token {
        id: Uuid::new_v4(),
        identity,
    };
    let (result, waiter) = watch::channel(None);
    let work_token = token.clone();
    let work_shared = shared.clone();
    let is_cached = cached.is_some();
    let job = runtime.spawn(async move {
        let completion = Completion {
            shared: work_shared.clone(),
            token: work_token.clone(),
        };
        let outcome = match run(work_shared.clone(), work_token.clone(), gate, blobs, cached).await
        {
            Err(error)
                if matches!(
                    error.code,
                    ErrorCode::StorageUnavailable
                        | ErrorCode::StorageIncompatible
                        | ErrorCode::Internal
                ) =>
            {
                // A failed transaction must not leave an apparently live job.
                // If even this write fails, propagate the storage error rather
                // than pretending that the failed state was persisted.
                match fail(&work_shared, &work_token, BodyFailureCode::Storage).await {
                    Ok(()) => Ok(()),
                    Err(_) => Err(error),
                }
            }
            outcome => outcome,
        };
        complete(&work_shared, &work_token, outcome);
        drop(completion);
    });
    flights.messages.insert(
        id.to_owned(),
        Flight {
            token,
            cached: is_cached,
            result,
            abort: job.abort_handle(),
        },
    );
    Ok(waiter)
}

async fn store<T: Send + 'static>(
    shared: &Arc<Shared>,
    token: &Token,
    action: impl FnOnce(&mut AccountService, &BodyIdentity) -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    // Upgrade before handing off, so a closing core counts this reference.
    let service = shared.service.upgrade().ok_or_else(internal)?;
    let shared = shared.clone();
    let token = token.clone();
    tokio::task::spawn_blocking(move || {
        let mut service = service.lock().map_err(|_| internal())?;
        let flights = shared.flights.lock().map_err(|_| internal())?;
        if flights
            .messages
            .get(&token.identity.message_id)
            .is_none_or(|flight| flight.token.id != token.id)
            || !service.body_current(&token.identity)?
        {
            return Err(stale());
        }
        action(&mut service, &token.identity)
    })
    .await
    .map_err(|_| internal())?
}

async fn fail(shared: &Arc<Shared>, token: &Token, code: BodyFailureCode) -> Outcome {
    store(shared, token, move |service, identity| {
        if service.body_fail(identity, code)? {
            Ok(())
        } else {
            Err(stale())
        }
    })
    .await
}

async fn run(
    shared: Arc<Shared>,
    token: Token,
    gate: Arc<Semaphore>,
    blobs: BlobStore,
    cached: Option<CachedBody>,
) -> Outcome {
    let _permit = gate.acquire_owned().await.map_err(|_| internal())?;
    store(&shared, &token, |_, _| Ok(())).await?;
    if let Some(cached) = cached {
        let message_id = token.identity.message_id.clone();
        let updated = tokio::task::spawn_blocking(move || {
            let raw = blobs
                .read(&cached.raw)
                .map_err(|_| BodyFailureCode::Storage)?;
            let rendered =
                render::render(&raw, &message_id).map_err(|_| BodyFailureCode::Protocol)?;
            Ok::<_, BodyFailureCode>((cached.raw, rendered))
        })
        .await
        .map_err(|_| internal())?;
        let updated = match updated {
            Ok(updated) => updated,
            Err(code) => return fail(&shared, &token, code).await,
        };
        return store(&shared, &token, move |service, identity| {
            if service.body_commit(identity, &updated.0, &updated.1)? {
                Ok(())
            } else {
                Err(stale())
            }
        })
        .await;
    }
    if token
        .identity
        .size
        .is_some_and(|size| size > MAX_MESSAGE_BYTES as u64)
    {
        return fail(&shared, &token, BodyFailureCode::TooLarge).await;
    }
    let plan = store(&shared, &token, |service, identity| {
        Ok(service.sync_plan(&identity.account_id))
    })
    .await?;
    let plan = match plan {
        Ok(plan) => plan,
        Err(error) => return fail(&shared, &token, plan_failure(error)).await,
    };
    let fetched = async {
        let mut session = connect(&shared, &plan).await?;
        let mailbox = session.examine(&token.identity.remote_name).await?;
        if mailbox.uid_validity != token.identity.uid_validity {
            return Ok(None);
        }
        session
            .fetch_message(token.identity.uid, token.identity.size)
            .await
    }
    .await;
    let raw = match fetched {
        Ok(Some(raw)) => raw,
        Ok(None) => return fail(&shared, &token, BodyFailureCode::Stale).await,
        Err(error) => return fail(&shared, &token, imap_failure(error)).await,
    };
    let message_id = token.identity.message_id.clone();
    let prepared = tokio::task::spawn_blocking(move || {
        let rendered = render::render(&raw, &message_id).map_err(|error| match error {
            render::RenderError::TooLarge => BodyFailureCode::TooLarge,
            _ => BodyFailureCode::Protocol,
        })?;
        let blob = blobs.put(&raw).map_err(|_| BodyFailureCode::Storage)?;
        Ok::<_, BodyFailureCode>((blob, rendered))
    })
    .await
    .map_err(|_| internal())?;
    match prepared {
        Ok((blob, rendered)) => {
            store(&shared, &token, move |service, identity| {
                if service.body_commit(identity, &blob, &rendered)? {
                    Ok(())
                } else {
                    Err(stale())
                }
            })
            .await
        }
        Err(code) => fail(&shared, &token, code).await,
    }
}

async fn connect(
    shared: &Shared,
    plan: &crate::accounts::sync_plan::SyncPlan,
) -> Result<imap::Session, ImapError> {
    #[cfg(test)]
    if let Some(connector) = &shared.connector {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        connector
            .send(sender)
            .map_err(|_| ImapError::Protocol("test connector stopped".into()))?;
        return receiver
            .await
            .map_err(|_| ImapError::Protocol("test connector stopped".into()))?;
    }
    let _ = shared;
    imap::open_body_session(&plan.route, &plan.login).await
}

fn complete(shared: &Shared, token: &Token, result: Outcome) {
    if let Ok(mut flights) = shared.flights.lock()
        && flights
            .messages
            .get(&token.identity.message_id)
            .is_some_and(|flight| flight.token.id == token.id)
        && let Some(flight) = flights.messages.remove(&token.identity.message_id)
    {
        flight.result.send_replace(Some(result));
    }
}

/// Also settles waiters if a worker panics. On explicit cancellation restart has
/// already removed the token, so this cannot finish or reset a replacement task.
struct Completion {
    shared: Arc<Shared>,
    token: Token,
}

impl Drop for Completion {
    fn drop(&mut self) {
        complete(&self.shared, &self.token, Err(internal()));
    }
}

fn plan_failure(error: PlanError) -> BodyFailureCode {
    match error {
        PlanError::CredentialMissing | PlanError::CredentialUntrusted => {
            BodyFailureCode::Authentication
        }
        PlanError::Account(error) => match error.code {
            ErrorCode::SecretStoreLocked
            | ErrorCode::SecretStoreUnavailable
            | ErrorCode::SecretMissing => BodyFailureCode::Authentication,
            ErrorCode::NotFound | ErrorCode::Conflict => BodyFailureCode::Stale,
            _ => BodyFailureCode::Storage,
        },
    }
}

fn imap_failure(error: ImapError) -> BodyFailureCode {
    match error {
        ImapError::TooLarge => BodyFailureCode::TooLarge,
        ImapError::Authentication(_) => BodyFailureCode::Authentication,
        ImapError::ProviderPolicy => BodyFailureCode::ProviderPolicy,
        ImapError::Net(_) | ImapError::Timeout => BodyFailureCode::Network,
        _ => BodyFailureCode::Protocol,
    }
}

#[cfg(test)]
pub(crate) mod tests;
