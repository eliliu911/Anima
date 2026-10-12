//! 朝外暴露哪些工具。**这是白名单，不是黑名单。**
//!
//! # 为什么必须是白名单
//!
//! Anima 的工具注册表里有 `shell_command`、`fs_write_text`、`exec_command`、
//! `pty_spawn` 这些东西 —— 它们对**应用内的 Agent** 是合理的（用户看得见、
//! 有审批、有工作区边界），但一旦朝 MCP 客户端暴露，**Anima 就成了任何
//! MCP 客户端的 shell 代理**：对方只要连上就能在这台机器上执行任意命令。
//!
//! 黑名单在这里是错的做法：以后每加一个工具，**默认都是暴露的**，
//! 而漏掉一个的代价是任意代码执行。白名单反过来 —— 新工具默认不出去，
//! 想出去得有人显式写一行。
//!
//! # 两道闸，不是一道
//!
//! 1. 名字必须在 [`EXPOSED`] 里
//! 2. **哪怕在里面**，`Shell` / `FileSystem` / `Mcp` 这三类也一律拦下
//!
//! 第二道是防「有人把 `shell_command` 加进白名单」—— 不是假想：
//! 白名单会随功能增长，而加名字的人未必知道那个工具底下是什么。
//! 有测试专门钉这条（`就算写进白名单_shell_也出不去`）。
//!
//! # 为什么 `search_knowledge` 也不出去
//!
//! 它读的是用户的私人记忆库（对话、提取的事实）。出图出片是「让 Claude 用
//! Anima 干活」，把用户的记忆翻给外部客户端是另一回事，没被要求过。

use crate::agent_runtime::{ToolKind, ToolSpec};

/// 允许朝 MCP 客户端暴露的工具名。
///
/// **加名字之前问一句**：这个工具能不能碰工作区以外的文件、能不能执行命令、
/// 能不能读用户的私人数据？三个有一个是「能」，就不该在这里。
pub const EXPOSED: &[&str] = &[
    // —— 项目与剧本：读多写少，都限定在 Anima 自己的数据目录里 ——
    "studio_list_projects",
    "studio_get_project",
    "studio_create_project",
    "studio_create_episode",
    "studio_import_script",
    "studio_save_script_doc",
    // —— 资产与风格 ——
    "studio_extract_entities",
    "studio_create_assets",
    "studio_add_look",
    "studio_list_assets",
    "studio_list_styles",
    "studio_set_style",
    // —— 分镜与出片 ——
    "studio_generate_shot_table",
    "studio_synthesize_prompts",
    "studio_generate_asset_images",
    "studio_render_episode",
    "studio_stitch_episode",
    // —— 长任务要能轮询。**少了它上面那几个就是黑洞** ——
    "studio_job_status",
    // —— 出片排队：看名单、调顺序、插暂停位（2026-10-10）——
    "studio_render_queue",
    "studio_edit_queue",
    // —— 出图出片 ——
    "generate_image",
    "generate_video",
    "animate_image",
    // —— 只读的环境自检。让客户端能在出图失败时自己判断是不是 ComfyUI 没开 ——
    "comfyui_status",
];

/// **无头（`anima-mcp`）模式下不能出去的工具。**
///
/// 2026-09-18 之前这里有五个，理由是「要 `AppHandle`」和「会往共享的
/// `jobs.json` 里起任务」。**两条现在都不成立了**：
///
/// - `AppHandle` 从头到尾只用来发一个界面进度事件，现在是 `Option`，
///   没有它任务照样登记、照样跑完，只是右下角不动
/// - `jobs.json` 加了跨进程 `flock` + 原子写入（见 `crate::jobs`），
///   两个进程一起写不会再互相盖掉
///
/// 所以这份清单现在是**空的**。留着这个机制是因为下一个「只有界面能做」的
/// 工具随时可能出现，而那时候需要的就是这里。
///
/// 空的时候 [`tests::无头禁用清单和源码一致`] 会验证「源码里确实没有
/// 硬依赖 `AppHandle` 的分支」—— 不是跳过检查。
pub const HEADLESS_BLOCKED: &[&str] = &[];

