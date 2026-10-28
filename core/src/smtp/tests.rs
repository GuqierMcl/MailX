use std::sync::{Arc, Mutex};
use std::time::Duration;

use mail_parser::MessageParser;
use tokio::net::TcpListener;

use super::fake_server::{self, Mode, Script, Seen, Stage, commands, saw};
use super::message::{OutgoingMessage, SmtpEnvelope, build};
use super::reply::sanitize;
use super::*;
use crate::accounts::types::ProxyKind;
use crate::drafts::types::{Draft, DraftKind};
use crate::mail::MailAddress;
use crate::net::test_support::{TestTls, dead_port, fake_http_connect, fake_socks5, proxy_route};

const PASSWORD: &str = "hunter2";

fn login(port: u16, tls: TlsMode) -> SmtpLogin {
    SmtpLogin {
        host: "localhost".into(),
        port,
        tls,
        username: "zhang".into(),
        password: SecretValue::new(PASSWORD.into()),
    }
}

fn envelope(recipients: &[&str]) -> SmtpEnvelope {
    SmtpEnvelope {
        from: "zhang@example.com".into(),
        recipients: recipients.iter().map(|r| (*r).to_owned()).collect(),
    }
}

const MIME: &[u8] = b"Subject: hi\r\n\r\nhello\r\n";

fn fast() -> Timeouts {
    Timeouts::for_tests(Duration::from_millis(400))
}

fn tls_for(config: &TestTls) -> TlsSource {
    TlsSource::Test(config.client.clone())
}

fn hook(seen: &Seen) -> DataStarted {
    let seen = seen.clone();
    DataStarted::new(move || async move {
        seen.lock().unwrap().log.push("HOOK".to_owned());
        Ok(())
    })
}

struct Run {
    route: NetRoute,
    login: SmtpLogin,
    envelope: SmtpEnvelope,
    mime: Vec<u8>,
    timeouts: Timeouts,
    tls: TlsSource,
}

impl Run {
    fn new(port: u16, tls: TlsMode) -> Self {
        Self {
            route: NetRoute::default(),
            login: login(port, tls),
            envelope: envelope(&["li@example.org"]),
            mime: MIME.to_vec(),
            timeouts: fast(),
            tls: TlsSource::Platform,
        }
    }

    async fn go(&self, hook: DataStarted) -> SmtpOutcome {
        send_with_tls(
            &self.route,
            &self.login,
            &self.envelope,
            &self.mime,
            &self.timeouts,
            &self.tls,
            hook,
        )
        .await
    }
}

fn plain() -> Script {
    Script::new(Mode::Plain)
}

fn accepted(outcome: &SmtpOutcome) -> bool {
    matches!(outcome, SmtpOutcome::Accepted(reply) if reply.code == 250)
}

fn reason_of(outcome: &SmtpOutcome) -> (bool, FailureReason) {
    match outcome {
        SmtpOutcome::Transient(failure) => (true, failure.reason),
        SmtpOutcome::Permanent(failure) => (false, failure.reason),
        other => panic!("not a pre-data failure: {other:?}"),
    }
}

// Routes

#[tokio::test]
async fn direct_route_delivers() {
    let tls = TestTls::for_names(&["localhost"]);
    let (port, seen) = fake_server::start(Script::new(Mode::Implicit(tls.server.clone()))).await;
    let mut run = Run::new(port, TlsMode::Implicit);
    run.tls = tls_for(&tls);
    let outcome = run.go(hook(&seen)).await;
    assert!(accepted(&outcome), "{outcome:?}");
    assert!(saw(&seen, "MAIL FROM:<zhang@example.com>"));
    assert!(saw(&seen, "RCPT TO:<li@example.org>"));
    assert_eq!(seen.lock().unwrap().body, b"Subject: hi\r\n\r\nhello\r\n");
}

