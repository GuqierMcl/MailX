use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD};
use rusqlite::Connection;
use tempfile::TempDir;
use tokio::sync::Notify;

use super::*;
use crate::accounts::MAX_SECRET_BYTES;
use crate::secrets::test_support::MemorySecrets;

const KEY: &str = "sk-service-test-key";
const ENGLISH: &str = "The quarterly report is attached to this message. Please review the numbers carefully before our meeting on Thursday, and let me know if anything looks wrong or if you need more details from the finance team.";
const CHINESE: &str = "你好，这是一封用于测试语言检测的邮件，请您在周四的会议之前仔细阅读附件中的季度报告，如有任何问题请及时与我联系，谢谢您的配合。";

fn mail(html: &str) -> Vec<u8> {
    format!(
        "MIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{}\r\n",
        STANDARD.encode(html)
    )
    .into_bytes()
}

struct Fixture {
    _directory: TempDir,
    core: MailxCore,
    ids: Vec<String>,
    db: Connection,
    secrets: Arc<MemorySecrets>,
}

fn fixture(htmls: &[&str]) -> Fixture {
    let directory = TempDir::new().unwrap();
    let secrets = Arc::new(MemorySecrets::default());
    let raws: Vec<Vec<u8>> = htmls.iter().map(|html| mail(html)).collect();
    let refs: Vec<&[u8]> = raws.iter().map(Vec::as_slice).collect();
    let (core, ids) = MailxCore::body_test_fixture_with_secrets(
        directory.path(),
        tokio::runtime::Handle::current(),
        &refs,
        secrets.clone(),
    )
    .unwrap();
    let db = Connection::open(directory.path().join("mailx.sqlite3")).unwrap();
    Fixture {
        _directory: directory,
        core,
        ids,
        db,
        secrets,
    }
}

fn settings(provider: TranslationProviderId) -> TranslationSettings {
    TranslationSettings {
        provider: Some(provider),
        consented_provider: Some(provider),
        target_language: "de".into(),
        ..TranslationSettings::default()
    }
}

async fn with_key(f: &Fixture, provider: TranslationProviderId) {
    f.core
        .set_translation_key(provider, SecretValue::new(KEY.into()))
        .await
        .unwrap();
}

type Calls = Arc<Mutex<Vec<TestCall>>>;

fn translated(text: &str) -> String {
    text.replace("Hello", "Hallo")
        .replace("Synthetic fixture", "Synthetisch")
}

/// Answers instantly with a recognizable "translation".
fn echo(f: &Fixture) -> Calls {
    let calls: Calls = Arc::default();
    let record = calls.clone();
    f.core
        .translation
        .set_backend(Arc::new(move |call: TestCall| {
            let response = TranslateResponse {
                texts: call.texts.iter().map(|text| translated(text)).collect(),
                detected_source: Some("en".into()),
            };
            record.lock().unwrap().push(call);
            Box::pin(async move { Ok(response) })
        }));
    calls
}

fn stats_count(f: &Fixture) -> i64 {
    f.db.query_row("SELECT count(*) FROM message_translations", [], |row| {
        row.get(0)
    })
    .unwrap()
}

async fn folded_html(f: &Fixture, index: usize) -> String {
    let document = f.core.body_document(f.ids[index].clone()).await.unwrap();
    render::fold_quotes(&document.html, FOLD_LABEL)
}

async fn translate_to_de(
    f: &Fixture,
    index: usize,
    s: &TranslationSettings,
) -> Result<TranslationStatus, AppError> {
    f.core
        .translate_email(f.ids[index].clone(), s.clone(), "en".into(), None)
        .await
}

fn code(result: Result<impl std::fmt::Debug, AppError>) -> ErrorCode {
    result.expect_err("expected an error").code
}

// ----------------------------------------------------------- preconditions

#[tokio::test]
async fn preconditions_fail_in_the_documented_order() {
    let f = fixture(&["<p>Hello</p>"]);
    let calls = echo(&f);
    // The default is a keyless service: it only waits for consent.
    let mut s = TranslationSettings::default();
    assert_eq!(
        code(translate_to_de(&f, 0, &s).await),
        ErrorCode::TranslationConsentRequired
    );

    s.provider = None;
    assert_eq!(
        code(translate_to_de(&f, 0, &s).await),
        ErrorCode::TranslationNotConfigured
    );

    s.provider = Some(TranslationProviderId::Deepl);
    assert_eq!(
        code(translate_to_de(&f, 0, &s).await),
        ErrorCode::TranslationNotConfigured
    );

    with_key(&f, TranslationProviderId::Deepl).await;
    assert_eq!(
        code(translate_to_de(&f, 0, &s).await),
        ErrorCode::TranslationConsentRequired
    );

    s.consented_provider = Some(TranslationProviderId::Deepl);
    let bad_target = f
        .core
        .translate_email(f.ids[0].clone(), s.clone(), "en".into(), Some("xx".into()))
        .await
        .unwrap_err();
    assert_eq!(
        (bad_target.code, bad_target.field.as_deref()),
        (ErrorCode::Validation, Some("target"))
    );

    f.db.execute("UPDATE messages SET body_state = 'missing'", [])
        .unwrap();
    assert_eq!(
        code(translate_to_de(&f, 0, &s).await),
        ErrorCode::BodyNotCached
    );
    f.db.execute("UPDATE messages SET body_state = 'ready'", [])
        .unwrap();

    f.db.execute(
        "UPDATE message_bodies SET sanitized_html = ?1",
        [format!("<p>{}</p>", "a".repeat(MAX_HTML_BYTES))],
    )
    .unwrap();
    assert_eq!(
        code(translate_to_de(&f, 0, &s).await),
        ErrorCode::TranslationTooLarge
    );

    assert!(
        calls.lock().unwrap().is_empty(),
        "no failed check may reach the service"
    );
    assert_eq!(stats_count(&f), 0);
}

