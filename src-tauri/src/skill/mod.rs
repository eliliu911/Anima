//! 技能（角色扮演）。一个技能同时切换身份、方法、可用工具与模型偏好。
//!
//! ## 一个技能不只是换个称呼
//!
//! 按 `docs/PERSONALITY_MEMORY_SKILLS.md` 第三节定的，一个技能是一组同时切换的东西：
//!
//! | 部分 | 落点 |
//! |---|---|
//! | 身份与说话方式 | System Prompt 片段 |
//! | 专业方法与边界 | 同上 |
//! | 可用工具白名单 | 收窄给模型的工具表 |
//! | 模型偏好 | 走实时还是后台链路 |
//! | 思考模式 | 诊断类默认开，闲聊类默认关 |
//!
//! **工具白名单是关键设计**：家庭医生不该能执行 shell 命令，软件工程师不需要
//! 查药品。缩小工具面既降风险也提准确率 —— 工具越少，模型选错的机会越少。
//!
//! ## 为什么是文件而不是代码里的常量
//!
//! 技能内容是**要被反复调的东西**，而调它不该需要重新编译。放在项目的
//! `skills/<id>/SKILL.md`，用户能直接改、能看到 diff、能加自己的角色。
//!
//! 格式是 YAML frontmatter + Markdown 正文：前者是机器读的元数据，
//! 后者原样进 System Prompt。选 Markdown 是因为**它同时给人和给模型看** ——
//! 换成 JSON 之类的话，正文里的段落结构会变得很难写。
//!
//! ## 内容来源
//!
//! 三个预设的专业内容来自外部素材，原文与全部参考资料完整保存在各自的
//! `reference/` 下 —— 那是几万字的体系化内容，值得留着。
//!
//! 但**没有整段注入**，理由有两条：原文是英文的书面风格
//! （"documentation-ready"、"clinical precision"），直接用会让语音回答变成公文；
//! 而且每轮注入几千字符会明显推高首字延迟（这一点本项目实测过）。
//! 所以另写了中文的语音版提示词，从原文忠实提炼。
//!
//! **危机热线是地区相关的，写错比不写更糟** —— 对方会真的去打，而打不通的时候
//! 他正处在危机里。当前用的是美国的号码（988 / 741741 / 911），换地区时必须
//! 连同 `family-doctor` 的急救号码一起改，并且更新对应的测试断言。

pub mod expand;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::Tier;
use crate::error::{Error, Result};

/// 一个技能的完整定义。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Skill {
    /// **有默认值，由目录名兜住。** `list_in` 里早就有「id 空则用目录名」
    /// 那一段，但 `id` 曾经是必填的 —— 解析先失败，那段永远走不到。
    /// 两个地方描述同一件事，其中一个是死的。
    #[serde(default)]
    pub id: String,
    pub name: String,
    /// 顶部状态栏显示的标记。短，两到四个字。
    ///
    /// **有默认值。** 早先它是必填的，而 `list_in` 对解析失败的技能是
    /// **静默跳过**的 —— 于是一份只写了 `name` + `description` 的 SKILL.md
    /// 在磁盘上、看起来像配置、用户会去改它，而它从来没被加载过，一行报错都没有。
    /// 一个纯展示用的字段不该让整份文件消失。空值时界面自己不渲染这个标记。
    #[serde(default)]
    pub marker: String,
    /// 一句话说清它是什么。设置面板里展示。同样是展示用，同样给默认值。
    #[serde(default)]
    pub description: String,
    /// 中文界面下显示的名称。空则回落到 `name`。
    ///
    /// **放在 frontmatter 里而不是前端的语言包里**，因为技能是用户可以自己
    /// 新增和编辑的文件 —— 别人加一个自己的技能时，应该也能给它起中文名，
    /// 而不是必须去改应用的 i18n 资源。
    #[serde(default)]
    pub name_zh: String,
    /// 中文界面下显示的说明。空则回落到 `description`。
    #[serde(default)]
    pub description_zh: String,
    /// 中文界面下的标记。空则回落到 `marker`。
    #[serde(default)]
    pub marker_zh: String,
    /// 走哪条链路。诊断类需要更强的模型，闲聊类要快
    #[serde(default)]
    pub prefer_tier: Option<Tier>,
    /// 思考模式的默认值。None 表示跟随全局设置
    #[serde(default)]
    pub thinking: Option<bool>,
    /// 可用工具白名单。支持 `server__*` 这种前缀通配。
    ///
    /// **空表示不限制**（跟随全局），而不是"一个工具都不给" ——
    /// 后者会让忘写这一项的技能悄悄失去所有工具。
    #[serde(default)]
    pub tools: Vec<String>,
    /// 内容来源与署名。上游许可要求保留
    #[serde(default)]
    pub source: String,
    /// 激活语义：身份还是任务。
    ///
    /// 默认 `Persona`（向后兼容 —— 这个字段出现之前所有技能都是身份）。
    #[serde(default)]
    pub kind: SkillKind,
    /// 在哪条链路上可用。
    ///
    /// **默认 `Both`。** 隐藏比显示更糟 —— 一个在列表里消失的技能看起来像坏了，
    /// 而用户没有任何线索去查。所以未声明就两边都能用，
    /// 需要收窄的（家庭医生不该出现在 coding 会话里）显式写。
    #[serde(default)]
    pub surface: SkillSurface,
    /// Markdown 正文，原样拼进 System Prompt
    #[serde(skip)]
    pub prompt: String,
    /// 这个技能是从哪一层找到的。**不来自 frontmatter**，由发现过程填。
    ///
    /// 界面上必须能看出来：同 id 时项目覆盖全局，而"我改了全局那份却没生效"
    /// 只有显示来源才解释得清。
    ///
    /// **只出不进**（`skip_deserializing`）：要发给前端，但绝不能让某份
    /// SKILL.md 在 frontmatter 里声明自己是项目级的 —— 那会让"它是从哪来的"
    /// 变成文件里的一句自称，而不是发现过程的结论。
    #[serde(default, skip_deserializing)]
    pub scope: SkillScope,
}

/// 激活语义。
///
/// ## 为什么这是一个字段而不是两套系统
///
/// 「身份」和「任务」在结构上完全一样：一段进 System Prompt 的 Markdown +
/// 一个工具白名单。差别只在**激活方式** —— 身份勾一次一直生效（顶栏显示
/// 当前是谁），任务触发一次这一轮生效（无状态）。
///
/// 分成两套注册表就要维护两套发现、两套分层、两套覆盖规则，
/// 而它们本来会长得一模一样。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SkillKind {
    /// 身份：勾选后持续生效
    #[default]
    Persona,
    /// 任务：触发一次，这一轮生效。`/` 弹层和 `run_skill` 工具列的是这一类
    Task,
}

/// 技能在哪条链路上可用。
///
/// ## 为什么需要这一维
///
/// STT 只是传输层 —— 语音变成文本进输入框，之后和敲键盘没有区别。所以
/// **不该有「语音技能」和「聊天技能」两套**。但确实有一部分技能只对陪伴
/// 链路有意义（家庭医生、心理咨询师），把它们摆在 coding 会话的列表里，
/// 用户勾了之后会得到一个拿不到任何代码工具的状态，而且看不出为什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SkillSurface {
    /// 只在陪伴/语音链路
    Companion,
    /// 只在项目 coding 链路
    Coding,
    /// 两边都可用
    #[default]
    Both,
}

