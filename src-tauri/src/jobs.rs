//! 出图 / 出片 / 生成镜头表的**任务登记**。
//!
//! 所有后端长任务都登记在这里：落盘、在 QUEUE 里可见、状态变化时发
//! [`EVENT`]。右下角的任务提示框监听这个事件 —— 用户切到别的页面、切回来，
//! 进度都还在；应用重启后没跑完的会被判为失败（[`reap_stale`]），不会悄悄消失。
//!
//! # 为什么非有不可
//!
//! 在这之前，一次渲染就是一个阻塞的 Tauri 命令：前端 await 着，
//! 界面上只有一个转圈。跑完之前**没有任何地方**能回答这几个问题：
//! 现在有几个在跑、这个跑了多久、刚才那个为什么失败、失败的原文是什么。
//! 失败信息只在那一次 `Err` 的字符串里，用户点掉提示就永远没了。
//!
//! # 落盘，不是只在内存里
//!
//! 「重启后生成结果全没了」这件事已经被指出来过一次。任务日志同理 ——
//! 昨天那次为什么失败，是明天还会想查的东西。
//!
//! # 上限是必须的
//!
//! 不封顶的话这个文件会一直长，而它每次状态变化都要整份读写。
//! 超出上限时**先扔已完成的、再扔失败的** —— 失败的诊断价值更高。
//!
//! # 这里**没有**取消
//!
//! 真取消要拿到 ComfyUI 的 `prompt_id` 再去 `/queue` 删或 `/interrupt`，
//! 而那个 id 现在没往外暴露。做一个「点了把状态改成已取消、
//! 但 ComfyUI 照跑」的按钮，就是这个项目最典型的病 ——
//! 所以在能真取消之前，这里一个取消入口都不给。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// 日志里最多留多少条。
const MAX_JOBS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    /// 出首帧（文生图 / 图生图）
    Frame,
    /// 出片（图生视频 / 参考生视频）
    Video,
    /// 由剧本生成整集镜头表。按集登记，`shot_id` 为空。
    ShotTable,
    /// 合成提示词（按当前线路，一次一批镜头）。按集登记，`shot_id` 为空。
    Prompts,
    /// 批量出片：按顺序一镜一镜出，某一镜失败记下接着出。按集登记，`shot_id` 为空；
    /// 每一镜自己的出片仍然各登记一条 `Video`，QUEUE 里看得到每一镜。
    VideoBatch,
    /// 画布上一个节点的生成（出图、出片、动作迁移、深度捕捉、抠图、补帧）。
    /// `project_id` 是画布项目，`shot_id` 放节点 id，`prompt` 放给人看的名字。
    /// 结果整条存在 `result` 里 —— 页面重新加载后节点按任务 id 取回。
    Canvas,
    /// 合成一集（可带逐镜超分）。按集登记，`shot_id` 为空。
    Stitch,
    /// 资产批量出图（`asset_batch`）。按集登记，`targets` 是资产 id。
    AssetImages,
    /// 批量出首帧：按镜头顺序一镜一镜出，某一镜失败记下接着出。按集登记，`shot_id` 为空，
    /// `targets` 是镜头 id；每一镜自己仍登记一条 `Frame`。
    FrameBatch,
    /// 风格广场：同一个镜头铺开多个风格（`style_compare`）。
    /// 按**项目**登记（风格是项目级的），`shot_id` 为空，`targets` 是风格 id。
    StyleCompare,
    /// 细化时间线（`timeline_detail`）：逐镜把时间段写细。按集登记，`shot_id` 为空，`targets` 是镜头 id。
    Timeline,
    /// 导出到 Final Cut Pro（`cinema_export_fcp`，可带逐镜超分）。按集登记，`shot_id` 为空，`output` 是 `.fcpxml` 路径。
    FcpExport,
}

/// 任务状态变化时发给前端的事件名。载荷是整条 [`Job`]。
pub const EVENT: &str = "cinema://job-updated";

/// 跑到哪了。`label` 是给人看的，比如当前这一场的场名。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct JobProgress {
    pub done: usize,
    pub total: usize,
    #[serde(default)]
    pub label: String,
    /// 预计什么时候跑完（unix 秒）。只有出片任务、拿得到步数时有 —— 界面按本地时间显示「预计 21:45 完成」
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eta_at: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub kind: JobKind,
    pub status: JobStatus,
    pub project_id: String,
    pub episode_id: String,
    pub shot_id: String,
    /// 卡片上那个编号。**存下来而不是现查** —— 任务列表要能显示
    /// 已经被删掉的那一镜，现查的话它会变成空白。
    pub shot_num: u32,
    /// 提示词摘要，列表里一行显示。
    pub prompt: String,
    pub created_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<u64>,
    /// 成功时的产物路径（显示缩略图用）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// **失败原文照抄。** 概括过的错误没法用来排查。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// 跑到哪了。只有能分步报进度的任务有（生成镜头表按场）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<JobProgress>,
    /// **这一镜实际走的链路**（「FL2VA 首帧 · LightX2V 4 次」）。提交给 ComfyUI 之前从那张图里读出来（`weetodd::graph_pipeline`），
    /// 不是按设置推的 —— 右下角进度前面显示它（用户 2026-10-07）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pipeline: Option<String>,
    /// 成功时一句话结果，比如「4 场 10 镜，3 个问题」。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// 用户需要知道的备注（资产库里没有的名字、没图的参考图）。
    /// **存在任务上，不是只返回给点按钮的那个页面** —— 页面切走了备注就没人接收。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    /// **成了，但有话要说。** 和 `notes` 严格分开。
    ///
    /// 2026-09-20 出过事：合成提示词把「这一镜正文偏薄」这类**成功时的备注**
    /// 塞进了 `notes`，而界面上 `notes` 那一段的标题写死是
    /// 「上一次合成有 N 镜**没通过检查（没有保存）**」——
    /// 于是四镜明明存好了，用户看到的是「四镜失败」，还会去一镜镜重合成。
    ///
    /// **一个列表两种含义，迟早会被读错。** 所以宁可多一个字段。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remarks: Vec<String>,
    /// **这条的提示已经过时了** —— 它跑完之后，上游重新跑了一轮。
    ///
    /// 用户 2026-09-20：「每次重新生成都会提示之前的错误提示……
    /// 在点击重新生成、重新合成的时候就先清空错误提示状态」。
    ///
    /// 重新生成镜头表之后，上一轮「合成有 4 镜没过」「批量出片有 1 镜失败」
    /// 说的还是**旧镜号**，甚至旧镜头已经不存在了。
    ///
    /// **不是删记录。** 队列里还要看得到「那次为什么失败」——
    /// 删掉的话，用户回头想查就什么都没有了。所以只打标记，
    /// 由界面决定哪些横幅不再显示（见 `jobs::supersede_downstream`）。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub superseded: bool,
    /// 批量任务要处理的镜头 id（按顺序）。界面据此标「排队中」—— 刷新之后也知道还有哪几镜没轮到。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<String>,
    /// 用户点了「停止」。批量任务在开始下一镜之前看这个标记（正在出的那一镜 H3 不响应中断，只能跑完）。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cancel_requested: bool,
    /// 画布任务的完整返回（路径、备注、首帧……）。别的任务不用它
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// **出片续接单**：这一镜交给 ComfyUI 之后，结果该落到哪、这一次 take 怎么记、ComfyUI 的 prompt_id。
    ///
    /// 应用中途关掉，ComfyUI 那边照样在跑；重开时按它去 ComfyUI 收结果（见 `crate::render_resume`）。
    /// 没有它的出片任务只能判失败（以前全部如此，2026-10-05 用户：「不然那么长时间白生成」）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobLog {
    #[serde(default)]
    pub jobs: Vec<Job>,
}

