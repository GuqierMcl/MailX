use std::fs;

use mailx_core::diagnostics::{DiagnosticBackend, DiagnosticContext};
use serde_json::{Value, json};

use super::*;

impl Diagnostics {
    pub(crate) fn stop_and_wait_for_writer(&self) {
        assert!(self.stop());
        // The writer signals after releasing its files. The sender may still
        // be alive when that signal is received.
        self.done
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(10))
            .expect("diagnostic writer did not finish");
    }
}

fn read_records(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn shutdown_drains_records_and_next_session_appends() {
    let profile = tempfile::tempdir().unwrap();
    let logs = profile.path().join("logs/diagnostics.jsonl");
    for _ in 0..2 {
        let diagnostics = Diagnostics::start(profile.path());
        let sink = diagnostics.sink();
        sink(DiagnosticEvent::Startup);
        diagnostics.stop_and_wait_for_writer();
        sink(DiagnosticEvent::Startup);
    }
    let records = read_records(&logs);
    assert_eq!(records.len(), 2);
    for record in &records {
        assert_eq!(record["event"], "startup");
        assert_eq!(record["version"], env!("CARGO_PKG_VERSION"));
        assert!(record["timestamp"].as_str().unwrap().ends_with('Z'));
    }
    assert_ne!(records[0]["session"], records[1]["session"]);
}

#[test]
fn retry_storms_keep_the_first_event_and_count_subsequent_rounds() {
    let profile = tempfile::tempdir().unwrap();
    let mut writer = Writer::new(profile.path());
    let now = Instant::now();
    for round in 1..=100 {
        writer.record(
            DiagnosticEvent::SyncRetry {
                context: DiagnosticContext {
                    account: 1,
                    round,
                    backend: DiagnosticBackend::Imap,
                },
                delay_ms: round * 1000,
            },
            now,
        );
    }
    writer.flush_repeats(now + REPEAT_INTERVAL, false);
    let records = read_records(&profile.path().join("logs/diagnostics.jsonl"));
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["context"]["round"], 1);
    assert_eq!(records[1]["context"]["round"], 100);
    assert_eq!(records[1]["repeats"], 99);
    assert_eq!(records[1]["delay_ms"], 100000);
}

#[test]
fn recovery_ends_a_retry_episode() {
    let profile = tempfile::tempdir().unwrap();
    let mut writer = Writer::new(profile.path());
    let now = Instant::now();
    let context = |round| DiagnosticContext {
        account: 1,
        round,
        backend: DiagnosticBackend::Imap,
    };
    for round in [1, 2] {
        writer.record(
            DiagnosticEvent::SyncRetry {
                context: context(round),
                delay_ms: 1000,
            },
            now,
        );
    }
    writer.record(
        DiagnosticEvent::SyncRecovery {
            context: context(3),
        },
        now,
    );
    writer.record(
        DiagnosticEvent::SyncRetry {
            context: context(4),
            delay_ms: 1000,
        },
        now,
    );
    let records = read_records(&profile.path().join("logs/diagnostics.jsonl"));
    assert_eq!(records[0]["event"], "sync_retry");
    assert_eq!(records[1]["repeats"], 1);
    assert_eq!(records[2]["event"], "sync_recovery");
    assert_eq!(records[3]["context"]["round"], 4);
    assert_eq!(records[3].get("repeats"), None);
}

#[test]
fn idle_writer_stops_even_when_a_sink_is_retained() {
    let profile = tempfile::tempdir().unwrap();
    let diagnostics = Diagnostics::start(profile.path());
    let sink = diagnostics.sink();
    diagnostics.stop_and_wait_for_writer();
    sink(DiagnosticEvent::Startup);
    assert!(!profile.path().join("logs").exists());
}

#[test]
fn queue_overflow_and_encoded_record_limits() {
    let profile = tempfile::tempdir().unwrap();
    let control = Control::default();
    control.dropped.store(30, Ordering::Relaxed);
    let mut writer = Writer::new(profile.path());
    writer.report_overflow(&control);
    writer.report_overflow(&control);
    let records = read_records(&profile.path().join("logs/diagnostics.jsonl"));
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["event"], "queue_overflow");
    assert_eq!(records[0]["dropped"], 30);
    let accepted = encode("session", &json!({"test":"escaped\nline"}), None).unwrap();
    assert_eq!(accepted.iter().filter(|&&byte| byte == b'\n').count(), 1);
    assert!(encode("session", &json!({"test":"x".repeat(LINE_LIMIT)}), None).is_none());
}

#[test]
fn repairs_a_partial_line_and_serializes_writers_by_profile() {
    let profile = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let path = profile.path().join("logs/diagnostics.jsonl");
    drop(LogFiles::open(profile.path()).unwrap());
    fs::write(&path, b"{\"event\":1}\n{\"ev").unwrap();

    let mut writer = LogFiles::open(profile.path()).unwrap();
    assert!(LogFiles::open(profile.path()).is_err());
    assert!(LogFiles::open(second.path()).is_ok());
    writer.append(b"{\"event\":2}\n").unwrap();
    drop(writer);
    assert_eq!(
        read_records(&path),
        vec![json!({"event":1}), json!({"event":2})]
    );
}

