//! Tauri command 层。前端唯一能碰到 Rust 的入口。
//!
//! 流式 token 走 **event 广播**而不是 command 返回值（见 ARCHITECTURE.md 坑点 1）：
//! 每条流一个 `stream_id`，前端订阅 [`STREAM_EVENT`] 后按 id 过滤。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, State};

use crate::agent_runtime::image_engine as eng;
use crate::config::{secrets, ProviderStatus, Tier};
use crate::error::{Error, Result};
use crate::llm::{ChatChunk, ChatRequest, Message};
use crate::AppState;

/// 流式事件名。前端 `listen(STREAM_EVENT, ...)`。
pub const STREAM_EVENT: &str = "anima://llm-stream";

/// 一条流式事件。`stream_id` 用于前端区分并发的多条流
/// （例如实时对话之外还挂着一个后台记忆整理任务）。
#[derive(Debug, Clone, Serialize)]
pub struct StreamEvent {
    pub stream_id: String,
    #[serde(flatten)]
    pub chunk: ChatChunk,
}

/// 上游出错时单独发一个 error 事件，而不是塞进 ChatChunk。
#[derive(Debug, Clone, Serialize)]
pub struct StreamErrorEvent {
    pub stream_id: String,
    pub kind: &'static str,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct ChatArgs {
    pub messages: Vec<Message>,
    #[serde(default)]
    pub tier: Option<Tier>,
    #[serde(default)]
    pub workspace_path: Option<String>,
    /// 当前会话 id。任务台账按它分键 —— 同一项目里开两个会话做两件事
    /// 是常见操作，那正是 session 存在的理由。
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct SaveChatImageAttachmentArgs {
    pub data_url: String,
    pub filename: String,
    #[serde(default)]
    pub workspace_path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SavedChatImageAttachment {
    pub path: String,
    pub file_url: String,
}

#[derive(Debug, Serialize)]
pub struct ChatHandle {
    /// 前端拿这个 id 订阅流式事件，也用它调 `cancel_chat`
    pub stream_id: String,
    /// 本轮实际使用的后端（可能因工具调用降级而与设置里的不同）
    pub provider_id: String,
    pub model: String,
}

/// 发起一轮对话。立刻返回 stream_id，token 通过 event 陆续推送。
#[tauri::command]
pub async fn start_chat(
    app: AppHandle,
    state: State<'_, AppState>,
    args: ChatArgs,
) -> Result<ChatHandle> {
    let stream_id = uuid::Uuid::new_v4().to_string();
    let tier = args.tier.unwrap_or(Tier::Realtime);

    let mut req = ChatRequest::new(args.messages);
    req.temperature = args.temperature;
    req.max_tokens = args.max_tokens;
    // 按链路取档位。实时档默认关（本机实测 6.99s → 0.08s），
    // 后台档默认不干预 —— 拿时间换质量正是那个档存在的意义。
    // 档位怎么落到线上由 llm::thinking 决定，这里只表达意图。
    req.thinking = {
        let config = state.gateway.config();
        let config = config.read().await;
        config.thinking.for_tier(tier)
    };
    // 项目指令（PROMPT.md / rules.md）。
    //
    // **在这里读而不是让前端带过来。** 前端只在切项目时调
    // `workspace_check_status`，用户改完 PROMPT.md 不会重新拉，于是文件里
    // 写的规矩和模型收到的对不上、且没有任何迹象。在此之前这两份文件只被
    // 设置面板读来显示 —— 模板生成的「Principal 级软件架构师」那套指令
    // 一直躺在磁盘上没送出去，选 developer 模板和选 general 没有区别。
    //
    // 放在最前面：后面前端那条通用人设更靠后、优先级更高，
    // 项目指令是背景约束而不是覆盖人设。
    if let Some(ws) = args
        .workspace_path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        // **先告诉它工作区在哪。** 在此之前 workspace_path 只用来决定
        // 要不要注入工具，从来没进过提示词 —— 模型只能猜，而两种猜法都会出事：
        //
        //   猜 "."          内置工具认（resolve_workspace_path 会拼上根目录），
        //                   但 MCP filesystem 只认绝对路径 → 反复调同一个
        //                   list_directory("."），把轮次烧光
        //   问 MCP 允许目录  拿到的是那个服务器的启动参数（和当前项目无关），
        //                   于是跑去分析了别的仓库
        //
        // 两种都实际发生过。
        //
        // 下面「多步任务」那一段里的**「禁止」**不是修辞。实测同一个多步任务、
        // 三种措辞，看首个工具调用是不是 task_plan：
        //
        //   |                     | 「先用 task_plan 写下计划」 | 加触发条件 | **禁止**先调其它工具 |
        //   | ornith-1.5-35b-a3b  | ❌ 直奔 fs_list_directory   | ❌ 同上    | ✅ goal + 4 步       |
        //   | qwen/qwen3.8-27b    | ✅                          | ✅         | ✅                   |
        //
        // 措辞比模型更决定结果。改这段话之前先跑一遍那个对照。
        //
        // ── 2026-08-24：「一步就能做完的任务会不会被推去写计划」──
        //
        // 起因是用户只说了「把所有未跟踪文件加入 git 跟踪」，却等来一大篇分析。
        // 用 `live_trivial_task_should_not_trigger_analysis` 量首个工具调用，
        // 三个真实模型各 3 次（`PROMPT_AB_MODELS` 指定模型）：
        //
        //   |                     | 真实线上栈      | 加了「Act directly」 |
        //   | ornith-1.5-35b-a3b  | shell_command   | shell_command        |
        //   | openai/gpt-oss-120b | **task_plan**   | shell_command        |
        //   | qwen/qwen3.8-27b    | shell_command   | shell_command        |
        //
        // 三点结论：
        //
        // 1. **只有 gpt-oss 需要下面那句 `Act directly`**，另两个本来就直接动手。
        // 2. 罪魁不是本 Header，是**前端人设 + 项目 `.anima/rules.md`** 那几条
        //    （「结论先行」「分板块归纳总结」「透明反馈：清晰列出当前所处阶段」）。
        //    单跑本 Header 时 gpt-oss 也是 shell_command 3/3 ——
        //    **测提示词必须测完整那一栈**，少一段就是在测线上不存在的东西。
        // 3. 我一度以为是 ornith 不听话（上面那张表里它在别的场景确实最不听话），
        //    测完发现猜错了。**依从性是按场景变的，不是按模型排座次。**
        //
        // 这个对照的局限也记一笔：它**只量第一个动作**，量不到「动手之后停不下来」。
        // 用户反馈里那次 5 条命令 198 秒，问题出在第 2~5 条，现有对照测不出来。
        //
        // ── 一条**测过之后决定不加**的规则 ──
        //
        // 用户又反馈模型会说「我先冷静下来」「我先停一下，冷静核查真实状态」这类
        // 自我情绪调节的话。看着像是人设那句「有性格、自然灵动」加上项目规则
        // 「透明反馈：清晰列出当前所处阶段」催出来的，于是试着加了一句
        // 「Report findings, not your process. Never narrate your own state…」。
        //
        // 实测（ornith，各 3 次）：不加是 44 字、加了 49 字，
        // 而且**干净上下文里它压根不说那种话** —— 说的是很正常的
        // 「我来帮你把…添加到 git 跟踪。先看一下当前的…」。
        //
        // 也就是说那个语气不是提示词来的，是**那次会话的历史**来的：
        // 模型在收拾自己上一轮的烂摊子（它自己的话是「避免继续绕圈子」），
        // 而上一轮之所以乱，是命令闸门看不穿 `git -C` 把它连拦 7 次
        // （见 `mcp/guard.rs` 的 `normalize_for_whitelist`）。
        //
        // 结论：**治源头，别往提示词里堆规则。** 加一句没量出收益的话，
        // 代价是后面每个人都得绕过它去理解真正起作用的是哪几条。
        let mut header = format!(
            "# Current Workspace Environment

- **Workspace Absolute Path**: `{ws}`
- **Act directly on simple requests.** If the request maps to a single command or a single edit, just do it — do not plan, survey, or explain first.
- **`.anima/` is Anima's own state, not the user's code.** `.anima/worktrees/` holds internal sandbox checkouts. NEVER inspect, report, or reason about them when the user asks about their project's status, changes, or files — their contents are irrelevant to the user's work.
- **Recommended Tooling Guidelines**:
  - Use `shell_command` to run commands (git, npm, build, test).
  - Use `fs_list_directory` to inspect directory hierarchies (supports `.` relative paths, ignores node_modules/target by default).
  - Use `view_file` (with line slicing) or `code_outline` to read file structures and symbols.
  - Use `lsp_find_definition` / `lsp_find_references` for precise cross-file navigation.
  - Use `fetch_url` to read anything on the web — docs, a GitHub file, an API reference. **NEVER `git clone`/`curl`/`wget` through a shell just to read something**: that leaves files on disk nobody cleans up, and network hiccups turn into retry loops.
  - Use `generate_image` when the user requests generating images, app logos, icons, UI concept sketches, or illustrations. **Prompt language follows the engine.** The default engine is Qwen-Image 2.1, whose Qwen3-VL encoder reads Chinese natively and takes long prompts (Z-Image and Krea 2 read Chinese too) — when the user writes in Chinese, keep it in Chinese; long, detailed prompts are fine. Only SDXL needs English: its CLIP encoder does not understand Chinese and silently mangles it (measured: same seed, a Chinese prompt lost the hairstyle, clothing, background and framing). The tool falls back to SDXL automatically for `pose_image`, `layout: character_sheet` and SDXL LoRAs — for those calls write an English prompt under ~150 words. `generate_video` and `animate_image` run MiniMax H3 (Qwen3-VL encoder), so Chinese prompts and in-scene dialogue work there natively too. **When the user asks for a specific body pose**, pass `pose_image` (a reference photo) rather than trying to describe the pose in words — SDXL largely ignores written pose descriptions (measured: a fully explicit tree-pose prompt produced handstands, forward folds and lunges across 4 seeds). ControlNet copies only the skeleton, so appearance and background still come from `prompt`, and it also prevents extra limbs. **LoRA adapters**: pass `loras: [{{\"name\": \"...\", \"strength\": 0.8}}]` to steer style, faces, or detail. The name may be a fragment of the filename; if it does not match, the error lists every LoRA installed on this machine — read that list and retry with an exact name. `generate_video` and `animate_image` take the same parameter. **To refine an image you already generated** (change a colour, adjust a detail), call `generate_image` again with the *edited prompt* AND the **exact `seed` reported in that image's result** — a new random seed produces a completely different subject, not a refinement. **Always present the image directly using standard Markdown `![description](file:///absolute/path/to/image.png)` in your reply** (do not shell out to inspect or read the binary image).
  - Use `generate_video` for text-to-video and `animate_image` to animate an existing image (it needs `image_path` — generate the still first if there is none). Describe motion, camera movement, and lighting, not just the subject. **These are slow: roughly 4-5 minutes of wall time per second of 480p H3 video on this machine (measured 4.3; 720p is 11-14).** Keep `duration_secs` at its default of 2 unless the user asked for a longer clip, and never re-issue a call that reported a timeout — the job is still running in ComfyUI and a second one only queues behind it. **Putting an existing character into a NEW scene**: do NOT animate the portrait directly — `animate_image`'s default `first_frame` mode makes that still the literal opening frame, so frame 0 is the portrait and frame 1 jumps to the new scene. Pass `mode: \"reference\"` instead: the image becomes an identity reference and the scene is generated freely, with no jarring first frame. For identity use each character's single-person main image, not a whole multi-view character sheet — on a sheet each face is only ~40 px and H3 cannot lock identity from it. **Present the finished video the same way you present an image**: a standard Markdown embed `![description](file:///absolute/path/to/video.mp4)`. Writing the path as bare prose (\"saved to /path/x.mp4\") leaves the reader with no player. Never shell out to open or play it.
  - Use `apply_patch` to make file edits.
- **Subagent Topology & Sandbox Isolation**:
  - Deterministic or single-file edits should be executed directly in the current workspace.
  - Call `invoke_subagent(mode: \"explore\")` for read-only probing and broad architectural research to keep the main conversation clean.
  - Call `invoke_subagent(mode: \"isolated\")` when running parallel multi-file changes or high-risk refactoring experiments; the runtime will automatically provision an isolated Git Worktree sandbox.
- **Never narrate a tool call instead of making one**: lines of the form `〔已调用工具: … 参数: … 返回: …〕` in the history are a **system-generated record of past calls** — they are not something you write. Never emit that shape, and never emit its elision marker `…(共 N 字符)`. If you need a tool, emit a real tool call; if you cannot, say so plainly. Claiming work you did not do (\"image generated successfully\") is worse than reporting failure.
- **Paths & Formatting**:
  - Use relative paths (e.g. `.`, `src/main.rs`); paths are automatically resolved relative to the workspace root.
  - All code snippets or command blocks MUST be wrapped in standard markdown fences (```lang ... ```). NEVER output raw unescaped code.
  - Keep summaries concise and avoid dumping unmodified file contents into the chat stream.
- **Command Execution Context**:
  - `shell_command` and `exec_command` execute directly in the workspace root. DO NOT prepend `cd <path> &&` or git `-C <path>`.
  - To execute in a subdirectory, pass the relative path via the `workdir` parameter.
  - **Generated images are already shown inline in the chat.** NEVER call `open_url_or_file` on them — that pops a system preview window over the user's work, which is not what they asked for. Just mention the image in your reply.
  - **To play audio, always call `play_audio` with the file path.** NEVER shell out to `afplay`/`mpv`/`ffplay` through ANY command tool — including MCP ones such as `system__run_command`. Those are blocked and do not drive the player drawer.
  - **When you download, clone, or write anything outside the workspace, say so.** State the absolute path in your reply, and say whether you will clean it up or leave it. NEVER let the user wonder where their data went or whether it will disappear.
- **Multi-step Task Ledger (`task_plan`)**:
  - For tasks requiring more than 3 steps, step 1 MUST ALWAYS be `task_plan` (specifying goal and verifiable steps).
  - **NEVER call any other tools prior to invoking `task_plan`.**
  - Ensure each step is independently verifiable. Progress is tracked automatically by the runtime.
- **Language & Communication**:
  - Always communicate and respond to the user in their preferred language (e.g. Chinese).
  - Preserve exact names when citing symbols, code elements, or filesystem paths."
        );
        if let Some(instructions) = crate::workspace::project_instructions(ws) {
            header.push_str("\n\n---\n\n");
            header.push_str(&instructions);
        }

        // 长期记忆召回。
        //
        // **过滤成「全局 + 本项目」**：别的项目的工程约定跑到这里就是噪声，
        // 而它挤掉的是当前项目真正相关的那几条。相似度下限（0.65）负责
        // 挡住不相关的全局事实 —— bge-m3 上实测相关答案落在 0.699~0.721、
        // 无关答案最高 0.603，所以「喜欢冰镇西瓜汁」不会因为一个 coding 问题被召回。
        //
        // 这一步会加一次嵌入调用 + 一次向量检索的延迟（本机 ~50ms）。
        // 相对 coding 一轮动辄几十秒到几百秒，可以忽略；放在语音链路里
        // 才需要斟酌（那边的注释里记了这一点）。
        // 从 req 里取而不是 args —— args.messages 已经移进 ChatRequest
        if let Some(last_user) = req
            .messages
            .iter()
            .rev()
            .find(|m| m.role == crate::llm::Role::User)
            .map(|m| m.content.clone())
        {
            let memory = { state.memory.read().await.clone() };
            if let Some(memory) = memory {
                // scope 走 memory.scope_for：注册过的项目用它的 id，
                // 这样改名挪位置都不影响已累积的记忆
                let scope = memory.scope_for(ws).await;
                let hits = memory
                    .recall(
                        &last_user,
                        &crate::memory::vector::ScopeFilter::Project(scope),
                    )
                    .await;
                // **项目里用另一套措辞。** 陪伴场景那句「可在需要时自然地运用」
                // 会让模型把一条召回的旧记录和一份刚读过的文件当成同等权重，
                // 而关于代码的事实每次 commit 都可能失效。
                if let Some(block) = crate::memory::Memory::format_recalled_for(&hits, true) {
                    header.push_str("\n\n---\n\n");
                    header.push_str(&block);
                }
            }
        }

        req.messages.insert(0, Message::system(header));
    }

    // 注入工具。MCP 已经 runtime 化：模型看到的 MCP tool 名仍兼容
    // `server__tool`，但执行路径会走 RuntimeService 的审批、事件与重试。
    if args
        .workspace_path
        .as_deref()
        .is_some_and(|p| !p.trim().is_empty())
    {
        // 项目代码工作区：严格注入 RuntimeService 规范内建工具集与过滤后的 MCP 工具
        req.tools = state
            .runtime
            .tool_declarations_with_mcp_for(
                state.mcp.clone(),
                crate::agent_runtime::ToolProfile::Coding,
            )
            .await;
    } else {
        // 非项目模式：注入基础 MCP 工具 + 运行时基础工具（含 generate_image、fetch_url）+ 闲聊内置工具
        req.tools = state
            .runtime
            .tool_declarations_with_mcp_for(
                state.mcp.clone(),
                crate::agent_runtime::ToolProfile::Companion,
            )
            .await;
        req.tools.extend(crate::system_tools::declarations());
    }

    // ── 工具分层：常驻的给完整 schema，其余的先藏起来 ──
    //
    // 60 个工具的完整 schema 是 58314 字符 / 约 23000 token，而本机模型的
    // 窗口只有 33024 —— **一句话没说就用掉七成**。实测 LM Studio 直接报
    // `request (47079 tokens) exceeds the available context size (33024)`，
    // 而这边只看到一个 HTML 的 500。
    //
    // 藏起来的那些留在 `tool_search` 的描述里，模型想用时先查、拿到 schema、
    // 之后就解锁（解锁是从会话历史里认的，见 `tool_tier`）。
    // 分层之前的全集留一份：技能白名单里点了名的工具要能从这里找回来（见下面「应用 persona 技能」）
    let tools_before_tier = req.tools.clone();
    {
        use crate::agent_runtime::tool_tier;
        let haystack: String = req
            .messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let unlocked =
            tool_tier::unlocked_from_history(&haystack, req.tools.iter().map(|t| t.name.as_str()));
        let before = req.tools.len();
        let hidden: Vec<String> = req
            .tools
            .iter()
            .filter(|t| !tool_tier::is_always_on(&t.name) && !unlocked.contains(&t.name))
            .map(|t| format!("{}（{}）", t.name, first_sentence(&t.description)))
            .collect();
        req.tools
            .retain(|t| tool_tier::is_always_on(&t.name) || unlocked.contains(&t.name));

        // **藏起来的必须留下名字**，否则模型不知道它们存在，也就不会去查。
        if !hidden.is_empty() {
            if let Some(search) = req.tools.iter_mut().find(|t| t.name == "tool_search") {
                search.description.push_str(
                    "\n\n当前未展开参数的工具（要用先在这里查，查到之后即可调用）：\n",
                );
                search.description.push_str(&hidden.join("\n"));
            }
        }
        tracing::debug!(before, after = req.tools.len(), unlocked = unlocked.len(), "工具分层");
    }

    // ── 应用 persona 技能 ──
    //
    // 在此之前 coding 链路**完全不用技能**：`allows_tool` 全代码库只在
    // `voice/mod.rs` 被调过一次。于是一份「只读的审查角色」在设置面板里能勾、
    // 能显示、能算白名单，而在 start_chat 里没人问它 —— 收窄工具面这件事
    // 只在语音链路成立。
    //
    // **不覆盖 tier 和 thinking。** 技能的 `prefer_tier` / `thinking` 在语音链路
    // 里是有意义的（那边没有别的入口），但这里输入框上就有模型选择器和 Effort
    // 滑杆 —— 用户刚显式选了后台档，技能把它改成实时档会非常意外，而且
    // 界面上还显示着用户选的那个。所以这里只取两样：提示词 + 工具白名单。
    if let Some(ws) = args
        .workspace_path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        let wants = state.gateway.config().read().await.active_skills.clone();
        let mut applied: Vec<crate::skill::Skill> = Vec::new();
        for id in wants {
            match crate::skill::resolve_layered(
                &state.config_dir,
                Some(std::path::Path::new(ws)),
                &id,
            ) {
                Ok(skill) => {
                    // 只有声明了在 coding 链路可用的 persona 才作数。
                    // 家庭医生进了代码会话是纯噪声，而任务型技能是按次触发的
                    // （`run_skill` / `/`），不该常驻。
                    if skill.surface.available_in_coding()
                        && skill.kind == crate::skill::SkillKind::Persona
                    {
                        applied.push(skill);
                    }
                }
                Err(err) => tracing::warn!(%err, "配置里的某个技能加载失败"),
            }
        }

        if !applied.is_empty() {
            // 提示词拼到那条 system 消息上（它一定在 0 号位，上面刚插的）。
            // **不新开一条 system 消息** —— 多条 system 会让某些本地模型的
            // 模板直接 500（见 gateway::merge_system_messages 的注释）
            if let Some(sys) = req
                .messages
                .first_mut()
                .filter(|m| m.role == crate::llm::Role::System)
            {
                for skill in &applied {
                    sys.content.push_str("\n\n---\n\n");
                    sys.content.push_str(&skill.prompt);
                }
            }

            let (kept, wiped) =
                crate::skill::narrow_tools(std::mem::take(&mut req.tools), &applied);
            req.tools = kept;
            // **技能点了名的工具直接给完整 schema。** 白名单在分层之后才过滤，
            // 以前「Studio 制作统筹」开着时 studio_* 仍被藏在 tool_search 后面，每次都要先查一轮。
            crate::skill::restore_named_tools(&mut req.tools, &tools_before_tier, &applied);
            if wiped {
                // narrow_tools 已经 warn 过原因了。这里补一句后果：
                // 这一轮模型没有任何工具，它会开始凭想象描述文件内容
                tracing::warn!(
                    skills = ?applied.iter().map(|s| &s.id).collect::<Vec<_>>(),
                    "本轮模型没有任何工具可用"
                );
            }
            tracing::info!(
                skills = ?applied.iter().map(|s| &s.id).collect::<Vec<_>>(),
                tools = req.tools.len(),
                "coding 链路已应用技能"
            );
        }

        // ── 声明 run_skill ──
        //
        // 任务型技能**不常驻**，所以不在上面那个 applied 里。它们通过这个工具
        // 按次载入：用户说「审查一下 lru.ts」，模型自己调 run_skill，
        // 工具结果就是那份规程。
        //
        // **声明在这里而不是在工具的 spec() 里**，因为可用列表取决于当前工作区
        // （项目技能在 `.anima/skills/`），而 `spec(&self)` 拿不到工作区。
        // switch_skill 也是这个做法。
        //
        // 放在白名单过滤**之后**：一个 persona 若显式收窄了工具面，
        // 不该被 run_skill 绕开 —— 但也不该因此让用户失去任务技能，
        // 所以只有在没有 persona 收窄时才加。
        let all = crate::skill::list_layered(&state.config_dir, Some(std::path::Path::new(ws)));
        let tasks = crate::skill::tasks_for(&all, true);
        if !tasks.is_empty() && !req.tools.is_empty() {
            req.tools.push(crate::skill::run_declaration(&tasks));
            tracing::debug!(
                tasks = ?tasks.iter().map(|s| &s.id).collect::<Vec<_>>(),
                "已声明 run_skill"
            );
        }
    }

    // ── 任务台账 ──
    //
    // 声明 `task_plan`，并把台账注入进**当前那条 user 消息的开头**。
    //
    // **不放 system header。** 台账每轮都变，而 system 在最前面 —— 一改，
    // 从第 0 个 token 起全部 cache miss。放当前 user 消息的代价是精确可算的：
    // 前缀在上一条 user 消息处分叉，要重新 prefill 的是「上一条 user +
    // 上一条 assistant + 这一条 user」，而后两个本来就是新内容 ——
    // 多付的只有一条 user 消息。放 system 则要重付全部。
    //
    // 另一个方案是把台账也存进历史（前缀完全稳定），否掉：历史里会累积好几份
    // 互相矛盾的进度快照，那比多付一条消息的 prefill 严重得多。
    let ledger_key =
        crate::task_ledger::key_for(args.session_id.as_deref(), args.workspace_path.as_deref());
    if let Some(key) = &ledger_key {
        // 一个工具都没有时不声明它 —— 那种场景（纯聊天）没有多步任务
        if !req.tools.is_empty() {
            req.tools.push(crate::task_ledger::declaration());
        }
        if let Some(block) = crate::task_ledger::load(&state.config_dir, key).render() {
            // 只注入进**当前**这一轮。历史里绝不能有旧台账，否则上下文里会有
            // 好几份互相矛盾的进度快照。这一条天然成立：`args.messages` 是前端
            // 每轮从自己干净的 turns[] 重新组装的，注入只对这一次请求生效。
            crate::task_ledger::inject(&mut req.messages, &block);
        }
    }

    // 探一次路由，好把实际后端告诉前端（可能因不支持工具而降级到云端）。
    // 真正的流由 agent::run 自己建，因为它要跑多轮
    let provider = state.gateway.resolve(tier, !req.tools.is_empty()).await?;
    let handle = ChatHandle {
        stream_id: stream_id.clone(),
        provider_id: provider.id.clone(),
        model: provider.model.clone(),
    };

    let deps = crate::agent::LoopDeps {
        gateway: state.gateway.clone(),
        runtime: state.runtime.clone(),
        ledger_key: crate::task_ledger::key_for(
            args.session_id.as_deref(),
            args.workspace_path.as_deref(),
        ),
        workspace_path: args.workspace_path,
        memory: state.memory.clone(),
        mcp: state.mcp.clone(),
        http: state.http.clone(),
        music_player: state.music_player.clone(),
        turn_cancels: state.turn_cancels.clone(),
        config_dir: state.config_dir.clone(),
    };
    tauri::async_runtime::spawn(crate::agent::run(app, deps, stream_id, tier, req));

    Ok(handle)
}

fn safe_attachment_stem(filename: &str) -> String {
    let name = Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image");
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_alphanumeric() || matches!(ch, '-' | '_') {
            out.push(ch);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let out = out.trim_matches('_');
    if out.is_empty() {
        "image".to_string()
    } else {
        out.chars().take(48).collect()
    }
}

fn image_ext_from_mime(mime: &str) -> Option<&'static str> {
    match mime {
        "image/png" => Some("png"),
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        "image/bmp" => Some("bmp"),
        _ => None,
    }
}

fn chat_upload_dir(config_dir: &Path, workspace_path: Option<&str>) -> PathBuf {
    if let Some(ws) = workspace_path
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        return ws.join(".anima/uploads/chat");
    }
    config_dir.join("anima/uploads/chat")
}

/// 保存 Chat 输入框上传/粘贴的图片，返回可交给 `animate_image.image_path` 的 file URL。
///
/// 多模态模型吃的是 base64，但 ComfyUI I2V 工具吃的是本地路径。之前这两条链路
/// 没有桥接：模型能看图，却不能把同一张图传给 `animate_image`。
#[tauri::command]
pub async fn save_chat_image_attachment(
    state: State<'_, AppState>,
    args: SaveChatImageAttachmentArgs,
) -> Result<SavedChatImageAttachment> {
    use base64::Engine;

    let Some(rest) = args.data_url.strip_prefix("data:") else {
        return Err(Error::config("图片附件必须是 data:image/*;base64,... 格式"));
    };
    let Some((mime, b64)) = rest.split_once(";base64,") else {
        return Err(Error::config("图片附件缺少 base64 数据"));
    };
    let Some(ext) = image_ext_from_mime(mime) else {
        return Err(Error::config(format!("不支持的图片类型：{mime}")));
    };

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| Error::config(format!("解析图片附件失败：{e}")))?;
    if bytes.is_empty() {
        return Err(Error::config("图片附件为空"));
    }