#[tokio::test]
async fn socks5_route_sends_the_unresolved_host_name() {
    let tls = TestTls::for_names(&["smtp.example.test"]);
    let (smtp, seen) = fake_server::start(Script::new(Mode::StartTls(tls.server.clone()))).await;
    let (proxy, targets) = fake_socks5(smtp).await;
    let mut run = Run::new(587, TlsMode::StartTls);
    run.login.host = "smtp.example.test".into();
    run.route = proxy_route(ProxyKind::Socks5, proxy);
    run.tls = tls_for(&tls);
    let outcome = run.go(hook(&seen)).await;
    assert!(accepted(&outcome), "{outcome:?}");
    assert_eq!(*targets.lock().unwrap(), ["smtp.example.test:587"]);
}

#[tokio::test]
async fn http_connect_route_sends_the_unresolved_host_name() {
    let tls = TestTls::for_names(&["smtp.example.test"]);
    let (smtp, seen) = fake_server::start(Script::new(Mode::Implicit(tls.server.clone()))).await;
    let (proxy, requests) = fake_http_connect(smtp).await;
    let mut run = Run::new(465, TlsMode::Implicit);
    run.login.host = "smtp.example.test".into();
    run.route = proxy_route(ProxyKind::HttpConnect, proxy);
    run.tls = tls_for(&tls);
    let outcome = run.go(hook(&seen)).await;
    assert!(accepted(&outcome), "{outcome:?}");
    assert_eq!(
        requests.lock().unwrap()[0],
        "CONNECT smtp.example.test:465 HTTP/1.1"
    );
}

#[tokio::test]
async fn dead_proxy_is_transient_and_never_falls_back_to_a_direct_connection() {
    let direct = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let direct_port = direct.local_addr().unwrap().port();
    let dead = dead_port().await;
    for kind in [ProxyKind::Socks5, ProxyKind::HttpConnect] {
        let mut run = Run::new(direct_port, TlsMode::StartTls);
        run.login.host = "127.0.0.1".into();
        run.route = proxy_route(kind, dead);
        let outcome = run.go(DataStarted::new(|| async { Ok(()) })).await;
        assert_eq!(
            reason_of(&outcome),
            (true, FailureReason::Proxy),
            "{kind:?}"
        );
    }
    let accepted = tokio::time::timeout(Duration::from_millis(300), direct.accept()).await;
    assert!(accepted.is_err(), "a direct connection was attempted");
}

#[tokio::test]
async fn refused_direct_connection_is_transient() {
    let mut run = Run::new(dead_port().await, TlsMode::StartTls);
    run.login.host = "127.0.0.1".into();
    let outcome = run.go(DataStarted::new(|| async { Ok(()) })).await;
    assert_eq!(reason_of(&outcome), (true, FailureReason::Connect));
}

// TLS

#[tokio::test]
async fn starttls_swaps_the_stream_and_delivers() {
    let tls = TestTls::for_names(&["localhost"]);
    let (port, seen) = fake_server::start(Script::new(Mode::StartTls(tls.server.clone()))).await;
    let mut run = Run::new(port, TlsMode::StartTls);
    run.tls = tls_for(&tls);
    let outcome = run.go(hook(&seen)).await;
    assert!(accepted(&outcome), "{outcome:?}");
    let log = commands(&seen);
    let position = |prefix: &str| log.iter().position(|c| c.starts_with(prefix)).unwrap();
    let ehlos: Vec<_> = log
        .iter()
        .enumerate()
        .filter(|(_, c)| c.starts_with("EHLO"))
        .collect();
    assert_eq!(
        ehlos.len(),
        2,
        "EHLO is repeated after the handshake: {log:?}"
    );
    assert!(ehlos[0].0 < position("STARTTLS") && position("STARTTLS") < ehlos[1].0);
    assert!(
        ehlos[1].0 < position("AUTH"),
        "AUTH only after TLS: {log:?}"
    );
    assert_eq!(seen.lock().unwrap().body, MIME);
}

#[tokio::test]
async fn implicit_tls_delivers() {
    let tls = TestTls::for_names(&["localhost"]);
    let (port, seen) = fake_server::start(Script::new(Mode::Implicit(tls.server.clone()))).await;
    let mut run = Run::new(port, TlsMode::Implicit);
    run.tls = tls_for(&tls);
    assert!(accepted(&run.go(hook(&seen)).await));
}

