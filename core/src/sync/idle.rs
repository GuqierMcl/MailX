//! Per-account IMAP IDLE watcher.
//!
//! One long-lived connection idles on INBOX and, when the server announces a
//! change, wakes the account's regular sync worker. The watcher never touches
//! storage itself: the sync round does the real work, so a watcher that is
//! broken, unsupported or switched off only costs latency, never correctness.

use std::collections::VecDeque;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use tokio::sync::{Notify, watch};
use tokio::time::Instant;

use super::manager::{Shared, SyncError, WorkerToken, with_store};
use super::writeback::retry_delay;
use crate::accounts::sync_plan::PlanError;
use crate::error::ErrorCode;
use crate::imap::{self, IdleWake, ImapError, Session};

/// RFC 3501 reserves the literal name `INBOX` for the primary mailbox, so no
/// folder lookup is needed (and the watcher needs no storage access).
const INBOX: &str = "INBOX";

/// Servers may drop connections idle for 30 minutes (RFC 2177); re-entering
/// IDLE well inside that also refreshes NAT/proxy state. Measured by our own
/// clock, because server keepalives do not mean the path is alive.
const REIDLE_INTERVAL: Duration = Duration::from_secs(10 * 60);
/// A burst of pushes (a bulk delivery, a filter moving mail) becomes one wake.
const DEBOUNCE: Duration = Duration::from_millis(1500);
/// More change pushes than this per [`RATE_WINDOW`] means a misbehaving server
/// or a mailbox in flux; drop the connection and let polling cover it.
const RATE_LIMIT: usize = 60;
const RATE_WINDOW: Duration = Duration::from_secs(60);
/// A connection that survived this long counts as healthy, which resets the
/// backoff; otherwise a server that accepts and immediately drops us would be
/// retried at the shortest delay forever.
const STABLE_AFTER: Duration = Duration::from_secs(60);
/// Bound for the DONE + LOGOUT courtesy on shutdown or sleep recovery.
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(3);

/// How well IDLE currently covers the inbox. Only `Live` lengthens the sync
/// worker's poll interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum IdleHealth {
    #[default]
    Starting,
    Live,
    Reconnecting,
    /// The server does not advertise IDLE; not retried until the worker restarts.
    Unsupported,
    /// Refused by the provider or the credentials; not retried until restart.
    Disabled,
}

#[cfg(test)]
impl IdleHealth {
    /// A health receiver whose sender is gone, for tests that run the sync
    /// worker without a watcher.
    pub(crate) fn watch_closed() -> watch::Receiver<IdleHealth> {
        watch::channel(IdleHealth::Starting).1
    }
}

/// Signals from the manager into a running watcher.
#[derive(Default)]
pub(super) struct IdleControl {
    /// Leave cleanly and end the task.
    pub stop: Notify,
    /// Drop the connection and reconnect immediately (the machine slept).
    pub reconnect: Notify,
}

/// Why a connection attempt did not produce a session.
pub(super) enum ConnectFailure {
    Imap(ImapError),
    /// Worth another try after a delay (storage hiccup).
    Retry,
    /// IDLE cannot work for this account until the worker restarts.
    Disable,
    /// The worker generation is gone; end the task.
    Stop,
}

/// Everything the watcher loop needs besides the connection itself.
pub(super) struct WatchContext {
    pub health: Arc<watch::Sender<IdleHealth>>,
    pub sync_wake: Arc<Notify>,
    pub control: Arc<IdleControl>,
    pub timing: Timing,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Timing {
    pub reidle: Duration,
    pub debounce: Duration,
    pub stable_after: Duration,
    pub cleanup: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            reidle: REIDLE_INTERVAL,
            debounce: DEBOUNCE,
            stable_after: STABLE_AFTER,
            cleanup: CLEANUP_TIMEOUT,
        }
    }
}

impl WatchContext {
    fn set_health(&self, health: IdleHealth) {
        // `send_if_modified` so the sync worker is only woken by real changes.
        self.health.send_if_modified(|current| {
            let changed = *current != health;
            *current = health;
            changed
        });
    }
}

/// Production entry: resolves the plan and opens a session through the
/// account route on every attempt, so credential or route edits are picked up
/// and the proxy is never bypassed.
pub(super) async fn run(shared: Arc<Shared>, token: WorkerToken, context: WatchContext) {
    watch(|| connect(shared.clone(), token.clone()), context).await;
}