impl SkillSurface {
    /// 这个技能在「当前是不是 coding 会话」的语境下可见吗
    pub fn available_in_coding(self) -> bool {
        matches!(self, Self::Coding | Self::Both)
    }
    pub fn available_in_companion(self) -> bool {
        matches!(self, Self::Companion | Self::Both)
    }
}

/// 技能来自哪一层。项目层优先。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SkillScope {
    /// 项目自带（`<工作区>/.anima/skills/`）
    Project,
    /// 全局（项目目录的 `skills/` 或应用数据目录）
    #[default]
    Global,
}

impl Skill {
    /// 这个工具在这个技能下能不能用。
    ///
    /// 空白名单 = 不限制。通配只支持结尾的 `*`（`filesystem__*`）——
    /// 完整的 glob 在这里没有必要，而且越复杂越容易写出意外放行的规则。
    pub fn allows_tool(&self, qualified_name: &str) -> bool {
        if self.tools.is_empty() {
            return true;
        }
        self.tools
            .iter()
            .any(|pattern| match pattern.strip_suffix('*') {
                Some(prefix) => qualified_name.starts_with(prefix),
                None => qualified_name == pattern,
            })
    }
}

/// 技能目录。默认是项目里的 `skills/`。
///
/// 解析顺序和音色库一致：环境变量 → 往上找项目目录 → 应用数据目录。
/// 打包成 .app 之后没有项目目录，那时要靠把 skills/ 一起装进资源里。
pub fn skills_dir(config_dir: &Path) -> PathBuf {
    if let Some(explicit) = std::env::var_os("ANIMA_SKILLS") {
        let path = PathBuf::from(explicit);
        if !path.as_os_str().is_empty() {
            return path;
        }
    }
    if let Some(found) = find_project_skills(config_dir) {
        return found;
    }
    config_dir.join("skills")
}

fn find_project_skills(config_dir: &Path) -> Option<PathBuf> {
    // 同 `characters`：要求里面至少有一个带 `SKILL.md` 的子目录，
    // 否则同名空目录会误命中。
    crate::app_root::find_dir("skills", Some(config_dir), |c| {
        std::fs::read_dir(c)
            .map(|it| it.flatten().any(|e| e.path().join("SKILL.md").is_file()))
            .unwrap_or(false)
    })
}

/// 把 `---` 包起来的 YAML frontmatter 和正文拆开。
///
/// 手写而不是引一个 frontmatter crate：规则就这一条，而多一个依赖多一处
/// 版本维护。
fn split_frontmatter(raw: &str) -> Option<(&str, &str)> {
    let rest = raw.strip_prefix("---")?.trim_start_matches(['\r', '\n']);
    let end = rest.find("\n---")?;
    let meta = &rest[..end];
    let body = rest[end + 4..].trim_start_matches(['\r', '\n']);
    Some((meta, body))
}

fn parse(raw: &str, id_hint: &str) -> Result<Skill> {
    let (meta, body) = split_frontmatter(raw).ok_or_else(|| {
        Error::config(format!(
            "技能「{id_hint}」缺少 YAML frontmatter（文件要以 --- 开头、再用 --- 收尾）"
        ))
    })?;
    let mut skill: Skill = serde_yaml::from_str(meta)
        .map_err(|e| Error::config(format!("技能「{id_hint}」的 frontmatter 解析失败：{e}")))?;
    if skill.prompt.is_empty() {
        skill.prompt = body.trim().to_string();
    }
    if skill.prompt.is_empty() {
        return Err(Error::config(format!("技能「{id_hint}」的正文是空的")));
    }
    Ok(skill)
}

/// 一个没能加载的技能。**要能报到界面上**，不能只进日志。
///
/// 实测踩到过：`kind: spec`（别的技能体系的取值）会让 serde 解析失败，
/// 整份 SKILL.md 被跳过。文件在磁盘上、看着像装好了、用户会去改它，
/// 而它从来没被加载过 —— 唯一的痕迹是一行 WARN 日志。
#[derive(Debug, Clone, Serialize)]
pub struct SkillLoadError {
    pub id: String,
    pub error: String,
}

/// 列出所有技能。目录读不到就返回空 —— 没有技能不该让应用起不来。
pub fn list_in(root: &Path) -> Vec<Skill> {
    list_in_reporting(root).0
}

/// 同上，但**把失败的也带回来**。见 [`SkillLoadError`]。
pub fn list_in_reporting(root: &Path) -> (Vec<Skill>, Vec<SkillLoadError>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return (Vec::new(), Vec::new());
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();

    let mut out = Vec::new();
    let mut bad = Vec::new();
    for dir in dirs {
        let file = dir.join("SKILL.md");
        if !file.is_file() {
            continue;
        }
        let id_hint = dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .to_string();
        match std::fs::read_to_string(&file)
            .map_err(|e| Error::config(format!("读不了 {}：{e}", file.display())))
            .and_then(|raw| parse(&raw, &id_hint))
        {
            Ok(mut skill) => {
                // 目录名是唯一的，用它兜住 frontmatter 里 id 写错或漏写
                if skill.id.is_empty() {
                    skill.id = id_hint;
                }
                out.push(skill);
            }
            // 一个技能坏了不该让其它的也用不了 —— 但必须说出来，
            // 而且不能只说给日志听：`bad` 会一路带到界面上。
            Err(err) => {
                tracing::warn!(%err, skill = %id_hint, "技能加载失败，跳过");
                bad.push(SkillLoadError {
                    id: id_hint,
                    error: err.to_string(),
                });
            }
        }
    }
    (out, bad)
}

/// 项目层的技能目录：`<工作区>/.anima/skills/`。
///
/// 和 Claude Code 的 `.claude/skills/` 同一个意思 —— 跟着仓库走、能进版本库、
/// 换机器也在。放在 `.anima/` 下而不是仓库根，是因为那个目录已经是
/// 「这个项目给 Anima 看的东西」的约定位置（project.json / rules.md / knowledge）。
pub fn project_skills_dir(workspace: &Path) -> PathBuf {
    workspace.join(".anima").join("skills")
}