#[tokio::test]
async fn certificate_for_the_wrong_host_fails_permanently_before_auth() {
    let wrong = TestTls::for_names(&["other.example"]);
    for mode in [TlsMode::StartTls, TlsMode::Implicit] {
        let server = match mode {
            TlsMode::StartTls => Mode::StartTls(wrong.server.clone()),
            TlsMode::Implicit => Mode::Implicit(wrong.server.clone()),
        };
        let (port, seen) = fake_server::start(Script::new(server)).await;
        let mut run = Run::new(port, mode);
        run.tls = tls_for(&wrong);
        let outcome = run.go(hook(&seen)).await;
        assert_eq!(
            reason_of(&outcome),
            (false, FailureReason::TlsVerification),
            "{mode:?}"
        );
        assert!(!saw(&seen, "AUTH") && !saw(&seen, "MAIL") && !saw(&seen, "HOOK"));
    }
}

#[tokio::test]
async fn missing_starttls_is_permanent_and_never_continues_in_plaintext() {
    let tls = TestTls::for_names(&["localhost"]);
    let mut script = Script::new(Mode::StartTls(tls.server.clone()));
    script.offer_starttls = false;
    let (port, seen) = fake_server::start(script).await;
    let run = Run::new(port, TlsMode::StartTls);
    let outcome = run.go(hook(&seen)).await;
    assert_eq!(reason_of(&outcome), (false, FailureReason::TlsUnavailable));
    assert!(!saw(&seen, "AUTH") && !saw(&seen, "MAIL") && !saw(&seen, "STARTTLS"));
    // Plain server with no STARTTLS capability at all.
    let (port, seen) = fake_server::start(plain()).await;
    let outcome = Run::new(port, TlsMode::StartTls).go(hook(&seen)).await;
    assert_eq!(reason_of(&outcome), (false, FailureReason::TlsUnavailable));
    assert!(!saw(&seen, "AUTH") && !saw(&seen, "MAIL"));
}

#[tokio::test]
async fn plaintext_injected_after_the_starttls_reply_is_an_error() {
    let tls = TestTls::for_names(&["localhost"]);
    let mut script = Script::new(Mode::StartTls(tls.server.clone()));
    script.inject_after_starttls = b"250 AUTH PLAIN injected\r\n".to_vec();
    let (port, seen) = fake_server::start(script).await;
    let mut run = Run::new(port, TlsMode::StartTls);
    run.tls = tls_for(&tls);
    let outcome = run.go(hook(&seen)).await;
    assert_eq!(reason_of(&outcome), (false, FailureReason::Protocol));
    assert!(!saw(&seen, "AUTH") && !saw(&seen, "MAIL"));
}

// Trust and secrets

#[test]
fn untrusted_or_missing_credentials_are_permanent_pre_data_failures() {
    for error in [PlanError::CredentialUntrusted, PlanError::CredentialMissing] {
        let outcome = plan_failure(&error);
        assert!(matches!(outcome, SmtpOutcome::Permanent(_)), "{error:?}");
    }
    let locked = PlanError::Account(crate::AppError::new(crate::ErrorCode::SecretStoreLocked));
    assert_eq!(
        reason_of(&plan_failure(&locked)),
        (true, FailureReason::SecretStore)
    );
}

mod plan {
    use std::sync::Arc;

    use serde_json::json;
    use tempfile::TempDir;
    use uuid::Uuid;

    use crate::accounts::AccountService;
    use crate::accounts::sync_plan::PlanError;
    use crate::accounts::sync_plan::tests::{config, connection_update};
    use crate::accounts::types::CreateAccountRequest;
    use crate::secrets::test_support::MemorySecrets;

    fn setup() -> (TempDir, AccountService, crate::accounts::types::Account) {
        let directory = TempDir::new().unwrap();
        let mut service =
            AccountService::open(directory.path(), Arc::new(MemorySecrets::default())).unwrap();
        let request: CreateAccountRequest = serde_json::from_value(
            json!({"requestId": Uuid::new_v4().to_string(), "config": config(json!({"kind":"direct"}))}),
        )
        .unwrap();
        let account = service.create(request).unwrap();
        (directory, service, account)
    }

