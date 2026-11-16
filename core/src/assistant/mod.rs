//! The built-in mail assistant: runs (a Now summary, or one chat message)
//! that talk to the user's model and, for chats, use the same tool layer as
//! local MCP. Design: `docs/ai-runtime/README.md`; permissions:
//! `docs/architecture/ai-permissions.md`.
//!
//! Hosts construct an [`Assistant`] next to the core and forward its
//! [`CoreEvent::Assistant`](crate::CoreEvent) events. Everything is in memory:
//! runs, conversations and cached summaries vanish with the process.
//!
//! **Consent** is enforced here. A run only starts when
//! `assistant.consented` equals the binding (connection id and normalized base
//! URL) of the model the assistant would use now. Otherwise [`Assistant::start`]
//! returns a run in `awaitingConsent` that already ended and sent nothing; the
//! interface asks the user, calls [`Assistant::grant_consent`] and starts
//! again. Changing the connection or its address makes the comparison fail
//! with no extra bookkeeping, and the loop re-checks before every model
//! request, so a revocation also stops runs that are already going.
mod agent;
mod confirm;
mod conversations;
mod policy;
mod prompt;
mod runs;
mod summary;
#[cfg(test)]
mod tests;
mod types;

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use crate::{
    AppError, ErrorCode, MailOverviewQuery, MailxCore, SupportedLocale,
    ai::AiModelTarget,
    app::AiService,
    events::Events,
    settings::{AiConsent, AssistantSettings, SettingsStore},
    tools::Tools,
};
use conversations::Conversations;
use runs::Runs;
use summary::SummaryCache;
pub use types::{
    AssistantEvent, AssistantEventKind, ConfirmAction, ConfirmAttachment, ConfirmDraftChange,
    ConfirmWarning, ConfirmationDetails, ConfirmationRequest, FinishedReason, FlagRestore,
    MAX_MESSAGE_CHARS, OrganizeChange, PendingConfirmation, RunKind, RunKindTag, RunSnapshot,
    RunState, StartRun, ToolActivity, ToolActivitySummary, ToolStatus, ToolUndo,
};

/// The built-in assistant. Cheap to clone; clones share their runs.
#[derive(Clone)]
pub struct Assistant {
    inner: Arc<Inner>,
}

pub(crate) struct Inner {
    core: MailxCore,
    ai: Arc<AiService>,
    settings: Arc<SettingsStore>,
    tools: Arc<Tools>,
    runs: Runs,
    conversations: Arc<Conversations>,
    summaries: SummaryCache,
    confirmation_timeout_ms: AtomicU64,
}

/// What a run needs, resolved from the settings at one moment.
pub(crate) struct Ready {
    pub settings: AssistantSettings,
    pub target: AiModelTarget,
    pub locale: SupportedLocale,
}

pub(crate) enum Gate {
    Ready(Box<Ready>),
    NeedsConsent,
}

impl Inner {
    pub(crate) fn confirmation_timeout(&self) -> Duration {
        Duration::from_millis(self.confirmation_timeout_ms.load(Ordering::Relaxed))
    }

    /// Checks the switch, finds the model and compares the consent. Reads only
    /// settings until the consent matches.
    pub(crate) async fn gate(&self) -> Result<Gate, AppError> {
        let snapshot = self.settings.get()?;
        let settings = snapshot.settings.assistant;
        if !settings.enabled {
            return Err(AppError::new(ErrorCode::AssistantDisabled));
        }
        let binding = self
            .ai
            .assistant_binding()
            .await?
            .ok_or_else(|| AppError::new(ErrorCode::AiNotConfigured))?;
        if settings.consented.as_ref() != Some(&binding) {
            return Ok(Gate::NeedsConsent);
        }
        let target = self.ai.assistant_target().await?;
        Ok(Gate::Ready(Box::new(Ready {
            settings,
            target,
            locale: snapshot.effective_locale,
        })))
    }
}