/// 跨层列出技能，同 id 时**项目层胜出**。两个根都显式传入。
///
/// ## 为什么是 `_in` 而不是只有 `list_layered(config_dir, ..)`
///
/// `skills_dir()` 会从**进程工作目录**往上爬四层找 `skills/` —— 在测试里
/// 那会命中仓库自己的技能目录，于是断言"列表里有几个"永远是错的。
/// 和 `list_in` 一样把根显式化，测试才能真的隔离。
///
/// 为什么是覆盖而不是报冲突：这正是分层的用处 —— 项目里想让
/// `code-reviewer` 更严格一点，就在项目里放一份同名的，不用改全局那份、
/// 也不用给它另起一个名字。而覆盖发生了必须看得见，所以留下 `scope`。
/// 同 [`list_layered_in`]，但把两层里加载失败的都带回来。
pub fn list_layered_in_reporting(
    project_root: Option<&Path>,
    global_root: &Path,
) -> (Vec<Skill>, Vec<SkillLoadError>) {
    let mut bad = Vec::new();
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(p) = project_root.filter(|p| !p.as_os_str().is_empty()) {
        roots.push(p.to_path_buf());
    }
    roots.push(global_root.to_path_buf());
    for root in roots {
        let (_, mut e) = list_in_reporting(&root);
        // 同一个 id 在两层都坏了只报一次 —— 报两遍看着像两个问题
        e.retain(|x| !bad.iter().any(|b: &SkillLoadError| b.id == x.id));
        bad.append(&mut e);
    }
    (list_layered_in(project_root, global_root), bad)
}

pub fn list_layered_in(project_root: Option<&Path>, global_root: &Path) -> Vec<Skill> {
    let mut roots: Vec<(PathBuf, SkillScope)> = Vec::new();
    if let Some(p) = project_root.filter(|p| !p.as_os_str().is_empty()) {
        roots.push((p.to_path_buf(), SkillScope::Project));
    }
    roots.push((global_root.to_path_buf(), SkillScope::Global));

    let mut out: Vec<Skill> = Vec::new();
    for (root, scope) in roots {
        for mut skill in list_in(&root) {
            if out.iter().any(|s| s.id == skill.id) {
                tracing::debug!(id = %skill.id, ?scope, "同名技能被更高层覆盖");
                continue;
            }
            skill.scope = scope;
            out.push(skill);
        }
    }
    // 项目层排在前面便于界面分组；同层内按 id 稳定排序
    out.sort_by(|a, b| match (a.scope, b.scope) {
        (SkillScope::Project, SkillScope::Global) => std::cmp::Ordering::Less,
        (SkillScope::Global, SkillScope::Project) => std::cmp::Ordering::Greater,
        _ => a.id.cmp(&b.id),
    });
    out
}

pub fn resolve_layered_in(
    project_root: Option<&Path>,
    global_root: &Path,
    id: &str,
) -> Result<Skill> {
    let all = list_layered_in(project_root, global_root);
    all.iter().find(|s| s.id == id).cloned().ok_or_else(|| {
        Error::config(format!(
            "没有技能「{id}」。现有：{}",
            if all.is_empty() {
                "（一个都没有）".to_string()
            } else {
                all.iter()
                    .map(|s| s.id.as_str())
                    .collect::<Vec<_>>()
                    .join("、")
            }
        ))
    })
}

pub fn list_layered(config_dir: &Path, workspace: Option<&Path>) -> Vec<Skill> {
    let project = workspace.map(project_skills_dir);
    list_layered_in(project.as_deref(), &skills_dir(config_dir))
}

pub fn resolve_layered(config_dir: &Path, workspace: Option<&Path>, id: &str) -> Result<Skill> {
    let project = workspace.map(project_skills_dir);
    resolve_layered_in(project.as_deref(), &skills_dir(config_dir), id)
}

pub fn list(config_dir: &Path) -> Vec<Skill> {
    list_in(&skills_dir(config_dir))
}

/// 按 id 找一个技能。
pub fn resolve_in(root: &Path, id: &str) -> Result<Skill> {
    let all = list_in(root);
    all.iter().find(|s| s.id == id).cloned().ok_or_else(|| {
        Error::config(format!(
            "没有技能「{id}」。现有：{}",
            if all.is_empty() {
                "（一个都没有）".to_string()
            } else {
                all.iter()
                    .map(|s| s.id.as_str())
                    .collect::<Vec<_>>()
                    .join("、")
            }
        ))
    })
}

pub fn resolve(config_dir: &Path, id: &str) -> Result<Skill> {
    resolve_in(&skills_dir(config_dir), id)
}

/// 保存或新建一个技能文件（YAML frontmatter + Markdown 正文）
pub fn save_skill(
    config_dir: &Path,
    workspace: Option<&Path>,
    scope: SkillScope,
    id: &str,
    name: &str,
    marker: &str,
    description: &str,
    prefer_tier: Option<Tier>,
    thinking: Option<bool>,
    tools: Vec<String>,
    source: &str,
    kind: SkillKind,
    surface: SkillSurface,
    prompt: &str,
) -> Result<Skill> {
    let clean_id = id.trim().to_lowercase();
    if clean_id.is_empty() {
        return Err(Error::config("技能 ID 不能为空"));
    }
    // 校验安全合法的文件夹名
    if !clean_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(Error::config("技能 ID 只能包含英文字母、数字、下划线和中划线"));
    }

    let root = match scope {
        SkillScope::Project => {
            let ws = workspace.ok_or_else(|| Error::config("未绑定项目工作区，无法保存项目级技能"))?;
            project_skills_dir(ws)
        }
        SkillScope::Global => skills_dir(config_dir),
    };

    let skill_dir = root.join(&clean_id);
    std::fs::create_dir_all(&skill_dir)
        .map_err(|e| Error::config(format!("创建技能目录失败 {}: {e}", skill_dir.display())))?;

    #[derive(Serialize)]
    struct Frontmatter<'a> {
        id: &'a str,
        name: &'a str,
        marker: &'a str,
        description: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        prefer_tier: Option<Tier>,
        #[serde(skip_serializing_if = "Option::is_none")]
        thinking: Option<bool>,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        tools: Vec<String>,
        #[serde(skip_serializing_if = "str::is_empty")]
        source: &'a str,
        kind: SkillKind,
        surface: SkillSurface,
    }

    let fm = Frontmatter {
        id: &clean_id,
        name: name.trim(),
        marker: marker.trim(),
        description: description.trim(),
        prefer_tier,
        thinking,
        tools,
        source: source.trim(),
        kind,
        surface,
    };

    let yaml = serde_yaml::to_string(&fm)
        .map_err(|e| Error::config(format!("序列化技能元数据失败: {e}")))?;

    let clean_prompt = prompt.trim();
    let file_content = format!("---\n{}\n---\n\n{}\n", yaml.trim(), if clean_prompt.is_empty() { "你是一个拥有专业技能的智能体助手。" } else { clean_prompt });
    let file_path = skill_dir.join("SKILL.md");
    std::fs::write(&file_path, file_content)
        .map_err(|e| Error::config(format!("写入 SKILL.md 失败 {}: {e}", file_path.display())))?;

    let mut saved = parse(&std::fs::read_to_string(&file_path)?, &clean_id)?;
    saved.scope = scope;
    Ok(saved)
}

/// 删除一个技能目录
pub fn delete_skill(
    config_dir: &Path,
    workspace: Option<&Path>,
    id: &str,
) -> Result<()> {
    let clean_id = id.trim();
    if clean_id.is_empty() {
        return Err(Error::config("技能 ID 不能为空"));
    }

    let mut deleted = false;
    // 优先检查项目级技能
    if let Some(ws) = workspace {
        let proj_dir = project_skills_dir(ws).join(clean_id);
        if proj_dir.is_dir() {
            let _ = std::fs::remove_dir_all(&proj_dir);
            deleted = true;
        }
    }

    // 再检查全局技能目录
    let global_dir = skills_dir(config_dir).join(clean_id);
    if global_dir.is_dir() {
        let _ = std::fs::remove_dir_all(&global_dir);
        deleted = true;
    }

    if !deleted {
        return Err(Error::config(format!("未找到技能「{clean_id}」")));
    }
    Ok(())
}