#[tokio::test]
async fn a_service_without_consent_is_refused_after_switching_providers() {
    let f = fixture(&["<p>Hello</p>"]);
    let calls = echo(&f);
    with_key(&f, TranslationProviderId::Deepl).await;
    with_key(&f, TranslationProviderId::Google).await;
    let mut s = settings(TranslationProviderId::Deepl);
    translate_to_de(&f, 0, &s).await.unwrap();
    s.provider = Some(TranslationProviderId::Google);
    assert_eq!(
        code(translate_to_de(&f, 0, &s).await),
        ErrorCode::TranslationConsentRequired
    );
    assert_eq!(calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn invalid_settings_are_rejected_before_anything_else() {
    let f = fixture(&["<p>Hello</p>"]);
    let mut s = settings(TranslationProviderId::Deepl);
    s.target_language = "xx".into();
    let error = translate_to_de(&f, 0, &s).await.unwrap_err();
    assert_eq!(error.field.as_deref(), Some("translation.targetLanguage"));
}

// ---------------------------------------------------------------- happy path

#[tokio::test]
async fn a_translation_is_stored_and_rebuilds_the_document() {
    let f = fixture(&["<p>Hello <b>big</b> world</p><p>Second</p>"]);
    let calls = echo(&f);
    with_key(&f, TranslationProviderId::Deepl).await;
    let s = settings(TranslationProviderId::Deepl);
    let status = translate_to_de(&f, 0, &s).await.unwrap();
    assert_eq!(status.target_language, "de");
    let info = status.translation.unwrap();
    assert_eq!(info.provider, TranslationProviderId::Deepl);
    assert_eq!(info.source_language.as_deref(), Some("en"));
    assert!(!info.partial);
    assert_eq!(stats_count(&f), 1);

    {
        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].target, "de");
        assert_eq!(calls[0].key, KEY);
        assert_eq!(calls[0].proxy, None);
    }

    let document = f.core.body_document(f.ids[0].clone()).await.unwrap();
    let segments = f
        .core
        .body_translation(&document.binding, "de".into(), false)
        .await
        .unwrap()
        .unwrap();
    let html = translate::apply(&folded_html(&f, 0).await, false, &segments).unwrap();
    assert!(html.contains("Hallo <b>big</b> world"), "{html}");
    assert!(html.contains("Second"));
    // Another target or quote setting has no translation of its own.
    for (target, quotes) in [("fr", false), ("de", true)] {
        assert!(
            f.core
                .body_translation(&document.binding, target.into(), quotes)
                .await
                .unwrap()
                .is_none()
        );
    }
    let invalid = f
        .core
        .body_translation(&document.binding, "xx".into(), false)
        .await;
    assert_eq!(code(invalid), ErrorCode::Validation);
}

#[tokio::test]
async fn the_subject_is_translated_as_plain_text_in_the_first_request() {
    let f = fixture(&["<p>Hello</p>"]);
    let calls = echo(&f);
    with_key(&f, TranslationProviderId::Deepl).await;
    let s = settings(TranslationProviderId::Deepl);
    let status = translate_to_de(&f, 0, &s).await.unwrap();
    assert_eq!(
        status.translation.unwrap().subject.as_deref(),
        Some("Synthetisch")
    );
    assert_eq!(calls.lock().unwrap()[0].texts[0], "Synthetic fixture");

    // An empty subject is skipped, and one with markup characters is escaped.
    f.db.execute("UPDATE messages SET subject = ''", [])
        .unwrap();
    calls.lock().unwrap().clear();
    let status = translate_to_de(&f, 0, &s).await.unwrap();
    assert_eq!(status.translation.unwrap().subject, None);
    assert_eq!(calls.lock().unwrap()[0].texts, ["Hello"]);

    f.db.execute("UPDATE messages SET subject = 'a < b & c'", [])
        .unwrap();
    calls.lock().unwrap().clear();
    translate_to_de(&f, 0, &s).await.unwrap();
    assert_eq!(calls.lock().unwrap()[0].texts[0], "a &lt; b &amp; c");
}

#[tokio::test]
async fn identical_segments_are_sent_once_and_translated_everywhere() {
    let f = fixture(&["<p>Hello</p><p>Other</p><p>Hello</p><p>Hello</p>"]);
    let calls = echo(&f);
    with_key(&f, TranslationProviderId::Google).await;
    let s = settings(TranslationProviderId::Google);
    translate_to_de(&f, 0, &s).await.unwrap();
    let sent: Vec<String> = calls
        .lock()
        .unwrap()
        .iter()
        .flat_map(|c| c.texts.clone())
        .collect();
    assert_eq!(
        sent.iter().filter(|text| *text == "Hello").count(),
        1,
        "{sent:?}"
    );
    let document = f.core.body_document(f.ids[0].clone()).await.unwrap();
    let segments = f
        .core
        .body_translation(&document.binding, "de".into(), false)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        segments,
        [
            Some("Hallo".into()),
            Some("Other".into()),
            Some("Hallo".into()),
            Some("Hallo".into())
        ]
    );
}

#[tokio::test]
async fn text_over_the_budget_stays_untranslated_and_marks_the_result_partial() {
    let paragraphs: String = (0..60)
        .map(|n| format!("<p>Hello {n} {}</p>", "word ".repeat(200)))
        .collect();
    // One paragraph repeated at the end still gets its answer after the cutoff.
    let html = format!("{paragraphs}<p>Hello 0 {}</p>", "word ".repeat(200));
    let f = fixture(&[&html]);
    let calls = echo(&f);
    with_key(&f, TranslationProviderId::Deepl).await;
    let s = settings(TranslationProviderId::Deepl);
    let status = translate_to_de(&f, 0, &s).await.unwrap();
    assert!(status.translation.unwrap().partial);
    let document = f.core.body_document(f.ids[0].clone()).await.unwrap();
    let segments = f
        .core
        .body_translation(&document.binding, "de".into(), false)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(segments.len(), 61);
    let done = segments.iter().filter(|s| s.is_some()).count();
    // The budget is checked before each paragraph (about 1,000 characters each).
    assert_eq!(done, 50 + 1, "50 unique paragraphs plus the repeat");
    assert!(segments[..50].iter().all(Option::is_some));
    assert!(segments[50..60].iter().all(Option::is_none));
    assert!(segments[60].is_some());

    // Requests respect the service's limits.
    let limits = provider::limits(TranslationProviderId::Deepl);
    let calls = calls.lock().unwrap();
    assert!(calls.len() > 1);
    for call in calls.iter() {
        assert!(call.texts.len() <= limits.max_items);
        assert!(call.texts.iter().map(|t| t.len()).sum::<usize>() <= limits.max_bytes);
    }
}

