use std::sync::{Arc, Mutex};

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::*;
use crate::accounts::types::ProxyKind;
use crate::net::test_support::{dead_port, fake_http_connect, proxy_route};
use crate::translate::languages;

const KEY: &str = "sk-test-0123456789";

struct Captured {
    head: String,
    body: Vec<u8>,
}

impl Captured {
    fn request_line(&self) -> &str {
        self.head.lines().next().unwrap()
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.head.lines().skip(1).find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name).then(|| value.trim())
        })
    }

    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }

    /// Where the secret appears in the request, which must be one header only.
    fn secret_occurrences(&self, secret: &str) -> usize {
        self.head.matches(secret).count()
            + String::from_utf8_lossy(&self.body).matches(secret).count()
    }
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> Captured {
    let mut buffer = Vec::new();
    let end = loop {
        let mut chunk = [0u8; 4096];
        let read = stream.read(&mut chunk).await.unwrap();
        assert!(read > 0, "client closed before finishing its request");
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = buffer.windows(4).position(|w| w == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let head = String::from_utf8(buffer[..end].to_vec()).unwrap();
    let length = head
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0);
    let mut body = buffer[end..].to_vec();
    while body.len() < length {
        let mut chunk = [0u8; 4096];
        let read = stream.read(&mut chunk).await.unwrap();
        assert!(read > 0);
        body.extend_from_slice(&chunk[..read]);
    }
    Captured { head, body }
}

async fn respond(stream: &mut tokio::net::TcpStream, status: u16, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();
    let _ = stream.shutdown().await;
}

/// Plain-HTTP listener that answers each connection with the next canned reply.
async fn serve(replies: Vec<(u16, Vec<u8>)>) -> (u16, Arc<Mutex<Vec<Captured>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let captured = Arc::new(Mutex::new(Vec::new()));
    let record = captured.clone();
    tokio::spawn(async move {
        for (status, body) in replies {
            let (mut stream, _) = listener.accept().await.unwrap();
            let request = read_request(&mut stream).await;
            record.lock().unwrap().push(request);
            respond(&mut stream, status, &body).await;
        }
    });
    (port, captured)
}

fn context<'a>(
    key: &'a SecretValue,
    region: &'a str,
    route: &'a NetRoute,
    port: u16,
) -> Context<'a> {
    Context {
        key,
        microsoft_region: region,
        route,
        ai: None,
        base: Some(Base::plain(port)),
    }
}

fn texts() -> Vec<String> {
    vec![
        "Hello <span id=\"1\">world</span>".to_owned(),
        "Second &amp; last".to_owned(),
    ]
}

async fn call(
    provider: TranslationProviderId,
    status: u16,
    body: impl Into<Vec<u8>>,
) -> Result<TranslateResponse, ErrorCode> {
    let (port, _captured) = serve(vec![(status, body.into())]).await;
    let key = SecretValue::new(KEY.into());
    let route = NetRoute::default();
    let texts = texts();
    translate(
        provider,
        &context(&key, "", &route, port),
        &TranslateRequest {
            texts: &texts,
            target: "de",
        },
    )
    .await
}

fn error_of(result: Result<TranslateResponse, ErrorCode>) -> ErrorCode {
    result.expect_err("expected an error")
}

#[tokio::test]
async fn deepl_sends_the_documented_request_and_reads_the_answer() {
    let answer = json!({"translations": [
        {"detected_source_language": "EN", "text": "Hallo <span id=\"1\">Welt</span>"},
        {"detected_source_language": "EN", "text": "Zweite &amp; letzte"},
    ]});
    let (port, captured) = serve(vec![(200, answer.to_string().into_bytes())]).await;
    let key = SecretValue::new(KEY.into());
    let route = NetRoute::default();
    let texts = texts();
    let response = translate(
        TranslationProviderId::Deepl,
        &context(&key, "", &route, port),
        &TranslateRequest {
            texts: &texts,
            target: "en",
        },
    )
    .await
    .unwrap();
    assert_eq!(response.texts[0], "Hallo <span id=\"1\">Welt</span>");
    assert_eq!(response.detected_source.as_deref(), Some("en"));
    let captured = captured.lock().unwrap();
    let request = &captured[0];
    assert_eq!(request.request_line(), "POST /v2/translate HTTP/1.1");
    assert_eq!(
        request.header("authorization"),
        Some(format!("DeepL-Auth-Key {KEY}").as_str())
    );
    assert_eq!(request.header("content-type"), Some("application/json"));
    assert_eq!(
        request.json(),
        json!({"text": texts, "target_lang": "EN-US", "tag_handling": "html"})
    );
    assert_eq!(request.secret_occurrences(KEY), 1);
}

