use std::{sync::atomic::Ordering, time::Duration};

use serde_json::json;

use super::*;
use crate::{
    accounts::types::ProxyKind,
    net::{
        NetRoute,
        test_support::{MockStream, StreamReply, dead_port, mock_stream, proxy_route},
    },
};

const LIMITS: StreamLimits = StreamLimits {
    first_byte: Duration::from_secs(5),
    idle: Duration::from_secs(5),
    // Generous: a stream cut into single bytes is still fast locally, but a
    // slow CI machine should not turn this into a timeout.
    total: Duration::from_secs(90),
    max_bytes: 1024 * 1024,
};

fn data(value: serde_json::Value) -> Vec<u8> {
    format!("data: {value}\n\n").into_bytes()
}

fn named(event: &str, value: serde_json::Value) -> Vec<u8> {
    format!("event: {event}\ndata: {value}\n\n").into_bytes()
}

fn done() -> Vec<u8> {
    b"data: [DONE]\n\n".to_vec()
}

/// All events as one byte string, cut into pieces of `size` bytes.
fn pieces(events: Vec<Vec<u8>>, size: usize) -> Vec<Vec<u8>> {
    let all: Vec<u8> = events.concat();
    all.chunks(size).map(<[u8]>::to_vec).collect()
}

async fn serve(chunks: Vec<Vec<u8>>) -> MockStream {
    mock_stream(move |_| StreamReply::sse_chunks(chunks.clone())).await
}

struct Run {
    events: Vec<StreamEvent>,
    result: Result<(), ErrorCode>,
}