#[tokio::test]
async fn the_route_comes_from_the_translation_settings() {
    let f = fixture(&["<p>Hello</p>"]);
    let calls = echo(&f);
    with_key(&f, TranslationProviderId::Microsoft).await;
    let mut s = settings(TranslationProviderId::Microsoft);
    s.microsoft_region = "westeurope".into();
    s.route = TranslationRoute::Proxy {
        proxy_kind: crate::accounts::types::ProxyKind::Socks5,
        host: "proxy.example".into(),
        port: 1080,
        username: String::new(),
    };
    translate_to_de(&f, 0, &s).await.unwrap();
    let calls = calls.lock().unwrap();
    assert_eq!(calls[0].proxy, Some(("proxy.example".into(), 1080)));
    assert_eq!(calls[0].proxy_auth, None);
    assert_eq!(calls[0].region, "westeurope");
    assert_eq!(calls[0].provider, TranslationProviderId::Microsoft);
}

#[tokio::test]
async fn a_failed_request_writes_nothing() {
    let f = fixture(&["<p>Hello</p>"]);
    f.core.translation.set_backend(Arc::new(|_| {
        Box::pin(async { Err(ErrorCode::TranslationQuotaExceeded) })
    }));
    with_key(&f, TranslationProviderId::Deepl).await;
    let result = translate_to_de(&f, 0, &settings(TranslationProviderId::Deepl)).await;
    assert_eq!(code(result), ErrorCode::TranslationQuotaExceeded);
    assert_eq!(stats_count(&f), 0);
    // A short answer is as bad as an error.
    f.core.translation.set_backend(Arc::new(|_| {
        Box::pin(async {
            Ok(TranslateResponse {
                texts: Vec::new(),
                detected_source: None,
            })
        })
    }));
    let result = translate_to_de(&f, 0, &settings(TranslationProviderId::Deepl)).await;
    assert_eq!(code(result), ErrorCode::TranslationInvalidResponse);
    assert_eq!(stats_count(&f), 0);
}

#[tokio::test]
async fn translating_again_overwrites_the_row() {
    let f = fixture(&["<p>Hello</p>"]);
    echo(&f);
    with_key(&f, TranslationProviderId::Deepl).await;
    let s = settings(TranslationProviderId::Deepl);
    translate_to_de(&f, 0, &s).await.unwrap();
    f.core.translation.set_backend(Arc::new(|call: TestCall| {
        let texts = call.texts.iter().map(|_| "Neu".to_owned()).collect();
        Box::pin(async move {
            Ok(TranslateResponse {
                texts,
                detected_source: None,
            })
        })
    }));
    translate_to_de(&f, 0, &s).await.unwrap();
    assert_eq!(stats_count(&f), 1);
    let document = f.core.body_document(f.ids[0].clone()).await.unwrap();
    let segments = f
        .core
        .body_translation(&document.binding, "de".into(), false)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(segments, [Some("Neu".into())]);
}

#[tokio::test]
async fn a_row_is_stale_once_the_body_or_the_segmenter_changes() {
    let f = fixture(&["<p>Hello</p>"]);
    echo(&f);
    with_key(&f, TranslationProviderId::Deepl).await;
    let s = settings(TranslationProviderId::Deepl);
    translate_to_de(&f, 0, &s).await.unwrap();
    let status = |f: &Fixture, s: &TranslationSettings| {
        let (core, id, s) = (f.core.clone(), f.ids[0].clone(), s.clone());
        async move { core.translation_status(id, s, "en".into()).await.unwrap() }
    };
    assert!(status(&f, &s).await.translation.is_some());
    let document = f.core.body_document(f.ids[0].clone()).await.unwrap();

    f.db.execute(
        "UPDATE message_translations SET segmenter_version = segmenter_version + 1",
        [],
    )
    .unwrap();
    assert!(status(&f, &s).await.translation.is_none());
    assert!(
        f.core
            .body_translation(&document.binding, "de".into(), false)
            .await
            .unwrap()
            .is_none()
    );
    f.db.execute(
        "UPDATE message_translations SET segmenter_version = segmenter_version - 1",
        [],
    )
    .unwrap();
    assert!(status(&f, &s).await.translation.is_some());

    f.db.execute(
        "UPDATE message_bodies SET raw_sha256 = ?1",
        ["0".repeat(64)],
    )
    .unwrap();
    assert!(status(&f, &s).await.translation.is_none());
}

#[tokio::test]
async fn a_body_that_changes_mid_flight_is_not_cached() {
    let f = fixture(&["<p>Hello</p>"]);
    let db = Mutex::new(Connection::open(f._directory.path().join("mailx.sqlite3")).unwrap());
    f.core
        .translation
        .set_backend(Arc::new(move |call: TestCall| {
            db.lock()
                .unwrap()
                .execute(
                    "UPDATE message_bodies SET raw_sha256 = ?1",
                    ["1".repeat(64)],
                )
                .unwrap();
            let texts = call.texts;
            Box::pin(async move {
                Ok(TranslateResponse {
                    texts,
                    detected_source: None,
                })
            })
        }));
    with_key(&f, TranslationProviderId::Deepl).await;
    let result = translate_to_de(&f, 0, &settings(TranslationProviderId::Deepl)).await;
    assert_eq!(code(result), ErrorCode::BodyNotCached);
    assert_eq!(stats_count(&f), 0);
}

// ------------------------------------------------------- cancel / concurrency

/// Waits for `release`; records when its future is dropped.
fn blocking_backend(
    started: Arc<AtomicUsize>,
    dropped: Arc<AtomicBool>,
    release: Arc<Notify>,
) -> TestBackend {
    struct OnDrop(Arc<AtomicBool>);
    impl Drop for OnDrop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    Arc::new(move |call: TestCall| {
        started.fetch_add(1, Ordering::SeqCst);
        let (guard, release) = (OnDrop(dropped.clone()), release.clone());
        Box::pin(async move {
            let _guard = guard;
            release.notified().await;
            Ok(TranslateResponse {
                texts: call.texts,
                detected_source: None,
            })
        })
    })
}