    let dir = chat_upload_dir(&state.config_dir, args.workspace_path.as_deref());
    std::fs::create_dir_all(&dir)
        .map_err(|e| Error::config(format!("创建图片上传目录失败 {}: {e}", dir.display())))?;

    let stem = safe_attachment_stem(&args.filename);
    let path = dir.join(format!("{}_{}.{}", stem, uuid::Uuid::new_v4(), ext));
    std::fs::write(&path, bytes)
        .map_err(|e| Error::config(format!("保存图片附件失败 {}: {e}", path.display())))?;

    let path = path
        .canonicalize()
        .map_err(|e| Error::config(format!("解析图片附件路径失败 {}: {e}", path.display())))?;
    let path_str = path.to_string_lossy().to_string();
    Ok(SavedChatImageAttachment {
        file_url: format!("file://{path_str}"),
        path: path_str,
    })
}

#[derive(Debug, Deserialize)]
pub struct SaveGenerationLogArgs { pub output_path: String, pub mode: String, pub details: String }

/// 为浏览器侧渲染的媒体补齐同名生成日志。
#[tauri::command]
pub fn save_generation_log(state: State<'_, AppState>, args: SaveGenerationLogArgs) -> Result<String> {
    let output = PathBuf::from(&args.output_path);
    if !output.is_absolute() || !output.exists() { return Err(Error::config("日志对应的输出文件不存在")); }
    let output = output.canonicalize().map_err(|e| Error::config(format!("解析输出路径失败：{e}")))?;
    let root = state.config_dir.canonicalize().map_err(|e| Error::config(format!("解析 Anima 数据目录失败：{e}")))?;
    if !output.starts_with(&root) { return Err(Error::config("只能为 Anima 数据目录里的生成产物写日志")); }
    let log = PathBuf::from(format!("{}.generation.md", output.display()));
    let body = format!("# Anima 生成日志\n\n- 状态：成功\n- 模式：{}\n- 输出文件：{}\n- 时间（Unix秒）：{}\n\n## 提示词 / 生成依据\n\n此模式不使用文本提示词；画面由导演世界的3D场景、人物站位和相机关键帧确定。\n\n## 完整参数与关联关系\n\n```json\n{}\n```\n", args.mode, output.display(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs(), args.details);
    std::fs::write(&log, body).map_err(|e| Error::config(format!("写生成日志失败：{e}")))?;
    Ok(log.to_string_lossy().into_owned())
}

#[derive(Debug, Deserialize)]
pub struct LoadDroppedAttachmentArgs {
    pub path: String,
    #[serde(default)]
    pub workspace_path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DroppedAttachmentInfo {
    pub name: String,
    pub size: u64,
    pub is_image: bool,
    pub data_url: Option<String>,
    pub local_path: Option<String>,
    pub file_url: Option<String>,
    pub text_content: Option<String>,
    pub extension: String,
}

/// 读取并处理从系统桌面/访达拖拽进来的文件
#[tauri::command]
pub async fn load_dropped_attachment(
    state: State<'_, AppState>,
    args: LoadDroppedAttachmentArgs,
) -> Result<DroppedAttachmentInfo> {
    use base64::Engine;

    let clean = args.path.trim_start_matches("file://");
    let path = PathBuf::from(clean);
    if !path.is_file() {
        return Err(Error::config(format!("找不到拖拽的文件：{}", path.display())));
    }

    let meta = std::fs::metadata(&path)
        .map_err(|e| Error::config(format!("无法读取文件信息：{e}")))?;
    let size = meta.len();
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    let is_img = matches!(
        ext.as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "svg"
    );

    if is_img {
        let dir = chat_upload_dir(&state.config_dir, args.workspace_path.as_deref());
        let _ = std::fs::create_dir_all(&dir);

        let target_path = dir.join(format!("{}_{}.{}", safe_attachment_stem(&name), uuid::Uuid::new_v4(), ext));
        let _ = std::fs::copy(&path, &target_path);

        let final_path = if target_path.is_file() { target_path } else { path.clone() };
        let final_canonical = final_path.canonicalize().unwrap_or(final_path);
        let final_str = final_canonical.to_string_lossy().to_string();

        let bytes = std::fs::read(&final_canonical)
            .map_err(|e| Error::config(format!("读取图片数据失败：{e}")))?;

        // 转为通用标准 JPEG base64，确保各大多模态大模型及本地 LM Studio / Ollama 端点 100% 兼容
        let data_url = if let Ok(dyn_img) = image::load_from_memory(&bytes) {
            let mut jpeg_buf = std::io::Cursor::new(Vec::new());
            if dyn_img.write_to(&mut jpeg_buf, image::ImageFormat::Jpeg).is_ok() {
                let b64 = base64::engine::general_purpose::STANDARD.encode(jpeg_buf.into_inner());
                format!("data:image/jpeg;base64,{b64}")
            } else {
                let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                format!("data:image/jpeg;base64,{b64}")
            }
        } else {
            let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
            format!("data:image/jpeg;base64,{b64}")
        };

        Ok(DroppedAttachmentInfo {
            name,
            size,
            is_image: true,
            data_url: Some(data_url),
            local_path: Some(final_str.clone()),
            file_url: Some(format!("file://{final_str}")),
            text_content: None,
            extension: ext,
        })
    } else {
        let text_content = if size <= 1_000_000 {
            std::fs::read_to_string(&path).ok()
        } else {
            None
        };

        let path_canonical = path.canonicalize().unwrap_or(path);
        let path_str = path_canonical.to_string_lossy().to_string();

        Ok(DroppedAttachmentInfo {
            name,
            size,
            is_image: false,
            data_url: None,
            local_path: Some(path_str.clone()),
            file_url: Some(format!("file://{path_str}")),
            text_content,
            extension: ext,
        })
    }
}

/// 读当前会话的任务台账。给界面上那张常驻卡片用。
///
/// **和 `AgentActivityTrajectory` 是两种东西**：轨迹记录微观物理操作
/// （哪次 RPC 干了什么），台账记录宏观里程碑。混在一起、或者把台账塞进
/// 可折叠列表里，这个功能就白做了 —— 用户和 agent 建立不了共识。
#[tauri::command]
pub async fn task_ledger_get(
    state: State<'_, AppState>,
    session_id: Option<String>,
    workspace_path: Option<String>,
) -> Result<Option<crate::task_ledger::TaskLedger>> {
    let Some(key) = crate::task_ledger::key_for(session_id.as_deref(), workspace_path.as_deref())
    else {
        return Ok(None);
    };
    let l = crate::task_ledger::load(&state.config_dir, &key);
    Ok(if l.is_empty() { None } else { Some(l) })
}

/// 归档台账：停止注入，文件留着。
///
/// **只有用户能触发。** 不做自动关闭 —— 每一种启发式都朝着「破坏这个功能」
/// 的方向失效：「全部步骤 done 就关」（没有 status，而且模型会把 failed 改成
/// done 脱身）、「N 轮没有工具调用就关」（任务中间讨论几轮方案很正常）、
/// 「隔一天算废弃」（一个任务隔一天接着做很正常）。
///
/// 代价是任务做完后台账会作为噪声多留几轮（约 600 字符）。这比「该在的时候
/// 不在」小得多，而且卡片就在眼前、关掉是一次点击。
#[tauri::command]
pub async fn task_ledger_archive(
    state: State<'_, AppState>,
    session_id: Option<String>,
    workspace_path: Option<String>,
) -> Result<()> {
    let Some(key) = crate::task_ledger::key_for(session_id.as_deref(), workspace_path.as_deref())
    else {
        return Ok(());
    };
    let mut l = crate::task_ledger::load(&state.config_dir, &key);
    if l.is_empty() {
        return Ok(());
    }
    l.archived = true;
    crate::task_ledger::save(&state.config_dir, &key, &l)
}

/// 取消一轮对话。语音打断（barge-in）时由 VAD 触发调用。
#[tauri::command]
pub async fn cancel_chat(state: State<'_, AppState>, stream_id: String) -> Result<bool> {
    // **两个都要取消。** Gateway 的令牌只管"正在吐 token 的那条流"，流一结束
    // 就被摘掉；而工具执行发生在流结束之后，最多还要跑 60 秒。
    // 只取消前者的话，用户点了停止，工具照样跑完、结果照样回填给模型、
    // 然后还会再开一轮 —— 表现就是"点了停止但它还在动"。
    let stream = state.gateway.cancel(&stream_id).await;
    let turn = state.turn_cancels.cancel(&stream_id).await;
    Ok(stream || turn)
}

/// 这份构建是什么。
///
/// **「我在跑哪一版」必须能答得出来** —— 用户报 bug 时说的「0.9.0」，
/// 如果是带脏工作区编出来的开发构建，和发布的 0.9.0 不是一个东西。
/// 所以除了版本号，还带 git sha 和构建时间，脏工作区会在 sha 后面加 `-dirty`。
#[derive(Debug, Clone, serde::Serialize)]
pub struct BuildInfo {
    /// 来自 `CARGO_PKG_VERSION`。三处版本一致由 `src/version.test.ts` 守着
    pub version: String,
    /// `abc1234` 或 `abc1234-dirty`；取不到是 `unknown`
    pub git_sha: String,
    pub built_at: String,
    /// debug 构建。界面上要和正式版区分开
    pub debug: bool,
}

#[tauri::command]
pub fn build_info() -> BuildInfo {
    BuildInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        git_sha: env!("ANIMA_GIT_SHA").to_string(),
        built_at: env!("ANIMA_BUILD_DATE").to_string(),
        debug: cfg!(debug_assertions),
    }
}

/// 读取配置。API Key 不在返回值里，只有 `has_key` 布尔位。
#[tauri::command]
pub async fn get_config(state: State<'_, AppState>) -> Result<ConfigView> {
    let config = state.gateway.config();
    let config = config.read().await;

    Ok(ConfigView {
        providers: config.providers.iter().map(ProviderStatus::of).collect(),
        realtime_provider: config.realtime_provider.clone(),
        background_provider: config.background_provider.clone(),
        tool_fallback_provider: config.tool_fallback_provider.clone(),
        realtime_model: config.realtime_model.clone(),
        background_model: config.background_model.clone(),
        tool_fallback_model: config.tool_fallback_model.clone(),
        startup_window: config.startup_window.clone(),
        theme_mode: config.theme_mode.clone(),
        language: config.language.clone(),
        sheet_loras: config.sheet_loras.clone(),
        zimage_loras: config.zimage_loras.clone(),
        image_engine: config.image_engine.clone(),
        sdxl_checkpoint: config.sdxl_checkpoint.clone(),
        h3_native_ref2va: config.h3_native_ref2va,
        h3_ref2va_installed: crate::agent_runtime::weetodd::h3_ref2va_native_installed(),
        disable_thinking: config.disable_thinking,
        thinking: config.thinking.clone(),
    })
}

#[derive(Debug, Serialize)]
pub struct ConfigView {
    pub providers: Vec<ProviderStatus>,
    pub realtime_provider: String,
    pub background_provider: String,
    /// 工具调用的备用后端。None = 自动挑（优先配了密钥的云端）
    pub tool_fallback_provider: Option<String>,
    /// 每条链路各自的模型。None = 沿用该后端的默认模型
    pub realtime_model: Option<String>,
    pub background_model: Option<String>,
    pub tool_fallback_model: Option<String>,
    /// 启动时默认显示的窗口："avatar" | "chat"
    pub startup_window: String,
    pub theme_mode: String,
    /// 界面语言
    pub language: Option<String>,
    /// 出图默认底模：`zimage` / `chroma` / `sdxl` / `flux`
    ///
    /// **加字段时别忘了这里。** 这个结构是前端**唯一**能读到配置的通道 ——
    /// `AppConfig` 里加了、`ConfigView` 里没加，结果是「写得进去、读不出来」：
    /// 点击确实把值落进了 config.json，但界面读回来永远是 `undefined`，
    /// 高亮停在默认项，看起来完全像点了没反应。实测撞到过。
    pub image_engine: String,
    /// SDXL 那条路用哪个 checkpoint。`None` = 自动挑
    pub sdxl_checkpoint: Option<String>,
    /// H3 参考生视频用原生 Ref2VA 权重吗（默认关 = 兼容模式）
    pub h3_native_ref2va: bool,
    /// 本机装没装原生 Ref2VA 权重。没装时开关不起作用，界面上要说清楚
    pub h3_ref2va_installed: bool,
    /// 设定卡专用 LoRA
    pub sheet_loras: Vec<crate::config::SheetLora>,
    pub zimage_loras: Vec<crate::config::SheetLora>,
    /// **已废弃**，只为不打断还没改完的前端保留。用 `thinking` 代替。
    pub disable_thinking: bool,
    /// 每条链路的思考档位
    pub thinking: crate::config::ThinkingPolicy,
}

/// 设置启动时默认展示的窗口。
///
/// **认得的值必须和 `windows::PANELS` 里的 label 对得上** ——
/// 对不上的话启动时那句 `windows::open` 会静默失败，
/// 表现是「选了但启动后什么都没开」。[`normalize_startup_window`] 有测试钉住。
#[tauri::command]
pub async fn set_startup_window(state: State<'_, AppState>, window: String) -> Result<ConfigView> {
    let w = normalize_startup_window(&window);
    {
        let config = state.gateway.config();
        let mut config = config.write().await;
        config.startup_window = w;
        config.save(&state.config_path)?;
    }
    get_config(state).await
}

/// 设置界面主题。`"system"` / `"light"` / `"dark"`，不认识的回 `system`。
#[tauri::command]
pub async fn set_theme_mode(state: State<'_, AppState>, mode: String) -> Result<ConfigView> {
    let m = normalize_theme_mode(&mode);
    {
        let config = state.gateway.config();
        let mut config = config.write().await;
        config.theme_mode = m;
        config.save(&state.config_path)?;
    }
    get_config(state).await
}

/// 认得的主题。
///
/// **不认识的回 `system`，不回 `light`** —— 回 light 的话，
/// 配置文件被手改坏之后，深色用户开机会被闪一脸白。
pub fn normalize_theme_mode(raw: &str) -> String {
    match raw.trim().to_lowercase().as_str() {
        "light" => "light",
        "dark" => "dark",
        _ => "system",
    }
    .to_string()
}

/// 认得的启动窗口。不认识的一律回 `avatar`（默认桌宠形态）。
pub fn normalize_startup_window(raw: &str) -> String {
    match raw.trim().to_lowercase().as_str() {
        "chat" => "chat",
        "anima_studio" => "anima_studio",
        _ => "avatar",
    }
    .to_string()
}

/// 列出 ComfyUI 里可选的 SDXL checkpoint。
///
/// 直接问 ComfyUI 而不是扫目录 —— `extra_model_paths.yaml` 可以把模型放在
/// 任意位置，扫目录会漏掉，而 `/object_info` 报的就是它真正认得的那一份。
/// 不猜文件名是这个项目里反复吃过亏的一条。
#[tauri::command]
pub async fn list_sdxl_checkpoints() -> Result<Vec<String>> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .unwrap_or_default();
    let url = "http://127.0.0.1:8188/object_info/CheckpointLoaderSimple";
    let Ok(resp) = client.get(url).send().await else {
        return Ok(Vec::new());
    };
    let Ok(json) = resp.json::<serde_json::Value>().await else {
        return Ok(Vec::new());
    };
    Ok(json
        .pointer("/CheckpointLoaderSimple/input/required/ckpt_name/0")
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default())
}