#[test]
fn deepl_uses_the_free_host_only_for_free_keys() {
    assert_eq!(deepl::host_for_key("abc:fx"), "api-free.deepl.com");
    assert_eq!(deepl::host_for_key("abc"), "api.deepl.com");
    assert_eq!(deepl::host_for_key("abc:fx "), "api.deepl.com");
}

#[tokio::test]
async fn deepl_statuses_map_to_plain_errors() {
    for (status, expected) in [
        (403, ErrorCode::TranslationKeyRejected),
        (456, ErrorCode::TranslationQuotaExceeded),
        (429, ErrorCode::TranslationRateLimited),
        (529, ErrorCode::TranslationRateLimited),
        (413, ErrorCode::TranslationTooLarge),
        (500, ErrorCode::TranslationUnavailable),
        (504, ErrorCode::TranslationUnavailable),
        (400, ErrorCode::TranslationInvalidResponse),
    ] {
        let result = call(
            TranslationProviderId::Deepl,
            status,
            r#"{"message":"secret-looking service text"}"#,
        )
        .await;
        assert_eq!(error_of(result), expected, "status {status}");
    }
}

#[tokio::test]
async fn microsoft_sends_the_documented_request_and_reads_the_answer() {
    let answer = json!([
        {"detectedLanguage": {"language": "en", "score": 1.0}, "translations": [{"text": "Hallo", "to": "zh-Hans"}]},
        {"detectedLanguage": {"language": "en", "score": 1.0}, "translations": [{"text": "Zweite", "to": "zh-Hans"}]},
    ]);
    for region in ["", "westeurope"] {
        let (port, captured) = serve(vec![(200, answer.to_string().into_bytes())]).await;
        let key = SecretValue::new(KEY.into());
        let route = NetRoute::default();
        let texts = texts();
        let response = translate(
            TranslationProviderId::Microsoft,
            &context(&key, region, &route, port),
            &TranslateRequest {
                texts: &texts,
                target: "zh-Hans",
            },
        )
        .await
        .unwrap();
        assert_eq!(response.texts, ["Hallo", "Zweite"]);
        assert_eq!(response.detected_source.as_deref(), Some("en"));
        let captured = captured.lock().unwrap();
        let request = &captured[0];
        assert_eq!(
            request.request_line(),
            "POST /translate?api-version=3.0&to=zh-Hans&textType=html HTTP/1.1"
        );
        assert_eq!(request.header("ocp-apim-subscription-key"), Some(KEY));
        // The region header exists only for a regional resource.
        assert_eq!(
            request.header("ocp-apim-subscription-region"),
            (!region.is_empty()).then_some(region)
        );
        assert_eq!(
            request.json(),
            json!([{"Text": texts[0]}, {"Text": texts[1]}])
        );
        assert_eq!(request.secret_occurrences(KEY), 1);
    }
}

#[tokio::test]
async fn microsoft_statuses_and_shapes_are_handled() {
    for (status, expected) in [
        (401, ErrorCode::TranslationKeyRejected),
        (403, ErrorCode::TranslationQuotaExceeded),
        (408, ErrorCode::TranslationUnavailable),
        (429, ErrorCode::TranslationRateLimited),
        (503, ErrorCode::TranslationUnavailable),
    ] {
        let result = call(TranslationProviderId::Microsoft, status, "{}").await;
        assert_eq!(error_of(result), expected, "status {status}");
    }
    // One answer per target language is expected, exactly one here.
    let two = json!([
        {"translations": [{"text": "a"}, {"text": "b"}]},
        {"translations": [{"text": "c"}]},
    ]);
    let result = call(TranslationProviderId::Microsoft, 200, two.to_string()).await;
    assert_eq!(error_of(result), ErrorCode::TranslationInvalidResponse);
}

