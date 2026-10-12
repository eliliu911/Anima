//! 验证前端与 Rust 后端在模型能力分级（model_tier）上采用单一事实来源（Single Source of Truth）。
//!
//! 前端直接消费后端 ProviderStatus.model_tier，杜绝双端重复实现 IP/正则判断而导致静默分叉。

use anima_lib::agent::{is_local_endpoint, tier_of};
use anima_lib::config::{ProviderConfig, ProviderKind, ProviderStatus};
use std::path::PathBuf;

fn read(rel: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("读不到 {}: {e}", p.display()))
}

#[test]
fn 前端直接消费后端的_model_tier单一真值源() {
    let chat_ts = read("src/store/chat.ts");

    // 确认前端直接读取 provider.model_tier，没有重复的端点正则判断
    assert!(
        chat_ts.contains("provider.model_tier"),
        "src/store/chat.ts 应该直接读取 provider.model_tier 作为单一真值源"
    );
    assert!(
        !chat_ts.contains("export function isLocalEndpoint"),
        "前端不应该维护重复的 isLocalEndpoint 实现"
    );

    // 验证 ProviderStatus::of 统一正确生成 model_tier
    let p_lmstudio = ProviderConfig {
        id: "lmstudio".into(),
        label: "LM Studio".into(),
        kind: ProviderKind::OpenAiCompatible,
        base_url: "http://127.0.0.1:1234/v1".into(),
        model: "qwen".into(),
        supports_tools: true,
        requires_key: false,
        api_version: None,
    };
    assert_eq!(ProviderStatus::of(&p_lmstudio).model_tier, "local");

    let p_lan = ProviderConfig {
        id: "lan_server".into(),
        label: "LAN Server".into(),
        kind: ProviderKind::OpenAiCompatible,
        base_url: "http://192.168.1.100:1234/v1".into(),
        model: "qwen".into(),
        supports_tools: true,
        requires_key: false,
        api_version: None,
    };
    assert_eq!(ProviderStatus::of(&p_lan).model_tier, "local");

    let p_deepseek = ProviderConfig {
        id: "deepseek".into(),
        label: "DeepSeek".into(),
        kind: ProviderKind::OpenAiCompatible,
        base_url: "https://api.deepseek.com/v1".into(),
        model: "deepseek-chat".into(),
        supports_tools: true,
        requires_key: true,
        api_version: None,
    };
    assert_eq!(ProviderStatus::of(&p_deepseek).model_tier, "cloud");

    let p_claude = ProviderConfig {
        id: "claude".into(),
        label: "Claude".into(),
        kind: ProviderKind::Anthropic,
        base_url: "https://api.anthropic.com".into(),
        model: "claude-3-5".into(),
        supports_tools: true,
        requires_key: true,
        api_version: None,
    };
    assert_eq!(ProviderStatus::of(&p_claude).model_tier, "cloud");

    // 验证 is_local_endpoint 判定覆盖
    let local_cases = [
        "http://localhost:1234/v1",
        "http://127.0.0.1:11434",
        "http://192.168.1.100:1234/v1",
        "http://10.0.0.8:8000/v1",
        "http://172.20.0.5:1234/v1",
        "http://macbook.local:1234/v1",
        "http://0.0.0.0:8000/v1",
        "http://[::1]:11434",
    ];

    for url in local_cases {
        assert!(is_local_endpoint(url), "URL 应被判定为 local 端点：{url}");
    }

    let cloud_cases = [
        "https://api.openai.com/v1",
        "https://api.anthropic.com/v1",
        "https://api.deepseek.com/v1",
        "https://api.groq.com/openai/v1",
        "https://api.together.xyz/v1",
        "https://graph.example.com/v10.0/chat",
        "https://api.x.ai/v1/chat?t=110.5",
        "https://model-710.vendor.ai/v1",
    ];

    for url in cloud_cases {
        assert!(!is_local_endpoint(url), "URL 应被判定为 cloud 端点：{url}");
    }
}