/// 指定 SDXL 那条路用哪个 checkpoint。空字符串 = 恢复自动挑选。
///
/// **不校验文件是否存在。** 用户可能先选好、之后再把模型放进去；
/// 真正的兜底在 `find_best_checkpoint` —— 提交前对不上就退回自动挑，
/// 而不是让 ComfyUI 拒掉整个任务。
#[tauri::command]
pub async fn set_sdxl_checkpoint(
    state: State<'_, AppState>,
    checkpoint: String,
) -> Result<ConfigView> {
    let c = checkpoint.trim();
    let normalized = if c.is_empty() { None } else { Some(c.to_string()) };
    {
        let config = state.gateway.config();
        let mut config = config.write().await;
        config.sdxl_checkpoint = normalized;
        config.save(&state.config_path)?;
    }
    get_config(state).await
}

/// H3 参考生视频 / 长镜头用原生 Ref2VA 还是兼容模式。默认兼容（见 `AppConfig::h3_native_ref2va`）。
/// 下一次提交出片才生效，已经在 ComfyUI 里跑的不受影响。
#[tauri::command]
pub async fn set_h3_native_ref2va(state: State<'_, AppState>, enabled: bool) -> Result<ConfigView> {
    {
        let config = state.gateway.config();
        let mut config = config.write().await;
        config.h3_native_ref2va = enabled;
        config.save(&state.config_path)?;
    }
    get_config(state).await
}

