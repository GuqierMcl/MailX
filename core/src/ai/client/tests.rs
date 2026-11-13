use super::*;
use crate::{
    accounts::types::ProxyKind,
    net::test_support::{MockReply, dead_port, mock_http, proxy_route},
};

const OPENAI_LIST: &str = r#"{"object":"list","data":[
    {"id":"gpt-4o","object":"model"},
    {"id":"gpt-4o-mini","name":"Mini"},
    {"id":"gpt-4o"},
    {"id":""},
    {"object":"model"},
    {"id":"a-model"}]}"#;
const OPENAI_CHAT: &str = r#"{"choices":[{"message":{"role":"assistant","content":"OK"}}]}"#;
const ANTHROPIC_CHAT: &str = r#"{"content":[{"type":"text","text":"OK"}],"role":"assistant"}"#;

/// Test-only: lets an endpoint borrow a URL built on the spot.
fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

fn endpoint<'a>(
    protocol: AiProtocol,
    base: &'a str,
    key: Option<&'a str>,
    route: &'a NetRoute,
) -> Endpoint<'a> {
    Endpoint {
        protocol,
        base_url: base,
        key,
        route,
    }
}

#[tokio::test]
async fn openai_compatible_lists_models_with_a_bearer_key() {
    let server = mock_http(|_| MockReply::json(200, OPENAI_LIST)).await;
    let base = format!("{}/v1", server.url());
    let route = NetRoute::default();
    let models = list_models(&endpoint(
        AiProtocol::OpenaiCompatible,
        &base,
        Some("sk-test"),
        &route,
    ))
    .await
    .unwrap()
    .unwrap();
    // Sorted, one per id, entries without an id skipped.
    let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["a-model", "gpt-4o", "gpt-4o-mini"]);
    assert_eq!(models[2].name.as_deref(), Some("Mini"));
    assert_eq!(models[1].name, None);
    let request = server.last();
    assert_eq!(request.method, "GET");
    assert_eq!(request.target, "/v1/models");
    assert_eq!(request.header("authorization"), Some("Bearer sk-test"));
    assert_eq!(request.header("user-agent"), Some("MailX"));
    assert_eq!(request.header("x-api-key"), None);
}

#[tokio::test]
async fn a_keyless_connection_sends_no_credentials() {
    let server = mock_http(|_| MockReply::json(200, OPENAI_LIST)).await;
    let route = NetRoute::default();
    for key in [None, Some("")] {
        list_models(&endpoint(
            AiProtocol::OpenaiCompatible,
            leak(server.url()),
            key,
            &route,
        ))
        .await
        .unwrap();
        let request = server.last();
        assert_eq!(request.target, "/models");
        assert_eq!(request.header("authorization"), None);
    }
    let server = mock_http(|_| MockReply::json(200, ANTHROPIC_CHAT)).await;
    test(
        &endpoint(AiProtocol::Anthropic, leak(server.url()), None, &route),
        "m",
    )
    .await
    .unwrap();
    assert_eq!(server.last().header("x-api-key"), None);
    assert_eq!(
        server.last().header("anthropic-version"),
        Some("2023-06-01")
    );
}

#[tokio::test]
async fn anthropic_lists_models_with_its_own_headers() {
    let server = mock_http(|_| {
        MockReply::json(
            200,
            r#"{"data":[{"id":"claude-b","display_name":"Claude B","type":"model"},{"id":"claude-a","display_name":"Claude A"}],"has_more":false}"#,
        )
    })
    .await;
    let base = format!("{}/v1", server.url());
    let route = NetRoute::default();
    let models = list_models(&endpoint(
        AiProtocol::Anthropic,
        &base,
        Some("sk-ant"),
        &route,
    ))
    .await
    .unwrap()
    .unwrap();
    assert_eq!(models[0].id, "claude-a");
    assert_eq!(models[0].name.as_deref(), Some("Claude A"));
    let request = server.last();
    assert_eq!(request.target, "/v1/models?limit=1000");
    assert_eq!(request.header("x-api-key"), Some("sk-ant"));
    assert_eq!(request.header("anthropic-version"), Some("2023-06-01"));
    assert_eq!(request.header("authorization"), None);
}

#[tokio::test]
async fn the_openai_test_call_has_the_fixed_body_and_no_token_cap() {
    let server = mock_http(|_| MockReply::json(200, OPENAI_CHAT)).await;
    let base = format!("{}/v1", server.url());
    let route = NetRoute::default();
    let latency = test(
        &endpoint(AiProtocol::OpenaiCompatible, &base, Some("k"), &route),
        "gpt-x",
    )
    .await
    .unwrap();
    assert!(latency < 10_000);
    let request = server.last();
    assert_eq!(request.method, "POST");
    assert_eq!(request.target, "/v1/chat/completions");
    assert_eq!(request.header("authorization"), Some("Bearer k"));
    assert_eq!(request.header("content-type"), Some("application/json"));
    assert_eq!(
        request.json(),
        json!({"model":"gpt-x","messages":[{"role":"user","content":"Reply with OK."}],"stream":false})
    );
}