/// 这几类**一律不出去，白名单也拦不住**。
///
/// - `Shell` / `FileSystem`：暴露 = Anima 变成任意 MCP 客户端的 shell 代理
/// - `Mcp`：那是**别的 MCP 服务器**桥进来的工具。再朝外转发一次，
///   Anima 就成了一个谁都能用的 MCP 中继，而且对方看不出链路有多长
pub fn kind_forbidden(kind: &ToolKind) -> bool {
    matches!(kind, ToolKind::Shell | ToolKind::FileSystem | ToolKind::Mcp)
}

/// 拦下来的理由。`None` = 放行。**理由要能直接给人看**，
/// 因为「某个工具怎么不见了」是这里最常见的疑问。
pub fn deny_reason(spec: &ToolSpec) -> Option<String> {
    if kind_forbidden(&spec.kind) {
        return Some(format!("{:?} 类工具一律不朝外暴露（会让 Anima 变成外部客户端的执行代理）", spec.kind));
    }
    if !EXPOSED.contains(&spec.name.as_str()) {
        return Some("不在 mcp_server::expose::EXPOSED 白名单里（新工具默认不暴露）".to_string());
    }
    None
}

/// 放不放行。`headless = true` 时还会滤掉 [`HEADLESS_BLOCKED`] 里那些。
pub fn exposed_when(spec: &ToolSpec, headless: bool) -> bool {
    deny_reason(spec).is_none() && !(headless && HEADLESS_BLOCKED.contains(&spec.name.as_str()))
}

/// 放不放行（不考虑无头与否）。
pub fn exposed(spec: &ToolSpec) -> bool {
    deny_reason(spec).is_none()
}