/// 列出 ComfyUI 里可选的 LoRA。
#[tauri::command]
pub async fn list_loras() -> Result<Vec<String>> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .unwrap_or_default();
    let url = "http://127.0.0.1:8188/object_info/LoraLoader";
    let Ok(resp) = client.get(url).send().await else {
        return Ok(Vec::new());
    };
    let Ok(json) = resp.json::<serde_json::Value>().await else {
        return Ok(Vec::new());
    };
    Ok(json
        .pointer("/LoraLoader/input/required/lora_name/0")
        .and_then(|v| v.as_array())
        .map(|l| l.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default())
}

/// 设定卡专用 LoRA。传空数组 = 清空。
///
/// **只影响角色设定卡那条路。** 刻意不做全局常挂 —— LoRA 只在 SDXL 架构上
/// 有效，而默认引擎是 Z-Image，全局设置会在那条路上静默失效。
#[tauri::command]
pub async fn set_sheet_loras(
    state: State<'_, AppState>,
    loras: Vec<crate::config::SheetLora>,
) -> Result<ConfigView> {
    {
        let config = state.gateway.config();
        let mut config = config.write().await;
        config.sheet_loras = loras
            .into_iter()
            .filter(|l| !l.name.trim().is_empty())
            .map(|l| crate::config::SheetLora {
                name: l.name.trim().to_string(),
                // 夹到合理区间：负权重和超过 2.0 都只会把图弄坏
                strength: l.strength.clamp(0.0, 2.0),
            })
            .collect();
        config.save(&state.config_path)?;
    }
    get_config(state).await
}

