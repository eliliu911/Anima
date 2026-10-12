//! Approval request data model.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::mcp::{Operation, Risk};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalScope {
    Once,
    Session,
    Workspace,
}

/// 从一条命令里提取**能写进项目白名单**的前缀。
///
/// ## 为什么不能直接把整条命令存进去
///
/// 白名单是**整条命令开头**匹配的（见 `mcp::guard`：`sub == p ||
/// sub.starts_with("{p} ")`）。把 `git log --oneline -20` 整条存进去，
/// 换个参数就不命中了 —— 等于点了「本项目允许」却什么也没记住。
///
/// 所以只取「程序名 + 子命令」这一段：`git log --oneline` → `git log`。
///
/// ## 什么情况下拒绝给前缀
///
/// - **复合命令**（`&&` / `;` / `|` / 换行）：白名单要求链上每一条子命令都命中，
///   把复合命令降成一个前缀会让「后半截」被顺带放行。
/// - **子 shell**（`$(` / 反引号）：`guard` 那边本来就绝不自动放行。
/// - **程序名带路径**：那通常是一次性脚本，写进白名单没有复用价值。
///
/// 拒绝时调用方应当退回「本次会话允许」并把原因说给用户 —— 静默降级
/// 会让下次又来问，而用户以为已经永久授权了。
pub fn grant_prefix(command: &str) -> std::result::Result<String, &'static str> {
    let cmd = command.trim();
    if cmd.is_empty() {
        return Err("命令是空的");
    }
    if cmd.contains("$(") || cmd.contains('`') {
        return Err("命令里有子 shell（$( 或 `），不能加入白名单");
    }
    if cmd.contains("&&") || cmd.contains("||") || cmd.contains(';') || cmd.contains('|')
        || cmd.contains('\n')
    {
        return Err(
            "这是复合命令，加入白名单会顺带放行后半截。命令默认已在工作区根目录执行，通常不需要 cd",
        );
    }

    let mut tokens = cmd.split_whitespace();
    let program = tokens.next().ok_or("命令是空的")?;
    if program.starts_with('-') {
        return Err("命令以标志开头，形状不对");
    }
    if program.contains('/') {
        return Err("程序名带路径，不加入白名单");
    }

    let mut out = String::from(program);
    // 再收最多两个「子命令样」的词：不以 - 开头、不含 / 和 = 。
    //
    // 两个而不是一个：`npm run build` 是三段，只收一个会变成 `npm run` ——
    // 那把所有 script 一起放行了，范围比用户点的那一下大。
    for t in tokens
        .take_while(|t| {
            !t.starts_with('-') && !t.contains('/') && !t.contains('=') && !t.starts_with('$')
        })
        .take(2)
    {
        out.push(' ');
        out.push_str(t);
    }
    Ok(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Allow,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id: String,
    pub tool_name: String,
    pub operation: Operation,
    pub risk: Risk,
    pub reason: String,
    pub arguments_preview: String,
    pub command_preview: Option<String>,
    pub diff_preview: Option<String>,
    pub scope: ApprovalScope,
    /// 点「本项目允许」会写进白名单的那个前缀。`None` = 这条命令没法泛化。
    ///
    /// **必须显示在弹窗上。** 用户授权的范围和他以为的范围不一致，是这类
    /// 对话框最容易出的问题 —— 他点的是「这一条」，实际放行的可能是
    /// 「所有 `npm run *`」。
    #[serde(default)]
    pub grant_prefix: Option<String>,
    /// 给人看的一句话：这次要做什么。**有它就优先显示**，参数 JSON 退到下面。
    /// 「新建 3 个角色：林婉清、皇帝、大臣」比 `{"assets":[{"kind":"character",…}]}` 好认得多。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// 只能「允许一次」：出资产图、生成分镜、出片这类占显卡几十分钟到几小时的工具，每次都要问。
    /// 后端也按这个把会话 / 项目级授权降成一次（弹窗只是不显示那几个选项）
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub once_only: bool,
    pub created_at_ms: u128,
}

impl ApprovalRequest {
    /// 用户选的授权范围，按这个工具允许的收紧：`once_only` 的一律只算这一次 ——
    /// 弹窗虽然不显示那几个选项，但不信前端传来的范围
    pub fn clamp_scope(&self, scope: Option<ApprovalScope>) -> ApprovalScope {
        match scope {
            _ if self.once_only => ApprovalScope::Once,
            Some(s) => s,
            None => ApprovalScope::Once,
        }
    }

