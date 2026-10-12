//! 各后端适配器的协议级测试。
//!
//! 覆盖的是最容易出错、且出错时最难排查的部分：流式分片的拼接、
//! 工具调用参数的累积、错误信息有没有带上上游 body。

mod support;

use std::time::Duration;

use anima_lib::llm::anthropic::AnthropicConnector;
use anima_lib::llm::ollama::OllamaConnector;
use anima_lib::llm::openai::OpenAiConnector;
use anima_lib::llm::{ChatChunk, ChatRequest, FinishReason, LlmConnector, Message, Role, ToolCall};
use futures_util::StreamExt;
use support::{spawn, spawn_many, MockSpec};

/// 把整条流收完，方便断言
async fn collect(stream: anima_lib::llm::ChatStream) -> Vec<ChatChunk> {
    stream
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .map(|r| r.expect("流中出现意外错误"))
        .collect()
}

/// 只取正文，拼成完整回答
fn text_of(chunks: &[ChatChunk]) -> String {
    chunks
        .iter()
        .filter_map(|c| match c {
            ChatChunk::Text { delta } => Some(delta.as_str()),
            _ => None,
        })
        .collect()
}

fn tool_calls_of(chunks: &[ChatChunk]) -> Vec<&ToolCall> {
    chunks
        .iter()
        .filter_map(|c| match c {
            ChatChunk::ToolCall { call } => Some(call),
            _ => None,
        })
        .collect()
}

fn finish_of(chunks: &[ChatChunk]) -> Option<FinishReason> {
    chunks.iter().rev().find_map(|c| match c {
        ChatChunk::Done { reason, .. } => Some(*reason),
        _ => None,
    })
}

/// `Result::expect_err` 要求 T: Debug，而 ChatStream 是 trait object，没法 Debug。
fn expect_stream_err(
    result: anima_lib::error::Result<anima_lib::llm::ChatStream>,
    context: &str,
) -> anima_lib::error::Error {
    match result {
        Ok(_) => panic!("{context}：期望建流失败，实际成功了"),
        Err(err) => err,
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}

fn openai(base_url: &str, supports_tools: bool) -> OpenAiConnector {
    OpenAiConnector::new(
        "test",
        base_url,
        "test-model",
        None,
        supports_tools,
        client(),
    )
}

// ---------------------------------------------------------------------------
// OpenAI / LM Studio
// ---------------------------------------------------------------------------

#[tokio::test]
async fn openai_streams_text_and_reports_usage() {
    let server = spawn(MockSpec::sse(&[
        r#"{"choices":[{"delta":{"content":"你好"}}]}"#,
        r#"{"choices":[{"delta":{"content":"，世界"}}]}"#,
        r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        r#"{"choices":[],"usage":{"prompt_tokens":12,"completion_tokens":5}}"#,
        "[DONE]",
    ]))
    .await;

    let chunks = collect(
        openai(&server.base_url, true)
            .chat_stream(ChatRequest::new(vec![Message::user("hi")]))
            .await
            .unwrap(),
    )
    .await;

    assert_eq!(text_of(&chunks), "你好，世界");
    assert_eq!(finish_of(&chunks), Some(FinishReason::Stop));

    let usage = chunks.iter().rev().find_map(|c| match c {
        ChatChunk::Done { usage, .. } => *usage,
        _ => None,
    });
    let usage = usage.expect("应当带回 usage");
    assert_eq!(usage.prompt_tokens, 12);
    assert_eq!(usage.completion_tokens, 5);
}

#[tokio::test]
async fn openai_accumulates_tool_call_arguments_across_frames() {
    // 关键场景：OpenAI 把 arguments 拆成多个片段下发，只有 index 关联它们
    let server = spawn(MockSpec::sse(&[
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"get_weather","arguments":""}}]}}]}"#,
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"city\""}}]}}]}"#,
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":":\"北京\"}"}}]}}]}"#,
        r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        "[DONE]",
    ]))
    .await;

    let chunks = collect(
        openai(&server.base_url, true)
            .chat_stream(ChatRequest::new(vec![Message::user("北京天气")]))
            .await
            .unwrap(),
    )
    .await;

    let calls = tool_calls_of(&chunks);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "call_1");
    assert_eq!(calls[0].name, "get_weather");
    // 三个片段必须拼成合法 JSON
    assert_eq!(calls[0].arguments, r#"{"city":"北京"}"#);
    assert_eq!(finish_of(&chunks), Some(FinishReason::ToolCalls));
}