/// Z-Image 那条路常挂哪些 LoRA。
///
/// 和 [`set_sheet_loras`] 同形，但**不是同一批权重** —— 架构不同，
/// 混用会静默失效（SDXL LoRA 在 Z-Image 图上不生效且不报错，反之亦然）。
#[tauri::command]
pub async fn set_zimage_loras(
    state: State<'_, AppState>,
    loras: Vec<crate::config::SheetLora>,
) -> Result<ConfigView> {
    {
        let config = state.gateway.config();
        let mut config = config.write().await;
        config.zimage_loras = loras
            .into_iter()
            .filter(|l| !l.name.trim().is_empty())
            .map(|l| crate::config::SheetLora {
                name: l.name.trim().to_string(),
                strength: l.strength.clamp(0.0, 2.0),
            })
            .collect();
        config.save(&state.config_path)?;
    }
    get_config(state).await
}

/// 记住界面语言。
///
/// **必须落到 config.json，不能只留在前端的 localStorage。** macOS 托盘菜单是
/// Rust 在启动时建的，读不到 localStorage —— 不存这一份的话，每次启动托盘
/// 都会回到英文。
#[tauri::command]
pub async fn set_language(state: State<'_, AppState>, lang: String) -> Result<ConfigView> {
    let l = lang.trim();
    let normalized = if l.is_empty() { None } else { Some(l.to_string()) };
    {
        let config = state.gateway.config();
        let mut config = config.write().await;
        config.language = normalized;
        config.save(&state.config_path)?;
    }
    get_config(state).await
}