    pub fn new(
        tool_name: impl Into<String>,
        operation: Operation,
        risk: Risk,
        reason: impl Into<String>,
        arguments: &Value,
    ) -> Self {
        let tool_name = tool_name.into();
        let (command_preview, diff_preview) = previews_for(&tool_name, arguments);
        let summary = super::studio_provider::approval_summary(&tool_name, arguments);
        let once_only = super::studio_provider::once_only(&tool_name);
        Self {
            id: Uuid::new_v4().to_string(),
            tool_name,
            operation,
            risk,
            reason: reason.into(),
            arguments_preview: preview_json(arguments),
            // 从命令预览里算出前缀，好让弹窗把「将要写进白名单的东西」显示出来
            grant_prefix: command_preview
                .as_deref()
                .and_then(|c| grant_prefix(c).ok()),
            summary,
            once_only,
            command_preview,
            diff_preview,
            scope: ApprovalScope::Once,
            created_at_ms: now_ms(),
        }
    }
}

fn previews_for(tool_name: &str, arguments: &Value) -> (Option<String>, Option<String>) {
    let command_preview = match tool_name {
        "shell_command" => arguments
            .get("command")
            .and_then(|value| value.as_str())
            .map(preview_text),
        "exec_command" => arguments
            .get("cmd")
            .or_else(|| arguments.get("command"))
            .and_then(|value| value.as_str())
            .map(preview_text),
        _ => None,
    };
    let diff_preview = if tool_name == "apply_patch" {
        if let Some(patch) = arguments.get("patch").and_then(|value| value.as_str()) {
            Some(preview_text(patch))
        } else if let (Some(path), Some(search), Some(replace)) = (
            arguments.get("path").and_then(|value| value.as_str()),
            arguments.get("search").and_then(|value| value.as_str()),
            arguments.get("replace").and_then(|value| value.as_str()),
        ) {
            Some(preview_text(&format!(
                "*** Update File: {path}\n--- search\n{search}\n+++ replace\n{replace}"
            )))
        } else {
            None
        }
    } else {
        None
    };
    (command_preview, diff_preview)
}

fn preview_json(value: &Value) -> String {
    let raw = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
    if raw.chars().count() <= 1200 {
        raw
    } else {
        raw.chars().take(1200).collect::<String>() + "..."
    }
}

fn preview_text(value: &str) -> String {
    if value.chars().count() <= 2000 {
        value.to_string()
    } else {
        value.chars().take(2000).collect::<String>() + "..."
    }
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn approval_preview_is_bounded() {
        let args = json!({ "text": "x".repeat(2000) });
        let req = ApprovalRequest::new(
            "write_file",
            Operation::Write,
            Risk::Medium,
            "写入文件",
            &args,
        );
        assert!(req.arguments_preview.chars().count() <= 1203);
    }

    #[test]
    fn 前缀只取程序名加子命令() {
        assert_eq!(grant_prefix("git log --oneline -20").unwrap(), "git log");
        assert_eq!(grant_prefix("npm test").unwrap(), "npm test");
        // 三段的要收全 —— 只收一个会变成 `npm run`，把所有 script 一起放行
        assert_eq!(grant_prefix("npm run build").unwrap(), "npm run build");
        assert_eq!(grant_prefix("cargo check --all-targets").unwrap(), "cargo check");
        assert_eq!(grant_prefix("ls -la").unwrap(), "ls");
        // 路径参数不进前缀
        assert_eq!(grant_prefix("cat src/diff.ts").unwrap(), "cat");
        assert_eq!(grant_prefix("git -C /some/path log").unwrap(), "git");
    }

    #[test]
    fn 复合命令拒绝给前缀() {
        // 白名单要求链上每一条子命令都命中；降成一个前缀会顺带放行后半截
        for bad in [
            "cd /x && npm test",
            "npm test; rm -rf build",
            "cat a | sh",
            "npm test || echo fail",
        ] {
            let err = grant_prefix(bad).unwrap_err();
            assert!(err.contains("复合命令"), "{bad} → {err}");
        }
        // 提示里要给出治根的办法，否则用户不知道该怎么改
        assert!(grant_prefix("cd /x && npm test")
            .unwrap_err()
            .contains("不需要 cd"));
    }

    #[test]
    fn 子shell和带路径的程序名一律拒绝() {
        assert!(grant_prefix("echo $(whoami)").is_err());
        assert!(grant_prefix("echo `whoami`").is_err());
        assert!(grant_prefix("/usr/bin/git log").is_err());
        assert!(grant_prefix("./script.sh").is_err());
        assert!(grant_prefix("   ").is_err());
        assert!(grant_prefix("--help").is_err());
    }

    #[test]
    fn 前缀真的能被白名单匹配上() {
        // **这条是这段代码存在的理由。** 白名单是整条命令开头匹配的
        // （`sub == p || sub.starts_with("{p} ")`），所以提取出的前缀必须
        // 真的能匹配原命令 —— 否则点了「本项目允许」却什么也没记住。
        for cmd in ["git log --oneline", "npm test", "npm run build -- --watch", "ls"] {
            let p = grant_prefix(cmd).unwrap();
            let low = cmd.to_ascii_lowercase();
            let pl = p.to_ascii_lowercase();
            assert!(
                low == pl || low.starts_with(&format!("{pl} ")),
                "前缀 {p:?} 匹配不上原命令 {cmd:?}"
            );
        }
    }
}
