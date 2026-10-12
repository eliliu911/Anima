use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::Path, sync::Mutex};
use tauri::State;
use crate::AppState;

static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// 一份文档是原文还是剧本。
///
/// **这两样东西在编辑器里长得像，但作用完全不同**：原文是小说/大纲/素材，
/// 是**输入**；剧本是分场分镜的成品，是**输出**，而且下游（抽实体、生成分镜）
/// 只认它。混在一张平铺的列表里，用户打开一份不知道它属于哪一类、
/// 下一步该干什么 —— 这就是「不明不白」。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptDocKind {
    /// 小说原稿、故事大纲、人物小传、素材。
    Source,
    /// **改编方案** —— 原文和剧本之间缺的那一层。
    ///
    /// # 为什么要有它
    ///
    /// 原来是「一段原文 → 一次提示词 → 剧本」。问题是**没人先决定
    /// 第 3 章和第 5 章该并成第几集** —— 模型每次自己现分，
    /// 于是同一本小说跑两次得到两套完全不同的分集，而且都不报错。
    ///
    /// drama-skills 把这一段拆成两步（`novel-analyze` 读懂、
    /// `develop` 定方案）才去写。我们合成一步：一份方案说清
    /// 「哪几章并成哪一集、每集要兑现什么、退出状态是什么」。
    ///
    /// **它不是剧本**，下游（抽实体、生成分镜）照样只认 `Screenplay`。
    Plan,
    /// 分集剧本。**下游只认这个。**
    #[default]
    Screenplay,
}

impl ScriptDocKind {
    pub fn label(self) -> &'static str {
        match self {
            ScriptDocKind::Source => "原文",
            ScriptDocKind::Plan => "改编方案",
            ScriptDocKind::Screenplay => "剧本",
        }
    }
}

/// 这份剧本是从哪份原文生成的。
///
/// 存下来是为了**可追溯、可重生成** —— 改了原文之后能找回来重跑一次，
/// 而不是从头再来。
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DerivedFrom {
    /// 主来源。跨多章时是**第一章** —— 「回到源头」跳到它。
    pub source_doc_id: String,
    /// 还用了哪几份（不含 `source_doc_id`）。
    ///
    /// **章节和方案不是一对一的。** 一章可能出两集，几章可能并成一集，
    /// 十个字的一章根本撑不起一集 —— 得和下一章并起来。
    /// 只记一个 id 的话，「这份方案覆盖了哪些章」就丢了，
    /// 而那正是改了原文之后知道该重跑哪一份的依据。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub also_source_doc_ids: Vec<String>,
    /// 原文里的哪一段（章节标题，或者「第 1200~3400 字」这种）。
    #[serde(default)]
    pub range: String,
    /// 用的哪个提示词槽位 / 版本，重生成时对得上。
    #[serde(default)]
    pub recipe: String,
    /// 生成时间（毫秒）。原文后来改过的话，这个时间就是「过期」的判据。
    #[serde(default)]
    pub at: u64,
    /// 生成落盘那一刻的 `revision`。
    ///
    /// # 它是「有没有被人手改过」的**精确**判据
    ///
    /// 重新生成要覆盖上一份，但**手改过的那份绝不能覆盖** ——
    /// 静默冲掉用户写了半天的稿子是这里面最不能接受的事。
    ///
    /// 一度想用时间差（`updated_at` 比 `at` 晚多少算手改），但任何时间
    /// 边界都必然误判：边界内的真编辑被当成生成本身冲掉，边界外的
    /// 生成本身被当成手改而白留一份。版本号没有这个问题 ——
    /// `revision != gen_revision` 就是**确凿**地被改过。
    ///
    /// `0` = 旧数据没有这个字段，**当成手改过**（保守方向：宁可多留
    /// 一份，不可冲掉）。
    #[serde(default)]
    pub gen_revision: u64,
}

/// 一个剧本项目。
///
/// **和视频项目是两回事，而且它不知道视频项目的存在。**
/// 可以只写剧本、完全不建视频项目；等剧本成了再「送去建视频项目」。
/// 反过来不成立 —— 视频项目引用剧本，不是剧本挂在视频项目下面。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScriptProject {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default)]
    pub updated_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScriptDocument {
    pub id: String,
    pub title: String,
    pub text: String,
    pub doc: Option<String>,
    pub revision: u64,
    /// 属于哪个剧本项目。**`None` = 没归档的散落文档** ——
    /// 这正是所有存量数据的状态，所以它必须有默认值，不能要求迁移。
    #[serde(default)]
    pub project_id: Option<String>,
    /// 原文还是剧本。**默认剧本** —— 存量文档就是当剧本用的。
    #[serde(default)]
    pub kind: ScriptDocKind,
    #[serde(default)]
    pub derived_from: Option<DerivedFrom>,
    /// 上次保存的时刻（毫秒）。**存量数据是 0** ——
    /// `#[serde(default)]` 兜住，下次保存时自己就有了。
    ///
    /// 有它才排得了「最近改动」。`revision` 顶不了这个用：
    /// 它是**改过几次**，不是**什么时候改的** —— 一份改过 30 次的旧稿
    /// 会排在昨天刚碰过一次的新稿前面。
    #[serde(default)]
    pub updated_at: u64,
}

fn validate(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > 80 || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
        return Err("无效的剧本文档 ID".into());
    }
    Ok(())
}

fn list(root: &Path) -> Result<Vec<ScriptDocument>, String> {
    if !root.exists() { return Ok(vec![]); }
    let mut docs = Vec::new();
    for entry in fs::read_dir(root).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") { continue; }
        // **项目清单不是文档。** 它和文档放在同一个目录里，扩展名也一样 ——
        // 不跳过的话，建了第一个项目之后整个剧本库就读不出来了
        // （报的是「invalid length 0, expected struct ScriptDocument」，
        // 看不出跟项目有关）。测试里撞上的。
        if path.file_name().and_then(|s| s.to_str()) == Some("projects.json") { continue; }
        let bytes = fs::read(&path).map_err(|e| e.to_string())?;
        docs.push(serde_json::from_slice(&bytes).map_err(|e| format!("无法读取 {}：{e}", path.display()))?);
    }
    Ok(docs)
}