async fn until(condition: impl Fn() -> bool) {
    for _ in 0..500 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("condition not reached");
}

#[tokio::test]
async fn cancelling_stops_the_request_and_writes_nothing() {
    let f = fixture(&["<p>Hello</p>"]);
    let (started, dropped, release) = (
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicBool::new(false)),
        Arc::new(Notify::new()),
    );
    f.core
        .translation
        .set_backend(blocking_backend(started.clone(), dropped.clone(), release));
    with_key(&f, TranslationProviderId::Deepl).await;
    let s = settings(TranslationProviderId::Deepl);
    let task = tokio::spawn({
        let (core, id) = (f.core.clone(), f.ids[0].clone());
        async move { core.translate_email(id, s, "en".into(), None).await }
    });
    until(|| started.load(Ordering::SeqCst) == 1).await;
    f.core.cancel_translation(&f.ids[0]);
    assert_eq!(code(task.await.unwrap()), ErrorCode::TranslationCancelled);
    assert!(
        dropped.load(Ordering::SeqCst),
        "the service call must be dropped with the translation"
    );
    assert_eq!(stats_count(&f), 0);
    // Cancelling when nothing runs is harmless.
    f.core.cancel_translation(&f.ids[0]);
}

#[tokio::test]
async fn a_second_translation_of_the_same_message_cancels_the_first() {
    let f = fixture(&["<p>Hello</p>"]);
    let started = Arc::new(AtomicUsize::new(0));
    let counter = started.clone();
    f.core
        .translation
        .set_backend(Arc::new(move |call: TestCall| {
            let first = counter.fetch_add(1, Ordering::SeqCst) == 0;
            Box::pin(async move {
                if first {
                    std::future::pending::<()>().await;
                }
                Ok(TranslateResponse {
                    texts: call.texts,
                    detected_source: None,
                })
            })
        }));
    with_key(&f, TranslationProviderId::Deepl).await;
    let s = settings(TranslationProviderId::Deepl);
    let first = tokio::spawn({
        let (core, id, s) = (f.core.clone(), f.ids[0].clone(), s.clone());
        async move { core.translate_email(id, s, "en".into(), None).await }
    });
    until(|| started.load(Ordering::SeqCst) == 1).await;
    let second = translate_to_de(&f, 0, &s).await.unwrap();
    assert!(second.translation.is_some());
    assert_eq!(code(first.await.unwrap()), ErrorCode::TranslationCancelled);
    assert_eq!(stats_count(&f), 1);
}

#[tokio::test]
async fn at_most_two_translations_run_at_once_and_waiting_ones_can_be_cancelled() {
    let f = fixture(&["<p>One</p>", "<p>Two</p>", "<p>Three</p>"]);
    let (started, dropped, release) = (
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicBool::new(false)),
        Arc::new(Notify::new()),
    );
    f.core
        .translation
        .set_backend(blocking_backend(started.clone(), dropped, release.clone()));
    with_key(&f, TranslationProviderId::Deepl).await;
    let s = settings(TranslationProviderId::Deepl);
    let spawn = |index: usize| {
        let (core, id, s) = (f.core.clone(), f.ids[index].clone(), s.clone());
        tokio::spawn(async move { core.translate_email(id, s, "en".into(), None).await })
    };
    let (a, b, c) = (spawn(0), spawn(1), spawn(2));
    until(|| started.load(Ordering::SeqCst) == 2).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        started.load(Ordering::SeqCst),
        2,
        "the third must wait for a free slot"
    );

    f.core.cancel_translation(&f.ids[2]);
    assert_eq!(code(c.await.unwrap()), ErrorCode::TranslationCancelled);
    assert_eq!(started.load(Ordering::SeqCst), 2);

    release.notify_waiters();
    release.notify_one();
    release.notify_one();
    assert!(a.await.unwrap().is_ok());
    assert!(b.await.unwrap().is_ok());
    assert_eq!(stats_count(&f), 2);
}

// -------------------------------------------------------------------- status

#[tokio::test]
async fn status_detects_the_language_without_the_network() {
    let f = fixture(&[&format!("<p>{ENGLISH}</p>"), &format!("<p>{CHINESE}</p>")]);
    let calls = echo(&f);
    with_key(&f, TranslationProviderId::Deepl).await;
    let s = TranslationSettings {
        target_language: "auto".into(),
        ..settings(TranslationProviderId::Deepl)
    };
    let status = |index: usize, ui: &'static str, s: TranslationSettings| {
        let (core, id) = (f.core.clone(), f.ids[index].clone());
        async move { core.translation_status(id, s, ui.into()).await.unwrap() }
    };

    let english = status(0, "zh-CN", s.clone()).await;
    assert_eq!(english.target_language, "zh-Hans");
    assert_eq!(english.detected_language.as_deref(), Some("en"));
    assert!(english.detection_reliable && english.suggested && !english.auto_translate);
    assert!(english.translation.is_none());

    // The same language as the UI needs no offer.
    let english_ui = status(0, "en", s.clone()).await;
    assert_eq!(english_ui.target_language, "en");
    assert!(!english_ui.suggested);

    // Both Chinese scripts count as one language.
    for target in ["zh-Hans", "zh-Hant"] {
        let chinese = status(
            1,
            "en",
            TranslationSettings {
                target_language: target.into(),
                ..s.clone()
            },
        )
        .await;
        assert_eq!(chinese.detected_language.as_deref(), Some("zh"));
        assert!(!chinese.suggested, "{target}");
    }
    let chinese = status(1, "en", s.clone()).await;
    assert!(chinese.suggested);

    assert!(
        calls.lock().unwrap().is_empty(),
        "status must not call the service"
    );
}