pub(crate) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn jobs_path(config_dir: &Path) -> PathBuf {
    crate::cinema::cinema_projects_dir(config_dir).join("jobs.json")
}

/// 整份读写要串行。**同一进程里可能有多个渲染并发**，
/// 各读各写的话后写的会把先写的那条整个盖掉。
fn lock() -> std::sync::MutexGuard<'static, ()> {
    static L: Mutex<()> = Mutex::new(());
    L.lock().unwrap_or_else(|e| e.into_inner())
}

/// 跨进程的文件锁。**进程内 Mutex 不够。**
///
/// 应用本体和 `anima-mcp` 是两个进程，都会往同一份 `jobs.json` 里写。
/// 上面那把 `Mutex` 只管得住自己进程里的并发，两个进程一起写就会互相盖掉 ——
/// 最坏的后果不是「少一条记录」，是 [`start_exclusive`] 看不见对方正在跑的任务，
/// **于是重复发起一轮渲染**。2026-09-16 那次内核 panic 就是 GPU 叠满。
///
/// 用 `flock` 而不是自己写 pidfile：进程被 kill 掉时内核会自动释放，
/// 自己实现的话必然要处理「上一个进程死了但锁还在」那一堆情况。
#[cfg(unix)]
struct FileLock(std::fs::File);

#[cfg(unix)]
impl Drop for FileLock {
    fn drop(&mut self) {
        use std::os::unix::io::AsRawFd;
        // SAFETY: fd 来自还活着的 File，解锁失败也没有补救办法
        unsafe { libc::flock(self.0.as_raw_fd(), libc::LOCK_UN) };
    }
}

/// 拿到跨进程锁。**拿不到就直接往下走** —— 锁是为了减少互相覆盖，
/// 不是为了把功能卡死；拿不到锁时退化成原来的行为，不比以前差。
#[cfg(unix)]
fn file_lock(config_dir: &Path) -> Option<FileLock> {
    use std::os::unix::io::AsRawFd;
    let p = jobs_path(config_dir);
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(p.with_extension("lock"))
        .ok()?;
    // SAFETY: fd 来自刚打开的 File
    let rc = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) };
    (rc == 0).then_some(FileLock(f))
}

#[cfg(not(unix))]
fn file_lock(_config_dir: &Path) -> Option<()> {
    None
}

