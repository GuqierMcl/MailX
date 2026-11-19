//! The built-in mail assistant: runs (a Now summary, or one chat message)
//! that talk to the user's model and, for chats, use the same tool layer as
//! local MCP. Design: `docs/ai-runtime/README.md`; permissions:
//! `docs/architecture/ai-permissions.md`.
//!
//! Hosts construct an [`Assistant`] next to the core and forward its
//! [`CoreEvent::Assistant`](crate::CoreEvent) events. Runs and cached summaries
//! are in memory and vanish with the process. Chat conversations are also
//! saved to local storage unless the user turned that off (`history`); a saved
//! one can be listed, shown, continued, and deleted.
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
mod history;
mod policy;
mod prompt;
mod runs;
mod summary;
#[cfg(test)]
mod tests;
mod types;
mod view;

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use crate::{
    AppError, CoreEvent, ErrorCode, MailOverviewQuery, MailxCore, SupportedLocale,
    accounts::AccountService,
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
    AssistantEvent, AssistantEventKind, ChatContext, ChatTurn, ChatTurnConfirmation,
    ChatTurnReasoning, ChatTurnTool, ConfirmAction, ConfirmAttachment, ConfirmDraftChange,
    ConfirmOutcome, ConfirmWarning, ConfirmationDetails, ConfirmationRequest, ConversationChange,
    ConversationDetail, ConversationList, ConversationSummary, ConversationsChanged,
    DEFAULT_CONVERSATION_PAGE, FinishedReason, FlagRestore, MAX_CONTEXT_EMAILS,
    MAX_CONVERSATION_PAGE, MAX_MESSAGE_CHARS, Occasion, OrganizeChange, PendingConfirmation,
    RunKind, RunKindTag, RunReasoning, RunSnapshot, RunState, StartRun, ToolActivity,
    ToolActivitySummary, ToolStatus, ToolUndo, TurnState,
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
    events: Events,
    /// Saving a conversation and deleting one never overlap, so a save in
    /// flight cannot bring a deleted conversation back.
    save_gate: tokio::sync::Mutex<()>,
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
    /// Runs `f` on a blocking thread with the account service (and so the
    /// local database).
    pub(crate) async fn with_store<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut AccountService) -> Result<T, AppError> + Send + 'static,
    ) -> Result<T, AppError> {
        let core = self.core.clone();
        tokio::task::spawn_blocking(move || f(&mut *core.accounts()?))
            .await
            .map_err(|_| AppError::new(ErrorCode::Internal))?
    }

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
                runs: Runs::new(events.clone()),
                conversations: Arc::new(Conversations::default()),
                summaries: SummaryCache::default(),
                confirmation_timeout_ms: AtomicU64::new(
                    u64::try_from(agent::CONFIRMATION_TIMEOUT.as_millis()).unwrap_or(u64::MAX),
                ),
                events,
                save_gate: tokio::sync::Mutex::new(()),
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
            RunKind::Chat {
                message, context, ..
            } => {
                let message = message.trim();
                if message.is_empty() || message.chars().count() > MAX_MESSAGE_CHARS {
                    return Err(AppError::validation("message"));
                }
                if context
                    .as_ref()
                    .is_some_and(|context| !view::valid(context))
                {
                    return Err(AppError::validation("context"));
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
                occasion,
            } => {
                self.start_summary(*ready, since, day_starts, fresh, occasion)
                    .await
            }
            RunKind::Chat {
                conversation_id,
                context,
                ..
            } => {
                let message = message.unwrap_or_default();
                if let Some(id) = conversation_id.as_deref() {
                    self.ensure_loaded(id).await?;
                }
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
                    context,
                ));
                inner.runs.set_abort(&run_id, task.abort_handle());
                inner.runs.snapshot(&run_id)
            }
        }
    }

    /// Makes sure a conversation the user continues is in memory: a saved one
    /// is read from storage; one that was removed there since (its account
    /// was deleted) is `NotFound`.
    async fn ensure_loaded(&self, id: &str) -> Result<(), AppError> {
        let inner = &self.inner;
        match inner.conversations.saved_state(id) {
            Some(false) => return Ok(()),
            Some(true) => {
                let owned = id.to_owned();
                if inner
                    .with_store(move |service| service.assistant_chat_exists(&owned))
                    .await?
                {
                    return Ok(());
                }
                inner.conversations.remove_idle(id);
                return Err(AppError::new(ErrorCode::NotFound));
            }
            None => {}
        }
        let owned = id.to_owned();
        let stored = inner
            .with_store(move |service| service.assistant_chat_get(&owned))
            .await?
            .ok_or_else(|| AppError::new(ErrorCode::NotFound))?;
        inner
            .conversations
            .insert_loaded(id, history::load(&stored.rows)?)
    }

    /// Saved conversations, newest first. `limit` is 1 to
    /// [`MAX_CONVERSATION_PAGE`] (default [`DEFAULT_CONVERSATION_PAGE`]);
    /// `cursor` is the previous page's `nextCursor`.
    ///
    /// Errors: `Validation` (`limit`, `cursor`).
    pub async fn list_conversations(
        &self,
        limit: Option<u32>,
        cursor: Option<String>,
    ) -> Result<ConversationList, AppError> {
        let limit = limit.unwrap_or(DEFAULT_CONVERSATION_PAGE);
        if limit == 0 || limit > MAX_CONVERSATION_PAGE {
            return Err(AppError::validation("limit"));
        }
        let after = cursor
            .as_deref()
            .map(|cursor| {
                cursor
                    .split_once('|')
                    .map(|(at, id)| (at.to_owned(), id.to_owned()))
                    .ok_or_else(|| AppError::validation("cursor"))
            })
            .transpose()?;
        let wanted = limit as usize;
        let mut found = self
            .inner
            .with_store(move |service| {
                service.assistant_chat_list(
                    wanted + 1,
                    after.as_ref().map(|(at, id)| (at.as_str(), id.as_str())),
                )
            })
            .await?;
        let more = found.len() > wanted;
        found.truncate(wanted);
        let next_cursor = more
            .then(|| {
                found
                    .last()
                    .map(|last| format!("{}|{}", last.updated_at, last.id))
            })
            .flatten();
        Ok(ConversationList {
            items: found
                .into_iter()
                .map(|stored| ConversationSummary {
                    id: stored.id,
                    title: stored.title,
                    updated_at: stored.updated_at,
                    message_count: stored.message_count,
                })
                .collect(),
            next_cursor,
        })
    }

    /// A saved conversation as turns the interface can show. `NotFound` for
    /// an unknown id.
    pub async fn get_conversation(&self, id: &str) -> Result<ConversationDetail, AppError> {
        let owned = id.to_owned();
        let stored = self
            .inner
            .with_store(move |service| service.assistant_chat_get(&owned))
            .await?
            .ok_or_else(|| AppError::new(ErrorCode::NotFound))?;
        Ok(ConversationDetail {
            turns: history::turns(&stored.rows),
            id: stored.id,
            title: stored.title,
            created_at: stored.created_at,
            updated_at: stored.updated_at,
        })
    }

    /// Deletes a conversation, saved and in memory. A run that is using it is
    /// canceled. `NotFound` for an unknown id.
    pub async fn delete_conversation(&self, id: &str) -> Result<(), AppError> {
        let inner = &self.inner;
        let _gate = inner.save_gate.lock().await;
        inner.runs.cancel_conversation(id);
        let in_memory = inner.conversations.remove(id);
        let owned = id.to_owned();
        let stored = inner
            .with_store(move |service| service.assistant_chat_delete(&owned))
            .await?;
        if !in_memory && !stored {
            return Err(AppError::new(ErrorCode::NotFound));
        }
        inner.events.emit(&CoreEvent::AssistantConversationsChanged(
            ConversationsChanged {
                conversation_id: Some(id.to_owned()),
                change: ConversationChange::Deleted,
            },
        ));
        Ok(())
    }

    /// Deletes every conversation, saved and in memory, and cancels the chat
    /// runs that are going.
    pub async fn clear_conversations(&self) -> Result<(), AppError> {
        let inner = &self.inner;
        let _gate = inner.save_gate.lock().await;
        inner.runs.cancel_chats();
        inner.conversations.clear();
        inner
            .with_store(|service| service.assistant_chat_clear())
            .await?;
        inner.events.emit(&CoreEvent::AssistantConversationsChanged(
            ConversationsChanged {
                conversation_id: None,
                change: ConversationChange::Cleared,
            },
        ));
        Ok(())
    }

    async fn start_summary(
        &self,
        ready: Ready,
        since: Option<String>,
        day_starts: Option<Vec<String>>,
        fresh: bool,
        occasion: Option<Occasion>,
    ) -> Result<RunSnapshot, AppError> {
        let inner = &self.inner;
        let query = MailOverviewQuery {
            since: since.clone(),
            day_starts,
        };
        let core = inner.core.clone();
        let overview = tokio::task::spawn_blocking(move || core.mail_overview(&query))
            .await
            .map_err(|_| AppError::new(ErrorCode::Internal))??;
        let input = summary::build_input(
            &overview,
            &ready.settings.excluded_accounts,
            since.as_deref(),
            occasion,
        );
        // The key is made from the facts alone: the lines shown lately are
        // added when the model is asked, and must not change what is cached.
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