#[tokio::test]
async fn openai_accumulates_multiple_parallel_tool_calls_in_order() {
    let server = spawn(MockSpec::sse(&[
        r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a","function":{"name":"first","arguments":"{}"}}]}}]}"#,
        r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"id":"b","function":{"name":"second","arguments":"{}"}}]}}]}"#,
        r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        "[DONE]",
    ]))
    .await;

    let chunks = collect(
        openai(&server.base_url, true)
            .chat_stream(ChatRequest::new(vec![Message::user("两个工具")]))
            .await
            .unwrap(),
    )
    .await;

    let calls = tool_calls_of(&chunks);
    assert_eq!(calls.len(), 2);
    // 必须按 index 顺序，否则上层把结果和调用对错
    assert_eq!(calls[0].name, "first");
    assert_eq!(calls[1].name, "second");
}

#[tokio::test]
async fn openai_reassembles_frames_split_across_chunk_boundaries() {
    // 一个 SSE 帧被切成四段跨 chunk 送达，其中还夹着多字节中文
    let server = spawn(MockSpec::raw(&[
        "data: {\"choices\":[{\"delta\":{\"content\":\"",
        "中文",
        "测试",
        "\"}}]}\n\ndata: [DONE]\n\n",
    ]))
    .await;

    let chunks = collect(
        openai(&server.base_url, true)
            .chat_stream(ChatRequest::new(vec![Message::user("hi")]))
            .await
            .unwrap(),
    )
    .await;

    assert_eq!(text_of(&chunks), "中文测试");
}

#[tokio::test]
async fn openai_skips_unparseable_frames_instead_of_failing() {
    // 本地推理服务偶尔会插入非标准帧，不该让整轮对话失败
    let server = spawn(MockSpec::sse(&[
        r#"{"choices":[{"delta":{"content":"前"}}]}"#,
        "这不是 JSON",
        r#"{"choices":[{"delta":{"content":"后"}}]}"#,
        "[DONE]",
    ]))
    .await;

    let chunks = collect(
        openai(&server.base_url, true)
            .chat_stream(ChatRequest::new(vec![Message::user("hi")]))
            .await
            .unwrap(),
    )
    .await;

    assert_eq!(text_of(&chunks), "前后");
}

#[tokio::test]
async fn openai_omits_tools_when_backend_does_not_support_them() {
    // MLX server 这类后端：supports_tools = false，请求里不该出现 tools 字段
    let server = spawn(MockSpec::sse(&["[DONE]"])).await;

    let mut req = ChatRequest::new(vec![Message::user("hi")]);
    req.tools = vec![anima_lib::llm::Tool {
        name: "noop".into(),
        description: "".into(),
        parameters: serde_json::json!({"type":"object"}),
    }];

    let _ = collect(
        openai(&server.base_url, false)
            .chat_stream(req)
            .await
            .unwrap(),
    )
    .await;

    let body = server.request_body();
    assert!(
        body.get("tools").is_none(),
        "supports_tools=false 时不应发送 tools 字段，实际请求体：{body}"
    );
}