#[tokio::test]
async fn the_anthropic_test_call_caps_the_reply_at_16_tokens() {
    let server = mock_http(|_| MockReply::json(200, ANTHROPIC_CHAT)).await;
    let base = format!("{}/v1", server.url());
    let route = NetRoute::default();
    test(
        &endpoint(AiProtocol::Anthropic, &base, Some("k"), &route),
        "claude-x",
    )
    .await
    .unwrap();
    let request = server.last();
    assert_eq!(request.target, "/v1/messages");
    assert_eq!(request.header("x-api-key"), Some("k"));
    assert_eq!(
        request.json(),
        json!({"model":"claude-x","max_tokens":16,"messages":[{"role":"user","content":"Reply with OK."}]})
    );
}

#[tokio::test]
async fn complete_returns_the_reply_text_for_both_protocols() {
    let route = NetRoute::default();
    let server = mock_http(|_| MockReply::json(200, OPENAI_CHAT)).await;
    let text = complete(
        &endpoint(
            AiProtocol::OpenaiCompatible,
            leak(server.url()),
            Some("k"),
            &route,
        ),
        "m",
        Some("Be brief."),
        "Hi",
        Some(100),
    )
    .await
    .unwrap();
    assert_eq!(text, "OK");
    assert_eq!(
        server.last().json(),
        json!({"model":"m","messages":[{"role":"system","content":"Be brief."},{"role":"user","content":"Hi"}],"stream":false,"max_tokens":100})
    );

    let server = mock_http(|_| {
        MockReply::json(
            200,
            r#"{"content":[{"type":"thinking","text":"x"},{"type":"text","text":"A"},{"type":"text","text":"B"}]}"#,
        )
    })
    .await;
    let text = complete(
        &endpoint(AiProtocol::Anthropic, leak(server.url()), Some("k"), &route),
        "m",
        Some("S"),
        "U",
        None,
    )
    .await
    .unwrap();
    assert_eq!(text, "AB");
    assert_eq!(
        server.last().json(),
        json!({"model":"m","max_tokens":1024,"system":"S","messages":[{"role":"user","content":"U"}]})
    );
}

#[test]
fn statuses_map_to_fixed_codes() {
    use ErrorCode::*;
    let cases = [
        (401, false, AiKeyRejected),
        (403, true, AiKeyRejected),
        (402, false, AiQuotaExceeded),
        (404, true, AiModelNotFound),
        (404, false, AiRequestRejected),
        (429, true, AiRateLimited),
        (400, true, AiRequestRejected),
        (422, false, AiRequestRejected),
        (301, true, AiRequestRejected),
        (307, false, AiRequestRejected),
        (500, true, AiUnavailable),
        (503, false, AiUnavailable),
        (204, true, AiInvalidResponse),
    ];
    for (status, chat, code) in cases {
        assert_eq!(status_error(status, chat), code, "{status} chat={chat}");
    }
}

