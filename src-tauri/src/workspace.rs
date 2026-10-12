//! 项目执行目录 (Workspace Directory) 与脚手架初始化管理系统
//!
//! 提供对标 Claude Code / Codex / Antigravity 的标准工作区规范：
//! 1. 结构化目录：`.anima/` (project.json, rules.md, memory/, knowledge/)
//! 2. 系统级提示词模板：`PROMPT.md`
//! 3. 原生文件夹选择器与 Finder 快速唤起
//! 4. 项目级 RAG 知识库与规则动态加载

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceStatus {
    pub path: String,
    pub is_valid_dir: bool,
    pub is_initialized: bool,
    pub has_prompt: bool,
    pub has_rules: bool,
    pub has_project_meta: bool,
    pub knowledge_doc_count: usize,
    pub prompt_content: Option<String>,
    pub rules_content: Option<String>,
    pub project_meta: Option<serde_json::Value>,
    /// **这一次** init 实际新建了哪些文件（相对工作区）。
    ///
    /// `workspace_init` 对已有文件不覆盖，所以「跑完了」和「补齐了三个文件」
    /// 是两件事 —— 界面上不区分的话，用户点完看不出发生了什么，
    /// 也不知道该不该再点一次。`workspace_check_status` 走的是同一个结构，
    /// 那条路上它恒为空。
    #[serde(default)]
    pub created: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct WorkspaceInitReq {
    pub path: String,
    pub project_name: String,
    pub template_type: Option<String>, // "general" | "drama" | "developer" | "creative"
}

/// PROMPT.md 的位置：根目录优先，其次 `.anima/`。
fn find_prompt(root: &Path) -> Option<PathBuf> {
    [root.join("PROMPT.md"), root.join(".anima/PROMPT.md")]
        .into_iter()
        .find(|f| f.is_file())
}

/// rules.md 的位置：`.anima/rules.md` 优先，其次根目录的 `.rules`。
fn find_rules(root: &Path) -> Option<PathBuf> {
    [root.join(".anima/rules.md"), root.join(".rules")]
        .into_iter()
        .find(|f| f.is_file())
}

/// 单份指令的字符上限。
///
/// 它**每一轮都进上下文**，和滚动摘要一样是固定成本。本地档的历史预算是
/// 48000 字符，两份指令加起来占掉 8000 已经是六分之一 —— 再多就该把内容
/// 挪进知识库让模型按需读，而不是每轮都背一遍。
const MAX_INSTRUCTION_CHARS: usize = 4000;

/// 读取一个工作区的项目指令（`PROMPT.md` + `rules.md`），拼成一条 system 消息。
///
/// **必须在后端读，不能让前端缓存。** 前端只在切项目时调
/// `workspace_check_status`，用户改完 PROMPT.md 不会重新拉 ——
/// 于是文件里写的规矩和模型实际收到的对不上，而且没有任何迹象。
///
/// 在此之前这两份文件只被设置面板读来显示：模板生成的
/// 「Principal 级软件架构师」那套指令躺在磁盘上，**从来没送给过模型**，
/// 所以选 `developer` 模板和选 `general` 对实际对话没有区别。
pub fn project_instructions(workspace_path: &str) -> Option<String> {
    let root = Path::new(workspace_path.trim());
    if workspace_path.trim().is_empty() || !root.is_dir() {
        return None;
    }
    let mut parts = Vec::new();
    for (label, path) in [
        ("项目指令（PROMPT.md）", find_prompt(root)),
        ("项目规则（rules.md）", find_rules(root)),
    ] {
        let Some(text) = path.and_then(|f| std::fs::read_to_string(f).ok()) else {
            continue;
        };
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        // 截断而不是整份丢掉：一份超长的 PROMPT.md 里，开头的身份与原则
        // 通常比末尾的附录重要，留前面比什么都不留有用。
        let clipped: String = if text.chars().count() > MAX_INSTRUCTION_CHARS {
            text.chars()
                .take(MAX_INSTRUCTION_CHARS)
                .chain("\n\n（后续内容因长度限制未载入）".chars())
                .collect()
        } else {
            text.to_string()
        };
        parts.push(format!("# {label}\n\n{clipped}"));
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("\n\n---\n\n"))
}

/// 把一条命令前缀加进项目的免审批白名单。
///
/// ## 为什么必须落盘
///
/// 「本项目允许」原先只写进 `RuntimeService` 的内存 Vec，而那个 service 是
/// **per-session** 的 —— 重启应用或换会话就没了，用户会被重新问一遍，
/// 而且不会说为什么。**按钮承诺了持久，行为没有。**
///
/// 落盘的家就是 `project.json` 的 `terminal_policy.auto_approve_commands` ——
/// 那本来就是「这个项目免审批什么」的唯一真相，而且已经被 `policy.rs` 执行。
/// 写进去之后连审批流程都不会再进入。
///
/// 读-改-写合并，不覆写整个文件（`workspace_init` 现在也是合并式的）。
pub async fn add_auto_approve_prefix(path: &str, prefix: &str) -> Result<bool> {
    let prefix = prefix.trim();
    if prefix.is_empty() {
        return Ok(false);
    }
    let meta_file = PathBuf::from(path).join(".anima/project.json");
    if let Some(parent) = meta_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut meta: serde_json::Value = std::fs::read_to_string(&meta_file)
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if !meta.is_object() {
        meta = serde_json::json!({});
    }

    let list = meta
        .pointer_mut("/permissions/terminal_policy/auto_approve_commands")
        .and_then(|v| v.as_array_mut());
    let mut arr = match list {
        Some(a) => a.clone(),
        None => Vec::new(),
    };
    if arr
        .iter()
        .any(|v| v.as_str().map(str::trim) == Some(prefix))
    {
        return Ok(false); // 已经在了
    }
    arr.push(serde_json::json!(prefix));

    // 逐层补齐路径 —— 半成品的 project.json（只有 permissions 没有
    // terminal_policy）也要能写进去
    let obj = meta.as_object_mut().expect("上面保证是 object");
    let perms = obj
        .entry("permissions")
        .or_insert_with(|| serde_json::json!({}));
    if !perms.is_object() {
        *perms = serde_json::json!({});
    }
    let tp = perms
        .as_object_mut()
        .unwrap()
        .entry("terminal_policy")
        .or_insert_with(|| serde_json::json!({}));
    if !tp.is_object() {
        *tp = serde_json::json!({});
    }
    tp.as_object_mut()
        .unwrap()
        .insert("auto_approve_commands".into(), serde_json::json!(arr));

    std::fs::write(
        &meta_file,
        serde_json::to_string_pretty(&meta).unwrap_or_default(),
    )
    .map_err(|e| Error::Io(format!("写 project.json 失败：{e}")))?;
    tracing::info!(path, prefix, "已加入项目免审批白名单");
    Ok(true)
}

/// 把一个**工具名**加进项目免审批名单。
///
/// 和 [`add_auto_approve_prefix`] 并列：命令类工具走前缀（更窄），
/// 非命令类工具（`apply_patch` / `fs_write_text`）只能按工具名。
pub async fn add_auto_approve_tool(path: &str, tool: &str) -> Result<bool> {
    let tool = tool.trim();
    if tool.is_empty() {
        return Ok(false);
    }
    let meta_file = PathBuf::from(path).join(".anima/project.json");
    if let Some(parent) = meta_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut meta: serde_json::Value = std::fs::read_to_string(&meta_file)
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if !meta.is_object() {
        meta = serde_json::json!({});
    }
    let mut arr: Vec<serde_json::Value> = meta
        .pointer("/permissions/auto_approve_tools")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if arr.iter().any(|v| v.as_str().map(str::trim) == Some(tool)) {
        return Ok(false);
    }
    arr.push(serde_json::json!(tool));

    let obj = meta.as_object_mut().expect("上面保证是 object");
    let perms = obj
        .entry("permissions")
        .or_insert_with(|| serde_json::json!({}));
    if !perms.is_object() {
        *perms = serde_json::json!({});
    }
    perms
        .as_object_mut()
        .unwrap()
        .insert("auto_approve_tools".into(), serde_json::json!(arr));

    std::fs::write(
        &meta_file,
        serde_json::to_string_pretty(&meta).unwrap_or_default(),
    )
    .map_err(|e| Error::Io(format!("写 project.json 失败：{e}")))?;
    tracing::info!(path, tool, "已加入项目免审批工具名单");
    Ok(true)
}

/// 工作区绑到项目上时自动初始化脚手架。
///
/// ## 为什么要自动
///
/// Claude Code / Codex 从来不需要手动 init —— 目录一绑上就有 `.claude/`。
/// 让用户先创建项目、再去设置面板点一个按钮，是把实现细节暴露成了操作步骤。
///
/// 时机是**路径第一次绑上项目**（`create_project` / `update_project`），
/// 不是创建项目时 —— 侧栏创建项目只给名字，路径是后设的。
///
/// 幂等：`.anima/project.json` 已经在就什么都不做。`workspace_init` 本身也
/// 不覆盖已有文件，所以重复调最多是补齐缺的那几个。
pub async fn auto_init_if_needed(path: Option<&str>, project_name: &str) {
    let Some(path) = path.map(str::trim).filter(|p| !p.is_empty()) else {
        return;
    };
    let root = Path::new(path);
    if !root.is_dir() {
        return;
    }
    if root.join(".anima").join("project.json").is_file() {
        return;
    }
    let template = detect_template(root);
    tracing::info!(path, template, "工作区绑定，自动初始化脚手架");
    if let Err(err) = workspace_init(WorkspaceInitReq {
        path: path.to_string(),
        project_name: project_name.to_string(),
        template_type: Some(template.to_string()),
    })
    .await
    {
        // 自动初始化失败不该让「设置工作区」这个动作失败 ——
        // 用户还能在设置面板里手动点
        tracing::warn!(%err, path, "自动初始化失败");
    }
}

/// 按目录内容猜模板。
///
/// **这不是在猜用户意图，是在读文件系统** —— 确定、可查、而且猜错的后果只是
/// 一份可以直接改的 PROMPT.md。不猜的代价是 `developer` 那套永远用不上
/// （界面上原本只有 general 和 drama 两个按钮，developer 够不着）。
fn detect_template(root: &Path) -> &'static str {
    const CODE_MARKERS: &[&str] = &[
        "package.json",
        "Cargo.toml",
        "pyproject.toml",
        "go.mod",
        "pom.xml",
        "build.gradle",
        "Gemfile",
        "composer.json",
        "CMakeLists.txt",
        "tsconfig.json",
    ];
    if CODE_MARKERS.iter().any(|m| root.join(m).is_file()) || root.join(".git").exists() {
        return "developer";
    }
    "general"
}