async fn connect(shared: Arc<Shared>, token: WorkerToken) -> Result<Session, ConnectFailure> {
    let id = token.account_id.clone();
    let plan = match with_store(&shared, &token, move |service| Ok(service.sync_plan(&id))).await {
        Ok(Ok(plan)) => plan,
        Ok(Err(PlanError::Account(error))) if error.code == ErrorCode::NotFound => {
            return Err(ConnectFailure::Stop);
        }
        Ok(Err(PlanError::Account(_))) => return Err(ConnectFailure::Retry),
        Ok(Err(PlanError::CredentialUntrusted | PlanError::CredentialMissing)) => {
            return Err(ConnectFailure::Disable);
        }
        Err(SyncError::Stale | SyncError::Closed) => return Err(ConnectFailure::Stop),
        Err(SyncError::Imap(error)) => return Err(ConnectFailure::Imap(error)),
        Err(SyncError::Storage(_)) => return Err(ConnectFailure::Retry),
    };
    if plan.mailbox_source_id != token.source {
        return Err(ConnectFailure::Stop);
    }
    imap::open_session(&plan.route, &plan.login)
        .await
        .map_err(ConnectFailure::Imap)
}

/// Why a single connection ended.
enum Ended {
    /// Manager asked us to stop.
    Stop,
    /// Sleep detected: reconnect right away.
    Reconnect,
    /// Lost or misbehaving connection: back off, then reconnect.
    Retry,
    /// IDLE is not available for this account for the rest of this run.
    Disabled,
    Unsupported,
}

/// The watcher loop. `connect` yields an authenticated session on every call.
pub(super) async fn watch<C, F>(mut connect: C, context: WatchContext)
where
    C: FnMut() -> F,
    F: Future<Output = Result<Session, ConnectFailure>>,
{
    let mut failures = 0u32;
    loop {
        let attempt = tokio::select! {
            biased;
            () = context.control.stop.notified() => return,
            attempt = connect() => attempt,
        };
        let session = match attempt {
            Ok(session) => session,
            Err(ConnectFailure::Stop) => return,
            Err(ConnectFailure::Disable) => {
                context.set_health(IdleHealth::Disabled);
                return;
            }
            Err(ConnectFailure::Imap(error)) if disables(&error) => {
                context.set_health(IdleHealth::Disabled);
                return;
            }
            Err(ConnectFailure::Imap(_) | ConnectFailure::Retry) => {
                if !backoff(&context, &mut failures).await {
                    return;
                }
                continue;
            }
        };
        let started = Instant::now();
        let ended = serve(session, &context).await;
        if started.elapsed() >= context.timing.stable_after {
            failures = 0;
        }
        match ended {
            Ended::Stop => return,
            Ended::Unsupported => {
                context.set_health(IdleHealth::Unsupported);
                return;
            }
            Ended::Disabled => {
                context.set_health(IdleHealth::Disabled);
                return;
            }
            Ended::Reconnect => context.set_health(IdleHealth::Reconnecting),
            Ended::Retry => {
                if !backoff(&context, &mut failures).await {
                    return;
                }
            }
        }
    }
}

/// Provider refusals and bad credentials will not fix themselves; hammering
/// the server would only risk the account. Polling carries on without IDLE.
fn disables(error: &ImapError) -> bool {
    matches!(
        error,
        ImapError::ProviderPolicy
            | ImapError::Authentication(_)
            | ImapError::SignInRequired
            | ImapError::Unsupported(_)
    )
}

/// Sleeps the next retry delay. Returns false when asked to stop. A sleep
/// signal cuts the wait short: the old delay was for a network that is gone.
async fn backoff(context: &WatchContext, failures: &mut u32) -> bool {
    context.set_health(IdleHealth::Reconnecting);
    let delay = retry_delay(*failures);
    *failures = failures.saturating_add(1);
    tokio::select! {
        biased;
        () = context.control.stop.notified() => false,
        () = context.control.reconnect.notified() => true,
        () = tokio::time::sleep(delay) => true,
    }
}

