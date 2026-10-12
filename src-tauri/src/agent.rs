//! 带工具的对话循环（agentic loop）。
//!
//! 没有工具时一轮对话就是「请求 → 流式输出 → 结束」。有工具之后变成多轮：
//!
//! ```text
//! 请求 ─→ 流 ─→ 模型要调工具？ ─是→ 过闸门 ─→ 执行 ─→ 结果回填 ─→ 再来一轮
//!                    └─否→ 结束
//! ```
//!
//! ## 几个必须在这层守住的点
//!
//! - **轮数上限**：模型可能来回调工具停不下来（尤其本地小模型），必须有硬上限，
//!   否则一次对话能烧掉整个上下文窗口。
//! - **单次工具超时**：MCP server 卡住时调用会一直挂着。没有超时的话，
//!   整轮对话就永远停在那儿，用户只看到光标闪。
//! - **可取消**：语音打断要能在工具执行中途生效，不能等工具跑完
//!   （见 ARCHITECTURE.md 坑点 2）。
//! - **失败要回填给模型而不是中断对话**：工具报错、被拒、超时，都作为
//!   `role=tool` 的内容送回去。模型看到原因才能改口径或换方案；直接中断的话
//!   用户只会看到对话莫名停住。

use std::sync::Arc;

use futures_util::StreamExt;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::commands::{StreamErrorEvent, StreamEvent, STREAM_EVENT};
use crate::config::{ProviderConfig, ProviderKind, Tier};
use crate::llm::{ChatChunk, ChatRequest, FinishReason, Message, Role, ToolCall};
use crate::mcp::McpRegistry;
use crate::Gateway;
use tokio_util::sync::CancellationToken;

/// 工具活动事件名。前端据此显示「正在执行 xxx」。
pub const TOOL_EVENT: &str = "anima://tool-activity";
pub const IMAGE_PROGRESS_EVENT: &str = "anima://image-generation-progress";
pub const VIDEO_PROGRESS_EVENT: &str = "anima://video-generation-progress";

/// 本地模型的工具往返上限。
///
/// 这个上限只是防无限循环的最后保险，不能作为日常任务的工作量上限。
/// 跑偏由 [`RepeatGuard`] 按“有没有推进”拦截；正常的多文件编码任务很容易
/// 超过 12 次工具往返，因此这里给到足够完成一轮实现 + 测试 + 修复的空间。
const MAX_TOOL_ROUNDS_LOCAL: usize = 40;

/// 云端模型的工具往返上限。
///
/// 云端档位用于后台链路和较复杂任务，应更接近 Codex/Claude Code 的“做到结束”
/// 体验；真正失控时仍有重复调用检测、用户停止和上下文预算兜底。
const MAX_TOOL_ROUNDS_CLOUD: usize = 100;

/// 第几轮还没有台账就点名要一次。
///
/// **触发条件是轮数，不是「从用户输入里识别多步任务特征」。** 后者是个双向
/// 会错的启发式，而且要么再调一次模型、要么写中文关键词规则（一定会漏）。
/// 轮数可观测、单调、零成本。
///
/// 3 而不是 1：第 1、2 轮探目录读文件是正常开局，那时还看不出是不是多步任务。
const LEDGER_NUDGE_ROUND: usize = 3;

/// 点名之后又过了几轮还没有台账，就由系统兜底写一份。
///
/// **不是重复点名，是不再问模型。**
///
/// 实测后台档 `openai/gpt-oss-120b`：Effort=Low 时前置指令和点名兜底都有效
/// （3/3）；Effort=High 时两者都无效（0/3，它重复调 `fs_list_directory`）。
/// 换个任务措辞（点名具体路径、明说不用列目录）High 能回到 2/3 ——
/// 所以是**档位和任务文本共同起作用**，不是单一因素。完整的表在
/// `task_ledger::TaskLedger::seed_from_user` 的注释里。
///
/// 不论哪个因素，结论一样：载荷最重的那个字段（`goal`）不能依赖模型配合。
const LEDGER_SEED_ROUND: usize = 6;

const LEDGER_NUDGE_NOTE: &str = "\
你已经调了好几轮工具，但还没有写下任务计划。**现在立刻调一次 `task_plan`**，\
写清 goal 和剩下的全部步骤。对话历史会被压缩，不写下来你后面会不知道自己在做什么。\
进度由系统自动记录，你只需要写计划这一次。";

const TOOL_ROUNDS_EXHAUSTED_SYSTEM_NOTE: &str = "\
工具已经因为本轮工具往返上限而停用。你现在不能再调用工具。请基于已经看到的\
信息给用户一个清晰交接：说明已完成什么、验证到哪里、还剩什么没做，以及用户\
可以发送“继续”来开启下一轮。不要把工具调用参数、JSON 或命令参数当作正文输出。";

/// 自愈与终验提醒（Self-healing Verification Prompt）。
/// 当任务已发生代码修改且最近命令执行失败时，在模型试图草草结束前触发一次纠错自愈。
const VERIFICATION_NUDGE_NOTE: &str = "\
【系统终验与自愈提醒】检测到你刚刚修改了工作区代码，但最近的命令执行报错。\
请优先调用相关工具修复报错代码或运行验证。若本次报错属于预期内现象（例如用户明确要求探测/汇总错误），请在回复中说明情况即可。";