/// 打开原生系统文件夹选择框（在 macOS 下调用原生选择器）
#[tauri::command]
pub async fn workspace_choose_folder() -> Result<Option<String>> {
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("osascript")
            .args([
                "-e",
                "POSIX path of (choose folder with prompt \"请选择项目执行工作区目录\")",
            ])
            .output();
        if let Ok(o) = output {
            if o.status.success() {
                let path = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if !path.is_empty() {
                    return Ok(Some(path));
                }
            }
        }
    }
    Ok(None)
}

/// 检查指定工作区目录的初始化状态与文件清单
#[tauri::command]
pub async fn workspace_check_status(path: String) -> Result<WorkspaceStatus> {
    let p = PathBuf::from(&path);
    if !p.is_dir() {
        return Ok(WorkspaceStatus {
        created: Vec::new(),
            path,
            is_valid_dir: false,
            is_initialized: false,
            has_prompt: false,
            has_rules: false,
            has_project_meta: false,
            knowledge_doc_count: 0,
            prompt_content: None,
            rules_content: None,
            project_meta: None,
        });
    }

    let anima_dir = p.join(".anima");
    // **看实质，不看目录在不在。**
    //
    // 原先是 `anima_dir.is_dir()`。但 `workspace_save_permissions` 和
    // `workspace_save_files` 都会顺手 `create_dir_all(.anima)` —— 于是「改一下
    // 权限」就让目录存在了，而里面只有 project.json，没有 knowledge、
    // 没有 skills、没有 PROMPT.md。那种半成品被报成「已初始化」之后，
    // 界面上的初始化提示就消失了，而缺的东西一个都没补上。
    let is_initialized = anima_dir.join("project.json").is_file()
        && (find_prompt(&p).is_some() || find_rules(&p).is_some());

    let prompt_content = find_prompt(&p).and_then(|f| std::fs::read_to_string(f).ok());
    let has_prompt = prompt_content.is_some();
    let rules_content = find_rules(&p).and_then(|f| std::fs::read_to_string(f).ok());
    let has_rules = rules_content.is_some();

    // 检查 project.json
    let meta_file = anima_dir.join("project.json");
    let has_project_meta = meta_file.is_file();
    let project_meta = if has_project_meta {
        std::fs::read_to_string(&meta_file)
            .ok()
            .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
    } else {
        None
    };

    // 统计 knowledge/ 目录下的文档数
    let knowledge_dir = anima_dir.join("knowledge");
    let mut knowledge_doc_count = 0;
    if let Ok(entries) = std::fs::read_dir(knowledge_dir) {
        for entry in entries.flatten() {
            if entry.path().is_file() {
                knowledge_doc_count += 1;
            }
        }
    }

    Ok(WorkspaceStatus {
        created: Vec::new(),
        path,
        is_valid_dir: true,
        is_initialized,
        has_prompt,
        has_rules,
        has_project_meta,
        knowledge_doc_count,
        prompt_content,
        rules_content,
        project_meta,
    })
}