pub fn load(config_dir: &Path) -> JobLog {
    let p = jobs_path(config_dir);
    std::fs::read_to_string(p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// **先写临时文件再 rename。**
///
/// 直接 `fs::write` 是「先截断再写」——写到一半被打断（崩溃、被 kill、
/// 另一个进程同时读）就留下一份半截的 JSON，而读取那边 `unwrap_or_default()`
/// 会把它当成「一条任务都没有」。rename 在同一个文件系统上是原子的，
/// 读到的要么是旧的完整内容、要么是新的完整内容。
fn save(config_dir: &Path, log: &JobLog) -> Result<()> {
    let p = jobs_path(config_dir);
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(log).unwrap_or_default())
        .map_err(|e| Error::runtime(format!("写任务日志失败：{e}")))?;
    std::fs::rename(&tmp, &p).map_err(|e| Error::runtime(format!("换上任务日志失败：{e}")))?;
    // **有任务在跑就别让机器睡过去。**
    //
    // 放在这里是因为这是任务列表唯一的写入口 —— 起任务、跑完、报进度
    // 全经过它。挂在各个调用点上的话，漏掉任何一处的表现都是
    // 「这一批出片半夜睡过去了」，而且看不出来。见 [`crate::keep_awake`]。
    crate::keep_awake::sync(log.jobs.iter().any(|j| j.status == JobStatus::Running));
    Ok(())
}

/// 超出上限时扔哪些。
///
/// **先扔已完成的，再扔失败的，正在跑的一条都不扔。**
/// 按时间一刀切的话，一条排在前面的失败记录会被一串成功记录挤掉 ——
/// 而那条恰恰是唯一有诊断价值的。
pub fn prune(mut jobs: Vec<Job>, max: usize) -> Vec<Job> {
    if jobs.len() <= max {
        return jobs;
    }
    // 旧的排前面，这样从头扔就是先扔最旧的
    jobs.sort_by_key(|j| j.created_at);
    for want in [JobStatus::Done, JobStatus::Failed] {
        while jobs.len() > max {
            let Some(i) = jobs.iter().position(|j| j.status == want) else {
                break;
            };
            jobs.remove(i);
        }
    }
    // 全是 Running 还超了（不该发生）—— 扔最旧的，保住上限
    while jobs.len() > max {
        jobs.remove(0);
    }
    jobs
}

/// 同一镜同一类还有在跑的任务时拒绝登记，回那个在跑的任务。
///
/// **查和登记在同一把锁里做**，否则两次几乎同时的点击都会查到「没在跑」。
///
/// 2026-09-13 实拍：同一镜 32 秒内被提交了两次出片。ComfyUI 串行执行，
/// 第二个只会排在后面白等（那次它排队时被取消了，队列上空计时 34 分钟）。
/// 一台机器一镜跑一次要二十分钟，重复提交没有任何好处。
// Err 带回在跑的那条任务（调用方要它的镜号和开始时间）；`Job` 多了续接单之后过了 clippy 的大小线，不值得为此装箱
#[allow(clippy::result_large_err)]
pub fn start_exclusive(
    config_dir: &Path,
    kind: JobKind,
    project_id: &str,
    episode_id: &str,
    shot_id: &str,
    shot_num: u32,
    prompt: &str,
) -> std::result::Result<String, Job> {
    // **查和登记在同一把锁里**（以前查完放锁再调 `start`，两次几乎同时的请求都能查到「没在跑」、各登记一条 ——
    // 2026-09-28 审查指出，注释写的和代码做的相反）
    let _g = lock();
    let _fl = file_lock(config_dir);
    let mut log = load(config_dir);
    if let Some(j) = log.jobs.iter().find(|j| {
        j.status == JobStatus::Running && j.kind == kind && j.project_id == project_id && j.episode_id == episode_id && j.shot_id == shot_id
    }) {
        return Err(j.clone());
    }
    let id = register(&mut log, kind, project_id, episode_id, shot_id, shot_num, prompt);
    let _ = save(config_dir, &log);
    Ok(id)
}

/// 这一类任务一开跑，**哪些已完成任务的提示就作废了**。
///
/// 顺序是流水线本身：镜头表 → 提示词 → 出片 → 合成一集。
/// 重新生成镜头表之后，上一轮的合成报告和出片报告说的都是旧镜号。
///
/// # 为什么单镜出片（`Video`）不在这张表里
///
/// 重出一镜不该把「上一次批量出片有 3 镜失败」这条抹掉 —— 那 3 镜多半
/// 还是失败的。只有**整批重来**才说明上一批的账已经作废。
pub fn supersedes(kind: JobKind) -> &'static [JobKind] {
    use JobKind::*;
    match kind {
        ShotTable => &[ShotTable, Prompts, VideoBatch, Video, Frame, Stitch],
        Prompts => &[Prompts, VideoBatch, Video, Stitch],
        VideoBatch => &[VideoBatch, Video, Stitch],
        Stitch => &[Stitch],
        // 资产出图、风格广场、画布、单镜出片/首帧：各管各的，不作废别人
        _ => &[],
    }
}

/// 给这一集里被 `kind` 作废的那些**已完成**任务打标记。
///
/// **只打标记，不删记录** —— 队列里还要能看到「上次为什么失败」。
/// 见 [`Job::superseded`]。
fn supersede_downstream(log: &mut JobLog, kind: JobKind, project_id: &str, episode_id: &str) {
    let kinds = supersedes(kind);
    if kinds.is_empty() {
        return;
    }
    for j in log.jobs.iter_mut() {
        if j.status != JobStatus::Running
            && j.project_id == project_id
            && j.episode_id == episode_id
            && kinds.contains(&j.kind)
        {
            j.superseded = true;
        }
    }
}

/// 登记一个刚开始的任务，回它的 id。
pub fn start(
    config_dir: &Path,
    kind: JobKind,
    project_id: &str,
    episode_id: &str,
    shot_id: &str,
    shot_num: u32,
    prompt: &str,
) -> String {
    let _g = lock();
    let _fl = file_lock(config_dir);
    let mut log = load(config_dir);
    let id = register(&mut log, kind, project_id, episode_id, shot_id, shot_num, prompt);
    let _ = save(config_dir, &log);
    id
}

/// 往登记表里加一条在跑的任务（调用方已经拿着锁、负责存盘）。
fn register(log: &mut JobLog, kind: JobKind, project_id: &str, episode_id: &str, shot_id: &str, shot_num: u32, prompt: &str) -> String {
    let id = format!("job_{}", uuid::Uuid::new_v4().simple());
    // **重新跑一轮，上一轮的提示就作废。** 用户不用再去点「清空」——
    // 那些提示说的是旧镜号，留着只会把人支到不存在的镜头上。
    supersede_downstream(log, kind, project_id, episode_id);
    log.jobs.push(Job {
        id: id.clone(),
        kind,
        status: JobStatus::Running,
        project_id: project_id.into(),
        episode_id: episode_id.into(),
        shot_id: shot_id.into(),
        shot_num,
        prompt: prompt.chars().take(200).collect(),
        created_at: now(),
        finished_at: None,
        output: None,
        error: None,
        progress: None,
        summary: None,
        notes: Vec::new(),
        remarks: Vec::new(),
        pipeline: None,
        superseded: false,
        targets: Vec::new(),
        cancel_requested: false,
        result: None,
        resume: None,
    });
    log.jobs = prune(std::mem::take(&mut log.jobs), MAX_JOBS);
    id
}

/// 收尾。**成功和失败都要走这里** —— 只在成功路径上收尾的话，
/// 失败的任务会永远停在「正在跑」，队列上的数字再也不归零。
pub fn finish(config_dir: &Path, id: &str, output: Option<&str>, error: Option<&str>) -> Option<Job> {
    finish_with(config_dir, id, output, None, &[], error)
}

