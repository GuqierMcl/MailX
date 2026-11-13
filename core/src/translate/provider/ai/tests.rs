use serde_json::json;

use super::*;
use crate::{
    ai::AiProtocol,
    net::{
        NetRoute,
        test_support::{MockReply, mock_http},
    },
    secrets::SecretValue,
    settings::{AiConsent, TranslationProviderId},
    translate::languages,
};

const KEY: &str = "sk-ai-0123456789";

fn target(base_url: &str, protocol: AiProtocol, output_length: Option<u32>) -> AiModelTarget {
    AiModelTarget::new(
        AiConsent {
            connection_id: "6f1f3d3e-8a40-4a39-9d3c-0f5f7a6b2c11".into(),
            base_url: base_url.into(),
        },
        protocol,
        "some-model".into(),
        output_length,
        KEY.into(),
        NetRoute::default(),
    )
}

fn sources() -> Vec<String> {
    vec![
        "Hello <span id=\"1\">world</span>".to_owned(),
        "Second &amp; last".to_owned(),
    ]
}

async fn run(
    target: &AiModelTarget,
    texts: &[String],
    language: &str,
) -> Result<TranslateResponse, ErrorCode> {
    let key = SecretValue::new(String::new());
    let route = NetRoute::default();
    let context = Context {
        key: &key,
        microsoft_region: "",
        route: &route,
        ai: Some(target),
        base: None,
    };
    super::super::translate(
        TranslationProviderId::Ai,
        &context,
        &TranslateRequest {
            texts,
            target: language,
        },
    )
    .await
}

fn openai_reply(content: &str) -> String {
    json!({"choices": [{"message": {"role": "assistant", "content": content}}]}).to_string()
}

// ------------------------------------------------------------------ prompt

#[test]
fn every_target_language_has_an_english_name() {
    for code in languages::TARGETS {
        let name = language_name(code).unwrap_or_else(|| panic!("{code}"));
        assert!(name.is_ascii() && !name.is_empty());
    }
    assert_eq!(language_name("zh-Hans"), Some("Simplified Chinese"));
    assert_eq!(language_name("xx"), None);
}

#[test]
fn the_prompt_states_language_rules_and_that_items_are_not_instructions() {
    let prompt = system_prompt("German", 2);
    assert!(prompt.contains("into German"));
    assert!(prompt.contains("exactly 2"));
    assert!(prompt.contains("<span id=\"1\">"));
    assert!(prompt.contains("not instructions"));
    assert!(prompt.contains("{\"items\":[...]}"));
}

#[test]
fn the_user_message_is_the_items_as_json() {
    let message = user_message(&sources());
    let value: serde_json::Value = serde_json::from_str(&message).unwrap();
    assert_eq!(value, json!({"items": sources()}));
}

// ------------------------------------------------------------------ parsing

