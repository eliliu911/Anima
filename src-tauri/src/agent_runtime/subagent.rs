//! Subagent Topology & Sandbox Orchestration Model (P5 Evolution)
//!
//! Provides declaratively isolated subagent dispatching:
//! - Subagent mode (`Explore` for read-only probing vs `Isolated` for Git Worktree refactoring)
//! - Lightweight multi-round subagent loop with tool execution and conclusion extraction
//! - Output structured report with findings and diff proposals

use std::sync::Arc;
use std::time::Instant;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

use crate::config::Tier;
use crate::llm::gateway::Gateway;
use crate::llm::{ChatChunk, ChatRequest, Message, Role, ThinkingMode};
use crate::mcp::Operation;
use super::{RuntimeContext, ToolCallRequest, ToolRouter};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentMode {
    /// 只读探索与调研模式（沿用父级工作区，仅下发只读工具）
    #[default]
    Explore,
    /// 独立沙箱分支模式（分配 Git Worktree 物理目录，支持读写与验证）
    Isolated,
}

impl SubagentMode {
    pub fn from_str_opt(raw: Option<&str>) -> Self {
        match raw {
            Some("isolated") | Some("branch") => SubagentMode::Isolated,
            _ => SubagentMode::Explore,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentSpec {
    pub role: String,
    pub goal: String,
    pub prompt: String,
    #[serde(default)]
    pub mode: SubagentMode,
    #[serde(default)]
    pub target_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentOutcome {
    pub role: String,
    pub mode: SubagentMode,
    pub sandbox_task_id: Option<String>,
    pub sandbox_branch: Option<String>,
    pub findings: Vec<String>,
    pub diff_summary: Option<String>,
    pub conclusion: String,
    pub elapsed_ms: u128,
}

/// 全局并发子智能体上限（保护本地模型实例 LM Studio/Ollama 不被并发请求压垮）
pub const MAX_CONCURRENT_SUBAGENTS: usize = 2;

static SUBAGENT_SEMAPHORE: std::sync::OnceLock<Arc<tokio::sync::Semaphore>> =
    std::sync::OnceLock::new();

pub fn subagent_semaphore() -> Arc<tokio::sync::Semaphore> {
    SUBAGENT_SEMAPHORE
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_SUBAGENTS)))
        .clone()
}

/// 子智能体走哪一档模型。
///
/// **一律 `Background`，两种模式都是。**
///
/// Realtime / Background 在这个项目里分的是**用途与延迟**，不是「好模型 / 差模型」：
/// 语音管线（`voice/mod.rs`）和主会话（`commands.rs`）走的都是 Realtime，
/// 它是「用户正在等着的那一档」。Background 是「用户没在等的那一档」。
///
/// 曾经把 `Isolated` 改成 Realtime，理由是「对齐主会话的高精度模型」。
/// 那个理由站不住，两个原因：
///
/// 1. **哪一档更强完全取决于用户怎么配。** 本机实测两档同指
///    `lmstudio` 的 `gpt-oss-120b` —— 换过去和换回来是同一个实例，零收益；
/// 2. **默认配置下它是负面的。** `AppConfig::default()` 是
///    realtime=lmstudio / background=ollama，两个不同实例。把子智能体挪到 Realtime，
///    等于挪到语音管线正在用的那个实例上 —— 一个 4 轮带长上下文的子智能体推理，
///    会让用户对数字人说的下一句话排在它后面。这个退化不报任何错。
///
/// 要提子智能体的质量，正确做法是**配一个更强的 background 模型**，
/// 而不是让它去抢用户正在等的那一档。
pub fn tier_for(_mode: SubagentMode) -> Tier {
    Tier::Background
}

/// 子智能体要跑循环就得有 LLM 网关，而网关是在工具**注册之后**才由
/// `RuntimeService::set_gateway` 装上的 —— 所以这里用一个进程级的惰性 cell，
/// 而不是把它顺着 24 个 `register_local_tools` 调用点一路传进来。
/// （同样的形状见 `pty_session::LIVE_SESSION_MAPS` 和 `knowledge_tool::MemoryCell`。）
static SUBAGENT_GATEWAY: std::sync::OnceLock<std::sync::Mutex<Option<Arc<Gateway>>>> =
    std::sync::OnceLock::new();

fn gateway_cell() -> &'static std::sync::Mutex<Option<Arc<Gateway>>> {
    SUBAGENT_GATEWAY.get_or_init(|| std::sync::Mutex::new(None))
}

