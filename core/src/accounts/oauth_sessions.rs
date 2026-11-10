//! Opaque, bounded authorization sessions. A session owns its target, route,
//! callback listener, PKCE state and, once the browser comes back, the signed-in
//! grant. Waiting for the browser is short; a signed-in grant is kept longer so
//! the user can type a mailbox address and name the account, then it is dropped.
use crate::{
    accounts::types::{CreateAccountRequest, RouteInput},
    auth::{
        IdentityVerifier, OAuthError, OAuthSession, ProviderRegistration, SignedIn,
        VerifiedAuthorization, loopback::LoopbackCallback,
    },
    net::NetRoute,
};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// Waiting for the user to finish in the browser.
const BROWSER_WAIT: Duration = Duration::from_secs(300);
/// After the browser returned: time to give an address and name the account.
const READY_LIFETIME: Duration = Duration::from_secs(900);
const MAX_SESSIONS: usize = 8;
/// Typed mailbox addresses tried per session before it is dropped.
pub(crate) const MAX_MAILBOX_ATTEMPTS: u8 = 5;

pub(crate) struct OAuthAuthorizationStart {
    pub session_id: String,
    pub authorization_url: String,
}
/// What the page learns after the browser returned or an address was tried.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SignInState {
    /// The reviewed mail server opened this mailbox with the new sign-in.
    Verified(String),
    /// No mailbox yet: the sign-in gave no usable address, or the mail server
    /// refused it. `suggestion` is the address that was refused, if any.
    NeedsMailbox(Option<String>),
}
/// Everything needed to create an account from a ready session. The session
/// stays until [`SessionCoordinator::created`] consumes it.
pub(crate) struct CreateTarget {
    pub route: RouteInput,
    pub registration: ProviderRegistration,
    pub authorization: VerifiedAuthorization,
}
pub(crate) struct CompletedReauthorization {
    pub registration: ProviderRegistration,
    pub authorization: VerifiedAuthorization,
    pub account_id: String,
    pub expected_revision: u32,
}

enum Kind {
    Create {
        /// The route the page chose; it also becomes the account's route.
        route: RouteInput,
    },
    Reauthorize {
        account_id: String,
        expected_revision: u32,
    },
}
struct Ready {
    signed: SignedIn,
    attempts: u8,
    /// A mailbox proof or an account creation is running for this session.
    busy: bool,
}
enum State {
    /// The browser has not come back; the task is taken while someone awaits it.
    Waiting(Option<tokio::task::JoinHandle<Result<SignedIn, OAuthError>>>),
    Ready(Box<Ready>),
}
struct Entry {
    kind: Kind,
    verifier: Arc<dyn IdentityVerifier>,
    route: Arc<NetRoute>,
    deadline: Mutex<Instant>,
    state: Mutex<State>,
    abort: tokio::task::AbortHandle,
    /// Held for as long as the session exists, so a signed-in session still
    /// counts against the limit.
    _permit: tokio::sync::OwnedSemaphorePermit,
}
impl Entry {
    fn deadline(&self) -> Instant {
        self.deadline
            .lock()
            .map(|deadline| *deadline)
            .unwrap_or_else(|_| Instant::now())
    }
    fn set_deadline(&self, deadline: Instant) {
        if let Ok(mut current) = self.deadline.lock() {
            *current = deadline;
        }
    }
}