/// 同 [`finish_with`]，另外记下「成了但有话要说」的那些。见 [`Job::remarks`]。
pub fn finish_with_remarks(
    config_dir: &Path,
    id: &str,
    output: Option<&str>,
    summary: Option<&str>,
    notes: &[String],
    remarks: &[String],
    error: Option<&str>,
) -> Option<Job> {
    let out = finish_with(config_dir, id, output, summary, notes, error);
    if remarks.is_empty() {
        return out;
    }
    let _g = lock();
    let _fl = file_lock(config_dir);
    let mut log = load(config_dir);
    let out = log.jobs.iter_mut().find(|j| j.id == id).map(|j| {
        j.remarks = remarks.to_vec();
        j.clone()
    });
    let _ = save(config_dir, &log);
    out
}

/// 同 [`finish`]，另外记下一句话结果和备注。回收尾后的那条，给调用方发事件用。
pub fn finish_with(
    config_dir: &Path,
    id: &str,
    output: Option<&str>,
    summary: Option<&str>,
    notes: &[String],
    error: Option<&str>,
) -> Option<Job> {
    let _g = lock();
    let _fl = file_lock(config_dir);
    let mut log = load(config_dir);
    let out = log.jobs.iter_mut().find(|j| j.id == id).map(|j| {
        j.status = if error.is_some() { JobStatus::Failed } else { JobStatus::Done };
        j.finished_at = Some(now());
        j.output = output.filter(|s| !s.is_empty()).map(|s| s.to_string());
        j.error = error.map(|s| s.to_string());
        j.summary = summary.map(|s| s.to_string());
        j.notes = notes.to_vec();
        j.clone()
    });
    let _ = save(config_dir, &log);
    out
}

/// 画布任务收尾：成功时存完整结果。
pub fn finish_canvas(config_dir: &Path, id: &str, result: Option<serde_json::Value>, error: Option<&str>) -> Option<Job> {
    let _g = lock();
    let _fl = file_lock(config_dir);
    let mut log = load(config_dir);
    let out = log.jobs.iter_mut().find(|j| j.id == id).map(|j| {
        j.status = if error.is_some() { JobStatus::Failed } else { JobStatus::Done };
        j.finished_at = Some(now());
        j.output = result.as_ref().and_then(|r| r.get("path")).and_then(|p| p.as_str()).map(String::from);
        j.error = error.map(String::from);
        j.result = result;
        j.clone()
    });
    let _ = save(config_dir, &log);
    out
}

/// 记下进度。回更新后的那条，给调用方发事件用。
pub fn set_progress(config_dir: &Path, id: &str, done: usize, total: usize, label: &str) -> Option<Job> {
    let _g = lock();
    let _fl = file_lock(config_dir);
    let mut log = load(config_dir);
    let out = log.jobs.iter_mut().find(|j| j.id == id).map(|j| {
        j.progress = Some(JobProgress { done, total, label: label.to_string(), eta_at: None });
        j.clone()
    });
    let _ = save(config_dir, &log);
    out
}

/// 记下批量任务要处理的镜头。
pub fn set_targets(config_dir: &Path, id: &str, targets: &[String]) -> Option<Job> {
    let _g = lock();
    let _fl = file_lock(config_dir);
    let mut log = load(config_dir);
    let out = log.jobs.iter_mut().find(|j| j.id == id).map(|j| {
        j.targets = targets.to_vec();
        j.clone()
    });
    let _ = save(config_dir, &log);
    out
}

/// **换掉没轮到的那段名单**（`render_queue::reorder`）：已经出完的和正在出的那一镜（下标 ≤ `progress.done`）原样留着，
/// 后面换成 `pending`。任务不在跑回 `None`。顺序合不合法由调用方查（接上一镜的不能拆开）
pub fn replace_pending(config_dir: &Path, id: &str, pending: &[String]) -> Option<Job> {
    let _g = lock();
    let _fl = file_lock(config_dir);
    let mut log = load(config_dir);
    let j = log.jobs.iter_mut().find(|j| j.id == id && j.status == JobStatus::Running)?;
    let keep = j.progress.as_ref().filter(|p| !p.label.starts_with("排队中")).map_or(0, |p| p.done + 1).min(j.targets.len());
    j.targets.truncate(keep);
    j.targets.extend(pending.iter().cloned());
    let out = j.clone();
    let _ = save(config_dir, &log);
    Some(out)
}

/// 从名单里删掉某一项（撤暂停位用；正在等的那个也能删）。没有这一项回 `None`
pub fn remove_target(config_dir: &Path, id: &str, target: &str) -> Option<Job> {
    let _g = lock();
    let _fl = file_lock(config_dir);
    let mut log = load(config_dir);
    let j = log.jobs.iter_mut().find(|j| j.id == id && j.status == JobStatus::Running)?;
    let at = j.targets.iter().position(|t| t == target)?;
    j.targets.remove(at);
    let out = j.clone();
    let _ = save(config_dir, &log);
    Some(out)
}

/// **加入出片队列**：往一条还在跑的批量任务后面追加镜头。回 `(任务, 实际加进去的)`；任务不在跑回 `None`。
///
/// 还没轮到的（排在正在出的那一镜后面）不重复加；正在出、或已经出过的再加一次，就是出完再重出一次。
/// `targets` 只追加不改动 —— 批量那边按下标往下走（见 `cinema::render_batch` 的 `more`）。
pub fn append_targets(config_dir: &Path, id: &str, ids: &[String]) -> Option<(Job, Vec<String>)> {
    let _g = lock();
    let _fl = file_lock(config_dir);
    let mut log = load(config_dir);
    let j = log.jobs.iter_mut().find(|j| j.id == id && j.status == JobStatus::Running)?;
    // 正在出的那一镜的下标；还在「排队中」（等前一镜出完、一镜都没开始）时一个都没开始
    let current = j.progress.as_ref().filter(|p| !p.label.starts_with("排队中")).map_or(-1, |p| p.done as isize);
    let mut added = Vec::new();
    for x in ids {
        let pending = j.targets.iter().enumerate().any(|(i, t)| t == x && i as isize > current);
        if !pending && !added.contains(x) {
            j.targets.push(x.clone());
            added.push(x.clone());
        }
    }
    // **总数跟着变**（2026-10-06 用户：点了 1 个出片、4 个排队，提示框一直是「第 1/1 个」）。
    // 原来只往 targets 里加，进度里的 total 还是建批量时的数，界面按它显示
    if !added.is_empty() {
        let total = j.targets.len();
        if let Some(p) = j.progress.as_mut() {
            p.total = total;
        } else {
            j.progress = Some(JobProgress { done: 0, total, label: "排队中 · 等前一镜出完".into(), eta_at: None });
        }
    }
    let out = (j.clone(), added);
    let _ = save(config_dir, &log);
    Some(out)
}