#[tokio::test]
async fn error_statuses_come_back_as_codes_without_the_provider_text() {
    let route = NetRoute::default();
    for (status, listing, chat) in [
        (401, ErrorCode::AiKeyRejected, ErrorCode::AiKeyRejected),
        (402, ErrorCode::AiQuotaExceeded, ErrorCode::AiQuotaExceeded),
        (429, ErrorCode::AiRateLimited, ErrorCode::AiRateLimited),
        (
            400,
            ErrorCode::AiRequestRejected,
            ErrorCode::AiRequestRejected,
        ),
        (502, ErrorCode::AiUnavailable, ErrorCode::AiUnavailable),
    ] {
        let server = mock_http(move |_| {
            MockReply::json(status, r#"{"error":{"message":"secret provider text"}}"#)
        })
        .await;
        let ep = endpoint(
            AiProtocol::OpenaiCompatible,
            leak(server.url()),
            Some("k"),
            &route,
        );
        assert_eq!(list_models(&ep).await.unwrap_err(), listing);
        assert_eq!(test(&ep, "m").await.unwrap_err(), chat);
    }
    let server = mock_http(|_| MockReply::json(404, "{}")).await;
    let ep = endpoint(
        AiProtocol::OpenaiCompatible,
        leak(server.url()),
        None,
        &route,
    );
    assert_eq!(
        test(&ep, "nope").await.unwrap_err(),
        ErrorCode::AiModelNotFound
    );
}

#[tokio::test]
async fn a_missing_model_list_is_not_an_error() {
    let route = NetRoute::default();
    for status in [404, 405] {
        let server = mock_http(move |_| MockReply::status(status)).await;
        let ep = endpoint(
            AiProtocol::OpenaiCompatible,
            leak(server.url()),
            None,
            &route,
        );
        assert_eq!(list_models(&ep).await.unwrap(), None);
    }
}

#[tokio::test]
async fn redirects_are_not_followed() {
    let route = NetRoute::default();
    let other = mock_http(|_| MockReply::json(200, OPENAI_LIST)).await;
    let target = format!("{}/models", other.url());
    let server = mock_http(move |_| MockReply::status(302).header("Location", &target)).await;
    let ep = endpoint(
        AiProtocol::OpenaiCompatible,
        leak(server.url()),
        Some("k"),
        &route,
    );
    assert_eq!(
        list_models(&ep).await.unwrap_err(),
        ErrorCode::AiRequestRejected
    );
    assert_eq!(
        test(&ep, "m").await.unwrap_err(),
        ErrorCode::AiRequestRejected
    );
    assert_eq!(other.count(), 0, "the key must not follow a redirect");
}

#[tokio::test]
async fn wrong_shapes_and_oversized_answers_are_invalid_responses() {
    let route = NetRoute::default();
    for body in ["not json", "{}", r#"{"data":"x"}"#, r#"[1]"#] {
        let server = mock_http(move |_| MockReply::json(200, body)).await;
        let ep = endpoint(
            AiProtocol::OpenaiCompatible,
            leak(server.url()),
            None,
            &route,
        );
        assert_eq!(
            list_models(&ep).await.unwrap_err(),
            ErrorCode::AiInvalidResponse,
            "{body}"
        );
    }
    for (protocol, body) in [
        (AiProtocol::OpenaiCompatible, r#"{"content":[]}"#),
        (AiProtocol::OpenaiCompatible, r#"{"choices":"x"}"#),
        (AiProtocol::Anthropic, r#"{"choices":[]}"#),
        (AiProtocol::Anthropic, "oops"),
    ] {
        let server = mock_http(move |_| MockReply::json(200, body)).await;
        let ep = endpoint(protocol, leak(server.url()), None, &route);
        assert_eq!(
            test(&ep, "m").await.unwrap_err(),
            ErrorCode::AiInvalidResponse,
            "{body}"
        );
    }
    // An empty `choices` array is the right shape for a test, but no text.
    let server = mock_http(|_| MockReply::json(200, r#"{"choices":[]}"#)).await;
    let ep = endpoint(
        AiProtocol::OpenaiCompatible,
        leak(server.url()),
        None,
        &route,
    );
    assert!(test(&ep, "m").await.is_ok());
    assert_eq!(
        complete(&ep, "m", None, "x", None).await.unwrap_err(),
        ErrorCode::AiInvalidResponse
    );
    let big = "x".repeat(MAX_RESPONSE_BYTES + 1);
    let server = mock_http(move |_| MockReply::json(200, &big)).await;
    let ep = endpoint(
        AiProtocol::OpenaiCompatible,
        leak(server.url()),
        None,
        &route,
    );
    assert_eq!(
        list_models(&ep).await.unwrap_err(),
        ErrorCode::AiInvalidResponse
    );
}

#[tokio::test]
async fn network_and_proxy_failures_have_their_own_codes() {
    let dead = dead_port().await;
    let base = format!("http://127.0.0.1:{dead}");
    let direct = NetRoute::default();
    assert_eq!(
        list_models(&endpoint(
            AiProtocol::OpenaiCompatible,
            &base,
            None,
            &direct
        ))
        .await
        .unwrap_err(),
        ErrorCode::AiNetwork
    );
    // A dead proxy is an error; the request never goes out directly.
    let server = mock_http(|_| MockReply::json(200, OPENAI_LIST)).await;
    let proxied = proxy_route(ProxyKind::Socks5, dead_port().await);
    assert_eq!(
        list_models(&endpoint(
            AiProtocol::OpenaiCompatible,
            leak(server.url()),
            None,
            &proxied
        ))
        .await
        .unwrap_err(),
        ErrorCode::AiProxy
    );
    assert_eq!(server.count(), 0);
}

#[tokio::test]
async fn requests_can_go_through_a_proxy() {
    let server = mock_http(|_| MockReply::json(200, OPENAI_LIST)).await;
    let (proxy_port, targets) = crate::net::test_support::fake_http_connect(server.port).await;
    let route = proxy_route(ProxyKind::HttpConnect, proxy_port);
    let base = "http://models.invalid:8080/v1";
    let models = list_models(&endpoint(AiProtocol::OpenaiCompatible, base, None, &route))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(models.len(), 3);
    assert!(targets.lock().unwrap()[0].starts_with("CONNECT models.invalid:8080"));
    assert_eq!(server.last().header("host"), Some("models.invalid:8080"));
}

#[test]
fn debug_output_never_contains_the_key() {
    let route = NetRoute::default();
    let ep = endpoint(
        AiProtocol::Anthropic,
        "https://example.com/v1",
        Some("sk-very-secret"),
        &route,
    );
    assert!(!format!("{ep:?}").contains("sk-very-secret"));
    let headers = auth_headers(&ep).unwrap();
    assert!(!format!("{headers:?}").contains("sk-very-secret"));
    let headers = auth_headers(&endpoint(
        AiProtocol::OpenaiCompatible,
        "https://e",
        Some("sk-very-secret"),
        &route,
    ))
    .unwrap();
    assert!(!format!("{headers:?}").contains("sk-very-secret"));
}