/// 一键初始化项目工作区脚手架 (.anima / PROMPT.md / rules.md / knowledge / memory)
#[tauri::command]
pub async fn workspace_init(req: WorkspaceInitReq) -> Result<WorkspaceStatus> {
    let p = PathBuf::from(&req.path);
    std::fs::create_dir_all(&p)
        .map_err(|e| Error::config(format!("无法创建目录 {}: {}", req.path, e)))?;

    let anima_dir = p.join(".anima");
    let knowledge_dir = anima_dir.join("knowledge");
    std::fs::create_dir_all(&knowledge_dir)
        .map_err(|e| Error::config(format!("无法创建 knowledge 目录: {e}")))?;

    // **不建 `.anima/memory/`。** 它曾经在这里被创建，然后全代码库再无第二处
    // 引用 —— 一个永久空目录。记忆和知识库索引都存在应用数据目录的
    // `vectors/`（一个 LanceDB 库、两张表），按 scope 分项目，不落在仓库里。
    //
    // 一个看起来像存储位置、实际什么都不存的目录，会让人以为记忆能跟着仓库走。

    // **没指定模板时沿用已有的那个。**
    //
    // `/init` 这类「补齐脚手架」的入口不该被迫猜模板 —— 一个 developer 项目
    // 被当成 general 重新初始化，生成的 PROMPT.md 就换了套人设。
    // 而这里对已存在的文件是不覆盖的，所以真正的后果更隐蔽：
    // project.json 里的 template 字段被改写，而 PROMPT.md 还是旧的那套，
    // 两者从此不一致。
    let existing_template = std::fs::read_to_string(anima_dir.join("project.json"))
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|v| {
            v.get("template")
                .and_then(|t| t.as_str())
                .map(|s| s.to_string())
        });
    let template = req
        .template_type
        .as_deref()
        .filter(|t| !t.trim().is_empty())
        .or(existing_template.as_deref())
        .unwrap_or("general");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // 这一次实际新建了哪些文件。init 对已有文件不覆盖，所以
    // 「跑完了」和「补齐了三个文件」是两件事 —— 不区分的话用户点完
    // 看不出发生了什么，也不知道该不该再点一次。
    let mut created: Vec<String> = Vec::new();

    // 1. `.anima/project.json` —— **合并，不是覆写。**
    //
    // 原先这里无条件 `fs::write` 整个文件，包括 `permissions`。后果是每次
    // 重跑 init（`/init`、自动初始化、界面上再点一次）都会**把用户改过的
    // 权限全部重置** —— 允许目录、免审批命令白名单，全没了。
    //
    // 而 `workspace_save_permissions` 是读-改-写合并的，两边行为不一致。
    // 现在统一成合并：`name` / `template` 每次更新（那是身份和选择），
    // 其余键只在缺失时补。
    let meta_file = anima_dir.join("project.json");
    let existed = meta_file.is_file();
    let mut meta: serde_json::Value = if existed {
        std::fs::read_to_string(&meta_file)
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or_else(|| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };
    if !meta.is_object() {
        meta = serde_json::json!({});
    }
    {
        let obj = meta.as_object_mut().expect("上面已经保证是 object");
        obj.insert("name".into(), serde_json::json!(req.project_name));
        obj.insert("template".into(), serde_json::json!(template));
        obj.entry("created_at").or_insert(serde_json::json!(now));
        obj.entry("version").or_insert(serde_json::json!("1.0.0"));
        obj.entry("description").or_insert(serde_json::json!(format!(
            "Anima 智能工作区：{}",
            req.project_name
        )));
        obj.entry("permissions").or_insert(serde_json::json!({
            "security_preset": "inherit",
            "file_access": {
                "allow_reads": true,
                "allow_writes": true,
                "allowed_paths": [format!("{}/", req.path)],
                "denied_paths": [".git/", ".env*", "node_modules/", "id_rsa*"]
            },
            "terminal_policy": {
                // **项目的这份会覆盖全局默认**（见 policy.rs 的
                // `command_allow_prefixes: permissions.terminal_policy.auto_approve_commands`），
                // 所以它比 `mcp::guard` 的默认名单窄就等于收紧了权限。
                // 只读的 git 子命令必须都在 —— 少了它们，每次看历史都要点一下审批，
                // 而那会让用户养成"闭眼点允许"的习惯，比多几条只读白名单危险。
                //
                // 加东西进来之前先问一句：这条命令会改文件或改远端吗？
                "auto_approve_commands": [
                    "ls", "cat", "head", "tail", "wc", "pwd",
                    "git status", "git diff", "git log", "git show", "git branch",
                    "npm test", "npm run build", "cargo check", "cargo test", "pytest",
                    "node --test", "npx tsc"
                ],
                "blocked_commands": ["rm -rf /", "sudo", "mkfs", "dd"],
                "sandbox_mode": "standard"
            }
        }));
    }
    let _ = std::fs::write(
        &meta_file,
        serde_json::to_string_pretty(&meta).unwrap_or_default(),
    );
    if !existed {
        created.push(".anima/project.json".into());
    }

    // 2. 生成专业级 PROMPT.md (对标 Claude / Codex / Antigravity 指令范式)
    let prompt_file = p.join("PROMPT.md");
    if !prompt_file.exists() {
        let prompt_content = match template {
            "drama" => format!(
                r#"# {0} — AI Cinema & Short Drama Director Directive

<identity>
You are the Executive Director and Chief Screenwriter for the short-drama project "{0}". You master fast-paced episodic storytelling (vertical 9:16 and widescreen 16:9), suspense cliffhangers, character tension, and AI cinematic production pipelines.
</identity>

## 🎬 Core Directives & Creative Workflow
1. **Hook Within First 3 Seconds**:
   - Every episode must open with an immediate conflict, power shift, or compelling suspense hook to capture viewer attention.
2. **Dynamic Pacing & Emotional Escalation**:
   - Introduce narrative twists and escalating stakes every 30-45 seconds. Conclude episodes on a gripping cliffhanger.
3. **Standardized Cinematic Storyboards**:
   - Deconstruct scripts into explicit shot lists: specify Shot Scale (CU/MS/WS), Camera Movement (Push/Pan/Orbit), Audio & Music cues, and ComfyUI image prompts.
4. **Visual & Character Consistency**:
   - Strictly maintain character facial likeness, costume palettes, lighting moods, and world-building coherence across all generated sequences.

## 📚 Knowledge & Reference
- Character dossiers and story bibles are located in `.anima/knowledge/`. Consult them proactively before generating new scenes.
- Content boundaries and production guidelines adhere to `.anima/rules.md`.
"#,
                req.project_name
            ),
            "developer" => format!(
                r#"# {0} — Principal Software Architect & Pair Programming Directive

<identity>
You are the Principal Software Architect and elite Pair Programming Assistant for the project "{0}". You specialize in modern software engineering, clean architecture, test-driven development, and building robust, maintainable codebases.
</identity>

## 🎯 Core Engineering Directives
1. **Observe Before Action**:
   - Thoroughly inspect relevant source files, analyze dependencies, and trace call chains using workspace tools before proposing or executing code modifications. Avoid baseless assumptions.
2. **Test-Driven & Verifiable Changes**:
   - Ensure every functional iteration or bugfix is accompanied by automated unit/integration tests or verified via command execution.
3. **Documentation & Architecture Integrity**:
   - Preserve comments, type definitions, and docstrings. Proactively update architectural documentation in `.anima/knowledge/` upon significant design changes.
4. **Structured Decision Making**:
   - When encountering complex refactoring or trade-offs, present 2-3 structured options evaluating pros, cons, performance, and maintainability impacts.
"#,
                req.project_name
            ),
            _ => format!(
                r#"# {0} — AI Agent Directives & Project Guidelines

<identity>
You are the dedicated AI Autonomous Agent and collaboration partner for the project "{0}". You understand the project's architecture, workflows, and business objectives. You leverage workspace tools, local knowledge bases, and safe sandboxed execution to deliver reliable, end-to-end assistance.
</identity>

## 🎯 Core Directives & Workflow
1. **Observe Before Action**:
   - Prioritize investigating workspace files and searching `.anima/knowledge/` before modifying code or generating assets.
2. **Architectural Integrity**:
   - Strictly follow existing conventions, naming patterns, and design principles. Minimize unnecessary external dependencies.
3. **Safe & Non-Intrusive Execution**:
   - Preserve existing code comments and functional intent. Ensure every modification is focused, verifiable, and cleanly reversible.
4. **Transparent Structured Options**:
   - When facing architectural decisions or ambiguous requirements, proactively outline 2-3 structured choices for user alignment.

## 📚 Domain Knowledge & Context
- Project domain knowledge and reference materials are stored in `.anima/knowledge/`.
- Operational constraints and safety rules are defined in `.anima/rules.md`.
"#,
                req.project_name
            ),
        };
        let _ = std::fs::write(&prompt_file, prompt_content);
        created.push("PROMPT.md".into());
    }

    // 3. Generate standard .anima/rules.md
    let rules_file = anima_dir.join("rules.md");
    if !rules_file.exists() {
        let rules_content = r#"# Project Rules & Operating Constraints

## 1. Communication & Style
- **Lead with Conclusions**: Present key solutions, findings, and trade-offs concisely and structured.
- **Clickable File Links**: Reference workspace files, symbols, and functions using exact Markdown link syntax.
- **Transparent Progress**: Clearly communicate execution phases, tool outcomes, and verification status.

## 2. Security & Execution Safety
- **No Destructive Operations**: Never run destructive deletions (`rm -rf`), repository resets, or unauthorized formatting overrides without explicit user confirmation.
- **Sandbox Isolation**: Run shell commands inside secure sandboxes, isolating unauthorized network and external system access.
- **Secret Redaction**: Never output or leak `.env` keys, tokens, or credentials in logs, prompts, or artifacts.

## 3. Artifacts & Documentation Standards
- **Standardized Locations**: Store interim artifacts, vector memories, and local search caches under `.anima/`.
- **Documentation as Code**: Keep technical documentation in sync with real implementation. Update `.anima/knowledge/` after significant system changes.
"#;
        let _ = std::fs::write(&rules_file, rules_content);
        created.push(".anima/rules.md".into());
    }

    // 4. Generate .anima/knowledge/architecture.md
    let arch_file = knowledge_dir.join("architecture.md");
    if !arch_file.exists() {
        let arch_content = format!(
            r#"# 🏛️ {} System Architecture & Guidelines

## 1. Project Overview
- **Project Name**: {0}
- **Objective**: High-quality automated AI workflows and robust software engineering.

## 2. Workspace Structure
- Root directory: Core business code and project assets.
- `.anima/`: Project configuration, vector memory database, and domain knowledge base.
- `.anima/knowledge/`: Domain reference specifications, design docs, and semantic retrieval sources.

## 3. Knowledge Ingestion Guide
Place product requirement documents, API specifications, architectural decisions, or character bibles into `.anima/knowledge/`.
Then trigger "Rebuild Vector Index" in Project Settings. The agent will semantically retrieve relevant chunks via `search_knowledge`.
"#,
            req.project_name
        );
        let _ = std::fs::write(&arch_file, arch_content);
        created.push(".anima/knowledge/architecture.md".into());
    }

    // 5. 生成 `.anima/skills/` 里的一份**能用的**示例技能。
    //
    // 这里曾经生成过一份 `project-assistant/SKILL.md`，它双重失效且都不报错：
    // 位置不在任何搜索路径上（技能加载从进程 cwd 往上爬找 `skills/`），
    // 而且 frontmatter 只有 `name` + `description`、缺必填的 `marker`，
    // 就算放对位置也会被 `list_in` 静默跳过。更根本的是它的正文
    // （"分析工作区文件结构、审查代码质量"）和 `PROMPT.md` 说的是同一件事。
    //
    // 现在 `.anima/skills/` 是真的搜索路径（`skill::list_layered`，项目层优先），
    // 所以这里换成一份**体现技能真实价值**的示例：**收窄工具面**。
    // 那是 `PROMPT.md` 做不到的事 —— 后者只是背景约束，改不了模型能调什么。
    // 一个只读的审查角色拿不到 `fs_write_text` / `apply_patch` / `shell_command`，
    // 于是"审查时不小心改了代码"从提醒变成了结构上不可能。
    let skills_dir = anima_dir.join("skills").join("code-reviewer");
    let _ = std::fs::create_dir_all(&skills_dir);
    let skill_file = skills_dir.join("SKILL.md");
    if !skill_file.exists() {
        let skill_content = r#"---
id: code-reviewer
name: Code Reviewer
kind: task
surface: coding
marker: Review
description: Read-only code reviewer for current project changes. Constrained toolset prevents accidental file mutations.
tools:
  - view_file
  - fs_read_text
  - fs_list_directory
  - code_outline
  - workspace_search
  - search_knowledge
  - git_status
---

Review Scope: $ARGUMENTS
(If empty, inspect all files modified in the latest working turn.)

You are in **Read-Only Code Review** mode. Write and execution tools are structurally unavailable to guarantee non-invasive review.
Therefore, do not offer to make edits directly; leave code modifications to the user or an editing agent.

## Review Sequence
1. Check `git_status` to determine exact files changed.
2. Use `code_outline` and `view_file` to read the modified files along with surrounding context lines.
3. Use `search_knowledge` to verify alignment with project guidelines in `.anima/knowledge/`.

## Reporting Guidelines
- **Report Concrete Line-Level Findings**: Identify exact file paths, line numbers, failure triggers, and potential edge cases.
- **Categorize Severity**: Distinguish between Critical Bugs, Performance Bottlenecks, and Style Suggestions.
- **Do Not Dump Raw Code**: Keep feedback concise, actionable, and focused on architectural decisions.
- **State Clearly When No Issues Found**: If the changes are clean, state approval directly without fabricating artificial nitpicks.
"#;
        let _ = std::fs::write(&skill_file, skill_content);
        created.push(".anima/skills/code-reviewer/SKILL.md".into());
    }

    let mut status = workspace_check_status(req.path).await?;
    status.created = created;
    Ok(status)
}

/// 保存修改后的 PROMPT.md 与 rules.md
#[tauri::command]
pub async fn workspace_save_files(
    path: String,
    prompt_content: Option<String>,
    rules_content: Option<String>,
) -> Result<()> {
    let p = PathBuf::from(&path);
    if !p.is_dir() {
        return Err(Error::config(format!("目录不存在: {}", path)));
    }

    if let Some(content) = prompt_content {
        let _ = std::fs::write(p.join("PROMPT.md"), content);
    }

    if let Some(content) = rules_content {
        let anima_dir = p.join(".anima");
        let _ = std::fs::create_dir_all(&anima_dir);
        let _ = std::fs::write(anima_dir.join("rules.md"), content);
    }

    Ok(())
}

/// 直接写入工作区内的任意指定文本文件（支持新建或修改）
#[tauri::command]
pub async fn workspace_write_file(
    workspace_path: String,
    relative_path: String,
    content: String,
) -> Result<()> {
    let ws = PathBuf::from(&workspace_path);
    if !ws.is_dir() {
        return Err(Error::config(format!("工作区目录不存在: {}", workspace_path)));
    }

    let rel = Path::new(&relative_path);
    // 安全防路径穿越
    if relative_path.contains("..") || rel.is_absolute() {
        // 如果给的是绝对路径，验证其是否在工作区内部
        if rel.is_absolute() {
            if !rel.starts_with(&ws) {
                return Err(Error::config("禁止写入工作区外部文件"));
            }
            if let Some(parent) = rel.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::write(rel, content).map_err(|e| Error::config(format!("写入文件失败: {e}")))?;
            return Ok(());
        }
        return Err(Error::config("非法相对路径"));
    }

    let target_path = ws.join(rel);
    if let Some(parent) = target_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&target_path, content).map_err(|e| Error::config(format!("写入文件失败: {e}")))?;
    Ok(())
}

#[derive(serde::Serialize)]
pub struct GitCommitResult {
    pub commit_hash: String,
    pub branch: String,
    pub message: String,
    pub files_committed: Vec<String>,
}