#[tokio::test]
async fn openai_error_response_carries_upstream_body() {
    let server = spawn(MockSpec::error(
        400,
        r#"{"error":{"message":"model not loaded"}}"#,
    ))
    .await;

    let err = expect_stream_err(
        openai(&server.base_url, true)
            .chat_stream(ChatRequest::new(vec![Message::user("hi")]))
            .await,
        "400 响应",
    );

    let text = err.to_string();
    // body 必须在错误里，否则调试本地服务时只能看到光秃秃的状态码
    assert!(text.contains("400"), "错误里应含状态码：{text}");
    assert!(
        text.contains("model not loaded"),
        "错误里应含上游 body：{text}"
    );
}

#[tokio::test]
async fn openai_reports_reasoning_separately_from_content() {
    let server = spawn(MockSpec::sse(&[
        r#"{"choices":[{"delta":{"reasoning_content":"让我想想"}}]}"#,
        r#"{"choices":[{"delta":{"content":"答案是 42"}}]}"#,
        "[DONE]",
    ]))
    .await;

    let chunks = collect(
        openai(&server.base_url, true)
            .chat_stream(ChatRequest::new(vec![Message::user("hi")]))
            .await
            .unwrap(),
    )
    .await;

    assert_eq!(text_of(&chunks), "答案是 42");
    let reasoning: String = chunks
        .iter()
        .filter_map(|c| match c {
            ChatChunk::Reasoning { delta } => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(reasoning, "让我想想");
}

// ---------------------------------------------------------------------------
// Anthropic / Claude
// ---------------------------------------------------------------------------

#[tokio::test]
async fn anthropic_streams_text_and_hoists_system_prompt() {
    let server = spawn(MockSpec::sse(&[
        r#"{"type":"message_start","message":{"usage":{"input_tokens":9}}}"#,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"你好"}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"呀"}}"#,
        r#"{"type":"content_block_stop","index":0}"#,
        r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":4}}"#,
        r#"{"type":"message_stop"}"#,
    ]))
    .await;

    let connector = AnthropicConnector::new(&server.base_url, "claude-test", "sk-test", client());

    let chunks = collect(
        connector
            .chat_stream(ChatRequest::new(vec![
                Message::system("你是 Anima"),
                Message::user("hi"),
            ]))
            .await
            .unwrap(),
    )
    .await;

    assert_eq!(text_of(&chunks), "你好呀");
    assert_eq!(finish_of(&chunks), Some(FinishReason::Stop));

    // system 必须提到顶层，不能留在 messages 里
    let body = server.request_body();
    assert_eq!(body["system"], "你是 Anima");
    assert_eq!(body["messages"].as_array().unwrap().len(), 1);
    assert_eq!(body["messages"][0]["role"], "user");
}

#[tokio::test]
async fn anthropic_assembles_tool_use_from_input_json_deltas() {
    let server = spawn(MockSpec::sse(&[
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"search"}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"q\":"}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"\"anima\"}"}}"#,
        r#"{"type":"content_block_stop","index":0}"#,
        r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
        r#"{"type":"message_stop"}"#,
    ]))
    .await;

    let connector = AnthropicConnector::new(&server.base_url, "claude-test", "sk-test", client());

    let chunks = collect(
        connector
            .chat_stream(ChatRequest::new(vec![Message::user("搜一下")]))
            .await
            .unwrap(),
    )
    .await;

    let calls = tool_calls_of(&chunks);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "toolu_1");
    assert_eq!(calls[0].name, "search");
    assert_eq!(calls[0].arguments, r#"{"q":"anima"}"#);
    assert_eq!(finish_of(&chunks), Some(FinishReason::ToolCalls));
}

