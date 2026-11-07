//! Opaque, bounded, one-use authorization sessions. A session owns its exact
//! account target, route, callback listener, PKCE state and verified result.
use crate::{
    accounts::types::CreateAccountRequest,
    auth::{
        IdentityVerifier, OAuthError, OAuthSession, ProviderRegistration, VerifiedAuthorization,
        loopback::LoopbackCallback,
    },
    net::NetRoute,
};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub(crate) struct OAuthAuthorizationStart {
    pub session_id: String,
    pub authorization_url: String,
}
struct Pending {
    deadline: Instant,
    task: Mutex<Option<tokio::task::JoinHandle<Result<CompletedAuthorization, OAuthError>>>>,
    abort: tokio::task::AbortHandle,
}
pub(crate) struct CompletedAuthorization {
    pub request: CreateAccountRequest,
    pub registration: ProviderRegistration,
    pub authorization: VerifiedAuthorization,
    pub reauthorization: Option<(String, u32)>,
}
pub(crate) struct SessionCoordinator {
    pending: Arc<Mutex<HashMap<String, Arc<Pending>>>>,
    capacity: Arc<tokio::sync::Semaphore>,
    closed: AtomicBool,
}
impl Default for SessionCoordinator {
    fn default() -> Self {
        Self {
            pending: Arc::default(),
            closed: AtomicBool::new(false),
            capacity: Arc::new(tokio::sync::Semaphore::new(8)),
        }
    }
}
impl SessionCoordinator {
    pub(crate) async fn begin(
        &self,
        registration: ProviderRegistration,
        verifier: Arc<dyn IdentityVerifier>,
        route: NetRoute,
        request: CreateAccountRequest,
    ) -> Result<OAuthAuthorizationStart, OAuthError> {
        self.begin_target(registration, verifier, route, request, None)
            .await
    }
    pub(crate) async fn begin_reauthorization(
        &self,
        registration: ProviderRegistration,
        verifier: Arc<dyn IdentityVerifier>,
        route: NetRoute,
        request: CreateAccountRequest,
        account_id: String,
        expected_revision: u32,
    ) -> Result<OAuthAuthorizationStart, OAuthError> {
        self.begin_target(
            registration,
            verifier,
            route,
            request,
            Some((account_id, expected_revision)),
        )
        .await
    }
    async fn begin_target(
        &self,
        registration: ProviderRegistration,
        verifier: Arc<dyn IdentityVerifier>,
        route: NetRoute,
        mut request: CreateAccountRequest,
        reauthorization: Option<(String, u32)>,
    ) -> Result<OAuthAuthorizationStart, OAuthError> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(OAuthError::ExpiredSession);
        }
        super::oauth::normalize_oauth_request(&mut request)
            .map_err(|_| OAuthError::InvalidRegistration)?;
        registration.validate_mail_target(
            &request.config.email_address,
            &request.config.imap,
            &request.config.smtp,
        )?;
        // Reserve capacity before opening a listening socket. Expired sessions
        // are aborted even if their host never polls or cancels them.
        {
            let mut pending = self.pending.lock().map_err(|_| OAuthError::Storage)?;
            pending.retain(|_, entry| {
                if entry.deadline <= Instant::now() {
                    entry.abort.abort();
                    false
                } else {
                    true
                }
            });
            if pending.len() >= 8 {
                return Err(OAuthError::ExpiredSession);
            }
            // Binding is immediate and does not perform any remote network I/O.
            // Drop the lock before the await; recheck the bound on insertion below.
        }
        let permit = self
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| OAuthError::ExpiredSession)?;
        let callback = LoopbackCallback::bind().await?;
        let (session, url) =
            OAuthSession::begin(registration.clone(), callback.redirect_uri().clone())?;
        let expected_mailbox = request.config.email_address.clone();
        let session_id = uuid::Uuid::new_v4().to_string();
        let task = tokio::spawn(async move {
            let _permit = permit;
            tokio::time::timeout(Duration::from_secs(300), async move {
                let callback = callback.receive().await?;
                let authorization = session
                    .complete(&callback, &route, verifier.as_ref(), &expected_mailbox)
                    .await?;
                Ok(CompletedAuthorization {
                    request,
                    registration,
                    authorization,
                    reauthorization,
                })
            })
            .await
            .map_err(|_| OAuthError::ExpiredSession)?
        });
        let mut pending = self.pending.lock().map_err(|_| {
            task.abort();
            OAuthError::Storage
        })?;
        if pending.len() >= 8 || self.closed.load(Ordering::SeqCst) {
            task.abort();
            return Err(OAuthError::ExpiredSession);
        }
        let abort = task.abort_handle();
        pending.insert(
            session_id.clone(),
            Arc::new(Pending {
                deadline: Instant::now() + Duration::from_secs(300),
                task: Mutex::new(Some(task)),
                abort,
            }),
        );
        self.schedule_expiry(session_id.clone());
        Ok(OAuthAuthorizationStart {
            session_id,
            authorization_url: url.to_string(),
        })
    }
    fn schedule_expiry(&self, session_id: String) {
        let pending = Arc::downgrade(&self.pending);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(300)).await;
            if let Some(pending) = pending.upgrade()
                && let Ok(mut pending) = pending.lock()
                && let Some(entry) = pending.remove(&session_id)
            {
                // Removing the handle also drops an already completed task's
                // unconsumed verified tokens without requiring another call.
                entry.abort.abort();
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
    pub(crate) async fn finish(
        &self,
        session_id: &str,
    ) -> Result<CompletedAuthorization, OAuthError> {
        let entry = self
            .pending
            .lock()
            .map_err(|_| OAuthError::Storage)?
            .get(session_id)
            .cloned()
            .ok_or(OAuthError::ExpiredSession)?;
        let mut task = entry
            .task
            .lock()
            .map_err(|_| OAuthError::Storage)?
            .take()
            .ok_or(OAuthError::ExpiredSession)?;
        let budget = entry.deadline.saturating_duration_since(Instant::now());
        let result = tokio::time::timeout(budget, &mut task).await;
        // Cancel remains able to abort a task while finish is awaiting it.
        {
            let mut pending = self.pending.lock().map_err(|_| OAuthError::Storage)?;
            let still_owned = pending
                .get(session_id)
                .is_some_and(|current| Arc::ptr_eq(current, &entry));
            if !still_owned
                || self.closed.load(Ordering::SeqCst)
                || entry.deadline <= Instant::now()
            {
                task.abort();
                return Err(OAuthError::ExpiredSession);
            }
            pending.remove(session_id);
        }
        match result {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(OAuthError::ExpiredSession),
            Err(_) => {
                task.abort();
                Err(OAuthError::ExpiredSession)
            }
        }
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

#[cfg(test)]
mod expiry_tests {
    use super::*;
    #[tokio::test(start_paused = true)]
    async fn completed_results_expire_without_another_host_operation() {
        let coordinator = SessionCoordinator::default();
        let task = tokio::spawn(async {
            Err::<CompletedAuthorization, _>(OAuthError::AuthorizationDenied)
        });
        let abort = task.abort_handle();
        coordinator.pending.lock().unwrap().insert(
            "session".into(),
            Arc::new(Pending {
                deadline: Instant::now() + Duration::from_secs(300),
                task: Mutex::new(Some(task)),
                abort,
            }),
        );
        let weak_result_owner =
            Arc::downgrade(coordinator.pending.lock().unwrap().get("session").unwrap());
        coordinator.schedule_expiry("session".into());
        tokio::task::yield_now().await;
        assert_eq!(coordinator.pending.lock().unwrap().len(), 1);
        tokio::time::advance(Duration::from_secs(301)).await;
        tokio::task::yield_now().await;
        assert!(coordinator.pending.lock().unwrap().is_empty());
        assert!(weak_result_owner.upgrade().is_none());
    }
    #[tokio::test]
    async fn cancellation_after_task_completion_still_invalidates_a_waiting_finish() {
        use std::future::Future;
        let coordinator = SessionCoordinator::default();
        let (complete, wait) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async {
            let _ = wait.await;
            Err::<CompletedAuthorization, _>(OAuthError::AuthorizationDenied)
        });
        let abort = task.abort_handle();
        coordinator.pending.lock().unwrap().insert(
            "race".into(),
            Arc::new(Pending {
                deadline: Instant::now() + Duration::from_secs(300),
                task: Mutex::new(Some(task)),
                abort: abort.clone(),
            }),
        );
        let mut finish = Box::pin(coordinator.finish("race"));
        let waker = futures_util::task::noop_waker();
        assert!(
            finish
                .as_mut()
                .poll(&mut std::task::Context::from_waker(&waker))
                .is_pending()
        );
        complete.send(()).unwrap();
        while !abort.is_finished() {
            tokio::task::yield_now().await;
        }
        coordinator.cancel("race").unwrap();
        assert!(matches!(finish.await, Err(OAuthError::ExpiredSession)));
    }
}