    #[test]
    fn plan_carries_the_smtp_login_and_sender() {
        let (_directory, service, account) = setup();
        let plan = service.smtp_plan(&account.id).unwrap();
        assert_eq!(
            (
                plan.login.host.as_str(),
                plan.login.port,
                plan.login.username.as_str()
            ),
            ("smtp.example.com", 465, "lin")
        );
        assert_eq!(plan.login.password.expose_secret(), "mail-secret");
        assert_eq!(plan.sender_address, "lin@example.com");
        assert!(plan.route.proxy.is_none());
        assert!(!format!("{plan:?}").contains("mail-secret"));
    }

    #[test]
    fn a_changed_smtp_server_does_not_receive_the_old_password() {
        let (_directory, mut service, account) = setup();
        let mut value = config(json!({"kind":"direct"}));
        value["authentication"]["password"] = json!({"action":"keep"});
        value["smtp"]["host"] = json!("smtp.attacker.example");
        service
            .update_connection(connection_update(&account.id, account.revision, value))
            .unwrap();
        assert!(matches!(
            service.smtp_plan(&account.id),
            Err(PlanError::CredentialUntrusted)
        ));
    }
}

use crate::accounts::sync_plan::PlanError;

async fn plain_via_starttls(script_change: impl FnOnce(&mut Script)) -> (Run, Seen, TestTls) {
    let tls = TestTls::for_names(&["localhost"]);
    let mut script = Script::new(Mode::StartTls(tls.server.clone()));
    script_change(&mut script);
    let (port, seen) = fake_server::start(script).await;
    let mut run = Run::new(port, TlsMode::StartTls);
    run.tls = tls_for(&tls);
    (run, seen, tls)
}

#[tokio::test]
async fn auth_plain_and_login_both_work() {
    for (mechanisms, expected) in [
        (vec!["PLAIN", "LOGIN"], "AUTHOK:PLAIN"),
        (vec!["LOGIN"], "AUTHOK:LOGIN"),
    ] {
        let (run, seen, _tls) = plain_via_starttls(|s| s.auth = mechanisms).await;
        let outcome = run.go(hook(&seen)).await;
        assert!(accepted(&outcome), "{outcome:?}");
        assert!(saw(&seen, expected), "{:?}", commands(&seen));
    }
}

#[tokio::test]
async fn rejected_credentials_are_permanent_and_the_password_never_surfaces() {
    let (mut run, seen, _tls) = plain_via_starttls(|s| s.password = "different").await;
    run.login.password = SecretValue::new("hunter2-wrong".into());
    let outcome = run.go(hook(&seen)).await;
    assert_eq!(reason_of(&outcome), (false, FailureReason::AuthRejected));
    assert!(!saw(&seen, "MAIL"));
    // The fake echoes the submitted password in its 535 text.
    for text in [format!("{outcome:?}"), format!("{outcome}")] {
        assert!(!text.contains("hunter2-wrong"), "{text}");
    }
    assert!(!format!("{:?}", run.login).contains("hunter2"));
}

#[tokio::test]
async fn no_advertised_auth_is_a_distinct_permanent_failure() {
    let (run, seen, _tls) = plain_via_starttls(|s| s.auth.clear()).await;
    let outcome = run.go(hook(&seen)).await;
    assert_eq!(reason_of(&outcome), (false, FailureReason::AuthUnavailable));
    assert!(!saw(&seen, "AUTH") && !saw(&seen, "MAIL"));
}

#[test]
fn server_text_is_sanitized_limited_and_masked() {
    let secret = SecretValue::new("hunter2".into());
    let long = format!("550 line1\r\nline2\t\u{7}hunter2 {}", "x".repeat(500));
    let clean = sanitize(&long, &secret);
    assert!(clean.chars().count() <= 200);
    assert!(!clean.contains(['\r', '\n', '\t', '\u{7}']));
    assert!(!clean.contains("hunter2"));
}