#[tokio::test]
async fn google_sends_the_key_in_a_header_only() {
    let answer = json!({"data": {"translations": [
        {"translatedText": "Hallo", "detectedSourceLanguage": "en"},
        {"translatedText": "Zweite"},
    ]}});
    let (port, captured) = serve(vec![(200, answer.to_string().into_bytes())]).await;
    let key = SecretValue::new(KEY.into());
    let route = NetRoute::default();
    let texts = texts();
    let response = translate(
        TranslationProviderId::Google,
        &context(&key, "", &route, port),
        &TranslateRequest {
            texts: &texts,
            target: "zh-Hant",
        },
    )
    .await
    .unwrap();
    assert_eq!(response.texts, ["Hallo", "Zweite"]);
    assert_eq!(response.detected_source.as_deref(), Some("en"));
    let captured = captured.lock().unwrap();
    let request = &captured[0];
    assert_eq!(
        request.request_line(),
        "POST /language/translate/v2 HTTP/1.1"
    );
    assert_eq!(request.header("x-goog-api-key"), Some(KEY));
    assert_eq!(
        request.json(),
        json!({"q": texts, "target": "zh-TW", "format": "html"})
    );
    assert_eq!(request.secret_occurrences(KEY), 1);
}

#[tokio::test]
async fn google_errors_follow_the_reason_not_the_text() {
    let invalid_key = json!({"error": {"code": 400, "message": "API key not valid.",
        "status": "INVALID_ARGUMENT", "details": [{"reason": "API_KEY_INVALID"}]}});
    let daily = json!({"error": {"code": 403, "errors": [{"reason": "dailyLimitExceeded"}]}});
    let per_user = json!({"error": {"code": 403, "errors": [{"reason": "userRateLimitExceeded"}]}});
    let blocked =
        json!({"error": {"code": 403, "details": [{"reason": "API_KEY_SERVICE_BLOCKED"}]}});
    let other_400 = json!({"error": {"code": 400, "errors": [{"reason": "invalid"}]}});
    for (status, body, expected) in [
        (
            400,
            invalid_key.to_string(),
            ErrorCode::TranslationKeyRejected,
        ),
        (
            400,
            other_400.to_string(),
            ErrorCode::TranslationInvalidResponse,
        ),
        (403, daily.to_string(), ErrorCode::TranslationQuotaExceeded),
        (403, per_user.to_string(), ErrorCode::TranslationRateLimited),
        (403, blocked.to_string(), ErrorCode::TranslationKeyRejected),
        (
            403,
            "not json".to_owned(),
            ErrorCode::TranslationKeyRejected,
        ),
        (429, "{}".to_owned(), ErrorCode::TranslationRateLimited),
        (503, "{}".to_owned(), ErrorCode::TranslationUnavailable),
    ] {
        let result = call(TranslationProviderId::Google, status, body).await;
        assert_eq!(error_of(result), expected, "status {status}");
    }
}

#[tokio::test]
async fn unsupported_targets_fail_before_any_connection() {
    let key = SecretValue::new(KEY.into());
    let route = NetRoute::default();
    // Nothing listens here: a connection attempt would be a different error.
    let port = dead_port().await;
    let texts = texts();
    for provider in TranslationProviderId::ALL {
        let result = translate(
            provider,
            &context(&key, "", &route, port),
            &TranslateRequest {
                texts: &texts,
                target: "tlh",
            },
        )
        .await;
        assert_eq!(error_of(result), ErrorCode::TranslationLanguageUnsupported);
    }
}

#[test]
fn every_offered_target_has_a_code_at_every_service() {
    for code in languages::TARGETS {
        assert!(deepl::target_code(code).is_some(), "deepl {code}");
        assert!(microsoft::target_code(code).is_some(), "microsoft {code}");
        assert!(google::target_code(code).is_some(), "google {code}");
    }
}

