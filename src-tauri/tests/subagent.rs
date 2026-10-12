//! 子智能体派发的端到端验收。
//!
//! 这里的每条断言都遵守同一条规矩：
//!
//! > **必须断言一个只有功能真的发生才会成立的事实** —— 文件内容变了、
//! > 工具被调用了、模型被请求了。断言返回文本里的标题、断言 `status == Ok`，都不算。
//!
//! 定这条规矩是有来由的：之前的 `invoke_subagent` 测试断言的是报告标题里的字面量
//! （`"Explore (只读探路)"`、`"anima/sandbox-subagent-"`），于是在
//! **执行引擎整个是死代码**（`run_subagent_loop` 全仓零调用、`tools` 恒为空）
//! 的情况下依然全绿。

mod support;

use std::path::Path;
use std::sync::Arc;

use anima_lib::agent_runtime::{
    register_local_tools, RuntimeContext, RuntimePolicy, ToolCallRequest, ToolRegistry, ToolRouter,
    ToolStatus,
};
use anima_lib::config::{AppConfig, ProviderConfig, ProviderKind};
use anima_lib::llm::gateway::Gateway;
use serde_json::json;
use support::{spawn_many, MockSpec};

/// 网关是**进程级**的（生产上一个应用只有一个网关，这样才不用把它顺着
/// 24 个 `register_local_tools` 调用点一路传进来）。代价是这几条测试共享它，
/// 并行跑会互相把对方的网关指向自己的 mock —— 表现为莫名其妙的
/// "Connection reset by peer"，因为请求打到了另一个已经把 spec 消费完的服务器上。
/// 所以这里显式串行。
static GATEWAY_LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();

async fn exclusive_gateway() -> tokio::sync::MutexGuard<'static, ()> {
    GATEWAY_LOCK
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