/// 按 id 取一条。
pub fn get(config_dir: &Path, id: &str) -> Option<Job> {
    load(config_dir).jobs.into_iter().find(|j| j.id == id)
}

/// 把一条任务的最新状态推给前端（右下角提示框、QUEUE、各步骤页都听它）。
/// 把任务状态推给界面。
///
/// **`app` 是 `Option`**：`anima-mcp` 那条路没有界面，也就没有 `AppHandle`。
/// 事件只是进度显示，没有它任务照样跑完 —— 所以缺了就跳过，不是错误。
pub fn emit(app: Option<&tauri::AppHandle>, job: Option<&Job>) {
    use tauri::Emitter;
    if let (Some(app), Some(job)) = (app, job) {
        let _ = app.emit(EVENT, job);
    }
}

// ---------------------------------------------------------------------------
// 出片续接：把「结果该落到哪」和 ComfyUI 的 prompt_id 记在任务上
// ---------------------------------------------------------------------------

tokio::task_local! {
    /// 当前这条登记任务（配置目录, 任务 id）。出片时 scope 进来，提交给 ComfyUI 的地方顺手把 prompt_id 记上 ——
    /// 和 `comfy_jobs::JOB_ID` 同一个理由：调用链七八层，往下传参漏一处就是「这条路续接不上」且不报错。
    static CURRENT: (PathBuf, String);
}

/// 在「当前任务」里跑 `fut`（见 [`note_resume`] / [`note_prompt_id`]）。
pub async fn scoped<F: std::future::Future>(config_dir: &Path, id: &str, fut: F) -> F::Output {
    CURRENT.scope((config_dir.to_path_buf(), id.to_string()), fut).await
}

fn update_current(f: impl FnOnce(&mut Job)) {
    let _ = CURRENT.try_with(|(dir, id)| {
        let _g = lock();
        let _fl = file_lock(dir);
        let mut log = load(dir);
        if let Some(j) = log.jobs.iter_mut().find(|j| &j.id == id) {
            f(j);
            let _ = save(dir, &log);
        }
    });
}

/// 记下这一次出片的续接单（交给 ComfyUI 之前调）。会清掉上一次（同一任务出第二条 take）留下的 prompt_id。
pub fn note_resume(mut value: serde_json::Value) {
    value["prompt_ids"] = serde_json::json!([]);
    update_current(|j| j.resume = Some(value));
}

fn app_cell() -> &'static std::sync::OnceLock<tauri::AppHandle> {
    static APP: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();
    &APP
}

/// 启动时把界面的句柄放进来：深处的等待循环（[`note_progress`]）也能把进度推给界面。
pub fn set_app(app: tauri::AppHandle) {
    let _ = app_cell().set(app);
}

/// **当前出片任务跑到哪了**（ComfyUI 的步数、百分比、还要多久）：写进任务、推给界面。
/// 不在任务里（对话、画布）什么也不做。见 `comfy_progress::video_line`。
pub fn note_progress(done: usize, total: usize, label: &str, eta_at: Option<u64>) {
    let _ = CURRENT.try_with(|(dir, id)| {
        let job = {
            let _g = lock();
            let _fl = file_lock(dir);
            let mut log = load(dir);
            let out = log.jobs.iter_mut().find(|j| &j.id == id && j.status == JobStatus::Running).map(|j| {
                j.progress = Some(JobProgress { done, total, label: label.to_string(), eta_at });
                j.clone()
            });
            let _ = save(dir, &log);
            out
        };
        emit(app_cell().get(), job.as_ref());
    });
}

/// 记下这一次提交实际走的链路（[`Job::pipeline`]）并推给界面。不在任务里（对话、画布）什么也不做。
pub fn note_pipeline(pipeline: &str) {
    let _ = CURRENT.try_with(|(dir, id)| {
        let job = {
            let _g = lock();
            let _fl = file_lock(dir);
            let mut log = load(dir);
            let out = log.jobs.iter_mut().find(|j| &j.id == id && j.status == JobStatus::Running).map(|j| {
                j.pipeline = Some(pipeline.to_string());
                j.clone()
            });
            let _ = save(dir, &log);
            out
        };
        emit(app_cell().get(), job.as_ref());
    });
}

/// **ComfyUI 开始跑这一镜的时刻**（unix 秒）记进续接单，只记第一次。
///
/// 2026-10-06 用户截图：重启后续接的那一镜显示「已跑 1 分钟」，其实已经跑了二十分钟 ——
/// 「已跑」是从这一次看到它在跑算起的，重启一次就清零。记下来，续接时接着算（[`current_run_started_at`]）。
/// 不在出片任务里（对话、画布）什么也不做。
pub fn note_run_started(at: u64) {
    update_current(|j| {
        if let Some(r) = j.resume.as_mut().filter(|r| r.get("run_started_at").is_none()) {
            r["run_started_at"] = serde_json::json!(at);
        }
    });
}

/// 当前出片任务上次记下的开跑时刻（见 [`note_run_started`]）；没记过 / 不在任务里是 `None`。
pub fn current_run_started_at() -> Option<u64> {
    CURRENT
        .try_with(|(dir, id)| get(dir, id).and_then(|j| j.resume.as_ref().and_then(|r| r["run_started_at"].as_u64())))
        .ok()
        .flatten()
}

/// ComfyUI 收下了这张图：把 prompt_id 记进续接单。不在出片任务里（对话、画布）什么也不做。
pub fn note_prompt_id(prompt_id: &str) {
    update_current(|j| {
        if let Some(ids) = j.resume.as_mut().and_then(|r| r.get_mut("prompt_ids")).and_then(|v| v.as_array_mut()) {
            ids.push(serde_json::Value::String(prompt_id.to_string()));
        }
    });
}