/// 按一组技能收窄工具表。**两条链路共用这一份。**
///
/// 返回 `(保留下来的工具, 是否把所有工具都过滤掉了)`。
///
/// ## 为什么要报「全被过滤掉了」
///
/// 白名单写的是 MCP 时代的命名（`filesystem__*`）而 coding 链路给的是裸名
/// （`view_file`）时，结果是**一个工具都不剩且不报错** —— 模型只会看起来
/// 突然什么都不会干，而这个现象离「白名单里的模式写错了」非常远。
/// 实测 `software-engineer` 的旧白名单在 17 个内建工具上通过 0 个。
///
/// 不能因此就退回「不限制」：那会让家庭医生拿到 shell。所以照样过滤，
/// 但把这个状态显式返回，让调用方去 warn / 在界面上标出来。
pub fn narrow_tools(tools: Vec<crate::llm::Tool>, skills: &[Skill]) -> (Vec<crate::llm::Tool>, bool) {
    if skills.is_empty() {
        return (tools, false);
    }
    // 任意一个技能不限制，取并集就等于全量开放
    if skills.iter().any(|s| s.tools.is_empty()) {
        return (tools, false);
    }
    let before = tools.len();
    let kept: Vec<crate::llm::Tool> = tools
        .into_iter()
        .filter(|t| skills.iter().any(|s| s.allows_tool(&t.name)))
        .collect();
    let wiped = before > 0 && kept.is_empty();
    if wiped {
        tracing::warn!(
            before,
            skills = ?skills.iter().map(|s| &s.id).collect::<Vec<_>>(),
            patterns = ?skills.iter().flat_map(|s| &s.tools).collect::<Vec<_>>(),
            "技能的工具白名单把**全部**工具都过滤掉了 —— \
             通常是白名单里写的是 MCP 命名（filesystem__*）而这条链路给的是裸名（view_file）"
        );
    }
    (kept, wiped)
}

/// 技能白名单里**点了名**（或用 `前缀*`）的工具，分层时被藏起来了的，从全集里找回来补上。
///
/// 技能的白名单是「这个角色要用这些」—— 那就不该再让模型先 `tool_search` 一轮才拿得到参数。
/// 只补白名单允许的，不会多放工具出来。
pub fn restore_named_tools(tools: &mut Vec<crate::llm::Tool>, all: &[crate::llm::Tool], skills: &[Skill]) {
    if skills.is_empty() || skills.iter().any(|s| s.tools.is_empty()) {
        return;
    }
    for t in all {
        if tools.iter().any(|x| x.name == t.name) {
            continue;
        }
        if skills.iter().any(|s| s.allows_tool(&t.name)) {
            tools.push(t.clone());
        }
    }
}

/// `run_skill` 的工具名。
pub const RUN_SKILL_NAME: &str = "run_skill";

pub fn is_run_call(name: &str) -> bool {
    name == RUN_SKILL_NAME || name.ends_with(&format!("__{RUN_SKILL_NAME}"))
}

/// `run_skill` 的工具声明。
///
/// ## 为什么任务技能是「一个返回自己指令的工具」
///
/// 用户可以敲 `/review src/lru.ts`，也可以**说**「帮我审查一下 lru.ts」。
/// STT 只是传输层，两者到达时都是文本 —— 所以不该有两套分发。
///
/// 做成工具之后：键盘那条路由前端展开后直接发出去，语音那条路由模型自己
/// 调 `run_skill`，而**工具的返回值就是那份技能的正文**（参数已填好），
/// 模型在同一轮里照着做。两条路共用同一个 `expand`，
/// 而且语音入口不需要任何新 UI。
///
/// 换成「起一个子轮次去执行技能」也能做，但那要一套嵌套的流式事件、
/// 取消传播和预算归属 —— 而收益是零，因为指令进上下文就够了。
///
/// `available` 为空时**不要声明这个工具**：一个列不出任何选项的工具，
/// 模型会拿它去猜 id，然后收到一串"没有这个技能"。
pub fn run_declaration(available: &[Skill]) -> crate::llm::Tool {
    let list = available
        .iter()
        .map(|s| {
            let takes = expand::placeholders(&s.prompt);
            format!(
                "- `{}`（{}）：{}{}",
                s.id,
                s.name,
                if s.description.is_empty() {
                    "—"
                } else {
                    &s.description
                },
                if takes.takes_args() {
                    " **需要参数**"
                } else {
                    ""
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    crate::llm::Tool {
        name: RUN_SKILL_NAME.to_string(),
        description: format!(
            "载入一份**任务技能**的操作规程并立即照着执行。用户说\"审查一下这个文件\"、\
             \"按我们的规范提交\"、\"给这个模块补测试\"这类话时调用它 —— \
             那些规程是这个项目自己定的，你猜不出来。\n\n\
             返回值就是规程正文，收到之后**在本轮内直接按它做**，不要复述它、\
             也不要问用户要不要执行。\n\n可用的技能：\n{list}"
        ),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "skill": {
                    "type": "string",
                    "description": "技能 id，必须是上面列出的其中一个",
                    "enum": available.iter().map(|s| s.id.clone()).collect::<Vec<_>>(),
                },
                "arguments": {
                    "type": "string",
                    "description": "传给规程的参数，例如要审查的文件路径。规程里的 $ARGUMENTS / $1 会被替换成它"
                }
            },
            "required": ["skill"]
        }),
    }
}

/// 从 `run_skill` 的参数里取出 (技能 id, 参数)。
///
/// 解析失败返回 `None` —— 和 `skill_from_arguments` 一样的宽容策略：
/// 本地小模型经常给 JSON 裹围栏或者把字段名写错，硬报错只会让对话卡死。
pub fn run_from_arguments(raw: &str) -> Option<(String, String)> {
    let v: serde_json::Value = serde_json::from_str(raw.trim()).ok()?;
    let id = v
        .get("skill")
        .or_else(|| v.get("id"))
        .or_else(|| v.get("name"))
        .and_then(|x| x.as_str())?
        .trim()
        .to_string();
    if id.is_empty() {
        return None;
    }
    let args = v
        .get("arguments")
        .or_else(|| v.get("args"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    Some((id, args))
}

/// 把一份任务技能展开成「给模型看的一段规程」。
///
/// 前面那段框架语是必要的：单独一段 Markdown 进到工具结果里，模型很容易
/// 把它当成**要总结给用户的资料**，而不是**要照着做的指令**。实测本地模型
/// 会回一句"好的，这份规范要求……"然后停下。
pub fn expand_task(skill: &Skill, args: &str) -> String {
    let body = expand::substitute(&skill.prompt, args);
    let mut out = format!("以下是「{}」的操作规程，现在开始照它执行。\n\n", skill.name);
    if !args.trim().is_empty() {
        out.push_str(&format!("本次的参数：{}\n\n", args.trim()));
    }
    out.push_str("---\n\n");
    out.push_str(&body);
    out.push_str(
        "\n\n---\n\n（以上是规程本身，不要把它复述给用户。直接开始做，做完报结果。）",
    );
    out
}

/// 筛出任务型技能，按当前是不是 coding 会话过滤。
pub fn tasks_for(all: &[Skill], in_coding: bool) -> Vec<Skill> {
    all.iter()
        .filter(|s| s.kind == SkillKind::Task)
        .filter(|s| {
            if in_coding {
                s.surface.available_in_coding()
            } else {
                s.surface.available_in_companion()
            }
        })
        .cloned()
        .collect()
}

/// 模型可以调用的 `switch_skill` 工具声明。
///
/// 内置工具（裸名、不过 MCP）—— 用户在语音里说"切换到家庭医生"
/// 时，模型自己完成切换，不用去点设置面板。
pub fn declaration(available: &[Skill]) -> crate::llm::Tool {
    let list = available
        .iter()
        .map(|s| format!("{}（{}）", s.id, s.name))
        .collect::<Vec<_>>()
        .join("、");
    crate::llm::Tool {
        name: BUILTIN_NAME.to_string(),
        description: format!(
            "切换到某个专业角色，或退出当前角色。用户说\"切到家庭医生\"、\
             \"用心理咨询师的方式\"、\"退出角色\"这类话的时候调用它。\
             可选：{list}。退出用空字符串。"
        ),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "skill": {
                    "type": "string",
                    "description": "要切到的技能 id；空字符串表示退回默认状态"
                }
            },
            "required": ["skill"]
        }),
    }
}