/// 由 `RuntimeService::set_gateway` 调用。
pub fn set_gateway(gateway: Arc<Gateway>) {
    if let Ok(mut cell) = gateway_cell().lock() {
        *cell = Some(gateway);
    }
}

/// 仅供测试：清掉网关，用来验证"没有网关时必须明确失败"这条不变式。
#[doc(hidden)]
pub fn clear_gateway_for_test() {
    if let Ok(mut cell) = gateway_cell().lock() {
        *cell = None;
    }
}

/// 取当前网关。`None` 表示还没就绪 —— 调用方必须**明确报错**，
/// 不能退化成"不调模型也给一份报告"，那样产出的是凭空编造的内容。
pub fn gateway() -> Option<Arc<Gateway>> {
    gateway_cell().lock().ok().and_then(|cell| cell.clone())
}

/// 子智能体**不允许**碰的工具。
///
/// - `invoke_subagent`：禁止递归派发。子智能体再派子智能体，模型很容易绕进去，
///   而且沙箱会嵌套（沙箱本身就是一个 worktree）。
/// - `worktree_*`：沙箱的生命周期由调度器托管，子智能体不该知道 worktree 的存在
///   —— 它眼里只有一个普通工作区，这是整个拓扑设计的前提。
const SUBAGENT_TOOL_DENYLIST: &[&str] = &[
    "invoke_subagent",
    "worktree_create",
    "worktree_merge",
    "worktree_discard",
];

/// 按模式推导下发给子智能体的工具集。
///
/// 按 `Operation` 推导而不是写死名单 —— 这样新增一个只读工具，
/// Explore 子智能体会自动获得它，不需要有人记得来这里补一行。
/// （写死名单的下场见 `CODING_RUNTIME_TOOLS`：两处分开写就会漂，而且漂了不报错。）
fn tools_for(router: &ToolRouter, mode: SubagentMode) -> Vec<crate::llm::Tool> {
    router
        .specs()
        .into_iter()
        .filter(|spec| !SUBAGENT_TOOL_DENYLIST.contains(&spec.name.as_str()))
        .filter(|spec| match mode {
            // 只读探路：只给 Read。给了写工具就不叫"只读"了，
            // 而且它工作在**主工作区**上 —— 写出去就是真改。
            SubagentMode::Explore => spec.operation == Operation::Read,
            // 沙箱模式：读写执行都给，但不给 Delete。
            // 沙箱本身可以整体丢弃，不需要子智能体去删文件。
            SubagentMode::Isolated => matches!(
                spec.operation,
                Operation::Read | Operation::Write | Operation::Execute
            ),
        })
        .map(|spec| crate::llm::Tool {
            name: spec.name,
            description: spec.description,
            parameters: spec.parameters,
        })
        .collect()
}