/// 设置出图默认底模。
///
/// 只认这几个值，别的一律回落 `zimage` —— 写进配置的字符串会被
/// `image_engine::ImageEngine::from_id` 解析，塞个不认识的值进去等于静默用默认，
/// 用户会以为自己选上了。
/// 设置里选的「出图底模」落成哪个引擎 id。认不出就回 `zimage`。
///
/// # 不要在这里再列一遍引擎名
///
/// 这里原本是一张手写白名单（`chroma` / `sdxl` / `krea2`，其余落 `zimage`）。
/// 加引擎时漏改这一处的表现是：设置里那张卡片点得下去、界面也高亮了，
/// **存进去的却是 Z-Image**，不报错 —— 2026-09-22 加 Qwen 2.1 时就撞上了。
///
/// 认不认得出交给 [`eng::ImageEngine::from_id`]（引擎名只有那一个所有者）。
/// 这里只额外挡一件事：**参考生图引擎不能当默认底模** ——
/// 它们必须有参考图才跑得动，设成默认等于让每一次普通出图都失败。
pub fn resolve_default_image_engine(engine: &str) -> &'static str {
    eng::ImageEngine::from_id(engine)
        .filter(|x| x.reference_slots() == 0)
        .map(|x| x.id())
        .unwrap_or("zimage")
}

#[tauri::command]
pub async fn set_image_engine(state: State<'_, AppState>, engine: String) -> Result<ConfigView> {
    let e = resolve_default_image_engine(&engine);
    {
        let config = state.gateway.config();
        let mut config = config.write().await;
        config.image_engine = e.to_string();
        config.save(&state.config_path)?;
    }
    get_config(state).await
}

/// 设置某条链路的思考档位。
///
/// **按链路而不是按模型**：同一个模型可以同时服务两个档，按模型记的话
/// 在后台档开了思考、切回实时档会跟着开。
///
/// 传进来的档位不做能力校验 —— 用户可能先设档位再换模型。
/// 「这个模型支不支持这一档」在发请求时由 `llm::thinking::resolve` 兜底，
/// 前端也会拿 `thinking_levels` 只渲染可用的那几格。
#[tauri::command]
pub async fn set_thinking_effort(
    state: State<'_, AppState>,
    tier: crate::config::Tier,
    effort: crate::llm::ThinkingEffort,
) -> Result<ConfigView> {
    {
        let config = state.gateway.config();
        let mut config = config.write().await;
        match tier {
            crate::config::Tier::Realtime => config.thinking.realtime = effort,
            crate::config::Tier::Background => config.thinking.background = effort,
        }
        config.save(&state.config_path)?;
    }
    get_config(state).await
}

#[derive(Debug, Deserialize)]
pub struct UpdateProviderArgs {
    pub id: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub supports_tools: Option<bool>,
    /// 仅 Azure：对话接口的 api-version
    #[serde(default)]
    pub api_version: Option<String>,
}

/// 改某个后端的连接参数。
#[tauri::command]
pub async fn update_provider(
    state: State<'_, AppState>,
    args: UpdateProviderArgs,
) -> Result<ConfigView> {
    {
        let config = state.gateway.config();
        let mut config = config.write().await;

        let provider = config
            .providers
            .iter_mut()
            .find(|p| p.id == args.id)
            .ok_or_else(|| Error::UnknownProvider(args.id.clone()))?;

        if let Some(url) = args.base_url {
            provider.base_url = url.trim().trim_end_matches('/').to_string();
        }
        if let Some(model) = args.model {
            provider.model = model.trim().to_string();
        }
        if let Some(flag) = args.supports_tools {
            provider.supports_tools = flag;
        }
        if let Some(version) = args.api_version {
            // 传空字符串表示"回退到适配器默认版本"
            let version = version.trim();
            provider.api_version = (!version.is_empty()).then(|| version.to_string());
        }

        config.save(&state.config_path)?;
    }

    // 连接参数变了，在途的流用的还是旧配置，一并取消避免行为不一致
    state.gateway.cancel_all().await;
    get_config(state).await
}

/// 切换某个分层使用的后端。
#[tauri::command]
pub async fn set_tier_provider(
    state: State<'_, AppState>,
    tier: Tier,
    provider_id: String,
) -> Result<ConfigView> {
    {
        let config = state.gateway.config();
        let mut config = config.write().await;

        // 先确认存在，避免写进一个悬空引用
        config.provider(&provider_id)?;

        match tier {
            Tier::Realtime => config.realtime_provider = provider_id,
            Tier::Background => config.background_provider = provider_id,
        }
        config.save(&state.config_path)?;
    }

    state.gateway.cancel_all().await;
    get_config(state).await
}

/// 写入 API Key（存 Keychain）。传空字符串则删除。
#[tauri::command]
pub async fn set_api_key(
    state: State<'_, AppState>,
    provider_id: String,
    key: String,
) -> Result<ConfigView> {
    {
        // 确认是已知后端，避免往 Keychain 里写垃圾条目
        let config = state.gateway.config();
        config.read().await.provider(&provider_id)?;
    }
    secrets::set_secret(&provider_id, key.trim())?;
    get_config(state).await
}

/// 测试连接，返回该后端可用的模型列表。
#[tauri::command]
pub async fn test_provider(state: State<'_, AppState>, provider_id: String) -> Result<Vec<String>> {
    state.gateway.health_check(&provider_id).await
}

/// 查这个后端有哪些模型能吃图。
///
/// 返回 `None` 表示**查不到**，前端应当什么都不显示 ——
/// 而不是显示"不支持视觉"。云端后端的视觉能力按型号定，
/// 硬编码一张表迟早过期，老实说不知道比猜错好。
#[tauri::command]
pub async fn provider_vision_models(
    state: State<'_, AppState>,
    provider_id: String,
) -> Result<Option<Vec<String>>> {
    state.gateway.vision_models(&provider_id).await
}

/// 这个后端当前加载的模型有多大上下文窗口（token）。
///
/// `None` = 查不到，前端回落到默认的字符预算，并且**要显示成「默认」
/// 而不是一个具体数字** —— 显示一个猜的数比不显示更糟。
#[tauri::command]
pub async fn provider_context_window(
    state: State<'_, AppState>,
    provider_id: String,
) -> Result<Option<u32>> {
    state.gateway.context_window(&provider_id).await
}

/// 描述的第一句。藏起来的工具只留一行 —— 几百字符的完整描述留给 `tool_search`。
fn first_sentence(desc: &str) -> String {
    let one = desc.replace('\n', " ");
    let cut = one
        .find(['。', '.'])
        .map(|i| i + one[i..].chars().next().map_or(1, char::len_utf8))
        .unwrap_or(one.len());
    let s = one[..cut.min(one.len())].trim().to_string();
    if s.chars().count() > 80 {
        s.chars().take(80).collect::<String>() + "…"
    } else {
        s
    }
}

/// 指定工具调用的备用后端。`None` = 恢复自动挑。
///
/// 存在的意义：自动规则是"任何配了密钥的云端"，而用户可能有好几个云端后端、
/// 只想让其中某一个跑工具（比如便宜的那个，或者只有它开了函数调用）。
#[tauri::command]
pub async fn set_tool_fallback(
    state: State<'_, AppState>,
    provider_id: Option<String>,
) -> Result<ConfigView> {
    {
        let config = state.gateway.config();
        let mut config = config.write().await;
        if let Some(id) = provider_id.as_deref() {
            // 先确认存在且真的支持工具，避免写进一个用不了的值 ——
            // 那会让回退静默失效，而现象是"工具调用突然不工作了"
            let provider = config.provider(id)?;
            if !provider.supports_tools {
                return Err(crate::error::Error::config(format!(
                    "{} 不支持流式工具调用，不能当备用后端",
                    provider.label
                )));
            }
        }
        config.tool_fallback_provider = provider_id;
        config.save(&state.config_path)?;
    }
    get_config(state).await
}

/// 设置某条链路用哪个模型。空字符串 = 清掉，沿用该后端的默认模型。
///
/// **模型属于链路，不属于后端。** 存在后端上的话，两条链路指向同一个后端时
/// 就必然共用一个模型 —— 而"实时对话用 8B、后台推理用 35B、都在本机
/// LM Studio 上"是最自然的配法，本机资源也够。
#[tauri::command]
pub async fn set_route_model(
    state: State<'_, AppState>,
    route: String,
    model: String,
) -> Result<ConfigView> {
    {
        let config = state.gateway.config();
        let mut config = config.write().await;
        let value = {
            let trimmed = model.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        };
        match route.as_str() {
            "realtime" => config.realtime_model = value,
            "background" => config.background_model = value,
            "tool_fallback" => config.tool_fallback_model = value,
            other => return Err(crate::error::Error::config(format!("未知的链路：{other}"))),
        }
        config.save(&state.config_path)?;
    }
    get_config(state).await
}

use crate::memory::store::{Project, SessionInfo, StoredMessage};