#[test]
fn regional_codes_differ_per_service() {
    assert_eq!(deepl::target_code("en"), Some("EN-US"));
    assert_eq!(deepl::target_code("pt"), Some("PT-BR"));
    assert_eq!(deepl::target_code("zh-Hant"), Some("ZH-HANT"));
    assert_eq!(microsoft::target_code("zh-Hans"), Some("zh-Hans"));
    assert_eq!(google::target_code("zh-Hans"), Some("zh-CN"));
    assert_eq!(google::target_code("zh-Hant"), Some("zh-TW"));
    assert_eq!(google::target_code("nb"), Some("no"));
}

#[tokio::test]
async fn short_or_malformed_answers_are_invalid() {
    let one = json!({"translations": [{"text": "only one"}]});
    let result = call(TranslationProviderId::Deepl, 200, one.to_string()).await;
    assert_eq!(error_of(result), ErrorCode::TranslationInvalidResponse);
    let result = call(TranslationProviderId::Deepl, 200, "<html>").await;
    assert_eq!(error_of(result), ErrorCode::TranslationInvalidResponse);
    let result = call(TranslationProviderId::Google, 200, "{}").await;
    assert_eq!(error_of(result), ErrorCode::TranslationInvalidResponse);
}

#[tokio::test]
async fn an_answer_far_longer_than_its_source_is_invalid() {
    let long = "x".repeat(texts()[1].len() * MAX_GROWTH_RATIO + MAX_GROWTH_SLACK + 1);
    let fits = "x".repeat(texts()[1].len() * MAX_GROWTH_RATIO + MAX_GROWTH_SLACK);
    for (second, ok) in [(long, false), (fits, true)] {
        let answer = json!({"translations": [{"text": "a"}, {"text": second}]});
        let result = call(TranslationProviderId::Deepl, 200, answer.to_string()).await;
        assert_eq!(result.is_ok(), ok);
        if !ok {
            assert_eq!(error_of(result), ErrorCode::TranslationInvalidResponse);
        }
    }
}

#[tokio::test]
async fn an_answer_over_the_size_cap_is_invalid() {
    let body = vec![b' '; 4 * 1024 * 1024 + 1];
    let result = call(TranslationProviderId::Deepl, 200, body).await;
    assert_eq!(error_of(result), ErrorCode::TranslationInvalidResponse);
}

#[tokio::test]
async fn a_refused_connection_is_a_network_error() {
    let port = dead_port().await;
    let key = SecretValue::new(KEY.into());
    let route = NetRoute::default();
    let texts = texts();
    let result = translate(
        TranslationProviderId::Deepl,
        &context(&key, "", &route, port),
        &TranslateRequest {
            texts: &texts,
            target: "de",
        },
    )
    .await;
    assert_eq!(error_of(result), ErrorCode::TranslationNetwork);
}

#[tokio::test(start_paused = true)]
async fn a_silent_service_times_out() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let _held = tokio::spawn(async move {
        let (_stream, _) = listener.accept().await.unwrap();
        std::future::pending::<()>().await;
    });
    let key = SecretValue::new(KEY.into());
    let route = NetRoute::default();
    let texts = texts();
    let result = translate(
        TranslationProviderId::Deepl,
        &context(&key, "", &route, port),
        &TranslateRequest {
            texts: &texts,
            target: "de",
        },
    )
    .await;
    assert_eq!(error_of(result), ErrorCode::TranslationTimeout);
}

#[tokio::test]
async fn dropping_the_future_closes_the_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (closed, closed_receiver) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        read_request(&mut stream).await;
        let mut rest = [0u8; 64];
        // EOF means the client let go of the connection.
        let _ = stream.read(&mut rest).await;
        let _ = closed.send(());
    });
    let key = SecretValue::new(KEY.into());
    let route = NetRoute::default();
    let texts = texts();
    let request = TranslateRequest {
        texts: &texts,
        target: "de",
    };
    let context = context(&key, "", &route, port);
    let outcome = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        translate(TranslationProviderId::Deepl, &context, &request),
    )
    .await;
    assert!(outcome.is_err(), "the service never answered");
    tokio::time::timeout(std::time::Duration::from_secs(5), closed_receiver)
        .await
        .expect("the connection stays open after the future is dropped")
        .unwrap();
}