#[test]
fn rotation_preserves_exactly_the_newest_logs_and_respects_the_cap() {
    const FILE_LIMIT: usize = 1024 * 1024;
    let profile = tempfile::tempdir().unwrap();
    let mut writer = LogFiles::open(profile.path()).unwrap();
    for marker in b'0'..=b'6' {
        let mut line = vec![marker; FILE_LIMIT];
        line[FILE_LIMIT - 1] = b'\n';
        writer.append(&line).unwrap();
    }
    drop(writer);
    let logs = profile.path().join("logs");
    assert_eq!(fs::read_dir(&logs).unwrap().count(), 6);
    for age in 0..5 {
        let name = if age == 0 {
            "diagnostics.jsonl".to_owned()
        } else {
            format!("diagnostics.{age}.jsonl")
        };
        let content = fs::read(logs.join(name)).unwrap();
        assert_eq!(content.len(), FILE_LIMIT);
        assert_eq!(content[0], b'6' - age);
    }
}

#[test]
fn reopens_after_an_interrupted_rotation_and_bounds_oversize_archives() {
    const FILE_LIMIT: usize = 1024 * 1024;
    let profile = tempfile::tempdir().unwrap();
    let logs = profile.path().join("logs");
    drop(LogFiles::open(profile.path()).unwrap());
    // A crash leaves a gap after moving .3 to .4 and before moving .2 to .3.
    fs::write(logs.join("diagnostics.4.jsonl"), b"{\"event\":1}\n").unwrap();
    fs::write(logs.join("diagnostics.2.jsonl"), b"{\"event\":2}\n").unwrap();
    let path = logs.join("diagnostics.jsonl");
    let mut current = vec![b' '; FILE_LIMIT];
    current[FILE_LIMIT - 1] = b'\n';
    fs::write(&path, &current).unwrap();
    let mut oversize = b"{\"event\":3}\n".to_vec();
    oversize.resize(FILE_LIMIT + 1, b' ');
    fs::write(logs.join("diagnostics.1.jsonl"), oversize).unwrap();

    let mut writer = LogFiles::open(profile.path()).unwrap();
    assert_eq!(
        read_records(&logs.join("diagnostics.1.jsonl")),
        vec![json!({"event":3})]
    );
    writer.append(b"{\"event\":4}\n").unwrap();
    assert_eq!(fs::read(logs.join("diagnostics.1.jsonl")).unwrap(), current);
    assert_eq!(
        read_records(&logs.join("diagnostics.2.jsonl")),
        vec![json!({"event":3})]
    );
    assert_eq!(
        read_records(&logs.join("diagnostics.3.jsonl")),
        vec![json!({"event":2})]
    );
    assert_eq!(
        read_records(&logs.join("diagnostics.4.jsonl")),
        vec![json!({"event":1})]
    );
    assert_eq!(read_records(&path), vec![json!({"event":4})]);
}

#[test]
fn recording_failure_leaves_host_running_and_can_recover() {
    let profile = tempfile::tempdir().unwrap();
    let obstructed = profile.path().join("logs");
    fs::write(&obstructed, "not a directory").unwrap();
    let mut writer = Writer::new(profile.path());
    writer.record(DiagnosticEvent::Startup, Instant::now());
    assert!(writer.files.is_none());
    assert_eq!(fs::read_to_string(&obstructed).unwrap(), "not a directory");
    fs::remove_file(&obstructed).unwrap();
    writer.reopen_at = Instant::now();
    writer.record(DiagnosticEvent::Startup, Instant::now());
    assert_eq!(read_records(&obstructed.join("diagnostics.jsonl")).len(), 1);
}

#[test]
fn rejects_a_hardlinked_log_without_modifying_the_target() {
    let profile = tempfile::tempdir().unwrap();
    let logs = profile.path().join("logs");
    let outside = profile.path().join("outside");
    fs::create_dir(&logs).unwrap();
    fs::write(&outside, "untouched").unwrap();
    fs::hard_link(&outside, logs.join("diagnostics.jsonl")).unwrap();
    assert!(LogFiles::open(profile.path()).is_err());
    assert_eq!(fs::read_to_string(&outside).unwrap(), "untouched");
}

#[cfg(unix)]
#[test]
fn rejects_symlinks_and_uses_private_permissions() {
    use std::os::unix::fs::{MetadataExt, symlink};

    let profile = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let logs = profile.path().join("logs");
    symlink(outside.path(), &logs).unwrap();
    assert!(LogFiles::open(profile.path()).is_err());
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);

    fs::remove_file(&logs).unwrap();
    drop(LogFiles::open(profile.path()).unwrap());
    assert_eq!(fs::metadata(&logs).unwrap().mode() & 0o777, 0o700);
    let active = logs.join("diagnostics.jsonl");
    assert_eq!(fs::metadata(&active).unwrap().mode() & 0o777, 0o600);
    fs::remove_file(&active).unwrap();
    let target = outside.path().join("target");
    fs::write(&target, "untouched").unwrap();
    symlink(&target, &active).unwrap();
    assert!(LogFiles::open(profile.path()).is_err());
    assert_eq!(fs::read_to_string(&target).unwrap(), "untouched");
}