/// 单条工具结果发给前端保存的字符上限。
///
/// 前端会按最近/较旧历史再分层压缩；这里保留足够内容，保证用户说“继续”时
/// 模型不用因为只剩 300 字符摘要而重读刚刚看过的文件。
const MAX_FRONTEND_TOOL_RESULT_CHARS: usize = 20_000;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum ToolActivity {
    /// 已批准，开始执行
    Started { name: String, arguments: String },
    /// 持续执行心跳（特别针对耗时 10~60s 的子智能体/大型命令）
    Running {
        name: String,
        elapsed_secs: u64,
        hint: Option<String>,
    },
    /// 执行完毕（`ok=false` 表示工具自身报错，不是通信失败）
    Finished {
        name: String,
        ok: bool,
        preview: String,
    },
    /// 被闸门拒绝
    Rejected { name: String, reason: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolActivityEvent {
    pub stream_id: String,
    #[serde(flatten)]
    pub activity: ToolActivity,
}

/// 每一轮对话的取消令牌。
///
/// **为什么不能复用 Gateway 里的那个。** Gateway 给每条流配一个令牌，但流一结束
/// 就把它从表里摘掉 —— 而工具执行发生在流结束**之后**（模型先说完"我要调工具"，
/// 流才结束，然后才轮到执行）。也就是说工具跑的那最多 60 秒里，
/// Gateway 那边已经没有这条流了，用户点"停止"完全影响不到它。
///
/// 语音链路本来就有这个能力（`Turn.cancel` 贯穿到 `tokio::select!`），
/// 文字链路缺的就是这一份。
#[derive(Default)]
pub struct TurnCancels {
    tokens: tokio::sync::RwLock<std::collections::HashMap<String, CancellationToken>>,
}

impl TurnCancels {
    async fn register(&self, stream_id: &str) -> CancellationToken {
        let token = CancellationToken::new();
        self.tokens
            .write()
            .await
            .insert(stream_id.to_string(), token.clone());
        token
    }

    async fn forget(&self, stream_id: &str) {
        self.tokens.write().await.remove(stream_id);
    }

    /// 取消这一轮。返回是否真的取消了（找不到说明这轮已经结束）。
    pub async fn cancel(&self, stream_id: &str) -> bool {
        match self.tokens.write().await.remove(stream_id) {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }
}

/// 离开作用域就把这一轮的令牌摘掉。
///
/// 用 Drop 而不是在每个 return 前手动摘：`run` 有七八个提前返回的分支
/// （建流失败、被取消、没有工具调用…），漏一个就会在表里留一条永远不走的记录。
struct CancelGuard {
    cancels: Arc<TurnCancels>,
    stream_id: String,
}

impl Drop for CancelGuard {
    fn drop(&mut self) {
        let cancels = Arc::clone(&self.cancels);
        let id = self.stream_id.clone();
        // Drop 里不能 await，交给一个短任务
        tauri::async_runtime::spawn(async move { cancels.forget(&id).await });
    }
}

#[derive(Clone)]
pub struct LoopDeps {
    pub gateway: Arc<Gateway>,
    pub runtime: Arc<crate::agent_runtime::RuntimeService>,
    pub workspace_path: Option<String>,
    /// 惰性初始化的记忆系统格子（与 `AppState::memory` 同一个）。
    ///
    /// 用来在一轮说完之后提取**项目事实**。拿 `Arc<Memory>` 在构造这个结构时
    /// 可能还是 None（记忆要先探嵌入后端的维度才能建库），所以传格子。
    pub memory: crate::agent_runtime::MemoryCell,
    /// 任务台账的键（`task_ledger::key_for` 算出来的）。None = 不启用台账。
    pub ledger_key: Option<String>,
    pub mcp: Arc<tokio::sync::RwLock<McpRegistry>>,
    /// 共享的 HTTP 客户端。
    pub http: reqwest::Client,
    /// 全局背景音乐播放器。
    pub music_player: Arc<tokio::sync::Mutex<Option<Arc<crate::audio::playback::Player>>>>,
    /// 整轮的取消令牌表。见 [`TurnCancels`]
    pub turn_cancels: Arc<TurnCancels>,
    /// 应用数据目录。
    pub config_dir: std::path::PathBuf,
}

/// 跑完一整轮（可能含多次工具往返）。
///
/// 出错时已经通过 event 通知了前端，所以这里不返回 Result ——
/// 调用方是 `spawn` 出来的后台任务，没人会去看返回值。
/// 这条用户消息是不是「接着上一件事干」。
///
/// 只认**明确的接续词**，且限定很短的消息 —— 判错的代价不对称：
/// 误判成接续，最多是旧任务卡多留一会儿；误判成新任务，会把一个真正
/// 多轮任务的目标在中途抹掉，而那正是台账存在的理由
///（撞上 40 轮上限之后用户说「继续」，是设计里明确要支持的场景）。
fn is_continuation(text: &str) -> bool {
    let t = text.trim();
    if t.chars().count() > 12 {
        return false;
    }
    const WORDS: &[&str] = &[
        "继续", "接着", "接着做", "往下", "往下做", "go on", "continue", "keep going", "next",
    ];
    let lower = t.to_ascii_lowercase();
    WORDS.iter().any(|w| lower == *w || lower.starts_with(w))
}

/// 这一个工具调用要不要交给后台档（而不是让实时档的小模型自己执行）。
///
/// 每轮对话最多换一次：换过之后就是后台档在调工具，照常执行。
pub fn should_escalate(escalate_to: Option<Tier>, already: bool) -> bool {
    escalate_to.is_some() && !already
}

pub async fn run(
    app: AppHandle,
    deps: LoopDeps,
    stream_id: String,
    tier: Tier,
    mut req: ChatRequest,
) {
    let mut tier = tier;
    let cancel = deps.turn_cancels.register(&stream_id).await;
    // 无论从哪条路径退出都要把令牌摘掉，否则表会一直长
    let _guard = CancelGuard {
        cancels: Arc::clone(&deps.turn_cancels),
        stream_id: stream_id.clone(),
    };

    // ── 换任务就归档旧台账 ──
    //
    // 台账是**按会话**存的（`s-<session_id>`），而 `archived` 以前只有用户手动点
    // 卡片上那个 × 才会设。于是上一件事的台账会一直挂在那儿，
    // 新问题的轮次继续累加到**旧目标**上 —— 用户看到的是
    // 「我在问六爻算法对比，任务卡标题却写着『查看当前项目是否还有挂起的更改』，
    //   20 轮、2 次命令失败」。任务本身也跟着被带跑偏。
    //
    // 判据只用一条**保守**的：新用户消息不是「继续」这类接续词，就当新任务。
    // 宁可少归档（旧卡多留一会儿）也不能多归档 ——
    // 多归档会把一个真正多轮任务的目标在中途抹掉，那正是台账要防的事。
    if let Some(key) = &deps.ledger_key {
        let latest_user = req
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .map(|m| m.content.trim().to_string())
            .unwrap_or_default();
        if !is_continuation(&latest_user) {
            let mut ledger = crate::task_ledger::load(&deps.config_dir, key);
            if !ledger.is_empty() && !ledger.archived {
                tracing::info!(
                    stream_id = %stream_id,
                    goal = %ledger.goal,
                    "收到新任务，归档上一件事的台账"
                );
                ledger.archived = true;
                let _ = crate::task_ledger::save(&deps.config_dir, key, &ledger);
            }
        }
    }

    // 仅在非项目工作区模式（无 workspace_path 绑定）时才注入 MCP roots 提示。
    // 项目模式下由 commands.rs 中的 workspace header 负责提供环境真值。
    if deps.workspace_path.is_none() {
        if let Some(hint) = deps.mcp.read().await.usage_hint() {
            if let Some(msg) = req.messages.first_mut().filter(|m| m.role == Role::System) {
                msg.content.push_str("\n\n");
                msg.content.push_str(&hint);
            } else {
                req.messages.insert(0, Message::system(hint));
            }
        }
    }

    // 首轮还不知道 provider tier，保守用本地上限；首次获知后锁定实际值
    let mut round_limit = MAX_TOOL_ROUNDS_LOCAL;
    // 整轮对话共享一个重复检测器 —— 跨轮次才拦得住「每轮调一次同样的东西」
    let mut repeat_guard = RepeatGuard::default();
    // 撞过上限吗。撞了的话收尾要告诉前端 —— 那一轮的内容大概率是半截的。
    let mut hit_round_limit = false;
    // 记录本轮自愈提醒触发次数，避免无限重试震荡（最多自愈重试 2 次）
    let mut self_healing_attempts: usize = 0;
    // ── 实时档小模型一要调工具，就交给后台档 ──（见 `Gateway::tool_escalation`）
    // 闲聊不受影响；只有带着工具的请求才可能换，而且每轮对话最多换一次。
    let escalate_to = if req.tools.is_empty() {
        None
    } else {
        deps.gateway.tool_escalation(tier).await
    };
    let mut escalated = false;
    // 换档后第一轮要按新后端重新定轮次上限
    let mut relock_round_limit = false;

    for round in 0..=MAX_TOOL_ROUNDS_CLOUD {
        // 超过当前 tier 的上限就退出
        if round > round_limit {
            break;
        }
        // ── 台账兜底 ──
        //
        // 实测创建台账的依从性靠措辞可以救（工作区 header 用了「禁止先调其它
        // 工具」那一档），但那是概率不是保证 —— ornith 在较弱的措辞下会直奔
        // fs_list_directory。第 3 轮还没有台账就点名要一次。
        if let Some(key) = &deps.ledger_key {
            if round == LEDGER_NUDGE_ROUND && !req.tools.is_empty()
                && crate::task_ledger::load(&deps.config_dir, key).is_empty() {
                    tracing::info!(stream_id = %stream_id, "第 {LEDGER_NUDGE_ROUND} 轮仍无台账，点名要一次");
                    req.messages.push(Message::system(LEDGER_NUDGE_NOTE));
                }
            // 点名过了还没有 → **系统自己写**，不再问模型。
            // goal 就是用户原话，我们本来就有。steps 空着不要紧 ——
            // 台账里载荷最重的是"意图能不能活过上下文压缩"。
            if round == LEDGER_SEED_ROUND {
                let mut ledger = crate::task_ledger::load(&deps.config_dir, key);
                if ledger.is_empty() {
                    let user_text = req
                        .messages
                        .iter()
                        .find(|m| m.role == Role::User)
                        .map(|m| m.content.clone())
                        .unwrap_or_default();
                    ledger.seed_from_user(&user_text);
                    if !ledger.is_empty() {
                        tracing::info!(
                            stream_id = %stream_id,
                            "第 {LEDGER_SEED_ROUND} 轮仍无台账，系统按用户原话兜底建一份"
                        );
                        let _ = crate::task_ledger::save(&deps.config_dir, key, &ledger);
                    }
                }
            }
        }

        // 最后一轮不再给工具：给了模型还会继续调，然后被上限截断，
        // 用户看到的就是一次没有结论的对话。收掉工具它就会用已有信息作答。
        if round == round_limit && !req.tools.is_empty() {
            tracing::warn!(stream_id = %stream_id, round_limit, "工具往返达到上限，本轮不再提供工具");
            req.tools.clear();
            req.messages
                .push(Message::system(TOOL_ROUNDS_EXHAUSTED_SYSTEM_NOTE));
            hit_round_limit = true;
        }

        let (provider, mut stream) = match deps
            .gateway
            .chat_stream(stream_id.clone(), tier, req.clone())
            .await
        {
            Ok((provider, stream)) => (provider, stream),
            Err(err) => {
                emit_error(&app, &stream_id, &err.to_string());
                return;
            }
        };

        let mut text = String::new();
        let mut calls: Vec<ToolCall> = Vec::new();
        let mut finish = FinishReason::Stop;

        // 模型能力分级。**只有 `tier_of` 一个计算入口** ——
        // 它同时被 `ProviderStatus::of`（下发给前端）用，两边不可能分叉
        let model_tier_str = tier_of(&provider);

        // 首轮（以及换档后的第一轮）确定后锁定实际 tier 的轮次上限
        if round == 0 || relock_round_limit {
            relock_round_limit = false;
            round_limit = if model_tier_str == "cloud" {
                MAX_TOOL_ROUNDS_CLOUD
            } else {
                MAX_TOOL_ROUNDS_LOCAL
            };
        }

        // 这一轮是不是要换人重来（小模型想调工具了）
        let mut switch_now = false;
        while let Some(item) = stream.next().await {
            match item {
                Ok(chunk) => {
                    // **小模型的工具调用不执行、也不发给前端** —— 发了界面上会多一张
                    // 永远不会执行的工具卡。掐断这条流，同一轮交给后台档从头来。
                    if matches!(chunk, ChatChunk::ToolCall { .. }) && should_escalate(escalate_to, escalated) {
                        switch_now = true;
                        break;
                    }
                    match &chunk {
                        ChatChunk::Text { delta } => text.push_str(delta),
                        ChatChunk::ToolCall { call } => calls.push(call.clone()),
                        ChatChunk::Done { reason, .. } => finish = *reason,
                        ChatChunk::Reasoning { .. } => {}
                        // ToolResult 由我们自己发射，不会从流中出现
                        ChatChunk::ToolResult { .. } => {}
                    }

                    // **中间轮次的 Done 绝对不能转发给前端。**
                    //
                    // 前端把 Done 当作"这轮对话彻底说完了"：它会解除 stream_id 与消息的绑定。
                    // 很多本地模型（LM Studio / Ollama / DeepSeek）在发出 tool_calls 的同时，
                    // finish_reason 却可能返回 "stop" 或 null，而不是标准的 "tool_calls"。
                    // 如果在此处转发任何 Done，会导致第 0 轮结束时前端就断开绑定，
                    // 后续工具执行完再流式输出的正文就全部被前端丢弃（表现为 Finished streaming 却无内容）。
                    //
                    // 因此：单轮流内的 Done 一律不转发，由下方统一在全流程收敛时发射真正的 Done。
                    if matches!(chunk, ChatChunk::Done { .. }) {
                        continue;
                    }

                    // 工具调用与正文/思考等 chunk 照发：前端要能实时显示状态
                    if app
                        .emit(
                            STREAM_EVENT,
                            StreamEvent {
                                stream_id: stream_id.clone(),
                                chunk,
                            },
                        )
                        .is_err()
                    {
                        // 发不出去通常是窗口关了，没必要继续生成
                        return;
                    }
                }
                Err(err) => {
                    tracing::warn!(%err, stream_id = %stream_id, "流式对话出错");
                    emit_error(&app, &stream_id, &err.to_string());
                    return;
                }
            }
        }

        if switch_now {
            drop(stream);
            let to = escalate_to.unwrap_or(Tier::Background);
            tracing::info!(stream_id = %stream_id, model = %provider.model, "实时档想调工具，交给后台档接手");
            // 已经流给用户的开场白留在历史里，并交代后台模型别再说一遍
            if !text.trim().is_empty() {
                req.messages.push(Message::assistant(text));
            }
            req.messages.push(Message::system(crate::llm::gateway::TOOL_ESCALATION_NOTE));
            tier = to;
            escalated = true;
            relock_round_limit = true;
            continue;
        }

        // 用户打断：立刻停，不要执行任何工具。
        // barge-in 的语义是"别说了"，顺手把工具也停掉才符合直觉
        if finish == FinishReason::Cancelled {
            tracing::info!(stream_id = %stream_id, "对话被取消，放弃后续工具执行");
            let _ = app.emit(
                STREAM_EVENT,
                StreamEvent {
                    stream_id: stream_id.clone(),
                    chunk: ChatChunk::Done {
                        reason: FinishReason::Cancelled,
                        usage: None,
                    },
                },
            );
            return;
        }
        if calls.is_empty() {
            // ── 自愈循环 (Self-healing Loop) ──
            // 当工作区发生了文件修改，且最近一次命令执行失败（exit_code != 0），
            // 若模型此时没有调用任何工具就试图宣布结束，系统拦截一次并注入自愈提醒，引导模型进行修复。
            if !hit_round_limit && self_healing_attempts < 2 {
                if let Some(key) = &deps.ledger_key {
                    let ledger = crate::task_ledger::load(&deps.config_dir, key);
                    let has_modified = ledger.touched.values().any(|k| *k == crate::task_ledger::TouchKind::Modified);
                    let last_cmd_failed = ledger.runs.last().map(|r| !r.ok).unwrap_or(false) || ledger.last_failure.is_some();

                    if has_modified && last_cmd_failed {
                        self_healing_attempts += 1;
                        tracing::warn!(stream_id = %stream_id, attempts = self_healing_attempts, "检测到代码修改但最近命令执行失败，触发自愈修复循环");
                        req.messages.push(Message {
                            role: Role::Assistant,
                            content: text,
                            images: Vec::new(),
                            tool_calls: Vec::new(),
                            tool_call_id: None,
                        });
                        req.messages.push(Message::system(VERIFICATION_NUDGE_NOTE));
                        continue;
                    }
                }
            }

            // 没有更多工具调用，说明模型已经给出了最终回复，发射最终 Done 结束流。
            //
            // **但如果这一轮的工具是被我们摘掉的，就不能报 Stop。** 模型不是
            // 说完了，是没工具可用只好收场 —— 内容往往是半截的，实测见过它把
            // exec_command 的参数当正文写出来（一行命令、下一行 30000，
            // 那是 timeout_ms 的默认值）。报 Stop 前端就分不清是模型的问题
            // 还是我们截断了它，用户只看到"话没说完就停了"。
            let reason = if hit_round_limit {
                FinishReason::ToolRoundsExhausted
            } else {
                finish
            };
            let _ = app.emit(
                STREAM_EVENT,
                StreamEvent {
                    stream_id: stream_id.clone(),
                    chunk: ChatChunk::Done {
                        reason,
                        usage: None,
                    },
                },
            );

            // **只在这一个出口提取。** 别的退出路径（出错、被打断、轮次耗尽）
            // 都意味着这一轮是半截的，从半截对话里提取出的"事实"往往是
            // 模型的中间计划而不是结论 —— 而它会永久留在库里。
            //
            // 撞了轮次上限也不提取：`hit_round_limit` 时内容是被我们截断的。
            if !hit_round_limit {
                if let Some(ws) = deps.workspace_path.as_deref() {
                    if let Some(memory) = deps.memory.read().await.clone() {
                        // scope 走 memory.scope_for（项目 id 优先），不是裸路径
                        let scope = memory.scope_for(ws).await;
                        if !scope.is_empty() {
                            // 用户这一轮到底问的是什么 —— 取最后一条 user 消息。
                            // 工具结果是 Role::Tool，不会混进来
                            let user_text = req
                                .messages
                                .iter()
                                .rev()
                                .find(|m| m.role == Role::User)
                                .map(|m| m.content.clone())
                                .unwrap_or_default();
                            memory.remember_project_turn(&user_text, &text, &scope);
                        }
                    }
                }
            }
            return;
        }

        // 模型这一轮说了什么 + 要调什么，先记进历史
        req.messages.push(Message {
            role: Role::Assistant,
            content: text,
            images: Vec::new(),
            tool_calls: calls.clone(),
            tool_call_id: None,
        });

        // 收集 call_id → tool_name 映射，用于发射 tool_result 事件
        let call_names: std::collections::HashMap<String, String> = calls
            .iter()
            .map(|c| (c.id.clone(), c.name.clone()))
            .collect();

        // ── task_plan 前置执行 ──
        //
        // 模型可能在同一轮里并行发射 `[task_plan, fs_list_directory]`（工作区
        // header 里写了「调 task_plan 之前禁止调其它工具」，但那是概率不是保证）。
        // 台账必须**先原子落地**，否则下面那批工具的系统记录会写进一个还不存在
        // 的台账里 —— `record_tool` 在 `is_empty()` 时是空操作，本轮进度静默丢失。
        //
        // 它不走 RuntimeService，理由和 run_skill 一样：只改本地状态，
        // 走审批闸门反而会因为「未知工具默认 Write」被拦。
        // 结果和它的 ok **一起走**，不用 id 反查。反查就需要一个「查不到怎么办」
        // 的默认值，而对「成功还是失败」这种标志，任何默认值都是在猜 ——
        // 而猜「成功」正好是不安全的那一侧。
        let mut plan_results: Vec<(Message, bool)> = Vec::new();
        if let Some(key) = &deps.ledger_key {
            for call in calls.iter().filter(|c| is_task_plan_call(&c.name)) {
                let mut ledger = crate::task_ledger::load(&deps.config_dir, key);
                let text = match parse_task_plan(&call.arguments) {
                    Some((goal, steps, revision)) => {
                        let msg = ledger.apply_plan(&goal, steps, revision.as_deref());
                        match crate::task_ledger::save(&deps.config_dir, key, &ledger) {
                            Ok(()) => msg,
                            Err(err) => format!("台账写入失败：{err}"),
                        }
                    }
                    None => "调用未执行：参数解析不出 goal / steps。".to_string(),
                };
                emit_activity(
                    &app,
                    &stream_id,
                    ToolActivity::Finished {
                        name: call.name.clone(),
                        ok: !text.starts_with("调用未执行"),
                        preview: truncate(&text, 200),
                    },
                );
                let ok = !text.starts_with("调用未执行");
                plan_results.push((
                    Message {
                        role: Role::Tool,
                        content: text,
                        images: Vec::new(),
                        tool_calls: Vec::new(),
                        tool_call_id: Some(call.id.clone()),
                    },
                    ok,
                ));
            }
        }

        let mut tasks = Vec::new();
        for call in calls {
            // task_plan 已经在上面前置执行完了
            if is_task_plan_call(&call.name) {
                continue;
            }
            // 用户点了停止就别再执行剩下的工具
            if cancel.is_cancelled() {
                tracing::info!(stream_id = %stream_id, "本轮已取消，跳过剩余工具");
                break;
            }

            // 同名同参重复太多次就不再执行，直接把原因回给模型。
            // 放在派发之前：既省掉一次工具执行，也避免它把同样的结果
            // 当成"新信息"再想一轮。
            if repeat_guard.should_block(&call.name, &call.arguments) {
                tracing::warn!(
                    stream_id = %stream_id,
                    tool = %call.name,
                    "同一次工具调用重复超限，已拦下"
                );
                let msg = repeat_blocked_message(&call.name, &call.arguments);
                let id = call.id.clone();
                let name = call.name.clone();
                let arguments = call.arguments.clone();
                tasks.push(Box::pin(async move {
                    ToolRun {
                        message: Message {
                            role: Role::Tool,
                            content: msg,
                            images: Vec::new(),
                            tool_calls: Vec::new(),
                            tool_call_id: Some(id),
                        },
                        name,
                        arguments,
                        // 被拦下不算执行成功 —— 台账里不该出现一条"命令跑过了"
                        ok: false,
                    }
                }) as BoxedToolFuture);
                continue;
            }

            let app_clone = app.clone();
            let deps_clone = deps.clone();
            let stream_id_clone = stream_id.clone();
            let cancel_clone = cancel.clone();
            let tier_str = model_tier_str.to_string();

            tasks.push(Box::pin(async move {
                let (result_text, images, ok) = execute_one(
                    &app_clone,
                    &deps_clone,
                    &stream_id_clone,
                    &call,
                    &cancel_clone,
                    &tier_str,
                )
                .await;
                let message = Message {
                    role: Role::Tool,
                    // **必须封顶。** 内置工具各自有上限（shell_command 20000、
                    // exec_command 64000），但 MCP 那条路一个都没有 ——
                    // 实测 filesystem 的 directory_tree 在这个仓库返回约 9.3 MB
                    // （233364 个条目，含 target/ 和 node_modules/），
                    // 折合两百多万 token，而模型上下文是 32768。
                    // 结果是这一轮和后面每一轮都带着它，请求越来越大。
                    content: clamp_tool_result(&call.name, result_text),
                    images,
                    tool_calls: Vec::new(),
                    tool_call_id: Some(call.id.clone()),
                };
                ToolRun {
                    message,
                    name: call.name.clone(),
                    arguments: call.arguments.clone(),
                    ok,
                }
            }) as BoxedToolFuture);
        }

        let runs = futures_util::future::join_all(tasks).await;

        // ── 系统侧把这一轮记进台账 ──
        //
        // **每轮 flush 一次，不是每次工具调用。** 至多 40 次小写入，
        // 而且崩溃最多丢一轮。这是台账里「进度」的唯一来源 ——
        // 实测模型在失败之后从不回头更新台账（两个模型 0/6，逐次相同），
        // 所以进度问系统、不问模型。
        if let Some(key) = &deps.ledger_key {
            let mut ledger = crate::task_ledger::load(&deps.config_dir, key);
            if !ledger.is_empty() {
                for r in &runs {
                    ledger.record_tool(
                        &r.name,
                        &r.arguments,
                        r.ok,
                        &r.message.content,
                        round as u32 + 1,
                    );
                }
                if let Err(err) = crate::task_ledger::save(&deps.config_dir, key, &ledger) {
                    // 台账写不进去不该让这一轮失败 —— 它是增强，不是必需
                    tracing::warn!(%err, "台账写入失败");
                }
            }
        }

        let mut results: Vec<(Message, bool)> = plan_results;
        results.extend(runs.into_iter().map(|r| (r.message, r.ok)));

        // 把工具执行的完整结果推给前端，用于跨轮上下文保留。
        // 与 ToolActivity::Finished 的 preview 不同：那边截断到 200 字符供日志，
        // 这里保留完整内容（上限 20000 字符）供压缩和上下文重建使用。
        for (result_msg, ok) in &results {
            if result_msg.role == Role::Tool {
                let call_id = result_msg.tool_call_id.clone().unwrap_or_default();
                let tool_name = call_names.get(&call_id).cloned().unwrap_or_default();
                let ok = *ok;
                let _ = app.emit(
                    STREAM_EVENT,
                    StreamEvent {
                        stream_id: stream_id.clone(),
                        chunk: ChatChunk::ToolResult {
                            call_id,
                            name: tool_name,
                            content: truncate(&result_msg.content, MAX_FRONTEND_TOOL_RESULT_CHARS),
                            // 发出去时永远是确定值 —— 它和结果一起走，不靠反查
                            ok: Some(ok),
                        },
                    },
                );
            }
        }

        req.messages.extend(results.into_iter().map(|(m, _)| m));
    }

    // 走到这里说明每一轮都以 tool_calls 收尾（中间的 Done 都被吞掉了），
    // 前端还在等一个结束信号。不补的话界面会永远停在"思考中"。
    tracing::warn!(stream_id = %stream_id, "工具往返用尽仍未收敛，补发结束事件");
    let _ = app.emit(
        STREAM_EVENT,
        StreamEvent {
            stream_id: stream_id.clone(),
            chunk: ChatChunk::Done {
                // **不能报 Stop。** 这不是模型说完了，是轮次用尽被我们截断的。
                // 报 Stop 前端就没法区分，用户只能看到"话没说完"。
                reason: FinishReason::ToolRoundsExhausted,
                usage: None,
            },
        },
    );
}

/// 过闸门 → （必要时）问用户 → 执行。
///
/// 无论走到哪个分支，**都会返回一段要回填给模型的文本** ——
/// 被拒也要告诉模型为什么被拒，它才不会原地重试。
/// 两条产出 `Message` 的路径（真执行 / 被重复检测拦下）要能放进同一个
/// `Vec` 里 join_all，所以统一装箱。不装箱的话两个 async block 是不同类型。
type BoxedToolFuture = std::pin::Pin<Box<dyn std::future::Future<Output = ToolRun> + Send>>;

/// `task_plan` 的工具名。
pub const TASK_PLAN_NAME: &str = "task_plan";

fn is_task_plan_call(name: &str) -> bool {
    name == TASK_PLAN_NAME || name.ends_with(&format!("__{TASK_PLAN_NAME}"))
}

/// 解析 `task_plan` 的参数。
///
/// 和别处一样宽容：本地小模型会把字段名写错。解析不出来返回 `None`，
/// 调用方回一句人能看懂的话，不 panic。
fn parse_task_plan(raw: &str) -> Option<(String, Vec<String>, Option<String>)> {
    let v: serde_json::Value = serde_json::from_str(raw.trim()).ok()?;
    let goal = v
        .get("goal")
        .or_else(|| v.get("objective"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let steps: Vec<String> = v
        .get("steps")
        .or_else(|| v.get("plan"))
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|e| {
                    // 字符串数组，或者 [{title: "…"}] —— 两种都见过
                    e.as_str().map(|s| s.to_string()).or_else(|| {
                        e.get("title")
                            .or_else(|| e.get("step"))
                            .and_then(|t| t.as_str())
                            .map(|s| s.to_string())
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    if goal.trim().is_empty() && steps.is_empty() {
        return None;
    }
    let revision = v
        .get("goal_revision")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());
    Some((goal, steps, revision))
}

/// 一次工具执行的产出。
///
/// 除了回填给模型的 `Message`，还带着系统侧记台账要用的三样。
/// **`ok` 不能从 `message.content` 反推** —— 失败的输出和正常输出长得一样，
/// 而台账里「命令失败过」这条信息的唯一来源就是它。
struct ToolRun {
    message: Message,
    name: String,
    arguments: String,
    ok: bool,
}

/// 同名同参的工具调用重复多少次就拦下来。
///
/// 3 次而不是 2 次：偶尔重试一次是合理的（上一次返回了空、或者刚写完想再读
/// 一遍确认），第三次就说明它卡住了。
const REPEAT_LIMIT: usize = 3;

/// 一整轮对话里，同名同参的工具调用出现过几次。
///
/// **为什么需要。** 实测撞到过：模型不知道工作区的绝对路径，于是反复
/// `list_directory(".")`，MCP 每次返回同样的东西，它就再调一次 ——
/// 十几次之后轮次烧光，用户看到一次没有结论的对话。
/// 光靠轮次上限兜不住：那只保证会结束，不保证结束前做了有用的事。
#[derive(Default)]
struct RepeatGuard {
    seen: std::collections::HashMap<String, usize>,
}

impl RepeatGuard {
    /// 记一次调用，返回这是第几次。
    fn record(&mut self, name: &str, arguments: &str) -> usize {
        let key = format!("{name}\u{1}{arguments}");
        let n = self.seen.entry(key).or_insert(0);
        *n += 1;
        *n
    }

    /// 该拦下来了吗
    fn should_block(&mut self, name: &str, arguments: &str) -> bool {
        self.record(name, arguments) > REPEAT_LIMIT
    }
}

/// 拦下重复调用时回给模型的话。
///
/// 必须说清楚**为什么**被拦、以及**该换什么做法** —— 只说「重复调用」的话
/// 模型往往换个空格再调一次。
fn repeat_blocked_message(name: &str, arguments: &str) -> String {
    format!(
        "已拦下：`{name}` 带完全相同的参数被调用了超过 {REPEAT_LIMIT} 次，参数是 \
         {arguments}。重复同一次调用不会得到不同的结果。\n\n\
         请换一种做法：确认路径是否正确（工作区的绝对路径在 system 提示里）、\
         换用别的工具、缩小或扩大查询范围，或者直接基于已有信息作答。"
    )
}

/// 单条工具结果进上下文的字符上限。
///
/// 本地档的历史预算是 48000 字符，所以一条结果占到 20000 已经是「这一轮别想
/// 装下别的东西了」。真需要读大文件应该分段读，而不是把整棵树塞进来。
const MAX_TOOL_RESULT_CHARS: usize = 20_000;

/// 给工具结果封顶，并且**告诉模型被截断了**。
///
/// 不说的话模型会以为自己看到了全部 —— 比"结果太长"糟得多：
/// 它会基于一棵被砍掉尾巴的目录树下结论，而且看不出问题在哪。
fn clamp_tool_result(tool_name: &str, text: String) -> String {
    if text.chars().count() <= MAX_TOOL_RESULT_CHARS {
        return text;
    }
    let kept: String = text.chars().take(MAX_TOOL_RESULT_CHARS).collect();
    let total = text.chars().count();
    tracing::warn!(
        tool = tool_name,
        total_chars = total,
        kept = MAX_TOOL_RESULT_CHARS,
        "工具结果超长，已截断"
    );
    format!(
        "{kept}\n\n[工具 {tool_name} 的输出共 {total} 字符，此处只保留了前 \
         {MAX_TOOL_RESULT_CHARS} 字符。**你看到的不是全部** —— \
         需要完整内容请缩小范围重新调用，例如只列某一个子目录、或分段读文件。]"
    )
}

async fn execute_one(
    app: &AppHandle,
    deps: &LoopDeps,
    stream_id: &str,
    call: &ToolCall,
    cancel: &CancellationToken,
    model_tier: &str,
) -> (String, Vec<String>, bool) {
    // ── 内置工具先派发 ──
    //
    // `run_skill`：任务技能。**返回值就是那份技能的规程正文**，模型在本轮照着做。
    //
    // 在这里拦而不走 RuntimeService：它不碰文件、不执行命令，
    // 唯一的作用是把一段文本放进上下文。走闸门反而会因为
    // 「未知工具默认 Write」被拦下。
    //
    // 这也是「语音调用技能」的实现 —— 用户说"审查一下 lru.ts"，模型自己
    // 调这个工具。键盘那条路（`/review`）由前端展开后直接发消息，
    // 两条路共用 `skill::expand_task`。
    if crate::skill::is_run_call(&call.name) {
        let Some((id, args)) = crate::skill::run_from_arguments(&call.arguments) else {
            return (
                format!(
                    "调用未执行：`{}` 的参数解析不出技能 id。参数应当是 \
                     {{\"skill\": \"<id>\", \"arguments\": \"<可选>\"}}。",
                    call.name
                ),
                vec![],
                false,
            );
        };
        let ws = deps.workspace_path.as_deref().map(std::path::Path::new);
        return match crate::skill::resolve_layered(&deps.config_dir, ws, &id) {
            Ok(skill) if skill.kind == crate::skill::SkillKind::Task => {
                (crate::skill::expand_task(&skill, &args), vec![], true)
            }
            // 身份型技能不该用 run_skill 载入：它是常驻的，由用户在设置里勾选。
            // 说清楚而不是照样展开 —— 否则「怎么切角色」会出现两种做法。
            Ok(skill) => (
                format!(
                    "「{}」是身份型技能（persona），不能用 run_skill 载入 —— \
                     它由用户在设置面板里勾选后常驻生效。如实告诉用户。",
                    skill.name
                ),
                vec![],
                false,
            ),
            Err(err) => (format!("调用未执行：{err}"), vec![], false),
        };
    }

    if deps.runtime.contains_tool(&call.name)
        || deps
            .runtime
            .contains_tool_with_mcp(&call.name, deps.mcp.clone())
            .await
    {
        let workspace_path = deps
            .workspace_path
            .clone()
            .unwrap_or_else(|| deps.config_dir.to_string_lossy().to_string());
        let mut arguments = match serde_json::from_str::<serde_json::Value>(&call.arguments) {
            Ok(v) => v,
            Err(err) => return (format!("调用未执行：参数不是合法 JSON：{err}"), vec![], false),
        };
        if matches!(call.name.as_str(), "generate_image" | "generate_video" | "animate_image") {
            if let Some(obj) = arguments.as_object_mut() {
                obj.insert(
                    "__ui_call_id".to_string(),
                    serde_json::Value::String(call.id.clone()),
                );
            }
        }
        emit_activity(
            app,
            stream_id,
            ToolActivity::Started {
                name: call.name.clone(),
                arguments: pretty(&arguments),
            },
        );
        let app_hb = app.clone();
        let stream_id_hb = stream_id.to_string();
        let tool_name_hb = call.name.clone();
        let is_subagent = tool_name_hb.contains("subagent");
        let mut runtime_event_cursor = deps.runtime.recent_events().await.len();

        let outcome = {
            let mut exec_fut = Box::pin(deps.runtime.execute_tool_with_mcp(
                crate::agent_runtime::RuntimeToolCallArgs {
                    workspace_path,
                    tool_name: call.name.clone(),
                    arguments,
                    network_enabled: matches!(call.name.as_str(), "search_web" | "web_search"),
                    approval_mode: crate::agent_runtime::RuntimeApprovalMode::Wait,
                    model_tier: model_tier.to_string(),
                },
                deps.mcp.clone(),
            ));

            let mut ticker = tokio::time::interval(tokio::time::Duration::from_millis(500));
            // 第一次 tick 会立即完成，先 consume 掉
            ticker.tick().await;
            let mut elapsed_secs = 0u64;

            loop {
                tokio::select! {
                    _ = cancel.cancelled() => {
                        return ("用户取消了工具调用。".to_string(), vec![], false);
                    }
                    r = &mut exec_fut => {
                        forward_image_progress_events(
                            &app_hb,
                            deps,
                            &stream_id_hb,
                            &mut runtime_event_cursor,
                        )
                        .await;
                        break r;
                    }
                    _ = ticker.tick() => {
                        forward_image_progress_events(
                            &app_hb,
                            deps,
                            &stream_id_hb,
                            &mut runtime_event_cursor,
                        )
                        .await;
                        elapsed_secs += 1;
                        let hint = if is_subagent {
                            Some("Subagent executing background reasoning and tools...".to_string())
                        } else {
                            None
                        };
                        emit_activity(
                            &app_hb,
                            &stream_id_hb,
                            ToolActivity::Running {
                                name: tool_name_hb.clone(),
                                elapsed_secs,
                                hint,
                            },
                        );
                    }
                }
            }
        };
        match outcome {
            Ok(result) => {
                let ok = result.outcome.status == crate::agent_runtime::ToolStatus::Ok;
                let model_output = runtime_outcome_for_model(&result.outcome);
                emit_activity(
                    app,
                    stream_id,
                    ToolActivity::Finished {
                        name: call.name.clone(),
                        ok,
                        preview: truncate(&model_output, 200),
                    },
                );
                let image_data_urls: Vec<String> = result
                    .outcome
                    .artifacts
                    .into_iter()
                    .filter(|a| {
                        a.starts_with("data:image/")
                            || a.starts_with("http://")
                            || a.starts_with("https://")
                    })
                    .collect();
                return (model_output, image_data_urls, ok);
            }
            Err(err) => {
                let message = format!("runtime 工具执行失败：{err}");
                emit_activity(
                    app,
                    stream_id,
                    ToolActivity::Finished {
                        name: call.name.clone(),
                        ok: false,
                        preview: truncate(&message, 200),
                    },
                );
                return (message, vec![], false);
            }
        }
    }

    if call.name.contains("__") {
        let reason =
            "该 MCP 工具未在 runtime registry 中暴露，可能已被权限策略隐藏或 server 已断开。";
        emit_activity(
            app,
            stream_id,
            ToolActivity::Rejected {
                name: call.name.clone(),
                reason: reason.to_string(),
            },
        );
        return (format!("调用未执行：{reason}"), vec![], false);
    }

    let content = format!("未知工具：{}。请检查工具名或重新选择可用工具。", call.name);
    emit_activity(
        app,
        stream_id,
        ToolActivity::Finished {
            name: call.name.clone(),
            ok: false,
            preview: truncate(&content, 200),
        },
    );
    (content, vec![], false)
}

fn runtime_outcome_for_model(outcome: &crate::agent_runtime::ToolOutcome) -> String {
    if outcome.status == crate::agent_runtime::ToolStatus::Ok {
        return outcome.content.clone();
    }

    let mut text = format!(
        "runtime 工具 {} 执行未成功，状态：{:?}。",
        outcome.tool_name, outcome.status
    );
    if !outcome.content.trim().is_empty() {
        text.push_str("\n结果：");
        text.push_str(&outcome.content);
    }
    if let Some(failure) = &outcome.failure {
        text.push_str("\n失败类型：");
        text.push_str(&format!("{:?}", failure.kind));
        if let Some(hint) = &failure.hint {
            text.push_str("\n建议：");
            text.push_str(hint);
        }
    }
    text
}

/// 把 runtime 侧新产生的**图片生成进度**事件转发给前端。
///
/// 图片生成是长任务（ComfyUI 在 MPS 上一张图几十秒），而工具调用是一次
/// `await` —— 期间前端只能看到"某个工具在跑"，看不到进展。`image_tool.rs`
/// 会往 runtime 事件日志里发 `ImageGenerationProgress`，这里负责搬到前端。
///
/// `cursor` 是**已消费到的位置**，调用方在开始等之前初始化成当时的事件数
/// （`recent_events().len()`），所以只会转发这次工具调用期间新增的那些。
/// 每次转发后要往前推 —— 不推的话同一条进度会随心跳反复发给前端。
///
/// 注意 `recent_events()` 是**有上限的环形日志**：如果期间事件多到把游标
/// 冲掉，`len()` 会比 cursor 小，这时从 0 开始重放剩下的（宁可重复一次，
/// 也不要因为下标越界 panic 掉整轮对话）。
async fn forward_image_progress_events(
    app: &AppHandle,
    deps: &LoopDeps,
    stream_id: &str,
    cursor: &mut usize,
) {
    let events = deps.runtime.recent_events().await;
    if events.len() < *cursor {
        *cursor = 0;
    }
    for event in events.iter().skip(*cursor) {
        match event {
            crate::agent_runtime::RuntimeEvent::ImageGenerationProgress { output_path, .. } => {
                if let Some(path) = output_path {
                    if let Some(dir) = std::path::Path::new(path).parent() {
                        use tauri::Manager;
                        let _ = app.asset_protocol_scope().allow_directory(dir, false);
                    }
                }
                let _ = app.emit(
                    IMAGE_PROGRESS_EVENT,
                    serde_json::json!({
                        "stream_id": stream_id,
                        "event": event,
                    }),
                );
            }
            crate::agent_runtime::RuntimeEvent::VideoGenerationProgress { output_path, .. } => {
                if let Some(path) = output_path {
                    if let Some(dir) = std::path::Path::new(path).parent() {
                        use tauri::Manager;
                        let _ = app.asset_protocol_scope().allow_directory(dir, false);
                    }
                }
                let _ = app.emit(
                    VIDEO_PROGRESS_EVENT,
                    serde_json::json!({
                        "stream_id": stream_id,
                        "event": event,
                    }),
                );
            }
            _ => {}
        }
    }
    *cursor = events.len();
}

fn emit_activity(app: &AppHandle, stream_id: &str, activity: ToolActivity) {
    let _ = app.emit(
        TOOL_EVENT,
        ToolActivityEvent {
            stream_id: stream_id.to_string(),
            activity,
        },
    );
}

fn emit_error(app: &AppHandle, stream_id: &str, message: &str) {
    let _ = app.emit(
        STREAM_EVENT,
        StreamErrorEvent {
            stream_id: stream_id.to_string(),
            kind: "error",
            message: message.to_string(),
        },
    );
}

fn pretty(value: &serde_json::Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    text.chars().take(max_chars).collect::<String>() + "…"
}

/// 一个 provider 属于哪一档模型能力（`"local"` / `"cloud"`）。
///
/// **这是 tier 的唯一计算入口。** 前端不做判断，只读后端下发的
/// `ProviderStatus::model_tier`（见 `config/mod.rs`）；后端两处需要
/// tier 的地方（下发给前端、agent 流式对话）都调这个函数。
///
/// 早先这段 match 在 `config/mod.rs` 和这里各写了一份，逐字相同。
/// 加新 `ProviderKind` 时编译器会在两处同时报错（match 是穷尽的、
/// 没有 `_` 通配臂），所以那个方向是安全的；但**改某一条已有臂**
/// 编译器抓不到 —— 比如决定 `AzureOpenAi` 在某种部署下算 local，
/// 只在一处改了，两处就静默分叉：前端按一档给历史预算，
/// agent 按另一档给读文件行数，谁都不报错。抽成一份就没有这种可能。
pub fn tier_of(config: &ProviderConfig) -> &'static str {
    match config.kind {
        ProviderKind::OpenAi | ProviderKind::Anthropic | ProviderKind::AzureOpenAi => "cloud",
        ProviderKind::OpenAiCompatible => {
            // 同一个协议既有本机的 LM Studio / MLX，也有 DeepSeek / Groq
            // 这类公网兼容端点 —— 判据只能是地址，不是协议
            if is_local_endpoint(&config.base_url) {
                "local"
            } else {
                "cloud"
            }
        }
        ProviderKind::Ollama => "local",
    }
}

/// 判断给定的 endpoint URL 是否为本地或局域网私网端点（RFC1918）
pub fn is_local_endpoint(base_url: &str) -> bool {
    let url_lower = base_url.trim().to_lowercase();
    let without_proto = if let Some(rest) = url_lower.strip_prefix("http://") {
        rest
    } else if let Some(rest) = url_lower.strip_prefix("https://") {
        rest
    } else {
        &url_lower
    };

    let host_and_port = without_proto
        .split(&['/', '?', '#'][..])
        .next()
        .unwrap_or("")
        .trim();

    let host = if host_and_port.starts_with('[') {
        host_and_port
            .split(']')
            .next()
            .map(|s| format!("{s}]"))
            .unwrap_or_else(|| host_and_port.to_string())
    } else {
        host_and_port.split(':').next().unwrap_or("").to_string()
    };

    if host.is_empty() {
        return true; // 解析异常时保守判 local
    }

    host == "localhost"
        || host.ends_with(".local")
        || host == "0.0.0.0"
        || host == "::1"
        || host == "[::1]"
        || host.starts_with("127.")
        || host.starts_with("10.")
        || host.starts_with("192.168.")
        || (host.starts_with("172.")
            && (16..=31).any(|octet| host.starts_with(&format!("172.{}.", octet))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 轮次用尽和正常结束必须是不同的_finish_reason() {
        // 报 Stop 的话前端分不清"模型说完了"和"我们截断了它"。
        // 实测撞到过：工具被摘掉后模型把 exec_command 的参数当正文写出来
        // （一行 `cd app && npm test`、下一行 `30000` —— 那是 timeout_ms
        // 的默认值），用户看到的就是一段半截的话然后停了。
        assert_ne!(FinishReason::ToolRoundsExhausted, FinishReason::Stop);
        // 也不能和取消混为一谈：那是用户主动打断，语义完全不同
        assert_ne!(FinishReason::ToolRoundsExhausted, FinishReason::Cancelled);
    }

    /// 有换档目标、还没换过 → 换；换过一次之后后台档自己调工具，照常执行
    #[test]
    fn 小模型想调工具时每轮只换一次档() {
        assert!(should_escalate(Some(Tier::Background), false));
        assert!(!should_escalate(Some(Tier::Background), true), "换过还换 —— 后台档的工具永远执行不了");
        assert!(!should_escalate(None, false), "两档同一个模型时不该换");
    }

    #[test]
    fn 轮次上限本地比云端小且都留有余量() {
        // 本地模型慢、错更多，上限低是有意的；但太低会让多文件任务做不完 ——
        // 实测一个五文件的 LRU 任务在 12 轮上撞了两次。
        assert!(MAX_TOOL_ROUNDS_LOCAL < MAX_TOOL_ROUNDS_CLOUD);
        assert_eq!(MAX_TOOL_ROUNDS_LOCAL, 40);
        assert_eq!(MAX_TOOL_ROUNDS_CLOUD, 100);
    }

    #[test]
    fn 工具耗尽收尾提示要求模型不要伪造工具调用正文() {
        assert!(TOOL_ROUNDS_EXHAUSTED_SYSTEM_NOTE.contains("不能再调用工具"));
        assert!(TOOL_ROUNDS_EXHAUSTED_SYSTEM_NOTE.contains("不要把工具调用参数"));
        assert!(TOOL_ROUNDS_EXHAUSTED_SYSTEM_NOTE.contains("继续"));
    }

    #[test]
    fn 前三次同样的调用放行第四次拦下() {
        // 3 次而不是 2 次：偶尔重试一次是合理的（上次返回空、或刚写完想确认），
        // 第三次才说明卡住了。
        let mut g = RepeatGuard::default();
        let (n, a) = ("filesystem__list_directory", r#"{"path":"."}"#);
        assert!(!g.should_block(n, a), "第 1 次要放行");
        assert!(!g.should_block(n, a), "第 2 次要放行");
        assert!(!g.should_block(n, a), "第 3 次要放行");
        assert!(g.should_block(n, a), "第 4 次要拦下");
    }

    #[test]
    fn 参数不同就是不同的调用() {
        // 遍历目录本来就要连着调很多次不同路径，不能因为工具名相同就拦
        let mut g = RepeatGuard::default();
        for i in 0..20 {
            let args = format!(r#"{{"path":"/a/{i}"}}"#);
            assert!(
                !g.should_block("filesystem__list_directory", &args),
                "第 {i} 个不同路径被误拦"
            );
        }
    }

    #[test]
    fn 工具名不同也是不同的调用() {
        let mut g = RepeatGuard::default();
        let a = r#"{"path":"."}"#;
        for t in [
            "list_directory",
            "read_text_file",
            "directory_tree",
            "view_file",
        ] {
            assert!(!g.should_block(t, a), "{t} 被误拦");
        }
    }

    #[test]
    fn 拦下时要说清原因和替代做法() {
        // 只说"重复调用"的话，模型往往换个空格再调一次
        let msg = repeat_blocked_message("list_directory", r#"{"path":"."}"#);
        assert!(msg.contains("list_directory"), "要指名是哪个工具");
        assert!(msg.contains(r#"{"path":"."}"#), "要把参数回显出来");
        assert!(msg.contains("不会得到不同的结果"), "要说清为什么拦");
        assert!(msg.contains("绝对路径"), "要给出下一步该怎么做");
    }

    #[test]
    fn 键的拼接不会让两次不同调用撞在一起() {
        // 用 \u{1} 分隔而不是空字符串：否则 ("ab","c") 和 ("a","bc") 同键
        let mut g = RepeatGuard::default();
        assert_eq!(g.record("ab", "c"), 1);
        assert_eq!(g.record("a", "bc"), 1, "两个不同调用被算成同一个");
    }

    #[test]
    fn 短结果原样通过() {
        let t = "hello".to_string();
        assert_eq!(clamp_tool_result("view_file", t.clone()), t);
    }

    #[test]
    fn 超长结果截断并明确告知不是全部() {
        // 实测背景：filesystem 的 directory_tree 在 Anima 仓库返回约 9.3 MB
        // （233364 个条目），折合两百多万 token，而模型上下文是 32768。
        let huge = "x".repeat(MAX_TOOL_RESULT_CHARS * 3);
        let out = clamp_tool_result("filesystem__directory_tree", huge);
        assert!(out.contains("不是全部"), "必须告诉模型被截断了");
        assert!(
            out.contains("filesystem__directory_tree"),
            "要指名是哪个工具"
        );
        // 总量真的受控 —— 尾注本身也算在上下文里
        assert!(
            out.chars().count() < MAX_TOOL_RESULT_CHARS + 300,
            "实际 {} 字符",
            out.chars().count()
        );
    }

    #[test]
    fn 按字符截断而不是字节() {
        // 中文一个字三个字节，按字节切会切出半个字，模型侧解码成乱码
        let cn = "目".repeat(MAX_TOOL_RESULT_CHARS + 100);
        let out = clamp_tool_result("t", cn);
        assert!(out.starts_with("目目目"), "开头不能是乱码");
    }

    #[test]
    fn 恰好等于上限时不截断() {
        // 边界：<= 才放行，多一个字符才截。差一个字符就会给每条正常结果都挂尾注
        let exact = "y".repeat(MAX_TOOL_RESULT_CHARS);
        let out = clamp_tool_result("t", exact.clone());
        assert_eq!(out, exact);
    }

    #[test]
    fn truncate_counts_characters_not_bytes() {
        // 按字节切会把中文切成半个字符，输出乱码
        assert_eq!(truncate("中文测试", 2), "中文…");
        assert_eq!(truncate("abc", 10), "abc");
    }

    #[test]
    fn local_endpoint_recognizes_localhost_and_rfc1918() {
        assert!(is_local_endpoint("http://localhost:1234/v1"));
        assert!(is_local_endpoint("http://127.0.0.1:11434"));
        assert!(is_local_endpoint("http://192.168.1.50:1234/v1"));
        assert!(is_local_endpoint("http://10.0.0.5:8000/v1"));
        assert!(is_local_endpoint("http://172.20.10.2:1234/v1"));
        assert!(is_local_endpoint("http://my-mac.local:1234/v1"));

        assert!(!is_local_endpoint("https://api.openai.com/v1"));
        assert!(!is_local_endpoint("https://api.deepseek.com/v1"));
        assert!(!is_local_endpoint("https://api.groq.com/openai/v1"));

        // 避免误判含 10. 的非私网域名或路径
        assert!(!is_local_endpoint("https://graph.example.com/v10.0/chat"));
        assert!(!is_local_endpoint("https://api.x.ai/v1/chat?t=110.5"));
        assert!(!is_local_endpoint("https://model-710.vendor.ai/v1"));
    }
}


#[cfg(test)]
mod 换任务归档 {
    use super::is_continuation;

    /// 明确的接续词要认出来 —— 认错的代价是把一个多轮任务的目标中途抹掉。
    #[test]
    fn 接续词不算新任务() {
        for t in ["继续", "接着", "接着做", "往下", "继续 ", "go on", "Continue", "next"] {
            assert!(is_continuation(t), "`{t}` 该被当成接续");
        }
    }

    /// 正名：正常的新请求必须被当成新任务，否则旧任务卡永远不消失。
    #[test]
    fn 正常请求算新任务() {
        for t in [
            "查看当前项目是否还有挂起的更改？",
            "帮我对比几个六爻项目的算法",
            "继续优化一下这段代码的性能并补上测试", // 以「继续」开头但明显是新活儿
            "",
        ] {
            assert!(!is_continuation(t), "`{t}` 不该被当成接续");
        }
    }
}