#[tokio::test]
async fn status_flags_follow_the_lists_the_key_and_the_consent() {
    let f = fixture(&[&format!("<p>{ENGLISH}</p>")]);
    let base = TranslationSettings {
        target_language: "de".into(),
        ..settings(TranslationProviderId::Deepl)
    };
    let status = |s: TranslationSettings| {
        let (core, id) = (f.core.clone(), f.ids[0].clone());
        async move { core.translation_status(id, s, "en".into()).await.unwrap() }
    };

    // No key yet: nothing is offered, but the language is still detected.
    let missing = status(base.clone()).await;
    assert_eq!(missing.detected_language.as_deref(), Some("en"));
    assert!(!missing.suggested && !missing.auto_translate);
    with_key(&f, TranslationProviderId::Deepl).await;
    assert!(status(base.clone()).await.suggested);

    let never = TranslationSettings {
        never_suggest: vec!["en".into()],
        ..base.clone()
    };
    assert!(!status(never).await.suggested);
    let quiet = TranslationSettings {
        suggest: false,
        ..base.clone()
    };
    assert!(!status(quiet).await.suggested);

    let always = TranslationSettings {
        always_translate: vec!["en".into()],
        ..base.clone()
    };
    let flags = status(always.clone()).await;
    assert!(flags.suggested && flags.auto_translate);
    let unconsented = TranslationSettings {
        consented_provider: None,
        ..always.clone()
    };
    assert!(!status(unconsented).await.auto_translate);
    let other_consent = TranslationSettings {
        consented_provider: Some(TranslationProviderId::Google),
        ..always.clone()
    };
    assert!(!status(other_consent).await.auto_translate);
    let no_provider = TranslationSettings {
        provider: None,
        ..always
    };
    let flags = status(no_provider).await;
    assert!(!flags.suggested && !flags.auto_translate);
}

#[tokio::test]
async fn short_text_gives_no_reliable_detection_and_no_offer() {
    let f = fixture(&["<p>Hi</p>"]);
    with_key(&f, TranslationProviderId::Deepl).await;
    let s = settings(TranslationProviderId::Deepl);
    let status = f
        .core
        .translation_status(f.ids[0].clone(), s, "zh-CN".into())
        .await
        .unwrap();
    assert!(!status.detection_reliable && !status.suggested && !status.auto_translate);
}

#[tokio::test]
async fn status_without_a_cached_body_does_not_know_the_language() {
    let f = fixture(&[&format!("<p>{ENGLISH}</p>")]);
    with_key(&f, TranslationProviderId::Deepl).await;
    f.db.execute("UPDATE messages SET body_state = 'missing'", [])
        .unwrap();
    let s = settings(TranslationProviderId::Deepl);
    let status = f
        .core
        .translation_status(f.ids[0].clone(), s, "en".into())
        .await
        .unwrap();
    assert_eq!(status.detected_language, None);
    assert!(!status.detection_reliable && !status.suggested);
}

#[tokio::test]
async fn status_of_a_body_over_the_html_limit_has_no_detection() {
    let f = fixture(&["<p>Hello</p>"]);
    f.db.execute(
        "UPDATE message_bodies SET sanitized_html = ?1",
        [format!(
            "<p>{ENGLISH}</p><p>{}</p>",
            "a".repeat(MAX_HTML_BYTES)
        )],
    )
    .unwrap();
    let s = settings(TranslationProviderId::Deepl);
    let status = f
        .core
        .translation_status(f.ids[0].clone(), s, "en".into())
        .await
        .unwrap();
    assert_eq!(status.detected_language, None);
}

fn input<'a>(
    settings: &'a TranslationSettings,
    id: &'a str,
    detection: Option<(&str, bool)>,
) -> StatusInput<'a> {
    StatusInput {
        email_id: id,
        settings,
        target: "zh-Hans".into(),
        provider_ready: true,
        detection: detection.map(|(language, reliable)| Detection {
            language: language.into(),
            reliable,
        }),
        translation: None,
    }
}

#[test]
fn status_rules_are_exact_for_detection_edges() {
    let s = settings(TranslationProviderId::Deepl);
    // Detection can say `zh`, `no` or a three-letter code; matching is by language.
    let zh = build_status(input(&s, "id", Some(("zh", true))));
    assert!(!zh.suggested);
    let nb = TranslationSettings {
        target_language: "nb".into(),
        ..s.clone()
    };
    let mut norwegian = input(&nb, "id", Some(("no", true)));
    norwegian.target = "nb".into();
    assert!(!build_status(norwegian).suggested);
    let unreliable = build_status(input(&s, "id", Some(("en", false))));
    assert_eq!(unreliable.detected_language.as_deref(), Some("en"));
    assert!(!unreliable.suggested && !unreliable.auto_translate);
    let unknown = build_status(input(&s, "id", None));
    assert_eq!(unknown.detected_language, None);
    assert!(!unknown.suggested);
    // Local Sent copies are never offered.
    let local = build_status(input(&s, "local-sent:abc", Some(("en", true))));
    assert!(!local.suggested && !local.auto_translate);
}

// --------------------------------------------------------------------- planning

#[test]
fn batches_respect_both_limits() {
    let limits = Limits {
        max_items: 3,
        max_bytes: 10,
    };
    let items: Vec<String> = ["aaaa", "bbbb", "cc", "d", "eeeeeeee", "f", "g", "h", "i"]
        .iter()
        .map(ToString::to_string)
        .collect();
    let ranges = batches(&items, limits);
    assert_eq!(ranges, [0..3, 3..6, 6..9]);
    let wide: Vec<String> = ["aaaaaa", "bbbbbb", "c"]
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        batches(
            &wide,
            Limits {
                max_items: 10,
                max_bytes: 10
            }
        ),
        [0..1, 1..3]
    );
    for range in &ranges {
        assert!(range.len() <= 3);
        assert!(items[range.clone()].iter().map(|i| i.len()).sum::<usize>() <= 10);
    }
    assert!(batches(&[], limits).is_empty());
}

#[test]
fn a_single_oversized_segment_is_skipped_not_split() {
    let segments = translate::extract("<p>short</p><p>0123456789abcdef</p>", false).unwrap();
    let planned = plan(
        &segments,
        Limits {
            max_items: 10,
            max_bytes: 10,
        },
    );
    assert_eq!(planned.unique, ["short"]);
    assert_eq!(planned.slots, [Some(0), None]);
    assert!(planned.partial);
}

