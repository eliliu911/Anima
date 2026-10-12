//! 工具分层：常驻的给完整 schema，其余的**先不给**，模型用 `tool_search` 现取。
//!
//! # 修的是什么
//!
//! 60 个工具的完整 schema 一次性铺开是 **58314 字符 / 约 23000 token**，
//! 而本机模型的窗口是 33024 —— **一句话没说就用掉了七成**。
//! 实测症状是 LM Studio 直接报
//! `request (47079 tokens) exceeds the available context size (33024 tokens)`，
//! 而 Anima 那边只看到一个 HTML 的 500。
//!
//! 用户当时在写小说，而工具表里装着剪映导出、ComfyUI 出图、LSP、
//! 终端会话、worktree 合并 —— 一个都用不上，每轮都在付钱。
//!
//! # 为什么不是让用户写白名单
//!
//! 技能的 `tools:` 白名单确实能收窄，但那是**让用户替每个技能猜一遍**。
//! 分层是结构上就不铺开：常用的直接给，冷门的留个名字，
//! 模型想用时先 `tool_search` 取回 schema。
//!
//! `tool_search` 这个工具**早就在了**，它的描述里写着「当需要……而
//! **当前未直接暴露**时」—— 设计意图一直是分层的，只是没有任何东西
//! 在做「不暴露」这一步，于是它变成了第 61 个工具，非但没省还多占一份。
//!
//! # 解锁是无状态的
//!
//! 模型调过 `tool_search` 之后，那次调用和返回**就在会话历史里**。
//! 下一轮组装请求时扫一遍历史，把提到过的工具名解锁 —— 不需要另存一份
//! 会话状态，也就不存在「状态和历史对不上」这种问题。

/// 常驻工具：**每轮都给完整 schema**。
///
/// 判据是「几乎每一轮都可能用到」，不是「重要」。剪映导出很重要，
/// 但一百轮里用一次，那九十九轮的 schema 就是纯浪费。
///
/// 改这份名单的影响是直接的：多一个就多几百到几千字符 × 每一轮。
pub const ALWAYS_ON: &[&str] = &[
    // 读写文件 —— 任何任务的第一公民
    "view_file",
    "fs_read_text",
    "fs_write_text",
    "fs_list_directory",
    "apply_patch",
    // 找东西
    "workspace_search",
    "search_knowledge",
    "code_outline",
    // 干活
    "shell_command",
    "task_plan",
    // 取回被藏起来的那些。**它必须常驻**，否则冷门工具永远够不着
    "tool_search",
];

/// 这个工具是不是常驻。
pub fn is_always_on(name: &str) -> bool {
    ALWAYS_ON.contains(&name)
}

/// `tool_search` 返回结果里工具名的写法。解锁只认这一种。
const SEARCH_HIT: &str = r#""name": ""#;

