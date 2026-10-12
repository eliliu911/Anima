//! 用量账本接在网关上：一次调用跑完，`usage` 按「后端 + 模型 + 分集 + 步骤」落进 `token_usage.json`。
//!
//! 要钉的是**接线**，不是累加算术（那在 `token_usage` 的单元测试里）：
//! 网关真的在流结束时记了账、记的是调用方所在的范围 —— 范围在建流时就量好，流被 poll 时已经出了范围也不丢。

mod support;

use anima_lib::config::{AppConfig, Tier};
use anima_lib::llm::gateway::Gateway;
use anima_lib::llm::{ChatRequest, Message};
use futures_util::StreamExt;
use support::{spawn, MockSpec};

#[tokio::test]
async fn 网关记账_按调用方的分集和步骤() {
    let server = spawn(MockSpec::sse(&[
        r#"{"choices":[{"delta":{"content":"好"}}]}"#,
        r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        r#"{"choices":[],"usage":{"prompt_tokens":1200,"completion_tokens":300}}"#,
        "[DONE]",
    ]))
    .await;
    let mut config = AppConfig::default();
    for p in &mut config.providers {
        if p.id == "lmstudio" {
            p.base_url = server.base_url.clone();
            p.model = "mock-model".into();
        }
    }
    config.background_provider = "lmstudio".into();
    config.background_model = None;
    let gateway = Gateway::new(config).unwrap();

    let dir = tempfile::tempdir().unwrap();
    anima_lib::token_usage::init(dir.path());

    // 建流在范围里，poll 在范围外 —— 和 `prompts::run` 拿到流之后在外面读是一样的
    let (_, stream) = anima_lib::token_usage::scoped(
        "p1",
        "ep_01",
        anima_lib::token_usage::with_step("镜头表", gateway.chat_stream("t".into(), Tier::Background, ChatRequest::new(vec![Message::user("hi")]))),
    )
    .await
    .unwrap();
    let _: Vec<_> = stream.collect().await;

    let ledger: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.path().join("token_usage.json")).expect("没记账")).unwrap();
    let rows = ledger["rows"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "{ledger}");
    let r = &rows[0];
    assert_eq!((r["provider"].as_str(), r["model"].as_str()), (Some("lmstudio"), Some("mock-model")));
    assert_eq!((r["project_id"].as_str(), r["episode_id"].as_str(), r["step"].as_str()), (Some("p1"), Some("ep_01"), Some("镜头表")));
    assert_eq!((r["calls"].as_u64(), r["prompt_tokens"].as_u64(), r["completion_tokens"].as_u64()), (Some(1), Some(1200), Some(300)));
}