// ------------------------------------------------------------------------- keys

#[tokio::test]
async fn keys_are_trimmed_validated_listed_and_never_returned() {
    let f = fixture(&["<p>Hello</p>"]);
    let all_missing: Vec<bool> = f
        .core
        .translation_keys()
        .await
        .unwrap()
        .iter()
        .map(|k| k.configured)
        .collect();
    assert_eq!(all_missing, [false, false, false]);

    f.core
        .set_translation_key(
            TranslationProviderId::Microsoft,
            SecretValue::new(format!("  {KEY}\n")),
        )
        .await
        .unwrap();
    assert_eq!(
        f.secrets
            .get("mailx/translation/microsoft")
            .unwrap()
            .unwrap()
            .expose_secret(),
        KEY
    );
    let listed = f.core.translation_keys().await.unwrap();
    assert_eq!(
        listed
            .iter()
            .map(|k| (k.provider, k.configured))
            .collect::<Vec<_>>(),
        [
            (TranslationProviderId::Deepl, false),
            (TranslationProviderId::Microsoft, true),
            (TranslationProviderId::Google, false),
        ]
    );
    let serialized = serde_json::to_string(&listed).unwrap();
    assert!(!serialized.contains(KEY) && !format!("{listed:?}").contains(KEY));

    for bad in ["", "   ", "a\nb", "a\u{0}b", &"k".repeat(513)] {
        let error = f
            .core
            .set_translation_key(TranslationProviderId::Deepl, SecretValue::new(bad.into()))
            .await
            .unwrap_err();
        assert_eq!(
            (error.code, error.field.as_deref()),
            (ErrorCode::Validation, Some("translation.key"))
        );
        assert!(!format!("{error:?}").contains("kkkk"));
    }
    assert!(f.secrets.get("mailx/translation/deepl").unwrap().is_none());
    f.core
        .set_translation_key(
            TranslationProviderId::Deepl,
            SecretValue::new("k".repeat(512)),
        )
        .await
        .unwrap();

    f.core
        .remove_translation_key(TranslationProviderId::Microsoft)
        .await
        .unwrap();
    f.core
        .remove_translation_key(TranslationProviderId::Microsoft)
        .await
        .unwrap();
    assert!(!f.core.translation_keys().await.unwrap()[1].configured);
}

const PROXY_PASSWORD: &str = " p@ss word ";

fn signed_in(username: &str) -> TranslationSettings {
    let mut s = settings(TranslationProviderId::Deepl);
    s.route = TranslationRoute::Proxy {
        proxy_kind: crate::accounts::types::ProxyKind::HttpConnect,
        host: "proxy.example".into(),
        port: 8080,
        username: username.into(),
    };
    s
}