// Pre-data protocol outcomes

#[tokio::test]
async fn size_over_the_advertised_limit_stops_before_mail_from() {
    let (run, seen, _tls) = plain_via_starttls(|s| s.size = Some(10)).await;
    let outcome = run.go(hook(&seen)).await;
    assert_eq!(reason_of(&outcome), (false, FailureReason::MessageTooLarge));
    assert!(!saw(&seen, "MAIL") && !saw(&seen, "HOOK"));
}

#[tokio::test]
async fn size_within_the_limit_is_declared_on_mail_from() {
    let (run, seen, _tls) = plain_via_starttls(|s| s.size = Some(1_000_000)).await;
    assert!(accepted(&run.go(hook(&seen)).await));
    assert!(saw(
        &seen,
        &format!("MAIL FROM:<zhang@example.com> SIZE={}", MIME.len())
    ));
}

#[tokio::test]
async fn rejected_recipients_are_all_reported_and_data_is_never_sent() {
    let (mut run, seen, _tls) = plain_via_starttls(|s| {
        s.reject_rcpt = vec!["b@example.org", "c@example.org"];
    })
    .await;
    run.envelope = envelope(&["a@example.org", "b@example.org", "c@example.org"]);
    let outcome = run.go(hook(&seen)).await;
    let SmtpOutcome::RecipientsRejected(list) = &outcome else {
        panic!("{outcome:?}")
    };
    let addresses: Vec<_> = list.iter().map(|r| (r.address.as_str(), r.code)).collect();
    assert_eq!(addresses, [("b@example.org", 550), ("c@example.org", 550)]);
    let log = commands(&seen);
    assert_eq!(log.iter().filter(|c| c.starts_with("RCPT")).count(), 3);
    assert!(saw(&seen, "RSET"));
    assert!(!saw(&seen, "DATA") && !saw(&seen, "HOOK") && !saw(&seen, "BODY"));
}

#[tokio::test]
async fn greeting_and_stage_replies_map_to_transient_or_permanent() {
    let cases: [(Stage, &'static str, bool); 10] = [
        (Stage::Greeting, "421 busy", true),
        (Stage::Greeting, "554 go away", false),
        (Stage::Ehlo, "451 later", true),
        (Stage::Ehlo, "550 no", false),
        (Stage::Auth, "454 temporary", true),
        (Stage::Mail, "451 4.3.0 try later", true),
        (Stage::Mail, "550 5.7.1 sender refused", false),
        (Stage::Data, "451 4.3.0 later", true),
        (Stage::Data, "554 no", false),
        (Stage::Rcpt, "250 fine", true),
    ];
    for (stage, line, is_transient) in cases {
        if stage == Stage::Rcpt {
            continue;
        }
        let (run, seen, _tls) = plain_via_starttls(|s| *s = s.clone().reply(stage, line)).await;
        let outcome = run.go(hook(&seen)).await;
        assert_eq!(
            reason_of(&outcome).0,
            is_transient,
            "{stage:?} {line}: {outcome:?}"
        );
        assert!(!saw(&seen, "BODY") && !saw(&seen, "HOOK"), "{stage:?}");
        let (SmtpOutcome::Transient(failure) | SmtpOutcome::Permanent(failure)) = &outcome else {
            unreachable!()
        };
        assert!(failure.code.is_some());
    }
}

#[tokio::test]
async fn stalls_before_data_are_transient_timeouts() {
    for stage in [
        Stage::Greeting,
        Stage::Ehlo,
        Stage::Mail,
        Stage::Rcpt,
        Stage::Data,
    ] {
        let (run, seen, _tls) = plain_via_starttls(|s| s.stall = Some(stage)).await;
        let outcome = run.go(hook(&seen)).await;
        assert_eq!(
            reason_of(&outcome),
            (true, FailureReason::Timeout),
            "{stage:?}"
        );
        assert!(!saw(&seen, "BODY") && !saw(&seen, "HOOK"));
    }
}

// Data phase

#[tokio::test]
async fn the_data_mark_is_written_before_the_first_body_byte() {
    let (run, seen, _tls) = plain_via_starttls(|_| {}).await;
    let calls = Arc::new(Mutex::new(0));
    let counter = calls.clone();
    let probe = seen.clone();
    let outcome = run
        .go(DataStarted::new(move || async move {
            *counter.lock().unwrap() += 1;
            // Give a misordered implementation time to write the body first.
            tokio::time::sleep(Duration::from_millis(100)).await;
            probe.lock().unwrap().log.push("HOOK".into());
            Ok(())
        }))
        .await;
    assert!(accepted(&outcome));
    assert_eq!(*calls.lock().unwrap(), 1);
    let log = commands(&seen);
    let hook_at = log.iter().position(|c| c == "HOOK").unwrap();
    let body_at = log.iter().position(|c| c == "BODY").unwrap();
    let data_at = log.iter().position(|c| c == "DATA").unwrap();
    assert!(data_at < hook_at && hook_at < body_at, "{log:?}");
}

#[tokio::test]
async fn a_failing_data_mark_sends_no_body() {
    let (run, seen, _tls) = plain_via_starttls(|_| {}).await;
    let outcome = run.go(DataStarted::new(|| async { Err(()) })).await;
    assert_eq!(reason_of(&outcome), (true, FailureReason::DataMarkFailed));
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!saw(&seen, "BODY"));
    assert!(seen.lock().unwrap().body.is_empty());
}

