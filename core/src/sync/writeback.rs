//! Writes the user's read and starred changes, and the deletions they asked
//! for, back to the server.
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

use chrono::Utc;

use crate::{
    AppError, ErrorCode,
    accounts::{AccountService, sync_plan::PlanError},
    imap::{self, ImapError, Session},
    storage::{Flag, PendingChange, deletions::PendingDeletion},
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
    /// Changes and deletions that stay queued and will be tried again.
    pub waiting: usize,
    /// Messages the server deleted (moved to Trash or expunged).
    pub deleted: usize,
    /// Deletions that could not be done and whose messages show again. Hosts
    /// tell the user when this is non-zero.
    pub deletions_given_back: usize,
    /// When the next scheduled deletion falls due, counted from the end of
    /// this round. The worker wakes for it without a new user action.
    pub next_deletion_in: Option<Duration>,
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

impl WriteBackReport {
    /// Whether the host has anything to hear about this round.
    pub(crate) fn has_news(&self) -> bool {
        self.sent > 0
            || self.local_only_stars > 0
            || self.deleted > 0
            || self.deletions_given_back > 0
    }
}

/// The store operations deleting needs, split from [`Queue`] the same way.
pub(crate) trait DeletionQueue: Send + Sync {
    /// The server confirmed; the messages become tombstones.
    fn deleted(&self, message_ids: &[String]) -> Result<(), AppError>;
    /// The server refused this time. Returns how many were given back for
    /// having failed too often.
    fn deletion_failed(&self, message_ids: &[String]) -> Result<u32, AppError>;
    /// The server can never do it; give the messages back now.
    fn deletion_impossible(&self, message_ids: &[String]) -> Result<(), AppError>;
}

impl DeletionQueue for Mutex<AccountService> {
    fn deleted(&self, message_ids: &[String]) -> Result<(), AppError> {
        self.lock()
            .map_err(|_| AppError::new(ErrorCode::Internal))?
            .finish_deletions(message_ids)
    }

    fn deletion_failed(&self, message_ids: &[String]) -> Result<u32, AppError> {
        self.lock()
            .map_err(|_| AppError::new(ErrorCode::Internal))?
            .deletion_attempt_failed(message_ids)
    }

    fn deletion_impossible(&self, message_ids: &[String]) -> Result<(), AppError> {
        self.lock()
            .map_err(|_| AppError::new(ErrorCode::Internal))?
            .abandon_deletions(message_ids)
    }
}

fn deletion_refused(queue: &dyn DeletionQueue, ids: &[String], report: &mut WriteBackReport) {
    // A store error leaves the rows claimed; the next round tries again.
    let given_back = queue.deletion_failed(ids).unwrap_or(0) as usize;
    report.deletions_given_back += given_back;
    report.waiting += ids.len().saturating_sub(given_back);
}

fn deletion_impossible(queue: &dyn DeletionQueue, ids: &[String], report: &mut WriteBackReport) {
    match queue.deletion_impossible(ids) {
        Ok(()) => report.deletions_given_back += ids.len(),
        Err(_) => report.waiting += ids.len(),
    }
}

/// Carries out `deletions` over `session`, one source folder and target at a
/// time.
///
/// Moving to Trash uses `UID MOVE` when the server has it, otherwise `UID COPY`,
/// `\Deleted` and `UID EXPUNGE`. Deleting for good is `\Deleted` plus
/// `UID EXPUNGE`. A plain `EXPUNGE` is never sent: it would also remove what
/// other clients marked `\Deleted`, so a server without `UIDPLUS` (and, for
/// Trash, without `MOVE`) gets the messages given back instead.
pub(crate) async fn push_deletions(
    session: &mut Session,
    queue: &dyn DeletionQueue,
    deletions: Vec<PendingDeletion>,
) -> Result<WriteBackReport, ImapError> {
    let mut report = WriteBackReport::default();
    let moves = session.supports("MOVE");
    let expunges = session.supports("UIDPLUS");
    type Group = (String, u32, Option<String>);
    let mut groups: BTreeMap<Group, Vec<PendingDeletion>> = BTreeMap::new();
    for deletion in deletions {
        groups
            .entry((
                deletion.folder_remote_name.clone(),
                deletion.uid_validity,
                deletion.target_remote_name.clone(),
            ))
            .or_default()
            .push(deletion);
    }
    for ((remote_name, uid_validity, target), group) in groups {
        let ids: Vec<String> = group.iter().map(|d| d.message_id.clone()).collect();
        let uids: Vec<u32> = group.iter().map(|d| d.uid).collect();
        if !(expunges || moves && target.is_some()) {
            deletion_impossible(queue, &ids, &mut report);
            continue;
        }
        let mailbox = match session.select(&remote_name).await {
            Ok(mailbox) => mailbox,
            Err(error) if error.session_usable() => {
                deletion_refused(queue, &ids, &mut report);
                continue;
            }
            Err(error) => return Err(error),
        };
        if mailbox.uid_validity != uid_validity {
            // The UIDs belong to an older generation of this folder; acting on
            // them could hit different messages.
            deletion_impossible(queue, &ids, &mut report);
            continue;
        }
        if mailbox.read_only {
            deletion_refused(queue, &ids, &mut report);
            continue;
        }
        let result = match &target {
            Some(trash) if moves => session.uid_move(&uids, trash).await,
            Some(trash) => match session.uid_copy(&uids, trash).await {
                Ok(()) => match session.mark_deleted(&uids).await {
                    Ok(()) => session.uid_expunge(&uids).await,
                    Err(error) => Err(error),
                },
                Err(error) => Err(error),
            },
            None => match session.mark_deleted(&uids).await {
                Ok(()) => session.uid_expunge(&uids).await,
                Err(error) => Err(error),
            },
        };
        match result {
            Ok(()) => match queue.deleted(&ids) {
                Ok(()) => report.deleted += ids.len(),
                // The server is done; a later round finds the UIDs gone and
                // the header sync turns them into tombstones anyway.
                Err(_) => report.waiting += ids.len(),
            },
            Err(error) if error.session_usable() => deletion_refused(queue, &ids, &mut report),
            Err(error) => return Err(error),
        }
    }
    Ok(report)
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
        let mut service = service_for_read
            .lock()
            .map_err(|_| AppError::new(ErrorCode::Internal))?;
        let changes: Vec<PendingChange> = service
            .pending_flag_changes(&id, BATCH)?
            .into_iter()
            .filter(|change| change.attempts < MAX_ATTEMPTS)
            .collect();
        let (deletions, given_back) = service.claim_due_deletions(&id, Utc::now())?;
        let plan = (!changes.is_empty() || !deletions.is_empty()).then(|| service.sync_plan(&id));
        Ok::<_, AppError>((changes, deletions, given_back, plan))
    })
    .await;
    let (changes, deletions, given_back, plan) = match prepared {
        Ok(Ok(prepared)) => prepared,
        Ok(Err(error)) if error.code == ErrorCode::NotFound => return RoundEnd::Blocked,
        _ => return RoundEnd::Retry(WriteBackReport::default()),
    };
    let mut report = WriteBackReport {
        deletions_given_back: given_back as usize,
        ..WriteBackReport::default()
    };
    let Some(plan) = plan else {
        report.next_deletion_in = next_deletion_in(service, account_id).await;
        return RoundEnd::Done(report);
    };
    let plan = match plan {
        Ok(plan) => plan,
        // Credentials are not trusted, missing or the account is gone: waiting
        // does not help, and retrying could lock the account.
        Err(PlanError::Account(error)) if error.code != ErrorCode::NotFound => {
            return RoundEnd::Retry(report);
        }
        Err(_) => return RoundEnd::Blocked,
    };
    let queued = changes.len() + deletions.len();
    let mut session = match imap::open_session(&plan.route, &plan.login).await {
        Ok(session) => session,
        Err(error) => return failed_round(error, queued),
    };
    // Flags first: a message about to leave its folder should still get the
    // read state the user gave it, since Trash keeps it.
    let result = match push_changes(&mut session, service.as_ref(), changes).await {
        Ok(flags) => push_deletions(&mut session, service.as_ref(), deletions)
            .await
            .map(|deleted| (flags, deleted)),
        Err(error) => Err(error),
    };
    let _ = session.logout().await;
    match result {
        Ok((flags, deleted)) => {
            report.sent = flags.sent;
            report.local_only_stars = flags.local_only_stars;
            report.waiting = flags.waiting + deleted.waiting;
            report.deleted = deleted.deleted;
            report.deletions_given_back += deleted.deletions_given_back;
            report.next_deletion_in = next_deletion_in(service, account_id).await;
            if report.waiting > 0 {
                RoundEnd::Retry(report)
            } else {
                RoundEnd::Done(report)
            }
        }
        Err(error) => failed_round(error, 0),
    }
}