pub const BUILTIN_NAME: &str = "switch_skill";

pub fn is_switch_call(name: &str) -> bool {
    name == BUILTIN_NAME
}

/// 从工具参数里取要切到的技能 id。
///
/// `Some("")` 表示明确要退出角色，`None` 表示参数不可用。这两者必须分开 ——
/// 把解析失败当成"退出"会让一次格式错误悄悄把角色摘掉。
pub fn skill_from_arguments(raw: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    Some(value.get("skill")?.as_str()?.trim().to_string())
}

#[cfg(test)]
mod tests {

    /// **技能正文里指向的参考文件必须真的存在。**
    ///
    /// 正文告诉模型「用 view_file 打开 reference/xxx.md」，而模型有 view_file ——
    /// 文件不在的话它会拿到一个读取失败，然后照着自己的通用知识编。
    /// 这是典型的静默失效：技能加载成功、对话继续、内容却是编的。
    #[test]
    fn 技能正文引用的参考文件都存在() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("skills");
        let Ok(entries) = std::fs::read_dir(&root) else {
            return; // 没有 skills 目录时不算失败（打包场景）
        };
        for e in entries.flatten().filter(|e| e.path().is_dir()) {
            let skill_md = e.path().join("SKILL.md");
            let Ok(body) = std::fs::read_to_string(&skill_md) else {
                continue;
            };
            let id = e.file_name().to_string_lossy().into_owned();
            // 正文里形如 `xxx.md` 的引用：我们自己的技能放 reference/，外部装进来的
            // 放 references/ 或技能根目录（ai-mandrama 的 `reference.md`），三处都算。
            // 带路径的 `references/xxx.md` 按技能目录解析。
            for cap in body.split('`').skip(1).step_by(2) {
                if !cap.ends_with(".md") || cap == "SKILL.md" {
                    continue;
                }
                let found = if cap.contains('/') {
                    if !(cap.starts_with("reference/") || cap.starts_with("references/")) {
                        continue;
                    }
                    e.path().join(cap).is_file()
                } else {
                    ["reference", "references", ""].iter().any(|d| e.path().join(d).join(cap).is_file())
                };
                // 上游原文就这么写（「见 `reference.md`（如有需要可补充）」），文件在上游也不存在；
                // 正文照上游不改，这一条单独放行
                if !found && (id.as_str(), cap) == ("edge-tts-chinese-roleplay", "reference.md") {
                    continue;
                }
                assert!(
                    found,
                    "{id}/SKILL.md 引用了 {cap}，但文件不存在 —— \
                     模型会读取失败然后靠通用知识编，且不报错"
                );
            }
        }
    }

    use super::*;

    /// **仓库里的每一份 SKILL.md 都要真的加载得上。**
    ///
    /// `list_in` 对解析失败是静默跳过的（只记日志、界面上列出坏的那份）——
    /// 从外部仓库搬进来的技能 frontmatter 写法五花八门，一份坏了在列表里就是
    /// 「少了一个」，没人会去翻日志。2026-09-28 一次装了 18 份外部技能，这条钉住它们。
    #[test]
    fn 仓库里的技能全部能解析() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("skills");
        if !root.is_dir() {
            return; // 打包环境里没有源码目录
        }
        let (ok, bad) = list_in_reporting(&root);
        assert!(bad.is_empty(), "这些技能加载失败：{:?}", bad.iter().map(|b| (&b.id, &b.error)).collect::<Vec<_>>());
        for want in ["ai-storyboard-director", "ai-short-drama-production", "cinematic-video-prompt-engineer", "ai-mandrama-pipeline", "minimax-h3-combat"] {
            let s = ok.iter().find(|s| s.id == want).unwrap_or_else(|| panic!("{want} 没加载"));
            assert!(!s.name_zh.is_empty() && !s.source.is_empty(), "{want} 缺中文名或来源");
        }
    }

    #[test]
    fn 技能点了名的工具被分层藏起来时补回来() {
        let tool = |n: &str| crate::llm::Tool { name: n.into(), description: String::new(), parameters: serde_json::json!({}) };
        let all = vec![tool("tool_search"), tool("studio_create_project"), tool("studio_list_projects"), tool("shell_command")];
        let mut tiered = vec![tool("tool_search")];
        let mut skill = task_skill("studio-producer", "x");
        skill.tools = vec!["studio_*".into(), "tool_search".into()];
        restore_named_tools(&mut tiered, &all, std::slice::from_ref(&skill));
        let names: Vec<&str> = tiered.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["tool_search", "studio_create_project", "studio_list_projects"], "只补白名单里的，不放出 shell_command");
        // 技能不限制工具时什么都不补（全集已经在那一步决定）
        let mut t2 = vec![tool("tool_search")];
        skill.tools.clear();
        restore_named_tools(&mut t2, &all, &[skill]);
        assert_eq!(t2.len(), 1);
    }


    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("skills");
        write(
            &root.join("doc/SKILL.md"),
            "---\nid: doc\nname: 家庭医生\nmarker: 医生\ndescription: 看病\n\
             thinking: true\nprefer_tier: background\ntools:\n  - search_knowledge\n---\n\n你是医生。\n",
        );
        write(
            &root.join("eng/SKILL.md"),
            "---\nid: eng\nname: 工程师\nmarker: 工程\ndescription: 写码\n\
             tools:\n  - search_knowledge\n  - filesystem__*\n---\n\n你是工程师。\n",
        );
        // 没有白名单 —— 应当表示"不限制"
        write(
            &root.join("open/SKILL.md"),
            "---\nid: open\nname: 开放\nmarker: 开放\ndescription: 什么都能用\n---\n\n随便。\n",
        );
        dir
    }

    #[test]
    fn skills_load_from_disk_with_frontmatter_and_body() {
        let dir = fixture();
        let all = list_in(&dir.path().join("skills"));
        assert_eq!(
            all.len(),
            3,
            "{:?}",
            all.iter().map(|s| &s.id).collect::<Vec<_>>()
        );
        let doc = all.iter().find(|s| s.id == "doc").unwrap();
        assert_eq!(doc.name, "家庭医生");
        assert_eq!(doc.marker, "医生");
        assert_eq!(doc.thinking, Some(true));
        assert_eq!(doc.prefer_tier, Some(Tier::Background));
        assert_eq!(doc.prompt, "你是医生。");
    }

    #[test]
    fn an_empty_allowlist_means_unrestricted_not_nothing() {
        // **这一条很重要。** 空表当成"一个工具都不给"的话，忘写 tools 的技能会
        // 悄悄失去所有工具，而症状是"它突然不会用工具了"，极难归因
        let dir = fixture();
        let open = resolve_in(&dir.path().join("skills"), "open").unwrap();
        assert!(open.tools.is_empty());
        assert!(open.allows_tool("system__run_command"));
        assert!(open.allows_tool("filesystem__read_file"));
        assert!(open.allows_tool("search_knowledge"));
    }

    #[test]
    fn a_medical_role_cannot_reach_shell_tools() {
        // 工具白名单是这套设计的关键：家庭医生不该能执行 shell 命令。
        // 缩小工具面既降风险也提准确率
        let dir = fixture();
        let doc = resolve_in(&dir.path().join("skills"), "doc").unwrap();
        assert!(doc.allows_tool("search_knowledge"));
        assert!(!doc.allows_tool("system__run_command"));
        assert!(!doc.allows_tool("filesystem__read_file"));
    }

    #[test]
    fn prefix_wildcards_work_but_only_at_the_end() {
        let dir = fixture();
        let eng = resolve_in(&dir.path().join("skills"), "eng").unwrap();
        assert!(eng.allows_tool("filesystem__read_file"));
        assert!(eng.allows_tool("filesystem__write_file"));
        assert!(
            !eng.allows_tool("system__run_command"),
            "没写的前缀不该放行"
        );
        // 前缀匹配不能被"名字里含这一段"骗过去
        assert!(!eng.allows_tool("evil__filesystem__read"));
    }

    #[test]
    fn a_broken_skill_does_not_take_down_the_others() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("skills");
        write(
            &root.join("good/SKILL.md"),
            "---\nid: good\nname: 好\nmarker: 好\ndescription: d\n---\n\n正文\n",
        );
        // 缺 frontmatter
        write(&root.join("bad1/SKILL.md"), "没有 frontmatter 的正文\n");
        // frontmatter 里缺必填字段
        write(&root.join("bad2/SKILL.md"), "---\nid: bad2\n---\n\n正文\n");
        // 正文是空的
        write(
            &root.join("bad3/SKILL.md"),
            "---\nid: bad3\nname: n\nmarker: m\ndescription: d\n---\n\n\n",
        );
        let all = list_in(&root);
        assert_eq!(all.len(), 1, "坏掉的技能应当被跳过而不是拖垮全部：{all:?}");
        assert_eq!(all[0].id, "good");
    }

    #[test]
    fn resolving_a_missing_skill_lists_what_exists() {
        let dir = fixture();
        let err = resolve_in(&dir.path().join("skills"), "nope")
            .unwrap_err()
            .to_string();
        assert!(err.contains("doc"), "报错要列出现有技能：{err}");
    }

    #[test]
    fn switching_distinguishes_exit_from_a_parse_failure() {
        // 空串 = 明确要退出角色；None = 参数不可用。
        // 把解析失败当成"退出"会让一次格式错误悄悄把角色摘掉
        assert_eq!(
            skill_from_arguments(r#"{"skill":"doc"}"#).as_deref(),
            Some("doc")
        );
        assert_eq!(skill_from_arguments(r#"{"skill":""}"#).as_deref(), Some(""));
        assert_eq!(
            skill_from_arguments(r#"{"skill":"  "}"#).as_deref(),
            Some("")
        );
        assert_eq!(skill_from_arguments("{}"), None);
        assert_eq!(skill_from_arguments("not json"), None);
    }

    #[test]
    fn the_declaration_lists_every_available_skill() {
        // 模型只能从描述里知道有哪些角色可切。漏掉一个就等于那个角色
        // 在语音里永远切不到
        let dir = fixture();
        let all = list_in(&dir.path().join("skills"));
        let decl = declaration(&all);
        assert_eq!(decl.name, BUILTIN_NAME);
        for s in &all {
            assert!(
                decl.description.contains(&s.id),
                "描述里应当有 {}：{}",
                s.id,
                decl.description
            );
        }
    }

    // ---------- 真实的三个预设 ----------

    /// 项目里那三个预设必须能加载，而且**医疗类必须带硬边界**。
    ///
    /// 这不是产品修饰：不做诊断、不给剂量、急症转诊，是这个角色能不能存在的前提
    /// （见 docs/PERSONALITY_MEMORY_SKILLS.md 3.3）。
    #[test]
    #[ignore = "公开版不含第三方的医疗 / 心理 / 软件工程技能（没有许可，不转发布）"]
    fn the_shipped_presets_load_and_medical_ones_carry_hard_limits() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("skills");
        if !root.is_dir() {
            // 打包环境里可能没有源码目录，那时跳过而不是失败
            return;
        }
        let all = list_in(&root);
        for want in ["family-doctor", "counselor", "software-engineer"] {
            assert!(
                all.iter().any(|s| s.id == want),
                "预设 {want} 应当能加载：{:?}",
                all.iter().map(|s| &s.id).collect::<Vec<_>>()
            );
        }

        let doc = all.iter().find(|s| s.id == "family-doctor").unwrap();
        assert!(
            doc.prompt.contains("No Diagnostic") || doc.prompt.contains("不做诊断"),
            "Family Doctor prompt must disclaim diagnostic confirmation"
        );
        // Emergency telephone check: 911
        assert!(doc.prompt.contains("911"), "Acute emergency must provide 911");
        assert!(
            !doc.allows_tool("system__run_command"),
            "Family doctor must not execute arbitrary commands"
        );

        let counselor = all.iter().find(|s| s.id == "counselor").unwrap();
        assert!(
            counselor.prompt.contains("988"),
            "Counselor must provide crisis hotline (988)"
        );
        assert!(
            counselor.prompt.contains("741741"),
            "Counselor must provide text crisis line (741741)"
        );
        assert!(
            counselor.prompt.contains("Not Therapy") || counselor.prompt.contains("不是心理治疗"),
            "Counselor must state this is not clinical therapy"
        );
        assert!(!counselor.allows_tool("filesystem__read_file"));

        // 工程师角色**需要**文件与命令，那是它工作的一部分
        let eng = all.iter().find(|s| s.id == "software-engineer").unwrap();
        assert!(eng.allows_tool("filesystem__read_file"));
        assert!(eng.allows_tool("system__run_command"));

        // 每个预设都要说清来源 —— 原创还是衍生。
        for s in &all {
            assert!(
                !s.source.trim().is_empty(),
                "技能 {} 没有写来源。原创写「原创」，衍生的写清出处",
                s.id
            );
        }

        // 验证预设技能声明的裸工具名全部在后端真实注册表中存在，防止工具名拼写漂移（如 kill_session -> kill_exec_session）
        let mut registry = crate::agent_runtime::ToolRegistry::new();
        crate::agent_runtime::local_tools::register_local_tools(&mut registry);
        let memory_cell: crate::agent_runtime::knowledge_tool::MemoryCell = std::sync::Arc::new(tokio::sync::RwLock::new(None));
        registry.register(crate::agent_runtime::knowledge_tool::SearchKnowledgeTool::new(memory_cell));
        crate::agent_runtime::comfyui_provider::register_comfyui_tools(&mut registry, std::path::PathBuf::from("/tmp"));
        crate::agent_runtime::cinema_provider::register_cinema_tools(&mut registry, std::path::PathBuf::from("/tmp"));
        let gateway = std::sync::Arc::new(crate::llm::gateway::Gateway::new(crate::config::AppConfig::default()).unwrap());
        crate::agent_runtime::cinema_provider::register_cinema_audio_tools(
            &mut registry,
            std::path::PathBuf::from("/tmp"),
            gateway,
            reqwest::Client::new(),
        );
        crate::agent_runtime::cinema_provider::register_cinema_http_tools(
            &mut registry,
            std::path::PathBuf::from("/tmp"),
            reqwest::Client::new(),
            None,
        );

        crate::agent_runtime::studio_provider::register_studio_tools(&mut registry, std::path::PathBuf::from("/tmp"), None, None);
        let names: Vec<String> = registry.specs().into_iter().map(|s| s.name).collect();

        for skill in &all {
            for tool_pattern in &skill.tools {
                if let Some(prefix) = tool_pattern.strip_suffix('*') {
                    // 通配符至少要对上一个真实工具，不然等于白写。MCP 的（`server__*`）运行时才有，不查
                    assert!(prefix.contains("__") || names.iter().any(|n| n.starts_with(prefix)), "预设技能「{}」的「{}」一个工具都对不上", skill.id, tool_pattern);
                    continue;
                }
                assert!(
                    // tool_search 是每轮组装请求时才注册的（它的目录就是当时的全部工具）
                    tool_pattern == "tool_search" || registry.contains(tool_pattern) || crate::system_tools::is_system_tool(tool_pattern),
                    "预设技能「{}」声明的工具「{}」不存在于运行时注册表中！请检查拼写。",
                    skill.id,
                    tool_pattern
                );
            }
        }
    }

    /// 造一份最小可用的 SKILL.md。`marker` 是必填的 —— 漏了它整份文件会被
    /// 静默跳过，而那正是 init 曾经生成的那份死文件踩的坑之一。
    fn skill_md(id: &str, name: &str) -> String {
        format!("---\nid: {id}\nname: {name}\nmarker: 测试\ndescription: 用于测试\n---\n\n正文。\n")
    }

    #[test]
    fn 项目技能和全局技能都能被发现且标出来源() {
        let home = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        write(
            &home.path().join("skills/global-only/SKILL.md"),
            &skill_md("global-only", "全局的"),
        );
        write(
            &project_skills_dir(ws.path()).join("proj-only/SKILL.md"),
            &skill_md("proj-only", "项目的"),
        );

        let all = list_layered_in(Some(&project_skills_dir(ws.path())), &home.path().join("skills"));
        let ids: Vec<&str> = all.iter().map(|s| s.id.as_str()).collect();
        assert!(ids.contains(&"global-only"), "{ids:?}");
        assert!(ids.contains(&"proj-only"), "{ids:?}");

        let by = |id: &str| all.iter().find(|s| s.id == id).unwrap().scope;
        assert_eq!(by("proj-only"), SkillScope::Project);
        assert_eq!(by("global-only"), SkillScope::Global);
        // 项目层排在前面，界面可以直接分组
        assert_eq!(all[0].scope, SkillScope::Project);
    }

    #[test]
    fn 同id时项目层覆盖全局层() {
        let home = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        write(
            &home.path().join("skills/reviewer/SKILL.md"),
            &skill_md("reviewer", "全局审查员"),
        );
        write(
            &project_skills_dir(ws.path()).join("reviewer/SKILL.md"),
            &skill_md("reviewer", "本项目审查员"),
        );

        let all = list_layered_in(Some(&project_skills_dir(ws.path())), &home.path().join("skills"));
        // 只出现一次，不是两条同 id
        assert_eq!(all.iter().filter(|s| s.id == "reviewer").count(), 1, "{all:#?}");
        let win = all.iter().find(|s| s.id == "reviewer").unwrap();
        assert_eq!(win.name, "本项目审查员");
        assert_eq!(win.scope, SkillScope::Project);

        // resolve 走同一条路，不能和 list 分叉
        let r = resolve_layered_in(Some(&project_skills_dir(ws.path())), &home.path().join("skills"), "reviewer").unwrap();
        assert_eq!(r.name, "本项目审查员");
    }

    #[test]
    fn 不在项目里时看不到项目技能() {
        let home = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        write(
            &project_skills_dir(ws.path()).join("proj-only/SKILL.md"),
            &skill_md("proj-only", "项目的"),
        );
        // workspace = None：项目技能不该出现
        let all = list_layered_in(None, &home.path().join("skills"));
        assert!(all.iter().all(|s| s.id != "proj-only"), "{all:#?}");
        assert!(resolve_layered_in(None, &home.path().join("skills"), "proj-only").is_err());
    }

    #[test]
    fn scope不能被frontmatter自己声明() {
        // 「它是从哪来的」必须是发现过程的结论，不能是文件里的一句自称 ——
        // 否则一份放在全局目录里的 SKILL.md 可以把自己写成项目级的
        let home = tempfile::tempdir().unwrap();
        write(
            &home.path().join("skills/liar/SKILL.md"),
            "---\nid: liar\nname: 撒谎的\nmarker: 测试\ndescription: d\nscope: project\n---\n\n正文。\n",
        );
        let all = list_layered_in(None, &home.path().join("skills"));
        let s = all.iter().find(|s| s.id == "liar").expect("多余字段不该导致整份解析失败");
        assert_eq!(s.scope, SkillScope::Global, "frontmatter 里的 scope 必须被忽略");
    }

    #[test]
    fn 项目目录不存在时不影响全局技能() {
        // 绝大多数项目不会有 .anima/skills/ —— 那不该让技能列表变空
        let home = tempfile::tempdir().unwrap();
        write(
            &home.path().join("skills/g/SKILL.md"),
            &skill_md("g", "全局"),
        );
        let ws = tempfile::tempdir().unwrap();
        let all = list_layered_in(Some(&project_skills_dir(ws.path())), &home.path().join("skills"));
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "g");
    }

    #[test]
    fn 只有name和description的最小技能也能加载() {
        // **展示用字段的缺失不该让整份文件消失。** `list_in` 对解析失败是静默
        // 跳过的，所以「marker 必填」这一条曾经让 init 生成的那份技能
        // 一直躺在磁盘上没被加载过，一行报错都没有。
        let home = tempfile::tempdir().unwrap();
        write(
            &home.path().join("skills/minimal/SKILL.md"),
            "---\nname: 最小技能\n---\n\n正文。\n",
        );
        let all = list_layered_in(None, &home.path().join("skills"));
        let s = all
            .iter()
            .find(|s| s.id == "minimal")
            .expect("最小技能应当能加载（id 由目录名兜住）");
        assert_eq!(s.name, "最小技能");
        assert!(s.marker.is_empty(), "缺省的 marker 是空串，界面自己不渲染");
        assert!(s.description.is_empty());
        // 空白名单 = 不限制，不是"一个工具都不给"
        assert!(s.allows_tool("view_file"));
    }

    fn task_skill(id: &str, body: &str) -> Skill {
        Skill {
            name_zh: String::new(),
            marker_zh: String::new(),
            description_zh: String::new(),
            id: id.into(),
            name: format!("任务-{id}"),
            marker: String::new(),
            description: "d".into(),
            prefer_tier: None,
            thinking: None,
            tools: Vec::new(),
            source: String::new(),
            prompt: body.into(),
            kind: SkillKind::Task,
            surface: SkillSurface::Coding,
            scope: SkillScope::Project,
        }
    }

    #[test]
    fn 任务技能的展开是指令而不是资料() {
        // 单独一段 Markdown 进到工具结果里，模型很容易当成「要总结给用户的资料」，
        // 回一句"好的，这份规范要求……"然后停下。框架语就是为了挡这个。
        let s = task_skill("review", "审查 $ARGUMENTS，先跑 git_status。");
        let out = expand_task(&s, "src/lru.ts");
        assert!(out.contains("审查 src/lru.ts"), "参数没替换：{out}");
        assert!(out.contains("现在开始照它执行"), "缺少「照着做」的框架语");
        assert!(out.contains("不要把它复述给用户"), "缺少「别复述」");
        assert!(out.contains("本次的参数：src/lru.ts"));
    }

    #[test]
    fn 没有参数时不显示空的参数行() {
        let out = expand_task(&task_skill("t", "固定规程"), "   ");
        assert!(!out.contains("本次的参数"), "{out}");
        assert!(out.contains("固定规程"));
    }

    #[test]
    fn run_skill的参数解析要容忍字段名不一致() {
        // 本地小模型经常把 skill 写成 id/name、arguments 写成 args
        assert_eq!(
            run_from_arguments(r#"{"skill":"review","arguments":"a.ts"}"#),
            Some(("review".into(), "a.ts".into()))
        );
        assert_eq!(
            run_from_arguments(r#"{"id":"review","args":"a.ts"}"#),
            Some(("review".into(), "a.ts".into()))
        );
        // 省略 arguments 是合法的（规程可以不要参数）
        assert_eq!(
            run_from_arguments(r#"{"skill":"review"}"#),
            Some(("review".into(), String::new()))
        );
        // 解析不出来就返回 None，让调用方回一句人能看懂的话，而不是 panic
        for bad in [r#"{"skill":""}"#, r#"{"nope":1}"#, "不是 JSON", "{}"] {
            assert_eq!(run_from_arguments(bad), None, "{bad}");
        }
    }

    #[test]
    fn 声明里列出每个任务技能且标出要不要参数() {
        let a = task_skill("needs-args", "处理 $1");
        let b = task_skill("no-args", "固定规程");
        let d = run_declaration(&[a, b]);
        assert_eq!(d.name, RUN_SKILL_NAME);
        assert!(d.description.contains("`needs-args`"), "{}", d.description);
        assert!(d.description.contains("`no-args`"));
        // 「需要参数」只标在真的用了占位符的那个上
        let needs_line = d
            .description
            .lines()
            .find(|l| l.contains("needs-args"))
            .unwrap();
        assert!(needs_line.contains("需要参数"), "{needs_line}");
        let no_line = d.description.lines().find(|l| l.contains("no-args")).unwrap();
        assert!(!no_line.contains("需要参数"), "{no_line}");
        // enum 约束住 id，模型就不会去猜
        let ids = d.parameters["properties"]["skill"]["enum"].as_array().unwrap();
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn 按适用面和类型筛任务技能() {
        let mut companion_task = task_skill("chat-task", "b");
        companion_task.surface = SkillSurface::Companion;
        let mut both = task_skill("both-task", "b");
        both.surface = SkillSurface::Both;
        let coding = task_skill("code-task", "b");
        let mut persona = task_skill("a-persona", "b");
        persona.kind = SkillKind::Persona;

        let all = vec![companion_task, both, coding, persona];

        let in_coding: Vec<String> = tasks_for(&all, true).iter().map(|s| s.id.clone()).collect();
        let has = |v: &[String], id: &str| v.iter().any(|s| s == id);
        assert!(has(&in_coding, "code-task"), "{in_coding:?}");
        assert!(has(&in_coding, "both-task"), "{in_coding:?}");
        // 陪伴专用的任务不该出现在代码会话里
        assert!(!has(&in_coding, "chat-task"), "{in_coding:?}");
        // persona 不由 run_skill 载入，它是常驻的
        assert!(!has(&in_coding, "a-persona"), "{in_coding:?}");

        let ids: Vec<String> = tasks_for(&all, false).iter().map(|s| s.id.clone()).collect();
        assert!(has(&ids, "chat-task"), "{ids:?}");
        assert!(has(&ids, "both-task"), "{ids:?}");
        assert!(!has(&ids, "code-task"), "{ids:?}");
    }

    #[test]
    fn 白名单全过滤掉时要能被上层看见() {
        // 这就是 software-engineer 撞过的那个坑：白名单写的是 MCP 命名
        // （filesystem__*），而 coding 链路给的是裸名（view_file），
        // 结果一个工具都不剩且不报错。
        let mut s = task_skill("bad", "b");
        s.kind = SkillKind::Persona;
        s.tools = vec!["filesystem__*".into(), "system__*".into()];
        let tools = ["view_file", "apply_patch", "shell_command"]
            .into_iter()
            .map(|n| crate::llm::Tool {
                name: n.into(),
                description: String::new(),
                parameters: serde_json::json!({}),
            })
            .collect();
        let (kept, wiped) = narrow_tools(tools, std::slice::from_ref(&s));
        assert!(kept.is_empty());
        assert!(wiped, "全被过滤掉了必须能被上层看见");

        // 空白名单 = 不限制，不是"一个都不给"
        s.tools.clear();
        let tools = vec![crate::llm::Tool {
            name: "view_file".into(),
            description: String::new(),
            parameters: serde_json::json!({}),
        }];
        let (kept, wiped) = narrow_tools(tools, std::slice::from_ref(&s));
        assert_eq!(kept.len(), 1);
        assert!(!wiped);
    }
}
