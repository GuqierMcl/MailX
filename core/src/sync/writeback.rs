//! Writes the user's read and starred changes back to the server.
//!
//! It runs beside the header sync on its own connection, never on the sync
//! connection: sync opens folders read-only (`EXAMINE`) and that guarantee is
//! kept; writing needs `SELECT`. The connection follows the account's route and
//! credential trust like sync does, because it uses the same plan.
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::{
    AppError, ErrorCode,
    accounts::{AccountService, sync_plan::PlanError},
    imap::{self, ImapError, Session},
    storage::{Flag, PendingChange},
};

/// Changes taken from the queue per round, so one huge backlog cannot hold the
/// connection forever.
const BATCH: usize = 200;
/// Waits before the first and later retries of a failed round.
const RETRY_BASE: Duration = Duration::from_secs(5);
const RETRY_CAP: Duration = Duration::from_secs(10 * 60);
/// A change that kept failing this often is left alone until the next round
/// that starts from a user action, instead of retrying every few minutes.
const MAX_ATTEMPTS: u32 = 12;

/// What one write-back round reports to the host.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WriteBackReport {
    /// Changes the server confirmed.
    pub sent: usize,
    /// Starred changes moved to "this device only" because their folder cannot
    /// keep the flag. Hosts show a notice when this is non-zero.
    pub local_only_stars: usize,
    /// Changes that stay queued and will be tried again.
    pub waiting: usize,
}

/// How a round ended, for the retry schedule.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RoundEnd {
    Done(WriteBackReport),
    /// Try again after the backoff for this many failures in a row.
    Retry(WriteBackReport),
    /// Nothing can be done until the account changes (login rejected, trust
    /// lost, account gone).
    Blocked,
    /// Keep all pending changes; only explicit sync/settings recovery restarts.
    ProviderPolicy,
}

fn backoff(failures: u32) -> Duration {
    RETRY_BASE
        .saturating_mul(1u32 << failures.min(7))
        .min(RETRY_CAP)
}

/// The store operations a round needs. The account service implements them; tests
/// supply a recorder, so the decisions below run without a database or network.
pub(crate) trait Queue: Send + Sync {
    fn finish(&self, change: &PendingChange) -> Result<(), AppError>;
    fn retry(&self, change: &PendingChange) -> Result<(), AppError>;
    fn unflaggable(&self, folder_id: &str) -> Result<usize, AppError>;
    fn flaggable(&self, folder_id: &str) -> Result<(), AppError>;
}

/// Sends `changes` over `session`, one folder at a time.
pub(crate) async fn push_changes(
    session: &mut Session,
    queue: &dyn Queue,
    changes: Vec<PendingChange>,
) -> Result<WriteBackReport, ImapError> {
    let mut report = WriteBackReport::default();
    let mut by_folder: BTreeMap<(String, u32), Vec<PendingChange>> = BTreeMap::new();
    for change in changes {
        by_folder
            .entry((change.remote_name.clone(), change.uid_validity))
            .or_default()
            .push(change);
    }
    for ((remote_name, uid_validity), folder_changes) in by_folder {
        let mailbox = match session.select(&remote_name).await {
            Ok(mailbox) => mailbox,
            Err(error) if error.session_usable() => {
                // This folder answered no (gone, not permitted): leave its changes
                // queued and carry on with the others.
                for change in &folder_changes {
                    queue.retry(change).ok();
                    report.waiting += 1;
                }
                continue;
            }
            Err(error) => return Err(error),
        };
        let folder_id = folder_changes[0].folder_id.clone();
        // The server reused the folder name for different mail: UIDs no longer
        // mean what they did, so nothing may be written here.
        if mailbox.uid_validity != uid_validity {
            for change in &folder_changes {
                queue.retry(change).ok();
                report.waiting += 1;
            }
            continue;
        }
        let can_flag = mailbox.keeps(Flag::Flagged.imap());
        match can_flag {
            Some(false) => report.local_only_stars += queue.unflaggable(&folder_id).unwrap_or(0),
            Some(true) => {
                queue.flaggable(&folder_id).ok();
            }
            None => {}
        }
        for change in folder_changes {
            let flag = change.flag;
            if flag == Flag::Flagged && can_flag == Some(false) {
                // Already moved to local-only above; never sent.
                continue;
            }
            if mailbox.read_only {
                queue.retry(&change).ok();
                report.waiting += 1;
                continue;
            }
            match session
                .store_flag(change.uid, flag.imap(), change.wanted)
                .await
            {
                Ok(_) => {
                    queue.finish(&change).ok();
                    report.sent += 1;
                }
                Err(ImapError::Rejected) if flag == Flag::Flagged && can_flag.is_none() => {
                    // The server never said whether it keeps stars and now refuses
                    // one: treat that as "cannot", once, instead of retrying forever.
                    report.local_only_stars += queue.unflaggable(&folder_id).unwrap_or(0);
                }
                Err(error) if error.session_usable() => {
                    queue.retry(&change).ok();
                    report.waiting += 1;
                }
                Err(error) => return Err(error),
            }
        }
    }
    Ok(report)
}

