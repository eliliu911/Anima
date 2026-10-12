//! MCP 工具的权限控制。
//!
//! 对应 ARCHITECTURE.md 坑点 6：**绝不能让模型的 tool_call 直接拿到无限制的
//! 系统权限**。这一层是唯一的闸门，所以它的默认值一律往安全侧倒。
//!
//! ## 为什么需要"分类"这一步
//!
//! MCP 协议**不声明**一个工具是读还是写。`tools/list` 只给名字、描述和参数
//! schema。要做 CRUD 粒度的控制，就必须自己判断某个工具属于哪一类。
//!
//! 判断分两级：
//!   1. 配置里针对具体工具名的**显式覆盖** —— 唯一可靠的来源，优先级最高
//!   2. 基于名字/描述关键词的**启发式** —— 会猜错，所以只用于兜底，
//!      且猜不出来时按「写」处理（不是按「读」）
//!
//! 启发式必然有漏网的，所以真正的安全边界是「默认策略」：未知与写操作默认需要
//! 确认、删除默认拒绝。用户可以按 server 放宽，但那是显式的选择。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// 工具操作的风险分类
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    /// 只读：查询、列举、检索
    Read,
    /// 写入：创建、更新、发送
    Write,
    /// 删除：不可逆的数据移除
    Delete,
    /// 执行：shell / 终端 / 任意代码 —— 风险最高，因为它能绕过其它所有分类
    Execute,
}

impl Operation {
    pub fn label(self) -> &'static str {
        match self {
            Operation::Read => "读取",
            Operation::Write => "写入",
            Operation::Delete => "删除",
            Operation::Execute => "执行",
        }
    }
}

/// 对某一类操作的许可
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    /// 直接拒绝，模型连工具声明都看不到
    Deny,
    /// 需要用户逐次确认
    Ask,
    /// 放行
    Allow,
}

/// 一个 server 的权限策略
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerPermissions {
    #[serde(default = "allow")]
    pub read: Access,
    #[serde(default = "ask")]
    pub write: Access,
    #[serde(default = "deny")]
    pub delete: Access,
    #[serde(default = "ask")]
    pub execute: Access,
    /// 按**工具名**覆盖分类判定。启发式猜错时用这个纠正。
    #[serde(default)]
    pub classify: BTreeMap<String, Operation>,
    /// 按**工具名**直接覆盖许可，优先级高于分类策略。
    #[serde(default)]
    pub overrides: BTreeMap<String, Access>,
}

fn allow() -> Access {
    Access::Allow
}
fn ask() -> Access {
    Access::Ask
}
fn deny() -> Access {
    Access::Deny
}

impl Default for ServerPermissions {
    /// 默认值一律往安全侧倒：
    /// 读放行、写要确认、删除直接拒绝、执行要确认。
    ///
    /// 删除默认 `Deny` 而不是 `Ask`，因为不可逆 —— 在语音场景下用户可能来不及
    /// 看清确认框就随口答应了。要用就去配置里显式打开。
    fn default() -> Self {
        Self {
            read: Access::Allow,
            write: Access::Ask,
            delete: Access::Deny,
            execute: Access::Ask,
            classify: BTreeMap::new(),
            overrides: BTreeMap::new(),
        }
    }
}

impl ServerPermissions {
    /// 全部放行。仅用于完全可信的本地 server，配置里要显式选。
    pub fn permissive() -> Self {
        Self {
            read: Access::Allow,
            write: Access::Allow,
            delete: Access::Ask,
            execute: Access::Ask,
            ..Default::default()
        }
    }

    /// 只读。适合接入生产系统时先观察一段。
    pub fn read_only() -> Self {
        Self {
            read: Access::Allow,
            write: Access::Deny,
            delete: Access::Deny,
            execute: Access::Deny,
            ..Default::default()
        }
    }

    fn access_for(&self, op: Operation) -> Access {
        match op {
            Operation::Read => self.read,
            Operation::Write => self.write,
            Operation::Delete => self.delete,
            Operation::Execute => self.execute,
        }
    }

