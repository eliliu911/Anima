//! 任务台账：task-scoped 记忆层。
//!
//! ## 要解决的症状
//!
//! 不是「记不住事」，是这两个已经发生过的：撞满 40 轮之后半截停下、接一句
//! 「继续」它从头开始探索目录；以及界面显示「完成」而内容没完成。
//!
//! 共同根因是**任务自己的计划和进度混在可被压缩的对话历史里**。
//! 前端的 `buildBoundedMessages` 一裁，计划就没了。
//!
//! ## 为什么进度是系统记录的，不是模型自报的
//!
//! 本机实测（构造一个真实中局：台账已存在、刚跑完 `npm test` 且失败，
//! 系统提示明确要求「每当一个步骤完成、失败、或你改变了计划，都要立刻再调一次
//! task_plan 更新状态」）：
//!
//! | 模型 | 3 次运行的调用序列 | 更新台账 |
//! |---|---|---|
//! | `ornith-1.5-35b-a3b` | `[view_file, view_file]` ×3 | **0/3** |
//! | `qwen/qwen3.8-27b` | `[apply_patch]` ×3 | **0/3** |
//!
//! **0/6，两个模型，逐次相同。** 连首轮听话的 qwen3.8-27b 在失败之后也直奔修补。
//!
//! 一份只在第 0 轮写、之后再不更新的台账，跑到第 10 轮时会说所有步骤都还是
//! todo —— **那比没有台账更糟**，模型会照着它重做已经完成的事。
//!
//! 所以 [`TaskLedger`] 里**没有 status 字段**。计划问模型（一次），
//! 进度问系统（每轮）：`ToolActivity::Finished` 的 `ok` 已经够用，
//! 而 `shell_command` 本来就把非零退出分流到 `ToolOutcome::failed`。
//!
//! ## 创建这一半靠措辞
//!
//! 同一个多步任务，三种系统提示措辞，看首个工具调用是不是 `task_plan`：
//!
//! | | 「多步任务先用 task_plan」 | 加触发条件 | **禁止**先调其它工具 |
//! |---|---|---|---|
//! | `ornith-1.5-35b-a3b` | ❌ | ❌ | ✅ |
//! | `qwen/qwen3.8-27b` | ✅ | ✅ | ✅ |
//!
//! 措辞比模型更决定结果。工作区 header 里必须用「禁止」那一档。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// goal 的字符上限。它是一句话，超了说明模型在写计划而不是目标。
const MAX_GOAL_CHARS: usize = 300;
const MAX_STEPS: usize = 20;
const MAX_STEP_CHARS: usize = 120;
/// 渲染时「改过/读过」最多占几行
const MAX_TOUCHED_LINES: usize = 15;
/// 渲染时命令执行最多占几行
const MAX_RUN_LINES: usize = 10;
/// `last_failure` 的字符上限
const MAX_FAILURE_CHARS: usize = 800;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TouchKind {
    Read,
    /// 改过的比读过的重要 —— 渲染封顶时优先保留
    Modified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandRun {
    pub cmd: String,
    pub ok: bool,
    pub round: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Failure {
    pub tool: String,
    pub target: String,
    /// 输出的**关键片段**，见 [`failure_excerpt`]
    pub excerpt: String,
    pub round: u32,
}

/// 一个任务的台账。
///
/// **没有 status 字段**，理由见模块注释。`steps` 是模型写下的计划（意图），
/// `touched` / `runs` / `last_failure` 是系统记录（事实）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskLedger {
    /// 模型写一次。**首次写入后不可覆盖** —— 步骤会被重写，意图不该被重写。
    pub goal: String,
    /// 这个 goal 是系统兜底填的（用户原话），不是模型写的。
    ///
    /// 系统填的占位可以被模型的正式 goal 直接替换、不需要 `goal_revision` ——
    /// 模型的措辞通常比用户原话更贴合执行，而"保护意图不被改写"保护的是
    /// **已经确立的**意图，不是一个占位。
    #[serde(default)]
    pub goal_from_system: bool,
    /// 计划。只是标题列表
    pub steps: Vec<String>,
    /// 计划被改过几次。>1 时界面要标出来 —— 反复重新规划本身是个信号
    pub plan_revision: u32,
    /// goal 的改写记录。改 goal 必须留痕，否则「意图漂移」查不出来
    pub goal_revisions: Vec<String>,
    /// 系统记录：被读过 / 改过的文件
    pub touched: BTreeMap<String, TouchKind>,
    /// 系统记录：命令执行（事件，顺序重要）
    pub runs: Vec<CommandRun>,
    /// 系统记录：最近一次失败
    pub last_failure: Option<Failure>,
    /// 已经跑了多少轮
    pub rounds: u32,
    pub updated_at: i64,
    /// 归档之后停止注入，但文件留着、界面可查
    pub archived: bool,
}

impl TaskLedger {
    pub fn is_empty(&self) -> bool {
        self.goal.is_empty() && self.steps.is_empty()
    }

    /// 模型调 `task_plan` 时应用计划。
    ///
    /// 返回给模型看的一句确认。**goal 首次写入后忽略后续覆盖**，
    /// 除非带 `goal_revision` —— 那时记一条改写理由。
    pub fn apply_plan(
        &mut self,
        goal: &str,
        steps: Vec<String>,
        goal_revision: Option<&str>,
    ) -> String {
        let goal = clip(goal.trim(), MAX_GOAL_CHARS);
        let mut notes = Vec::new();

        if self.goal.is_empty() {
            self.goal = goal;
        } else if self.goal_from_system && !goal.is_empty() {
            // 系统占位被正式 goal 替换，不算改写
            self.goal = goal;
            self.goal_from_system = false;
            notes.push("已用你写的目标替换系统占位".to_string());
        } else if goal != self.goal && !goal.is_empty() {
            match goal_revision {
                Some(reason) if !reason.trim().is_empty() => {
                    self.goal_revisions
                        .push(format!("{} → {}（{}）", self.goal, goal, reason.trim()));
                    self.goal = goal;
                    notes.push("目标已改写并留痕".to_string());
                }
                // **拒绝静默改写。** 目标被悄悄降级是模型在多次失败后最常见的
                // 自我妥协方式，而它一旦发生就没有任何痕迹。
                _ => notes.push(
                    "目标未改动 —— 要改 goal 必须带 goal_revision 说明理由".to_string(),
                ),
            }
        }

        let had_steps = !self.steps.is_empty();
        let new_steps: Vec<String> = steps
            .into_iter()
            .map(|s| clip(s.trim(), MAX_STEP_CHARS))
            .filter(|s| !s.is_empty())
            .take(MAX_STEPS)
            .collect();
        if !new_steps.is_empty() {
            if had_steps && new_steps != self.steps {
                self.plan_revision += 1;
            }
            self.steps = new_steps;
        }
        self.archived = false;
        self.touch_time();

        let mut msg = format!("台账已记录：{} 个步骤。", self.steps.len());
        if self.plan_revision > 0 {
            msg.push_str(&format!("（计划已改过 {} 次）", self.plan_revision));
        }
        for n in notes {
            msg.push_str(&format!(" {n}。"));
        }
        msg.push_str(
            " 进度由系统自动记录，**不需要**你再调 task_plan 更新状态；\
             只有真的改了计划才需要再调一次。",
        );
        msg
    }

    /// 系统兜底：用用户原话当 goal 建一份最小台账。
    ///
    /// ## 为什么必须有这条
    ///
    /// 实测 `openai/gpt-oss-120b`（本项目的后台档，各 3 次）：
    ///
    /// | 任务措辞 | Effort=Low | Effort=High |
    /// |---|---|---|
    /// | 「在 demo/app 里实现…」（提到目录） | 3/3 ✅ | **0/3 ❌** 直奔 `fs_list_directory` |
    /// | 点名具体路径 + 明说「不需要先列目录」 | 3/3 ✅ | 2/3 ⚠️ |
    ///
    /// 而且 High 档下**第 3 轮的点名兜底也无效**（0/3，它重复调
    /// `fs_list_directory`）。
    ///
    /// 两个变量都在起作用：**低档稳定合规，高档对任务文本敏感**
    /// （提到目录就想去列目录）。别只记住其中一个 —— 我第一次就把它
    /// 归因成"High 档坏了"，而换个措辞 High 也能到 2/3。
    ///
    /// 结论仍然是：「靠措辞让模型自己建台账」不可依赖。而台账里
    /// **载荷最重的字段是 `goal`**（意图能不能活过上下文压缩），
    /// 而那个字段的内容我们本来就有 —— 用户的原话。
    ///
    /// 所以不押宝在模型的自发行为上：steps 拿不到就先空着，
    /// 系统记录（改过哪些文件、跑了什么命令、失败在哪）照样在。
    pub fn seed_from_user(&mut self, user_text: &str) {
        if !self.goal.is_empty() {
            return;
        }
        let text = user_text.trim();
        if text.is_empty() {
            return;
        }
        // 取首段：用户原话常常很长，而 goal 是"一句话说清要什么"
        let first = text
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or(text);
        self.goal = clip(first, MAX_GOAL_CHARS);
        self.goal_from_system = true;
        self.touch_time();
    }

    /// 系统侧记录一次工具执行。零模型配合。
    pub fn record_tool(
        &mut self,
        name: &str,
        arguments: &str,
        ok: bool,
        output: &str,
        round: u32,
    ) {
        let target = primary_arg(arguments).unwrap_or_default();
        match classify(name) {
            ToolClass::Read => {
                if !target.is_empty() {
                    // 已经标成 Modified 的不要降级成 Read
                    self.touched
                        .entry(target.clone())
                        .or_insert(TouchKind::Read);
                }
            }
            ToolClass::Modify => {
                if !target.is_empty() {
                    self.touched.insert(target.clone(), TouchKind::Modified);
                }
            }
            ToolClass::Command => {
                self.runs.push(CommandRun {
                    cmd: clip(&target, MAX_STEP_CHARS),
                    ok,
                    round,
                });
            }
            ToolClass::Other => {}
        }
        if !ok {
            self.last_failure = Some(Failure {
                tool: name.to_string(),
                target: clip(&target, MAX_STEP_CHARS),
                excerpt: failure_excerpt(output, MAX_FAILURE_CHARS),
                round,
            });
        } else if matches!(classify(name), ToolClass::Command) {
            // 后续**命令**成功了，说明上一次失败已经解决，清掉陈旧的记录。
            //
            // **只认 Command，不认 Modify。** 「补丁写成功」不是「失败已解决」
            // 的证据 —— 真实序列是「测试失败 → 改代码 → 重跑测试」，
            // 中间那一步成功时失败**仍然没有解决**。
            //
            // 而 `last_failure` 是模型唯一能活过上下文压缩的「错在哪」记录
            // （它进注入块）。在改完代码、还没重跑验证的那一刻把它清掉，
            // 等于在最需要它的时候扔掉它 —— 而自愈那边照样会触发
            // （`runs.last()` 还是那次失败，Modify 不进 `runs`），
            // 于是模型被告知「最近的命令执行报错」，台账却不再告诉它报的什么错。
            self.last_failure = None;
        }
        self.rounds = self.rounds.max(round);
        self.touch_time();
    }

    fn touch_time(&mut self) {
        self.updated_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
    }

    /// 渲染成注入块。`None` 表示没什么可注入的。
    pub fn render(&self) -> Option<String> {
        if self.archived || self.is_empty() {
            return None;
        }
        let mut out = String::from("<task-ledger>\n");
        if !self.goal.is_empty() {
            out.push_str(&format!("目标：{}\n", self.goal));
            if self.goal_from_system {
                // 说清这是系统摘的，并邀请它写一个更准的 —— 否则模型会以为
                // 目标已经定好了，不再调 task_plan，steps 就永远是空的
                out.push_str(
                    "（这个目标是系统从你最初那条消息里摘的。如果理解有偏差，\
                     调一次 task_plan 写清 goal 和步骤。）\n",
                );
            }
        }
        if !self.goal_revisions.is_empty() {
            // 改写留痕要进注入块，否则「我一开始要的是什么」只有系统知道
            out.push_str(&format!(
                "（目标改写过 {} 次：{}）\n",
                self.goal_revisions.len(),
                self.goal_revisions.join("；")
            ));
        }

        if !self.steps.is_empty() {
            out.push_str(&format!("\n计划（共 {} 步", self.steps.len()));
            if self.plan_revision > 0 {
                out.push_str(&format!("，改过 {} 次", self.plan_revision));
            }
            out.push_str("）：\n");
            for (i, s) in self.steps.iter().enumerate() {
                out.push_str(&format!("  {}. {}\n", i + 1, s));
            }
        }

        let modified: Vec<&String> = self
            .touched
            .iter()
            .filter(|(_, k)| **k == TouchKind::Modified)
            .map(|(p, _)| p)
            .collect();
        let read: Vec<&String> = self
            .touched
            .iter()
            .filter(|(_, k)| **k == TouchKind::Read)
            .map(|(p, _)| p)
            .collect();

        if !modified.is_empty() || !read.is_empty() || !self.runs.is_empty() {
            out.push_str(&format!("\n已发生（系统记录，{} 轮）：\n", self.rounds));
            // 改过的优先占额度 —— 它比"读过"信息量高得多
            let mut budget = MAX_TOUCHED_LINES;
            if !modified.is_empty() {
                out.push_str(&format!("  改过：{}\n", join_capped(&modified, &mut budget)));
            }
            if !read.is_empty() && budget > 0 {
                out.push_str(&format!("  读过：{}\n", join_capped(&read, &mut budget)));
            }
            if !self.runs.is_empty() {
                out.push_str(&format!("  命令：{}\n", self.render_runs()));
            }
        }

        if let Some(f) = &self.last_failure {
            out.push_str(&format!(
                "\n最近一次失败（{} {}，第 {} 轮）：\n",
                f.tool, f.target, f.round
            ));
            for line in f.excerpt.lines() {
                out.push_str(&format!("  {line}\n"));
            }
        }

        // 这两行是必要的：没有它，模型会把台账当成待总结的资料而不是要接着做的事。
        // `run_skill` 撞过同一个坑（见 skill::expand_task 的框架语）。
        out.push_str(
            "\n按计划继续，不要重新规划。「已发生」是系统的真实记录，不是你的自述 —— \
             和你的印象不一致时以它为准。\n</task-ledger>",
        );
        Some(out)
    }

    /// 命令执行的渲染。**失败的优先保留** —— 成功的命令没什么信息量，
    /// 失败的是「别再走这条路」的唯一线索。
    fn render_runs(&self) -> String {
        let mut chosen: Vec<&CommandRun> = self.runs.iter().filter(|r| !r.ok).collect();
        let dropped_ok = self.runs.len() - chosen.len();
        if chosen.len() < MAX_RUN_LINES {
            for r in self.runs.iter().rev().filter(|r| r.ok) {
                if chosen.len() >= MAX_RUN_LINES {
                    break;
                }
                chosen.push(r);
            }
        }
        chosen.sort_by_key(|r| r.round);
        let extra = self.runs.len().saturating_sub(chosen.len());
        let mut s = chosen
            .iter()
            .take(MAX_RUN_LINES)
            .map(|r| {
                format!(
                    "{}{}（第 {} 轮）",
                    r.cmd,
                    if r.ok { " ✓" } else { " ✗" },
                    r.round
                )
            })
            .collect::<Vec<_>>()
            .join("、");
        if extra > 0 {
            s.push_str(&format!("，另有 {extra} 次"));
        } else if dropped_ok > 0 && chosen.len() >= MAX_RUN_LINES {
            s.push_str(&format!("，另有 {dropped_ok} 次成功的"));
        }
        s
    }
}

fn join_capped(items: &[&String], budget: &mut usize) -> String {
    let take = items.len().min(*budget);
    *budget -= take;
    let mut s = items
        .iter()
        .take(take)
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join("、");
    if items.len() > take {
        s.push_str(&format!("…及另外 {} 个", items.len() - take));
    }
    s
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).chain("…".chars()).collect()
}