/// 在工作区内执行 Git Commit 提交
#[tauri::command]
pub async fn workspace_commit_file(
    workspace_path: String,
    message: String,
    files: Option<Vec<String>>,
) -> Result<GitCommitResult> {
    let ws = PathBuf::from(&workspace_path);
    if !ws.join(".git").exists() {
        return Err(Error::config("当前工作区不是有效的 Git 仓库"));
    }
    let trimmed_msg = message.trim();
    if trimmed_msg.is_empty() {
        return Err(Error::config("提交信息 (Commit Message) 不能为空"));
    }

    // 1. 获取当前分支名
    let branch = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(&ws)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "main".to_string());

    // 2. 暂存指定文件或全部改动
    let mut add_cmd = std::process::Command::new("git");
    add_cmd.current_dir(&ws).arg("add");
    let committed_files = if let Some(ref target_files) = files {
        if target_files.is_empty() {
            add_cmd.arg(".");
            vec![".".to_string()]
        } else {
            for f in target_files {
                add_cmd.arg(f);
            }
            target_files.clone()
        }
    } else {
        add_cmd.arg(".");
        vec![".".to_string()]
    };

    let add_output = add_cmd.output().map_err(|e| Error::config(format!("执行 git add 失败: {e}")))?;
    if !add_output.status.success() {
        let err = String::from_utf8_lossy(&add_output.stderr);
        return Err(Error::config(format!("git add 失败: {err}")));
    }

    // 3. 执行 Commit
    let commit_output = std::process::Command::new("git")
        .args(["commit", "-m", trimmed_msg])
        .current_dir(&ws)
        .output()
        .map_err(|e| Error::config(format!("执行 git commit 失败: {e}")))?;

    if !commit_output.status.success() {
        let err = String::from_utf8_lossy(&commit_output.stderr);
        let out = String::from_utf8_lossy(&commit_output.stdout);
        // 如果没有需要提交的内容
        if out.contains("nothing to commit") || err.contains("nothing to commit") {
            return Err(Error::config("工作区没有需要提交的未暂存或已修改内容"));
        }
        return Err(Error::config(format!("git commit 失败: {err} {out}")));
    }

    // 4. 获取最新的 commit hash
    let hash = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(&ws)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|_| "HEAD".to_string());

    Ok(GitCommitResult {
        commit_hash: hash,
        branch,
        message: trimmed_msg.to_string(),
        files_committed: committed_files,
    })
}