#[cfg(test)]
pub fn set_resume_for_test(config_dir: &Path, id: &str, value: serde_json::Value) {
    let _g = lock();
    let mut log = load(config_dir);
    if let Some(j) = log.jobs.iter_mut().find(|j| j.id == id) {
        j.resume = Some(value);
    }
    let _ = save(config_dir, &log);
}

/// 进程启动时把上一次留下的「正在跑」全部判为失败。
///
/// **不做这一步的话队列永远挂着几个跑不完的任务。** 进程没了，
/// 那些 await 也就没了，没有任何人会再去给它们收尾 ——
/// 而界面上它们看起来和真正在跑的一模一样。
pub fn reap_stale(config_dir: &Path) -> usize {
    reap_stale_except(config_dir, &[])
}

/// 同上，但 `keep` 里的不动（续接得上的出片任务，见 `crate::render_resume`）。
pub fn reap_stale_except(config_dir: &Path, keep: &[String]) -> usize {
    let _g = lock();
    let _fl = file_lock(config_dir);
    let mut log = load(config_dir);
    let mut n = 0;
    for j in log.jobs.iter_mut().filter(|j| j.status == JobStatus::Running && !keep.contains(&j.id)) {
        j.status = JobStatus::Failed;
        j.finished_at = Some(now());
        j.error = Some("应用退出时这个任务还没跑完，结果丢了。重新生成一次。".into());
        n += 1;
    }
    if n > 0 {
        let _ = save(config_dir, &log);
    }
    n
}

// ---------------------------------------------------------------------------
// Tauri command
// ---------------------------------------------------------------------------

/// 请求停止一条在跑的任务。返回更新后的任务；任务不在跑返回 `None`。
pub fn request_cancel(config_dir: &Path, id: &str) -> Option<Job> {
    let _g = lock();
    let _fl = file_lock(config_dir);
    let mut log = load(config_dir);
    let out = log.jobs.iter_mut().find(|j| j.id == id && j.status == JobStatus::Running).map(|j| {
        j.cancel_requested = true;
        j.clone()
    });
    let _ = save(config_dir, &log);
    out
}

/// 这条任务有没有被请求停止。
pub fn cancel_requested(config_dir: &Path, id: &str) -> bool {
    load(config_dir).jobs.iter().any(|j| j.id == id && j.cancel_requested)
}

/// 停止一条在跑的批量任务：当前这一镜跑完就不再继续。
#[tauri::command]
pub fn cinema_cancel_job(app: tauri::AppHandle, state: tauri::State<'_, crate::AppState>, job_id: String) -> std::result::Result<Job, String> {
    let job = request_cancel(&state.config_dir, &job_id).ok_or("这个任务已经结束了")?;
    emit(Some(&app), Some(&job));
    Ok(job)
}

/// 所有项目里正在跑的任务。
pub fn all_running(config_dir: &Path) -> Vec<Job> {
    load(config_dir).jobs.into_iter().filter(|j| j.status == JobStatus::Running).collect()
}

/// 这一集正在跑的任务。
pub fn running(config_dir: &Path, project_id: &str, episode_id: &str) -> Vec<Job> {
    load(config_dir).jobs.into_iter().filter(|j| j.status == JobStatus::Running && j.project_id == project_id && j.episode_id == episode_id).collect()
}

/// 所有项目里正在跑的任务。右下角提示框在界面刷新后靠它恢复。
#[tauri::command]
pub fn cinema_jobs_running(state: tauri::State<'_, crate::AppState>) -> Vec<Job> {
    load(&state.config_dir).jobs.into_iter().filter(|j| j.status == JobStatus::Running).collect()
}

/// 这一集的任务，**新的在前**。
#[tauri::command]
pub fn cinema_jobs(
    state: tauri::State<'_, crate::AppState>,
    project_id: String,
    episode_id: String,
) -> Vec<Job> {
    let mut v: Vec<Job> = load(&state.config_dir)
        .jobs
        .into_iter()
        .filter(|j| j.project_id == project_id && j.episode_id == episode_id)
        .collect();
    v.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    v
}