enum ToolClass {
    Read,
    Modify,
    Command,
    Other,
}

/// 工具归类。名字里带 `__` 的 MCP 工具也认后缀。
fn classify(name: &str) -> ToolClass {
    let base = name.rsplit("__").next().unwrap_or(name);
    match base {
        "view_file" | "fs_read_text" | "read_file" | "code_outline" | "workspace_search" => {
            ToolClass::Read
        }
        "apply_patch" | "fs_write_text" | "write_file" | "edit_file" => ToolClass::Modify,
        "shell_command" | "exec_command" | "run_command" => ToolClass::Command,
        _ => ToolClass::Other,
    }
}

/// 从工具参数 JSON 里取「主要参数」。
///
/// 顺序有讲究：命令类的 `cmd`/`command` 要排在 `path` 前面，因为
/// `exec_command` 同时带 `command` 和可选的 `cwd`/`path`，而我们要的是命令本身。
pub fn primary_arg(arguments: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(arguments.trim()).ok()?;
    for key in [
        "cmd",
        "command",
        "path",
        "file_path",
        "filePath",
        "target",
        "query",
    ] {
        if let Some(s) = v.get(key).and_then(|x| x.as_str()) {
            let s = s.trim();
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

/// 失败输出里最该保留的片段。
///
/// ## 为什么不能只取尾部
///
/// 项目里别处的 `truncate()` 都取头部，而失败输出的结论在**末尾**
/// （`ℹ tests 6 pass 5 fail 1` / `exit code 1`），取头会正好把它切掉。
///
/// 但只取尾部也不够：模型喜欢一次跑复合命令
/// （`node --version && cat x && npm test`），而一个长堆栈会把
/// `AssertionError` 那一行挤出尾部窗口 —— 于是模型知道"失败了"却不知道失败在哪。
///
/// 所以取**两段**：最后一个错误标记周围的上下文 + 输出末尾，中间标明省略。
/// 两段重叠时合成一段。
pub fn failure_excerpt(output: &str, cap: usize) -> String {
    let text = output.trim_end();
    if text.chars().count() <= cap {
        return text.to_string();
    }
    // **先折叠调用栈再选片段。** 顺序反了的话尾部窗口会整个落在栈帧上。
    let collapsed = collapse_frames(&text.lines().collect::<Vec<_>>());
    let lines: Vec<&str> = collapsed.iter().map(|s| s.as_str()).collect();
    // 折叠之后可能已经够短了
    if lines.join("\n").chars().count() <= cap {
        return lines.join("\n");
    }

    // 先算尾部窗口，占一半额度。结论在末尾（`ℹ fail 1` / `exit code 1`），
    // 这一段永远要保住。
    let tail_cap = cap / 2;
    let tail_start = tail_boundary(&lines, tail_cap);

    // **只在尾部窗口之外找锚点，而且只认「错误身份」标记。**
    //
    // 早先的版本把 `fail ` 也当锚点，结果它命中了末尾的汇总行 `ℹ fail 1` ——
    // 锚点落在尾部窗口里，代码判断"一段就够"，于是 AssertionError 被丢掉。
    // 汇总行说明"失败了"，错误身份行说明"失败在哪"，只有后者能当锚点。
    const IDENTITY: &[&str] = &[
        "assertionerror",
        "error:",
        "error [",
        "typeerror",
        "referenceerror",
        "panic",
        "exception",
        "traceback",
        "not ok",
        "✖",
        "✗",
        "fatal",
        "syntaxerror",
    ];
    let anchor = lines[..tail_start].iter().rposition(|l| {
        let low = l.to_ascii_lowercase();
        IDENTITY.iter().any(|m| low.contains(m))
    });

    let Some(m) = anchor else {
        // 锚点在尾部里，或者压根没有 —— 一段就够，用完整额度
        return tail_from(&lines, cap);
    };

    // 两段：锚点前 3 行到锚点后 3 行，加尾部
    let head_from = m.saturating_sub(3);
    let head_to = (m + 4).min(tail_start);
    let mut out = clip(&lines[head_from..head_to].join("\n"), cap - tail_cap);
    if head_from > 0 {
        out = format!("…（省略前 {head_from} 行）…\n{out}");
    }
    let omitted = tail_start.saturating_sub(head_to);
    if omitted > 0 {
        out.push_str(&format!("\n…（省略 {omitted} 行）…\n"));
    } else {
        out.push('\n');
    }
    out.push_str(&lines[tail_start..].join("\n"));
    out
}

/// 这一行是调用栈帧吗。
///
/// 认三种常见形状：JS/Java 的 `    at foo (file:1:2)`、Python 的
/// `  File "x", line 3`、Rust backtrace 的 `  12: 0x... - symbol`。
fn is_stack_frame(line: &str) -> bool {
    let t = line.trim_start();
    if t.starts_with("at ") {
        return true;
    }
    if t.starts_with("File \"") {
        return true;
    }
    // Rust：`  12: 0x104...`
    let mut it = t.splitn(2, ": ");
    if let (Some(num), Some(rest)) = (it.next(), it.next()) {
        if !num.is_empty()
            && num.len() <= 3
            && num.chars().all(|c| c.is_ascii_digit())
            && rest.starts_with("0x")
        {
            return true;
        }
    }
    false
}

/// 把连续的调用栈折叠掉。
///
/// ## 为什么必须在选片段之前做
///
/// 不折叠的话，尾部窗口很可能整个落在栈帧上 —— 实测一个 60 层的 node 堆栈，
/// 800 字符的额度有 7 行给了 `at internal/frame/53..59`，纯噪声。
/// 断言（"AssertionError 在不在"）照样通过，**只有看输出才发现额度被浪费了**。
///
/// 保留前 2 层：第一层是出错位置（`lru.test.ts:41:5`），第二层给一点调用来源。
/// 再往下是运行时内部帧，对定位问题没有帮助。
fn collapse_frames(lines: &[&str]) -> Vec<String> {
    const KEEP: usize = 2;
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        if !is_stack_frame(lines[i]) {
            out.push(lines[i].to_string());
            i += 1;
            continue;
        }
        let start = i;
        while i < lines.len() && is_stack_frame(lines[i]) {
            i += 1;
        }
        let run = i - start;
        // 短栈原样保留 —— 折叠反而更长
        if run <= KEEP + 1 {
            out.extend(lines[start..i].iter().map(|s| s.to_string()));
            continue;
        }
        out.extend(lines[start..start + KEEP].iter().map(|s| s.to_string()));
        out.push(format!("      …（另有 {} 层调用栈）…", run - KEEP));
    }
    out
}

/// 从末尾往前，在 `cap` 字符内能装下的起始行号。
fn tail_boundary(lines: &[&str], cap: usize) -> usize {
    let mut start = lines.len();
    let mut len = 0usize;
    while start > 0 {
        let l = lines[start - 1];
        if len + l.chars().count() + 1 > cap && len > 0 {
            break;
        }
        len += l.chars().count() + 1;
        start -= 1;
    }
    start
}

fn tail_from(lines: &[&str], cap: usize) -> String {
    let mut start = lines.len();
    let mut len = 0usize;
    while start > 0 {
        let l = lines[start - 1];
        if len + l.chars().count() + 1 > cap && len > 0 {
            break;
        }
        len += l.chars().count() + 1;
        start -= 1;
    }
    let mut out = String::new();
    if start > 0 {
        out.push_str(&format!("…（省略前 {start} 行）…\n"));
    }
    out.push_str(&lines[start..].join("\n"));
    out
}

/// `task_plan` 的工具声明。
///
/// 描述里明确写了「进度由系统自动记录」—— 不写的话模型会试图在 steps 里带
/// status，而这个结构里**没有** status 字段（实测它写完第 0 轮再也不更新，
/// 一个永远显示 todo 的清单比没有清单更误导）。
pub fn declaration() -> crate::llm::Tool {
    crate::llm::Tool {
        name: crate::agent::TASK_PLAN_NAME.to_string(),
        description: "写下当前任务的目标与步骤。超过三个步骤的任务，\
             这是第一件要做的事 —— 对话历史会被压缩，不写下来你在第 10 轮\
             会不知道自己在做什么。\n\n\
             **进度由系统自动记录**（改了哪些文件、跑了什么命令、失败在哪），\
             你不需要回来更新状态；只有真的改了计划才再调一次。\n\n\
             每一步要能单独验证：写「跑 npm test 到全绿」，不要写「测试」。"
            .to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "goal": {
                    "type": "string",
                    "description": "一句话说清这次要达成什么。**首次写入后不可更改** —— 要改必须同时给 goal_revision 说明理由"
                },
                "steps": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "有序的步骤标题列表。不要带状态，状态由系统记录"
                },
                "goal_revision": {
                    "type": "string",
                    "description": "只有要改 goal 时才填：为什么改。不填则 goal 保持原样"
                }
            },
            "required": ["goal", "steps"]
        }),
    }
}