    /// 判定某个工具的许可。
    ///
    /// 顺序：工具级 override → 工具级分类覆盖 → 启发式分类 → 对应类别的策略。
    pub fn decide(&self, tool: &str, description: &str) -> Decision {
        if let Some(access) = self.overrides.get(tool) {
            return Decision {
                operation: self.operation_of(tool, description),
                access: *access,
                reason: DecisionReason::ToolOverride,
            };
        }
        let operation = self.operation_of(tool, description);
        Decision {
            operation,
            access: self.access_for(operation),
            reason: if self.classify.contains_key(tool) {
                DecisionReason::ConfiguredClass
            } else {
                DecisionReason::Heuristic
            },
        }
    }

    fn operation_of(&self, tool: &str, description: &str) -> Operation {
        self.classify
            .get(tool)
            .copied()
            .unwrap_or_else(|| classify(tool, description))
    }
}

/// 判定依据。UI 上要显示出来 —— 用户得知道这个结论是配的还是猜的。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionReason {
    /// 配置里针对该工具直接指定了许可
    ToolOverride,
    /// 配置里指定了该工具的分类
    ConfiguredClass,
    /// 由关键词猜出来的分类 —— **可能不准**
    Heuristic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Decision {
    pub operation: Operation,
    pub access: Access,
    pub reason: DecisionReason,
}

impl Decision {
    /// 是否允许把这个工具声明**暴露给模型**。
    /// Deny 的工具连声明都不给 —— 模型看不到就不会尝试调用，
    /// 比"看得到但调用时被拒"更省一轮往返，也更不容易让模型反复重试。
    pub fn exposed(&self) -> bool {
        self.access != Access::Deny
    }

    pub fn needs_confirmation(&self) -> bool {
        self.access == Access::Ask
    }
}

/// 关键词优先级表。顺序即优先级：先命中的赢。
///
/// **Execute 放在最前**：一个能跑 shell 的工具即使名字里带 "get"，
/// 也必须按执行处理 —— 它能绕过其它所有分类。
/// 工具名的分类关键词。匹配的是**分词后的词元**而不是裸子串 ——
/// 裸子串会把 `output` 里的 `put` 当成写操作（真踩过，见测试）。
const EXECUTE_HINTS: &[&str] = &[
    "exec",
    "execute",
    "shell",
    "bash",
    "zsh",
    "sh",
    "terminal",
    "command",
    "cmd",
    "spawn",
    "eval",
    "script",
    "run",
    "applescript",
    "osascript",
    "process",
];
const DELETE_HINTS: &[&str] = &[
    "delete", "del", "remove", "rm", "destroy", "purge", "drop", "unlink", "truncate", "erase",
    "rmdir", "clear",
];
const WRITE_HINTS: &[&str] = &[
    "create", "update", "write", "insert", "add", "modify", "edit", "set", "post", "put", "patch",
    "upload", "send", "move", "rename", "copy", "install", "commit", "push", "save", "apply",
    "invite", "schedule",
];
const READ_HINTS: &[&str] = &[
    "get",
    "list",
    "read",
    "search",
    "query",
    "find",
    "fetch",
    "describe",
    "show",
    "info",
    "status",
    "inspect",
    "count",
    "diff",
    "view",
    "browse",
    "screenshot",
    "navigate",
    "tree",
    "stat",
    "exists",
    "peek",
    "preview",
    "summary",
    "head",
    "tail",
    "dir",
    "directory",
    "ls",
];

/// 描述里的高风险信号。描述是自然语言，所以这里用**子串**匹配，
/// 并且带上中文 —— 国内自建的 server 描述基本都是中文，
/// 只认英文等于对它们完全没有防护。
const EXECUTE_DESC_HINTS: &[&str] = &[
    "shell",
    "bash",
    "command line",
    "execute",
    "run a command",
    "run command",
    "arbitrary code",
    "subprocess",
    "执行命令",
    "运行命令",
    "执行 shell",
    "运行 shell",
    "终端",
    "命令行",
    "执行脚本",
    "运行脚本",
    "任意代码",
    // 启动应用也是在起进程。只写进**描述**关键词而不加名字关键词 ——
    // "open" 作为名字词元太常见（open_file / open_connection），
    // 加进去会把一堆只读工具误判成执行类
    "launch app",
    "launch an app",
    "open an application",
    "start an app",
    "打开应用",
    "启动应用",
    "打开一个 macos 应用",
    "启动程序",
    "打开程序",
];
const DELETE_DESC_HINTS: &[&str] = &[
    "permanently delete",
    "irreversibl",
    "cannot be undone",
    "删除",
    "移除",
    "清空",
    "销毁",
    "不可恢复",
    "不可逆",
];