impl Queue for Mutex<AccountService> {
    fn finish(&self, change: &PendingChange) -> Result<(), AppError> {
        self.lock()
            .map_err(|_| AppError::new(ErrorCode::Internal))?
            .finish_flag_change(&change.message_id, change.flag, change.wanted)
            .map(|_| ())
    }

    fn retry(&self, change: &PendingChange) -> Result<(), AppError> {
        self.lock()
            .map_err(|_| AppError::new(ErrorCode::Internal))?
            .retry_flag_change(&change.message_id, change.flag)
    }

    fn unflaggable(&self, folder_id: &str) -> Result<usize, AppError> {
        self.lock()
            .map_err(|_| AppError::new(ErrorCode::Internal))?
            .mark_folder_unflaggable(folder_id)
    }

    fn flaggable(&self, folder_id: &str) -> Result<(), AppError> {
        self.lock()
            .map_err(|_| AppError::new(ErrorCode::Internal))?
            .mark_folder_flaggable(folder_id)
    }
}

fn failed_round(error: ImapError, waiting: usize) -> RoundEnd {
    match error {
        ImapError::ProviderPolicy => RoundEnd::ProviderPolicy,
        ImapError::Authentication(_) => RoundEnd::Blocked,
        _ => RoundEnd::Retry(WriteBackReport {
            waiting,
            ..WriteBackReport::default()
        }),
    }
}

/// Runs one round for an account: read the queue, connect, push.
pub(crate) async fn run_round(service: &Arc<Mutex<AccountService>>, account_id: &str) -> RoundEnd {
    let id = account_id.to_owned();
    let service_for_read = service.clone();
    let prepared = tokio::task::spawn_blocking(move || {
        let service = service_for_read
            .lock()
            .map_err(|_| AppError::new(ErrorCode::Internal))?;
        let changes: Vec<PendingChange> = service
            .pending_flag_changes(&id, BATCH)?
            .into_iter()
            .filter(|change| change.attempts < MAX_ATTEMPTS)
            .collect();
        if changes.is_empty() {
            return Ok(None);
        }
        Ok::<_, AppError>(Some((changes, service.sync_plan(&id))))
    })
    .await;
    let (changes, plan) = match prepared {
        Ok(Ok(Some(prepared))) => prepared,
        Ok(Ok(None)) => return RoundEnd::Done(WriteBackReport::default()),
        Ok(Err(error)) if error.code == ErrorCode::NotFound => return RoundEnd::Blocked,
        _ => return RoundEnd::Retry(WriteBackReport::default()),
    };
    let plan = match plan {
        Ok(plan) => plan,
        // Credentials are not trusted, missing or the account is gone: waiting
        // does not help, and retrying could lock the account.
        Err(PlanError::Account(error)) if error.code != ErrorCode::NotFound => {
            return RoundEnd::Retry(WriteBackReport::default());
        }
        Err(_) => return RoundEnd::Blocked,
    };
    let mut session = match imap::open_session(&plan.route, &plan.login).await {
        Ok(session) => session,
        Err(error) => return failed_round(error, changes.len()),
    };
    let result = push_changes(&mut session, service.as_ref(), changes).await;
    let _ = session.logout().await;
    match result {
        Ok(report) if report.waiting > 0 => RoundEnd::Retry(report),
        Ok(report) => RoundEnd::Done(report),
        Err(error) => failed_round(error, 0),
    }
}