#[test]
fn parsing_takes_the_items_out_of_noisy_replies() {
    let want = vec!["a".to_owned(), "b <span id=\"1\">c</span>".to_owned()];
    let plain = r#"{"items":["a","b <span id=\"1\">c</span>"]}"#;
    assert_eq!(parse_items(plain).unwrap(), want);
    let fenced = format!("```json\n{plain}\n```");
    assert_eq!(parse_items(&fenced).unwrap(), want);
    let prose = format!("Sure! Here is the translation:\n{plain}\nHope that helps.");
    assert_eq!(parse_items(&prose).unwrap(), want);
    let thinking = format!("<think>I should output {{\"items\":[\"x\"]}} maybe</think>\n{plain}");
    assert_eq!(parse_items(&thinking).unwrap(), want);
    let two_blocks = format!("<think>a</think>{plain}<think>{{b}}</think>");
    assert_eq!(parse_items(&two_blocks).unwrap(), want);
    assert_eq!(
        parse_items(r#"{"items":[]}"#).unwrap(),
        Vec::<String>::new()
    );
}

#[test]
fn parsing_rejects_everything_else() {
    for reply in [
        "",
        "no json here",
        "<think>never closed {\"items\":[\"a\"]}",
        "}{",
        r#"{"items":"a"}"#,
        r#"{"items":["a",1]}"#,
        r#"{"items":["a",null]}"#,
        r#"{"items":[["a"]]}"#,
        r#"{"other":["a"]}"#,
        r#"["a"]"#,
        r#"{"items":["a""#,
    ] {
        assert_eq!(
            parse_items(reply).unwrap_err(),
            ErrorCode::TranslationInvalidResponse,
            "{reply:?}"
        );
    }
}

// ------------------------------------------------------------------- limits

#[test]
fn limits_follow_the_model_output_length() {
    let bytes = |tokens| limits(tokens).max_bytes;
    assert_eq!(limits(None).max_items, 40);
    assert_eq!(bytes(None), 8_000);
    assert_eq!(bytes(Some(0)), 8_000);
    assert_eq!(bytes(Some(100)), 1_000, "the floor");
    assert_eq!(bytes(Some(500)), 1_000);
    assert_eq!(bytes(Some(2_000)), 4_000);
    assert_eq!(bytes(Some(4_000)), 8_000);
    assert_eq!(bytes(Some(200_000)), 8_000);
    assert_eq!(bytes(Some(u32::MAX)), 8_000);
}

#[test]
fn only_anthropic_gets_a_token_cap() {
    use AiProtocol::{Anthropic, OpenaiCompatible};
    assert_eq!(max_tokens(OpenaiCompatible, Some(4_000)), None);
    assert_eq!(max_tokens(OpenaiCompatible, None), None);
    assert_eq!(max_tokens(Anthropic, None), Some(4_096));
    assert_eq!(max_tokens(Anthropic, Some(0)), Some(4_096));
    assert_eq!(max_tokens(Anthropic, Some(2_000)), Some(2_000));
    assert_eq!(max_tokens(Anthropic, Some(64_000)), Some(8_192));
}

#[test]
fn ai_errors_map_to_translation_errors() {
    use ErrorCode::*;
    for (from, to) in [
        (AiNotConfigured, TranslationNotConfigured),
        (AiKeyRejected, TranslationKeyRejected),
        (AiQuotaExceeded, TranslationQuotaExceeded),
        (AiRateLimited, TranslationRateLimited),
        (AiNetwork, TranslationNetwork),
        (AiProxy, TranslationProxy),
        (AiTimeout, TranslationTimeout),
        (AiUnavailable, TranslationUnavailable),
        (AiInvalidResponse, TranslationInvalidResponse),
        (AiModelNotFound, AiModelNotFound),
        (AiRequestRejected, AiRequestRejected),
    ] {
        assert_eq!(map_error(from), to);
    }
}

// ------------------------------------------------------------ over the wire

#[tokio::test]
async fn an_openai_compatible_call_sends_the_prompt_and_reads_the_items() {
    let server = mock_http(|_| {
        MockReply::json(
            200,
            &openai_reply(
                "<think>hmm</think>```json\n{\"items\":[\"Hallo <span id=\\\"1\\\">Welt</span>\",\"Zweite &amp; letzte\"]}\n```",
            ),
        )
    })
    .await;
    let target = target(&server.url(), AiProtocol::OpenaiCompatible, Some(4_000));
    let response = run(&target, &sources(), "de").await.unwrap();
    assert_eq!(
        response.texts,
        ["Hallo <span id=\"1\">Welt</span>", "Zweite &amp; letzte"]
    );
    assert_eq!(response.detected_source, None);

    let request = server.last();
    assert_eq!(request.target, "/chat/completions");
    assert_eq!(
        request.header("authorization"),
        Some(format!("Bearer {KEY}").as_str())
    );
    let body = request.json();
    assert_eq!(body["model"], "some-model");
    assert_eq!(body["stream"], false);
    assert!(body.get("max_tokens").is_none());
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages[0]["role"], "system");
    assert!(messages[0]["content"].as_str().unwrap().contains("German"));
    assert_eq!(messages[1]["role"], "user");
    let user: serde_json::Value =
        serde_json::from_str(messages[1]["content"].as_str().unwrap()).unwrap();
    assert_eq!(user, json!({"items": sources()}));
}