/// 一个便于复用的 provider 骨架 —— 只改 kind 和 base_url。
fn provider(id: &str, kind: ProviderKind, base_url: &str) -> ProviderConfig {
    ProviderConfig {
        id: id.into(),
        label: id.into(),
        kind,
        base_url: base_url.into(),
        model: "m".into(),
        supports_tools: true,
        requires_key: false,
        api_version: None,
    }
}

/// **前端读到的是 JSON，那就断言 JSON。**
///
/// `ProviderStatus::of()` 的返回值对不对，和前端实际拿到什么是两件事：
/// 前端那行是 `provider.model_tier ?? "local"`，字段一旦没被序列化出来
/// （有人加了 `#[serde(skip_serializing_if)]`、改了字段名、或
/// `#[serde(flatten)]` 的展开出问题），**每个 provider 都会静默变成
/// local** —— 云端模型拿到 24K 而不是 120K 的历史预算，不报错，
/// 而只断言返回值的测试照样全绿。
///
/// 所以断言要落在真正的接缝上：序列化之后的那个键。
#[test]
fn 序列化后必须带上model_tier这个键() {
    let cloud = provider("openai", ProviderKind::OpenAi, "https://api.openai.com/v1");
    let json = serde_json::to_value(ProviderStatus::of(&cloud)).expect("ProviderStatus 应可序列化");
    assert_eq!(
        json.get("model_tier").and_then(|v| v.as_str()),
        Some("cloud"),
        "序列化结果里没有 model_tier=cloud —— 前端会静默把它当 local。实际 JSON 的键：{:?}",
        json.as_object().map(|o| o.keys().collect::<Vec<_>>())
    );

    let local = provider(
        "lmstudio",
        ProviderKind::OpenAiCompatible,
        "http://127.0.0.1:1234/v1",
    );
    let json = serde_json::to_value(ProviderStatus::of(&local)).unwrap();
    assert_eq!(
        json.get("model_tier").and_then(|v| v.as_str()),
        Some("local")
    );
}

/// **tier 只能有一个计算入口。**
///
/// 早先 `config/mod.rs` 和 `agent.rs` 各写了一份逐字相同的 match。
/// 加新 `ProviderKind` 时编译器会在两处同时报错（match 穷尽、无 `_` 臂），
/// 但**改某一条已有臂**编译器抓不到：比如把 `AzureOpenAi` 改成 local
/// 只改了一处，前端按一档给历史预算、agent 按另一档给读文件行数，
/// 两边静默分叉且都不报错。
///
/// 现在两处都调 `agent::tier_of`。这条测试守住它不被重新拆开。
#[test]
fn tier只有一个计算入口() {
    // ① 源码层面：config/mod.rs 不许自己再算一遍
    let config_rs = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/config/mod.rs"),
    )
    .expect("读不到 config/mod.rs");
    assert!(
        config_rs.contains("tier_of(config)"),
        "config/mod.rs 应该调 agent::tier_of，而不是自己 match 一遍"
    );
    assert!(
        !config_rs.contains("ProviderKind::Ollama => \"local\""),
        "config/mod.rs 里又出现了独立的 tier match —— 那就有第二份实现了"
    );

    // ② 行为层面：两条路对同一个 provider 必须给出同一个答案
    for (kind, url) in [
        (ProviderKind::OpenAi, "https://api.openai.com/v1"),
        (ProviderKind::Anthropic, "https://api.anthropic.com/v1"),
        (ProviderKind::AzureOpenAi, "https://x.openai.azure.com/"),
        (ProviderKind::Ollama, "http://127.0.0.1:11434"),
        (ProviderKind::OpenAiCompatible, "http://127.0.0.1:1234/v1"),
        (ProviderKind::OpenAiCompatible, "http://192.168.1.9:1234/v1"),
        (
            ProviderKind::OpenAiCompatible,
            "https://api.deepseek.com/v1",
        ),
    ] {
        let p = provider("x", kind, url);
        assert_eq!(
            tier_of(&p),
            ProviderStatus::of(&p).model_tier,
            "同一个 provider，两条路给出了不同的 tier：{kind:?} {url}"
        );
    }
}