/// 从会话历史里认出**已经被 `tool_search` 取出来过**的工具名。
///
/// # 判据必须窄，第一版栽在这儿
///
/// 第一版是「整段历史里出现过这个名字就解锁」。实测下来 **14 个工具
/// 在模型一次都没查过的情况下被解锁了** —— `animate_image`、
/// `generate_image`、`lsp_find_definition`、`invoke_subagent`…
///
/// 原因有两个，都跟「查」无关：
///
/// 1. **system prompt 会介绍「你能做什么」**，里面就带着工具名
/// 2. **调用过的工具，签名会一直留在压缩后的历史里**
///    （`〔已调用工具: filesystem__create_directory 参数: …〕`）
///
/// 本意是「查过就解锁」，实际成了「**提到过就解锁**」。
/// 于是 58 个只砍到 23 个，schema 又涨回 31574 字符。
///
/// 现在只认 `tool_search` 返回体里那种 `"name": "xxx"` 的写法 ——
/// 那是 JSON 序列化出来的形状，散文和调用签名里不会出现。
///
/// **代价说清楚**：模型真的用过某个工具、而那次 `tool_search` 已经滚出
/// 历史窗口时，它得再查一次。可恢复；而误解锁是不可见的浪费。
pub fn unlocked_from_history<'a>(
    haystack: &str,
    candidates: impl Iterator<Item = &'a str>,
) -> Vec<String> {
    candidates
        .filter(|name| !is_always_on(name))
        .filter(|name| haystack.contains(&format!("{SEARCH_HIT}{name}\"")))
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **`tool_search` 必须常驻。** 它是够到其余工具的唯一入口 ——
    /// 把它也藏起来的话，冷门工具就永远出不来了。
    #[test]
    fn 取回工具本身必须常驻() {
        assert!(is_always_on("tool_search"));
    }

    /// **`tool_search` 不能被任何一层过滤掉。**
    ///
    /// 分层之后它是够到那 49 个工具的唯一通路。任何一处把它滤掉，
    /// 那些工具就**永久失联** —— 而表现是「模型说这个做不了」，
    /// 看不出是被过滤了。
    #[test]
    fn 取回工具不被任何过滤名单挡住() {
        for list in [
            crate::agent_runtime::local_tools::HIDDEN_FROM_MODEL_TOOLS,
            crate::agent_runtime::local_tools::CODING_RUNTIME_TOOLS,
        ] {
            assert!(
                !list.contains(&"tool_search"),
                "tool_search 被过滤掉了 —— 那 49 个藏起来的工具会永久失联"
            );
        }
    }

    /// 常驻名单要**够小**。它的每一项都乘以「每一轮」。
    #[test]
    fn 常驻名单不能膨胀() {
        assert!(
            ALWAYS_ON.len() <= 14,
            "常驻工具 {} 个 —— 每多一个就是每轮多几百到几千字符，先想清楚",
            ALWAYS_ON.len()
        );
    }

    #[test]
    fn 名单里没有重复() {
        let mut seen = std::collections::HashSet::new();
        for n in ALWAYS_ON {
            assert!(seen.insert(*n), "{n} 在常驻名单里出现了两次");
        }
    }

    /// 重型工具**不该常驻**：剪映导出、出图、终端会话，
    /// 一百轮里用一次，那九十九轮的 schema 是纯浪费。
    #[test]
    fn 重型工具不在常驻名单里() {
        for n in [
            "cinema_export_jianying",
            "generate_image",
            "generate_video",
            "animate_image",
            "pty_spawn",
            "lsp_find_definition",
            "worktree_merge",
        ] {
            assert!(!is_always_on(n), "{n} 不该常驻");
        }
    }

    /// `tool_search` 返回体里那种写法才算解锁。
    #[test]
    fn 历史里查过的工具会解锁() {
        let history = r#"调用 tool_search 返回：{"name": "generate_image", "description": "..."}"#;
        let all = ["generate_image", "pty_spawn", "fs_read_text"];
        let got = unlocked_from_history(history, all.into_iter());
        assert_eq!(got, vec!["generate_image".to_string()]);
    }

    /// **只是「被提到」不算解锁。**
    ///
    /// 实测栽过：第一版用整段包含判定，结果 14 个工具在模型一次都没查过的
    /// 情况下被解锁 —— 因为 system prompt 会介绍「你能做什么」、
    /// 而调用签名 `〔已调用工具: xxx〕` 也一直留在压缩后的历史里。
    /// 于是 58 个只砍到 23 个。
    #[test]
    fn 提示词里提到和调用过都不算解锁() {
        let all = ["generate_image", "filesystem__create_directory", "lsp_find_definition"];
        for history in [
            // system prompt 介绍能力
            "你可以用 generate_image 出图、用 lsp_find_definition 查定义。",
            // 压缩后的调用签名
            "〔已调用工具: filesystem__create_directory 参数: {\"path\":\"novels\"}〕",
            // 用户随口提到
            "帮我 generate_image 一张封面",
            // **带引号地提到也不算** —— 只有 tool_search 返回体里
            // `"name": "xxx"` 那种写法才算。少了这条约束的话，
            // 模型在正文里写一句 `用 "generate_image" 出图` 就会解锁。
            r#"我打算用 "generate_image" 出一张封面"#,
            r#"{"tool": "lsp_find_definition", "why": "查定义"}"#,
        ] {
            let got = unlocked_from_history(history, all.into_iter());
            assert!(got.is_empty(), "「{history}」不该解锁任何工具，实际：{got:?}");
        }
    }

    /// 名字是前缀关系时不能互相带出来 —— `web_search` 不该解锁 `web_search_x`。
    #[test]
    fn 前缀相同的工具不会被带出来() {
        let history = r#"{"name": "web_search", "description": "..."}"#;
        let got = unlocked_from_history(history, ["web_search_advanced"].into_iter());
        assert!(got.is_empty(), "前缀相同就被带出来了：{got:?}");
    }

    /// **常驻的不进解锁名单** —— 它们本来就在，重复加会发两份同名工具。
    #[test]
    fn 常驻的不会被重复解锁() {
        let history = "fs_read_text tool_search";
        let got = unlocked_from_history(history, ["fs_read_text", "tool_search"].into_iter());
        assert!(got.is_empty(), "常驻工具被重复解锁了：{got:?}");
    }

    #[test]
    fn 没查过就不解锁() {
        let got = unlocked_from_history("随便聊聊天气", ["generate_image"].into_iter());
        assert!(got.is_empty());
    }
}