impl Run {
    fn text(&self) -> String {
        self.events
            .iter()
            .filter_map(|e| match e {
                StreamEvent::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect()
    }

    fn calls(&self) -> Vec<&ToolCallRequest> {
        self.events
            .iter()
            .filter_map(|e| match e {
                StreamEvent::ToolCall(c) => Some(c),
                _ => None,
            })
            .collect()
    }

    fn finished(&self) -> Option<(FinishReason, Option<Usage>)> {
        match self.events.last() {
            Some(StreamEvent::Finished { reason, usage }) => Some((*reason, usage.clone())),
            _ => None,
        }
    }
}

async fn run_with(
    protocol: AiProtocol,
    server: &MockStream,
    request: &ChatRequest<'_>,
    limits: StreamLimits,
    route: &NetRoute,
) -> Run {
    let base = format!("{}/v1", server.http.url());
    let endpoint = Endpoint {
        protocol,
        base_url: &base,
        key: Some("sk-test"),
        route,
    };
    let mut events = Vec::new();
    let result = stream_chat(&endpoint, request, limits, |e| events.push(e)).await;
    Run { events, result }
}

fn origin() -> ReasoningOrigin {
    ReasoningOrigin {
        protocol: AiProtocol::OpenaiCompatible,
        connection_id: "00000000-0000-4000-8000-000000000001".into(),
        base_url: "https://ai.example/v1".into(),
        model: "m".into(),
    }
}

fn user(text: &str) -> Vec<ChatMessage> {
    vec![ChatMessage::User(text.into())]
}

async fn go(protocol: AiProtocol, server: &MockStream) -> Run {
    let messages = user("hi");
    let request = ChatRequest {
        model: "m",
        system: None,
        messages: &messages,
        tools: &[],
        max_output: None,
        origin: &origin(),
    };
    run_with(protocol, server, &request, LIMITS, &NetRoute::default()).await
}

const OPENAI: AiProtocol = AiProtocol::OpenaiCompatible;
const ANTHROPIC: AiProtocol = AiProtocol::Anthropic;

fn oa_text(text: &str) -> Vec<u8> {
    data(json!({"choices":[{"index":0,"delta":{"content":text}}]}))
}

fn oa_finish(reason: &str) -> Vec<u8> {
    data(json!({"choices":[{"index":0,"delta":{},"finish_reason":reason}]}))
}

fn oa_call(index: u64, id: Option<&str>, name: Option<&str>, args: &str) -> Vec<u8> {
    let mut call = json!({"index": index, "type": "function", "function": {"arguments": args}});
    if let Some(id) = id {
        call["id"] = json!(id);
    }
    if let Some(name) = name {
        call["function"]["name"] = json!(name);
    }
    data(json!({"choices":[{"index":0,"delta":{"tool_calls":[call]}}]}))
}

fn call(id: &str, name: &str, arguments: serde_json::Value) -> ToolCallRequest {
    ToolCallRequest {
        id: id.into(),
        name: name.into(),
        arguments,
    }
}

// ---- OpenAI-compatible ----

#[tokio::test]
async fn openai_streams_text_split_inside_utf8_and_reports_usage() {
    let events = vec![
        data(json!({"choices":[{"delta":{"role":"assistant","content":""}}]})),
        data(json!({"choices":[{"delta":{"reasoning_content":"hmm","content":"你好"}}],"x":1})),
        oa_text(", \u{1F600} world"),
        oa_finish("stop"),
        data(json!({"choices":[],"usage":{"prompt_tokens":12,"completion_tokens":5}})),
        done(),
    ];
    for size in [1, 2, 3, 7, 1000] {
        let server = serve(pieces(events.clone(), size)).await;
        let run = go(OPENAI, &server).await;
        assert_eq!(run.result, Ok(()), "size {size}");
        assert_eq!(run.text(), "你好, \u{1F600} world", "size {size}");
        assert_eq!(
            run.finished(),
            Some((
                FinishReason::Stop,
                Some(Usage {
                    input_tokens: Some(12),
                    output_tokens: Some(5)
                })
            ))
        );
    }
}

#[tokio::test]
async fn openai_assembles_a_tool_call_from_fragments() {
    let server = serve(pieces(
        vec![
            oa_text("Looking."),
            oa_call(0, Some("call_1"), Some("search_mail"), ""),
            oa_call(0, None, None, "{\"que"),
            oa_call(0, None, None, "ry\":\"你"),
            oa_call(0, None, None, "好\",\"limit\":3}"),
            oa_finish("tool_calls"),
            done(),
        ],
        5,
    ))
    .await;
    let run = go(OPENAI, &server).await;
    assert_eq!(run.result, Ok(()));
    assert_eq!(
        run.events,
        [
            StreamEvent::Text("Looking.".into()),
            StreamEvent::ToolCall(call(
                "call_1",
                "search_mail",
                json!({"query":"你好","limit":3})
            )),
            StreamEvent::Finished {
                reason: FinishReason::ToolCalls,
                usage: None
            },
        ]
    );
}

#[tokio::test]
async fn openai_keeps_parallel_tool_calls_apart_in_index_order() {
    let server = serve(vec![
        oa_call(1, Some("b"), Some("second"), "{\"y\":"),
        oa_call(0, Some("a"), Some("first"), "{\"x\":"),
        oa_call(1, None, None, "2}"),
        oa_call(0, None, None, "1}"),
        oa_finish("tool_calls"),
        done(),
    ])
    .await;
    let run = go(OPENAI, &server).await;
    assert_eq!(run.result, Ok(()));
    assert_eq!(
        run.calls(),
        [
            &call("a", "first", json!({"x":1})),
            &call("b", "second", json!({"y":2}))
        ]
    );
    assert_eq!(run.finished().unwrap().0, FinishReason::ToolCalls);
}

#[tokio::test]
async fn openai_tool_calls_without_index_start_at_each_new_id() {
    let fragment = |id: Option<&str>, name: Option<&str>, args: &str| {
        let mut c = json!({"function": {"arguments": args}});
        if let Some(id) = id {
            c["id"] = json!(id);
        }
        if let Some(name) = name {
            c["function"]["name"] = json!(name);
        }
        data(json!({"choices":[{"delta":{"tool_calls":[c]}}]}))
    };
    let server = serve(vec![
        fragment(Some("a"), Some("one"), "{\"k\":"),
        fragment(None, None, "1}"),
        fragment(Some("b"), Some("two"), "{}"),
        oa_finish("stop"),
        done(),
    ])
    .await;
    let run = go(OPENAI, &server).await;
    assert_eq!(
        run.calls(),
        [
            &call("a", "one", json!({"k":1})),
            &call("b", "two", json!({}))
        ]
    );
    // A server that says `stop` after tool calls still means tool calls.
    assert_eq!(run.finished().unwrap().0, FinishReason::ToolCalls);
}

#[tokio::test]
async fn openai_empty_tool_arguments_become_an_empty_object() {
    let server = serve(vec![
        data(json!({"choices":[{"delta":{"tool_calls":[
            {"index":0,"id":"c","function":{"name":"list_accounts"}}]}}]})),
        oa_finish("tool_calls"),
        done(),
    ])
    .await;
    let run = go(OPENAI, &server).await;
    assert_eq!(run.calls(), [&call("c", "list_accounts", json!({}))]);
}

#[tokio::test]
async fn openai_request_has_the_expected_shape() {
    let server = serve(vec![oa_text("ok"), oa_finish("stop"), done()]).await;
    let messages = vec![
        ChatMessage::User("find it".into()),
        ChatMessage::Assistant {
            text: String::new(),
            tool_calls: vec![
                call("c1", "search", json!({"q":"x"})),
                call("c2", "list", json!({})),
            ],
            reasoning: None,
        },
        ChatMessage::ToolResult {
            call_id: "c1".into(),
            content: "3 hits".into(),
            is_error: false,
        },
        ChatMessage::ToolResult {
            call_id: "c2".into(),
            content: "boom".into(),
            is_error: true,
        },
        ChatMessage::Assistant {
            text: "Done.".into(),
            tool_calls: vec![],
            reasoning: None,
        },
        ChatMessage::User("thanks".into()),
    ];
    let tools = vec![ToolDefinition {
        name: "search".into(),
        description: "Search mail".into(),
        input_schema: json!({"type":"object","properties":{"q":{"type":"string"}}}),
    }];
    let request = ChatRequest {
        model: "gpt-x",
        system: Some("Be brief."),
        messages: &messages,
        tools: &tools,
        max_output: None,
        origin: &origin(),
    };
    let run = run_with(OPENAI, &server, &request, LIMITS, &NetRoute::default()).await;
    assert_eq!(run.result, Ok(()));
    let recorded = server.http.last();
    assert_eq!(recorded.method, "POST");
    assert_eq!(recorded.target, "/v1/chat/completions");
    assert_eq!(recorded.header("accept"), Some("text/event-stream"));
    assert_eq!(recorded.header("authorization"), Some("Bearer sk-test"));
    assert_eq!(recorded.header("x-api-key"), None);
    assert_eq!(
        recorded.json(),
        json!({
            "model": "gpt-x",
            "stream": true,
            "messages": [
                {"role":"system","content":"Be brief."},
                {"role":"user","content":"find it"},
                {"role":"assistant","content":null,"tool_calls":[
                    {"id":"c1","type":"function","function":{"name":"search","arguments":"{\"q\":\"x\"}"}},
                    {"id":"c2","type":"function","function":{"name":"list","arguments":"{}"}}]},
                {"role":"tool","tool_call_id":"c1","content":"3 hits"},
                {"role":"tool","tool_call_id":"c2","content":"boom"},
                {"role":"assistant","content":"Done."},
                {"role":"user","content":"thanks"}
            ],
            "tools": [{"type":"function","function":{
                "name":"search","description":"Search mail",
                "parameters":{"type":"object","properties":{"q":{"type":"string"}}}}}]
        })
    );
    assert!(recorded.json().get("stream_options").is_none());

    // No tools and a cap.
    let request = ChatRequest {
        model: "m",
        system: None,
        messages: &user("hi"),
        tools: &[],
        max_output: Some(256),
        origin: &origin(),
    };
    run_with(OPENAI, &server, &request, LIMITS, &NetRoute::default()).await;
    assert_eq!(
        server.http.last().json(),
        json!({"model":"m","stream":true,"max_tokens":256,
               "messages":[{"role":"user","content":"hi"}]})
    );
}

#[tokio::test]
async fn no_thinking_switch_is_sent_on_either_protocol() {
    let server = serve(vec![oa_text("x"), oa_finish("stop"), done()]).await;
    let request = ChatRequest {
        model: "deepseek-chat",
        system: None,
        messages: &user("hi"),
        tools: &[],
        max_output: Some(4096),
        origin: &origin(),
    };
    run_with(OPENAI, &server, &request, LIMITS, &NetRoute::default()).await;
    let body = server.http.last().json();
    assert!(body.get("thinking").is_none(), "{body}");
    assert!(body.get("reasoning_effort").is_none(), "{body}");

    let server = serve(vec![
        an_start(1),
        an_block(0, json!({"type": "text", "text": ""})),
        an_text(0, "x"),
        an_stop(0),
        an_end("end_turn", 2),
    ])
    .await;
    run_with(ANTHROPIC, &server, &request, LIMITS, &NetRoute::default()).await;
    assert!(server.http.last().json().get("thinking").is_none());
}

#[tokio::test]
async fn openai_maps_finish_reasons() {
    for (reason, expected) in [
        ("stop", FinishReason::Stop),
        ("length", FinishReason::Length),
        ("tool_calls", FinishReason::ToolCalls),
        ("function_call", FinishReason::ToolCalls),
        ("content_filter", FinishReason::Other),
    ] {
        let server = serve(vec![oa_text("x"), oa_finish(reason), done()]).await;
        let run = go(OPENAI, &server).await;
        assert_eq!(run.finished().unwrap().0, expected, "{reason}");
    }
}

#[tokio::test]
async fn openai_error_data_maps_to_a_fixed_code_after_earlier_text() {
    for (kind, code) in [
        ("rate_limit_error", ErrorCode::AiRateLimited),
        ("server_error", ErrorCode::AiUnavailable),
    ] {
        let server = serve(vec![
            oa_text("partial"),
            data(json!({"error":{"type":kind,"message":"secret provider text"}})),
        ])
        .await;
        let run = go(OPENAI, &server).await;
        assert_eq!(run.result, Err(code));
        assert_eq!(run.text(), "partial");
    }
}

#[tokio::test]
async fn non_success_statuses_map_to_the_usual_codes() {
    for (status, code) in [
        (401, ErrorCode::AiKeyRejected),
        (402, ErrorCode::AiQuotaExceeded),
        (404, ErrorCode::AiModelNotFound),
        (429, ErrorCode::AiRateLimited),
        (400, ErrorCode::AiRequestRejected),
        (503, ErrorCode::AiUnavailable),
    ] {
        for protocol in [OPENAI, ANTHROPIC] {
            let server = mock_stream(move |_| {
                StreamReply::sse_chunks([br#"{"error":"nope"}"#.to_vec()]).status(status)
            })
            .await;
            let run = go(protocol, &server).await;
            assert_eq!(run.result, Err(code), "{status}");
            assert!(run.events.is_empty());
        }
    }
}

#[tokio::test]
async fn openai_malformed_json_is_an_invalid_response() {
    let server = serve(vec![oa_text("a"), b"data: {not json\n\n".to_vec()]).await;
    let run = go(OPENAI, &server).await;
    assert_eq!(run.result, Err(ErrorCode::AiInvalidResponse));
    assert_eq!(run.text(), "a");

    let server = serve(vec![
        oa_call(0, Some("c"), Some("t"), "{\"a\":"),
        oa_finish("tool_calls"),
        done(),
    ])
    .await;
    let run = go(OPENAI, &server).await;
    assert_eq!(run.result, Err(ErrorCode::AiInvalidResponse));
    assert!(run.calls().is_empty());
}

#[tokio::test]
async fn openai_premature_end() {
    // Nothing arrived: invalid.
    let server = serve(vec![b": hello\n\n".to_vec()]).await;
    assert_eq!(
        go(OPENAI, &server).await.result,
        Err(ErrorCode::AiInvalidResponse)
    );
    let server = serve(vec![done()]).await;
    assert_eq!(
        go(OPENAI, &server).await.result,
        Err(ErrorCode::AiInvalidResponse)
    );
    // Text arrived but no marker: finished with `Other`.
    let server = serve(vec![oa_text("abc")]).await;
    let run = go(OPENAI, &server).await;
    assert_eq!(run.result, Ok(()));
    assert_eq!(run.text(), "abc");
    assert_eq!(run.finished(), Some((FinishReason::Other, None)));
    // A finish reason but no `[DONE]` is fine and keeps its reason.
    let server = serve(vec![oa_text("abc"), oa_finish("stop")]).await;
    assert_eq!(
        go(OPENAI, &server).await.finished(),
        Some((FinishReason::Stop, None))
    );
    // A final event without its blank line still counts.
    let server = serve(vec![
        oa_text("a"),
        b"data: {\"choices\":[{\"delta\":{\"content\":\"b\"}}]}".to_vec(),
    ])
    .await;
    assert_eq!(go(OPENAI, &server).await.text(), "ab");
}

// ---- Anthropic ----

fn an_start(input: u32) -> Vec<u8> {
    named(
        "message_start",
        json!({"type":"message_start","message":{"id":"m","role":"assistant","content":[],
               "usage":{"input_tokens":input,"output_tokens":1}}}),
    )
}

fn an_block(index: u64, block: serde_json::Value) -> Vec<u8> {
    named(
        "content_block_start",
        json!({"type":"content_block_start","index":index,"content_block":block}),
    )
}

fn an_delta(index: u64, delta: serde_json::Value) -> Vec<u8> {
    named(
        "content_block_delta",
        json!({"type":"content_block_delta","index":index,"delta":delta}),
    )
}

fn an_text(index: u64, text: &str) -> Vec<u8> {
    an_delta(index, json!({"type":"text_delta","text":text}))
}

fn an_json(index: u64, part: &str) -> Vec<u8> {
    an_delta(
        index,
        json!({"type":"input_json_delta","partial_json":part}),
    )
}

fn an_stop(index: u64) -> Vec<u8> {
    named(
        "content_block_stop",
        json!({"type":"content_block_stop","index":index}),
    )
}

fn an_end(reason: &str, output: u32) -> Vec<u8> {
    let mut out = named(
        "message_delta",
        json!({"type":"message_delta","delta":{"stop_reason":reason,"stop_sequence":null},
               "usage":{"output_tokens":output}}),
    );
    out.extend(named("message_stop", json!({"type":"message_stop"})));
    out
}

fn an_ping() -> Vec<u8> {
    named("ping", json!({"type":"ping"}))
}

#[tokio::test]
async fn anthropic_streams_text_after_thinking_and_reports_usage() {
    let events = vec![
        an_start(25),
        an_ping(),
        an_block(0, json!({"type":"thinking","thinking":""})),
        an_delta(0, json!({"type":"thinking_delta","thinking":"hmm"})),
        an_delta(0, json!({"type":"signature_delta","signature":"abc"})),
        an_stop(0),
        an_block(1, json!({"type":"text","text":""})),
        an_text(1, "你好"),
        an_ping(),
        an_text(1, ", \u{1F600}!"),
        an_stop(1),
        an_end("end_turn", 15),
    ];
    for size in [1, 2, 3, 11, 5000] {
        let server = serve(pieces(events.clone(), size)).await;
        let run = go(ANTHROPIC, &server).await;
        assert_eq!(run.result, Ok(()), "size {size}");
        assert_eq!(run.text(), "你好, \u{1F600}!");
        assert_eq!(
            run.finished(),
            Some((
                FinishReason::Stop,
                Some(Usage {
                    input_tokens: Some(25),
                    output_tokens: Some(15)
                })
            ))
        );
    }
}

#[tokio::test]
async fn anthropic_assembles_tool_use_blocks_and_emits_them_at_block_stop() {
    let server = serve(pieces(
        vec![
            an_start(3),
            an_block(0, json!({"type":"text","text":""})),
            an_text(0, "Searching."),
            an_stop(0),
            an_block(
                1,
                json!({"type":"tool_use","id":"toolu_1","name":"search_mail","input":{}}),
            ),
            an_json(1, ""),
            an_json(1, "{\"query\": \"你"),
            an_json(1, "好\"}"),
            an_stop(1),
            an_block(
                2,
                json!({"type":"tool_use","id":"toolu_2","name":"list_accounts","input":{}}),
            ),
            an_json(2, ""),
            an_stop(2),
            an_end("tool_use", 40),
        ],
        6,
    ))
    .await;
    let run = go(ANTHROPIC, &server).await;
    assert_eq!(run.result, Ok(()));
    assert_eq!(
        run.events[..3],
        [
            StreamEvent::Text("Searching.".into()),
            StreamEvent::ToolCall(call("toolu_1", "search_mail", json!({"query":"你好"}))),
            StreamEvent::ToolCall(call("toolu_2", "list_accounts", json!({}))),
        ]
    );
    assert_eq!(run.finished().unwrap().0, FinishReason::ToolCalls);
}

#[tokio::test]
async fn anthropic_request_has_the_expected_shape() {
    let server = serve(vec![an_start(1), an_end("end_turn", 1)]).await;
    let messages = vec![
        ChatMessage::User("find it".into()),
        ChatMessage::Assistant {
            text: "On it.".into(),
            tool_calls: vec![
                call("c1", "search", json!({"q":"x"})),
                call("c2", "list", json!({})),
            ],
            reasoning: None,
        },
        ChatMessage::ToolResult {
            call_id: "c1".into(),
            content: "3 hits".into(),
            is_error: false,
        },
        ChatMessage::ToolResult {
            call_id: "c2".into(),
            content: "boom".into(),
            is_error: true,
        },
        ChatMessage::Assistant {
            text: String::new(),
            tool_calls: vec![call("c3", "list", json!({}))],
            reasoning: None,
        },
        ChatMessage::ToolResult {
            call_id: "c3".into(),
            content: "ok".into(),
            is_error: false,
        },
    ];
    let tools = vec![ToolDefinition {
        name: "search".into(),
        description: "Search mail".into(),
        input_schema: json!({"type":"object"}),
    }];
    let request = ChatRequest {
        model: "claude-x",
        system: Some("Be brief."),
        messages: &messages,
        tools: &tools,
        max_output: None,
        origin: &origin(),
    };
    let run = run_with(ANTHROPIC, &server, &request, LIMITS, &NetRoute::default()).await;
    assert_eq!(run.result, Ok(()));
    let recorded = server.http.last();
    assert_eq!(recorded.target, "/v1/messages");
    assert_eq!(recorded.header("accept"), Some("text/event-stream"));
    assert_eq!(recorded.header("x-api-key"), Some("sk-test"));
    assert_eq!(recorded.header("anthropic-version"), Some("2023-06-01"));
    assert_eq!(recorded.header("authorization"), None);
    assert_eq!(
        recorded.json(),
        json!({
            "model": "claude-x",
            "max_tokens": 4096,
            "stream": true,
            "system": "Be brief.",
            "messages": [
                {"role":"user","content":"find it"},
                {"role":"assistant","content":[
                    {"type":"text","text":"On it."},
                    {"type":"tool_use","id":"c1","name":"search","input":{"q":"x"}},
                    {"type":"tool_use","id":"c2","name":"list","input":{}}]},
                {"role":"user","content":[
                    {"type":"tool_result","tool_use_id":"c1","content":"3 hits","is_error":false},
                    {"type":"tool_result","tool_use_id":"c2","content":"boom","is_error":true}]},
                {"role":"assistant","content":[
                    {"type":"tool_use","id":"c3","name":"list","input":{}}]},
                {"role":"user","content":[
                    {"type":"tool_result","tool_use_id":"c3","content":"ok","is_error":false}]}
            ],
            "tools": [{"name":"search","description":"Search mail","input_schema":{"type":"object"}}]
        })
    );

    let request = ChatRequest {
        model: "m",
        system: None,
        messages: &user("hi"),
        tools: &[],
        max_output: Some(100),
        origin: &origin(),
    };
    run_with(ANTHROPIC, &server, &request, LIMITS, &NetRoute::default()).await;
    assert_eq!(
        server.http.last().json(),
        json!({"model":"m","max_tokens":100,"stream":true,
               "messages":[{"role":"user","content":"hi"}]})
    );
}

#[tokio::test]
async fn anthropic_maps_stop_reasons() {
    for (reason, expected) in [
        ("end_turn", FinishReason::Stop),
        ("stop_sequence", FinishReason::Stop),
        ("tool_use", FinishReason::ToolCalls),
        ("max_tokens", FinishReason::Length),
        ("refusal", FinishReason::Other),
    ] {
        let server = serve(vec![an_start(1), an_end(reason, 2)]).await;
        let run = go(ANTHROPIC, &server).await;
        assert_eq!(run.finished().unwrap().0, expected, "{reason}");
    }
}

#[tokio::test]
async fn anthropic_error_events_map_to_fixed_codes() {
    for (kind, code) in [
        ("overloaded_error", ErrorCode::AiUnavailable),
        ("rate_limit_error", ErrorCode::AiRateLimited),
        ("api_error", ErrorCode::AiUnavailable),
    ] {
        let server = serve(vec![
            an_start(1),
            an_block(0, json!({"type":"text","text":""})),
            an_text(0, "partial"),
            named(
                "error",
                json!({"type":"error","error":{"type":kind,"message":"secret"}}),
            ),
        ])
        .await;
        let run = go(ANTHROPIC, &server).await;
        assert_eq!(run.result, Err(code), "{kind}");
        assert_eq!(run.text(), "partial");
    }
}

#[tokio::test]
async fn anthropic_malformed_and_premature_streams() {
    let server = serve(vec![an_start(1), b"data: {oops\n\n".to_vec()]).await;
    assert_eq!(
        go(ANTHROPIC, &server).await.result,
        Err(ErrorCode::AiInvalidResponse)
    );
    // Invalid tool JSON.
    let server = serve(vec![
        an_block(0, json!({"type":"tool_use","id":"t","name":"n","input":{}})),
        an_json(0, "{\"a\":"),
        an_stop(0),
    ])
    .await;
    assert_eq!(
        go(ANTHROPIC, &server).await.result,
        Err(ErrorCode::AiInvalidResponse)
    );
    // Cut inside a tool call.
    let server = serve(vec![
        an_block(0, json!({"type":"tool_use","id":"t","name":"n","input":{}})),
        an_json(0, "{\"a\":"),
    ])
    .await;
    assert_eq!(
        go(ANTHROPIC, &server).await.result,
        Err(ErrorCode::AiInvalidResponse)
    );
    // Nothing but the start.
    let server = serve(vec![an_start(1)]).await;
    assert_eq!(
        go(ANTHROPIC, &server).await.result,
        Err(ErrorCode::AiInvalidResponse)
    );
    // Text without `message_stop`: finished with `Other`.
    let server = serve(vec![
        an_start(1),
        an_block(0, json!({"type":"text","text":"hel"})),
        an_text(0, "lo"),
    ])
    .await;
    let run = go(ANTHROPIC, &server).await;
    assert_eq!(run.result, Ok(()));
    assert_eq!(run.text(), "hello");
    assert_eq!(
        run.finished(),
        Some((
            FinishReason::Other,
            Some(Usage {
                input_tokens: Some(1),
                output_tokens: Some(1)
            })
        ))
    );
}

// ---- Limits, routes and cancellation ----

async fn closed_within(server: &MockStream, wait: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + wait;
    while tokio::time::Instant::now() < deadline {
        if server.closed.load(Ordering::SeqCst) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    false
}

#[tokio::test]
async fn a_silent_gap_longer_than_idle_times_out_and_keeps_earlier_events() {
    let server = mock_stream(|_| {
        StreamReply::sse(vec![
            (Duration::ZERO, oa_text("early")),
            (Duration::from_secs(3), oa_text("late")),
        ])
    })
    .await;
    let messages = user("hi");
    let request = ChatRequest {
        model: "m",
        system: None,
        messages: &messages,
        tools: &[],
        max_output: None,
        origin: &origin(),
    };
    let limits = StreamLimits {
        idle: Duration::from_millis(200),
        ..LIMITS
    };
    let run = run_with(OPENAI, &server, &request, limits, &NetRoute::default()).await;
    assert_eq!(run.result, Err(ErrorCode::AiTimeout));
    assert_eq!(run.text(), "early");
    // The abandoned connection is closed.
    assert!(closed_within(&server, Duration::from_secs(2)).await);
}

#[tokio::test]
async fn steady_chunks_still_hit_the_total_limit() {
    let chunks: Vec<(Duration, Vec<u8>)> = (0..100)
        .map(|_| (Duration::from_millis(50), oa_text("x")))
        .collect();
    let server = mock_stream(move |_| StreamReply::sse(chunks.clone())).await;
    let messages = user("hi");
    let request = ChatRequest {
        model: "m",
        system: None,
        messages: &messages,
        tools: &[],
        max_output: None,
        origin: &origin(),
    };
    let limits = StreamLimits {
        idle: Duration::from_secs(2),
        total: Duration::from_millis(400),
        ..LIMITS
    };
    let run = run_with(OPENAI, &server, &request, limits, &NetRoute::default()).await;
    assert_eq!(run.result, Err(ErrorCode::AiTimeout));
    assert!(!run.text().is_empty());
}

#[tokio::test]
async fn a_server_that_never_answers_times_out_at_first_byte() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let _held = tokio::spawn(async move {
        let connection = listener.accept().await;
        std::future::pending::<()>().await;
        drop(connection);
    });
    let base = format!("http://127.0.0.1:{port}");
    let route = NetRoute::default();
    let endpoint = Endpoint {
        protocol: OPENAI,
        base_url: &base,
        key: None,
        route: &route,
    };
    let messages = user("hi");
    let request = ChatRequest {
        model: "m",
        system: None,
        messages: &messages,
        tools: &[],
        max_output: None,
        origin: &origin(),
    };
    let limits = StreamLimits {
        first_byte: Duration::from_millis(200),
        ..LIMITS
    };
    let result = stream_chat(&endpoint, &request, limits, |_| {}).await;
    assert_eq!(result, Err(ErrorCode::AiTimeout));
}

#[tokio::test]
async fn a_body_over_the_cap_is_rejected() {
    let big = oa_text(&"y".repeat(500));
    let server = serve(vec![big.clone(), big]).await;
    let messages = user("hi");
    let request = ChatRequest {
        model: "m",
        system: None,
        messages: &messages,
        tools: &[],
        max_output: None,
        origin: &origin(),
    };
    let limits = StreamLimits {
        max_bytes: 700,
        ..LIMITS
    };
    let run = run_with(OPENAI, &server, &request, limits, &NetRoute::default()).await;
    assert_eq!(run.result, Err(ErrorCode::AiInvalidResponse));
    assert_eq!(run.text().len(), 500);
}

#[tokio::test]
async fn dropping_the_stream_mid_way_closes_the_connection() {
    let server = mock_stream(|_| StreamReply::sse_chunks([oa_text("first")]).hold_open()).await;
    let base = format!("{}/v1", server.http.url());
    let route = NetRoute::default();
    let endpoint = Endpoint {
        protocol: OPENAI,
        base_url: &base,
        key: None,
        route: &route,
    };
    let messages = user("hi");
    let request = ChatRequest {
        model: "m",
        system: None,
        messages: &messages,
        tools: &[],
        max_output: None,
        origin: &origin(),
    };
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    {
        let stream = stream_chat(&endpoint, &request, LIMITS, |event| {
            let _ = sender.send(event);
        });
        tokio::pin!(stream);
        tokio::select! {
            result = &mut stream => panic!("the stream should still be open: {result:?}"),
            event = receiver.recv() => {
                assert_eq!(event, Some(StreamEvent::Text("first".into())));
            }
        }
        // `stream` is dropped here, in the middle of the answer.
    }
    assert!(closed_within(&server, Duration::from_secs(2)).await);
}

#[tokio::test]
async fn a_failed_proxy_is_an_error_not_a_direct_connection() {
    let server = serve(vec![oa_text("x"), done()]).await;
    let route = proxy_route(ProxyKind::Socks5, dead_port().await);
    let messages = user("hi");
    let request = ChatRequest {
        model: "m",
        system: None,
        messages: &messages,
        tools: &[],
        max_output: None,
        origin: &origin(),
    };
    let run = run_with(OPENAI, &server, &request, LIMITS, &route).await;
    assert_eq!(run.result, Err(ErrorCode::AiProxy));
    assert_eq!(server.http.count(), 0);
}

#[test]
fn the_endpoint_debug_output_hides_the_key() {
    let route = NetRoute::default();
    let endpoint = Endpoint {
        protocol: OPENAI,
        base_url: "http://x",
        key: Some("sk-secret"),
        route: &route,
    };
    assert!(!format!("{endpoint:?}").contains("sk-secret"));
}

// ---- Reasoning ----

use crate::ai::reasoning::{
    AnthropicPart, AssistantReasoning, ReasoningEcho, ReasoningField, ReasoningSpan,
};

impl Run {
    fn reasoning(&self) -> String {
        self.events
            .iter()
            .filter_map(|e| match e {
                StreamEvent::Reasoning(t) => Some(t.as_str()),
                _ => None,
            })
            .collect()
    }

    fn echo(&self) -> Option<&ReasoningEcho> {
        self.events.iter().find_map(|e| match e {
            StreamEvent::ReasoningEcho(echo) => Some(echo),
            _ => None,
        })
    }
}

#[tokio::test]
async fn openai_streams_reasoning_content_and_remembers_the_field() {
    let server = serve(pieces(
        vec![
            data(json!({"choices":[{"delta":{"role":"assistant","reasoning_content":""}}]})),
            data(json!({"choices":[{"delta":{"reasoning_content":"先想"}}]})),
            data(json!({"choices":[{"delta":{"reasoning_content":"一下"}}]})),
            oa_call(0, Some("c1"), Some("search"), "{}"),
            oa_finish("tool_calls"),
            done(),
        ],
        4,
    ))
    .await;
    let run = go(OPENAI, &server).await;
    assert_eq!(run.result, Ok(()));
    assert_eq!(run.reasoning(), "先想一下");
    assert_eq!(
        run.echo(),
        Some(&ReasoningEcho::Field {
            field: ReasoningField::ReasoningContent,
            text: "先想一下".into()
        })
    );
    // Reasoning first, then the call, then what to send back, then the end.
    let kinds: Vec<&str> = run
        .events
        .iter()
        .map(|e| match e {
            StreamEvent::Text(_) => "text",
            StreamEvent::Reasoning(_) => "reasoning",
            StreamEvent::ReasoningEcho(_) => "echo",
            StreamEvent::ToolCall(_) => "call",
            StreamEvent::Finished { .. } => "finished",
        })
        .collect();
    assert_eq!(
        kinds,
        ["reasoning", "reasoning", "call", "echo", "finished"]
    );
}

#[tokio::test]
async fn openai_reads_a_string_reasoning_field_and_ignores_other_shapes() {
    let server = serve(vec![
        data(json!({"choices":[{"delta":{"reasoning":"hmm"}}]})),
        // Not a string: some servers send structured details; not ours to echo.
        data(json!({"choices":[{"delta":{"reasoning":{"effort":"low"}}}]})),
        oa_text("Hi"),
        oa_finish("stop"),
        done(),
    ])
    .await;
    let run = go(OPENAI, &server).await;
    assert_eq!(run.reasoning(), "hmm");
    assert_eq!(run.text(), "Hi");
    assert_eq!(
        run.echo(),
        Some(&ReasoningEcho::Field {
            field: ReasoningField::Reasoning,
            text: "hmm".into()
        })
    );

    // Nothing at all: no reasoning events and nothing to send back.
    let server = serve(vec![oa_text("Hi"), oa_finish("stop"), done()]).await;
    let run = go(OPENAI, &server).await;
    assert_eq!(run.reasoning(), "");
    assert_eq!(run.echo(), None);
}

#[tokio::test]
async fn anthropic_streams_thinking_and_keeps_blocks_in_order() {
    let events = vec![
        an_start(25),
        an_block(0, json!({"type":"thinking","thinking":""})),
        an_delta(0, json!({"type":"thinking_delta","thinking":"hm"})),
        an_delta(0, json!({"type":"thinking_delta","thinking":"m"})),
        an_delta(0, json!({"type":"signature_delta","signature":"sig-"})),
        an_delta(0, json!({"type":"signature_delta","signature":"1"})),
        an_stop(0),
        an_block(1, json!({"type":"text","text":""})),
        an_text(1, "你好"),
        an_stop(1),
        an_block(2, json!({"type":"redacted_thinking","data":"opaque=="})),
        an_stop(2),
        an_block(
            3,
            json!({"type":"tool_use","id":"toolu_1","name":"search","input":{}}),
        ),
        an_json(3, "{}"),
        an_stop(3),
        an_end("tool_use", 15),
    ];
    for size in [1, 7, 5000] {
        let server = serve(pieces(events.clone(), size)).await;
        let run = go(ANTHROPIC, &server).await;
        assert_eq!(run.result, Ok(()), "size {size}");
        assert_eq!(run.reasoning(), "hmm");
        assert_eq!(run.text(), "你好");
        // The redacted block opens a stretch with no text.
        let reasoning: Vec<&StreamEvent> = run
            .events
            .iter()
            .filter(|e| matches!(e, StreamEvent::Reasoning(_)))
            .collect();
        assert_eq!(reasoning.len(), 4, "size {size}");
        assert_eq!(
            run.echo(),
            Some(&ReasoningEcho::Blocks {
                parts: vec![
                    AnthropicPart::Thinking {
                        thinking: "hmm".into(),
                        signature: Some("sig-1".into()),
                    },
                    AnthropicPart::Text { bytes: 6 },
                    AnthropicPart::RedactedThinking {
                        data: "opaque==".into()
                    },
                    AnthropicPart::ToolUse {
                        id: "toolu_1".into()
                    },
                ]
            })
        );
        assert_eq!(run.finished().unwrap().0, FinishReason::ToolCalls);
    }

    // Without thinking there is nothing to send back.
    let server = serve(vec![
        an_start(1),
        an_block(0, json!({"type":"text","text":"x"})),
        an_stop(0),
        an_end("end_turn", 1),
    ])
    .await;
    assert_eq!(go(ANTHROPIC, &server).await.echo(), None);
}

fn with_reasoning(echo: ReasoningEcho, origin: ReasoningOrigin) -> Option<AssistantReasoning> {
    Some(AssistantReasoning {
        origin,
        echo: Some(echo),
        spans: vec![ReasoningSpan {
            bytes: 2,
            duration_ms: 1200,
            text_at: 0,
            tools_before: 0,
        }],
    })
}

#[tokio::test]
async fn openai_sends_reasoning_back_under_the_field_it_came_in() {
    let server = serve(vec![oa_text("x"), oa_finish("stop"), done()]).await;
    let mut other = origin();
    other.model = "another-model".into();
    for (field, from, sent) in [
        (ReasoningField::ReasoningContent, origin(), true),
        (ReasoningField::Reasoning, origin(), true),
        (ReasoningField::ReasoningContent, other.clone(), false),
    ] {
        let messages = vec![
            ChatMessage::User("find".into()),
            ChatMessage::Assistant {
                text: String::new(),
                tool_calls: vec![call("c1", "search", json!({}))],
                reasoning: with_reasoning(
                    ReasoningEcho::Field {
                        field,
                        text: "想想".into(),
                    },
                    from,
                ),
            },
            ChatMessage::ToolResult {
                call_id: "c1".into(),
                content: "r".into(),
                is_error: false,
            },
            ChatMessage::Assistant {
                text: "plain".into(),
                tool_calls: vec![],
                reasoning: None,
            },
        ];
        let request = ChatRequest {
            model: "m",
            system: None,
            messages: &messages,
            tools: &[],
            max_output: None,
            origin: &origin(),
        };
        run_with(OPENAI, &server, &request, LIMITS, &NetRoute::default()).await;
        let body = server.http.last().json();
        let thought = &body["messages"][1];
        let names = ["reasoning_content", "reasoning"];
        for name in names {
            let expected = sent && name == field.name();
            assert_eq!(
                thought.get(name).is_some(),
                expected,
                "{name} for {field:?}: {thought}"
            );
        }
        if sent {
            assert_eq!(thought[field.name()], "想想");
        }
        // A message without reasoning never gets the field.
        for name in names {
            assert!(body["messages"][3].get(name).is_none(), "{body}");
        }
    }
}

#[tokio::test]
async fn anthropic_sends_thinking_back_unchanged_in_its_place() {
    let server = serve(vec![an_start(1), an_end("end_turn", 1)]).await;
    let mut anthropic = origin();
    anthropic.protocol = AiProtocol::Anthropic;
    let parts = vec![
        AnthropicPart::Thinking {
            thinking: "hmm".into(),
            signature: Some("sig".into()),
        },
        AnthropicPart::Text { bytes: 4 },
        AnthropicPart::ToolUse { id: "c1".into() },
        AnthropicPart::RedactedThinking {
            data: "opaque".into(),
        },
        AnthropicPart::Thinking {
            thinking: "unsigned".into(),
            signature: None,
        },
        AnthropicPart::ToolUse { id: "c2".into() },
    ];
    let messages = |from: ReasoningOrigin| {
        vec![
            ChatMessage::User("go".into()),
            ChatMessage::Assistant {
                text: "Look".into(),
                tool_calls: vec![
                    call("c1", "search", json!({})),
                    call("c2", "list", json!({})),
                ],
                reasoning: with_reasoning(
                    ReasoningEcho::Blocks {
                        parts: parts.clone(),
                    },
                    from,
                ),
            },
            ChatMessage::ToolResult {
                call_id: "c1".into(),
                content: "a".into(),
                is_error: false,
            },
            ChatMessage::ToolResult {
                call_id: "c2".into(),
                content: "b".into(),
                is_error: false,
            },
        ]
    };
    let same = messages(anthropic.clone());
    let request = ChatRequest {
        model: "m",
        system: None,
        messages: &same,
        tools: &[],
        max_output: None,
        origin: &anthropic,
    };
    run_with(ANTHROPIC, &server, &request, LIMITS, &NetRoute::default()).await;
    assert_eq!(
        server.http.last().json()["messages"][1]["content"],
        json!([
            {"type":"thinking","thinking":"hmm","signature":"sig"},
            {"type":"text","text":"Look"},
            {"type":"tool_use","id":"c1","name":"search","input":{}},
            {"type":"redacted_thinking","data":"opaque"},
            {"type":"thinking","thinking":"unsigned"},
            {"type":"tool_use","id":"c2","name":"list","input":{}}
        ])
    );

    // From another connection: the blocks are left out, the rest stays.
    let mut elsewhere = anthropic.clone();
    elsewhere.connection_id = "00000000-0000-4000-8000-000000000002".into();
    let moved = messages(elsewhere);
    let request = ChatRequest {
        messages: &moved,
        ..request
    };
    run_with(ANTHROPIC, &server, &request, LIMITS, &NetRoute::default()).await;
    assert_eq!(
        server.http.last().json()["messages"][1]["content"],
        json!([
            {"type":"text","text":"Look"},
            {"type":"tool_use","id":"c1","name":"search","input":{}},
            {"type":"tool_use","id":"c2","name":"list","input":{}}
        ])
    );
}