/// 在系统文件管理器 (Finder / 资源管理器) 中定位并高亮选中文件或打开目录
#[tauri::command]
pub fn workspace_open_in_finder(path: String) -> Result<()> {
    let p = Path::new(&path);
    #[cfg(target_os = "macos")]
    {
        if p.is_file() {
            // open -R 会打开 Finder 窗口并在目录中高亮选中该文件（Reveal in Finder 标准行为）
            let _ = std::process::Command::new("open").arg("-R").arg(p).spawn();
        } else {
            if !p.exists() {
                let _ = std::fs::create_dir_all(p);
            }
            let _ = std::process::Command::new("open").arg(p).spawn();
        }
    }
    #[cfg(target_os = "windows")]
    {
        if p.is_file() {
            let _ = std::process::Command::new("explorer")
                .arg(format!("/select,{}", p.display()))
                .spawn();
        } else {
            if !p.exists() {
                let _ = std::fs::create_dir_all(p);
            }
            let _ = std::process::Command::new("explorer").arg(p).spawn();
        }
    }
    #[cfg(target_os = "linux")]
    {
        let target = if p.is_file() {
            p.parent().unwrap_or(p)
        } else {
            if !p.exists() {
                let _ = std::fs::create_dir_all(p);
            }
            p
        };
        let _ = std::process::Command::new("xdg-open").arg(target).spawn();
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeDocInfo {
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    pub updated_at: u64,
    pub extension: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileAccessPolicy {
    pub allow_reads: bool,
    pub allow_writes: bool,
    pub allowed_paths: Vec<String>,
    pub denied_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalCommandPolicy {
    pub auto_approve_commands: Vec<String>,
    pub blocked_commands: Vec<String>,
    pub sandbox_mode: String, // "standard" | "unsandboxed"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectPermissions {
    pub security_preset: String, // "inherit" | "auto_read" | "strict"
    pub file_access: FileAccessPolicy,
    pub terminal_policy: TerminalCommandPolicy,
    /// 这个项目里**免审批的工具名**（如 `apply_patch`）。
    ///
    /// ## 为什么需要它
    ///
    /// 命令类工具可以按前缀免审批（`auto_approve_commands`），
    /// 但 `apply_patch` / `fs_write_text` 这些**不是命令**，没有可泛化的前缀。
    /// 于是「本项目允许 apply_patch」此前无处可存 —— 只写进内存，重启即失效。
    ///
    /// **免审批不等于免策略。** 工具自己仍然会调 `evaluate_path` /
    /// `evaluate_command`，`deny_paths` 和不可逆命令名单照样生效 ——
    /// 这里跳过的只有「问用户」那一步。
    #[serde(default)]
    pub auto_approve_tools: Vec<String>,
}

/// 列出项目 knowledge/ 目录下的所有文档
#[tauri::command]
pub async fn workspace_list_knowledge_docs(path: String) -> Result<Vec<KnowledgeDocInfo>> {
    let k_dir = PathBuf::from(&path).join(".anima/knowledge");
    if !k_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut list = Vec::new();
    if let Ok(entries) = std::fs::read_dir(k_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() {
                let name = p
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let ext = p
                    .extension()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let meta = entry.metadata().ok();
                let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                let updated = meta
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                list.push(KnowledgeDocInfo {
                    name,
                    path: p.to_string_lossy().to_string(),
                    size_bytes: size,
                    updated_at: updated,
                    extension: ext,
                });
            }
        }
    }
    list.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(list)
}

/// 删除知识库中的指定文档
#[tauri::command]
pub async fn workspace_delete_knowledge_doc(path: String, filename: String) -> Result<()> {
    let target = PathBuf::from(&path)
        .join(".anima/knowledge")
        .join(&filename);
    if target.is_file() {
        std::fs::remove_file(target).map_err(|e| Error::config(format!("删除文档失败: {e}")))?;
    }
    Ok(())
}

/// 获取项目的权限与安全策略配置
#[tauri::command]
pub async fn workspace_get_permissions(path: String) -> Result<ProjectPermissions> {
    let meta_file = PathBuf::from(&path).join(".anima/project.json");
    let fallback = ProjectPermissions {
        security_preset: "inherit".to_string(),
        file_access: FileAccessPolicy {
            allow_reads: true,
            allow_writes: true,
            allowed_paths: vec![format!("{}/", path)],
            denied_paths: vec![
                ".git/".to_string(),
                ".env*".to_string(),
                "node_modules/".to_string(),
            ],
        },
        terminal_policy: TerminalCommandPolicy {
            // 和 `workspace_init` 生成的那份保持一致 —— 两处不同的话，
            // 「有没有 project.json」会悄悄改变可用的命令集
            auto_approve_commands: [
                "ls", "cat", "head", "tail", "wc", "pwd",
                "git status", "git diff", "git log", "git show", "git branch",
                "npm test", "npm run build", "cargo check", "cargo test", "pytest",
                "node --test", "npx tsc",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            blocked_commands: vec![
                "rm -rf /".to_string(),
                "sudo".to_string(),
                "mkfs".to_string(),
                "dd".to_string(),
            ],
            sandbox_mode: "standard".to_string(),
        },
        // 兜底不预设任何免审批工具：没有 project.json 时宁可多问几次
        auto_approve_tools: Vec::new(),
    };

    if !meta_file.is_file() {
        return Ok(fallback);
    }

    let content = std::fs::read_to_string(&meta_file).unwrap_or_default();
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&content) {
        if let Some(perm) = val.get("permissions") {
            if let Ok(parsed) = serde_json::from_value::<ProjectPermissions>(perm.clone()) {
                return Ok(parsed);
            }
        }
    }

    Ok(fallback)
}

/// 保存项目的权限与安全策略配置
#[tauri::command]
pub async fn workspace_save_permissions(
    path: String,
    permissions: ProjectPermissions,
) -> Result<()> {
    let meta_file = PathBuf::from(&path).join(".anima/project.json");
    if !meta_file.exists() {
        let _ = std::fs::create_dir_all(meta_file.parent().unwrap());
    }

    let mut current_val: serde_json::Value = if meta_file.is_file() {
        let content = std::fs::read_to_string(&meta_file).unwrap_or_default();
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    current_val["permissions"] = serde_json::to_value(&permissions).unwrap_or_default();
    current_val["updated_at"] = serde_json::json!(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs());

    let _ = std::fs::write(
        &meta_file,
        serde_json::to_string_pretty(&current_val).unwrap_or_default(),
    );
    Ok(())
}

/// 重建项目知识库的向量索引。返回**真实**索引到的块数。
///
/// 在此之前这个函数是这样的：数一下 `.anima/knowledge/` 里有几个文件、
/// `sleep(300ms)` 装作在忙、返回文件数。而前端据此报「⚡ 向量知识库重构完成！
/// 已建立 N 篇文档语义索引」—— 什么都没索引。
///
/// 现在返回的是入库的**块**数（一份文档按标题切多块），所以这个数字通常
/// 大于文件数。它来自 LanceDB 的实际行数，索引失败时会报错而不是给个好看的数。
#[tauri::command]
pub async fn workspace_reindex_memory(
    state: tauri::State<'_, crate::AppState>,
    path: String,
) -> Result<usize> {
    let Some(memory) = crate::commands_voice::ensure_memory(&state).await else {
        return Err(Error::config(
            "记忆系统未启用或嵌入后端不可用，无法建立知识库索引。\
             检查设置里的嵌入后端（默认 ollama / bge-m3），或先打开记忆开关。",
        ));
    };
    memory.reindex_knowledge(&path).await
}

/// 某个项目已索引多少块。**必须来自库本身，不能是"数了几个文件"。**
///
/// 界面上「已索引 N 块」和「目录里有 N 个文件」是两个不同的数字，
/// 后者在索引失败时依然好看 —— 而那正是最需要被看见的时候。
#[tauri::command]
pub async fn workspace_knowledge_count(
    state: tauri::State<'_, crate::AppState>,
    path: String,
) -> Result<usize> {
    let Some(memory) = crate::commands_voice::ensure_memory(&state).await else {
        return Ok(0);
    };
    memory.knowledge_count(&path).await.or(Ok(0))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceFileResult {
    pub resolved_path: String,
    pub relative_path: String,
    pub content: String,
    pub is_binary: bool,
    pub media_type: Option<String>, // "image" | "audio" | "video" | "binary"
    pub data_url: Option<String>,
}

fn detect_media_type(ext: &str) -> Option<(&'static str, &'static str)> {
    match ext.to_lowercase().as_str() {
        "png" => Some(("image", "image/png")),
        "jpg" | "jpeg" => Some(("image", "image/jpeg")),
        "gif" => Some(("image", "image/gif")),
        "webp" => Some(("image", "image/webp")),
        "svg" => Some(("image", "image/svg+xml")),
        "ico" => Some(("image", "image/x-icon")),
        "bmp" => Some(("image", "image/bmp")),
        "mp3" => Some(("audio", "audio/mpeg")),
        "wav" => Some(("audio", "audio/wav")),
        "ogg" => Some(("audio", "audio/ogg")),
        "m4a" => Some(("audio", "audio/mp4")),
        "mp4" => Some(("video", "video/mp4")),
        "webm" => Some(("video", "video/webm")),
        _ => None,
    }
}

/// 读取工作区中的指定文件内容（供右侧代码查看器使用，支持文本解析与图片/音视频预览）
#[tauri::command]
pub async fn workspace_read_file(
    workspace_path: Option<String>,
    file_path: String,
) -> Result<WorkspaceFileResult> {
    let has_custom_workspace = workspace_path.is_some();
    let ws_root = workspace_path
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let target_path = if Path::new(&file_path).is_absolute() && Path::new(&file_path).is_file() {
        PathBuf::from(&file_path)
    } else if let Some(resolved) = resolve_workspace_file(&ws_root, &file_path) {
        resolved
    } else {
        // 如果未指定工作区或请求的是默认 README 但未找到，提供友好的工作区状态说明而不是抛红报错
        if !has_custom_workspace || file_path == "README.md" {
            let note = format!(
                "# Anima Code Viewer\n\n当前尚未绑定独立项目工作区，或未在 `{}` 中检索到 `{}`。\n\n- 您可以在左侧侧边栏切换或新建 Project；\n- 点击对话气泡中任意代码路径或补丁，即可在此并列查看多文件。",
                ws_root.display(),
                file_path
            );
            return Ok(WorkspaceFileResult {
                resolved_path: file_path.clone(),
                relative_path: file_path,
                content: note,
                is_binary: false,
                media_type: None,
                data_url: None,
            });
        }

        return Err(Error::runtime(format!(
            "文件不存在: {} (在工作区 {} 及其子目录中未检索到匹配文件)",
            file_path,
            ws_root.display()
        )));
    };

    let ext = target_path
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();

    let relative_path = target_path
        .strip_prefix(&ws_root)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| file_path.clone());

    // 检查是否为已知媒体类型（图片、音频、视频）
    if let Some((kind, mime)) = detect_media_type(&ext) {
        use base64::Engine;
        let bytes = std::fs::read(&target_path)
            .map_err(|e| Error::runtime(format!("读取文件失败 ({}): {e}", target_path.display())))?;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        let data_url = format!("data:{mime};base64,{b64}");

        return Ok(WorkspaceFileResult {
            resolved_path: target_path.to_string_lossy().to_string(),
            relative_path,
            content: format!("[{} preview]", kind),
            is_binary: true,
            media_type: Some(kind.to_string()),
            data_url: Some(data_url),
        });
    }

    // 尝试以 UTF-8 文本读取
    match std::fs::read_to_string(&target_path) {
        Ok(content) => Ok(WorkspaceFileResult {
            resolved_path: target_path.to_string_lossy().to_string(),
            relative_path,
            content,
            is_binary: false,
            media_type: None,
            data_url: None,
        }),
        Err(_) => {
            // 二进制非媒体文件（如 .zip, .exe, .bin 等）
            let metadata = std::fs::metadata(&target_path).ok();
            let size = metadata.map(|m| m.len()).unwrap_or(0);
            Ok(WorkspaceFileResult {
                resolved_path: target_path.to_string_lossy().to_string(),
                relative_path,
                content: format!("Binary file ({} bytes)", size),
                is_binary: true,
                media_type: Some("binary".to_string()),
                data_url: None,
            })
        }
    }
}

fn resolve_workspace_file(workspace_root: &Path, query_path: &str) -> Option<PathBuf> {
    let clean = query_path
        .trim()
        .trim_start_matches("./")
        .trim_start_matches('/');
    let direct = workspace_root.join(clean);
    if direct.is_file() {
        return Some(direct);
    }

    // 常用子目录优先探测
    let candidates = [
        workspace_root.join("docs").join(clean),
        workspace_root.join("src").join(clean),
        workspace_root.join("src-tauri").join(clean),
        workspace_root.join("src-tauri/src").join(clean),
        workspace_root.join("public").join(clean),
    ];
    for cand in candidates {
        if cand.is_file() {
            return Some(cand);
        }
    }

    // 递归查找（跳过 node_modules, target, .git, dist）
    fn walk_find(dir: &Path, target: &str, depth: usize) -> Option<PathBuf> {
        if depth > 6 {
            return None;
        }
        let entries = std::fs::read_dir(dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if name == "node_modules" || name == "target" || name == ".git" || name == "dist" {
                continue;
            }
            if path.is_file() {
                if name == target || path.ends_with(target) {
                    return Some(path);
                }
            } else if path.is_dir() {
                if let Some(found) = walk_find(&path, target, depth + 1) {
                    return Some(found);
                }
            }
        }
        None
    }

    if let Some(found) = walk_find(workspace_root, clean, 0) {
        return Some(found);
    }

    // 如果主工作区没找到，尝试在活跃的沙箱目录 (.anima/worktrees/*) 中探测
    let worktrees_dir = workspace_root.join(".anima/worktrees");
    if worktrees_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(worktrees_dir) {
            for entry in entries.flatten() {
                let wt_path = entry.path();
                if wt_path.is_dir() {
                    let direct_wt = wt_path.join(clean);
                    if direct_wt.is_file() {
                        return Some(direct_wt);
                    }
                    if let Some(found) = walk_find(&wt_path, clean, 0) {
                        return Some(found);
                    }
                }
            }
        }
    }

    None
}



#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileTreeNode {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size_bytes: u64,
    pub extension: Option<String>,
    pub children: Option<Vec<FileTreeNode>>,
    /// 这个目录**还有内容没走下去**（撞到了深度上限）。
    ///
    /// 没有这一位的话，`children: None`（没走）和 `children: []`（真的空）
    /// 在界面上长得一模一样 —— 都是展开之后什么都没有。用户看到的是
    /// 「第 5 层以下的东西不见了」，而且不报任何错，正是这个仓库最典型的
    /// 失败形状。前端拿它决定要不要按需再取一层。
    #[serde(default)]
    pub truncated: bool,
}

/// 单次请求最多往下走几层。
///
/// **不是"最多只能看到五层"** —— 超出的目录会带 `truncated: true` 回来，
/// 前端展开它时用 `sub_path` 再请求一次，深度从那里重新起算。
/// 所以整棵树没有深度上限，只是分批取。
///
/// 一次取太深的代价是实打实的：`novels/` 这种目录一次全展开要遍历上千个
/// 文件，而用户只想看其中一个。
const DEFAULT_TREE_DEPTH: usize = 4;
const MAX_TREE_DEPTH: usize = 8;

/// 不进树的目录/文件。构建产物和版本库内部占的条目数远多于源码本身。
const IGNORED_ENTRIES: &[&str] = &[
    ".git",
    ".svn",
    ".hg",
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    ".next",
    ".nuxt",
    ".turbo",
    ".cache",
    "__pycache__",
    ".pytest_cache",
    ".venv",
    "venv",
    ".DS_Store",
    "Thumbs.db",
];

/// 这个条目要不要进树。
fn tree_entry_visible(name: &str) -> bool {
    if name.starts_with('.') && name != ".anima" && name != ".env.example" {
        return false;
    }
    !IGNORED_ENTRIES.contains(&name)
}

/// 这个目录下**还有没有**能进树的条目。
///
/// 只用在深度截断的那一刻，用来区分「没走下去」和「真的空」——
/// 空目录标成 `truncated` 的话，前端会去取一次必然空手而归的请求，
/// 而且展开箭头会一直显示"还有内容"。
fn dir_has_visible_entries(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|e| {
        tree_entry_visible(&e.file_name().to_string_lossy())
    })
}

fn build_tree(
    base_root: &Path,
    current_dir: &Path,
    current_depth: usize,
    max_depth: usize,
) -> Vec<FileTreeNode> {
    let Ok(entries) = std::fs::read_dir(current_dir) else {
        return Vec::new();
    };

    let mut nodes = Vec::new();

    for entry in entries.flatten() {
        let name_str = entry.file_name().to_string_lossy().to_string();
        if !tree_entry_visible(&name_str) {
            continue;
        }

        let path = entry.path();
        let is_dir = path.is_dir();
        let rel_path = path
            .strip_prefix(base_root)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| name_str.clone());

        let meta = entry.metadata().ok();
        let size_bytes = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let ext = if !is_dir {
            path.extension().map(|e| e.to_string_lossy().to_string())
        } else {
            None
        };

        let mut truncated = false;
        let children = if is_dir {
            if current_depth < max_depth {
                Some(build_tree(base_root, &path, current_depth + 1, max_depth))
            } else {
                // 走到上限了。**要区分「没走」和「真的空」** —— 前者前端
                // 展开时再取一层，后者直接显示成空目录
                truncated = dir_has_visible_entries(&path);
                None
            }
        } else {
            None
        };

        nodes.push(FileTreeNode {
            name: name_str,
            path: rel_path,
            is_dir,
            size_bytes,
            extension: ext,
            children,
            truncated,
        });
    }

    // 目录排在前面，同类按字母序排序
    nodes.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
    });

    nodes
}