#[tokio::test]
async fn final_replies_are_classified_after_data() {
    let (run, seen, _tls) = plain_via_starttls(|_| {}).await;
    let outcome = run.go(hook(&seen)).await;
    assert_eq!(
        outcome,
        SmtpOutcome::Accepted(ServerReply {
            code: 250,
            text: "2.0.0 queued as 42".into()
        })
    );
    assert!(saw(&seen, "QUIT"));

    for (line, code) in [("550 5.7.1 spam", 550), ("452 4.3.1 full", 452)] {
        let (run, seen, _tls) =
            plain_via_starttls(|s| *s = s.clone().reply(Stage::Final, line)).await;
        let outcome = run.go(hook(&seen)).await;
        assert!(
            matches!(&outcome, SmtpOutcome::RejectedAfterData(reply) if reply.code == code),
            "{outcome:?}"
        );
    }
}

#[tokio::test]
async fn dropped_connection_after_the_body_is_unknown() {
    let (run, seen, _tls) = plain_via_starttls(|s| s.drop_after_body = true).await;
    let calls = Arc::new(Mutex::new(0));
    let counter = calls.clone();
    let probe = seen.clone();
    let outcome = run
        .go(DataStarted::new(move || async move {
            *counter.lock().unwrap() += 1;
            probe.lock().unwrap().log.push("HOOK".into());
            Ok(())
        }))
        .await;
    assert_eq!(outcome, SmtpOutcome::Unknown(UnknownReason::ConnectionLost));
    assert_eq!(*calls.lock().unwrap(), 1);
    let log = commands(&seen);
    let hook_at = log.iter().position(|c| c == "HOOK").unwrap();
    let body_at = log.iter().position(|c| c == "BODY").unwrap();
    assert!(hook_at < body_at, "{log:?}");
}

#[tokio::test]
async fn waiting_for_the_final_reply_times_out_as_unknown() {
    let (run, seen, _tls) = plain_via_starttls(|s| s.stall = Some(Stage::Final)).await;
    let outcome = run.go(hook(&seen)).await;
    assert_eq!(outcome, SmtpOutcome::Unknown(UnknownReason::Timeout));
}

#[tokio::test]
async fn an_unparsable_final_reply_is_unknown() {
    let (run, seen, _tls) =
        plain_via_starttls(|s| *s = s.clone().reply(Stage::Final, "garbage")).await;
    let outcome = run.go(hook(&seen)).await;
    assert_eq!(outcome, SmtpOutcome::Unknown(UnknownReason::MalformedReply));
}