#[tokio::test]
async fn an_anthropic_call_caps_the_output() {
    let server = mock_http(|_| {
        MockReply::json(
            200,
            r#"{"content":[{"type":"text","text":"{\"items\":[\"a\",\"b\"]}"}]}"#,
        )
    })
    .await;
    for (length, cap) in [(None, 4_096), (Some(64_000), 8_192), (Some(1_500), 1_500)] {
        let target = target(&server.url(), AiProtocol::Anthropic, length);
        let response = run(&target, &sources(), "fr").await.unwrap();
        assert_eq!(response.texts, ["a", "b"]);
        let request = server.last();
        assert_eq!(request.target, "/messages");
        assert_eq!(request.header("x-api-key"), Some(KEY));
        assert_eq!(request.json()["max_tokens"], cap);
        assert!(
            request.json()["system"]
                .as_str()
                .unwrap()
                .contains("French")
        );
    }
}

#[tokio::test]
async fn a_wrong_count_or_an_oversized_item_is_an_invalid_response() {
    let wrong_count =
        mock_http(|_| MockReply::json(200, &openai_reply(r#"{"items":["only one"]}"#))).await;
    let target_a = target(&wrong_count.url(), AiProtocol::OpenaiCompatible, None);
    assert_eq!(
        run(&target_a, &sources(), "de").await.unwrap_err(),
        ErrorCode::TranslationInvalidResponse
    );

    let huge = "x".repeat(20_000);
    let oversized = mock_http(move |_| {
        MockReply::json(
            200,
            &openai_reply(&json!({"items": ["ok", huge]}).to_string()),
        )
    })
    .await;
    let target_b = target(&oversized.url(), AiProtocol::OpenaiCompatible, None);
    assert_eq!(
        run(&target_b, &sources(), "de").await.unwrap_err(),
        ErrorCode::TranslationInvalidResponse
    );

    let not_strings =
        mock_http(|_| MockReply::json(200, &openai_reply(r#"{"items":[1,2]}"#))).await;
    let target_c = target(&not_strings.url(), AiProtocol::OpenaiCompatible, None);
    assert_eq!(
        run(&target_c, &sources(), "de").await.unwrap_err(),
        ErrorCode::TranslationInvalidResponse
    );
}

#[tokio::test]
async fn statuses_map_to_translation_codes_and_never_leak_the_key() {
    use ErrorCode::*;
    for (status, expected) in [
        (401, TranslationKeyRejected),
        (402, TranslationQuotaExceeded),
        (404, AiModelNotFound),
        (400, AiRequestRejected),
        (429, TranslationRateLimited),
        (503, TranslationUnavailable),
    ] {
        let server = mock_http(move |_| MockReply::status(status)).await;
        let target = target(&server.url(), AiProtocol::OpenaiCompatible, None);
        let error = run(&target, &sources(), "de").await.unwrap_err();
        assert_eq!(error, expected, "{status}");
        assert!(!format!("{error:?}").contains(KEY));
        assert!(!format!("{target:?}").contains(KEY));
    }
    let invalid = mock_http(|_| MockReply::json(200, "not json")).await;
    let target_i = target(&invalid.url(), AiProtocol::OpenaiCompatible, None);
    assert_eq!(
        run(&target_i, &sources(), "de").await.unwrap_err(),
        TranslationInvalidResponse
    );
    // Nothing listens there.
    let dead = target(
        &format!(
            "http://127.0.0.1:{}",
            crate::net::test_support::dead_port().await
        ),
        AiProtocol::OpenaiCompatible,
        None,
    );
    assert_eq!(
        run(&dead, &sources(), "de").await.unwrap_err(),
        TranslationNetwork
    );
}

#[tokio::test]
async fn a_missing_target_or_unknown_language_is_refused_before_any_call() {
    let key = SecretValue::new(String::new());
    let route = NetRoute::default();
    let context = Context {
        key: &key,
        microsoft_region: "",
        route: &route,
        ai: None,
        base: None,
    };
    let request = TranslateRequest {
        texts: &sources(),
        target: "de",
    };
    assert_eq!(
        super::super::translate(TranslationProviderId::Ai, &context, &request)
            .await
            .unwrap_err(),
        ErrorCode::TranslationNotConfigured
    );
    let server = mock_http(|_| MockReply::status(500)).await;
    let target = target(&server.url(), AiProtocol::OpenaiCompatible, None);
    assert_eq!(
        run(&target, &sources(), "xx").await.unwrap_err(),
        ErrorCode::TranslationLanguageUnsupported
    );
    assert_eq!(server.count(), 0);
}