/// Runs one connection until it ends.
async fn serve(mut session: Session, context: &WatchContext) -> Ended {
    if !session.supports("IDLE") {
        let _ = bounded(context, session.logout()).await;
        return Ended::Unsupported;
    }
    if let Err(error) = session.examine(INBOX).await {
        return end_for(&error);
    }
    let mut idle = match session.start_idle().await {
        Ok(idle) => idle,
        Err(error) => return end_for(&error),
    };
    // A sleep signal that landed during connect refers to the old connection;
    // this one is fresh. `Timeout` polls the inner future first, so a stored
    // permit is consumed. The stop permit is deliberately left alone.
    let _ = tokio::time::timeout(Duration::ZERO, context.control.reconnect.notified()).await;
    context.set_health(IdleHealth::Live);
    // Catch up on whatever arrived before IDLE began or while we were away.
    context.sync_wake.notify_one();

    let mut deadline = Instant::now() + context.timing.reidle;
    let mut quiet_until: Option<Instant> = None;
    let mut events: VecDeque<Instant> = VecDeque::new();
    loop {
        let reconnecting = AtomicBool::new(false);
        let signal = async {
            tokio::select! {
                () = context.control.stop.notified() => {}
                () = context.control.reconnect.notified() => {
                    reconnecting.store(true, Ordering::Relaxed);
                }
            }
        };
        let until = quiet_until.map_or(deadline, |quiet| quiet.min(deadline));
        let wake = idle.wait(until, signal).await;
        match wake {
            Ok(IdleWake::Changed) => {
                let now = Instant::now();
                events.push_back(now);
                while events
                    .front()
                    .is_some_and(|first| now.duration_since(*first) > RATE_WINDOW)
                {
                    events.pop_front();
                }
                if events.len() > RATE_LIMIT {
                    leave(idle, context).await;
                    return Ended::Retry;
                }
                // Trailing edge: the window opens at the first event and is
                // not extended, so latency stays bounded during a long burst.
                quiet_until.get_or_insert(now + context.timing.debounce);
            }
            Ok(IdleWake::Deadline) => {
                let now = Instant::now();
                if quiet_until.is_some_and(|quiet| now >= quiet) {
                    quiet_until = None;
                    context.sync_wake.notify_one();
                }
                if now >= deadline {
                    if quiet_until.take().is_some() {
                        context.sync_wake.notify_one();
                    }
                    let Some(next) = re_idle(idle).await else {
                        return Ended::Retry;
                    };
                    idle = next;
                    deadline = Instant::now() + context.timing.reidle;
                }
            }
            Ok(IdleWake::Stopped) => {
                leave(idle, context).await;
                return if reconnecting.load(Ordering::Relaxed) {
                    Ended::Reconnect
                } else {
                    Ended::Stop
                };
            }
            Ok(IdleWake::Disconnected) => return Ended::Retry,
            Err(error) => return end_for(&error),
        }
    }
}

fn end_for(error: &ImapError) -> Ended {
    if disables(error) {
        Ended::Disabled
    } else {
        Ended::Retry
    }
}

/// DONE + new IDLE. `None` when the connection is no longer usable. A change
/// announced during the hand-over is kept on the new IDLE so the loop's next
/// wait reports it (and the debounce then wakes sync): mail landing exactly at
/// the re-entry must not wait for the next poll.
async fn re_idle(idle: imap::IdleSession) -> Option<imap::IdleSession> {
    let (session, changed) = idle.finish().await.ok()?;
    let mut next = session.start_idle().await.ok()?;
    if changed {
        next.note_change();
    }
    Some(next)
}

/// Best-effort DONE + LOGOUT within the cleanup timeout.
async fn leave(idle: imap::IdleSession, context: &WatchContext) {
    let _ = bounded(context, async {
        let (session, _) = idle.finish().await?;
        session.logout().await
    })
    .await;
}

async fn bounded<T>(
    context: &WatchContext,
    operation: impl Future<Output = Result<T, ImapError>>,
) -> Option<T> {
    tokio::time::timeout(context.timing.cleanup, operation)
        .await
        .ok()?
        .ok()
}

/// Detects that the machine slept by comparing wall-clock and monotonic
/// progress: monotonic clocks stand still while suspended on macOS and Linux,
/// the wall clock does not. (Windows' monotonic clock may keep counting, so
/// detection there is unverified; the ordinary reconnect path still applies.)
pub(super) struct SleepDetector {
    wall: SystemTime,
    mono: Instant,
}

/// A suspended machine looks like a jump of at least this much between ticks.
const SLEEP_JUMP: Duration = Duration::from_secs(60);
/// Tick spacing of the sleep ticker.
pub(super) const SLEEP_TICK: Duration = Duration::from_secs(30);

impl SleepDetector {
    pub(super) fn new(wall: SystemTime, mono: Instant) -> Self {
        Self { wall, mono }
    }

    /// Records a tick and reports whether the wall clock outran the monotonic
    /// one by more than [`SLEEP_JUMP`]. Wall clock moving backwards (manual
    /// change, NTP step) never counts.
    pub(super) fn check(&mut self, wall: SystemTime, mono: Instant) -> bool {
        let wall_elapsed = wall.duration_since(self.wall).unwrap_or_default();
        let mono_elapsed = mono.saturating_duration_since(self.mono);
        self.wall = wall;
        self.mono = mono;
        wall_elapsed > mono_elapsed + SLEEP_JUMP
    }
}

/// Ticks forever, calling `on_sleep` after every detected suspend. The clocks
/// are injected so tests can fake a jump.
pub(super) async fn sleep_ticker(
    mut wall: impl FnMut() -> SystemTime,
    mut mono: impl FnMut() -> Instant,
    on_sleep: impl Fn(),
) {
    let mut detector = SleepDetector::new(wall(), mono());
    loop {
        tokio::time::sleep(SLEEP_TICK).await;
        if detector.check(wall(), mono()) {
            on_sleep();
        }
    }
}

#[cfg(test)]
mod tests;