/// 把台账注入进**最后一条 user 消息的开头**。返回是否注入成功。
///
/// ## 为什么单独一个函数
///
/// 这是整个功能里「静默出错」风险最高的一处：注错位置（塞进 system）不会报错、
/// 不会少功能，只会让**每一轮的前缀缓存全部失效** —— 表现是首字延迟变长，
/// 而那离「台账注入位置写错了」非常远，几乎不可能被联想到。
///
/// 前缀代价是精确可算的：台账在最后一条 user 消息里，所以下一轮的前缀在
/// **上一条 user 消息**处分叉，要重付的是「上一条 user + 上一条 assistant +
/// 这一条 user」，而后两个本来就是新内容 —— 多付的只有一条 user 消息。
/// 放 system 则要重付全部。
pub fn inject(messages: &mut [crate::llm::Message], block: &str) -> bool {
    use crate::llm::Role;
    match messages.iter_mut().rev().find(|m| m.role == Role::User) {
        Some(m) => {
            m.content = format!("{block}\n\n{}", m.content);
            true
        }
        // 没有 user 消息（理论上不该发生）时什么都不做 ——
        // **绝不退而求其次塞进 system**，那正是要避免的事
        None => false,
    }
}

// ─── 持久化 ───

/// 台账的键。session 优先 —— 同一项目里开两个会话做两件事是常见操作，
/// 那正是 session 存在的理由。
pub fn key_for(session_id: Option<&str>, workspace: Option<&str>) -> Option<String> {
    let clean = |s: Option<&str>| {
        s.map(str::trim)
            .filter(|x| !x.is_empty())
            .map(|x| x.to_string())
    };
    if let Some(s) = clean(session_id) {
        return Some(format!("s-{}", sanitize(&s)));
    }
    clean(workspace).map(|w| format!("w-{}", sanitize(&w)))
}