fn save(root: &Path, mut document: ScriptDocument) -> Result<ScriptDocument, String> {
    validate(&document.id)?;
    let _guard = WRITE_LOCK.lock().map_err(|e| e.to_string())?;
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let target = root.join(format!("{}.json", document.id));
    let current = if target.exists() {
        let stored: ScriptDocument = serde_json::from_slice(&fs::read(&target).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        stored.revision
    } else { 0 };
    if current != document.revision { return Err("剧本已被其他窗口修改，请重新打开后合并；当前修改尚未保存".into()); }
    document.revision += 1;
    // **时间由这一层盖，不信前端传来的。** 前端传什么都可能，
    // 而排序要的是「真的什么时候写进磁盘」。
    document.updated_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let temp = root.join(format!("{}.tmp", document.id));
    let result = (|| {
        let bytes = serde_json::to_vec_pretty(&document).map_err(|e| e.to_string())?;
        let mut file = fs::File::create(&temp).map_err(|e| e.to_string())?;
        file.write_all(&bytes).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        fs::rename(&temp, &target).map_err(|e| e.to_string())
    })();
    if result.is_err() { let _ = fs::remove_file(&temp); }
    result?;
    Ok(document)
}

fn projects_file(root: &Path) -> std::path::PathBuf {
    root.join("projects.json")
}

fn list_projects(root: &Path) -> Result<Vec<ScriptProject>, String> {
    let f = projects_file(root);
    match fs::read(&f) {
        Ok(b) => serde_json::from_slice(&b).map_err(|e| format!("剧本项目清单损坏：{e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(e.to_string()),
    }
}

fn save_projects(root: &Path, projects: &[ScriptProject]) -> Result<(), String> {
    let _guard = WRITE_LOCK.lock().map_err(|e| e.to_string())?;
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let temp = root.join("projects.tmp");
    let bytes = serde_json::to_vec_pretty(projects).map_err(|e| e.to_string())?;
    let mut file = fs::File::create(&temp).map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    fs::rename(&temp, projects_file(root)).map_err(|e| e.to_string())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[tauri::command]
pub fn studio_script_list(state: State<'_, AppState>) -> Result<Vec<ScriptDocument>, String> {
    list(&state.config_dir.join("studio/scripts"))
}

#[tauri::command]
pub fn studio_script_projects(state: State<'_, AppState>) -> Result<Vec<ScriptProject>, String> {
    list_projects(&state.config_dir.join("studio/scripts"))
}

#[tauri::command]
pub fn studio_script_project_save(
    state: State<'_, AppState>,
    project: ScriptProject,
) -> Result<ScriptProject, String> {
    let root = state.config_dir.join("studio/scripts");
    validate(&project.id)?;
    let mut all = list_projects(&root)?;
    let mut p = project;
    p.updated_at = now_ms();
    match all.iter_mut().find(|x| x.id == p.id) {
        Some(existing) => {
            p.created_at = existing.created_at;
            *existing = p.clone();
        }
        None => {
            if p.created_at == 0 {
                p.created_at = p.updated_at;
            }
            all.push(p.clone());
        }
    }
    save_projects(&root, &all)?;
    Ok(p)
}

/// 删掉一个剧本项目。**里面的文档不删**，只是回到「散落」状态。
///
/// 删项目就把一沓原稿一起删掉是不可接受的 —— 用户点「删除项目」时想的是
/// 「这个分类我不要了」，不是「把我写的东西烧了」。
#[tauri::command]
pub fn studio_script_project_delete(
    state: State<'_, AppState>,
    project_id: String,
) -> Result<usize, String> {
    let root = state.config_dir.join("studio/scripts");
    validate(&project_id)?;
    let mut all = list_projects(&root)?;
    let before = all.len();
    all.retain(|p| p.id != project_id);
    if all.len() == before {
        return Err("剧本项目不存在".into());
    }
    save_projects(&root, &all)?;

    // 把归属解开，文档留着
    let mut detached = 0usize;
    for mut d in list(&root)? {
        if d.project_id.as_deref() == Some(project_id.as_str()) {
            d.project_id = None;
            save(&root, d)?;
            detached += 1;
        }
    }
    Ok(detached)
}

/// 删掉一份文档。返回**因此失去源头**的那几份的标题。
///
/// # 派生出去的稿子不跟着删
///
/// 删掉第 01 章时，从它生成的方案和剧本**留着** —— 那两份可能已经被
/// 手改过很多，用户删的是素材，不是成果。
///
/// 但它们的 `derived_from.source_doc_id` 会指向一个不存在的文档，
/// 所以这里把受影响的标题**报回去**，界面上要说出来。
/// 不说的话，那几份稿子的「回到源头」会静默地跳不动 ——
/// 而这正是这个仓库最典型的失败形状。
///
/// 参考 `ScriptTree.groupDocuments` 里同一条：归属指向不存在的项目时
/// 当成未归档，**宁可显示在错的分组，也不能让它人间蒸发**。
/// 删掉 `id` 之后，哪几份稿子会失去源头。
///
/// **合并来源也算。** 一份方案可能是「第01章 + 第02章」并起来的
/// （`also_source_doc_ids`）—— 只比主来源的话，删掉第 02 章不会报出来，
/// 而那份方案的来源确实缺了一块。
fn orphaned_by(all: &[ScriptDocument], id: &str) -> Vec<String> {
    all.iter()
        .filter(|d| {
            d.derived_from.as_ref().is_some_and(|f| {
                f.source_doc_id == id || f.also_source_doc_ids.iter().any(|x| x == id)
            })
        })
        .map(|d| d.title.clone())
        .collect()
}

#[tauri::command]
pub fn studio_script_delete(
    state: State<'_, AppState>,
    id: String,
) -> Result<Vec<String>, String> {
    let root = state.config_dir.join("studio/scripts");
    validate(&id)?;
    let all = list(&root)?;
    if !all.iter().any(|d| d.id == id) {
        return Err("这份文档不存在 —— 可能已经在另一个窗口里删掉了".into());
    }
    let orphaned = orphaned_by(&all, &id);

    let _guard = WRITE_LOCK.lock().map_err(|e| e.to_string())?;
    let target = root.join(format!("{id}.json"));
    fs::remove_file(&target).map_err(|e| format!("删不掉：{e}"))?;
    Ok(orphaned)
}

/// 新建一份剧本文档（剧本库，不绑项目）。Chat 的「存剧本」工具用
pub fn create_screenplay_doc(config_dir: &std::path::Path, title: &str, text: &str) -> Result<ScriptDocument, String> {
    let doc = ScriptDocument {
        id: uuid::Uuid::new_v4().simple().to_string(),
        title: if title.trim().is_empty() { "未命名剧本".into() } else { title.trim().to_string() },
        text: text.to_string(),
        doc: None,
        revision: 0,
        project_id: None,
        kind: ScriptDocKind::Screenplay,
        derived_from: None,
        updated_at: 0,
    };
    save(&config_dir.join("studio/scripts"), doc)
}

#[tauri::command]
pub fn studio_script_save(state: State<'_, AppState>, document: ScriptDocument) -> Result<ScriptDocument, String> {
    save(&state.config_dir.join("studio/scripts"), document)
}

/// 从一份原文生成剧本，存成同项目下的新文档。
///
/// **不覆盖原文，也不覆盖已有剧本** —— 每次生成都是一份新文档，
/// 带上 `derived_from` 记着它从哪来。改了原文想重跑一次时，
/// 靠那条记录找得回来源，而不是从头再来。
///
/// # 下一步由「你站在哪份文档上」决定，不是另开一个命令
///
/// | 站在 | 生成 |
/// |---|---|
/// | 原文 | **改编方案**（先决定怎么分集） |
/// | 改编方案 | 剧本（连同它出自的那份原文一起发过去） |
/// | 剧本 | 剧本（重写一版） |
///
/// 这么设计是因为上游 SKILL.md 留了一条口子：「开发大纲和长篇分析都是
/// **可选**上游；当前材料足够时直接写」。短素材不该被逼着先出方案 ——
/// 想直接写就在方案那一步点「跳过」。
/// 原文 → 改编方案。存成同项目下的新文档，`derived_from` 指着原文。
/// 同一个项目下所有原文，按标题自然序排好。
///
/// # 只用来取**目录**，不用来拼正文
///
/// 这里有过一次过度修正。起因是只喂第 01 章时模型把后面的章节编出来了
/// （规划 12 集、标着「用第02章」、还断言「无遗漏」），我当时的对策是
/// **把整个项目的原文一起发** —— 那是错的，用户当场指出三条：
///
/// 1. 点的是第一章，却把六章全处理了 —— **我点了什么就该处理什么**；
/// 2. 五百万字的连载**根本发不进去**，这条路从一开始就不可扩展；
/// 3. `derived_from` 指着第 01 章，内容却覆盖六章，**自相矛盾**。
///
/// 真正的病因是提示词没说「不许规划没给的章节」，不是上下文不够。
/// 那一条补上就够了。这里现在只用来取**章节目录**（标题，不含正文）：
/// 让模型知道「后面还有」，但不让它去读、也不让它去编。
fn all_sources_of(project: Option<&str>, all: &[ScriptDocument]) -> Vec<ScriptDocument> {
    let mut v: Vec<ScriptDocument> = all
        .iter()
        .filter(|d| d.kind == ScriptDocKind::Source && d.project_id.as_deref() == project)
        .cloned()
        .collect();
    // 章节顺序要对。**按数值比不按字典序** —— 「第10章」不能排在「第2章」前面
    v.sort_by(|a, b| natural_cmp(&a.title, &b.title));
    v
}

/// 章节目录 —— **只有标题**，当前这一章标出来。
///
/// 五百万字的连载可能有几千章，目录本身也会撑爆，所以只取当前这一章
/// 前后各 [`TOC_WINDOW`] 章。前后被截掉时明写「…（前面还有 N 章）」，
/// **不能让它以为这就是全部** —— 那正是「把这一章当成全篇、硬凑大结局」
/// 的来源。
fn chapter_toc(chapters: &[ScriptDocument], here: Option<usize>) -> String {
    if chapters.len() <= 1 {
        return "这个项目里目前只有这一章。".into();
    }
    let i = here.unwrap_or(0);
    let lo = i.saturating_sub(TOC_WINDOW);
    let hi = (i + TOC_WINDOW + 1).min(chapters.len());
    let mut lines = Vec::new();
    if lo > 0 {
        lines.push(format!("…（前面还有 {lo} 章）"));
    }
    for (n, d) in chapters.iter().enumerate().take(hi).skip(lo) {
        let mark = if n == i { "  ← 现在要规划的就是这一章" } else { "" };
        lines.push(format!("- {}{mark}", d.title.trim()));
    }
    if hi < chapters.len() {
        lines.push(format!("…（后面还有 {} 章）", chapters.len() - hi));
    }
    format!("这个项目的章节目录（**只有标题，正文没给你**）：\n{}", lines.join("\n"))
}

/// 目录里当前这一章前后各显示几章。
pub const TOC_WINDOW: usize = 5;

/// 自然序：数字段按数值比。和前端 `sortDocs.ts` 的 `naturalCompare` 同一套。
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    fn chunks(s: &str) -> Vec<Result<u64, String>> {
        let mut out = Vec::new();
        let mut num = String::new();
        let mut txt = String::new();
        for c in s.chars() {
            if c.is_ascii_digit() {
                if !txt.is_empty() {
                    out.push(Err(std::mem::take(&mut txt)));
                }
                num.push(c);
            } else {
                if !num.is_empty() {
                    out.push(Ok(num.parse().unwrap_or(0)));
                    num.clear();
                }
                txt.push(c);
            }
        }
        if !num.is_empty() {
            out.push(Ok(num.parse().unwrap_or(0)));
        }
        if !txt.is_empty() {
            out.push(Err(txt));
        }
        out
    }
    let (x, y) = (chunks(a), chunks(b));
    for i in 0..x.len().max(y.len()) {
        match (x.get(i), y.get(i)) {
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(Ok(p)), Some(Ok(q))) if p != q => return p.cmp(q),
            (Some(Err(p)), Some(Err(q))) if p != q => return p.cmp(q),
            _ => {}
        }
    }
    std::cmp::Ordering::Equal
}

async fn plan_from_source(
    state: &State<'_, AppState>,
    root: &Path,
    source: &ScriptDocument,
    all: &[ScriptDocument],
    extra: &[String],
    note: &str,
) -> Result<ScriptDocument, String> {
    let cfg = crate::prompts::load(&state.config_dir);
    let note_line = if note.trim().is_empty() {
        String::new()
    } else {
        format!("额外要求：{}", note.trim())
    };
    // **只发这一章的正文。** 点的是哪一章就处理哪一章 ——
    // 一章一集（或两集）是改编的自然单位，连载动漫也是这么做的。
    // 整本一起发既违背「我点了什么就处理什么」，也扛不住五百万字的连载。
    //
    // 但要让它知道**后面还有**，否则它会把这一章当成全篇、
    // 硬凑一个大结局。所以给目录 —— 只有标题，没有正文。
    let chapters = all_sources_of(source.project_id.as_deref(), all);
    // 用户勾了几章就一起发几章，**按章号排** —— 勾选顺序不是阅读顺序。
    let mut picked: Vec<&ScriptDocument> = chapters
        .iter()
        .filter(|d| d.id == source.id || extra.contains(&d.id))
        .collect();
    picked.sort_by(|a, b| natural_cmp(&a.title, &b.title));
    let body = picked
        .iter()
        .map(|d| format!("## {}\n\n{}", d.title.trim(), d.text))
        .collect::<Vec<_>>()
        .join("\n\n");
    let names = picked.iter().map(|d| d.title.trim()).collect::<Vec<_>>().join("、");
    let here = chapters.iter().position(|d| d.id == source.id);
    let toc = chapter_toc(&chapters, here);
    let note_line = format!(
        "{note_line}\n\n{toc}\n\n**这次给了你正文的是：{names}（共 {} 章）。\
         只规划这些。** 目录只是让你知道前后有什么，不要为没给正文的章节排集。",
        picked.len(),
    );
    let raw = crate::prompts::run(
        &state.gateway,
        &cfg,
        crate::prompts::PromptSlot::AdaptationPlan,
        &[("TEXT", &body), ("NOTE", &note_line)],
    )
    .await
    .map_err(|e| e.to_string())?;
    let text = strip_code_fence(&raw);
    if text.trim().is_empty() {
        return Err("模型没有返回改编方案".into());
    }
    // **重新生成盖掉上一份**（手改过的除外）—— 见 `slot_for`。
    let plan_sources: Vec<String> = std::iter::once(source.id.clone())
        .chain(picked.iter().filter(|d| d.id != source.id).map(|d| d.id.clone()))
        .collect();
    let (slot_id, slot_rev) =
        slot_for(all, source.project_id.as_deref(), ScriptDocKind::Plan, &plan_sources);
    save(
        root,
        ScriptDocument {
            id: slot_id,
            // 覆盖多章时标题写成「第01章~第03章（改编方案）」——
            // 只写第一章的话，列表上看不出它管到哪儿。
            title: match picked.len() {
                0 | 1 => format!("{}（改编方案）", source.title.trim()),
                _ => format!(
                    "{}~{}（改编方案）",
                    picked[0].title.trim(),
                    picked[picked.len() - 1].title.trim()
                ),
            },
            text,
            doc: None,
            revision: slot_rev,
            updated_at: 0,
            project_id: source.project_id.clone(),
            kind: ScriptDocKind::Plan,
            derived_from: Some(DerivedFrom {
                source_doc_id: source.id.clone(),
                // **覆盖了哪几章要记全。** 只记一个 id 的话，
                // 改了其中一章之后没法知道该重跑哪一份方案。
                also_source_doc_ids: picked
                    .iter()
                    .filter(|d| d.id != source.id)
                    .map(|d| d.id.clone())
                    .collect(),
                range: format!("{names}，共 {} 字", body.chars().count()),
                recipe: "adaptation_plan".into(),
                at: now_ms(),
                gen_revision: slot_rev + 1,
            }),
        },
    )
}

#[tauri::command]
pub async fn studio_script_generate(
    state: State<'_, AppState>,
    source_doc_id: String,
    note: String,
    // `also` = 还要一起规划的章节。**章节和集数不是一对一** ——
    // 几章讲同一件事就该并成一集，而十个字的一章撑不起任何一集。
    // （参数上不能写 doc 注释，所以写在这儿。）
    also: Option<Vec<String>>,
) -> Result<ScriptDocument, String> {
    let root = state.config_dir.join("studio/scripts");
    validate(&source_doc_id)?;
    let all = list(&root)?;
    let source = all
        .iter()
        .find(|d| d.id == source_doc_id)
        .cloned()
        .ok_or("原文不存在")?;
    if source.text.trim().is_empty() {
        return Err("这份原文是空的".into());
    }

    // 站在原文上 → 先出改编方案。**不直接写剧本** ——
    // 没人先决定哪几章并成哪一集的话，模型每次自己现分，
    // 同一本小说跑两次得到两套完全不同的分集，而且都不报错。
    if source.kind == ScriptDocKind::Source {
        let extra = also.unwrap_or_default();
        for id in &extra {
            validate(id)?;
        }
        return plan_from_source(&state, &root, &source, &all, &extra, &note).await;
    }

    // 站在方案上 → 把它出自的那份原文一起发过去。
    // **只给方案不给原文写不出剧本** —— 方案里没有细节、没有语气、
    // 没有人物说话的样子，那些只在原文里。
    // **方案可能是多章合起来做的 —— 原文要全都回填。**
    //
    // `plan_from_source` 会把用户勾的几章合并，存成 `source_doc_id` +
    // `also_source_doc_ids`。而这里原来只找主来源那一章 ——
    // 于是提示词一边说「照这份多章方案写」，一边只给了一章的原文。
    // 模型只能漏掉后面的情节，或者**凭空补细节**，而这两样都不报错
    // （2026-09-19 Codex Astra 的审查抓到）。
    //
    // 按章号排，和当初做方案时的顺序一致 —— 勾选顺序不是阅读顺序。
    let upstream: Vec<&ScriptDocument> = if source.kind == ScriptDocKind::Plan {
        source
            .derived_from
            .as_ref()
            .map(|d| {
                let mut v: Vec<&ScriptDocument> = all
                    .iter()
                    .filter(|x| x.id == d.source_doc_id || d.also_source_doc_ids.iter().any(|a| a == &x.id))
                    .collect();
                v.sort_by(|a, b| natural_cmp(&a.title, &b.title));
                v
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    // **少了任何一章就明说，不要当成正常改编请求。**
    // 悄悄少一章的表现是「后半段情节没了」，而那时候完全看不出是输入缺的。
    if let Some(d) = source.derived_from.as_ref().filter(|_| source.kind == ScriptDocKind::Plan) {
        let want = 1 + d.also_source_doc_ids.len();
        if upstream.len() < want {
            return Err(format!(
                "这份方案是 {want} 章原文合起来做的，现在只找得到 {} 章 —— 缺的那几章是不是被删了？补回来再写，否则后半段只能靠编",
                upstream.len()
            ));
        }
    }

    let cfg = crate::prompts::load(&state.config_dir);
    let note_line = if note.trim().is_empty() {
        String::new()
    } else {
        format!("额外要求：{}", note.trim())
    };
    // **手艺按需给。**
    //
    // 检测的是**原文**（加上用户那句额外要求），不是我们想要什么 ——
    // 上游 SKILL.md 的原话：手艺「只帮助实现这些要求，不能反过来补造
    // 题面没有的精确数字、记录、资源、程序、关系或机关」。
    // 拿我们的意图去挑手艺，等于让模型往故事里塞它本来没有的东西。
    // 方案在前、原文在后，中间说清哪是哪 —— 不说的话模型分不出
    // 「这是要照着做的方案」和「这是素材」。
    let body = if upstream.is_empty() {
        source.text.clone()
    } else {
        // 多章时逐章标题分段 —— 不标的话模型分不出章与章的边界，
        // 会把两章的情节揉成一段
        let orig = upstream
            .iter()
            .map(|d| format!("### {}\n\n{}", d.title.trim(), d.text))
            .collect::<Vec<_>>()
            .join("\n\n");
        format!(
            "这是已经定好的**改编方案**，照它分集：\n\n{}\n\n             ---\n\n下面是方案对应的**原文素材**（共 {} 章，按章号排），细节、语气和人物怎么说话从这里取：\n\n{}",
            source.text,
            upstream.len(),
            orig
        )
    };
    let craft = crate::screenwriter::craft_block(&format!("{} {}", body, note.trim()));
    let raw = crate::prompts::run(
        &state.gateway,
        &cfg,
        crate::prompts::PromptSlot::ScreenplayFromSource,
        &[("TEXT", &body), ("NOTE", &note_line), ("CRAFT", &craft)],
    )
    .await
    .map_err(|e| e.to_string())?;

    let text = strip_code_fence(&raw);
    if text.trim().is_empty() {
        return Err("模型没有返回剧本内容".into());
    }

    let (slot_id, slot_rev) = slot_for(
        &all,
        source.project_id.as_deref(),
        ScriptDocKind::Screenplay,
        std::slice::from_ref(&source.id),
    );
    let doc = ScriptDocument {
        id: slot_id,
        title: format!("{}（剧本）", source.title.trim()),
        text,
        doc: None,
        revision: slot_rev,
        project_id: source.project_id.clone(),
        kind: ScriptDocKind::Screenplay,
        // `save` 会盖掉它，这里给个值只是为了字段齐全
        updated_at: 0,
        derived_from: Some(DerivedFrom {
            source_doc_id: source.id.clone(),
            also_source_doc_ids: Vec::new(),
            range: format!("全文 {} 字", source.text.chars().count()),
            recipe: "screenplay_from_source".into(),
            at: now_ms(),
            // **落盘之后的版本号** —— `save` 会把 revision 加一。
            // 手改过之后两者对不上，那份就不会被下次生成盖掉。
            gen_revision: slot_rev + 1,
        }),
    };
    save(&root, doc)
}

/// 重新生成时，要不要盖掉上一份。
///
/// # 为什么默认是盖
///
/// 以前每次生成都 `Uuid::new_v4()` 建一份新的，而文档层**没有删除**——
/// 对同一章点两次「生成方案」就得到两份，越攒越多且清不掉。
/// 用户 2026-09-12 反馈的就是这个。
///
/// # 手改过的绝不盖
///
/// 静默冲掉人写了半天的稿子，是这一块里最不能接受的事。判据是
/// [`DerivedFrom::gen_revision`]：`revision != gen_revision` 就是确凿地
/// 被改过。**旧数据没有这个字段（0），一律当手改过** ——
/// 保守方向是宁可多留一份，不可冲掉。
///
/// # 什么算「上一份」
///
/// 同项目、同类型、**而且出自同一组原文**。少一条都不行：
/// 不比项目会跨项目乱盖；不比类型会让方案盖掉剧本；
/// 不比来源的话，第 02 章的方案会盖掉第 01 章的。
fn previous_generated<'a>(
    all: &'a [ScriptDocument],
    project_id: Option<&str>,
    kind: ScriptDocKind,
    sources: &[String],
) -> Option<&'a ScriptDocument> {
    let want: std::collections::BTreeSet<&str> = sources.iter().map(|s| s.as_str()).collect();
    all.iter().find(|d| {
        if d.project_id.as_deref() != project_id || d.kind != kind {
            return false;
        }
        let Some(from) = &d.derived_from else {
            return false;
        };
        // 手改过的不算「上一份」—— 它会被留下，这次另建一份。
        if from.gen_revision == 0 || from.gen_revision != d.revision {
            return false;
        }
        let have: std::collections::BTreeSet<&str> = std::iter::once(from.source_doc_id.as_str())
            .chain(from.also_source_doc_ids.iter().map(|s| s.as_str()))
            .collect();
        have == want
    })
}

/// 这次生成落到哪个 id / 从哪个 revision 续。
///
/// 返回 `(id, revision)`：找得到可盖的上一份就沿用它的，
/// 否则是一个新 id 和 `revision: 0`。
fn slot_for(
    all: &[ScriptDocument],
    project_id: Option<&str>,
    kind: ScriptDocKind,
    sources: &[String],
) -> (String, u64) {
    match previous_generated(all, project_id, kind, sources) {
        Some(d) => (d.id.clone(), d.revision),
        None => (uuid::Uuid::new_v4().to_string(), 0),
    }
}

/// 模型爱把结果包在 ``` 里。**剥掉** —— 那三个反引号会原样进剧本，
/// 然后被当成动作描写，最后进提示词被画进画面。
pub fn strip_code_fence(raw: &str) -> String {
    let t = raw.trim();
    let Some(rest) = t.strip_prefix("```") else {
        return t.to_string();
    };
    // 跳过 ```后面可能跟的语言标记那一行
    let body = match rest.find('\n') {
        Some(i) => &rest[i + 1..],
        None => return t.to_string(),
    };
    body.trim_end()
        .strip_suffix("```")
        .unwrap_or(body)
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    /// **目录只取标题，用来告诉模型「后面还有」。**
    ///
    /// 不给目录的话它会把这一章当成全篇、硬凑一个大结局；
    /// 给正文的话既违背「点了什么就处理什么」，也扛不住五百万字的连载。
    #[test]
    fn 目录只取本项目的原文并按章号排() {
        let d = |id: &str, title: &str, kind: ScriptDocKind, pid: Option<&str>| ScriptDocument {
            id: id.into(), title: title.into(), text: "正文".into(),
            doc: None, revision: 0, updated_at: 0,
            project_id: pid.map(str::to_string), kind, derived_from: None,
        };
        let all = vec![
            d("a", "第10章", ScriptDocKind::Source, Some("p1")),
            d("b", "第02章", ScriptDocKind::Source, Some("p1")),
            d("c", "第01章", ScriptDocKind::Source, Some("p1")),
            d("x", "第01章", ScriptDocKind::Source, Some("p2")),   // 别的项目
            d("y", "第 1 集", ScriptDocKind::Screenplay, Some("p1")), // 剧本不算原文
            d("z", "方案", ScriptDocKind::Plan, Some("p1")),
        ];
        let got = all_sources_of(Some("p1"), &all);
        // **只拿本项目的原文**，剧本和方案不算
        assert_eq!(got.iter().map(|x| x.title.as_str()).collect::<Vec<_>>(),
                   vec!["第01章", "第02章", "第10章"]);
    }

    // ---- 重新生成盖不盖上一份 ------------------------------------------

    fn 派生(
        id: &str,
        kind: ScriptDocKind,
        revision: u64,
        gen_revision: u64,
        sources: &[&str],
    ) -> ScriptDocument {
        ScriptDocument {
            id: id.into(),
            title: id.into(),
            text: "正文".into(),
            doc: None,
            revision,
            updated_at: 0,
            project_id: Some("p1".into()),
            kind,
            derived_from: Some(DerivedFrom {
                source_doc_id: sources[0].into(),
                also_source_doc_ids: sources[1..].iter().map(|s| s.to_string()).collect(),
                range: String::new(),
                recipe: String::new(),
                at: 0,
                gen_revision,
            }),
        }
    }

    /// 没手改过的上一份要被盖掉 —— 否则越攒越多。
    #[test]
    fn 重新生成盖掉没动过的上一份() {
        let all = vec![派生("旧方案", ScriptDocKind::Plan, 1, 1, &["ch1"])];
        let (id, rev) = slot_for(&all, Some("p1"), ScriptDocKind::Plan, &["ch1".into()]);
        assert_eq!(id, "旧方案");
        assert_eq!(rev, 1, "要从上一份的版本号续，不然 save 会判冲突");
    }

    /// **手改过的绝不盖。** 静默冲掉人写了半天的稿子是这里最不能接受的事。
    #[test]
    fn 手改过的那份不被盖掉() {
        let all = vec![派生("改过的方案", ScriptDocKind::Plan, 5, 1, &["ch1"])];
        let (id, rev) = slot_for(&all, Some("p1"), ScriptDocKind::Plan, &["ch1".into()]);
        assert_ne!(id, "改过的方案");
        assert_eq!(rev, 0);
    }

    /// 旧数据没有 `gen_revision`（0），**一律当手改过** ——
    /// 保守方向是宁可多留一份，不可冲掉。
    #[test]
    fn 旧数据当成手改过() {
        let all = vec![派生("老方案", ScriptDocKind::Plan, 1, 0, &["ch1"])];
        let (id, _) = slot_for(&all, Some("p1"), ScriptDocKind::Plan, &["ch1".into()]);
        assert_ne!(id, "老方案");
    }

    /// **来源不同不算上一份。** 不比来源的话，第 02 章的方案会盖掉第 01 章的。
    #[test]
    fn 来源不同的不互相盖() {
        let all = vec![派生("第一章的方案", ScriptDocKind::Plan, 1, 1, &["ch1"])];
        let (id, _) = slot_for(&all, Some("p1"), ScriptDocKind::Plan, &["ch2".into()]);
        assert_ne!(id, "第一章的方案");
    }

    /// 合并章数变了也不算同一份 —— 「01+02 合并」和「只有 01」是两回事。
    #[test]
    fn 合并的章数变了就另起一份() {
        let all = vec![派生("合并方案", ScriptDocKind::Plan, 1, 1, &["ch1", "ch2"])];
        let (id, _) = slot_for(&all, Some("p1"), ScriptDocKind::Plan, &["ch1".into()]);
        assert_ne!(id, "合并方案");
        // 反过来，一模一样的一组（顺序不同也算）要认得出来
        let (same, _) = slot_for(
            &all,
            Some("p1"),
            ScriptDocKind::Plan,
            &["ch2".into(), "ch1".into()],
        );
        assert_eq!(same, "合并方案", "同一组来源换个顺序应该还是同一份");
    }

    /// **类型不同不算上一份。** 不比类型的话，方案会盖掉剧本。
    #[test]
    fn 方案不会盖掉剧本() {
        let all = vec![派生("剧本", ScriptDocKind::Screenplay, 1, 1, &["ch1"])];
        let (id, _) = slot_for(&all, Some("p1"), ScriptDocKind::Plan, &["ch1".into()]);
        assert_ne!(id, "剧本");
    }

    /// **项目不同不算上一份。** 不比项目会跨项目乱盖。
    #[test]
    fn 不跨项目盖() {
        let all = vec![派生("别的项目的方案", ScriptDocKind::Plan, 1, 1, &["ch1"])];
        let (id, _) = slot_for(&all, Some("p2"), ScriptDocKind::Plan, &["ch1".into()]);
        assert_ne!(id, "别的项目的方案");
    }

    /// **删掉源头时，受影响的稿子要报出来。**
    ///
    /// 不报的话，那几份的「回到源头」会静默地跳不动 ——
    /// 这个仓库最典型的失败形状。
    #[test]
    fn 删源头要报出谁失去了源头() {
        let all = vec![
            派生("第01章的方案", ScriptDocKind::Plan, 1, 1, &["ch1"]),
            派生("01+02合并的方案", ScriptDocKind::Plan, 1, 1, &["ch1", "ch2"]),
            派生("第03章的方案", ScriptDocKind::Plan, 1, 1, &["ch3"]),
        ];
        let mut hit = orphaned_by(&all, "ch1");
        hit.sort();
        assert_eq!(hit, vec!["01+02合并的方案", "第01章的方案"]);
    }

    /// **合并来源也算。** 只比主来源的话，删掉第 02 章不会报出来，
    /// 而那份方案的来源确实缺了一块。
    #[test]
    fn 合并来源里的那一份也算失去源头() {
        let all = vec![派生("01+02合并的方案", ScriptDocKind::Plan, 1, 1, &["ch1", "ch2"])];
        assert_eq!(orphaned_by(&all, "ch2"), vec!["01+02合并的方案"]);
    }

    /// 没人从它派生时不虚报 —— 多一句吓人的提示比不提示更糟。
    #[test]
    fn 没人依赖它时不报() {
        let all = vec![派生("别的方案", ScriptDocKind::Plan, 1, 1, &["ch9"])];
        assert!(orphaned_by(&all, "ch1").is_empty());
    }

    /// **第10章不能排在第2章前面。** 字典序会，自然序不会 ——
    /// 章节顺序错了，方案里的「哪几章并成哪一集」整个是乱的。
    #[test]
    fn 章节按数值排不按字典序() {
        use std::cmp::Ordering;
        assert_eq!(natural_cmp("第2章", "第10章"), Ordering::Less);
        assert_eq!(natural_cmp("第01章", "第1章"), Ordering::Equal);
        assert_eq!(natural_cmp("第03章-录音笔", "第10章-收网"), Ordering::Less);
    }

    /// **目录要标出「现在规划的是哪一章」**，否则模型不知道自己站在哪儿。
    #[test]
    fn 目录标出当前这一章() {
        let d = |t: &str| ScriptDocument {
            id: t.into(), title: t.into(), text: "t".into(), doc: None,
            revision: 0, updated_at: 0, project_id: None,
            kind: ScriptDocKind::Source, derived_from: None,
        };
        let ch: Vec<_> = (1..=3).map(|n| d(&format!("第0{n}章"))).collect();
        let toc = chapter_toc(&ch, Some(1));
        assert!(toc.contains("第02章  ← 现在要规划的就是这一章"), "{toc}");
        assert!(toc.contains("正文没给你"), "要说清目录里只有标题：{toc}");
    }

    /// **五百万字的连载有几千章，目录本身也会撑爆。**
    ///
    /// 只取前后各 `TOC_WINDOW` 章，截掉的部分明写「前面还有 N 章」——
    /// 不写的话模型会以为这就是全部，转头给这一章凑个大结局。
    #[test]
    fn 长连载的目录要截断而且说清截了多少() {
        let d = |n: usize| ScriptDocument {
            id: n.to_string(), title: format!("第{n:04}章"), text: "t".into(),
            doc: None, revision: 0, updated_at: 0, project_id: None,
            kind: ScriptDocKind::Source, derived_from: None,
        };
        let ch: Vec<_> = (1..=3000).map(d).collect();
        let toc = chapter_toc(&ch, Some(1500));
        assert!(toc.lines().count() < TOC_WINDOW * 2 + 6, "目录没截断：{} 行", toc.lines().count());
        assert!(toc.contains("前面还有 1495 章"), "{toc}");
        assert!(toc.contains("后面还有"), "{toc}");
    }

    #[test]
    fn 只有一章时目录说清楚() {
        let d = ScriptDocument {
            id: "a".into(), title: "唯一一章".into(), text: "t".into(), doc: None,
            revision: 0, updated_at: 0, project_id: None,
            kind: ScriptDocKind::Source, derived_from: None,
        };
        assert!(chapter_toc(&[d], Some(0)).contains("只有这一章"));
    }

    /// 散落文档（没归项目）也要能规划 —— 它们的 `project_id` 是 `None`。
    #[test]
    fn 未归档的原文也算一组() {
        let d = ScriptDocument {
            id: "a".into(), title: "散的".into(), text: "t".into(),
            doc: None, revision: 0, updated_at: 0,
            project_id: None, kind: ScriptDocKind::Source, derived_from: None,
        };
        assert_eq!(all_sources_of(None, &[d]).len(), 1);
    }

    /// **三种文档，下游只认剧本。**
    ///
    /// 改编方案是原文和剧本之间缺的那一层 —— 没有它的话模型每次自己
    /// 现分集，同一本小说跑两次得到两套完全不同的分法，而且都不报错。
    #[test]
    fn 三种文档各有名字而且默认是剧本() {
        assert_eq!(ScriptDocKind::Source.label(), "原文");
        assert_eq!(ScriptDocKind::Plan.label(), "改编方案");
        assert_eq!(ScriptDocKind::Screenplay.label(), "剧本");
        // **默认必须是剧本** —— 所有存量文档都没有这个字段
        assert_eq!(ScriptDocKind::default(), ScriptDocKind::Screenplay);
        let old: ScriptDocument = serde_json::from_str(
            r#"{"id":"x","title":"旧","text":"","doc":null,"revision":0}"#,
        ).unwrap();
        assert_eq!(old.kind, ScriptDocKind::Screenplay);
    }

    /// 新加的那一档要能存能读，而且**不能让老存档读不出来**。
    #[test]
    fn 改编方案能存能读() {
        let d: ScriptDocument = serde_json::from_str(
            // **`r##` 不是手滑。** 正文里那个 `"# ` 会把 `r#"…"#` 提前结束掉
            r##"{"id":"p","title":"方案","text":"# 改编方案","doc":null,"revision":0,"kind":"plan"}"##,
        ).unwrap();
        assert_eq!(d.kind, ScriptDocKind::Plan);
        let back: ScriptDocument =
            serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
        assert_eq!(back.kind, ScriptDocKind::Plan);
    }

    /// **保存时间由这一层盖，不信前端。**
    ///
    /// 排「最近改动」要的是「真的什么时候写进磁盘」，而前端传什么都可能。
    /// `revision` 顶不了这个用 —— 它是改过几次，不是什么时候改的。
    #[test]
    fn 存盘时盖上时间戳() {
        let dir = tempfile::tempdir().unwrap();
        let doc = ScriptDocument {
            id: "t1".into(), title: "稿".into(), text: "正文".into(),
            doc: None, revision: 0, updated_at: 0,
            project_id: None, kind: ScriptDocKind::Screenplay, derived_from: None,
        };
        let saved = save(dir.path(), doc).unwrap();
        assert!(saved.updated_at > 1_700_000_000_000, "没盖上时间：{}", saved.updated_at);
        // 老存档没有这个字段，读出来是 0 —— 不能因此读不出来
        let old: ScriptDocument = serde_json::from_str(
            r#"{"id":"x","title":"旧","text":"","doc":null,"revision":3}"#,
        ).unwrap();
        assert_eq!(old.updated_at, 0);
    }

    #[test]
    fn independent_storage_roundtrip_and_conflict() {
        let root = std::env::temp_dir().join(format!("anima-scripts-{}", uuid::Uuid::new_v4()));
        let draft = ScriptDocument { id: "sample".into(), title: "独立剧本".into(), text: "台词".into(), doc: None, revision: 0, updated_at: 0,
            project_id: None, kind: ScriptDocKind::Screenplay, derived_from: None };
        assert!(list(&root).unwrap().is_empty());
        let saved = save(&root, draft.clone()).unwrap();
        assert_eq!(list(&root).unwrap()[0].text, "台词");
        assert_eq!(saved.revision, 1);
        assert!(save(&root, draft).is_err());
        assert_eq!(save(&root, saved).unwrap().revision, 2);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn rejects_path_traversal() { assert!(validate("../escape").is_err()); assert!(validate("").is_err()); }

    /// **存量文档不需要迁移。** 老的 JSON 里没有 `project_id` / `kind` /
    /// `derived_from`，读出来必须是「散落的剧本」——
    /// 那正是它们本来的语义。
    #[test]
    fn 老文档没有新字段也能读出来() {
        let old = r#"{"id":"a1","title":"旧稿","text":"正文","doc":null,"revision":3}"#;
        let d: ScriptDocument = serde_json::from_str(old).unwrap();
        assert_eq!(d.project_id, None, "老文档该是散落的");
        assert_eq!(d.kind, ScriptDocKind::Screenplay, "老文档默认当剧本");
        assert!(d.derived_from.is_none());
        assert_eq!(d.revision, 3);
    }

    /// **删项目不能把里面的原稿一起删掉。**
    ///
    /// 用户点「删除项目」想的是「这个分类我不要了」，
    /// 不是「把我写的东西烧了」。
    #[test]
    fn 删项目只解除归属不删文档() {
        let root = std::env::temp_dir().join(format!("anima-scripts-{}", uuid::Uuid::new_v4()));
        let mk = |id: &str, pid: Option<&str>| ScriptDocument {
            id: id.into(), title: id.into(), text: "正文".into(), doc: None, revision: 0, updated_at: 0,
            project_id: pid.map(String::from), kind: ScriptDocKind::Source, derived_from: None,
        };
        save(&root, mk("d1", Some("p1"))).unwrap();
        save(&root, mk("d2", Some("p1"))).unwrap();
        save(&root, mk("d3", None)).unwrap();
        save_projects(&root, &[ScriptProject {
            id: "p1".into(), title: "山村来客".into(), note: String::new(),
            created_at: 1, updated_at: 1,
        }]).unwrap();

        // 手动走一遍 delete 的逻辑（命令层要 State，这里测的是数据部分）
        let mut all = list_projects(&root).unwrap();
        all.retain(|p| p.id != "p1");
        save_projects(&root, &all).unwrap();
        for mut d in list(&root).unwrap() {
            if d.project_id.as_deref() == Some("p1") {
                d.project_id = None;
                save(&root, d).unwrap();
            }
        }

        let docs = list(&root).unwrap();
        assert_eq!(docs.len(), 3, "文档一份都不能少");
        assert!(docs.iter().all(|d| d.project_id.is_none()), "归属该解开了");
        assert!(list_projects(&root).unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    /// **项目清单和文档放在同一个目录，扩展名也一样。**
    /// 列文档时不跳过它，建了第一个项目之后整个剧本库就读不出来了。
    #[test]
    fn 剥掉模型包的代码围栏() {
        assert_eq!(strip_code_fence("【场景：茶馆】\n正文"), "【场景：茶馆】\n正文");
        assert_eq!(strip_code_fence("```\n【场景：茶馆】\n正文\n```"), "【场景：茶馆】\n正文");
        assert_eq!(strip_code_fence("```text\n【场景：茶馆】\n正文\n```"), "【场景：茶馆】\n正文");
        // 只有开头有围栏、没结尾的，别把内容吃掉
        assert!(strip_code_fence("```\n正文").contains("正文"));
        // 正文里本来就有反引号的不要动
        assert!(strip_code_fence("他说：`别动`").contains("`别动`"));
    }

    #[test]
    fn 列文档时跳过项目清单() {
        let root = std::env::temp_dir().join(format!("anima-scripts-{}", uuid::Uuid::new_v4()));
        save(&root, ScriptDocument {
            id: "d1".into(), title: "稿".into(), text: "正文".into(), doc: None, revision: 0, updated_at: 0,
            project_id: None, kind: ScriptDocKind::Screenplay, derived_from: None,
        }).unwrap();
        save_projects(&root, &[ScriptProject {
            id: "p1".into(), title: "项目".into(), note: String::new(), created_at: 1, updated_at: 1,
        }]).unwrap();
        let docs = list(&root).expect("有项目清单时也要能列出文档");
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].id, "d1");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn 项目保存保留创建时间() {
        let root = std::env::temp_dir().join(format!("anima-scripts-{}", uuid::Uuid::new_v4()));
        let p = ScriptProject { id: "p1".into(), title: "初版".into(), note: String::new(),
            created_at: 111, updated_at: 111 };
        save_projects(&root, std::slice::from_ref(&p)).unwrap();
        let mut all = list_projects(&root).unwrap();
        assert_eq!(all[0].created_at, 111);
        all[0].title = "改名".into();
        save_projects(&root, &all).unwrap();
        assert_eq!(list_projects(&root).unwrap()[0].created_at, 111, "创建时间不该被覆盖");
        fs::remove_dir_all(root).unwrap();
    }
}