/// 拿到记忆系统，**必要时现场打开**。
///
/// 项目与会话都存在 memory.db 里，所以这些命令都要经过这里。
///
/// 之前这几个命令是直接读 `state.memory` 那个槽的 —— 而它是**惰性**的：
/// 应用刚启动时是 None，只有语音会话或一次文字对话跑过 `ensure_memory`
/// 之后才会有值。结果是"创建项目"「新建对话」在冷启动后**必然失败**，
/// 而失败信息只进了前端的 console（Tauri 的 webview 没有可读控制台），
/// 表现就是点了完全没反应。
async fn memory_of(state: &State<'_, AppState>) -> Result<std::sync::Arc<crate::memory::Memory>> {
    crate::commands_voice::ensure_memory(state)
        .await
        .ok_or_else(|| {
            Error::config(
                "记忆系统打不开，项目与会话功能不可用。\
                 检查两处：设置里记忆是否启用；嵌入后端（默认 Ollama :11434）能不能连上 —— \
                 建库时要先问它向量维度。"
                    .to_string(),
            )
        })
}

/// 当前情感轴。记忆没就绪就返回 None（界面把那一行藏起来）。
///
/// **为什么需要这个命令**：情感事件只在语音会话启动和每轮结束时发。
/// 应用刚打开、还没开语音的时候前端一个值都没有，于是状态行里的
/// 亲近/心情/活跃整行都不渲染 —— 看起来像"情感数值没了"。
/// 而情感是持续存在的背景信息，不该依赖"你得先开一次语音"。
///
/// 返回的形状要和 `VoiceEvent::Emotion` **完全一致** —— 前端两条路填的是同一个
/// 字段，档位和焦躁值少一个都会让那一行渲染不出来
#[derive(serde::Serialize)]
pub struct EmotionView {
    pub affection: f32,
    pub mood: f32,
    pub energy: f32,
    pub familiarity: crate::personality::Familiarity,
    pub irritation: f32,
}

#[tauri::command]
pub async fn get_emotion(state: tauri::State<'_, AppState>) -> Result<Option<EmotionView>> {
    // 这里刻意不报错：记忆连不上时界面照常用，只是没有情感显示
    let Some(memory) = crate::commands_voice::ensure_memory(&state).await else {
        return Ok(None);
    };
    let Ok(e) = memory.store().emotion().await else {
        return Ok(None);
    };
    Ok(Some(EmotionView {
        affection: e.affection,
        mood: e.mood,
        energy: e.energy,
        familiarity: crate::personality::Familiarity::of(e.affection),
        irritation: crate::personality::irritation(e),
    }))
}

#[tauri::command]
pub async fn get_projects(state: tauri::State<'_, AppState>) -> Result<Vec<Project>> {
    let memory = memory_of(&state).await?;
    memory.store().get_projects().await
}

#[tauri::command]
pub async fn create_project(
    state: tauri::State<'_, AppState>,
    name: String,
    workspace_path: Option<String>,
) -> Result<String> {
    let memory = memory_of(&state).await?;

    let id = uuid::Uuid::new_v4().to_string();
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    memory
        .store()
        .create_project(
            &id,
            &name,
            None,
            None,
            None,
            workspace_path.as_deref(),
            created_at,
        )
        .await?;

    // 路径一绑上就自动初始化 —— 不该让用户先创建、再去设置面板点一个按钮
    crate::workspace::auto_init_if_needed(workspace_path.as_deref(), &name).await;

    Ok(id)
}

#[tauri::command]
pub async fn update_project(
    state: tauri::State<'_, AppState>,
    id: String,
    name: String,
    description: Option<String>,
    system_prompt: Option<String>,
    character_id: Option<String>,
    workspace_path: Option<String>,
) -> Result<()> {
    let memory = memory_of(&state).await?;
    memory
        .store()
        .update_project(
            &id,
            &name,
            description.as_deref(),
            system_prompt.as_deref(),
            character_id.as_deref(),
            workspace_path.as_deref(),
        )
        .await?;

    // 工作区是在设置面板里后设的，所以这才是「路径第一次绑上项目」的
    // 主要入口。幂等：`.anima/project.json` 已经在就什么都不做
    crate::workspace::auto_init_if_needed(workspace_path.as_deref(), &name).await;
    Ok(())
}

#[tauri::command]
pub async fn delete_project(state: tauri::State<'_, AppState>, id: String) -> Result<()> {
    let memory = memory_of(&state).await?;
    memory.store().delete_project(&id).await
}

#[tauri::command]
pub async fn get_sessions(
    state: tauri::State<'_, AppState>,
    project_id: Option<String>,
    include_archived: bool,
) -> Result<Vec<SessionInfo>> {
    let memory = memory_of(&state).await?;
    memory
        .store()
        .get_sessions(project_id.as_deref(), include_archived)
        .await
}

#[tauri::command]
pub async fn get_all_sessions(
    state: tauri::State<'_, AppState>,
    include_archived: bool,
) -> Result<Vec<SessionInfo>> {
    let memory = memory_of(&state).await?;
    memory.store().get_all_sessions(include_archived).await
}

#[tauri::command]
pub async fn get_session_messages(
    state: tauri::State<'_, AppState>,
    session_id: String,
) -> Result<Vec<StoredMessage>> {
    let memory = memory_of(&state).await?;
    memory.store().get_session_messages(&session_id).await
}

#[tauri::command]
pub async fn update_session_title(
    state: tauri::State<'_, AppState>,
    id: String,
    title: String,
) -> Result<()> {
    let memory = memory_of(&state).await?;
    memory.store().update_session_title(&id, &title).await
}

#[tauri::command]
pub async fn archive_session(state: tauri::State<'_, AppState>, id: String) -> Result<()> {
    let memory = memory_of(&state).await?;
    memory.store().archive_session(&id).await
}

#[tauri::command]
pub async fn delete_session(state: State<'_, AppState>, id: String) -> Result<()> {
    let memory = memory_of(&state).await?;
    memory.store().delete_session(&id).await
}

#[tauri::command]
pub async fn create_session(
    state: State<'_, AppState>,
    project_id: Option<String>,
) -> Result<String> {
    let memory = memory_of(&state).await?;
    memory
        .store()
        .create_session_for_project(project_id.as_deref())
        .await
}

#[tauri::command]
pub async fn append_session_message(
    state: State<'_, AppState>,
    session_id: String,
    role: String,
    content: String,
    tool_json: Option<String>,
) -> Result<i64> {
    let memory = memory_of(&state).await?;
    let r = match role.as_str() {
        "user" => crate::llm::Role::User,
        "assistant" => crate::llm::Role::Assistant,
        "system" => crate::llm::Role::System,
        _ => crate::llm::Role::User,
    };
    memory
        .store()
        .append_message_with_tools(&session_id, r, &content, tool_json.as_deref())
        .await
}

#[tauri::command]
pub async fn delete_session_message(
    state: State<'_, AppState>,
    session_id: String,
    message_id: i64,
) -> Result<()> {
    let memory = memory_of(&state).await?;
    memory
        .store()
        .delete_session_message(&session_id, message_id)
        .await
}

#[tauri::command]
pub async fn rollback_session_to_message(
    state: State<'_, AppState>,
    session_id: String,
    message_id: i64,
) -> Result<()> {
    let memory = memory_of(&state).await?;
    memory
        .store()
        .rollback_session_to_message(&session_id, message_id)
        .await
}

// ---------------------------------------------------------------------------
// 上下文压缩（§1：把 voice 侧的 summary 引擎暴露给文字 chat）
// ---------------------------------------------------------------------------

/// 前端传入的简化消息格式，用于历史压缩。
#[derive(Debug, Deserialize)]
pub struct SimpleMessage {
    pub role: String,
    pub content: String,
}

/// 压缩结果。
#[derive(Debug, Serialize)]
pub struct CompressResult {
    pub summary: String,
    pub focus: Vec<String>,
    /// 这批消息是否真的被压进摘要了。
    ///
    /// 不够压（`worth_summarizing` 为假）或压缩失败时为 `false`，摘要原样返回。
    /// 调用方**必须**据此决定要不要推进「已压缩到哪」的游标 —— 不区分的话，
    /// 那几轮会被当成已压缩而永久跳过，内容真的就丢了。
    pub compressed: bool,
}

/// 压缩被裁掉的历史消息，返回新的累积摘要。
///
/// 文字 chat 调这个接口把滑动窗口丢弃的轮次压缩成摘要，
/// 与语音侧的 `voice/mod.rs` 共用同一套 `memory::summary` 引擎。
#[tauri::command]
pub async fn compress_history(
    state: State<'_, AppState>,
    previous_summary: String,
    previous_focus: Vec<String>,
    dropped_messages: Vec<SimpleMessage>,
) -> Result<CompressResult> {
    use crate::llm::Role;
    use crate::memory::summary::{self, Summary};

    let previous = Summary {
        text: previous_summary,
        focus: previous_focus,
    };
    let dropped: Vec<Message> = dropped_messages
        .into_iter()
        .map(|m| {
            let role = match m.role.as_str() {
                "user" => Role::User,
                "assistant" => Role::Assistant,
                _ => Role::System,
            };
            Message {
                role,
                content: m.content,
                images: Vec::new(),
                tool_calls: Vec::new(),
                tool_call_id: None,
            }
        })
        .collect();

    if !summary::worth_summarizing(&dropped) {
        return Ok(CompressResult {
            summary: previous.text,
            focus: previous.focus,
            compressed: false,
        });
    }

    match summary::compress(&state.gateway, &previous, &dropped).await {
        Some(s) => Ok(CompressResult {
            summary: s.text,
            focus: s.focus,
            compressed: true,
        }),
        None => Ok(CompressResult {
            summary: previous.text,
            focus: previous.focus,
            compressed: false,
        }),
    }
}

#[cfg(test)]
mod chat_attachment_tests {
    use super::*;