/// 把工具名切成词元：`list_directory_with_sizes` → [list, directory, with, sizes]，
/// `getFileInfo` → [get, file, info]。
///
/// 必须分词而不是直接 contains：`output` 含 `put`、`address` 含 `add`、
/// `postgres` 含 `post`，裸子串会把只读工具判成写操作。
fn tokenize(name: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in name.chars() {
        if ch.is_alphanumeric() {
            // camelCase 边界也要切
            if ch.is_uppercase() && !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            current.push(ch.to_ascii_lowercase());
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// 从工具名与描述猜测操作类别。
///
/// **这是启发式，会猜错。** 所以：
///   - 猜不出来时返回 [`Operation::Write`]（当作会改数据），而不是 Read。
///     把未知当只读是最危险的默认值。
///   - 名字的权重高于描述：描述里常有"list all files before deleting"这类
///     混合措辞，按描述判会误判。
pub fn classify(tool: &str, description: &str) -> Operation {
    let tokens = tokenize(tool);
    let has = |hints: &[&str]| tokens.iter().any(|t| hints.contains(&t.as_str()));

    // 名字：按风险从高到低取第一个命中
    let from_name = if has(EXECUTE_HINTS) {
        Some(Operation::Execute)
    } else if has(DELETE_HINTS) {
        Some(Operation::Delete)
    } else if has(WRITE_HINTS) {
        Some(Operation::Write)
    } else if has(READ_HINTS) {
        Some(Operation::Read)
    } else {
        None
    };

    // 描述：只认高风险信号。**并且要能盖过名字的判定** ——
    // 一个叫 get_data 但描述说"执行 shell 命令"的工具必须按执行处理。
    // 早先的实现只在名字判不出来时才看描述，等于把这类工具判成只读，
    // 那是最危险的方向。
    let desc = description.to_ascii_lowercase();
    let from_desc = if EXECUTE_DESC_HINTS.iter().any(|h| desc.contains(h)) {
        Some(Operation::Execute)
    } else if DELETE_DESC_HINTS.iter().any(|h| desc.contains(h)) {
        Some(Operation::Delete)
    } else {
        None
    };

    match (from_name, from_desc) {
        // Operation 的 Ord 就是风险序，取更高的那个
        (Some(a), Some(b)) => a.max(b),
        (Some(a), None) => a,
        (None, Some(b)) => b,
        // 完全猜不出来：按会改数据处理。把未知当只读是最危险的默认值
        (None, None) => Operation::Write,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_by_name_keywords() {
        assert_eq!(classify("list_orders", ""), Operation::Read);
        assert_eq!(classify("create_invoice", ""), Operation::Write);
        assert_eq!(classify("delete_customer", ""), Operation::Delete);
        assert_eq!(classify("run_shell_command", ""), Operation::Execute);
    }

    #[test]
    fn execute_wins_over_read_in_the_same_name() {
        // 能跑 shell 的工具即使带 get 也必须按执行处理
        assert_eq!(classify("get_shell_output", ""), Operation::Execute);
        assert_eq!(classify("terminal_list_files", ""), Operation::Execute);
    }

    #[test]
    fn delete_wins_over_write() {
        assert_eq!(classify("remove_and_update_row", ""), Operation::Delete);
    }

    #[test]
    fn unknown_tool_defaults_to_write_not_read() {
        // 把未知当只读是最危险的默认值
        assert_eq!(classify("frobnicate", ""), Operation::Write);
        assert_eq!(classify("", ""), Operation::Write);
    }

    #[test]
    fn description_only_escalates_never_relaxes() {
        // 描述里的高风险词要能提升风险
        assert_eq!(
            classify("frobnicate", "Executes an arbitrary shell command"),
            Operation::Execute
        );
        assert_eq!(
            classify("frobnicate", "Permanently deletes the record"),
            Operation::Delete
        );
        // 但描述里的 "list" 不足以把未知工具降级成只读
        assert_eq!(
            classify("frobnicate", "Will list all affected rows"),
            Operation::Write
        );
    }

    #[test]
    fn safe_defaults() {
        let p = ServerPermissions::default();
        assert_eq!(p.read, Access::Allow);
        assert_eq!(p.write, Access::Ask);
        // 删除不可逆，默认直接拒绝而不是"问一下"
        assert_eq!(p.delete, Access::Deny);
        assert_eq!(p.execute, Access::Ask);
    }

    #[test]
    fn denied_tools_are_not_exposed_to_the_model() {
        let p = ServerPermissions::default();
        let d = p.decide("delete_everything", "");
        assert_eq!(d.operation, Operation::Delete);
        assert_eq!(d.access, Access::Deny);
        assert!(!d.exposed(), "Deny 的工具连声明都不该给模型看到");
    }

    #[test]
    fn write_requires_confirmation_by_default() {
        let d = ServerPermissions::default().decide("create_order", "");
        assert!(d.needs_confirmation());
        assert!(d.exposed(), "需要确认的工具仍要暴露，只是调用前拦一下");
    }

    #[test]
    fn tool_override_beats_class_policy() {
        let mut p = ServerPermissions::default();
        // 默认删除是 Deny，这里针对具体工具放开
        p.overrides.insert("delete_temp_file".into(), Access::Allow);

        let d = p.decide("delete_temp_file", "");
        assert_eq!(d.access, Access::Allow);
        assert_eq!(d.reason, DecisionReason::ToolOverride);
        // 其它删除工具不受影响
        assert_eq!(p.decide("delete_customer", "").access, Access::Deny);
    }

    #[test]
    fn configured_class_corrects_bad_heuristic() {
        let mut p = ServerPermissions::default();
        // 名字里有 "send" 会被猜成 Write，但它其实只是查询
        assert_eq!(classify("send_query", ""), Operation::Write);

        p.classify.insert("send_query".into(), Operation::Read);
        let d = p.decide("send_query", "");
        assert_eq!(d.operation, Operation::Read);
        assert_eq!(d.access, Access::Allow);
        assert_eq!(d.reason, DecisionReason::ConfiguredClass);
    }

    #[test]
    fn read_only_preset_blocks_everything_mutating() {
        let p = ServerPermissions::read_only();
        for tool in ["create_x", "delete_x", "run_x"] {
            assert!(
                !p.decide(tool, "").exposed(),
                "只读策略下 {tool} 不该被暴露"
            );
        }
        assert!(p.decide("list_x", "").exposed());
    }

    #[test]
    fn heuristic_origin_is_reported_so_ui_can_warn() {
        // 用户需要知道某个判定是配的还是猜的
        let p = ServerPermissions::default();
        assert_eq!(p.decide("list_x", "").reason, DecisionReason::Heuristic);
    }

    #[test]
    fn permissions_deserialize_with_safe_defaults_from_partial_json() {
        // 用户只写一部分字段时，缺的必须补成安全值
        let p: ServerPermissions = serde_json::from_str(r#"{"read":"allow"}"#).expect("应当能补齐");
        assert_eq!(p.write, Access::Ask);
        assert_eq!(p.delete, Access::Deny);
        assert_eq!(p.execute, Access::Ask);
    }

    #[test]
    fn operation_ordering_reflects_risk() {
        // Ord 用于取"最高风险"，顺序必须是 Read < Write < Delete < Execute
        assert!(Operation::Read < Operation::Write);
        assert!(Operation::Write < Operation::Delete);
        assert!(Operation::Delete < Operation::Execute);
    }
}

#[cfg(test)]
mod real_world_tools {
    use super::*;

    /// 用真实 server 的工具名验证启发式分类。
    ///
    /// 这些名字取自 @modelcontextprotocol/server-filesystem 实际导出的 14 个工具。
    /// 启发式最容易在真实命名上失手，而失手的方向要么是「把写当成读」（危险），
    /// 要么是「把读当成写」（只是烦）——这个测试盯的是前者。
    #[test]
    fn filesystem_server_tools_are_classified_sensibly() {
        let cases: &[(&str, Operation)] = &[
            ("read_file", Operation::Read),
            ("read_text_file", Operation::Read),
            ("read_media_file", Operation::Read),
            ("read_multiple_files", Operation::Read),
            ("list_directory", Operation::Read),
            ("list_directory_with_sizes", Operation::Read),
            ("directory_tree", Operation::Read),
            ("search_files", Operation::Read),
            ("get_file_info", Operation::Read),
            ("list_allowed_directories", Operation::Read),
            ("write_file", Operation::Write),
            ("edit_file", Operation::Write),
            ("create_directory", Operation::Write),
            ("move_file", Operation::Write),
        ];
        for (tool, expected) in cases {
            let actual = classify(tool, "");
            assert_eq!(
                actual, *expected,
                "工具 {tool} 应判为 {expected:?}，实际 {actual:?}"
            );
        }
    }

    /// 裸子串匹配的假阳性 —— 这几个真踩过。
    #[test]
    fn substrings_inside_longer_words_do_not_trigger() {
        // put ⊂ output、add ⊂ address、post ⊂ postgres：
        // 都是只读工具，早先的 contains 实现会判成写
        assert_eq!(classify("get_output", ""), Operation::Read);
        assert_eq!(classify("list_addresses", ""), Operation::Read);
        assert_eq!(classify("query_postgres", ""), Operation::Read);
        // camelCase 也要正确分词
        assert_eq!(classify("getFileInfo", ""), Operation::Read);
        assert_eq!(classify("deleteRecord", ""), Operation::Delete);
    }

    /// 描述必须能**盖过**名字的判定。
    ///
    /// 早先的实现只在名字判不出来时才看描述，于是一个叫 get_data、描述里写着
    /// "执行 shell 命令" 的工具会被判成只读 —— 那是最危险的方向。
    #[test]
    fn description_escalates_over_a_read_sounding_name() {
        assert_eq!(
            classify("get_output", "运行 shell 命令并返回输出"),
            Operation::Execute
        );
        assert_eq!(
            classify("list_items", "permanently delete matching items"),
            Operation::Delete
        );
        // 但描述不能**降级**：名字说删，描述听起来只是查询，仍按删处理
        assert_eq!(classify("delete_note", "查询并返回结果"), Operation::Delete);
    }

    /// 中文描述也要认。国内自建的 server 描述基本都是中文，
    /// 只认英文等于对它们完全没有防护。
    #[test]
    fn chinese_descriptions_are_understood() {
        assert_eq!(classify("handle", "在终端里执行脚本"), Operation::Execute);
        assert_eq!(
            classify("cleanup", "清空指定表的全部数据"),
            Operation::Delete
        );
    }

    /// 反向确认：不能因为名字里带 read/list/get 就把写操作放过去。
    #[test]
    fn write_intent_wins_over_read_sounding_words() {
        // move_file 会改变文件位置，"file" 不该让它变成读
        assert_eq!(classify("move_file", "读取并移动文件"), Operation::Write);
        // 名字像读、但描述说明会执行命令 —— 描述只能升级风险
        assert_eq!(
            classify("get_output", "运行 shell 命令并返回输出"),
            Operation::Execute
        );
    }
}

#[cfg(test)]
mod system_server_tools {
    use super::*;

    /// 自带的系统控制 server 导出的三个工具，闸门必须判对。
    ///
    /// 这个 server 是整条工具链里权限最高的一环，分类错了后果最严重 ——
    /// 所以把它的工具名钉在测试里，改动 server 时会立刻发现分类偏了。
    #[test]
    fn system_server_tools_are_classified_correctly() {
        // run_command 必须是执行类：名字里有 command，且描述说明能跑任意命令
        assert_eq!(
            classify(
                "run_command",
                "在 shell 里执行一条命令并返回其标准输出与标准错误"
            ),
            Operation::Execute
        );
        // open_app 也是执行类 —— 它确实在启动进程。放行靠 overrides 单独指定，
        // 而不是靠把它误判成读操作
        assert_eq!(
            classify("open_app", "打开一个 macOS 应用程序"),
            Operation::Execute
        );
        // 系统信息是只读
        assert_eq!(
            classify("get_system_info", "读取系统概况：主机名、系统版本、CPU"),
            Operation::Read
        );
    }

    /// overrides 必须能把单个工具放行，且不影响同类的其它工具。
    #[test]
    fn override_allows_one_tool_without_opening_the_whole_class() {
        let mut permissions = ServerPermissions::default();
        permissions
            .overrides
            .insert("open_app".to_string(), Access::Allow);

        let opened = permissions.decide("open_app", "打开一个 macOS 应用程序");
        assert_eq!(opened.access, Access::Allow);

        // 同为执行类的 run_command 不该被顺带放开
        let shell = permissions.decide("run_command", "在 shell 里执行一条命令");
        assert_eq!(shell.operation, Operation::Execute);
        assert_eq!(
            shell.access,
            Access::Ask,
            "放行 open_app 不该连带放开 shell"
        );
    }
}