#[tokio::test]
async fn anthropic_maps_thinking_deltas_to_reasoning() {
    let server = spawn(MockSpec::sse(&[
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking"}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"推理中"}}"#,
        r#"{"type":"content_block_stop","index":0}"#,
        r#"{"type":"message_stop"}"#,
    ]))
    .await;

    let connector = AnthropicConnector::new(&server.base_url, "claude-test", "sk-test", client());

    let chunks = collect(
        connector
            .chat_stream(ChatRequest::new(vec![Message::user("hi")]))
            .await
            .unwrap(),
    )
    .await;

    let reasoning: String = chunks
        .iter()
        .filter_map(|c| match c {
            ChatChunk::Reasoning { delta } => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(reasoning, "推理中");
}

#[tokio::test]
async fn anthropic_ignores_unknown_event_types() {
    // 上游将来新增事件类型时不应让整条流失败
    let server = spawn(MockSpec::sse(&[
        r#"{"type":"some_future_event","payload":{}}"#,
        r#"{"type":"ping"}"#,
        r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
        r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"ok"}}"#,
        r#"{"type":"message_stop"}"#,
    ]))
    .await;

    let connector = AnthropicConnector::new(&server.base_url, "claude-test", "sk-test", client());

    let chunks = collect(
        connector
            .chat_stream(ChatRequest::new(vec![Message::user("hi")]))
            .await
            .unwrap(),
    )
    .await;

    assert_eq!(text_of(&chunks), "ok");
}

#[tokio::test]
async fn anthropic_requires_api_key() {
    let connector = AnthropicConnector::new("http://127.0.0.1:1", "claude-test", "", client());

    let err = expect_stream_err(
        connector
            .chat_stream(ChatRequest::new(vec![Message::user("hi")]))
            .await,
        "空 API Key",
    );

    assert!(err.to_string().contains("API Key"), "实际错误：{err}");
}

// ---------------------------------------------------------------------------
// Ollama
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ollama_streams_ndjson_and_finishes_on_done_flag() {
    let server = spawn(MockSpec::ndjson(&[
        r#"{"message":{"role":"assistant","content":"你"},"done":false}"#,
        r#"{"message":{"role":"assistant","content":"好"},"done":false}"#,
        r#"{"message":{"role":"assistant","content":""},"done":true,"done_reason":"stop","prompt_eval_count":7,"eval_count":3}"#,
    ]))
    .await;

    let connector = OllamaConnector::new(&server.base_url, "qwen3:32b", client());

    let chunks = collect(
        connector
            .chat_stream(ChatRequest::new(vec![Message::user("hi")]))
            .await
            .unwrap(),
    )
    .await;

    assert_eq!(text_of(&chunks), "你好");
    assert_eq!(finish_of(&chunks), Some(FinishReason::Stop));
}

#[tokio::test]
async fn ollama_uses_native_endpoint_and_object_shaped_tool_arguments() {
    let server = spawn(MockSpec::ndjson(&[
        r#"{"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"get_time","arguments":{"tz":"Asia/Shanghai"}}}]},"done":true}"#,
    ]))
    .await;

    let connector = OllamaConnector::new(&server.base_url, "qwen3:32b", client());

    let mut req = ChatRequest::new(vec![
        Message::user("几点了"),
        // 历史里的 assistant tool_call：原生协议要求 arguments 是对象而非字符串
        Message {
            role: Role::Assistant,
            content: String::new(),
            tool_calls: vec![ToolCall {
                id: "x".into(),
                name: "get_time".into(),
                arguments: r#"{"tz":"UTC"}"#.into(),
            }],
            tool_call_id: None,
            images: Vec::new(),
        },
    ]);
    req.tools = vec![anima_lib::llm::Tool {
        name: "get_time".into(),
        description: "当前时间".into(),
        parameters: serde_json::json!({"type":"object"}),
    }];

    let chunks = collect(connector.chat_stream(req).await.unwrap()).await;

    let calls = tool_calls_of(&chunks);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "get_time");
    // 适配器把对象序列化回字符串，交给上层统一处理
    assert_eq!(calls[0].arguments, r#"{"tz":"Asia/Shanghai"}"#);
    // 原生协议不给 id，适配器要自己编一个非空 id
    assert!(!calls[0].id.is_empty());

    let body = server.request_body();
    // 历史里的 arguments 必须是对象，字符串会被 Ollama 拒掉
    assert!(
        body["messages"][1]["tool_calls"][0]["function"]["arguments"].is_object(),
        "arguments 应为对象，实际：{body}"
    );
}