    #[test]
    fn 上传图片文件名会清洗掉路径和特殊字符() {
        assert_eq!(safe_attachment_stem("../../头像 final!.png"), "头像_final");
        assert_eq!(
            safe_attachment_stem("portrait-v1_02.webp"),
            "portrait-v1_02"
        );
        assert_eq!(safe_attachment_stem("!!!.png"), "image");
    }

    #[test]
    fn 上传图片只接受明确支持的mime() {
        assert_eq!(image_ext_from_mime("image/png"), Some("png"));
        assert_eq!(image_ext_from_mime("image/jpeg"), Some("jpg"));
        assert_eq!(image_ext_from_mime("image/webp"), Some("webp"));
        assert_eq!(image_ext_from_mime("text/plain"), None);
    }

    #[test]
    fn 有工作区就把chat图片存进项目目录() {
        let dir = chat_upload_dir(
            Path::new("/Users/you/Library/Application Support/com.anima.desktop"),
            Some("/Users/you/AI/workspace/Anima"),
        );
        assert_eq!(
            dir,
            PathBuf::from("/Users/you/AI/workspace/Anima/.anima/uploads/chat")
        );
    }
}

#[cfg(test)]
mod theme_mode_tests {
    use super::*;

    #[test]
    fn 三个取值都认得() {
        assert_eq!(normalize_theme_mode("light"), "light");
        assert_eq!(normalize_theme_mode("dark"), "dark");
        assert_eq!(normalize_theme_mode("system"), "system");
        assert_eq!(normalize_theme_mode(" Dark "), "dark");
    }

    /// **不认识的回 `system`，不能回 `light`。**
    ///
    /// 回 light 的话，配置文件被手改坏之后，深色用户开机会被闪一脸白。
    #[test]
    fn 不认识的回跟随系统() {
        for raw in ["", "nope", "auto", "深色"] {
            assert_eq!(normalize_theme_mode(raw), "system", "raw = {raw:?}");
        }
    }
}

#[cfg(test)]
mod startup_window_tests {
    use super::*;

    /// 启动窗口的取值**必须和 `windows::PANELS` 里的 label 对得上**。
    ///
    /// 对不上的话启动时那句 `windows::open` 静默失败 ——
    /// 表现是「设置里选了 Anima Studio，启动后什么都没开」，不报任何错。
    #[test]
    fn 启动窗口的取值都是真实存在的面板() {
        for raw in ["chat", "anima_studio", "AVATAR", " Chat "] {
            let w = normalize_startup_window(raw);
            assert!(
                w == "avatar" || crate::windows::panel(&w).is_some(),
                "{raw} 归一化成了不存在的面板 {w}"
            );
        }
    }

    #[test]
    fn 不认识的启动窗口回落到桌宠() {
        for raw in ["", "nope", "../etc"] {
            assert_eq!(normalize_startup_window(raw), "avatar", "raw = {raw:?}");
        }
    }

    #[test]
    fn 认得_anima_studio_和_chat() {
        assert_eq!(normalize_startup_window("anima_studio"), "anima_studio");
        assert_eq!(normalize_startup_window("chat"), "chat");
    }
}

#[cfg(test)]
mod config_view_tests {
    use super::first_sentence;

    /// **前端能改的每一项，`ConfigView` 都必须能读回来。**
    ///
    /// 撞到过一次，症状很迷惑：点击确实把值写进了 `config.json`
    /// （`image_engine: "sdxl"`、`sdxl_checkpoint: "AWPortrait_XL..."` 都在），
    /// 但界面高亮一直停在默认项，看起来完全像"点了没反应"。
    ///
    /// 原因是 `AppConfig` 加了字段而 `ConfigView` 没加 —— 而 `ConfigView` 是
    /// 前端**唯一**能读到配置的通道。写得进去、读不出来。
    ///
    /// 这条测试盯的是**结构对齐**：有 setter 命令的字段，必须出现在 view 里。
    #[test]
    fn 有setter的配置项都要能读回来() {
        let src = include_str!("commands.rs");
        // 从本文件里找出所有 `config.<字段> = ` 的赋值 —— 那就是有 setter 的项
        let mut settable: Vec<&str> = Vec::new();
        for line in src.lines() {
            let t = line.trim();
            if let Some(rest) = t.strip_prefix("config.") {
                if let Some(field) = rest.split(&[' ', '='][..]).next() {
                    if rest.contains('=') && !rest.contains("==") && !field.is_empty() {
                        settable.push(field);
                    }
                }
            }
        }
        assert!(!settable.is_empty(), "一个 setter 都没找到，这条测试失效了");

        // ConfigView 的构造块
        let start = src.find("Ok(ConfigView {").expect("找不到 ConfigView 构造");
        let end = src[start..].find("\n    })").expect("构造块没闭合") + start;
        let ctor = &src[start..end];

        for f in settable {
            assert!(
                ctor.contains(&format!("{f}:")),
                "配置项 `{f}` 有 setter 却不在 ConfigView 里 —— \
                 前端写得进去、读不出来，界面会停在默认值看起来像点了没反应"
            );
        }
    }

    /// **藏起来的工具必须在 `tool_search` 的描述里留下名字。**
    ///
    /// 不留的话模型根本不知道它们存在，也就永远不会去查 ——
    /// 那不是「省了上下文」，是「砍掉了能力」。
    #[test]
    fn 分层会把冷门工具的名字挂到检索工具上() {
        let src = include_str!("commands.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("源码是空的");
        let i = src.find("工具分层：常驻的给完整 schema").expect("找不到分层那一段");
        // **按字符切，不按字节切** —— 源码里全是中文，`&s[i..i+2200]`
        // 会落在某个汉字中间直接 panic。
        let block: String = src[i..].chars().take(1600).collect();
        let block = block.as_str();
        assert!(block.contains("tool_tier::is_always_on"), "没按常驻名单过滤");
        assert!(block.contains("unlocked_from_history"), "没从历史里认解锁");
        // **要钉住那个条件本身。**
        //
        // 只查「文件里有 t.name == "tool_search"」的话，把
        // `if !hidden.is_empty()` 改成 `if false` 也照样绿：那段代码还在，
        // 只是永远不执行。今天已经在别处踩过好几次同样的坑。
        let guard = block.find("if !hidden.is_empty()").expect("挂名字那段的条件没了");
        let arm: String = block[guard..].chars().take(400).collect();
        assert!(
            arm.contains(r#"t.name == "tool_search""#),
            "藏起来的名字没挂到 tool_search 上"
        );
        // 过滤必须真的发生，不能只算不用
        assert!(block.contains(".retain("), "算了却没过滤");
    }

    /// 一句话就够 —— 完整描述留给 `tool_search` 返回。
    #[test]
    fn 只取描述的第一句() {
        assert_eq!(first_sentence("把图片变成视频。支持三种引擎。"), "把图片变成视频。");
        assert_eq!(first_sentence("Animate an image. Routes through LTX."), "Animate an image.");
        // 没有句号就整段（但要截断）
        let long = "一".repeat(200);
        assert!(first_sentence(&long).chars().count() <= 81);
        // 换行不该把描述截断成两半
        assert_eq!(first_sentence("第一行\n第二行。"), "第一行 第二行。");
    }
}

#[cfg(test)]
mod default_image_engine_tests {
    use super::resolve_default_image_engine as r;

    /// 加了新引擎却没同步这里 —— 表现是卡片点得下去、存进去的是 Z-Image，**不报错**。
    #[test]
    fn 设置里能选的每一个底模都存得进去() {
        for id in ["zimage", "sdxl", "krea2", "qwen21"] {
            assert_eq!(r(id), id, "{id} 被静默换成了别的引擎");
        }
        // 已下架的引擎存不进去（2026-10-01 Chroma 连权重一起删了）
        assert_ne!(r("chroma"), "chroma");
    }

    #[test]
    fn 别名和大小写都认() {
        assert_eq!(r("Z-Image"), "zimage");
        assert_eq!(r("qwen-image-2.1"), "qwen21");
    }

    /// 参考生图引擎必须有参考图才跑得动，当默认底模会让每一次普通出图都失败。
    #[test]
    fn 参考生图引擎当不了默认底模() {
        assert_eq!(r("qwen_edit"), "zimage");
        assert_eq!(r("krea2_edit"), "zimage");
    }

    /// 认不出的（比如 `config.json` 里存着已退役的 `flux`）回落 Z-Image，不报错也不留空。
    #[test]
    fn 认不出的回落zimage() {
        assert_eq!(r("flux"), "zimage");
        assert_eq!(r(""), "zimage");
    }
}

#[cfg(test)]
mod media_guidance_tests {
    /// 对话里的出图 / 出片指引要跟上当前底模（2026-10-04 外部审查指出）：默认 Qwen 2.1 读中文，
    /// 只有回落 SDXL 时才要英文；整张设定卡不能当 H3 身份参考（脸只有 ~40 px）。
    #[test]
    fn 出图出片指引跟当前底模一致() {
        let src = include_str!("commands.rs");
        let prod = &src[..src.find("#[cfg(test)]").unwrap()];
        assert!(!prod.contains("`prompt` MUST be English"), "还在要求出图一律英文");
        assert!(prod.contains("Prompt language follows the engine.") && prod.contains("Only SDXL needs English"));
        assert!(!prod.contains("character sheet works about as well"));
        let tools = [include_str!("agent_runtime/image_tool.rs"), include_str!("agent_runtime/video_tool.rs")];
        for t in tools {
            assert!(!t.contains("carries as much identity") && !t.contains("sheet works about as well"), "工具说明还在推荐整张设定卡当身份参考");
            assert!(!t.contains("roughly 3 minutes of wall time"), "耗时还是旧数");
        }
    }
}