/// 递归获取项目工作区的目录文件树（供右侧 Overview 与文件浏览器使用）。
///
/// `sub_path` 指定从哪个子目录开始 —— 深度**从那里重新起算**，
/// 所以前端展开一个 `truncated` 的目录时，传它的路径再取一次就行。
#[tauri::command]
pub async fn workspace_list_directory_tree(
    workspace_path: Option<String>,
    sub_path: Option<String>,
    max_depth: Option<usize>,
) -> Result<Vec<FileTreeNode>> {
    let ws_root = workspace_path
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let target_dir = if let Some(sub) = sub_path {
        let clean = sub.trim().trim_start_matches("./").trim_start_matches('/');
        ws_root.join(clean)
    } else {
        ws_root.clone()
    };

    if !target_dir.is_dir() {
        return Ok(Vec::new());
    }

    let depth_limit = max_depth.unwrap_or(DEFAULT_TREE_DEPTH).min(MAX_TREE_DEPTH);

    // **路径一律相对 `ws_root`**，即使这次只取某个子目录 —— 前端拿它当
    // key、也拿它回传做下一次的 `sub_path`，相对根才对得上。
    Ok(build_tree(&ws_root, &target_dir, 1, depth_limit))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceGitFileChange {
    pub path: String,
    pub status: String, // "modified" | "added" | "deleted" | "untracked" | "renamed"
    pub additions: usize,
    pub deletions: usize,
    pub is_staged: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceGitStatusResult {
    pub is_git: bool,
    pub branch: String,
    pub clean: bool,
    pub changes: Vec<WorkspaceGitFileChange>,
}

/// 扫描项目工作区中所有未提交的代码变更（git status + git diff）
#[tauri::command]
pub async fn workspace_git_status(
    workspace_path: Option<String>,
) -> Result<WorkspaceGitStatusResult> {
    let ws_root = workspace_path
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    if !ws_root.join(".git").exists() {
        return Ok(WorkspaceGitStatusResult {
            is_git: false,
            branch: String::new(),
            clean: true,
            changes: Vec::new(),
        });
    }

    // 1. 获取当前分支名
    let branch_out = std::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(&ws_root)
        .output();
    let branch = branch_out
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "main".to_string());

    // 2. 获取 git status --porcelain=v1 -uall
    let status_out = std::process::Command::new("git")
        .args(["status", "--porcelain=v1", "-uall"])
        .current_dir(&ws_root)
        .output();

    let mut changes = Vec::new();

    if let Ok(out) = status_out {
        let stdout = String::from_utf8_lossy(&out.stdout);
        for line in stdout.lines() {
            if line.len() < 4 {
                continue;
            }
            let index_status = line.chars().next().unwrap_or(' ');
            let worktree_status = line.chars().nth(1).unwrap_or(' ');
            let file_rel = line[3..].trim().to_string();

            // 忽略 .anima 内部的临时缓存/元数据
            if file_rel.starts_with(".anima/") && !file_rel.starts_with(".anima/skills/") {
                continue;
            }

            let (kind, is_staged) = match (index_status, worktree_status) {
                ('?', '?') => ("untracked", false),
                ('A', _) => ("added", true),
                ('M', _) => ("modified", true),
                ('D', _) => ("deleted", true),
                ('R', _) => ("renamed", true),
                (_, 'M') => ("modified", false),
                (_, 'D') => ("deleted", false),
                (_, 'A') => ("added", false),
                _ => ("modified", false),
            };

            // 获取行增删估计
            let mut additions = 0;
            let mut deletions = 0;

            if kind == "untracked" || kind == "added" {
                if let Ok(content) = std::fs::read_to_string(ws_root.join(&file_rel)) {
                    additions = content.lines().count();
                }
            } else {
                let diff_out = std::process::Command::new("git")
                    .args(["diff", "--numstat", "HEAD", "--", &file_rel])
                    .current_dir(&ws_root)
                    .output();
                if let Ok(dout) = diff_out {
                    let dstr = String::from_utf8_lossy(&dout.stdout);
                    if let Some(first_line) = dstr.lines().next() {
                        let parts: Vec<&str> = first_line.split_whitespace().collect();
                        if parts.len() >= 2 {
                            additions = parts[0].parse::<usize>().unwrap_or(0);
                            deletions = parts[1].parse::<usize>().unwrap_or(0);
                        }
                    }
                }
            }

            changes.push(WorkspaceGitFileChange {
                path: file_rel,
                status: kind.to_string(),
                additions,
                deletions,
                is_staged,
            });
        }
    }

    let is_clean = changes.is_empty();

    Ok(WorkspaceGitStatusResult {
        is_git: true,
        branch,
        clean: is_clean,
        changes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ws(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (rel, body) in files {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        }
        dir
    }

    fn instructions(dir: &tempfile::TempDir) -> Option<String> {
        project_instructions(&dir.path().to_string_lossy())
    }

    #[test]
    fn 没有工作区或没有指令文件时不注入() {
        assert_eq!(project_instructions(""), None);
        assert_eq!(project_instructions("   "), None);
        assert_eq!(project_instructions("/nope/does/not/exist"), None);
        assert_eq!(instructions(&ws(&[])), None, "空目录不该注入空 system 消息");
    }

    #[test]
    fn 空白内容当作没有() {
        // 模板生成过但用户清空了 —— 注入一条只有标题的 system 消息纯属噪声
        assert_eq!(instructions(&ws(&[("PROMPT.md", "   \n\n  ")])), None);
    }

    #[test]
    fn prompt_和_rules_都会带上() {
        let d = ws(&[
            ("PROMPT.md", "你是架构师"),
            (".anima/rules.md", "禁止直接改 main"),
        ]);
        let got = instructions(&d).expect("应当有内容");
        assert!(got.contains("你是架构师"), "{got}");
        assert!(got.contains("禁止直接改 main"), "{got}");
        // PROMPT 在前：它是身份，rules 是约束
        assert!(
            got.find("你是架构师") < got.find("禁止直接改 main"),
            "{got}"
        );
    }

    #[test]
    fn 根目录的_prompt_优先于_anima_下的() {
        let d = ws(&[
            ("PROMPT.md", "根目录版本"),
            (".anima/PROMPT.md", "anima 版本"),
        ]);
        let got = instructions(&d).unwrap();
        assert!(got.contains("根目录版本"), "{got}");
        assert!(!got.contains("anima 版本"), "{got}");
    }

    #[test]
    fn 只有_anima_下有也能找到() {
        let d = ws(&[(".anima/PROMPT.md", "只在 anima 里")]);
        assert!(instructions(&d).unwrap().contains("只在 anima 里"));
    }

    #[test]
    fn 改完文件下一轮立刻生效() {
        // **这是把读取放在后端的全部理由。** 前端只在切项目时调
        // workspace_check_status，缓存下来的话用户改完 PROMPT.md 得重新切项目
        // 才生效，而且不生效时没有任何迹象。
        let d = ws(&[("PROMPT.md", "第一版")]);
        assert!(instructions(&d).unwrap().contains("第一版"));
        std::fs::write(d.path().join("PROMPT.md"), "第二版").unwrap();
        let got = instructions(&d).unwrap();
        assert!(got.contains("第二版"), "{got}");
        assert!(!got.contains("第一版"), "{got}");
    }

    #[test]
    fn 超长指令截断而不是整份丢掉() {
        // 一份超长 PROMPT.md 里，开头的身份与原则通常比末尾的附录重要，
        // 留前面比什么都不留有用。
        let long = format!("开头很重要{}结尾", "填".repeat(MAX_INSTRUCTION_CHARS * 2));
        let d = ws(&[("PROMPT.md", long.as_str())]);
        let got = instructions(&d).unwrap();
        assert!(got.contains("开头很重要"), "开头必须保留");
        assert!(!got.contains("结尾"), "超出部分应当被截掉");
        assert!(
            got.contains("未载入"),
            "要告诉模型有内容被截掉了：{}",
            &got[..80]
        );
        // 截断后的总量必须真的受控 —— 它每轮都进上下文
        assert!(
            got.chars().count() < MAX_INSTRUCTION_CHARS + 200,
            "实际 {} 字符",
            got.chars().count()
        );
    }

    /// init 生成的技能**必须真的能被加载**。
    ///
    /// 这条测试的存在理由很具体：上一版生成的 `project-assistant/SKILL.md`
    /// 缺 `marker` 字段，而 `list_in` 对解析失败的技能是**静默跳过**的 ——
    /// 于是那份文件在磁盘上、看起来像配置、用户会去改它，而它从来没被加载过，
    /// 一行报错都没有。走一遍生产路径（init → 分层发现）是唯一能挡住这类
    /// 「生成了一份坏文件」的办法。
    #[tokio::test]
    async fn init生成的项目技能能被真的加载且是只读的() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_string_lossy().to_string();

        workspace_init(WorkspaceInitReq {
            path: ws.clone(),
            project_name: "测试项目".into(),
            template_type: Some("developer".into()),
        })
        .await
        .expect("init 失败");

        // 走分层发现的生产路径。global_root 指一个空目录，隔离掉仓库自己的 skills/
        let empty = tempfile::tempdir().unwrap();
        let found = crate::skill::list_layered_in(
            Some(&crate::skill::project_skills_dir(dir.path())),
            empty.path(),
        );
        let skill = found
            .iter()
            .find(|s| s.id == "code-reviewer")
            .unwrap_or_else(|| panic!("init 生成的技能没被加载，只找到 {found:#?}"));

        assert_eq!(skill.scope, crate::skill::SkillScope::Project);
        assert_eq!(skill.kind, crate::skill::SkillKind::Task, "审查是「做一件事」，不是「当一个人」");
        assert_eq!(skill.surface, crate::skill::SkillSurface::Coding);
        // 规程里要有参数占位符，否则 `/review src/lru.ts` 的参数没处去
        assert!(
            crate::skill::expand::placeholders(&skill.prompt).takes_args(),
            "生成的规程没有参数占位符"
        );
        assert!(!skill.marker.is_empty(), "marker 是必填的，漏了会被静默跳过");
        assert!(!skill.prompt.trim().is_empty(), "正文必须进 prompt");

        // 只读的实质：拿不到写入与执行工具。**这是技能相对 PROMPT.md 的
        // 真实价值** —— 后者只是背景约束，改不了模型能调什么。
        for denied in ["fs_write_text", "apply_patch", "shell_command", "exec_command"] {
            assert!(!skill.allows_tool(denied), "只读技能不该拿到 {denied}");
        }
        for allowed in ["view_file", "code_outline", "search_knowledge", "git_status"] {
            assert!(skill.allows_tool(allowed), "审查需要 {allowed}");
        }
    }

    #[tokio::test]
    async fn 绑定工作区时自动初始化且按目录内容选模板() {
        // 代码项目 → developer。那套模板原先从界面上够不着
        // （只有 general 和 drama 两个按钮）
        let code = tempfile::tempdir().unwrap();
        std::fs::write(code.path().join("package.json"), "{}").unwrap();
        auto_init_if_needed(Some(&code.path().to_string_lossy()), "我的项目").await;
        assert!(code.path().join(".anima/project.json").is_file());
        let prompt = std::fs::read_to_string(code.path().join("PROMPT.md")).unwrap();
        assert!(
            prompt.contains("Principal Software Architect") || prompt.contains("软件工程架构"),
            "该用 developer 模板：{prompt}"
        );

        // 空目录 → general
        let plain = tempfile::tempdir().unwrap();
        auto_init_if_needed(Some(&plain.path().to_string_lossy()), "随便").await;
        let prompt = std::fs::read_to_string(plain.path().join("PROMPT.md")).unwrap();
        assert!(
            prompt.contains("AI Agent Directives") || prompt.contains("专属核心 AI 智能体"),
            "该用 general 模板：{prompt}"
        );
    }

    #[tokio::test]
    async fn 自动初始化是幂等的也不动用户改过的内容() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy().to_string();
        auto_init_if_needed(Some(&path), "P").await;
        std::fs::write(dir.path().join("PROMPT.md"), "我自己写的").unwrap();
        // 再绑一次
        auto_init_if_needed(Some(&path), "P").await;
        assert_eq!(
            std::fs::read_to_string(dir.path().join("PROMPT.md")).unwrap(),
            "我自己写的"
        );
        // 路径为空 / 目录不存在都该安静返回，不 panic
        auto_init_if_needed(None, "P").await;
        auto_init_if_needed(Some("   "), "P").await;
        auto_init_if_needed(Some("/nope/does/not/exist"), "P").await;
    }

    #[tokio::test]
    async fn 项目授权落盘且能被白名单真的匹配上() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy().to_string();
        workspace_init(WorkspaceInitReq {
            path: path.clone(),
            project_name: "P".into(),
            template_type: Some("developer".into()),
        })
        .await
        .unwrap();

        let read = || -> Vec<String> {
            let v: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(dir.path().join(".anima/project.json")).unwrap(),
            )
            .unwrap();
            v["permissions"]["terminal_policy"]["auto_approve_commands"]
                .as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_str().unwrap().to_string())
                .collect()
        };
        let before = read();
        assert!(add_auto_approve_prefix(&path, "eslint").await.unwrap());
        let after = read();
        assert_eq!(after.len(), before.len() + 1);
        assert!(after.contains(&"eslint".to_string()));
        // 原有的一条都不能少 —— 这是合并不是覆写
        for b in &before {
            assert!(after.contains(b), "丢了 {b}");
        }
        // 重复加是幂等的，返回 false 表示"本来就在"
        assert!(!add_auto_approve_prefix(&path, "eslint").await.unwrap());
        assert_eq!(read().len(), after.len());
        assert!(!add_auto_approve_prefix(&path, "   ").await.unwrap());
    }

    #[tokio::test]
    async fn 半成品的project_json也能写进白名单() {
        // 「改一下权限」造出来的 .anima/ 里可能只有一个 {}，
        // 逐层补齐路径才不会 panic
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy().to_string();
        std::fs::create_dir_all(dir.path().join(".anima")).unwrap();
        std::fs::write(dir.path().join(".anima/project.json"), "{}").unwrap();
        assert!(add_auto_approve_prefix(&path, "git log").await.unwrap());
        let v: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join(".anima/project.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            v["permissions"]["terminal_policy"]["auto_approve_commands"],
            serde_json::json!(["git log"])
        );

        // 连文件都没有也要能建
        let d2 = tempfile::tempdir().unwrap();
        let p2 = d2.path().to_string_lossy().to_string();
        assert!(add_auto_approve_prefix(&p2, "npm test").await.unwrap());
        assert!(d2.path().join(".anima/project.json").is_file());
    }

    #[tokio::test]
    async fn init生成的白名单里只读git子命令都在() {
        // 项目的这份会**覆盖**全局默认，少了只读 git 就等于每次看历史都要
        // 点一下审批 —— 而那会让用户养成"闭眼点允许"的习惯
        let dir = tempfile::tempdir().unwrap();
        let st = workspace_init(WorkspaceInitReq {
            path: dir.path().to_string_lossy().to_string(),
            project_name: "P".into(),
            template_type: None,
        })
        .await
        .unwrap();
        let list = st.project_meta.unwrap()["permissions"]["terminal_policy"]
            ["auto_approve_commands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x.as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        for want in ["git status", "git diff", "git log", "git show", "git branch"] {
            assert!(list.contains(&want.to_string()), "少了 {want}：{list:?}");
        }
        // 会改东西的一律不该在
        for bad in ["git push", "rm", "sudo", "git reset"] {
            assert!(!list.iter().any(|p| p == bad), "不该有 {bad}");
        }
    }

    #[tokio::test]
    async fn 重跑init不会重置用户改过的权限() {
        // **这是这块最危险的一条。** 原先 project.json 是无条件覆写的，
        // 于是每次重跑 init（`/init`、自动初始化、界面上再点一次）都会把
        // 用户改过的允许目录和免审批命令白名单全部重置 —— 而
        // `workspace_save_permissions` 是读-改-写合并的，两边行为不一致。
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy().to_string();
        let req = || WorkspaceInitReq {
            path: path.clone(),
            project_name: "P".into(),
            template_type: Some("developer".into()),
        };
        workspace_init(req()).await.unwrap();

        // 模拟用户改权限：加一条免审批命令、收窄允许目录
        let meta_file = dir.path().join(".anima/project.json");
        let mut meta: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&meta_file).unwrap()).unwrap();
        meta["permissions"]["terminal_policy"]["auto_approve_commands"] =
            serde_json::json!(["ls", "我加的命令"]);
        meta["permissions"]["file_access"]["allowed_paths"] =
            serde_json::json!(["/只允许这一个"]);
        meta["我自己加的字段"] = serde_json::json!("要留着");
        std::fs::write(&meta_file, serde_json::to_string_pretty(&meta).unwrap()).unwrap();

        // 重跑
        let st = workspace_init(req()).await.unwrap();
        let after: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&meta_file).unwrap()).unwrap();
        assert_eq!(
            after["permissions"]["terminal_policy"]["auto_approve_commands"],
            serde_json::json!(["ls", "我加的命令"]),
            "免审批命令被重置了"
        );
        assert_eq!(
            after["permissions"]["file_access"]["allowed_paths"],
            serde_json::json!(["/只允许这一个"]),
            "允许目录被重置了"
        );
        assert_eq!(after["我自己加的字段"], "要留着", "用户自加的字段被抹了");
        // 已经齐了，所以这一次没新建任何文件 —— 界面靠这个区分
        // 「补齐了三个」和「本来就是最新的」
        assert!(st.created.is_empty(), "{:?}", st.created);
    }

    #[tokio::test]
    async fn created_如实报出这一次新建了什么() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy().to_string();
        let st = workspace_init(WorkspaceInitReq {
            path: path.clone(),
            project_name: "P".into(),
            template_type: Some("general".into()),
        })
        .await
        .unwrap();
        let mut got = st.created.clone();
        got.sort();
        assert_eq!(
            got,
            vec![
                ".anima/knowledge/architecture.md",
                ".anima/project.json",
                ".anima/rules.md",
                ".anima/skills/code-reviewer/SKILL.md",
                "PROMPT.md",
            ]
        );

        // 删掉一个再补：只该报那一个
        std::fs::remove_file(dir.path().join(".anima/rules.md")).unwrap();
        let st = workspace_init(WorkspaceInitReq {
            path,
            project_name: "P".into(),
            template_type: None,
        })
        .await
        .unwrap();
        assert_eq!(st.created, vec![".anima/rules.md"]);
    }

    #[tokio::test]
    async fn check_status_不谎报created() {
        // 同一个结构走两条路：init 那条报新建了什么，check_status 那条恒为空。
        // 混了的话界面会以为「打开设置面板」也创建了文件
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy().to_string();
        workspace_init(WorkspaceInitReq {
            path: path.clone(),
            project_name: "P".into(),
            template_type: None,
        })
        .await
        .unwrap();
        let st = workspace_check_status(path).await.unwrap();
        assert!(st.created.is_empty());
    }

    #[tokio::test]
    async fn 半成品的anima不算已初始化() {
        // `workspace_save_permissions` 会顺手 create_dir_all(.anima) 再写
        // project.json —— 于是「改一下权限」就让目录存在了，而 PROMPT.md /
        // rules.md / knowledge / skills 一个都没有。
        //
        // 原先 `is_initialized` 只看目录在不在，这种半成品会被报成已初始化，
        // 界面上的初始化提示随之消失，而缺的东西一个都没补。
        let dir = tempfile::tempdir().unwrap();
        let anima = dir.path().join(".anima");
        std::fs::create_dir_all(&anima).unwrap();
        let path = dir.path().to_string_lossy().to_string();

        // 只有空目录
        let st = workspace_check_status(path.clone()).await.unwrap();
        assert!(!st.is_initialized, "空的 .anima/ 不该算已初始化");

        // 只有 project.json（保存权限造出来的形状）
        std::fs::write(anima.join("project.json"), "{}").unwrap();
        let st = workspace_check_status(path.clone()).await.unwrap();
        assert!(!st.is_initialized, "只有 project.json 也不算");

        // 补上指令文件才算
        std::fs::write(dir.path().join("PROMPT.md"), "指令").unwrap();
        let st = workspace_check_status(path).await.unwrap();
        assert!(st.is_initialized);
    }

    #[tokio::test]
    async fn init不建那个空的memory目录() {
        // 它曾经在这里被创建，然后全代码库再无第二处引用。一个看起来像存储
        // 位置、实际什么都不存的目录，会让人以为记忆能跟着仓库走 ——
        // 而记忆和知识库索引都在应用数据目录的 vectors/ 里，按 scope 分项目。
        let dir = tempfile::tempdir().unwrap();
        workspace_init(WorkspaceInitReq {
            path: dir.path().to_string_lossy().to_string(),
            project_name: "P".into(),
            template_type: Some("general".into()),
        })
        .await
        .unwrap();
        assert!(!dir.path().join(".anima/memory").exists());
        // 该建的都建了
        assert!(dir.path().join(".anima/knowledge/architecture.md").is_file());
        assert!(dir.path().join(".anima/project.json").is_file());
    }

    #[tokio::test]
    async fn 补齐脚手架时沿用已有的模板() {
        // `/init` 不传 template_type。如果那时退回 "general"，一个 developer
        // 项目的 project.json 会被改写成 general，而 PROMPT.md（不覆盖）还是
        // developer 那套 —— 两者从此不一致，且没有任何迹象。
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_string_lossy().to_string();
        workspace_init(WorkspaceInitReq {
            path: path.clone(),
            project_name: "P".into(),
            template_type: Some("developer".into()),
        })
        .await
        .unwrap();

        let template_of = || {
            let raw = std::fs::read_to_string(dir.path().join(".anima/project.json")).unwrap();
            serde_json::from_str::<serde_json::Value>(&raw).unwrap()["template"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert_eq!(template_of(), "developer");

        // 补齐：不传模板
        workspace_init(WorkspaceInitReq {
            path: path.clone(),
            project_name: "P".into(),
            template_type: None,
        })
        .await
        .unwrap();
        assert_eq!(template_of(), "developer", "模板被退回成 general 了");
        // PROMPT.md 仍是 developer 那套（不覆盖），所以两者一致
        let prompt = std::fs::read_to_string(dir.path().join("PROMPT.md")).unwrap();
        assert!(
            prompt.contains("Principal Software Architect") || prompt.contains("软件工程架构"),
            "{prompt}"
        );

        // 显式指定仍然能改
        workspace_init(WorkspaceInitReq {
            path,
            project_name: "P".into(),
            template_type: Some("drama".into()),
        })
        .await
        .unwrap();
        assert_eq!(template_of(), "drama");
    }

    #[tokio::test]
    async fn init不覆盖用户已经改过的文件() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_string_lossy().to_string();
        let req = || WorkspaceInitReq {
            path: ws.clone(),
            project_name: "P".into(),
            template_type: Some("developer".into()),
        };
        workspace_init(req()).await.unwrap();

        let skill_file = dir
            .path()
            .join(".anima/skills/code-reviewer/SKILL.md");
        std::fs::write(&skill_file, "---\nid: code-reviewer\nname: 我改过的\nmarker: 我\ndescription: d\n---\n\n我的正文\n").unwrap();
        let prompt_file = dir.path().join("PROMPT.md");
        std::fs::write(&prompt_file, "我自己写的项目指令").unwrap();

        // 再点一次初始化不该把用户的改动冲掉
        workspace_init(req()).await.unwrap();
        assert!(std::fs::read_to_string(&skill_file).unwrap().contains("我改过的"));
        assert_eq!(
            std::fs::read_to_string(&prompt_file).unwrap(),
            "我自己写的项目指令"
        );
    }

    #[tokio::test]
    async fn workspace_write_file直接写入并安全拦截路径穿越() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_string_lossy().to_string();

        // 正常相对路径写入与递归建目录
        workspace_write_file(ws.clone(), "src/main.rs".into(), "fn main() {}".into())
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("src/main.rs")).unwrap(),
            "fn main() {}"
        );

        // 拦截非法相对路径穿越
        let err = workspace_write_file(ws.clone(), "../outside.txt".into(), "leak".into()).await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn workspace_commit_file正常提交git仓库() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_string_lossy().to_string();

        // 初始化 git repo
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@anima.ai"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Anima Test"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        workspace_write_file(ws.clone(), "README.md".into(), "# Anima Project".into())
            .await
            .unwrap();

        let res = workspace_commit_file(ws.clone(), "feat: initial commit".into(), None)
            .await
            .unwrap();
        assert!(!res.commit_hash.is_empty());
        assert_eq!(res.message, "feat: initial commit");
    }

    #[tokio::test]
    async fn workspace_read_file支持图片预览与二进制自动检测() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_string_lossy().to_string();

        // 模拟写入 png 图像
        let png_bytes = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        std::fs::write(dir.path().join("icon.png"), png_bytes).unwrap();

        let res = workspace_read_file(Some(ws.clone()), "icon.png".into())
            .await
            .unwrap();
        assert!(res.is_binary);
        assert_eq!(res.media_type.as_deref(), Some("image"));
        assert!(res.data_url.as_deref().unwrap().starts_with("data:image/png;base64,"));
    }

    #[tokio::test]
    async fn workspace_git_status正确扫描未提交与新增文件() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_string_lossy().to_string();

        // 1. 初始化 Git 仓库并完成初次提交
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@anima.ai"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Anima Test"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        std::fs::write(dir.path().join("main.ts"), "console.log(1);").unwrap();
        workspace_commit_file(ws.clone(), "feat: initial".into(), None)
            .await
            .unwrap();

        // 初始状态下应该 clean
        let initial_status = workspace_git_status(Some(ws.clone())).await.unwrap();
        assert!(initial_status.is_git);
        assert!(initial_status.clean);
        assert_eq!(initial_status.changes.len(), 0);

        // 2. 修改现有文件并新增未跟踪文件
        std::fs::write(dir.path().join("main.ts"), "console.log(1);\nconsole.log(2);").unwrap();
        std::fs::write(dir.path().join("utils.ts"), "export const a = 1;").unwrap();

        let updated_status = workspace_git_status(Some(ws.clone())).await.unwrap();
        assert!(!updated_status.clean);
        assert_eq!(updated_status.changes.len(), 2);
        assert!(updated_status.changes.iter().any(|c| c.path == "main.ts"));
        assert!(updated_status.changes.iter().any(|c| c.path == "utils.ts"));
    }

    // ─────────────────────────────────────────────────────────────────────
    // 文件树
    // ─────────────────────────────────────────────────────────────────────

    /// 把树压成 `路径 -> (有没有 children, truncated)`，方便断言。
    fn flatten(nodes: &[FileTreeNode], out: &mut Vec<(String, bool, bool)>) {
        for n in nodes {
            out.push((n.path.clone(), n.children.is_some(), n.truncated));
            if let Some(kids) = &n.children {
                flatten(kids, out);
            }
        }
    }

    fn tree_of(dir: &tempfile::TempDir, depth: usize) -> Vec<(String, bool, bool)> {
        let mut out = Vec::new();
        flatten(&build_tree(dir.path(), dir.path(), 1, depth), &mut out);
        out
    }

    /// **超出深度的目录必须自报「我还有东西」。**
    ///
    /// 这是从真实故障反推的：`novels/ai-last-command/vol1-余波/剧本/` 正好是
    /// 第 5 层，默认只走 4 层，于是 `剧本` 展开之后是空的 —— 而**空目录也是
    /// 空的**，界面上完全一样，不报任何错。用户看到的是「超过 5 层就看不到
    /// 下面的内容了」。
    #[test]
    fn 撞到深度上限的目录会标成还有内容() {
        let dir = ws(&[("a/b/c/d/e/深处.md", "x")]);
        let t = tree_of(&dir, 4);

        let d = t.iter().find(|(p, ..)| p == "a/b/c/d").expect("d 应当在树里");
        assert!(!d.1, "d 在深度上限之外，不该带 children");
        assert!(d.2, "d 里还有东西，必须标 truncated —— 否则界面上和空目录一样");

        // 而且下一层确实没进来 —— 这正是要靠 truncated 告诉前端的
        assert!(!t.iter().any(|(p, ..)| p.starts_with("a/b/c/d/e")));
    }

    /// **真的空的目录不能标 truncated。**
    ///
    /// 标了的话前端会去取一次必然空手而归的请求，而且展开箭头一直暗示
    /// 「还有内容」—— 那是另一种骗人。
    #[test]
    fn 真的空的目录不标() {
        // 深度 4 走到 a=1 b=2 c=3 d=4；d 是最后一层，它自己进树但不再往下。
        // 所以要验「空目录不标」，那个空目录本身得在第 4 层。
        let dir = ws(&[("a/b/c/占位.md", "x")]);
        std::fs::create_dir_all(dir.path().join("a/b/c/空的")).unwrap();
        let t = tree_of(&dir, 4);

        let empty = t
            .iter()
            .find(|(p, ..)| p == "a/b/c/空的")
            .expect("第 4 层的目录应当在树里");
        assert!(!empty.1, "它在深度上限上，不该带 children");
        assert!(!empty.2, "空目录不该标 truncated");
    }

    /// 忽略名单里的东西**不算内容** —— 一个只有 `node_modules` 的目录
    /// 标成 truncated，展开之后还是空的。
    #[test]
    fn 只剩被忽略的条目时不算还有内容() {
        let dir = ws(&[
            ("a/b/c/d/e/x.ts", "x"),
            ("a/b/c/only_ignored/node_modules/pkg/index.js", "x"),
        ]);
        let t = tree_of(&dir, 4);
        let only = t
            .iter()
            .find(|(p, ..)| p == "a/b/c/only_ignored")
            .expect("应当在树里");
        assert!(!only.2, "里面只有 node_modules，不该标 truncated");

        let d = t.iter().find(|(p, ..)| p == "a/b/c/d").unwrap();
        assert!(d.2, "d 里有真内容，该标");
    }

    /// **`sub_path` 取子树时，路径仍然相对工作区根。**
    ///
    /// 前端拿这个路径当 key、也拿它回传做下一次的 `sub_path` ——
    /// 变成相对子目录的话，第二次展开就会拼出 `a/b/c/d/d/e` 这种路径，
    /// 而结果是空数组，看起来又是「展开了什么都没有」。
    #[tokio::test]
    async fn 取子树时路径仍然相对工作区根() {
        let dir = ws(&[("a/b/c/d/e/深处.md", "x")]);
        let root = dir.path().to_string_lossy().to_string();

        let sub = workspace_list_directory_tree(Some(root), Some("a/b/c/d".into()), Some(4))
            .await
            .unwrap();
        let mut out = Vec::new();
        flatten(&sub, &mut out);

        assert!(
            out.iter().any(|(p, ..)| p == "a/b/c/d/e/深处.md"),
            "第 5、6 层应当取得到，且路径相对根：{out:?}"
        );
    }

    /// 深度上限有个天花板，别人传 999 也不会把整块盘走一遍。
    #[tokio::test]
    async fn 深度请求被夹在上限内() {
        let dir = ws(&[("a/b/c/d/e/f/g/h/i/j/深处.md", "x")]);
        let root = dir.path().to_string_lossy().to_string();
        let t = workspace_list_directory_tree(Some(root), None, Some(999))
            .await
            .unwrap();
        let mut out = Vec::new();
        flatten(&t, &mut out);
        // MAX_TREE_DEPTH = 8，所以第 9 层（i）不该进来
        assert!(!out.iter().any(|(p, ..)| p.contains("/i")), "{out:?}");
        assert!(out.iter().any(|(p, ..)| p == "a/b/c/d/e/f/g/h"), "{out:?}");
    }
}