#[tokio::test]
async fn the_request_goes_to_the_proxy_which_resolves_the_service_host() {
    let answer = json!({"translations": [{"text": "a"}, {"text": "b"}]});
    let (upstream, captured) = serve(vec![(200, answer.to_string().into_bytes())]).await;
    let (proxy_port, proxy_requests) = fake_http_connect(upstream).await;
    let route = proxy_route(ProxyKind::HttpConnect, proxy_port);
    let key = SecretValue::new(KEY.into());
    let texts = texts();
    let response = translate(
        TranslationProviderId::Deepl,
        &context(&key, "", &route, upstream),
        &TranslateRequest {
            texts: &texts,
            target: "de",
        },
    )
    .await
    .unwrap();
    assert_eq!(response.texts, ["a", "b"]);
    assert_eq!(
        proxy_requests.lock().unwrap().as_slice(),
        [format!("CONNECT 127.0.0.1:{upstream} HTTP/1.1")]
    );
    assert_eq!(captured.lock().unwrap().len(), 1);
}

/// HTTP CONNECT proxy that records the whole CONNECT head, then forwards to
/// `upstream`, or refuses with 407 when `refuse` is set.
async fn recording_connect_proxy(upstream: u16, refuse: bool) -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let heads = Arc::new(Mutex::new(Vec::new()));
    let record = heads.clone();
    tokio::spawn(async move {
        let (mut client, _) = listener.accept().await.unwrap();
        let head = read_request(&mut client).await.head;
        record.lock().unwrap().push(head);
        if refuse {
            let _ = client
                .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\n\r\n")
                .await;
            return;
        }
        let mut server = tokio::net::TcpStream::connect(("127.0.0.1", upstream))
            .await
            .unwrap();
        client.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await.unwrap();
        let _ = tokio::io::copy_bidirectional(&mut client, &mut server).await;
    });
    (port, heads)
}

fn signed_route(port: u16, auth: Option<(&str, &str)>) -> NetRoute {
    let mut route = proxy_route(ProxyKind::HttpConnect, port);
    route.proxy.as_mut().unwrap().auth =
        auth.map(|(user, password)| (user.to_owned(), SecretValue::new(password.to_owned())));
    route
}

async fn deepl_through(route: &NetRoute, upstream: u16) -> Result<TranslateResponse, ErrorCode> {
    let key = SecretValue::new(KEY.into());
    let texts = texts();
    translate(
        TranslationProviderId::Deepl,
        &context(&key, "", route, upstream),
        &TranslateRequest {
            texts: &texts,
            target: "de",
        },
    )
    .await
}

#[tokio::test]
async fn a_signed_in_proxy_gets_basic_credentials_and_only_then() {
    let answer = json!({"translations": [{"text": "a"}, {"text": "b"}]}).to_string();
    for (auth, expected) in [
        // A leading space and a colon in the password are sent as typed.
        (Some((" alice", "s3:cret ")), Some(" alice:s3:cret ")),
        (Some(("alice", "")), Some("alice:")),
        (None, None),
    ] {
        let (upstream, _) = serve(vec![(200, answer.clone().into_bytes())]).await;
        let (proxy_port, heads) = recording_connect_proxy(upstream, false).await;
        let route = signed_route(proxy_port, auth);
        let response = deepl_through(&route, upstream).await.unwrap();
        assert_eq!(response.texts, ["a", "b"]);
        let heads = heads.lock().unwrap();
        let header = heads[0]
            .lines()
            .find_map(|line| line.strip_prefix("Proxy-Authorization: "));
        let wanted = expected.map(|pair| format!("Basic {}", STANDARD.encode(pair)));
        assert_eq!(header.map(str::to_owned), wanted, "{auth:?}");
        // The service key never reaches the proxy, signed in or not.
        assert!(!heads[0].contains(KEY));
    }
}