/// How long until the account's next not-yet-started deletion is due.
async fn next_deletion_in(
    service: &Arc<Mutex<AccountService>>,
    account_id: &str,
) -> Option<Duration> {
    let service = service.clone();
    let id = account_id.to_owned();
    let due =
        tokio::task::spawn_blocking(move || service.lock().ok()?.next_deletion_due(&id).ok()?)
            .await
            .ok()??;
    Some((due - Utc::now()).to_std().unwrap_or(Duration::ZERO))
}

pub(crate) fn retry_delay(failures: u32) -> Duration {
    backoff(failures)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::imap::{ImapError, test_writable_session, test_writable_session_with};
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
                waiting: 0,
                ..WriteBackReport::default()
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
                waiting: 0,
                ..WriteBackReport::default()
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
                waiting: 1,
                ..WriteBackReport::default()
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
                waiting: 2,
                ..WriteBackReport::default()
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

    #[derive(Default)]
    struct Deletions {
        calls: Mutex<Vec<String>>,
        give_back_on_failure: u32,
    }

    impl Deletions {
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
        fn record(&self, verb: &str, ids: &[String]) {
            self.calls
                .lock()
                .unwrap()
                .push(format!("{verb} {}", ids.join(",")));
        }
    }

    impl DeletionQueue for Deletions {
        fn deleted(&self, message_ids: &[String]) -> Result<(), AppError> {
            self.record("deleted", message_ids);
            Ok(())
        }
        fn deletion_failed(&self, message_ids: &[String]) -> Result<u32, AppError> {
            self.record("failed", message_ids);
            Ok(self.give_back_on_failure)
        }
        fn deletion_impossible(&self, message_ids: &[String]) -> Result<(), AppError> {
            self.record("impossible", message_ids);
            Ok(())
        }
    }

    fn deletion(uid: u32, trash: Option<&str>) -> PendingDeletion {
        PendingDeletion {
            message_id: format!("message-{uid}"),
            folder_id: "folder".into(),
            folder_remote_name: "INBOX".into(),
            uid_validity: 9,
            uid,
            target_remote_name: trash.map(str::to_owned),
            attempts: 0,
        }
    }

    fn ok(command: &str) -> (String, String) {
        (command.into(), "{tag} OK done\r\n".into())
    }

    #[tokio::test]
    async fn a_server_with_move_gets_one_uid_move_into_trash() {
        let (mut session, server) = test_writable_session_with(
            "IMAP4rev1 MOVE UIDPLUS",
            vec![
                select("\\Deleted \\Seen \\*", 9, "READ-WRITE"),
                ok("UID MOVE 4,7 \"Trash\""),
            ],
        )
        .await;
        let queue = Deletions::default();
        let report = push_deletions(
            &mut session,
            &queue,
            vec![deletion(7, Some("Trash")), deletion(4, Some("Trash"))],
        )
        .await
        .unwrap();
        assert_eq!(report.deleted, 2);
        assert_eq!(report.waiting, 0);
        assert_eq!(queue.calls(), ["deleted message-7,message-4"]);
        drop(session);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn without_move_trash_is_a_copy_then_an_expunge_of_exactly_those_uids() {
        let (mut session, server) = test_writable_session_with(
            "IMAP4rev1 UIDPLUS",
            vec![
                select("\\Deleted \\Seen \\*", 9, "READ-WRITE"),
                ok("UID COPY 4 \"Trash\""),
                ok("UID STORE 4 +FLAGS.SILENT (\\Deleted)"),
                ok("UID EXPUNGE 4"),
            ],
        )
        .await;
        let queue = Deletions::default();
        let report = push_deletions(&mut session, &queue, vec![deletion(4, Some("Trash"))])
            .await
            .unwrap();
        assert_eq!(report.deleted, 1);
        assert_eq!(queue.calls(), ["deleted message-4"]);
        drop(session);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn deleting_for_good_marks_and_expunges_only_the_chosen_uids() {
        let (mut session, server) = test_writable_session_with(
            "IMAP4rev1 MOVE UIDPLUS",
            vec![
                select("\\Deleted \\Seen \\*", 9, "READ-WRITE"),
                ok("UID STORE 5 +FLAGS.SILENT (\\Deleted)"),
                ok("UID EXPUNGE 5"),
            ],
        )
        .await;
        let queue = Deletions::default();
        let report = push_deletions(&mut session, &queue, vec![deletion(5, None)])
            .await
            .unwrap();
        assert_eq!(report.deleted, 1);
        drop(session);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn without_uidplus_nothing_is_expunged_and_the_messages_come_back() {
        // MOVE alone still covers Trash; deleting for good would need a plain
        // EXPUNGE, which could take other clients' \Deleted messages with it.
        let (mut session, server) = test_writable_session_with(
            "IMAP4rev1 MOVE",
            vec![
                select("\\Deleted \\Seen \\*", 9, "READ-WRITE"),
                ok("UID MOVE 2 \"Trash\""),
            ],
        )
        .await;
        let queue = Deletions::default();
        let report = push_deletions(
            &mut session,
            &queue,
            vec![deletion(1, None), deletion(2, Some("Trash"))],
        )
        .await
        .unwrap();
        assert_eq!(report.deleted, 1);
        assert_eq!(report.deletions_given_back, 1);
        assert_eq!(queue.calls(), ["impossible message-1", "deleted message-2"]);
        drop(session);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn a_server_with_neither_move_nor_uidplus_is_never_touched() {
        let (mut session, server) = test_writable_session_with("IMAP4rev1", vec![]).await;
        let queue = Deletions::default();
        let report = push_deletions(&mut session, &queue, vec![deletion(3, Some("Trash"))])
            .await
            .unwrap();
        assert_eq!(report.deletions_given_back, 1);
        assert_eq!(queue.calls(), ["impossible message-3"]);
        drop(session);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn a_new_uid_generation_gives_the_deletion_back_without_acting() {
        let (mut session, server) = test_writable_session_with(
            "IMAP4rev1 MOVE UIDPLUS",
            vec![select("\\Deleted \\Seen \\*", 10, "READ-WRITE")],
        )
        .await;
        let queue = Deletions::default();
        let report = push_deletions(&mut session, &queue, vec![deletion(3, Some("Trash"))])
            .await
            .unwrap();
        assert_eq!(report.deletions_given_back, 1);
        assert_eq!(queue.calls(), ["impossible message-3"]);
        drop(session);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn a_refused_move_stays_queued_until_it_has_failed_too_often() {
        let (mut session, server) = test_writable_session_with(
            "IMAP4rev1 MOVE UIDPLUS",
            vec![
                select("\\Deleted \\Seen \\*", 9, "READ-WRITE"),
                (
                    "UID MOVE 3 \"Trash\"".into(),
                    "{tag} NO [TRYCREATE] no such mailbox\r\n".into(),
                ),
            ],
        )
        .await;
        let queue = Deletions::default();
        let report = push_deletions(&mut session, &queue, vec![deletion(3, Some("Trash"))])
            .await
            .unwrap();
        assert_eq!(report.waiting, 1);
        assert_eq!(report.deleted, 0);
        assert_eq!(queue.calls(), ["failed message-3"]);
        drop(session);
        server.await.unwrap();
    }
}