#[tokio::test]
async fn ollama_maps_sampling_params_into_options_object() {
    let server = spawn(MockSpec::ndjson(&[r#"{"done":true}"#])).await;

    let mut req = ChatRequest::new(vec![Message::user("hi")]);
    req.temperature = Some(0.4);
    req.max_tokens = Some(256);

    let connector = OllamaConnector::new(&server.base_url, "qwen3:32b", client());
    let _ = collect(connector.chat_stream(req).await.unwrap()).await;

    let body = server.request_body();
    // Ollama 的采样参数在 options 子对象里，且 max_tokens 叫 num_predict。
    // temperature 是 f32，转成 JSON 的 f64 会有精度尾巴（0.4 → 0.40000000596），
    // 所以比较容差而不是相等 —— 上游 API 对这点差异不敏感。
    let temp = body["options"]["temperature"].as_f64().expect("应为数值");
    assert!((temp - 0.4).abs() < 1e-6, "temperature 实际为 {temp}");
    assert_eq!(body["options"]["num_predict"], 256);
    assert!(body.get("temperature").is_none());
}

#[tokio::test]
async fn ollama_surfaces_inline_error_object() {
    // Ollama 会在 200 的流里返回 {"error": "..."}
    let server = spawn(MockSpec::ndjson(&[r#"{"error":"model 'nope' not found"}"#])).await;

    let connector = OllamaConnector::new(&server.base_url, "nope", client());
    let mut stream = connector
        .chat_stream(ChatRequest::new(vec![Message::user("hi")]))
        .await
        .unwrap();

    let first = stream.next().await.expect("应当有一个事件");
    let err = first.expect_err("内联 error 应当变成流错误");
    assert!(err.to_string().contains("not found"), "实际错误：{err}");
}

// ---------------------------------------------------------------------------
// 取消（barge-in 的底层能力）
// ---------------------------------------------------------------------------

#[tokio::test]
async fn dropping_the_stream_stops_consuming_upstream() {
    // 模拟一个慢速上游：每 50ms 一个 chunk，共 100 个
    let events: Vec<String> = (0..100)
        .map(|i| format!(r#"{{"choices":[{{"delta":{{"content":"{i} "}}}}]}}"#))
        .collect();
    let refs: Vec<&str> = events.iter().map(|s| s.as_str()).collect();

    let server = spawn(MockSpec::sse(&refs).with_delay(Duration::from_millis(50))).await;

    let mut stream = openai(&server.base_url, true)
        .chat_stream(ChatRequest::new(vec![Message::user("hi")]))
        .await
        .unwrap();

    // 只取前两个 chunk 就丢掉整条流
    let first = stream.next().await.unwrap().unwrap();
    assert!(matches!(first, ChatChunk::Text { .. }));

    let started = std::time::Instant::now();
    drop(stream);

    // drop 应当立即返回，而不是等上游那 100 个 chunk 跑完（约 5 秒）
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "drop 流耗时过长：{:?}",
        started.elapsed()
    );
}

// ---------------------------------------------------------------------------
// 思考开关
// ---------------------------------------------------------------------------

#[tokio::test]
async fn openai_sends_reasoning_effort_none_when_thinking_off() {
    let server = spawn(MockSpec::sse(&["[DONE]"])).await;

    let mut req = ChatRequest::new(vec![Message::user("hi")]);
    req.thinking = anima_lib::llm::ThinkingMode::Off;

    let _ = collect(
        openai(&server.base_url, true)
            .chat_stream(req)
            .await
            .unwrap(),
    )
    .await;

    // 实测 LM Studio 上只有这个写法能让推理归零
    assert_eq!(server.request_body()["reasoning_effort"], "none");
}

#[tokio::test]
async fn openai_omits_reasoning_effort_when_thinking_auto() {
    let server = spawn(MockSpec::sse(&["[DONE]"])).await;

    let _ = collect(
        openai(&server.base_url, true)
            .chat_stream(ChatRequest::new(vec![Message::user("hi")]))
            .await
            .unwrap(),
    )
    .await;

    let body = server.request_body();
    assert!(
        body.get("reasoning_effort").is_none(),
        "Auto 模式不该干预后端默认行为：{body}"
    );
}

#[tokio::test]
async fn openai_retries_without_reasoning_effort_when_model_rejects_it() {
    // 实测 Azure 的 gpt-4o 会返回 400「Unrecognized request argument:
    // reasoning_effort」。这种情况要去掉参数重试，而不是把错误抛给用户。
    let server = spawn_many(vec![
        MockSpec::error(
            400,
            r#"{"error":{"message":"Unrecognized request argument supplied: reasoning_effort","type":"invalid_request_error"}}"#,
        ),
        MockSpec::sse(&[
            r#"{"choices":[{"delta":{"content":"重试成功"}}]}"#,
            "[DONE]",
        ]),
    ])
    .await;

    let mut req = ChatRequest::new(vec![Message::user("hi")]);
    req.thinking = anima_lib::llm::ThinkingMode::Off;

    // 用独立的 provider id，避免和其它测试共享那张"不支持"缓存表
    let connector = OpenAiConnector::new(
        "test-rejects-effort",
        &server.base_url,
        "picky-model",
        None,
        true,
        client(),
    );

    let chunks = collect(connector.chat_stream(req).await.unwrap()).await;
    assert_eq!(text_of(&chunks), "重试成功");

    let bodies = server.request_bodies();
    assert_eq!(bodies.len(), 2, "应当正好发两次请求");
    assert_eq!(bodies[0]["reasoning_effort"], "none", "第一次带参数");
    assert!(
        bodies[1].get("reasoning_effort").is_none(),
        "重试必须去掉该参数：{}",
        bodies[1]
    );
}

#[tokio::test]
async fn openai_does_not_swallow_unrelated_400() {
    // 只有「参数不被识别」才重试；其它 400 必须原样报给用户
    let server = spawn(MockSpec::error(
        400,
        r#"{"error":{"message":"model 'nope' does not exist"}}"#,
    ))
    .await;

    let mut req = ChatRequest::new(vec![Message::user("hi")]);
    req.thinking = anima_lib::llm::ThinkingMode::Off;

    let err = expect_stream_err(
        openai(&server.base_url, true).chat_stream(req).await,
        "无关的 400",
    );
    assert!(err.to_string().contains("does not exist"), "实际：{err}");
}

#[tokio::test]
async fn ollama_sends_think_false_when_thinking_off() {
    let server = spawn(MockSpec::ndjson(&[r#"{"done":true}"#])).await;

    let mut req = ChatRequest::new(vec![Message::user("hi")]);
    req.thinking = anima_lib::llm::ThinkingMode::Off;

    let connector = OllamaConnector::new(&server.base_url, "qwen3:32b", client());
    let _ = collect(connector.chat_stream(req).await.unwrap()).await;

    // 实测 qwen3:32b 加上这个之后响应里不再有 thinking 字段
    assert_eq!(server.request_body()["think"], false);
}

#[tokio::test]
async fn ollama_omits_think_when_thinking_auto() {
    let server = spawn(MockSpec::ndjson(&[r#"{"done":true}"#])).await;

    let connector = OllamaConnector::new(&server.base_url, "qwen3:32b", client());
    let _ = collect(
        connector
            .chat_stream(ChatRequest::new(vec![Message::user("hi")]))
            .await
            .unwrap(),
    )
    .await;

    assert!(server.request_body().get("think").is_none());
}