pub(crate) fn retry_delay(failures: u32) -> Duration {
    backoff(failures)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::imap::{ImapError, test_writable_session};
    use std::sync::Mutex;

    #[derive(Default)]
    struct Recorder {
        calls: Mutex<Vec<String>>,
        unflaggable_moves: usize,
    }

    impl Recorder {
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Queue for Recorder {
        fn finish(&self, change: &PendingChange) -> Result<(), AppError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("finish {} {:?}", change.uid, change.flag));
            Ok(())
        }
        fn retry(&self, change: &PendingChange) -> Result<(), AppError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("retry {} {:?}", change.uid, change.flag));
            Ok(())
        }
        fn unflaggable(&self, folder_id: &str) -> Result<usize, AppError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("unflaggable {folder_id}"));
            Ok(self.unflaggable_moves)
        }
        fn flaggable(&self, folder_id: &str) -> Result<(), AppError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("flaggable {folder_id}"));
            Ok(())
        }
    }

    fn change(uid: u32, flag: Flag, wanted: bool) -> PendingChange {
        PendingChange {
            message_id: format!("message-{uid}"),
            account_id: "account".into(),
            mailbox_source_id: "source".into(),
            folder_id: "folder".into(),
            remote_name: "INBOX".into(),
            uid_validity: 9,
            uid,
            flag,
            wanted,
            attempts: 0,
            folder_can_flag: None,
        }
    }

    fn select(permanent: &str, validity: u32, mode: &str) -> (String, String) {
        (
            "SELECT \"INBOX\"".into(),
            format!(
                "* 3 EXISTS\r\n* OK [PERMANENTFLAGS ({permanent})] x\r\n* OK [UIDVALIDITY {validity}] x\r\n{{tag}} OK [{mode}] done\r\n"
            ),
        )
    }

    fn stored(uid: u32, verb: &str, flag: &str, now: &str) -> (String, String) {
        (
            format!("UID STORE {uid} {verb}FLAGS ({flag})"),
            format!("* 1 FETCH (UID {uid} FLAGS ({now}))\r\n{{tag}} OK stored\r\n"),
        )
    }

    #[test]
    fn retries_back_off_but_never_exceed_the_cap() {
        let waits: Vec<_> = (0..12)
            .map(|failures| retry_delay(failures).as_secs())
            .collect();
        assert_eq!(&waits[..4], [5, 10, 20, 40]);
        assert!(waits.windows(2).all(|pair| pair[0] <= pair[1]));
        assert_eq!(*waits.last().unwrap(), RETRY_CAP.as_secs());
    }

    #[tokio::test]
    async fn a_server_that_keeps_both_flags_gets_every_change_and_confirms_each() {
        let (mut session, server) = test_writable_session(vec![
            select("\\Deleted \\Seen \\Flagged \\*", 9, "READ-WRITE"),
            stored(1, "+", "\\Seen", "\\Seen"),
            stored(2, "-", "\\Flagged", ""),
        ])
        .await;
        let queue = Recorder::default();
        let report = push_changes(
            &mut session,
            &queue,
            vec![change(1, Flag::Seen, true), change(2, Flag::Flagged, false)],
        )
        .await
        .unwrap();
        assert_eq!(
            report,
            WriteBackReport {
                sent: 2,
                local_only_stars: 0,
                waiting: 0
            }
        );
        assert_eq!(
            queue.calls(),
            ["flaggable folder", "finish 1 Seen", "finish 2 Flagged"]
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn a_server_without_star_support_keeps_stars_local_and_still_sends_read() {
        // 2925: PERMANENTFLAGS lists only \Deleted and \Seen.
        let (mut session, server) = test_writable_session(vec![
            select("\\Deleted \\Seen", 9, "READ-WRITE"),
            stored(2, "+", "\\Seen", "\\Seen"),
        ])
        .await;
        let queue = Recorder {
            unflaggable_moves: 1,
            ..Recorder::default()
        };
        let report = push_changes(
            &mut session,
            &queue,
            vec![change(1, Flag::Flagged, true), change(2, Flag::Seen, true)],
        )
        .await
        .unwrap();
        assert_eq!(
            report,
            WriteBackReport {
                sent: 1,
                local_only_stars: 1,
                waiting: 0
            }
        );
        // No STORE was scripted for the star: sending one would have failed the script.
        assert_eq!(queue.calls(), ["unflaggable folder", "finish 2 Seen"]);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn a_reused_folder_name_is_never_written_to() {
        // The server now reports UIDVALIDITY 77, the change was made against 9.
        let (mut session, server) =
            test_writable_session(vec![select("\\Deleted \\Seen \\Flagged", 77, "READ-WRITE")])
                .await;
        let queue = Recorder::default();
        let report = push_changes(&mut session, &queue, vec![change(1, Flag::Seen, true)])
            .await
            .unwrap();
        assert_eq!(
            report,
            WriteBackReport {
                sent: 0,
                local_only_stars: 0,
                waiting: 1
            }
        );
        assert_eq!(queue.calls(), ["retry 1 Seen"]);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn a_read_only_folder_keeps_changes_waiting_without_sending() {
        let (mut session, server) =
            test_writable_session(vec![select("\\Seen \\Flagged", 9, "READ-ONLY")]).await;
        let queue = Recorder::default();
        let report = push_changes(&mut session, &queue, vec![change(1, Flag::Seen, true)])
            .await
            .unwrap();
        assert_eq!(report.waiting, 1);
        assert_eq!(report.sent, 0);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn a_refusal_or_unconfirmed_store_stays_queued_and_the_next_change_still_goes_out() {
        let (mut session, server) = test_writable_session(vec![
            select("\\Seen \\Flagged", 9, "READ-WRITE"),
            ("UID STORE 1".into(), "{tag} NO [CANNOT] nope\r\n".into()),
            ("UID STORE 2".into(), "{tag} OK done\r\n".into()),
            stored(3, "+", "\\Seen", "\\Seen"),
        ])
        .await;
        let queue = Recorder::default();
        let report = push_changes(
            &mut session,
            &queue,
            vec![
                change(1, Flag::Seen, true),
                change(2, Flag::Seen, true),
                change(3, Flag::Seen, true),
            ],
        )
        .await
        .unwrap();
        assert_eq!(
            report,
            WriteBackReport {
                sent: 1,
                local_only_stars: 0,
                waiting: 2
            }
        );
        assert_eq!(
            queue.calls(),
            [
                "flaggable folder",
                "retry 1 Seen",
                "retry 2 Seen",
                "finish 3 Seen"
            ]
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn a_star_refused_by_a_server_that_never_said_is_given_up_on_once() {
        // No PERMANENTFLAGS: unknown. The refusal teaches us the folder cannot.
        let (mut session, server) = test_writable_session(vec![
            (
                "SELECT".into(),
                "* OK [UIDVALIDITY 9] x\r\n{tag} OK [READ-WRITE] done\r\n".into(),
            ),
            ("UID STORE 1".into(), "{tag} NO nope\r\n".into()),
        ])
        .await;
        let queue = Recorder {
            unflaggable_moves: 1,
            ..Recorder::default()
        };
        let report = push_changes(&mut session, &queue, vec![change(1, Flag::Flagged, true)])
            .await
            .unwrap();
        assert_eq!(report.local_only_stars, 1);
        assert_eq!(report.waiting, 0);
        assert_eq!(queue.calls(), ["unflaggable folder"]);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn a_dropped_connection_is_an_error_and_confirms_nothing() {
        let (mut session, server) = test_writable_session(vec![
            select("\\Seen", 9, "READ-WRITE"),
            ("UID STORE 1".into(), "* BYE going away\r\n".into()),
        ])
        .await;
        let queue = Recorder::default();
        let result = push_changes(&mut session, &queue, vec![change(1, Flag::Seen, true)]).await;
        assert!(matches!(result, Err(ImapError::Protocol(_))));
        assert!(queue.calls().iter().all(|call| !call.starts_with("finish")));
        let _ = server.await;
    }
    #[tokio::test]
    async fn provider_policy_keeps_flags_queued_and_never_downgrades_a_star_to_local_only() {
        for during_store in [false, true] {
            let policy =
                "{tag} NO Unsafe Login. Please contact kefu@188.com for help private-marker\r\n";
            let script = if during_store {
                vec![
                    (
                        "SELECT".into(),
                        "* OK [UIDVALIDITY 9] x\r\n{tag} OK [READ-WRITE] done\r\n".into(),
                    ),
                    ("UID STORE".into(), policy.into()),
                ]
            } else {
                vec![("SELECT".into(), policy.into())]
            };
            let (mut session, peer) = test_writable_session(script).await;
            let queue = Recorder::default();
            let error = push_changes(&mut session, &queue, vec![change(1, Flag::Flagged, true)])
                .await
                .unwrap_err();
            assert_eq!(error, ImapError::ProviderPolicy);
            assert_eq!(failed_round(error, 1), RoundEnd::ProviderPolicy);
            assert!(
                queue.calls().is_empty(),
                "policy must neither acknowledge nor alter the queue"
            );
            peer.await.unwrap();
        }
        assert!(matches!(
            failed_round(ImapError::Rejected, 1),
            RoundEnd::Retry(_)
        ));
    }
}