pub(crate) struct SessionCoordinator {
    pending: Arc<Mutex<HashMap<String, Arc<Entry>>>>,
    capacity: Arc<tokio::sync::Semaphore>,
    closed: AtomicBool,
}
impl Default for SessionCoordinator {
    fn default() -> Self {
        Self {
            pending: Arc::default(),
            closed: AtomicBool::new(false),
            capacity: Arc::new(tokio::sync::Semaphore::new(MAX_SESSIONS)),
        }
    }
}
impl SessionCoordinator {
    /// A new account: the address is learned from the sign-in.
    pub(crate) async fn begin(
        &self,
        registration: ProviderRegistration,
        verifier: Arc<dyn IdentityVerifier>,
        route: NetRoute,
        route_input: RouteInput,
    ) -> Result<OAuthAuthorizationStart, OAuthError> {
        self.begin_target(
            registration,
            verifier,
            route,
            Kind::Create { route: route_input },
            None,
        )
        .await
    }
    /// An existing account: its address is known and the only one accepted.
    pub(crate) async fn begin_reauthorization(
        &self,
        registration: ProviderRegistration,
        verifier: Arc<dyn IdentityVerifier>,
        route: NetRoute,
        mut request: CreateAccountRequest,
        account_id: String,
        expected_revision: u32,
    ) -> Result<OAuthAuthorizationStart, OAuthError> {
        super::oauth::normalize_oauth_request(&mut request)
            .map_err(|_| OAuthError::InvalidRegistration)?;
        registration.validate_mail_target(
            &request.config.email_address,
            &request.config.imap,
            &request.config.smtp,
        )?;
        let mailbox = request.config.email_address.clone();
        self.begin_target(
            registration,
            verifier,
            route,
            Kind::Reauthorize {
                account_id,
                expected_revision,
            },
            Some(mailbox),
        )
        .await
    }
    async fn begin_target(
        &self,
        registration: ProviderRegistration,
        verifier: Arc<dyn IdentityVerifier>,
        route: NetRoute,
        kind: Kind,
        known_mailbox: Option<String>,
    ) -> Result<OAuthAuthorizationStart, OAuthError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(OAuthError::ExpiredSession);
        }
        // Reserve capacity before opening a listening socket. Expired sessions
        // are aborted even if their host never polls or cancels them.
        {
            let mut pending = self.pending.lock().map_err(|_| OAuthError::Storage)?;
            pending.retain(|_, entry| {
                if entry.deadline() <= Instant::now() {
                    entry.abort.abort();
                    false
                } else {
                    true
                }
            });
            if pending.len() >= MAX_SESSIONS {
                return Err(OAuthError::ExpiredSession);
            }
        }
        let permit = self
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| OAuthError::ExpiredSession)?;
        let callback = LoopbackCallback::bind(&registration.redirect_path).await?;
        // A known address is also the sign-in hint; a new account has none yet.
        let (session, url) = OAuthSession::begin(
            registration,
            callback.redirect_uri(),
            known_mailbox.as_deref(),
        )?;
        let session_id = uuid::Uuid::new_v4().to_string();
        let route = Arc::new(route);
        let task_route = route.clone();
        let task_verifier = verifier.clone();
        let task = tokio::spawn(async move {
            tokio::time::timeout(BROWSER_WAIT, async move {
                let callback = callback.receive().await?;
                session
                    .complete(
                        &callback,
                        &task_route,
                        task_verifier.as_ref(),
                        known_mailbox.as_deref(),
                    )
                    .await
            })
            .await
            .map_err(|_| OAuthError::ExpiredSession)?
        });
        let abort = task.abort_handle();
        let entry = Arc::new(Entry {
            kind,
            verifier,
            route,
            deadline: Mutex::new(Instant::now() + BROWSER_WAIT),
            state: Mutex::new(State::Waiting(Some(task))),
            abort,
            _permit: permit,
        });
        let mut pending = self.pending.lock().map_err(|_| {
            entry.abort.abort();
            OAuthError::Storage
        })?;
        if pending.len() >= MAX_SESSIONS || self.closed.load(Ordering::SeqCst) {
            entry.abort.abort();
            return Err(OAuthError::ExpiredSession);
        }
        pending.insert(session_id.clone(), entry.clone());
        drop(pending);
        self.schedule_expiry(session_id.clone(), &entry);
        Ok(OAuthAuthorizationStart {
            session_id,
            authorization_url: url.to_string(),
        })
    }
    /// Removes the session once its (possibly extended) deadline has passed,
    /// without anyone having to poll for it.
    fn schedule_expiry(&self, session_id: String, entry: &Arc<Entry>) {
        let pending = Arc::downgrade(&self.pending);
        let entry: Weak<Entry> = Arc::downgrade(entry);
        tokio::spawn(async move {
            loop {
                let Some(deadline) = entry.upgrade().map(|entry| entry.deadline()) else {
                    return;
                };
                tokio::time::sleep(deadline.saturating_duration_since(Instant::now())).await;
                let Some(current) = entry.upgrade() else {
                    return;
                };
                if current.deadline() > Instant::now() {
                    continue;
                }
                if let Some(pending) = pending.upgrade()
                    && let Ok(mut pending) = pending.lock()
                    && pending
                        .get(&session_id)
                        .is_some_and(|stored| Arc::ptr_eq(stored, &current))
                {
                    // Removing the entry also drops an unconsumed grant.
                    pending.remove(&session_id);
                }
                current.abort.abort();
                return;
            }
        });
    }
    pub(crate) fn shutdown(&self) {
        self.closed.store(true, Ordering::SeqCst);
        if let Ok(mut pending) = self.pending.lock() {
            for (_, entry) in pending.drain() {
                entry.abort.abort();
            }
        }
    }
    pub(crate) fn cancel(&self, session_id: &str) -> Result<(), OAuthError> {
        if let Some(entry) = self
            .pending
            .lock()
            .map_err(|_| OAuthError::Storage)?
            .remove(session_id)
        {
            entry.abort.abort();
        }
        Ok(())
    }
    /// The live session, or `ExpiredSession` for unknown, expired or closed ones.
    fn get(&self, session_id: &str) -> Result<Arc<Entry>, OAuthError> {
        let mut pending = self.pending.lock().map_err(|_| OAuthError::Storage)?;
        let entry = pending
            .get(session_id)
            .cloned()
            .ok_or(OAuthError::ExpiredSession)?;
        if self.closed.load(Ordering::SeqCst) || entry.deadline() <= Instant::now() {
            pending.remove(session_id);
            entry.abort.abort();
            return Err(OAuthError::ExpiredSession);
        }
        Ok(entry)
    }
    /// Waits for the browser result of a session. A failed sign-in removes the
    /// session; a successful one stays for the caller to keep or consume.
    async fn await_signed(&self, session_id: &str) -> Result<(Arc<Entry>, SignedIn), OAuthError> {
        let entry = self.get(session_id)?;
        let mut task = {
            let mut state = entry.state.lock().map_err(|_| OAuthError::Storage)?;
            match &mut *state {
                State::Waiting(task) => task.take().ok_or(OAuthError::ExpiredSession)?,
                State::Ready(_) => return Err(OAuthError::ExpiredSession),
            }
        };
        let budget = entry.deadline().saturating_duration_since(Instant::now());
        let result = tokio::time::timeout(budget, &mut task).await;
        // Cancel remains able to abort a task while finish is awaiting it.
        {
            let mut pending = self.pending.lock().map_err(|_| OAuthError::Storage)?;
            let still_owned = pending
                .get(session_id)
                .is_some_and(|current| Arc::ptr_eq(current, &entry));
            if !still_owned
                || self.closed.load(Ordering::SeqCst)
                || entry.deadline() <= Instant::now()
            {
                task.abort();
                return Err(OAuthError::ExpiredSession);
            }
            if !matches!(result, Ok(Ok(Ok(_)))) {
                pending.remove(session_id);
            }
        }
        match result {
            Ok(Ok(Ok(signed))) => Ok((entry, signed)),
            Ok(Ok(Err(error))) => Err(error),
            Ok(Err(_)) => Err(OAuthError::ExpiredSession),
            Err(_) => {
                task.abort();
                Err(OAuthError::ExpiredSession)
            }
        }
    }
    fn state_of(signed: &SignedIn) -> SignInState {
        match signed.mailbox() {
            Some(mailbox) => SignInState::Verified(mailbox.to_owned()),
            None => SignInState::NeedsMailbox(signed.identity().mailbox_hint.clone()),
        }
    }
    /// Waits for the browser of a new-account session. The grant is kept, with
    /// a longer lifetime, so the page can ask for an address and create the account.
    pub(crate) async fn finish(&self, session_id: &str) -> Result<SignInState, OAuthError> {
        let entry = self.get(session_id)?;
        if !matches!(entry.kind, Kind::Create { .. }) {
            return Err(OAuthError::ExpiredSession);
        }
        let (entry, signed) = self.await_signed(session_id).await?;
        let state = Self::state_of(&signed);
        entry.set_deadline(Instant::now() + READY_LIFETIME);
        *entry.state.lock().map_err(|_| OAuthError::Storage)? = State::Ready(Box::new(Ready {
            signed,
            attempts: 0,
            busy: false,
        }));
        Ok(state)
    }
    /// Tries a typed address with the token the session already holds: no
    /// second browser trip. Only the mail server's acceptance makes it proven.
    pub(crate) async fn confirm_mailbox(
        &self,
        session_id: &str,
        address: &str,
    ) -> Result<SignInState, OAuthError> {
        let entry = self.get(session_id)?;
        if !matches!(entry.kind, Kind::Create { .. }) {
            return Err(OAuthError::ExpiredSession);
        }
        let mut signed = {
            let mut state = entry.state.lock().map_err(|_| OAuthError::Storage)?;
            match &mut *state {
                State::Ready(ready) if ready.signed.mailbox().is_none() && !ready.busy => {
                    ready.attempts += 1;
                    ready.busy = true;
                    ready.signed.clone()
                }
                State::Ready(_) => return Err(OAuthError::InvalidCallback),
                State::Waiting(_) => return Err(OAuthError::ExpiredSession),
            }
        };
        let result = signed
            .prove(entry.verifier.as_ref(), &entry.route, address)
            .await;
        let mut pending = self.pending.lock().map_err(|_| OAuthError::Storage)?;
        if !pending
            .get(session_id)
            .is_some_and(|current| Arc::ptr_eq(current, &entry))
        {
            return Err(OAuthError::ExpiredSession);
        }
        let mut state = entry.state.lock().map_err(|_| OAuthError::Storage)?;
        let State::Ready(ready) = &mut *state else {
            return Err(OAuthError::ExpiredSession);
        };
        ready.busy = false;
        match result {
            Ok(()) => {
                let verified = Self::state_of(&signed);
                ready.signed = signed;
                Ok(verified)
            }
            Err(error) => {
                if ready.attempts >= MAX_MAILBOX_ATTEMPTS {
                    // Out of tries: the user signs in again.
                    drop(state);
                    pending.remove(session_id);
                }
                Err(error)
            }
        }
    }
    /// Hands out what account creation needs and marks the session busy, so
    /// two creations cannot run at once. Call [`Self::created`] on success and
    /// [`Self::create_failed`] otherwise.
    pub(crate) fn begin_create(&self, session_id: &str) -> Result<CreateTarget, OAuthError> {
        let entry = self.get(session_id)?;
        let Kind::Create { route } = &entry.kind else {
            return Err(OAuthError::ExpiredSession);
        };
        let mut state = entry.state.lock().map_err(|_| OAuthError::Storage)?;
        let State::Ready(ready) = &mut *state else {
            return Err(OAuthError::ExpiredSession);
        };
        if ready.busy {
            return Err(OAuthError::InvalidCallback);
        }
        let authorization = ready.signed.authorization()?;
        ready.busy = true;
        Ok(CreateTarget {
            route: route.clone(),
            registration: ready.signed.registration().clone(),
            authorization,
        })
    }
    /// The account was created: the session and its tokens are consumed.
    pub(crate) fn created(&self, session_id: &str) {
        let _ = self.cancel(session_id);
    }
    /// Creation failed before anything was saved: the session can try again.
    pub(crate) fn create_failed(&self, session_id: &str) {
        if let Ok(entry) = self.get(session_id)
            && let Ok(mut state) = entry.state.lock()
            && let State::Ready(ready) = &mut *state
        {
            ready.busy = false;
        }
    }
    /// Waits for the browser of a reauthorization session. The address is known
    /// and has already been proven by the mail server.
    pub(crate) async fn finish_reauthorization(
        &self,
        session_id: &str,
    ) -> Result<CompletedReauthorization, OAuthError> {
        let entry = self.get(session_id)?;
        if !matches!(entry.kind, Kind::Reauthorize { .. }) {
            return Err(OAuthError::ExpiredSession);
        }
        let (entry, signed) = self.await_signed(session_id).await?;
        // Consumed exactly once, whatever happens next.
        let _ = self.cancel(session_id);
        let Kind::Reauthorize {
            account_id,
            expected_revision,
        } = &entry.kind
        else {
            return Err(OAuthError::ExpiredSession);
        };
        Ok(CompletedReauthorization {
            registration: signed.registration().clone(),
            authorization: signed.authorization()?,
            account_id: account_id.clone(),
            expected_revision: *expected_revision,
        })
    }
}
#[cfg(test)]
impl SessionCoordinator {
    /// How long the session has left, for lifetime tests.
    pub(crate) fn remaining(&self, session_id: &str) -> Option<Duration> {
        let pending = self.pending.lock().ok()?;
        let entry = pending.get(session_id)?;
        Some(entry.deadline().saturating_duration_since(Instant::now()))
    }
    /// Lets the session's deadline pass, as if its time were up.
    pub(crate) fn expire_now(&self, session_id: &str) {
        if let Some(entry) = self.pending.lock().unwrap().get(session_id) {
            entry.set_deadline(Instant::now());
        }
    }
    pub(crate) fn session_count(&self) -> usize {
        self.pending.lock().unwrap().len()
    }
}
impl Drop for SessionCoordinator {
    fn drop(&mut self) {
        if let Ok(mut pending) = self.pending.lock() {
            for (_, entry) in pending.drain() {
                entry.abort.abort();
            }
        }
    }
}