impl Assistant {
    /// `tools` is the same registry local MCP uses; the assistant sees the
    /// tools visible to `Audience::BuiltinAgent`.
    pub fn new(
        core: MailxCore,
        ai: Arc<AiService>,
        settings: Arc<SettingsStore>,
        tools: Tools,
        events: Events,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                core,
                ai,
                settings,
                tools: Arc::new(tools),
                runs: Runs::new(events),
                conversations: Arc::new(Conversations::default()),
                summaries: SummaryCache::default(),
                confirmation_timeout_ms: AtomicU64::new(
                    u64::try_from(agent::CONFIRMATION_TIMEOUT.as_millis()).unwrap_or(u64::MAX),
                ),
            }),
        }
    }

    /// Starts a run and returns its snapshot at that moment; later changes
    /// arrive as events (and as newer snapshots).
    ///
    /// Errors: `AssistantDisabled`, `AiNotConfigured`, `AssistantBusy` (two
    /// runs are active, or the conversation already has one), `NotFound` (an
    /// unknown conversation) and `Validation` (the request itself). A missing
    /// consent is not an error: the snapshot is `awaitingConsent`. A Now
    /// summary found in the cache is returned as `completed` at once, unless
    /// the request is `fresh`.
    pub async fn start(&self, request: StartRun) -> Result<RunSnapshot, AppError> {
        let inner = &self.inner;
        let message = match &request.kind {
            RunKind::Chat { message, .. } => {
                let message = message.trim();
                if message.is_empty() || message.chars().count() > MAX_MESSAGE_CHARS {
                    return Err(AppError::validation("message"));
                }
                Some(message.to_owned())
            }
            RunKind::NowSummary { .. } => None,
        };
        let tag = match request.kind {
            RunKind::NowSummary { .. } => RunKindTag::NowSummary,
            RunKind::Chat { .. } => RunKindTag::Chat,
        };
        let ready = match inner.gate().await? {
            Gate::Ready(ready) => ready,
            Gate::NeedsConsent => {
                let conversation_id = match &request.kind {
                    RunKind::Chat {
                        conversation_id, ..
                    } => conversation_id.clone(),
                    RunKind::NowSummary { .. } => None,
                };
                let id = inner
                    .runs
                    .create(tag, conversation_id, RunState::AwaitingConsent)?;
                return inner.runs.snapshot(&id);
            }
        };
        match request.kind {
            RunKind::NowSummary {
                since,
                day_starts,
                fresh,
            } => self.start_summary(*ready, since, day_starts, fresh).await,
            RunKind::Chat {
                conversation_id, ..
            } => {
                let message = message.unwrap_or_default();
                let (conversation, history) = inner
                    .conversations
                    .begin(conversation_id.as_deref(), message)?;
                let run_id = match inner.runs.create(
                    RunKindTag::Chat,
                    Some(conversation.id.clone()),
                    RunState::Running,
                ) {
                    Ok(id) => id,
                    Err(error) => {
                        conversation.rollback();
                        return Err(error);
                    }
                };
                if ready.target.tool_call == Some(false) {
                    inner.runs.set_no_tools(&run_id);
                }
                let task = tokio::spawn(agent::chat(
                    inner.clone(),
                    run_id.clone(),
                    conversation,
                    history,
                ));
                inner.runs.set_abort(&run_id, task.abort_handle());
                inner.runs.snapshot(&run_id)
            }
        }
    }

    async fn start_summary(
        &self,
        ready: Ready,
        since: Option<String>,
        day_starts: Option<Vec<String>>,
        fresh: bool,
    ) -> Result<RunSnapshot, AppError> {
        let inner = &self.inner;
        let has_since = since.is_some();
        let query = MailOverviewQuery { since, day_starts };
        let core = inner.core.clone();
        let overview = tokio::task::spawn_blocking(move || core.mail_overview(&query))
            .await
            .map_err(|_| AppError::new(ErrorCode::Internal))??;
        let input = summary::build_input(&overview, &ready.settings.excluded_accounts, has_since);
        let key = SummaryCache::key(&input, &ready.target.model_id, ready.locale);
        // A refresh asks the model again; its answer replaces the cached one.
        if !fresh && let Some((text, completed_at)) = inner.summaries.get(&key) {
            return Ok(inner
                .runs
                .create_completed(RunKindTag::NowSummary, text, completed_at));
        }
        let run_id = inner
            .runs
            .create(RunKindTag::NowSummary, None, RunState::Running)?;
        let task = tokio::spawn(agent::summary(inner.clone(), run_id.clone(), input, key));
        inner.runs.set_abort(&run_id, task.abort_handle());
        inner.runs.snapshot(&run_id)
    }

    /// Answers the confirmation a run is waiting on: `approved` runs the
    /// tool call with its original arguments, otherwise the model is told the
    /// user declined. Only the run's current request can be answered, once.
    ///
    /// Errors: `NotFound` (unknown run), `Conflict` (the run is not waiting,
    /// waits on a different request, or the request was already answered,
    /// canceled or timed out).
    pub fn respond(&self, run_id: &str, request_id: &str, approved: bool) -> Result<(), AppError> {
        self.inner.runs.respond(run_id, request_id, approved)
    }

    /// Stops a run: its connection closes and it ends `canceled`. A run that
    /// already ended is left as it is. `NotFound` for an unknown id.
    pub fn cancel(&self, run_id: &str) -> Result<(), AppError> {
        self.inner.runs.cancel(run_id)
    }

    /// The run as it is now; finished runs stay for ten minutes (at most
    /// twenty). `NotFound` after that.
    pub fn snapshot(&self, run_id: &str) -> Result<RunSnapshot, AppError> {
        self.inner.runs.snapshot(run_id)
    }

    /// Records that the user agreed to send mail content to the connection of
    /// the model the assistant would use now, and returns that binding.
    pub async fn grant_consent(&self) -> Result<AiConsent, AppError> {
        let binding = self
            .inner
            .ai
            .assistant_binding()
            .await?
            .ok_or_else(|| AppError::new(ErrorCode::AiNotConfigured))?;
        let settings = self.inner.settings.clone();
        let saved = binding.clone();
        tokio::task::spawn_blocking(move || {
            settings.update_assistant(|assistant| {
                assistant.consented = Some(saved);
                Ok(())
            })
        })
        .await
        .map_err(|_| AppError::new(ErrorCode::Internal))??;
        Ok(binding)
    }

    /// Shortens the confirmation timeout for a test.
    #[cfg(test)]
    pub(crate) fn set_confirmation_timeout(&self, timeout: Duration) {
        self.inner.confirmation_timeout_ms.store(
            u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
    }

    /// Takes the consent back and stops every run that is going.
    pub fn revoke_consent(&self) -> Result<(), AppError> {
        self.inner.settings.update_assistant(|assistant| {
            assistant.consented = None;
            Ok(())
        })?;
        self.inner.runs.cancel_all();
        Ok(())
    }
}