/// 执行轻量级子智能体 Loop（最大 4 轮往返）
pub async fn run_subagent_loop(
    gateway: Arc<Gateway>,
    router: Arc<ToolRouter>,
    ctx: &RuntimeContext,
    spec: &SubagentSpec,
) -> Result<SubagentOutcome, String> {
    let started = Instant::now();
    let stream_id = uuid::Uuid::new_v4().to_string();

    let mut system_prompt = format!(
        "# Subagent Execution Directive\n\n\
        You are a specialized subagent (Role: {}).\n\
        - **Core Goal**: {}\n\
        - **Workspace Root**: `{}`\n\
        - **Topology Mode**: {:?}\n\n\
        Investigate and execute the assigned subtask using available tools. Produce a concise, structured markdown report summarizing your findings.\
        NEVER dump unescaped raw code or irrelevant verbose logs.",
        spec.role,
        spec.goal,
        ctx.workspace_root.display(),
        spec.mode
    );

    if !spec.target_paths.is_empty() {
        system_prompt.push_str(&format!(
            "\n- **Suggested Target Paths**: {}",
            spec.target_paths.join(", ")
        ));
    }

    let mut messages = vec![
        Message::system(system_prompt),
        Message::user(&spec.prompt),
    ];

    let mut findings = Vec::new();
    let max_rounds = 4;

    // 并发限流：获取子智能体执行许可，避免多子智能体并发打垮单实例模型
    let _permit = subagent_semaphore()
        .acquire_owned()
        .await
        .map_err(|e| format!("获取子智能体并发信号量失败: {e}"))?;

    for _round in 0..max_rounds {
        // 用户正在等 Realtime 回复时，在**轮次之间**让路。
        //
        // 这台机器上两档指向同一个 LM Studio 实例（ollama 里只有嵌入模型），
        // 所以档位路由分不开负载 —— 不让路的话，一个 4 轮带长上下文的子智能体
        // 推理会把用户对数字人说的下一句话排到后面，而且不报任何错。
        //
        // 只在缝隙里让：已经发出去的那次推理不打断，那是服务端的事。
        // 等待有上限，最坏退回到「和用户抢」，也就是加这条通道之前的行为。
        crate::agent_runtime::user_activity::yield_to_user(std::time::Duration::from_secs(30))
            .await;

        // 每轮都重新取 —— 工具集不随轮次变化，但这样写不需要在循环外多存一份，
        // 也避免了"注册表变了而这份快照没变"的隐患。
        let tools = tools_for(&router, spec.mode);
        if tools.is_empty() {
            return Err(format!(
                "没有任何工具可下发给 {:?} 模式的子智能体 —— 它将无法读取任何东西，\
                 这种情况下跑循环只会得到一段凭空编造的报告",
                spec.mode
            ));
        }

        let req = ChatRequest {
            messages: messages.clone(),
            tools,
            temperature: Some(0.2),
            max_tokens: Some(4096),
            thinking: ThinkingMode::Off,
            stream: true,
            json_output: false,
            json_schema: None,
        };

        let tier = tier_for(spec.mode);
        let (_provider, mut stream) = gateway
            .chat_stream(stream_id.clone(), tier, req)
            .await
            .map_err(|e| format!("启动子智能体 LLM 流失败: {e}"))?;

        let mut assistant_text = String::new();
        let mut tool_calls = Vec::new();

        while let Some(chunk_res) = stream.next().await {
            match chunk_res {
                Ok(ChatChunk::Text { delta }) => {
                    assistant_text.push_str(&delta);
                }
                Ok(ChatChunk::ToolCall { call }) => {
                    tool_calls.push(call);
                }
                Ok(ChatChunk::Done { .. }) => break,
                Err(err) => return Err(format!("子智能体流式通信错误: {err}")),
                _ => {}
            }
        }

        if !assistant_text.is_empty() {
            messages.push(Message::assistant(&assistant_text));
        }

        if tool_calls.is_empty() {
            // 没有更多工具调用，生成最终结论
            let conclusion = if assistant_text.trim().is_empty() {
                "子智能体已完成探索，未输出额外文本。".to_string()
            } else {
                assistant_text.trim().to_string()
            };

            return Ok(SubagentOutcome {
                role: spec.role.clone(),
                mode: spec.mode,
                sandbox_task_id: None,
                sandbox_branch: None,
                findings,
                diff_summary: None,
                conclusion,
                elapsed_ms: started.elapsed().as_millis(),
            });
        }

        // 执行工具调用并构造 ToolResult 消息
        for call in tool_calls {
            let parsed_args: serde_json::Value = serde_json::from_str(&call.arguments)
                .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
            let tool_req = ToolCallRequest::new(&call.name, parsed_args);

            // 沙箱模式下放宽**需要确认**的那一档。
            //
            // 理由是隔离本身换来的：子智能体在一份物理独立、可以整体丢弃的副本里干活，
            // 而它在后台跑，用户不在旁边——照主会话那套逐条确认，它连 `cargo test`
            // 都跑不完，沙箱也就失去了意义。
            //
            // 放宽的只有 `needs_confirmation`。`denied()` 那一档不受影响，
            // 所以不可逆命令名单（`rm -rf /`、`dd of=/dev/` 之类）照样拦得住 ——
            // 沙箱能丢弃的是仓库副本，不是这台机器。
            //
            // **Explore 模式绝不放宽**：它工作在主工作区上，没有任何东西兜底。
            if spec.mode == SubagentMode::Isolated {
                ctx.grant_tool_call(tool_req.id.clone()).await;
            }

            let outcome = router.dispatch(ctx, tool_req).await;
            let preview = if outcome.content.len() > 1000 {
                format!("{}...", &outcome.content[..1000])
            } else {
                outcome.content.clone()
            };
            findings.push(format!("`{}`: {}", call.name, preview.trim()));
            messages.push(Message {
                role: Role::Tool,
                content: outcome.content,
                images: Vec::new(),
                tool_calls: Vec::new(),
                tool_call_id: Some(call.id),
            });
        }
    }

    Ok(SubagentOutcome {
        role: spec.role.clone(),
        mode: spec.mode,
        sandbox_task_id: None,
        sandbox_branch: None,
        findings,
        diff_summary: None,
        conclusion: "子智能体达到往返上限，已总结现有调研成果。".to_string(),
        elapsed_ms: started.elapsed().as_millis(),
    })
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_runtime::{register_local_tools, ToolRegistry};

    fn router() -> ToolRouter {
        let mut registry = ToolRegistry::new();
        register_local_tools(&mut registry);
        ToolRouter::new(registry)
    }

    /// 回归：子智能体必须**真的拿到工具**。
    ///
    /// 早先的实现里 `let tools = vec![]` 是全文唯一一次赋值 —— 子智能体每轮都被告知
    /// 「你没有任何工具」，于是它永远发不出 tool call，下游的派发代码整段不可达。
    /// 而这一切**不报任何错**：循环跑完一轮就退出，返回一段模型凭空写的文本，
    /// 读起来就像子智能体认真调研过。
    #[test]
    fn a_subagent_is_actually_given_tools() {
        let router = router();
        for mode in [SubagentMode::Explore, SubagentMode::Isolated] {
            let tools = tools_for(&router, mode);
            assert!(!tools.is_empty(), "{mode:?} 模式没有拿到任何工具");
        }
    }

    /// 回归：Explore 只拿只读工具。它工作在**主工作区**上 —— 拿到写工具就是真改。
    #[test]
    fn explore_mode_gets_read_only_tools() {
        let router = router();
        let names: Vec<String> = tools_for(&router, SubagentMode::Explore)
            .into_iter()
            .map(|t| t.name)
            .collect();
        let specs = router.specs();
        for name in &names {
            let op = specs
                .iter()
                .find(|s| &s.name == name)
                .map(|s| s.operation)
                .expect("声明的工具必须在注册表里");
            assert_eq!(op, Operation::Read, "Explore 拿到了非只读工具 `{name}`");
        }
        // Isolated 必须**多于** Explore，否则"沙箱能读写"这句话就是空的
        let isolated = tools_for(&router, SubagentMode::Isolated).len();
        assert!(
            isolated > names.len(),
            "Isolated 模式该比 Explore 多拿到写/执行工具（{isolated} vs {}）",
            names.len()
        );
    }

    /// 回归：子智能体不得递归派发，也不得碰 worktree —— 沙箱生命周期归调度器管，
    /// 子智能体眼里只该有一个普通工作区。
    #[test]
    fn a_subagent_can_neither_recurse_nor_touch_worktrees() {
        let router = router();
        for mode in [SubagentMode::Explore, SubagentMode::Isolated] {
            let names: Vec<String> = tools_for(&router, mode).into_iter().map(|t| t.name).collect();
            for banned in SUBAGENT_TOOL_DENYLIST {
                assert!(
                    !names.iter().any(|n| n == banned),
                    "{mode:?} 模式不该拿到 `{banned}`"
                );
            }
        }
        // 正名：这几个工具确实存在于注册表里，否则上面的断言是空过的
        let all: Vec<String> = router.specs().into_iter().map(|s| s.name).collect();
        for banned in SUBAGENT_TOOL_DENYLIST {
            assert!(all.iter().any(|n| n == banned), "`{banned}` 不在注册表里，断言无效");
        }
    }

    /// 指向**必然立刻拒绝连接**的端口。
    /// 这样「跑得动」是毫秒级、「被卡住」是超时，两者在时间上分得开 ——
    /// 用真实端点的话两种情况都慢，断言就测不出任何东西。
    fn dead_end_fixture() -> (
        Arc<crate::llm::gateway::Gateway>,
        Arc<ToolRouter>,
        crate::agent_runtime::RuntimeContext,
        SubagentSpec,
    ) {
        use crate::agent_runtime::{register_local_tools, RuntimeContext, RuntimePolicy, ToolRegistry};
        let dir = Box::leak(Box::new(tempfile::tempdir().unwrap()));
        let ctx = RuntimeContext::new(dir.path(), RuntimePolicy::workspace_write(dir.path()));
        let mut registry = ToolRegistry::new();
        register_local_tools(&mut registry);
        let mut config = crate::config::AppConfig::default();
        config.providers = vec![crate::config::ProviderConfig {
            id: "dead".into(),
            label: "dead".into(),
            kind: crate::config::ProviderKind::OpenAiCompatible,
            base_url: "http://127.0.0.1:1".into(),
            model: "none".into(),
            supports_tools: true,
            requires_key: false,
            api_version: None,
        }];
        config.realtime_provider = "dead".into();
        config.background_provider = "dead".into();
        (
            Arc::new(crate::llm::gateway::Gateway::new(config).unwrap()),
            Arc::new(ToolRouter::new(registry)),
            ctx,
            SubagentSpec {
                role: "r".into(),
                goal: "g".into(),
                prompt: "p".into(),
                mode: SubagentMode::Explore,
                target_paths: Vec::new(),
            },
        )
    }

    /// 回归：限流必须**真的接在 `run_subagent_loop` 的链路上**。
    ///
    /// 上一版的测试是这么写的：
    /// ```ignore
    /// assert_eq!(tier_for(SubagentMode::Explore), Tier::Background);
    /// assert_eq!(sem.available_permits(), MAX_CONCURRENT_SUBAGENTS);
    /// ```
    /// 两条都是同义反复 —— 前者断言 `match` 返回它自己写的值，后者断言
    /// `Semaphore::new` 的契约。**实测把整段 `acquire_owned` 注释掉，它依然通过。**
    ///
    /// 所以这条改成：先把 permit 全部占死，断言子智能体**跑不起来**；
    /// 放开之后必须能跑（正名 —— 否则上面那个"超时"可能只是功能整个坏了）。
    #[tokio::test]
    async fn the_concurrency_limit_is_actually_wired_into_the_loop() {
        use crate::agent_runtime::{register_local_tools, RuntimeContext, RuntimePolicy, ToolRegistry};

        let dir = tempfile::tempdir().unwrap();
        let ctx = RuntimeContext::new(dir.path(), RuntimePolicy::workspace_write(dir.path()));
        let mut registry = ToolRegistry::new();
        register_local_tools(&mut registry);
        let router = Arc::new(ToolRouter::new(registry));
        let spec = SubagentSpec {
            role: "r".into(),
            goal: "g".into(),
            prompt: "p".into(),
            mode: SubagentMode::Explore,
            target_paths: Vec::new(),
        };
        // 网关指向一个**必然立刻拒绝连接**的端口。
        //
        // 这一点是这条测试成立的关键：拿不到 permit → 根本走不到网关 → 超时；
        // 拿到了 permit → 立刻连接失败 → 迅速返回。两种情况在**时间上**分得开，
        // 断言才真的在测限流。
        //
        // 我第一版用的是 `AppConfig::default()`（指向真实的 lmstudio），
        // 结果无论有没有 permit，请求本身都慢过超时窗口 ——
        // 那条测试和它要替换掉的那条一样，摘掉限流照样绿。
        let mut config = crate::config::AppConfig::default();
        config.providers = vec![crate::config::ProviderConfig {
            id: "dead".into(),
            label: "dead".into(),
            kind: crate::config::ProviderKind::OpenAiCompatible,
            base_url: "http://127.0.0.1:1".into(),
            model: "none".into(),
            supports_tools: true,
            requires_key: false,
            api_version: None,
        }];
        config.realtime_provider = "dead".into();
        config.background_provider = "dead".into();
        let gateway = Arc::new(crate::llm::gateway::Gateway::new(config).unwrap());

        let sem = subagent_semaphore();
        let hold = Arc::clone(&sem)
            .acquire_many_owned(MAX_CONCURRENT_SUBAGENTS as u32)
            .await
            .expect("测试开始时该能占满 permit");

        let blocked = tokio::time::timeout(
            std::time::Duration::from_millis(400),
            run_subagent_loop(Arc::clone(&gateway), Arc::clone(&router), &ctx, &spec),
        )
        .await;
        assert!(
            blocked.is_err(),
            "permit 被占满时子智能体仍然跑了起来 —— 限流没有接在链路上"
        );

        // 正名：放开之后必须能走到网关（成败都行，重点是它不再被 permit 卡住）。
        drop(hold);
        let ran = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            run_subagent_loop(gateway, router, &ctx, &spec),
        )
        .await;
        assert!(
            ran.is_ok(),
            "permit 放开之后子智能体依然跑不动 —— 说明卡住的不是限流，是别的东西坏了"
        );
        // 连不上是预期结果；重点是它**跑到了**网关，而不是被 permit 拦在门外。
        assert!(ran.unwrap().is_err(), "指向死端口时该返回连接错误");

        // permit 必须归还，否则第二次派发就永远拿不到
        assert_eq!(
            sem.available_permits(),
            MAX_CONCURRENT_SUBAGENTS,
            "跑完之后 permit 没有归还"
        );
    }

    /// 子智能体一律走 Background —— 那是「用户没在等」的那一档。
    /// 详细理由见 `tier_for` 的文档注释。
    #[test]
    fn subagents_never_compete_for_the_tier_the_user_is_waiting_on() {
        for mode in [SubagentMode::Explore, SubagentMode::Isolated] {
            assert_eq!(
                tier_for(mode),
                Tier::Background,
                "{mode:?} 不该占用语音管线与主会话所在的 Realtime 档"
            );
        }
    }
    /// 回归：用户正在等 Realtime 回复时，子智能体必须在轮次之间让路。
    ///
    /// 这条盯的是 `run_subagent_loop` 里那一句 `yield_to_user` 真的在链路上。
    /// 网关指向死端口 —— 所以「跑得动」是毫秒级的，
    /// 而「在让路」是被 300ms 窗口截断的，两者在时间上分得开。
    #[tokio::test]
    async fn a_subagent_yields_while_the_user_is_waiting() {
        use crate::agent_runtime::user_activity;

        // 和 `user_activity` 里那两条共用同一把锁 —— 三条测试摸的是同一个
        // 进程级计数器，少锁一条就会在全量跑时互相打断前提。
        let _serial = user_activity::exclusive_for_test().await;

        let (gateway, router, ctx, spec) = dead_end_fixture();

        // 先确认不让路时它跑得很快 —— 否则下面的"超时"说明不了任何问题
        let quick = tokio::time::timeout(
            std::time::Duration::from_millis(2000),
            run_subagent_loop(Arc::clone(&gateway), Arc::clone(&router), &ctx, &spec),
        )
        .await;
        assert!(quick.is_ok(), "空闲时子智能体该很快跑完（连不上即返回）");

        // 有 Realtime 在飞 → 必须卡在轮次之前
        let guard = user_activity::begin_realtime();
        let blocked = tokio::time::timeout(
            std::time::Duration::from_millis(400),
            run_subagent_loop(Arc::clone(&gateway), Arc::clone(&router), &ctx, &spec),
        )
        .await;
        drop(guard);
        assert!(
            blocked.is_err(),
            "用户正在等的时候子智能体照跑不误 —— 让路没有接在链路上"
        );
    }

}