#[tokio::test]
async fn a_proxy_that_rejects_the_sign_in_is_a_proxy_error() {
    let (upstream, captured) = serve(vec![(200, b"{}".to_vec())]).await;
    let (proxy_port, heads) = recording_connect_proxy(upstream, true).await;
    let route = signed_route(proxy_port, Some(("alice", "wrong")));
    assert_eq!(
        error_of(deepl_through(&route, upstream).await),
        ErrorCode::TranslationProxy
    );
    assert_eq!(heads.lock().unwrap().len(), 1);
    assert!(captured.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_refused_proxy_is_a_proxy_error_and_never_a_direct_connection() {
    let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream_port = upstream.local_addr().unwrap().port();
    let route = proxy_route(ProxyKind::Socks5, dead_port().await);
    let key = SecretValue::new(KEY.into());
    let texts = texts();
    let result = translate(
        TranslationProviderId::Google,
        &context(&key, "", &route, upstream_port),
        &TranslateRequest {
            texts: &texts,
            target: "de",
        },
    )
    .await;
    assert_eq!(error_of(result), ErrorCode::TranslationProxy);
    let direct =
        tokio::time::timeout(std::time::Duration::from_millis(200), upstream.accept()).await;
    assert!(direct.is_err(), "the service was reached without the proxy");
}

#[test]
fn nothing_holding_a_key_prints_it() {
    let key = SecretValue::new(KEY.into());
    let route = NetRoute::default();
    let context = context(&key, "", &route, 1);
    assert!(!format!("{context:?}").contains(KEY));
    let signed = signed_route(1, Some(("alice", "s3cret-proxy")));
    let with_password = super::Context {
        route: &signed,
        ..context
    };
    assert!(!format!("{with_password:?} {signed:?}").contains("s3cret-proxy"));
    let header = http::secret_header(KEY).unwrap();
    assert!(!format!("{header:?}").contains(KEY));
}

#[test]
fn limits_leave_room_inside_the_documented_maximums() {
    let deepl = limits(TranslationProviderId::Deepl);
    assert!(deepl.max_bytes < 128 * 1024);
    let microsoft = limits(TranslationProviderId::Microsoft);
    assert!(microsoft.max_bytes <= 50_000 && microsoft.max_items <= 1000);
    let google = limits(TranslationProviderId::Google);
    assert!(google.max_bytes < 100_000 && google.max_items <= 128);
}

// ---------------------------------------------------------- keyless channels

async fn keyless_call(
    provider: TranslationProviderId,
    target: &str,
    status: u16,
    body: impl Into<Vec<u8>>,
) -> (
    Result<TranslateResponse, ErrorCode>,
    Arc<Mutex<Vec<Captured>>>,
) {
    let (port, captured) = serve(vec![(status, body.into())]).await;
    // A key in the context must never reach a keyless request.
    let key = SecretValue::new(KEY.into());
    let route = NetRoute::default();
    let texts = texts();
    let result = translate(
        provider,
        &context(&key, "westeurope", &route, port),
        &TranslateRequest {
            texts: &texts,
            target,
        },
    )
    .await;
    (result, captured)
}

fn assert_no_credentials(request: &Captured) {
    for name in [
        "authorization",
        "ocp-apim-subscription-key",
        "ocp-apim-subscription-region",
        "x-goog-api-key",
    ] {
        assert_eq!(request.header(name), None, "{name}");
    }
    assert_eq!(request.secret_occurrences(KEY), 0);
    assert_eq!(request.secret_occurrences("westeurope"), 0);
    assert_eq!(request.header("user-agent"), Some("MailX"));
}

#[tokio::test]
async fn microsoft_free_sends_tag_names_and_returns_the_html_wire_form() {
    let answer = json!([
        {"detectedLanguage": {"language": "en", "score": 1.0},
         "translations": [{"text": "Hallo <g1>Welt</g1> &lt;3", "to": "zh-Hans"}]},
        {"detectedLanguage": {"language": "en", "score": 1.0},
         "translations": [{"text": "Zweite &amp; letzte<n2/><x3 />", "to": "zh-Hans"}]},
    ]);
    let (result, captured) = keyless_call(
        TranslationProviderId::MicrosoftFree,
        "zh-Hans",
        200,
        answer.to_string(),
    )
    .await;
    let response = result.unwrap();
    assert_eq!(
        response.texts,
        [
            r#"Hallo <span id="1">Welt</span> &lt;3"#,
            r#"Zweite &amp; letzte<br id="2"><img id="3">"#,
        ]
    );
    assert_eq!(response.detected_source.as_deref(), Some("en"));
    let captured = captured.lock().unwrap();
    let request = &captured[0];
    assert_eq!(
        request.request_line(),
        "POST /translate/translatetext?from=&to=zh-Hans&isEnterpriseClient=false HTTP/1.1"
    );
    assert_eq!(request.header("content-type"), Some("application/json"));
    assert_eq!(
        request.json(),
        json!(["Hello <g1>world</g1>", "Second &amp; last"])
    );
    assert_no_credentials(request);
}

#[tokio::test]
async fn microsoft_free_statuses_and_shapes_are_handled() {
    for (status, expected) in [
        (401, ErrorCode::TranslationUnavailable),
        (403, ErrorCode::TranslationUnavailable),
        (404, ErrorCode::TranslationUnavailable),
        (410, ErrorCode::TranslationUnavailable),
        (429, ErrorCode::TranslationRateLimited),
        (500, ErrorCode::TranslationUnavailable),
        (503, ErrorCode::TranslationUnavailable),
        (400, ErrorCode::TranslationInvalidResponse),
    ] {
        let (result, _) =
            keyless_call(TranslationProviderId::MicrosoftFree, "de", status, "{}").await;
        assert_eq!(error_of(result), expected, "status {status}");
    }
    for body in [
        "<html>",
        r#"[{"translations": []}, {"translations": []}]"#,
        r#"[{"translations": [{"text": "a"}, {"text": "b"}]}]"#,
        r#"{"translations": [{"text": "a"}]}"#,
    ] {
        let (result, _) = keyless_call(TranslationProviderId::MicrosoftFree, "de", 200, body).await;
        assert_eq!(error_of(result), ErrorCode::TranslationInvalidResponse);
    }
}

#[tokio::test]
async fn microsoft_free_hostile_text_stays_text() {
    let answer = json!([
        {"translations": [{"text": "<script>x</script><g1 onclick=1>a</g1><g99999>b"}]},
        {"translations": [{"text": "<img src=x onerror=1>"}]},
    ]);
    let (result, _) = keyless_call(
        TranslationProviderId::MicrosoftFree,
        "de",
        200,
        answer.to_string(),
    )
    .await;
    assert_eq!(
        result.unwrap().texts,
        [
            // The stray `</g1>` makes the answer unbalanced, so only its text is kept.
            "&lt;script&gt;x&lt;/script&gt;&lt;g1 onclick=1&gt;a&lt;g99999&gt;b",
            "&lt;img src=x onerror=1&gt;"
        ]
    );
}

#[tokio::test]
async fn microsoft_free_crossed_tags_keep_only_their_text() {
    let answer = json!([
        {"translations": [{"text": "<g1>a<g2>b</g1>c</g2>"}]},
        {"translations": [{"text": "x<g1>y</g1><n2/>z"}]},
    ]);
    let (result, _) = keyless_call(
        TranslationProviderId::MicrosoftFree,
        "de",
        200,
        answer.to_string(),
    )
    .await;
    assert_eq!(
        result.unwrap().texts,
        ["abc", "x<span id=\"1\">y</span><br id=\"2\">z"]
    );
}

#[tokio::test]
async fn google_free_posts_a_form_and_reads_every_known_shape() {
    for (body, expected_detected) in [
        (
            json!([
                ["Hallo <span id=\"1\">Welt</span>", "en"],
                ["Zweite &amp; letzte", "en"]
            ]),
            Some("en"),
        ),
        (
            json!(["Hallo <span id=\"1\">Welt</span>", "Zweite &amp; letzte"]),
            None,
        ),
    ] {
        let (result, captured) = keyless_call(
            TranslationProviderId::GoogleFree,
            "zh-Hant",
            200,
            body.to_string(),
        )
        .await;
        let response = result.unwrap();
        assert_eq!(
            response.texts,
            ["Hallo <span id=\"1\">Welt</span>", "Zweite &amp; letzte"]
        );
        assert_eq!(response.detected_source.as_deref(), expected_detected);
        let captured = captured.lock().unwrap();
        let request = &captured[0];
        assert_eq!(
            request.request_line(),
            "POST /translate_a/t?client=gtx&sl=auto&tl=zh-TW&format=html HTTP/1.1"
        );
        assert_eq!(
            request.header("content-type"),
            Some("application/x-www-form-urlencoded")
        );
        let fields: Vec<(String, String)> = url::form_urlencoded::parse(&request.body)
            .into_owned()
            .collect();
        let expected: Vec<(String, String)> =
            texts().into_iter().map(|t| ("q".to_owned(), t)).collect();
        assert_eq!(fields, expected);
        assert_no_credentials(request);
    }
}

#[tokio::test]
async fn google_free_reads_a_single_item_answer() {
    let (port, _) = serve(vec![(200, br#"[["Hallo","en"]]"#.to_vec())]).await;
    let key = SecretValue::new(String::new());
    let route = NetRoute::default();
    let one = vec!["Hello".to_owned()];
    let response = translate(
        TranslationProviderId::GoogleFree,
        &context(&key, "", &route, port),
        &TranslateRequest {
            texts: &one,
            target: "de",
        },
    )
    .await
    .unwrap();
    assert_eq!(response.texts, ["Hallo"]);
    assert_eq!(response.detected_source.as_deref(), Some("en"));
}

#[tokio::test]
async fn google_free_statuses_and_shapes_are_handled() {
    for (status, expected) in [
        (401, ErrorCode::TranslationUnavailable),
        (403, ErrorCode::TranslationUnavailable),
        (404, ErrorCode::TranslationUnavailable),
        (410, ErrorCode::TranslationUnavailable),
        (429, ErrorCode::TranslationRateLimited),
        (503, ErrorCode::TranslationUnavailable),
        (400, ErrorCode::TranslationInvalidResponse),
    ] {
        let (result, _) = keyless_call(TranslationProviderId::GoogleFree, "de", status, "x").await;
        assert_eq!(error_of(result), expected, "status {status}");
    }
    for body in ["<html>", "{}", "[[]]", "[1, 2]", r#"["only one"]"#] {
        let (result, _) = keyless_call(TranslationProviderId::GoogleFree, "de", 200, body).await;
        assert_eq!(
            error_of(result),
            ErrorCode::TranslationInvalidResponse,
            "{body}"
        );
    }
}

#[tokio::test]
async fn keyless_requests_go_through_the_proxy_and_never_around_it() {
    let answer = json!([
        {"translations": [{"text": "a"}]},
        {"translations": [{"text": "b"}]},
    ]);
    let (upstream, captured) = serve(vec![(200, answer.to_string().into_bytes())]).await;
    let (proxy_port, proxy_requests) = fake_http_connect(upstream).await;
    let route = proxy_route(ProxyKind::HttpConnect, proxy_port);
    let key = SecretValue::new(String::new());
    let texts = texts();
    let response = translate(
        TranslationProviderId::MicrosoftFree,
        &context(&key, "", &route, upstream),
        &TranslateRequest {
            texts: &texts,
            target: "de",
        },
    )
    .await
    .unwrap();
    assert_eq!(response.texts, ["a", "b"]);
    assert_eq!(
        proxy_requests.lock().unwrap().as_slice(),
        [format!("CONNECT 127.0.0.1:{upstream} HTTP/1.1")]
    );
    assert_eq!(captured.lock().unwrap().len(), 1);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let direct_port = listener.local_addr().unwrap().port();
    let dead = proxy_route(ProxyKind::Socks5, dead_port().await);
    for provider in [
        TranslationProviderId::MicrosoftFree,
        TranslationProviderId::GoogleFree,
    ] {
        let result = translate(
            provider,
            &context(&key, "", &dead, direct_port),
            &TranslateRequest {
                texts: &texts,
                target: "de",
            },
        )
        .await;
        assert_eq!(error_of(result), ErrorCode::TranslationProxy);
    }
    let direct =
        tokio::time::timeout(std::time::Duration::from_millis(200), listener.accept()).await;
    assert!(direct.is_err(), "the channel was reached without the proxy");
}

#[test]
fn keyless_limits_are_conservative() {
    assert!(!TranslationProviderId::MicrosoftFree.needs_key());
    assert!(!TranslationProviderId::GoogleFree.needs_key());
    let edge = limits(TranslationProviderId::MicrosoftFree);
    assert_eq!((edge.max_items, edge.max_bytes), (50, 20_000));
    let google = limits(TranslationProviderId::GoogleFree);
    assert_eq!((google.max_items, google.max_bytes), (50, 5_000));
}