#[tokio::test]
async fn dot_lines_are_stuffed_and_line_endings_normalized() {
    let (mut run, seen, _tls) = plain_via_starttls(|_| {}).await;
    run.mime = b"Subject: dots\r\n\r\n.hidden\nplain\r\n..two\rlast".to_vec();
    assert!(accepted(&run.go(hook(&seen)).await));
    assert_eq!(
        seen.lock().unwrap().body,
        b"Subject: dots\r\n\r\n.hidden\r\nplain\r\n..two\r\nlast\r\n"
    );
}

#[test]
fn data_payload_is_stuffed_crlf_and_terminated() {
    assert_eq!(data_payload(b"a\n.b\r\n.\r\n"), b"a\r\n..b\r\n..\r\n.\r\n");
    assert_eq!(data_payload(b""), b".\r\n");
    assert_eq!(data_payload(b"x"), b"x\r\n.\r\n");
}

#[tokio::test]
async fn a_built_message_survives_the_session_and_parses_back() {
    let draft = Draft {
        id: "d".into(),
        account_id: "a".into(),
        kind: DraftKind::New,
        source_email_id: None,
        to: vec![MailAddress {
            name: Some("李四".into()),
            address: "li@example.org".into(),
        }],
        cc: vec![],
        bcc: vec![MailAddress {
            name: None,
            address: "hidden@example.org".into(),
        }],
        subject: "你好".into(),
        body_html: "<p>.点开头</p><p>正文</p><p></p>".into(),
        body_text: ".点开头\n正文\n".into(),
        in_reply_to: None,
        references: None,
        created_at: String::new(),
        updated_at: String::new(),
        revision: 1,
        attachments: vec![],
    };
    let id = super::message::new_message_id("zhang@example.com").unwrap();
    let built = build(&OutgoingMessage {
        sender_address: "zhang@example.com",
        sender_name: Some("张三"),
        draft: &draft,
        message_id: &id,
        date: super::message::local_now(),
        attachments: &[],
    })
    .unwrap();
    let (mut run, seen, _tls) = plain_via_starttls(|_| {}).await;
    run.envelope = built.envelope.clone();
    run.mime = built.mime.clone();
    assert!(accepted(&run.go(hook(&seen)).await));
    let log = commands(&seen);
    assert_eq!(log.iter().filter(|c| c.starts_with("RCPT")).count(), 2);
    let body = seen.lock().unwrap().body.clone();
    assert_eq!(
        body, built.mime,
        "the server received exactly the stored bytes"
    );
    let parsed = MessageParser::default().parse(&body).unwrap();
    assert_eq!(parsed.subject(), Some("你好"));
    assert_eq!(parsed.body_text(0).unwrap(), ".点开头\r\n正文\r\n");
}

#[test]
fn outcome_types_never_print_the_password() {
    let login = login(25, TlsMode::StartTls);
    assert!(!format!("{login:?}").contains(PASSWORD));
    let hook = DataStarted::new(|| async { Ok(()) });
    assert_eq!(format!("{hook:?}"), "DataStarted");
}

#[tokio::test]
async fn verification_authenticates_and_quits_without_any_delivery_commands() {
    for starttls in [false, true] {
        let tls = TestTls::for_names(&["localhost"]);
        let mode = if starttls {
            Mode::StartTls(tls.server.clone())
        } else {
            Mode::Implicit(tls.server.clone())
        };
        let (port, seen) = fake_server::start(Script::new(mode)).await;
        let login = login(
            port,
            if starttls {
                TlsMode::StartTls
            } else {
                TlsMode::Implicit
            },
        );
        verify_connection(&NetRoute::default(), &login, &fast(), &tls_for(&tls))
            .await
            .unwrap();
        let log = commands(&seen);
        assert!(saw(&seen, "AUTH") && saw(&seen, "QUIT"), "{log:?}");
        assert!(
            !log.iter().any(|command| ["MAIL", "RCPT", "DATA", "BODY"]
                .iter()
                .any(|verb| command.starts_with(verb))),
            "{log:?}"
        );
        assert!(seen.lock().unwrap().body.is_empty());
    }
}