#[tokio::test]
async fn the_proxy_password_is_validated_stored_and_never_returned() {
    let f = fixture(&["<p>Hello</p>"]);
    assert!(
        !f.core
            .translation_proxy_password()
            .await
            .unwrap()
            .configured
    );

    let state = f
        .core
        .set_translation_proxy_password(SecretValue::new(PROXY_PASSWORD.into()))
        .await
        .unwrap();
    assert!(state.configured);
    // Stored as typed, without trimming, under its own name.
    assert_eq!(
        f.secrets
            .get("mailx/translation/proxy-password")
            .unwrap()
            .unwrap()
            .expose_secret(),
        PROXY_PASSWORD
    );
    assert!(
        f.core
            .translation_proxy_password()
            .await
            .unwrap()
            .configured
    );
    assert!(f.secrets.get("mailx/translation/deepl").unwrap().is_none());
    let shown = format!("{state:?}{}", serde_json::to_string(&state).unwrap());
    assert!(!shown.contains("p@ss"));
    assert_eq!(
        serde_json::to_value(state).unwrap(),
        serde_json::json!({"configured": true})
    );

    let too_long = "p".repeat(MAX_SECRET_BYTES + 1);
    for bad in ["", "a\nb", "a\tb", "a\u{0}b", too_long.as_str()] {
        let error = f
            .core
            .set_translation_proxy_password(SecretValue::new(bad.into()))
            .await
            .unwrap_err();
        assert_eq!(
            (error.code, error.field.as_deref()),
            (ErrorCode::Validation, Some("translation.route.password"))
        );
        assert!(!format!("{error:?} {error}").contains("pppp"));
    }
    // Rejected input leaves the stored password alone.
    assert_eq!(
        f.secrets
            .get("mailx/translation/proxy-password")
            .unwrap()
            .unwrap()
            .expose_secret(),
        PROXY_PASSWORD
    );
    f.core
        .set_translation_proxy_password(SecretValue::new("p".repeat(MAX_SECRET_BYTES)))
        .await
        .unwrap();

    assert!(
        !f.core
            .remove_translation_proxy_password()
            .await
            .unwrap()
            .configured
    );
    assert!(
        !f.core
            .remove_translation_proxy_password()
            .await
            .unwrap()
            .configured
    );
    assert!(
        f.secrets
            .get("mailx/translation/proxy-password")
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn the_request_signs_in_to_the_proxy_only_with_a_username() {
    let f = fixture(&["<p>Hello</p>"]);
    let calls = echo(&f);
    with_key(&f, TranslationProviderId::Deepl).await;

    // A username without a stored password signs in with an empty one.
    translate_to_de(&f, 0, &signed_in("alice")).await.unwrap();
    f.core.clear_translations().await.unwrap();
    f.core
        .set_translation_proxy_password(SecretValue::new(PROXY_PASSWORD.into()))
        .await
        .unwrap();
    translate_to_de(&f, 0, &signed_in("alice")).await.unwrap();
    f.core.test_translation(signed_in("bob")).await.unwrap();
    // A stored password alone is ignored, on a proxy and on a direct route.
    f.core.test_translation(signed_in("")).await.unwrap();
    let mut direct = signed_in("alice");
    direct.route = TranslationRoute::Direct;
    f.core.test_translation(direct).await.unwrap();

    let seen: Vec<_> = calls
        .lock()
        .unwrap()
        .iter()
        .map(|call| (call.proxy.is_some(), call.proxy_auth.clone()))
        .collect();
    let pair = |user: &str, password: &str| Some((user.to_owned(), password.to_owned()));
    assert_eq!(
        seen,
        [
            (true, pair("alice", "")),
            (true, pair("alice", PROXY_PASSWORD)),
            (true, pair("bob", PROXY_PASSWORD)),
            (true, None),
            (false, None),
        ]
    );
}

/// A store that refuses to read one name, like a locked keychain.
struct LockedFor {
    inner: MemorySecrets,
    name: &'static str,
}

impl SecretStore for LockedFor {
    fn get(&self, key: &str) -> Result<Option<SecretValue>, crate::secrets::SecretStoreError> {
        if key == self.name {
            return Err(crate::secrets::SecretStoreError::Locked);
        }
        self.inner.get(key)
    }
    fn set(&self, key: &str, value: &SecretValue) -> Result<(), crate::secrets::SecretStoreError> {
        self.inner.set(key, value)
    }
    fn delete(&self, key: &str) -> Result<(), crate::secrets::SecretStoreError> {
        self.inner.delete(key)
    }
}

#[tokio::test]
async fn a_locked_store_with_a_username_fails_without_calling_the_service() {
    let directory = TempDir::new().unwrap();
    let store = Arc::new(LockedFor {
        inner: MemorySecrets::default(),
        name: "mailx/translation/proxy-password",
    });
    store
        .set("mailx/translation/deepl", &SecretValue::new(KEY.into()))
        .unwrap();
    let raw = mail("<p>Hello</p>");
    let (core, ids) = MailxCore::body_test_fixture_with_secrets(
        directory.path(),
        tokio::runtime::Handle::current(),
        &[raw.as_slice()],
        store,
    )
    .unwrap();
    let calls: Calls = Arc::default();
    let record = calls.clone();
    core.translation
        .set_backend(Arc::new(move |call: TestCall| {
            record.lock().unwrap().push(call);
            Box::pin(async {
                Ok(TranslateResponse {
                    texts: vec![],
                    detected_source: None,
                })
            })
        }));

    let translated = core
        .translate_email(ids[0].clone(), signed_in("alice"), "en".into(), None)
        .await;
    assert_eq!(code(translated), ErrorCode::SecretStoreLocked);
    assert_eq!(
        code(core.test_translation(signed_in("alice")).await),
        ErrorCode::SecretStoreLocked
    );
    assert_eq!(
        code(core.translation_proxy_password().await),
        ErrorCode::SecretStoreLocked
    );
    assert!(calls.lock().unwrap().is_empty());
    // Without a username the password is never read.
    core.test_translation(signed_in("")).await.unwrap_err();
    assert_eq!(calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn nothing_a_failure_reports_contains_the_key() {
    let f = fixture(&["<p>Hello</p>"]);
    f.core.translation.set_backend(Arc::new(|_| {
        Box::pin(async { Err(ErrorCode::TranslationKeyRejected) })
    }));
    with_key(&f, TranslationProviderId::Deepl).await;
    let error = translate_to_de(&f, 0, &settings(TranslationProviderId::Deepl))
        .await
        .unwrap_err();
    assert!(!format!("{error:?} {error}").contains(KEY));
    assert_eq!(
        error.message,
        "翻译服务没有接受这个密钥，请检查后重新填写。"
    );
    assert!(!format!("{:?}", SecretValue::new(KEY.into())).contains(KEY));
}

// ------------------------------------------------------------------- test call

#[tokio::test]
async fn the_test_translation_needs_a_key_not_consent_and_is_short() {
    let f = fixture(&["<p>Hello</p>"]);
    let calls: Calls = Arc::default();
    let record = calls.clone();
    f.core
        .translation
        .set_backend(Arc::new(move |call: TestCall| {
            let texts = vec![format!("{}&lt;{}", call.target, "x".repeat(400))];
            record.lock().unwrap().push(call);
            Box::pin(async move {
                Ok(TranslateResponse {
                    texts,
                    detected_source: None,
                })
            })
        }));
    let mut s = TranslationSettings {
        provider: Some(TranslationProviderId::Google),
        ..TranslationSettings::default()
    };
    assert_eq!(
        code(f.core.test_translation(s.clone()).await),
        ErrorCode::TranslationNotConfigured
    );
    with_key(&f, TranslationProviderId::Google).await;
    let result = f.core.test_translation(s.clone()).await.unwrap();
    assert_eq!(result.translated.chars().count(), 200);
    assert!(
        result.translated.starts_with("zh-Hans<x"),
        "{}",
        result.translated
    );
    assert_eq!(calls.lock().unwrap()[0].texts, ["Hello, this is a test."]);
    s.target_language = "fr".into();
    assert!(
        f.core
            .test_translation(s)
            .await
            .unwrap()
            .translated
            .starts_with("fr<")
    );
}

// ---------------------------------------------------------------------- cache

#[tokio::test]
async fn cached_translations_can_be_cleared_for_one_message_or_all() {
    let f = fixture(&["<p>Hello</p>", "<p>Hello again</p>"]);
    echo(&f);
    with_key(&f, TranslationProviderId::Deepl).await;
    let s = settings(TranslationProviderId::Deepl);
    translate_to_de(&f, 0, &s).await.unwrap();
    translate_to_de(&f, 1, &s).await.unwrap();
    let fr = TranslationSettings {
        target_language: "fr".into(),
        ..s.clone()
    };
    translate_to_de(&f, 0, &fr).await.unwrap();
    let stats = f.core.translation_cache_stats().await.unwrap();
    assert_eq!(stats.count, 3);
    assert!(stats.bytes > 0);

    f.core.clear_translation(f.ids[0].clone()).await.unwrap();
    assert_eq!(f.core.translation_cache_stats().await.unwrap().count, 1);
    f.core.clear_translations().await.unwrap();
    assert_eq!(
        f.core.translation_cache_stats().await.unwrap(),
        TranslationCacheStats { count: 0, bytes: 0 }
    );
}

#[tokio::test]
async fn local_sent_copies_are_not_translated() {
    let f = fixture(&["<p>Hello</p>"]);
    let calls = echo(&f);
    with_key(&f, TranslationProviderId::Deepl).await;
    let id = format!("{LOCAL_SENT_PREFIX}{}", uuid::Uuid::new_v4());
    let result = f
        .core
        .translate_email(
            id,
            settings(TranslationProviderId::Deepl),
            "en".into(),
            None,
        )
        .await;
    assert_eq!(result.unwrap_err().field.as_deref(), Some("emailId"));
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn four_byte_text_is_split_so_no_json_body_exceeds_the_documented_cap() {
    // Each segment is 4-byte characters wrapped in a placeholder span, whose
    // quotes are escaped inside JSON.
    let html: String = (0..400)
        .map(|n| format!("<p>{n} <b>{}</b></p>", "\u{20000}".repeat(500)))
        .collect();
    let segments = translate::extract(&html, false).unwrap();
    for (provider, cap) in [
        (TranslationProviderId::Deepl, 128 * 1024),
        (TranslationProviderId::Google, 100_000),
        (TranslationProviderId::Microsoft, 200_000),
        // UNVERIFIED caps: the browser channels publish none.
        (TranslationProviderId::MicrosoftFree, 100_000),
        (TranslationProviderId::GoogleFree, 100_000),
    ] {
        let limits = provider::limits(provider);
        let planned = plan(&segments, limits);
        let ranges = batches(&planned.unique, limits);
        assert!(ranges.len() > 1, "{provider:?}");
        for range in ranges {
            let items = &planned.unique[range];
            assert!(items.len() <= limits.max_items);
            let body = match provider {
                TranslationProviderId::Microsoft => serde_json::to_vec(
                    &items
                        .iter()
                        .map(|t| serde_json::json!({"Text": t}))
                        .collect::<Vec<_>>(),
                ),
                TranslationProviderId::Deepl => serde_json::to_vec(&serde_json::json!({
                    "text": items, "target_lang": "EN-US", "tag_handling": "html"
                })),
                TranslationProviderId::Google => serde_json::to_vec(&serde_json::json!({
                    "q": items, "target": "en", "format": "html"
                })),
                TranslationProviderId::MicrosoftFree => serde_json::to_vec(
                    &items
                        .iter()
                        .map(|t| translate::wire::encode_tags(&translate::wire::decode(t)))
                        .collect::<Vec<_>>(),
                ),
                TranslationProviderId::GoogleFree => Ok(items
                    .iter()
                    .fold(
                        url::form_urlencoded::Serializer::new(String::new()),
                        |mut form, t| {
                            form.append_pair("q", t);
                            form
                        },
                    )
                    .finish()
                    .into_bytes()),
            }
            .unwrap();
            assert!(body.len() <= cap, "{provider:?}: {} > {cap}", body.len());
        }
    }
}

#[tokio::test]
async fn a_body_past_the_segment_cap_is_stored_as_partial() {
    let paragraphs = |n: usize| (0..n).map(|i| format!("<p>n{i}</p>")).collect::<String>();
    let f = fixture(&[
        &paragraphs(translate::MAX_SEGMENTS + 1),
        &paragraphs(translate::MAX_SEGMENTS),
    ]);
    echo(&f);
    with_key(&f, TranslationProviderId::Deepl).await;
    let s = settings(TranslationProviderId::Deepl);
    let over = translate_to_de(&f, 0, &s).await.unwrap();
    assert!(over.translation.unwrap().partial);
    let exact = translate_to_de(&f, 1, &s).await.unwrap();
    assert!(!exact.translation.unwrap().partial);
}

#[tokio::test]
async fn keyless_services_translate_without_a_key_and_after_consent() {
    for provider in [
        TranslationProviderId::MicrosoftFree,
        TranslationProviderId::GoogleFree,
    ] {
        let f = fixture(&["<p>Hello <b>big</b> world</p>"]);
        let calls = echo(&f);
        let mut s = settings(provider);
        s.consented_provider = None;
        assert_eq!(
            code(translate_to_de(&f, 0, &s).await),
            ErrorCode::TranslationConsentRequired
        );
        s.consented_provider = Some(provider);
        let status = translate_to_de(&f, 0, &s).await.unwrap();
        assert_eq!(status.translation.unwrap().provider, provider);
        {
            let calls = calls.lock().unwrap();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].key, "");
        }
        // The offer needs no saved key either.
        let offered = f
            .core
            .translation_status(f.ids[0].clone(), s.clone(), "en".into())
            .await
            .unwrap();
        assert_eq!(offered.target_language, "de");
    }
}

#[tokio::test]
async fn test_translation_works_for_a_keyless_service_without_a_key() {
    let f = fixture(&["<p>Hello</p>"]);
    echo(&f);
    let s = settings(TranslationProviderId::MicrosoftFree);
    let result = f.core.test_translation(s).await.unwrap();
    assert!(!result.translated.is_empty());
}

#[tokio::test]
async fn keyless_services_have_no_key_to_set_remove_or_list() {
    let f = fixture(&["<p>Hello</p>"]);
    for provider in [
        TranslationProviderId::MicrosoftFree,
        TranslationProviderId::GoogleFree,
    ] {
        let set = f
            .core
            .set_translation_key(provider, SecretValue::new(KEY.into()))
            .await
            .unwrap_err();
        assert_eq!(set.field.as_deref(), Some("translation.provider"));
        let removed = f.core.remove_translation_key(provider).await.unwrap_err();
        assert_eq!(removed.field.as_deref(), Some("translation.provider"));
    }
    let listed: Vec<_> = f
        .core
        .translation_keys()
        .await
        .unwrap()
        .into_iter()
        .map(|state| state.provider)
        .collect();
    assert_eq!(
        listed,
        [
            TranslationProviderId::Deepl,
            TranslationProviderId::Microsoft,
            TranslationProviderId::Google
        ]
    );
}

#[tokio::test]
async fn a_keyed_service_without_a_key_is_still_not_ready() {
    let f = fixture(&[&format!("<p>{ENGLISH}</p>")]);
    let s = settings(TranslationProviderId::Deepl);
    let status = f
        .core
        .translation_status(f.ids[0].clone(), s, "zh-CN".into())
        .await
        .unwrap();
    assert!(!status.suggested);
}