/// 键要能当文件名。路径里的 `/` 必须换掉，否则会被当成子目录。
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub fn path_for(config_dir: &Path, key: &str) -> PathBuf {
    config_dir.join("runtime").join("ledgers").join(format!("{key}.json"))
}

pub fn load(config_dir: &Path, key: &str) -> TaskLedger {
    std::fs::read_to_string(path_for(config_dir, key))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// 原子写。**必须 temp + rename** —— 直接写的话，一次崩溃会留下半个 JSON，
/// 而下次 `load` 会静默退回 default，也就是台账凭空消失。
pub fn save(config_dir: &Path, key: &str, ledger: &TaskLedger) -> Result<()> {
    let path = path_for(config_dir, key);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| Error::Io(format!("建 ledgers 目录失败：{e}")))?;
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(ledger)
        .map_err(|e| Error::decode(format!("序列化台账失败：{e}")))?;
    std::fs::write(&tmp, body).map_err(|e| Error::Io(format!("写台账失败：{e}")))?;
    std::fs::rename(&tmp, &path).map_err(|e| Error::Io(format!("提交台账失败：{e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(json: &str) -> String {
        json.to_string()
    }

    #[test]
    fn 复合命令的长堆栈里错误行不能被挤掉() {
        // 模型喜欢一次跑 `node --version && cat x && npm test`。
        // 只取尾部的话，一个长堆栈会把 AssertionError 挤出窗口 ——
        // 于是模型知道"失败了"却不知道失败在哪。
        let mut out = String::from("v25.1.0\n（cat 输出的一大堆内容）\n");
        out.push_str("✖ ttl expires entries after ttlMs\n");
        out.push_str("  AssertionError [ERR_ASSERTION]: expected undefined to equal 1\n");
        out.push_str("      at TestContext.<anonymous> (lru.test.ts:41:5)\n");
        for i in 0..60 {
            out.push_str(&format!("      at internal/frame/{i} (node:internal/x:{i}:1)\n"));
        }
        out.push_str("ℹ tests 6\nℹ pass 5\nℹ fail 1\nexit code 1\n");

        let ex = failure_excerpt(&out, 800);
        // ① 错误身份保住了
        assert!(ex.contains("AssertionError"), "错误行被挤掉了：\n{ex}");
        assert!(ex.contains("lru.test.ts:41"), "出错位置被挤掉了：\n{ex}");
        // ② 末尾的结论也保住了
        assert!(ex.contains("fail 1"), "结论行被挤掉了：\n{ex}");
        assert!(ex.contains("exit code 1"), "退出码被挤掉了：\n{ex}");
        // ③ 压缩要标出来，否则读起来像连续的
        assert!(
            ex.contains("调用栈") || ex.contains("省略"),
            "压缩没标出来：\n{ex}"
        );
        // ④ **额度不许被噪声占掉。** 这一条是看了输出才加的：
        //    折叠之前，800 字符里有 7 行给了 `at internal/frame/53..59`，
        //    而上面三条断言照样全过 —— 断言证明不了「值不值」。
        assert!(
            !ex.contains("internal/frame/5"),
            "深层栈帧还在占额度：\n{ex}"
        );
        let frame_lines = ex.lines().filter(|l| l.trim_start().starts_with("at ")).count();
        assert!(frame_lines <= 3, "保留了 {frame_lines} 行栈帧，太多：\n{ex}");
        assert!(ex.chars().count() <= 900, "超了：{}", ex.chars().count());
    }

    #[test]
    fn 调用栈折叠只保留前两层() {
        let mut lines = vec![
            "AssertionError: nope",
            "    at foo (a.ts:1:1)",
            "    at bar (b.ts:2:2)",
        ];
        let deep: Vec<String> = (0..20)
            .map(|i| format!("    at frame{i} (node:internal/x:{i}:1)"))
            .collect();
        for d in &deep {
            lines.push(d);
        }
        lines.push("exit code 1");

        let out = collapse_frames(&lines);
        let joined = out.join("\n");
        // 第一层是出错位置，必须留
        assert!(joined.contains("at foo (a.ts:1:1)"), "{joined}");
        assert!(joined.contains("另有 20 层调用栈"), "{joined}");
        // 深层的运行时内部帧对定位没帮助
        assert!(!joined.contains("frame19"), "{joined}");
        // 非栈帧的行一律不动
        assert!(joined.contains("AssertionError: nope"));
        assert!(joined.ends_with("exit code 1"));
    }

    #[test]
    fn 短栈不折叠因为折叠反而更长() {
        let lines = vec!["Error: x", "    at a (f:1:1)", "    at b (f:2:2)"];
        assert_eq!(collapse_frames(&lines), lines);
    }

    #[test]
    fn 三种语言的栈帧都认() {
        assert!(is_stack_frame("    at foo (a.ts:1:1)"));
        assert!(is_stack_frame("  File \"x.py\", line 3, in f"));
        assert!(is_stack_frame("  12: 0x104abc - core::panic"));
        // 正文不能被误判成栈帧
        assert!(!is_stack_frame("attempting to read the file"));
        assert!(!is_stack_frame("exit code 1"));
        assert!(!is_stack_frame("ℹ fail 1"));
    }

    #[test]
    fn 错误就在尾部时不要切成两段() {
        let out = "npm test\n✖ 一条用例挂了\nAssertionError: nope\nℹ fail 1\nexit code 1";
        let ex = failure_excerpt(out, 800);
        // 全文没超上限，原样返回
        assert_eq!(ex, out);
        assert!(!ex.contains("省略"));
    }

    #[test]
    fn 没有错误标记时退回纯尾部() {
        // 结论在末尾，取头会正好切掉它 —— 项目里别处的 truncate 都取头
        let out: String = (0..200).map(|i| format!("第 {i} 行普通输出\n")).collect();
        let ex = failure_excerpt(&out, 200);
        assert!(ex.contains("第 199 行"), "尾部没保住：\n{ex}");
        assert!(!ex.contains("第 0 行"));
    }

    #[test]
    fn 系统兜底用用户原话当goal() {
        // 实测 gpt-oss @ Effort=High 下，前置指令和点名兜底都无效（0/3）；
        // 同一个模型 @ Effort=Low 都有效（3/3）。靠措辞只在低思考档成立，
        // 所以载荷最重的字段不能依赖它。
        let mut l = TaskLedger::default();
        l.seed_from_user(
            "在 demo/app 里实现一个 diff 引擎和并排对比视图。四件事：\n             1. src/diff.ts ……\n2. src/diff.test.ts ……",
        );
        assert!(l.goal.starts_with("在 demo/app 里实现"), "{}", l.goal);
        assert!(l.goal_from_system);
        // 只取首段 —— 用户原话常常很长，而 goal 是"一句话说清要什么"
        assert!(!l.goal.contains("src/diff.test.ts"), "{}", l.goal);
        // steps 空着不要紧，注入块照样出得来（意图活着才是重点）
        let r = l.render().unwrap();
        assert!(r.contains("在 demo/app 里实现"));
        assert!(r.contains("系统从你最初那条消息里摘的"), "该说明来源：{r}");
    }

    #[test]
    fn 系统占位可以被模型的正式goal直接替换() {
        // 保护意图不被改写，保护的是**已经确立的**意图，不是一个占位。
        // 模型的措辞通常比用户原话更贴合执行。
        let mut l = TaskLedger::default();
        l.seed_from_user("帮我把 lru.ts 改一改，加上 TTL，然后跑测试");
        let msg = l.apply_plan(
            "给 LRUCache 加 TTL 过期并让 npm test 全绿",
            vec!["改 lru.ts".into(), "跑 npm test".into()],
            None, // **不需要 goal_revision**
        );
        assert_eq!(l.goal, "给 LRUCache 加 TTL 过期并让 npm test 全绿");
        assert!(!l.goal_from_system);
        assert!(msg.contains("替换系统占位"), "{msg}");
        assert!(l.goal_revisions.is_empty(), "占位替换不该记成改写");
        // 替换之后就受保护了
        l.apply_plan("随便改个目标", vec!["x".into()], None);
        assert_eq!(l.goal, "给 LRUCache 加 TTL 过期并让 npm test 全绿");
    }

    #[test]
    fn 兜底不覆盖已有的goal也不接受空输入() {
        let mut l = TaskLedger::default();
        l.apply_plan("模型写的目标", vec!["a".into()], None);
        l.seed_from_user("用户原话");
        assert_eq!(l.goal, "模型写的目标");
        assert!(!l.goal_from_system);

        let mut e = TaskLedger::default();
        e.seed_from_user("   \n  ");
        assert!(e.is_empty(), "空输入不该造出一份空台账");
    }

    #[test]
    fn goal_不能被静默改写() {
        // 目标被悄悄降级是模型多次失败后最常见的自我妥协，而它一旦发生
        // 就没有任何痕迹
        let mut l = TaskLedger::default();
        l.apply_plan("npm test 全绿", vec!["改代码".into()], None);
        assert_eq!(l.goal, "npm test 全绿");

        let msg = l.apply_plan("让大部分测试通过", vec!["改代码".into()], None);
        assert_eq!(l.goal, "npm test 全绿", "目标被静默改写了");
        assert!(msg.contains("goal_revision"), "没告诉模型该怎么改：{msg}");
        assert!(l.goal_revisions.is_empty());

        // 带理由就允许，并留痕
        l.apply_plan(
            "让大部分测试通过",
            vec!["改代码".into()],
            Some("用户同意先合并，剩下一条另开任务"),
        );
        assert_eq!(l.goal, "让大部分测试通过");
        assert_eq!(l.goal_revisions.len(), 1);
        assert!(l.goal_revisions[0].contains("用户同意"));
        // 留痕要进注入块，否则「我一开始要的是什么」只有系统知道
        assert!(l.render().unwrap().contains("目标改写过"));
    }

    #[test]
    fn 重新规划会被计数() {
        let mut l = TaskLedger::default();
        l.apply_plan("g", vec!["a".into(), "b".into()], None);
        assert_eq!(l.plan_revision, 0);
        // 一样的计划不算改
        l.apply_plan("g", vec!["a".into(), "b".into()], None);
        assert_eq!(l.plan_revision, 0);
        // 真改了才计数 —— 反复重新规划本身是个信号
        l.apply_plan("g", vec!["a".into(), "c".into()], None);
        assert_eq!(l.plan_revision, 1);
        assert!(l.render().unwrap().contains("改过 1 次"));
    }

    #[test]
    fn 系统记录不需要模型配合() {
        let mut l = TaskLedger::default();
        l.apply_plan("g", vec!["s".into()], None);

        l.record_tool("view_file", &args(r#"{"path":"src/lru.ts"}"#), true, "", 1);
        l.record_tool("apply_patch", &args(r#"{"path":"src/lru.ts"}"#), true, "", 2);
        l.record_tool(
            "shell_command",
            &args(r#"{"cmd":"npm test"}"#),
            false,
            "✖ 挂了\nexit code 1",
            3,
        );

        // 同一个文件先读后改 → 记成 Modified，不能被 Read 降级
        assert_eq!(l.touched.get("src/lru.ts"), Some(&TouchKind::Modified));
        assert_eq!(l.runs.len(), 1);
        assert!(!l.runs[0].ok);
        assert_eq!(l.last_failure.as_ref().unwrap().target, "npm test");
        assert_eq!(l.rounds, 3);

        let r = l.render().unwrap();
        assert!(r.contains("改过：src/lru.ts"), "{r}");
        assert!(r.contains("npm test ✗"), "{r}");
        assert!(r.contains("最近一次失败"), "{r}");
        // 框架语必须在，否则模型会把台账当成待总结的资料
        assert!(r.contains("不要重新规划"));
        assert!(r.contains("不是你的自述"));
    }

    #[test]
    fn 改过的文件不会被读过的挤掉() {
        let mut l = TaskLedger::default();
        l.apply_plan("g", vec!["s".into()], None);
        // 一个长任务里读过的文件可能几十个，而改过的才是信号
        for i in 0..40 {
            l.record_tool(
                "view_file",
                &args(&format!(r#"{{"path":"read/{i}.ts"}}"#)),
                true,
                "",
                1,
            );
        }
        l.record_tool("apply_patch", &args(r#"{"path":"src/target.ts"}"#), true, "", 2);
        let r = l.render().unwrap();
        assert!(r.contains("src/target.ts"), "改过的被挤掉了：\n{r}");
        assert!(r.contains("及另外"), "没标出省略了多少：\n{r}");
    }

    #[test]
    fn 失败的命令优先保留() {
        let mut l = TaskLedger::default();
        l.apply_plan("g", vec!["s".into()], None);
        for i in 0..20 {
            l.record_tool("shell_command", &args(&format!(r#"{{"cmd":"ok-{i}"}}"#)), true, "", i);
        }
        l.record_tool("shell_command", &args(r#"{"cmd":"npm test"}"#), false, "挂了", 21);
        let r = l.render().unwrap();
        // 成功的命令没什么信息量，失败的是「别再走这条路」的唯一线索
        assert!(r.contains("npm test ✗"), "失败的被挤掉了：\n{r}");
    }

    #[test]
    fn 主参数抽取里命令排在路径前面() {
        // exec_command 同时带 command 和可选的 cwd/path，我们要的是命令本身
        assert_eq!(
            primary_arg(r#"{"command":"npm test","cwd":"/ws"}"#).unwrap(),
            "npm test"
        );
        assert_eq!(primary_arg(r#"{"path":"a.ts"}"#).unwrap(), "a.ts");
        assert_eq!(primary_arg(r#"{"file_path":"b.ts"}"#).unwrap(), "b.ts");
        assert_eq!(primary_arg("不是 JSON"), None);
        assert_eq!(primary_arg(r#"{"path":"  "}"#), None);
    }

    #[test]
    fn mcp_前缀的工具也能归类() {
        let mut l = TaskLedger::default();
        l.apply_plan("g", vec!["s".into()], None);
        l.record_tool(
            "filesystem__write_file",
            &args(r#"{"path":"x.ts"}"#),
            true,
            "",
            1,
        );
        assert_eq!(l.touched.get("x.ts"), Some(&TouchKind::Modified));
    }

    #[test]
    fn 归档之后不再注入但文件还在() {
        let mut l = TaskLedger::default();
        l.apply_plan("g", vec!["s".into()], None);
        assert!(l.render().is_some());
        l.archived = true;
        assert!(l.render().is_none());
        // 空台账也不注入
        assert!(TaskLedger::default().render().is_none());
    }

    #[test]
    fn 键以session优先且能当文件名() {
        // 同一项目里开两个会话做两件事是常见操作
        let k = key_for(Some("sess-1"), Some("/Users/x/p")).unwrap();
        assert!(k.starts_with("s-"), "{k}");
        let w = key_for(None, Some("/Users/x/p")).unwrap();
        assert!(w.starts_with("w-"), "{w}");
        // 路径里的 / 必须换掉，否则会被当成子目录
        assert!(!w.contains('/'), "{w}");
        assert_eq!(key_for(None, None), None);
        assert_eq!(key_for(Some("  "), None), None);
    }

    #[test]
    fn 原子写与读回() {
        let dir = tempfile::tempdir().unwrap();
        let mut l = TaskLedger::default();
        l.apply_plan("目标", vec!["一".into(), "二".into()], None);
        l.record_tool("apply_patch", &args(r#"{"path":"a.ts"}"#), true, "", 1);
        save(dir.path(), "k", &l).unwrap();

        let back = load(dir.path(), "k");
        assert_eq!(back, l);
        // 没有的键读出空台账而不是报错 —— 台账缺失不该让对话失败
        assert!(load(dir.path(), "nope").is_empty());
        // temp 文件不该留下
        assert!(!path_for(dir.path(), "k").with_extension("json.tmp").exists());
    }

    #[test]
    fn 注入进最后一条user消息而不是system() {
        use crate::llm::{Message, Role};
        let mut msgs = vec![
            Message::system("系统提示"),
            Message::user("第一句"),
            Message {
                role: Role::Assistant,
                content: "回答".into(),
                images: vec![],
                tool_calls: vec![],
                tool_call_id: None,
            },
            Message::user("第二句"),
        ];
        assert!(inject(&mut msgs, "<L>台账</L>"));

        // **system 一个字都不能变** —— 变了就是每轮从第 0 个 token 起全部
        // cache miss，而那只会表现成"变慢"，没人会联想到注入位置
        assert_eq!(msgs[0].content, "系统提示");
        // 历史里的那条 user 也不能变，否则前缀会在更前面分叉
        assert_eq!(msgs[1].content, "第一句");
        // 只动最后一条
        assert!(msgs[3].content.starts_with("<L>台账</L>"));
        assert!(msgs[3].content.ends_with("第二句"));
    }

    #[test]
    fn 没有user消息时什么都不做() {
        use crate::llm::Message;
        // 绝不退而求其次塞进 system —— 那正是要避免的事
        let mut msgs = vec![Message::system("只有系统提示")];
        assert!(!inject(&mut msgs, "<L>x</L>"));
        assert_eq!(msgs[0].content, "只有系统提示");
    }

    #[test]
    fn 失败后后续命令成功会清除陈旧的last_failure() {
        let mut l = TaskLedger::default();
        l.apply_plan("g", vec!["s".into()], None);
        l.record_tool("apply_patch", &args(r#"{"path":"a.ts"}"#), true, "", 1);
        l.record_tool("shell_command", &args(r#"{"cmd":"npm test"}"#), false, "挂了", 2);
        assert!(l.last_failure.is_some());

        // 修复后再次运行测试通过
        l.record_tool("shell_command", &args(r#"{"cmd":"npm test"}"#), true, "5 passing", 3);
        assert!(l.runs.last().unwrap().ok);
        assert!(l.last_failure.is_none(), "成功后陈旧的 last_failure 应被清除");
    }

    #[test]
    fn 补丁成功不清掉尚未解决的失败() {
        // 真实序列：测试失败 → 改代码 → 还没重跑就想收尾。
        // 这时失败仍然没有解决，而 last_failure 是模型唯一还留着的
        // 「错在哪」的记录（它进注入块，能活过上下文压缩）。
        let mut l = TaskLedger::default();
        l.apply_plan("g", vec!["s".into()], None);
        l.record_tool(
            "shell_command",
            r#"{"cmd":"npm test"}"#,
            false,
            "✖ ttl expires\n  AssertionError: expected undefined to equal 1\n    at lru.test.ts:41:5\nexit code 1",
            2,
        );
        assert!(l.render().unwrap().contains("AssertionError"), "错误该进注入块");

        // 补丁本身成功，但一次验证都还没跑
        l.record_tool("apply_patch", r#"{"path":"src/lru.ts"}"#, true, "", 3);
        assert!(
            l.last_failure.is_some(),
            "补丁成功把尚未解决的失败清掉了 —— 模型从此不知道错在哪"
        );
        assert!(l.render().unwrap().contains("AssertionError"));

        // 读文件之类更不该清
        l.record_tool("view_file", r#"{"path":"src/lru.ts"}"#, true, "内容", 4);
        assert!(l.last_failure.is_some());

        // **重跑验证通过**才是「解决了」的证据
        l.record_tool("shell_command", r#"{"cmd":"npm test"}"#, true, "5 passing", 5);
        assert!(l.last_failure.is_none(), "验证通过之后该清掉");
        assert!(!l.render().unwrap().contains("AssertionError"));
    }
}