fn git(args: &[&str], cwd: &Path) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git 该能跑");
    assert!(
        out.status.success(),
        "git {args:?} 失败: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// 建一个有一次提交的真仓库。
fn init_repo(root: &Path) {
    git(&["init", "-q", "."], root);
    git(&["config", "user.email", "a@b.c"], root);
    git(&["config", "user.name", "t"], root);
    std::fs::write(root.join("target.txt"), "before\n").unwrap();
    git(&["add", "."], root);
    git(&["commit", "-qm", "init"], root);
}

/// 让网关的两档都指向 mock。子智能体走的是 Background 档。
fn gateway_at(base_url: &str) -> Arc<Gateway> {
    let mut config = AppConfig::default();
    config.providers = vec![ProviderConfig {
        id: "mock".to_string(),
        label: "mock".to_string(),
        kind: ProviderKind::OpenAiCompatible,
        base_url: base_url.to_string(),
        model: "mock-model".to_string(),
        supports_tools: true,
        requires_key: false,
        api_version: None,
    }];
    config.realtime_provider = "mock".to_string();
    config.background_provider = "mock".to_string();
    Arc::new(Gateway::new(config).expect("网关该能建起来"))
}

/// 让假模型像真 agent 那样走三轮：**先读，再写，最后给结论**。
///
/// 「先读」不是凑数 —— `fs_write_text` 有读-写前置守卫
/// （`ctx.has_read_file`），覆写已存在的文件必须先 `view_file`。
/// 这条守卫在沙箱里同样生效，正说明子智能体继承了父级的安全策略。
fn read_write_conclude(path: &str, content: &str) -> Vec<MockSpec> {
    let read_args = json!({ "path": path }).to_string();
    let args = json!({ "path": path, "content": content }).to_string();
    vec![
        MockSpec::sse(&[
            &format!(
                r#"{{"choices":[{{"delta":{{"tool_calls":[{{"index":0,"id":"c0","type":"function","function":{{"name":"view_file","arguments":{}}}}}]}}}}]}}"#,
                serde_json::to_string(&read_args).unwrap()
            ),
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            "[DONE]",
        ]),
        MockSpec::sse(&[
            &format!(
                r#"{{"choices":[{{"delta":{{"tool_calls":[{{"index":0,"id":"c1","type":"function","function":{{"name":"fs_write_text","arguments":{}}}}}]}}}}]}}"#,
                serde_json::to_string(&args).unwrap()
            ),
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            "[DONE]",
        ]),
        MockSpec::sse(&[
            r#"{"choices":[{"delta":{"content":"已改完"}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
            "[DONE]",
        ]),
    ]
}

fn router_with_local_tools() -> ToolRouter {
    let mut registry = ToolRegistry::new();
    register_local_tools(&mut registry);
    ToolRouter::new(registry)
}

/// 沙箱子智能体必须**真的改到沙箱里的文件**，而且主工作区必须**纹丝不动**。
///
/// 这条是整个拓扑设计的核心承诺。它同时排除了三种曾经真实存在的失败形状：
/// 执行引擎没被调用、`tools` 恒空导致模型发不出工具调用、以及
/// 「建了沙箱就直接返回一段『子任务可安全在此修改』的报告」。
#[tokio::test]
async fn isolated_subagent_actually_edits_the_sandbox_and_leaves_main_untouched() {
    let _guard = exclusive_gateway().await;
    let server = spawn_many(read_write_conclude("target.txt", "after\n")).await;
    anima_lib::agent_runtime::subagent::set_gateway(gateway_at(&server.base_url));

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_repo(root);

    let ctx = RuntimeContext::new(root, RuntimePolicy::workspace_write(root));
    let router = router_with_local_tools();
    let out = router
        .dispatch(
            &ctx,
            ToolCallRequest::new(
                "invoke_subagent",
                json!({
                    "role": "refactorer",
                    "goal": "改掉 target.txt",
                    "prompt": "把 target.txt 内容换成 after",
                    "mode": "isolated"
                }),
            ),
        )
        .await;

    assert_eq!(out.status, ToolStatus::Ok, "派发失败: {:?}", out.failure);

    // ① 主工作区必须没被碰过 —— 隔离的全部意义所在
    assert_eq!(
        std::fs::read_to_string(root.join("target.txt")).unwrap(),
        "before\n",
        "主工作区被污染了"
    );

    // ② 沙箱里必须真的改了。这是「只有功能真发生才成立」的那条事实。
    let sandboxes: Vec<_> = std::fs::read_dir(root.join(".anima/worktrees"))
        .expect("沙箱目录该存在")
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(sandboxes.len(), 1, "该恰好留下一个沙箱");
    assert_eq!(
        std::fs::read_to_string(sandboxes[0].path().join("target.txt")).unwrap(),
        "after\n",
        "子智能体没有真的改动沙箱里的文件"
    );

    // ③ 报告里必须带上变更摘要与影子分支，主 Agent 才有得决策
    assert!(out.content.contains("沙箱变更"), "报告缺变更摘要: {}", out.content);
    assert!(out.content.contains("target.txt"), "变更摘要没提到改了哪个文件");
    assert!(out.content.contains("anima/sandbox-"), "报告缺影子分支名");
}

/// 子智能体没产生任何改动时，沙箱必须被立刻回收 —— 否则每次派发都留下一份完整 checkout。
#[tokio::test]
async fn a_subagent_that_changes_nothing_leaves_no_orphan_sandbox() {
    let _guard = exclusive_gateway().await;
    let server = spawn_many(vec![MockSpec::sse(&[
        r#"{"choices":[{"delta":{"content":"看了一圈，什么都不用改"}}]}"#,
        r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        "[DONE]",
    ])])
    .await;
    anima_lib::agent_runtime::subagent::set_gateway(gateway_at(&server.base_url));

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    init_repo(root);

    let ctx = RuntimeContext::new(root, RuntimePolicy::workspace_write(root));
    let out = router_with_local_tools()
        .dispatch(
            &ctx,
            ToolCallRequest::new(
                "invoke_subagent",
                json!({"role":"r","goal":"g","prompt":"p","mode":"isolated"}),
            ),
        )
        .await;

    assert_eq!(out.status, ToolStatus::Ok);
    let leftover = std::fs::read_dir(root.join(".anima/worktrees"))
        .map(|d| d.filter_map(|e| e.ok()).count())
        .unwrap_or(0);
    assert_eq!(leftover, 0, "没有改动却留下了 {leftover} 个孤儿沙箱");

    let worktrees = std::process::Command::new("git")
        .args(["worktree", "list"])
        .current_dir(root)
        .output()
        .unwrap();
    let listed = String::from_utf8_lossy(&worktrees.stdout);
    assert!(
        !listed.contains("worktrees/subagent-"),
        "git 里还挂着沙箱记录:\n{listed}"
    );
}

/// 网关没就绪时必须**明确失败**，而不是返回一份凭空编造的报告。
///
/// 这是「静默失败」的反面：早先的实现在完全没有模型参与的情况下也返回 `status: Ok`
/// 和一段看起来很像样的 Markdown，主 Agent 会当成子任务真的跑过了。
#[tokio::test]
async fn without_a_gateway_the_dispatch_fails_instead_of_fabricating_a_report() {
    let _guard = exclusive_gateway().await;
    anima_lib::agent_runtime::subagent::clear_gateway_for_test();

    let dir = tempfile::tempdir().unwrap();
    let ctx = RuntimeContext::new(dir.path(), RuntimePolicy::workspace_write(dir.path()));
    let out = router_with_local_tools()
        .dispatch(
            &ctx,
            ToolCallRequest::new(
                "invoke_subagent",
                json!({"role":"r","goal":"g","prompt":"p","mode":"explore"}),
            ),
        )
        .await;

    assert_ne!(out.status, ToolStatus::Ok, "没有网关却成功了: {}", out.content);
    let msg = out.failure.map(|f| f.message).unwrap_or_default();
    assert!(msg.contains("网关"), "错误该说明是网关没就绪，实际是：{msg}");
}

/// 子智能体**绝不能**泄露被安全策略拒绝的文件内容。
///
/// 这条保证以前由 `invoke_subagent` 自己那段读文件的代码兜着；改成真的跑 agent loop
/// 之后，兜底的位置变成了每个工具自己的 `evaluate_path`。位置变了，保证不能变 ——
/// 所以这里让假模型**真的去读** `.env`，再断言密钥一个字都没进报告。
#[tokio::test]
async fn a_subagent_never_leaks_a_denied_file() {
    let _guard = exclusive_gateway().await;

    let read_env = json!({ "path": ".env" }).to_string();
    let server = spawn_many(vec![
        MockSpec::sse(&[
            &format!(
                r#"{{"choices":[{{"delta":{{"tool_calls":[{{"index":0,"id":"c0","type":"function","function":{{"name":"view_file","arguments":{}}}}}]}}}}]}}"#,
                serde_json::to_string(&read_env).unwrap()
            ),
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            "[DONE]",
        ]),
        MockSpec::sse(&[
            r#"{"choices":[{"delta":{"content":"读不到"}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
            "[DONE]",
        ]),
    ])
    .await;
    anima_lib::agent_runtime::subagent::set_gateway(gateway_at(&server.base_url));

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join(".env"), "ANIMA_API_KEY=sk-绝密内容\n").unwrap();

    let mut policy = RuntimePolicy::workspace_write(root);
    policy.deny_patterns = vec![".env*".to_string(), "id_rsa*".to_string()];
    let ctx = RuntimeContext::new(root, policy);

    let out = router_with_local_tools()
        .dispatch(
            &ctx,
            ToolCallRequest::new(
                "invoke_subagent",
                json!({"role":"r","goal":"g","prompt":"看看 .env","mode":"explore"}),
            ),
        )
        .await;

    assert!(
        !out.content.contains("sk-绝密内容"),
        "被拒文件的内容泄露进了子智能体报告：{}",
        out.content
    );
    // 正名：模型确实尝试读了，否则上面那条"没泄露"是空过的
    assert!(
        out.content.contains("view_file"),
        "假模型该真的发起过 view_file 调用：{}",
        out.content
    );
}