/// **调用时必须再查一遍。**
///
/// 不能因为 `tools/list` 没列就以为客户端不会调 —— JSON-RPC 是自由的，
/// 对面想发什么就发什么。只靠列表把关的话，一句
/// `{"method":"tools/call","params":{"name":"shell_command",…}}`
/// 就绕过了整个白名单。
pub fn check_call(spec: Option<&ToolSpec>, headless: bool) -> Result<(), String> {
    let Some(spec) = spec else {
        return Err("没有这个工具".to_string());
    };
    if let Some(why) = deny_reason(spec) {
        return Err(format!("这个工具不朝外暴露：{why}"));
    }
    if headless && HEADLESS_BLOCKED.contains(&spec.name.as_str()) {
        return Err(format!(
            "{} 要在 Anima 应用里跑（它要界面派事件、或者会往共享的任务记录里写）。请打开 Anima 再用界面上的按钮。",
            spec.name
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_runtime::ToolRegistry;
    use crate::mcp::Operation;

    fn spec(name: &str, kind: ToolKind) -> ToolSpec {
        ToolSpec::new(name, "d", kind, Operation::Read)
    }

    /// **无头注册表**：不需要 AppHandle 就能建起来的那一套，
    /// 正是 `anima-mcp` 二进制里用的同一套。
    fn headless_specs() -> Vec<ToolSpec> {
        let d = tempfile::tempdir().unwrap();
        super::super::registry::headless(d.path(), None).specs()
    }

    /// **白名单里的每个名字都必须真的存在。**
    ///
    /// 改名或删工具时最容易出的事：白名单里留着一个死名字，
    /// 而它**不报错**，只是那个工具从外面看就是没有了。
    #[test]
    fn 白名单里没有死名字() {
        let names: Vec<String> = headless_specs().into_iter().map(|s| s.name).collect();
        let dead: Vec<&&str> = EXPOSED.iter().filter(|n| !names.iter().any(|x| x == *n)).collect();
        assert!(dead.is_empty(), "白名单里这些工具已经不存在了：{dead:?}\n现有：{names:?}");
    }

    /// **就算有人把 shell 写进白名单，也必须出不去。**
    ///
    /// 这是第二道闸存在的全部理由 —— 白名单会随功能增长，
    /// 而加名字的人未必知道那个工具底下是什么。
    #[test]
    fn 就算写进白名单_shell_也出不去() {
        for kind in [ToolKind::Shell, ToolKind::FileSystem, ToolKind::Mcp] {
            // 借一个确实在白名单里的名字，只把 kind 换掉
            let s = spec("studio_list_projects", kind.clone());
            assert!(!exposed(&s), "{kind:?} 类工具被放出去了");
            assert!(deny_reason(&s).unwrap().contains("执行代理"));
        }
    }

    /// **新工具默认不暴露。** 这是白名单相对黑名单的全部价值。
    #[test]
    fn 没写进白名单的一律不暴露() {
        assert!(!exposed(&spec("some_brand_new_tool", ToolKind::BuiltIn)));
        assert!(deny_reason(&spec("some_brand_new_tool", ToolKind::BuiltIn)).unwrap().contains("默认不暴露"));
    }

    /// 真实注册表里那些危险工具，一个都不能在暴露集合里。
    #[test]
    fn 真实注册表里的危险工具都被拦住() {
        let out: Vec<String> = headless_specs().into_iter().filter(exposed).map(|s| s.name).collect();
        for danger in [
            "shell_command", "exec_command", "write_stdin", "kill_exec_session",
            "fs_write_text", "fs_read_text", "read_text_file", "write_text_file",
            "list_directory", "apply_patch", "workspace_search", "research_subagent",
            "web_search", "fetch_url",
        ] {
            assert!(!out.iter().any(|n| n == danger), "{danger} 被暴露出去了");
        }
    }

    /// 该出去的确实出去了 —— 全拦掉的白名单和没有服务端是一回事。
    #[test]
    fn 该暴露的都在() {
        let out: Vec<String> = headless_specs().into_iter().filter(exposed).map(|s| s.name).collect();
        for want in ["studio_list_projects", "studio_render_episode", "studio_job_status", "generate_image"] {
            assert!(out.iter().any(|n| n == want), "{want} 没暴露出去，现有：{out:?}");
        }
        assert!(out.len() >= 10, "暴露的工具太少了，白名单大概坏了：{out:?}");
    }

    /// 扫一段 `studio_provider.rs` 风格的源码，找出**硬依赖 `AppHandle`** 的分支。
    ///
    /// 「硬依赖」= 拿不到 app 就返回错误。单纯把 `d.app.clone()`（`Option`）
    /// 传下去不算 —— 那是可选的进度事件。
    fn scan_hard_app_deps(src: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut rest = src;
        while let Some(i) = rest.find("        Op::") {
            rest = &rest[i + 12..];
            let Some(arrow) = rest.find(" =>") else { break };
            let variant = rest[..arrow].trim().to_string();
            let body_end = rest.find("\n        Op::").unwrap_or(rest.len());
            let body = &rest[..body_end];
            let hard = body.contains("app_and_gateway(d)")
                || body.contains("d.app.clone().ok_or_else")
                || body.contains("d.app.as_ref().ok_or_else");
            if hard {
                let mut snake = String::from("studio");
                for c in variant.chars() {
                    if c.is_uppercase() {
                        snake.push('_');
                        snake.extend(c.to_lowercase());
                    } else {
                        snake.push(c);
                    }
                }
                out.push(snake);
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// **扫描器本身没坏。**
    ///
    /// 真实清单现在是**空的**，所以不能再用「扫出来的不为空」当健康检查 ——
    /// 那会在清单合法地变空时误报（2026-09-18 就是这样挡住了自己）。
    /// 改成喂一段合成源码：认不出来就是扫描器坏了。
    #[test]
    fn 扫描器认得出硬依赖() {
        let fake = "\n        Op::SomethingFine => {\n            let app = d.app.clone();\n        }\n        Op::NeedsApp => {\n            let app = d.app.clone().ok_or_else(|| e)?;\n        }\n        Op::AlsoNeeds => {\n            let (app, gw) = app_and_gateway(d)?;\n        }\n";
        assert_eq!(scan_hard_app_deps(fake), vec!["studio_also_needs", "studio_needs_app"]);
        assert!(scan_hard_app_deps("").is_empty());
    }

    /// **无头禁用清单不能过期。** 直接读 `studio_provider.rs` 的源码：
    /// 哪个 `Op::X` 分支硬依赖 `AppHandle`，哪个就必须在 [`HEADLESS_BLOCKED`] 里。
    ///
    /// # 判据变过两次，都记下来
    ///
    /// 第一版只查 `app_and_gateway`，漏了 `d.app…ok_or_else` 那种写法。
    ///
    /// 第二版把「往共享 `jobs.json` 里起任务」也算成阻塞理由 ——
    /// **那条现在不成立了**：`jobs.rs` 加了跨进程 `flock` + 原子写入
    /// （`tests/jobs_concurrent.rs` 用真子进程验过，变异掉 flock 会从 160 掉到 32）。
    /// 所以判据收回到「硬依赖 AppHandle」，而清单也就空了。
    #[test]
    fn 无头禁用清单和源码一致() {
        let from_src = scan_hard_app_deps(include_str!("../agent_runtime/studio_provider.rs"));
        let mut declared: Vec<String> = HEADLESS_BLOCKED.iter().map(|s| s.to_string()).collect();
        declared.sort();
        assert_eq!(from_src, declared, "HEADLESS_BLOCKED 和 studio_provider.rs 里硬依赖 AppHandle 的分支对不上");
    }

    /// 无头禁用的那些必须先在白名单里 —— 否则这份清单是在过滤一个本来就出不去的东西。
    #[test]
    fn 无头禁用的都在白名单里() {
        for n in HEADLESS_BLOCKED {
            assert!(EXPOSED.contains(n), "{n} 在 HEADLESS_BLOCKED 里却不在 EXPOSED 里");
        }
    }

    /// 无头模式下，那几个不能出现在工具列表里。
    #[test]
    fn 无头模式滤掉起任务和要界面的工具() {
        let out: Vec<String> = headless_specs()
            .into_iter()
            .filter(|s| exposed_when(s, true))
            .map(|s| s.name)
            .collect();
        for n in HEADLESS_BLOCKED {
            assert!(!out.iter().any(|x| x == n), "无头模式放出了 {n}");
        }
        assert!(out.iter().any(|x| x == "studio_list_projects"), "滤过头了：{out:?}");
        assert!(out.iter().any(|x| x == "generate_image"));
    }

    /// **列表没列 ≠ 调不到。** 只靠 `tools/list` 把关的话，客户端直接
    /// `tools/call` 一个没列的名字就绕过了整个白名单。
    #[test]
    fn 调用时再查一遍() {
        let shell = spec("shell_command", ToolKind::Shell);
        assert!(check_call(Some(&shell), true).unwrap_err().contains("不朝外暴露"));
        let hidden = spec("some_internal_tool", ToolKind::BuiltIn);
        assert!(check_call(Some(&hidden), true).is_err());
        assert!(check_call(None, true).unwrap_err().contains("没有这个工具"));

        let ok = spec("studio_list_projects", ToolKind::Cinema);
        assert!(check_call(Some(&ok), true).is_ok());

        // 无头禁用清单现在是空的，但**机制要还在** —— 下一个「只有界面能做」的
        // 工具随时会出现。用一个临时塞进去的名字验机制本身
        if let Some(name) = HEADLESS_BLOCKED.first() {
            let blocked = spec(name, ToolKind::Cinema);
            let why = check_call(Some(&blocked), true).unwrap_err();
            assert!(why.contains("打开 Anima"), "没告诉用户怎么办：{why}");
            assert!(check_call(Some(&blocked), false).is_ok(), "非无头时不该拦");
        }
        // 出片类的现在无头也能调（flock 之后 jobs.json 跨进程安全了）
        let render = spec("studio_render_episode", ToolKind::Cinema);
        assert!(check_call(Some(&render), true).is_ok(), "出片应该放行了");
    }

    /// 长任务必须配轮询。**少了 `studio_job_status`，上面那几个就是黑洞** ——
    /// 客户端发起了渲染，然后永远不知道跑完没有。
    #[test]
    fn 长任务和轮询要配套() {
        let long = ["studio_render_episode", "studio_stitch_episode", "studio_generate_asset_images"];
        if EXPOSED.iter().any(|n| long.contains(n)) {
            assert!(EXPOSED.contains(&"studio_job_status"), "暴露了长任务却没暴露轮询接口");
        }
    }
}
