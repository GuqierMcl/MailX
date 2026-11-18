use std::hash::{BuildHasher, RandomState};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use crate::diagnostics::{DiagnosticBackend, DiagnosticContext, DiagnosticEvent, DiagnosticSink};
use crate::imap::Session;
use crate::sync::manager::{SyncFailure, WorkerToken};

pub(super) struct RoundDiagnostics {
    sink: DiagnosticSink,
    context: DiagnosticContext,
}

impl RoundDiagnostics {
    pub(super) fn new(sink: DiagnosticSink, token: &WorkerToken) -> Self {
        // A new random key on every launch keeps these aliases local to this process.
        static ALIASES: LazyLock<RandomState> = LazyLock::new(RandomState::new);
        static NEXT_ROUND: AtomicU64 = AtomicU64::new(1);
        Self {
            sink,
            context: DiagnosticContext {
                account: ALIASES.hash_one(&token.account_id),
                round: NEXT_ROUND.fetch_add(1, Ordering::Relaxed),
                backend: if token.is_ews {
                    DiagnosticBackend::Exchange
                } else {
                    DiagnosticBackend::Imap
                },
            },
        }
    }

    pub(super) fn attach(&self, session: &mut Session) {
        session.set_diagnostics(self.sink.clone(), self.context);
    }

    pub(super) fn failure(&self, failure: &SyncFailure) {
        (self.sink)(DiagnosticEvent::SyncFailure {
            context: self.context,
            code: failure.code,
            will_retry: failure.will_retry,
        });
    }

    pub(super) fn retry(&self, delay: Duration) {
        (self.sink)(DiagnosticEvent::SyncRetry {
            context: self.context,
            delay_ms: u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
        });
    }

    pub(super) fn recovered(&self) {
        (self.sink)(DiagnosticEvent::SyncRecovery {
            context: self.context,
        });
    }
}