/// 清掉这一集已结束的任务。**正在跑的不动** ——
/// 从列表里抹掉一个还在跑的任务，只会让人以为它停了。
#[tauri::command]
pub fn cinema_jobs_clear(
    state: tauri::State<'_, crate::AppState>,
    project_id: String,
    episode_id: String,
) -> Result<Vec<Job>> {
    {
        let _g = lock();
        let _fl = file_lock(&state.config_dir);
        let mut log = load(&state.config_dir);
        log.jobs.retain(|j| {
            j.status == JobStatus::Running
                || j.project_id != project_id
                || j.episode_id != episode_id
        });
        save(&state.config_dir, &log)?;
    }
    Ok(cinema_jobs(state, project_id, episode_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- 重新跑一轮，上一轮的提示作废（2026-09-20 用户要求）----

    /// 用户：「每次重新生成都会提示之前的错误提示……在点击重新生成、
    /// 重新合成的时候就先清空错误提示状态」。
    #[test]
    fn 重新生成镜头表_把下游那几轮的提示都作废() {
        let d = tempfile::tempdir().unwrap();
        let old_prompts = start(d.path(), JobKind::Prompts, "p", "e", "", 0, "");
        finish_with(d.path(), &old_prompts, None, None, &["第 3 镜没过".into()], None);
        let old_batch = start(d.path(), JobKind::VideoBatch, "p", "e", "", 0, "");
        finish_with(d.path(), &old_batch, None, None, &["第 1 镜失败".into()], None);
        // 别的分集不受影响
        let other = start(d.path(), JobKind::Prompts, "p", "e2", "", 0, "");
        finish_with(d.path(), &other, None, None, &["别动我".into()], None);

        start(d.path(), JobKind::ShotTable, "p", "e", "", 0, "");

        let jobs = load(d.path()).jobs;
        let by = |id: &str| jobs.iter().find(|j| j.id == id).unwrap().clone();
        assert!(by(&old_prompts).superseded, "重新生成镜头表之后，上一轮合成的提示还在");
        assert!(by(&old_batch).superseded, "上一轮出片的提示还在");
        assert!(!by(&other).superseded, "串到别的分集了");
        // **记录本身要留着** —— 队列里还要能看到上次为什么失败
        assert_eq!(by(&old_prompts).notes, ["第 3 镜没过"], "把记录删了，回头查不到了");
    }

    /// 加入出片队列：追加到在跑的批量后面；还没轮到的不重复加，正在出 / 出过的可以再加（出完再重出）；不在跑的加不进去
    #[test]
    fn 加入队列_追加到在跑的批量后面() {
        let d = tempfile::tempdir().unwrap();
        let batch = start(d.path(), JobKind::VideoBatch, "p", "e", "", 0, "");
        let ids = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        set_targets(d.path(), &batch, &ids(&["s1", "s2", "s3"]));
        set_progress(d.path(), &batch, 1, 3, "第 2 镜");
        let (job, added) = append_targets(d.path(), &batch, &ids(&["s3", "s1", "s2", "s9", "s9"])).unwrap();
        assert_eq!(added, ids(&["s1", "s2", "s9"]), "s3 还没轮到不重复加；s1 出过、s2 正在出，可以再出一次");
        assert_eq!(job.targets, ids(&["s1", "s2", "s3", "s1", "s2", "s9"]));
        // 总数跟着变（2026-10-06：点了 4 个排队，提示框一直是「第 1/1 个」）
        assert_eq!(job.progress.as_ref().map(|p| (p.done, p.total)), Some((1, 6)), "{:?}", job.progress);
        // 还在排队（一镜都没开始）：名单里的全算没轮到
        set_progress(d.path(), &batch, 0, 6, "排队中 · 等前一镜出完");
        assert!(append_targets(d.path(), &batch, &ids(&["s1"])).unwrap().1.is_empty(), "排队中时第一镜也还没开始");
        let (job, _) = append_targets(d.path(), &batch, &ids(&["s7"])).unwrap();
        assert_eq!(job.progress.as_ref().map(|p| (p.total, p.label.as_str())), Some((7, "排队中 · 等前一镜出完")), "排队中再加一个：总数 +1，说法不变");
        finish_with(d.path(), &batch, None, None, &[], None);
        assert!(append_targets(d.path(), &batch, &ids(&["s4"])).is_none(), "跑完的任务加不进去");
    }

    /// 重新合成提示词**不该**把上一轮镜头表的提示抹掉 —— 那一层没重来。
    #[test]
    fn 只作废下游_不碰上游() {
        let d = tempfile::tempdir().unwrap();
        let table = start(d.path(), JobKind::ShotTable, "p", "e", "", 0, "");
        finish_with(d.path(), &table, None, None, &["资产库里没有「皇帝」".into()], None);
        start(d.path(), JobKind::Prompts, "p", "e", "", 0, "");
        assert!(!load(d.path()).jobs.iter().find(|j| j.id == table).unwrap().superseded);
    }

    /// **重出一镜不该把「上一次批量出片有 3 镜失败」抹掉** —— 那 3 镜多半还是失败的。
    #[test]
    fn 单镜出片不作废整批的账() {
        let d = tempfile::tempdir().unwrap();
        let batch = start(d.path(), JobKind::VideoBatch, "p", "e", "", 0, "");
        finish_with(d.path(), &batch, None, None, &["第 1 镜失败".into()], None);
        start(d.path(), JobKind::Video, "p", "e", "shot_9", 9, "");
        assert!(!load(d.path()).jobs.iter().find(|j| j.id == batch).unwrap().superseded);
    }

    /// 正在跑的不打标记 —— 它还没产出提示，打了也没意义，
    /// 而且会让「它自己那一轮的结果」一出生就被当成过时的。
    #[test]
    fn 在跑的不打标记() {
        let d = tempfile::tempdir().unwrap();
        let running = start(d.path(), JobKind::Prompts, "p", "e", "", 0, "");
        start(d.path(), JobKind::ShotTable, "p", "e", "", 0, "");
        assert!(!load(d.path()).jobs.iter().find(|j| j.id == running).unwrap().superseded);
    }

    fn job(id: &str, status: JobStatus, at: u64) -> Job {
        Job {
            id: id.into(),
            kind: JobKind::Frame,
            status,
            project_id: "p1".into(),
            episode_id: "e1".into(),
            shot_id: "s1".into(),
            shot_num: 1,
            prompt: "一个纸箱".into(),
            created_at: at,
            finished_at: None,
            output: None,
            error: None,
            progress: None,
            summary: None,
            notes: Vec::new(),
            remarks: Vec::new(),
            pipeline: None,
            superseded: false,
            targets: Vec::new(),
            cancel_requested: false,
            result: None,
            resume: None,
        }
    }

    #[test]
    fn 没超上限时一条都不扔() {
        let v = vec![job("a", JobStatus::Done, 1), job("b", JobStatus::Done, 2)];
        assert_eq!(prune(v, 5).len(), 2);
    }

    /// **失败记录的诊断价值最高，要留到最后再扔。**
    /// 按时间一刀切的话，一条旧的失败会被一串成功挤掉。
    #[test]
    fn 先扔已完成再扔失败() {
        let v = vec![
            job("fail_old", JobStatus::Failed, 1),
            job("done_a", JobStatus::Done, 2),
            job("done_b", JobStatus::Done, 3),
        ];
        let left = prune(v, 2);
        let ids: Vec<&str> = left.iter().map(|j| j.id.as_str()).collect();
        assert!(ids.contains(&"fail_old"), "失败记录被先扔了：{ids:?}");
        assert_eq!(left.len(), 2);
    }

    /// 正在跑的一条都不能扔 —— 扔掉之后它永远不会被收尾，
    /// 而队列上的数字也再不会归零。
    #[test]
    fn 正在跑的不扔() {
        let v = vec![
            job("run", JobStatus::Running, 1),
            job("d1", JobStatus::Done, 2),
            job("d2", JobStatus::Done, 3),
        ];
        let left = prune(v, 1);
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, "run");
    }

    #[test]
    fn 全是正在跑且超了也不会无限增长() {
        let v: Vec<Job> = (0..5).map(|i| job(&format!("r{i}"), JobStatus::Running, i)).collect();
        assert_eq!(prune(v, 2).len(), 2);
    }

    #[test]
    fn 登记和收尾能落盘再读回() {
        let d = tempfile::tempdir().unwrap();
        let id = start(d.path(), JobKind::Video, "p1", "e1", "s1", 3, "画面描述");
        let all = load(d.path()).jobs;
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].status, JobStatus::Running);
        assert_eq!(all[0].shot_num, 3);

        finish(d.path(), &id, Some("/tmp/a.mp4"), None);
        let j = load(d.path()).jobs.pop().unwrap();
        assert_eq!(j.status, JobStatus::Done);
        assert_eq!(j.output.as_deref(), Some("/tmp/a.mp4"));
        assert!(j.finished_at.is_some());
    }

    /// **查和登记在同一把锁里**：十几个线程同时抢着登记同一集的细化时间线，只能有一个登记上（2026-09-28 审查）。
    #[test]
    fn 同时登记只有一个成功() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().to_path_buf();
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let dir = dir.clone();
                std::thread::spawn(move || start_exclusive(&dir, JobKind::Timeline, "p1", "e1", "", 0, "细化").is_ok())
            })
            .collect();
        let ok = handles.into_iter().map(|h| h.join().unwrap()).filter(|x| *x).count();
        assert_eq!(ok, 1);
        assert_eq!(load(&dir).jobs.iter().filter(|j| j.kind == JobKind::Timeline && j.status == JobStatus::Running).count(), 1);
    }

    #[test]
    fn 同一镜在出片时不能再登记一次出片_别的镜和首帧不受影响() {
        let d = tempfile::tempdir().unwrap();
        let id = start_exclusive(d.path(), JobKind::Video, "p1", "e1", "s1", 1, "").unwrap();
        let dup = start_exclusive(d.path(), JobKind::Video, "p1", "e1", "s1", 1, "").unwrap_err();
        assert_eq!(dup.id, id);
        assert!(start_exclusive(d.path(), JobKind::Video, "p1", "e1", "s2", 2, "").is_ok());
        assert!(start_exclusive(d.path(), JobKind::Frame, "p1", "e1", "s1", 1, "").is_ok());
        finish(d.path(), &id, Some("/tmp/a.mp4"), None);
        assert!(start_exclusive(d.path(), JobKind::Video, "p1", "e1", "s1", 1, "").is_ok(), "跑完了就能再出");
    }

    #[test]
    fn 进度和结果备注落盘_回的是最新那条() {
        let d = tempfile::tempdir().unwrap();
        let id = start(d.path(), JobKind::ShotTable, "p1", "e1", "", 0, "第 1 集");
        let j = set_progress(d.path(), &id, 1, 4, "金銮殿 · 日").unwrap();
        assert_eq!(j.progress, Some(JobProgress { done: 1, total: 4, label: "金銮殿 · 日".into(), eta_at: None }));
        assert_eq!(get(d.path(), &id).unwrap().progress.unwrap().done, 1, "进度没落盘");
        let notes = vec!["资产库里没有「皇帝」".to_string()];
        let j = finish_with(d.path(), &id, None, Some("4 场 10 镜"), &notes, None).unwrap();
        assert_eq!(j.status, JobStatus::Done);
        let back = get(d.path(), &id).unwrap();
        assert_eq!(back.summary.as_deref(), Some("4 场 10 镜"));
        assert_eq!(back.notes, notes, "备注没落盘 —— 切走页面就丢了");
        // 生成镜头表按集互斥
        let a = start_exclusive(d.path(), JobKind::ShotTable, "p1", "e2", "", 0, "").unwrap();
        assert!(start_exclusive(d.path(), JobKind::ShotTable, "p1", "e2", "", 0, "").is_err());
        finish(d.path(), &a, None, None);
    }

    #[test]
    fn 重启后没跑完的镜头表生成也判失败() {
        let d = tempfile::tempdir().unwrap();
        let id = start(d.path(), JobKind::ShotTable, "p1", "e1", "", 0, "");
        assert_eq!(reap_stale(d.path()), 1);
        let j = get(d.path(), &id).unwrap();
        assert_eq!(j.status, JobStatus::Failed);
        assert!(j.error.unwrap().contains("没跑完"));
    }

    /// **失败原文要照抄。** 概括过的错误没法用来排查 ——
    /// 「生成失败」和「ComfyUI 没起」是完全不同的两件事。
    #[test]
    fn 失败原文原样留下() {
        let d = tempfile::tempdir().unwrap();
        let id = start(d.path(), JobKind::Frame, "p1", "e1", "s1", 1, "x");
        finish(d.path(), &id, None, Some("connection refused: 127.0.0.1:8188"));
        let j = load(d.path()).jobs.pop().unwrap();
        assert_eq!(j.status, JobStatus::Failed);
        assert_eq!(j.error.as_deref(), Some("connection refused: 127.0.0.1:8188"));
    }

    /// 进程没了就没人给这些任务收尾了 —— 不清的话队列永远挂着。
    #[test]
    fn 重启后把上次没跑完的判为失败() {
        let d = tempfile::tempdir().unwrap();
        start(d.path(), JobKind::Frame, "p1", "e1", "s1", 1, "x");
        start(d.path(), JobKind::Frame, "p1", "e1", "s2", 2, "y");
        assert_eq!(reap_stale(d.path()), 2);
        let all = load(d.path()).jobs;
        assert!(all.iter().all(|j| j.status == JobStatus::Failed));
        assert!(all[0].error.as_deref().unwrap().contains("重新生成"));
        // 再跑一次不该重复处理
        assert_eq!(reap_stale(d.path()), 0);
    }

    #[test]
    fn 提示词过长会截断_不会把整份日志撑大() {
        let d = tempfile::tempdir().unwrap();
        start(d.path(), JobKind::Frame, "p1", "e1", "s1", 1, &"字".repeat(5000));
        assert_eq!(load(d.path()).jobs[0].prompt.chars().count(), 200);
    }
}
