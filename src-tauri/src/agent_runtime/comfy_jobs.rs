//! 提交给 ComfyUI、还没收回结果的任务 —— 记下来，开机时对账。
//!
//! ## 修的是什么
//!
//! `prompt_id` 以前**只活在内存里**：轮询循环持有它，循环结束它就没了。
//! 关掉 Anima 再打开，ComfyUI 那边任务还在跑（实测抓到过一个正在跑的
//! `MiniMaxH3ReferenceToVideo`），而 Anima 完全不知道它存在。用户看到对话里
//! 什么都没有，于是再说一次 —— 又排一个新任务，而先跑完的那个结果**永远回不来**。
//!
//! 一台机器上串行跑视频，多排一个就是几分钟到几十分钟的白等。
//!
//! ## 为什么用进程级格子
//!
//! 图片/视频工具是在 service 建好**之前**就注册进 registry 的，`Memory` 传不进去
//! （同样的形状见 [`super::subagent`] 的网关和 `knowledge_tool::MemoryCell`）。
//! 所以这里放一个进程级的格子，启动时填。没填的时候所有函数静默 no-op ——
//! 记账失败**绝不能**影响出图本身。

use std::sync::OnceLock;

use serde_json::Value;

use super::MemoryCell;

/// 和 `image_tool` / `video_tool` 用的是同一个地址；那两处也各自定义了一份常量。
const DEFAULT_COMFY_URL: &str = "http://127.0.0.1:8188";

fn cell() -> &'static MemoryCell {
    static CELL: OnceLock<MemoryCell> = OnceLock::new();
    CELL.get_or_init(Default::default)
}

/// 启动时把真正的 `Memory` 填进来。
///
/// **必须 await 完再对账。** 之前这里是 `tokio::spawn`，格子填进去的时机和
/// `reconcile_on_startup` 读它的时机没有先后关系 —— 读到 `None` 就静默返回，
/// 整个回收一声不响地不发生。这正是这个项目反复出现的那类 bug。
pub async fn set_memory(memory: std::sync::Arc<crate::memory::Memory>) {
    *cell().write().await = Some(memory);
}

// ── 取消 ────────────────────────────────────────────────────────────
//
// # 为什么不能只把状态改成「已取消」
//
// `jobs.rs` 的模块注释里立过一条规矩：**在能真取消之前，一个取消入口都不给** ——
// 因为「点了状态变成已取消、ComfyUI 照跑」是这个项目最典型的病。
// 现在能真取消了，靠的就是下面这张表：任务 id → 它发给 ComfyUI 的 prompt_id。
//
// # 为什么是 task_local 而不是往下传参
//
// `submit_and_fetch_video` 被七八条路径调用，每条都加一个 `job_id: Option<&str>`
// 参数的话，漏改一处就是「这条路上的任务取消不掉」，而且**不报错**。
// task_local 是「谁开的任务谁自己知道」，漏改的路径顶多取消不了，不会取消错人。

tokio::task_local! {
    /// 当前后台任务的 id。`canvas_job_start` 这类地方开 task 时 scope 进来。
    pub static JOB_ID: String;
}

type Submitted = std::collections::HashMap<String, Vec<String>>;

fn submitted() -> &'static std::sync::Mutex<Submitted> {
    static M: OnceLock<std::sync::Mutex<Submitted>> = OnceLock::new();
    M.get_or_init(Default::default)
}

fn cancelled() -> &'static std::sync::Mutex<std::collections::HashSet<String>> {
    static M: OnceLock<std::sync::Mutex<std::collections::HashSet<String>>> = OnceLock::new();
    M.get_or_init(Default::default)
}

/// 刚把一个 prompt 交给 ComfyUI —— 记到当前任务名下，取消时要用。
///
/// 不在任务里（对话那条路、命令行）就什么也不做。
pub fn attach(prompt_id: &str) {
    let _ = JOB_ID.try_with(|job| {
        if let Ok(mut m) = submitted().lock() {
            m.entry(job.clone()).or_default().push(prompt_id.to_string());
        }
    });
}

/// 这个任务发给 ComfyUI 的所有 prompt_id。
pub fn prompt_ids_of(job_id: &str) -> Vec<String> {
    submitted().lock().ok().and_then(|m| m.get(job_id).cloned()).unwrap_or_default()
}

/// 任务结束（成功或失败）——把它的登记清掉，别让表一直长。
pub fn forget(job_id: &str) {
    if let Ok(mut m) = submitted().lock() {
        m.remove(job_id);
    }
    if let Ok(mut c) = cancelled().lock() {
        c.remove(job_id);
    }
}

/// 标记「用户点了取消」。
///
/// **轮询循环要自己看这个标记**：ComfyUI 收到 interrupt 之后
/// 不一定会在 `/history` 里留下任何东西，循环只靠「查不到」判断的话，
/// 还得等上一轮轮询间隔才停 —— 而用户点完就想看到它停。
pub fn mark_cancelled(job_id: &str) {
    if let Ok(mut c) = cancelled().lock() {
        c.insert(job_id.to_string());
    }
}

/// 当前任务是不是已经被点了取消。
pub fn cancel_requested() -> bool {
    JOB_ID
        .try_with(|job| cancelled().lock().is_ok_and(|c| c.contains(job.as_str())))
        .unwrap_or(false)
}

/// 真去 ComfyUI 把这些 prompt 撤掉：排队中的直接删，正在跑的 interrupt。
///
/// 两件都要做 —— 只 delete 撤不掉正在跑的那个，只 interrupt 撤不掉后面排着的。
pub async fn cancel_prompts(prompt_ids: &[String]) -> usize {
    if prompt_ids.is_empty() {
        return 0;
    }
    let client = reqwest::Client::new();
    let _ = client
        .post(format!("{DEFAULT_COMFY_URL}/queue"))
        .json(&serde_json::json!({ "delete": prompt_ids }))
        .send()
        .await;
    // 删完再看队列：还在 running 里的，说明它正在跑，delete 撤不掉，要 interrupt。
    let queue = match client.get(format!("{DEFAULT_COMFY_URL}/queue")).send().await {
        Ok(r) => r.json::<Value>().await.ok(),
        Err(_) => None,
    };
    for id in prompt_ids {
        if queue.as_ref().is_some_and(|q| queue_contains(q, id)) {
            let _ = client
                .post(format!("{DEFAULT_COMFY_URL}/interrupt"))
                .json(&serde_json::json!({ "prompt_id": id }))
                .send()
                .await;
        }
    }
    prompt_ids.len()
}

/// 记下一个刚提交出去的任务。**失败只记日志**，不往上抛。
pub async fn remember(prompt_id: &str, session_id: &str, kind: &str, save_node: &str, summary: &str) {
    remember_to(prompt_id, session_id, kind, save_node, summary, None).await;
}

/// 同上，但**说清楚产物该落到哪个文件**。
///
/// 对话里那条路没有固定归属（结果进 `generated/` 再贴进消息），传 `None`。
/// 分镜和创作台一定要给 —— 它们没有会话，`session_id` 是空串，
/// 唯一的交付动作只能是「把文件写到该去的地方」。
///
/// 不给的后果是实测过的：两段 r2v 视频在 ComfyUI 那边出好了、
/// `download_outputs` 也下载成功了，然后 `settle` 拿空 session 去
/// `append_message`，撞外键、整个 settle 失败、任务销不掉 ——
/// 每次开机重试同一次失败，视频永远回不到那一镜。
pub async fn remember_to(
    prompt_id: &str,
    session_id: &str,
    kind: &str,
    save_node: &str,
    summary: &str,
    dest: Option<&std::path::Path>,
) {
    let Some(mem) = cell().read().await.clone() else {
        return;
    };
    let dest = dest.map(|p| p.to_string_lossy().to_string());
    if let Err(err) = mem
        .store()
        .record_comfy_job(prompt_id, session_id, kind, save_node, summary, dest.as_deref())
        .await
    {
        tracing::warn!(%err, prompt_id, "ComfyUI 任务记账失败");
    }
}

/// 结果已经收回（或确认失败），不用再开机回收了。
pub async fn finish(prompt_id: &str) {
    let Some(mem) = cell().read().await.clone() else {
        return;
    };
    if let Err(err) = mem.store().finish_comfy_job(prompt_id).await {
        tracing::warn!(%err, prompt_id, "ComfyUI 任务销账失败");
    }
}

/// 给用户看的一句话。提示词可以上千字，回收时只需要认得出是哪个任务。
pub fn summarize(prompt: &str) -> String {
    let one_line = prompt.replace('\n', " ");
    let trimmed = one_line.trim();
    let mut out: String = trimmed.chars().take(60).collect();
    if trimmed.chars().count() > 60 {
        out.push('…');
    }
    out
}

/// 一个挂着的任务在 ComfyUI 那边的实际下落。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobFate {
    /// 还在队列里（running 或 pending）—— 别重复提交，等它。
    StillRunning,
    /// 跑完了，产出在这些 `(filename, subfolder, type)` 里。
    Finished(Vec<OutputRef>),
    /// ComfyUI 既不认它、也没有它的产出。多半是 ComfyUI 重启过。
    Lost,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputRef {
    pub filename: String,
    pub subfolder: String,
    pub kind: String,
}

/// 判定一个任务的下落。
///
/// **纯函数**，喂 `/queue` 和 `/history/{id}` 的 JSON 原文。分出来是为了能测 ——
/// 三种下落里最要命的是把"还在跑"误判成"丢了"：那正是用户会去重复提交的时刻。
pub fn classify(prompt_id: &str, queue: &Value, history: &Value, save_nodes: &str) -> JobFate {
    // 先看产出。**顺序不能反** —— 任务可能刚跑完，两处都能查到，
    // 这时候有结果就该收结果，而不是继续等一个已经出队的任务。
    // 出图节点在不同引擎下 id 不一样，视频链路更是要试 "10"/"9"/"outputs"
    // 三个候选。所以这里存的是逗号分隔的候选表，逐个试。
    for save_node in save_nodes.split(',').map(str::trim).filter(|n| !n.is_empty()) {
    for key in ["images", "gifs", "videos", "audio"] {
        if let Some(arr) = history
            .pointer(&format!("/{prompt_id}/outputs/{save_node}/{key}"))
            .and_then(|v| v.as_array())
        {
            let outs: Vec<OutputRef> = arr
                .iter()
                .map(|o| OutputRef {
                    filename: o.get("filename").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                    subfolder: o.get("subfolder").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                    kind: o.get("type").and_then(|v| v.as_str()).unwrap_or("output").to_string(),
                })
                // `type: input` 是 LoadVideo 报出来的它载入的那条视频，不是产出（见 weetodd::find_video_output）
                .filter(|o| !o.filename.is_empty() && o.kind != "input")
                .collect();
            if !outs.is_empty() {
                return JobFate::Finished(outs);
            }
        }
    }
    }

    if queue_contains(queue, prompt_id) {
        return JobFate::StillRunning;
    }

    JobFate::Lost
}

/// `/queue` 里认不认这个 prompt_id。
///
/// 队列项的形状是 `[number, prompt_id, graph, extra, outputs]` —— prompt_id 在
/// **第 1 位**，不是第 0 位。写错的话每个任务都会被判成"丢了"，而且不报任何错。
pub fn queue_contains(queue: &Value, prompt_id: &str) -> bool {
    ["queue_running", "queue_pending"].iter().any(|k| {
        queue
            .get(k)
            .and_then(|v| v.as_array())
            .is_some_and(|items| {
                items
                    .iter()
                    .any(|it| it.get(1).and_then(|v| v.as_str()) == Some(prompt_id))
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn queue_with(running: &[&str], pending: &[&str]) -> Value {
        json!({
            "queue_running": running.iter().map(|p| json!([0, p, {}, {}, []])).collect::<Vec<_>>(),
            "queue_pending": pending.iter().map(|p| json!([1, p, {}, {}, []])).collect::<Vec<_>>(),
        })
    }

    #[test]
    fn 在跑的任务不能被判成丢了() {
        // 判错这一格，用户就会去重复提交 —— 一台机器串行跑视频，代价是几十分钟
        let fate = classify("abc", &queue_with(&["abc"], &[]), &json!({}), "9");
        assert_eq!(fate, JobFate::StillRunning);
    }

    #[test]
    fn 排队中的也算在跑() {
        assert_eq!(
            classify("abc", &queue_with(&[], &["abc"]), &json!({}), "9"),
            JobFate::StillRunning
        );
    }

    #[test]
    fn 队列项的_prompt_id_在第_1_位() {
        // 用真实形状钉死：写成 it[0] 的话下面这个会被判成不在队列里
        let q = queue_with(&["abc"], &[]);
        assert!(queue_contains(&q, "abc"));
        assert!(!queue_contains(&q, "0"), "第 0 位是序号，不该被当成 id");
    }

    #[test]
    fn 跑完的任务收得回产出() {
        let hist = json!({
            "abc": { "outputs": { "9": { "images": [
                { "filename": "img_42.png", "subfolder": "", "type": "output" }
            ] } } }
        });
        match classify("abc", &json!({}), &hist, "9") {
            JobFate::Finished(o) => {
                assert_eq!(o.len(), 1);
                assert_eq!(o[0].filename, "img_42.png");
            }
            other => panic!("应该收回产出，实际 {other:?}"),
        }
    }

    #[test]
    fn 视频产出也认() {
        let hist = json!({
            "abc": { "outputs": { "20": { "videos": [
                { "filename": "vid_7.mp4", "subfolder": "sub", "type": "output" }
            ] } } }
        });
        match classify("abc", &json!({}), &hist, "20") {
            JobFate::Finished(o) => assert_eq!(o[0].filename, "vid_7.mp4"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn 载入的输入视频不算产出() {
        // 动作迁移的 LTX 图里 LoadVideo 是节点 4，而恢复时的候选表是 "7,4"
        let hist = json!({
            "abc": { "outputs": { "4": { "images": [
                { "filename": "anima_depthctl_x.mp4", "subfolder": "", "type": "input" }
            ], "animated": [true] } } }
        });
        assert!(!matches!(classify("abc", &json!({}), &hist, "7,4"), JobFate::Finished(_)));
    }

    #[test]
    fn 有产出时优先收结果而不是继续等() {
        // 刚跑完的一瞬间两处都查得到。继续等的话会一直等到轮询预算用完
        let hist = json!({
            "abc": { "outputs": { "9": { "images": [
                { "filename": "img_1.png", "subfolder": "", "type": "output" }
            ] } } }
        });
        assert!(matches!(
            classify("abc", &queue_with(&["abc"], &[]), &hist, "9"),
            JobFate::Finished(_)
        ));
    }

    #[test]
    fn 候选节点表里任意一个命中都算跑完() {
        // 视频链路的出图节点要试 "10"/"9"/"outputs" 三个，只认第一个的话
        // 走 outputs 的那些任务会被判成丢了
        let hist = json!({
            "abc": { "outputs": { "outputs": { "gifs": [
                { "filename": "a.webp", "subfolder": "", "type": "output" }
            ] } } }
        });
        match classify("abc", &json!({}), &hist, "10,9,outputs") {
            JobFate::Finished(o) => assert_eq!(o[0].filename, "a.webp"),
            other => panic!("{other:?}"),
        }
    }

    /// 用**这台机器上真实抓下来的** `/queue` 与 `/history` 形状钉死解析。
    ///
    /// 队列项和产出的形状都是文档外的约定，靠猜的话写错了也不报错，
    /// 只会表现成「每个任务都被判成丢了」。抓这两份的时候机器上正好挂着一个
    /// 孤儿任务（`MiniMaxH3ReferenceToVideo`）—— 就是这次要修的那个现场。
    #[test]
    fn 真实的_comfyui_响应形状() {
        let queue = json!({
            "queue_running": [[37, "f9d0cf5e-e826-45f3-9c3c-47ad4917ece2", {}, {}, []]],
            "queue_pending": []
        });
        let history = json!({
            "bfb05bf0-c3d4-44b1-be00-cbed6a27c5b1": {
                "outputs": { "10": { "images": [
                    { "filename": "anima_img_00029_.png", "subfolder": "", "type": "output" }
                ] } }
            }
        });

        // 真在跑的那个：必须判成还在跑，否则用户就会去重复提交
        assert_eq!(
            classify("f9d0cf5e-e826-45f3-9c3c-47ad4917ece2", &queue, &json!({}), "10,9,outputs"),
            JobFate::StillRunning
        );
        // 真跑完的那个：必须收得回文件名
        match classify("bfb05bf0-c3d4-44b1-be00-cbed6a27c5b1", &queue, &history, "10,9,outputs") {
            JobFate::Finished(o) => assert_eq!(o[0].filename, "anima_img_00029_.png"),
            other => panic!("真实历史应该收回产出，实际 {other:?}"),
        }
    }

    #[test]
    fn 两边都不认才算丢了() {
        assert_eq!(
            classify("abc", &queue_with(&["xyz"], &["def"]), &json!({}), "9"),
            JobFate::Lost
        );
    }

    #[test]
    fn 空产出数组不算跑完() {
        // ComfyUI 报了这个节点但里面是空的，收不到东西 —— 当作没跑完，别写一条空消息
        let hist = json!({ "abc": { "outputs": { "9": { "images": [] } } } });
        assert_eq!(classify("abc", &json!({}), &hist, "9"), JobFate::Lost);
    }
}

// ───────────────────────── 开机对账 ─────────────────────────

/// 一条挂账任务对完之后的处置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Recovery {
    /// 结果收回来了，往会话里补一条消息
    Recovered { session_id: String, markdown: String },
    /// 还在跑，界面上给个提示，别让用户重复提交
    StillRunning { session_id: String, summary: String },
    /// 真丢了（多半是 ComfyUI 重启过），也得说一声
    Lost { session_id: String, summary: String },
}

/// 把一条挂账任务的下落翻译成给用户的处置。
///
/// **纯函数**，和 HTTP 分开是为了能测：三条岔路各自的措辞和归属会话
/// 都得对，而这三种情况在真机上很难凑齐。
pub fn plan_recovery(job: &crate::memory::store::ComfyJob, fate: &JobFate, files: &[String]) -> Recovery {
    let sid = job.session_id.clone();
    match fate {
        JobFate::Finished(_) => {
            let body = files
                .iter()
                .map(|p| {
                    if job.kind == "video" {
                        format!("[生成的视频](file://{p})")
                    } else {
                        format!("![生成的图像](file://{p})")
                    }
                })
                .collect::<Vec<_>>()
                .join("\n\n");
            Recovery::Recovered {
                session_id: sid,
                markdown: format!(
                    "⏳ 这个任务在 Anima 关闭期间跑完了 —— 结果补在这里：\n\n{body}\n\n（原始请求：{}）",
                    job.summary
                ),
            }
        }
        JobFate::StillRunning => Recovery::StillRunning {
            session_id: sid,
            summary: job.summary.clone(),
        },
        JobFate::Lost => Recovery::Lost {
            session_id: sid,
            summary: job.summary.clone(),
        },
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    use crate::memory::store::ComfyJob;

    fn job(kind: &str) -> ComfyJob {
        ComfyJob {
            prompt_id: "p1".into(),
            session_id: "sess-9".into(),
            kind: kind.into(),
            save_node: "9".into(),
            summary: "图书馆里的少女".into(),
            dest: None,
            created_at: 0,
        }
    }

    /// 开机对账接着盯还在跑的任务：**看进度不看时长**，读不出来不算丢（2026-09-30）。
    /// 以前固定两小时，一条十二小时的长镜头中途重启一次 Anima 就盯不到它跑完
    #[test]
    fn 接着盯_看进度不看时长_读不出来不算丢() {
        let src = include_str!("comfy_jobs.rs");
        // 带上行首的换行找定义 —— 测试模块在函数前面，不带的话先找到的是这里的字符串
        let body = &src[src.find("\nasync fn watch_until_done(").unwrap()..];
        let body = &body[..body.find("\n}\n").unwrap()];
        assert!(body.contains("comfy_watch::tracker(&job.prompt_id)") && body.contains("watch.observe(seen)"));
        assert!(body.contains("let (Some(queue), Some(history)) = ("), "读不出队列时不能当空队列去判");
        assert!(!body.contains("unwrap_or_default()"), "读不出来当成空的，会把在跑的判成丢了");
        assert!(!body.contains("for _ in 0.."), "不能再有按轮数的上限");
    }

    /// 有固定归属的任务 —— 分镜/创作台那条路。
    fn job_to(kind: &str, dest: &std::path::Path) -> ComfyJob {
        ComfyJob {
            session_id: String::new(), // **没有会话**，这正是出过事的那个形状
            dest: Some(dest.to_string_lossy().to_string()),
            ..job(kind)
        }
    }

    #[test]
    fn 收回的图片补进原来那个会话() {
        // 补错会话等于没补 —— 用户在自己那条对话里还是什么都看不到
        let r = plan_recovery(
            &job("image"),
            &JobFate::Finished(vec![]),
            &["/x/img_1.png".to_string()],
        );
        match r {
            Recovery::Recovered { session_id, markdown } => {
                assert_eq!(session_id, "sess-9");
                assert!(markdown.contains("![生成的图像](file:///x/img_1.png)"));
                assert!(markdown.contains("图书馆里的少女"), "要认得出是哪个任务");
            }
            other => panic!("{other:?}"),
        }
    }

    // ── 交付：把收回来的文件放回它该去的地方 ──

    /// 这是 2026-09-06 那次实际丢东西的形状：
    /// r2v 在 ComfyUI 那边出好了、回收也下载成功了，
    /// 然后没人知道该把文件放回哪一镜。
    #[test]
    fn 有归属的任务把文件放回原位() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("cinema_485999321.mp4");
        std::fs::write(&src, b"\x00\x00\x00\x18ftypmp42").unwrap();
        let dest = d.path().join("shots").join("shot_1.mp4"); // 目录还不存在

        let job = job_to("video", &dest);
        assert!(deliver(&job, &[src.to_string_lossy().to_string()]), "没交付");
        assert!(dest.is_file(), "文件没放到位");
        assert_eq!(std::fs::read(&dest).unwrap(), std::fs::read(&src).unwrap());
    }

    /// **目标已经有文件就不覆盖。** 有文件说明正常那条路已经收回来过了 ——
    /// 盖上去是拿一份旧产物换掉新的。
    #[test]
    fn 目标已存在时不覆盖() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("new.mp4");
        std::fs::write(&src, b"new").unwrap();
        let dest = d.path().join("shot_1.mp4");
        std::fs::write(&dest, b"already here").unwrap();

        assert!(!deliver(&job_to("video", &dest), &[src.to_string_lossy().to_string()]));
        assert_eq!(std::fs::read(&dest).unwrap(), b"already here");
    }

    /// 对话那条路没有固定归属 —— 结果进 `generated/` 再贴进消息，
    /// 这里什么都不该做。
    #[test]
    fn 没有归属的任务不落盘() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("x.png");
        std::fs::write(&src, b"png").unwrap();
        assert!(!deliver(&job("image"), &[src.to_string_lossy().to_string()]));
        // 空串也算没有归属，不能当成「放到当前目录」
        let mut j = job("image");
        j.dest = Some("   ".into());
        assert!(!deliver(&j, &[src.to_string_lossy().to_string()]));
    }

    #[test]
    fn 一个文件都没收回时不假装交付() {
        let d = tempfile::tempdir().unwrap();
        let dest = d.path().join("shot_1.mp4");
        assert!(!deliver(&job_to("video", &dest), &[]));
        assert!(!dest.exists());
    }

    /// **有归属的那条路 `session_id` 是空的。**
    /// 拿空串去 `append_message` 必然撞外键，而那个失败以前会把销账一起带走 ——
    /// 于是每次开机重试同一次失败，任务永远清不掉。
    #[test]
    fn 有归属的任务确实没有会话() {
        let d = tempfile::tempdir().unwrap();
        let j = job_to("video", &d.path().join("shot_1.mp4"));
        assert!(j.session_id.trim().is_empty(), "这条路本来就没有会话");
        assert!(j.dest.is_some(), "没有归属就没法交付");
    }

    #[test]
    fn 视频不用图片语法() {
        // 写成 ![] 的话 markdown 会当成图片渲染，出来一个坏掉的图标
        let r = plan_recovery(&job("video"), &JobFate::Finished(vec![]), &["/x/v.mp4".into()]);
        match r {
            Recovery::Recovered { markdown, .. } => {
                assert!(markdown.contains("[生成的视频](file:///x/v.mp4)"));
                assert!(!markdown.contains("!["));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn 一次多张全都补上() {
        let r = plan_recovery(
            &job("image"),
            &JobFate::Finished(vec![]),
            &["/x/a.png".into(), "/x/b.png".into()],
        );
        match r {
            Recovery::Recovered { markdown, .. } => {
                assert!(markdown.contains("a.png") && markdown.contains("b.png"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn 还在跑的要说出来而不是静悄悄() {
        // 不说的话用户看不到任何东西，只会再提交一次 —— 那正是这次要修的
        assert_eq!(
            plan_recovery(&job("video"), &JobFate::StillRunning, &[]),
            Recovery::StillRunning {
                session_id: "sess-9".into(),
                summary: "图书馆里的少女".into()
            }
        );
    }

    #[test]
    fn 丢了也要说一声() {
        // 静默丢弃的话用户会一直等一个永远不来的结果
        assert!(matches!(
            plan_recovery(&job("image"), &JobFate::Lost, &[]),
            Recovery::Lost { .. }
        ));
    }
}

/// 开机对账：把上次没收回的任务一个个问清楚。
///
/// 三条岔路：
/// - **跑完了** → 下载产出、补一条消息进原来那条会话、销账
/// - **还在跑** → **接着盯**（不是弹个提示就算完）。视频一跑十几分钟，
///   用户等的就是这个结果；盯到它出来再补进去，才叫真的没丢。
/// - **丢了** → 也补一条说明。静默丢弃的话用户会一直等一个永远不来的结果。
///
/// 整个过程失败都只记日志 —— 这是收尾工作，**不能挡住应用启动**。
pub async fn reconcile_on_startup(client: reqwest::Client, generated_dir: std::path::PathBuf) {
    let Some(mem) = cell().read().await.clone() else {
        return;
    };
    let jobs = match mem.store().pending_comfy_jobs().await {
        Ok(j) => j,
        Err(err) => {
            tracing::warn!(%err, "读取挂账的 ComfyUI 任务失败");
            return;
        }
    };
    if jobs.is_empty() {
        return;
    }
    tracing::info!(count = jobs.len(), "发现上次没收回的 ComfyUI 任务，开始对账");

    let base = DEFAULT_COMFY_URL;
    let queue: Value = fetch_json(&client, &format!("{base}/queue")).await.unwrap_or_default();

    for job in jobs {
        let history: Value = fetch_json(&client, &format!("{base}/history/{}", job.prompt_id))
            .await
            .unwrap_or_default();
        let fate = classify(&job.prompt_id, &queue, &history, &job.save_node);

        match fate {
            JobFate::StillRunning => {
                // 接着盯。这一支不销账 —— 中途再关一次，下次开机还认得它。
                let c = client.clone();
                let dir = generated_dir.clone();
                tokio::spawn(async move { watch_until_done(c, dir, job).await });
            }
            ref f => {
                let files = download_outputs(&client, f, &generated_dir).await;
                settle(&mem, &job, f, &files).await;
            }
        }
    }
}

/// 盯着一个还在跑的任务，直到它出结果或者真的丢了。
async fn watch_until_done(
    client: reqwest::Client,
    generated_dir: std::path::PathBuf,
    job: crate::memory::store::ComfyJob,
) {
    let base = DEFAULT_COMFY_URL;
    // 判据是「ComfyUI 还认不认这个任务」，**要不要接着等看它还有没有进度**（`comfy_watch`），不看过了多久。
    // 以前是固定两小时 —— 一条三窗口长镜头要十二个多小时（2026-09-30），开着 Anima 重启一次就盯不到它跑完。
    let mut watch = super::comfy_watch::tracker(&job.prompt_id);
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(15)).await;
        // **读不出来不是「没了」**：以前读不出队列就当空队列，ComfyUI 卡一下就把在跑的任务判成丢了
        let (Some(queue), Some(history)) = (
            fetch_json(&client, &format!("{base}/queue")).await,
            fetch_json(&client, &format!("{base}/history/{}", job.prompt_id)).await,
        ) else {
            if let Some(why) = watch.observe(super::comfy_watch::Seen::Unreachable).explain(&job.prompt_id) {
                tracing::warn!(prompt_id = %job.prompt_id, %why, "放弃跟进（记录仍保留，下次开机再对账）");
                return;
            }
            continue;
        };
        let fate = classify(&job.prompt_id, &queue, &history, &job.save_node);
        if fate == JobFate::StillRunning {
            let seen = if super::video_tool::queue_pending_only(&queue, &job.prompt_id) {
                super::comfy_watch::Seen::Queued
            } else {
                super::comfy_watch::Seen::Running
            };
            if let Some(why) = watch.observe(seen).explain(&job.prompt_id) {
                tracing::warn!(prompt_id = %job.prompt_id, %why, "放弃跟进（记录仍保留，下次开机再对账）");
                return;
            }
            continue;
        }
        let Some(mem) = cell().read().await.clone() else {
            return;
        };
        let files = download_outputs(&client, &fate, &generated_dir).await;
        settle(&mem, &job, &fate, &files).await;
        return;
    }
}

/// 把处置写进会话并销账。
/// 把收回来的文件放到它该去的地方。
///
/// **只在目标还不存在时放。** 已经有文件说明正常那条路已经收回来过了 ——
/// 覆盖它是拿一份旧产物盖掉新的。
///
/// 返回是否真的放了（`false` 包括「没有归属」和「目标已存在」两种）。
fn deliver(job: &crate::memory::store::ComfyJob, files: &[String]) -> bool {
    let Some(dest) = job.dest.as_deref().map(str::trim).filter(|d| !d.is_empty()) else {
        return false;
    };
    let Some(src) = files.first() else { return false };
    let dest = std::path::Path::new(dest);
    if dest.is_file() {
        tracing::info!(%src, dest = %dest.display(), "回收到的产物：目标已存在，不覆盖");
        return false;
    }
    if let Some(parent) = dest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::copy(src, dest) {
        Ok(_) => {
            tracing::info!(%src, dest = %dest.display(), "把上次没收回的产物放回原位");
            true
        }
        Err(err) => {
            tracing::warn!(%err, %src, dest = %dest.display(), "回收产物落盘失败");
            false
        }
    }
}

/// 把处置写进会话并销账。
///
/// # 交付和通知是两件事
///
/// 以前只有「通知」这一件：往会话里追加一条 markdown。分镜和创作台
/// **没有会话**，于是空 `session_id` 撞外键、整个 settle 失败、
/// 任务永远销不掉 —— 而文件其实已经下载好了，就差最后一步。
///
/// 现在先交付（有归属就把文件放回原位），再看要不要通知；
/// **通知失败不阻止销账** —— 产物已经到位了，为了一条聊天消息
/// 让整件事重来一遍是本末倒置。
async fn settle(
    mem: &std::sync::Arc<crate::memory::Memory>,
    job: &crate::memory::store::ComfyJob,
    fate: &JobFate,
    files: &[String],
) {
    let text = match plan_recovery(job, fate, files) {
        Recovery::Recovered { markdown, .. } => markdown,
        Recovery::Lost { summary, .. } => format!(
            "⚠️ 上次提交的任务（{summary}）在 ComfyUI 那边已经查不到了 —— \
             多半是 ComfyUI 期间重启过。结果拿不回来了，需要的话重新说一次。"
        ),
        // 还在跑的不会走到这里
        Recovery::StillRunning { .. } => return,
    };

    deliver(job, files);

    // **空 session 不去追加消息。** 那不是「消息发失败」，是这条任务
    // 压根不属于任何会话 —— 拿空串去查外键必然失败，而失败会把销账也带走。
    let session = job.session_id.trim();
    if !session.is_empty() {
        if let Err(err) = mem
            .store()
            .append_message(session, crate::llm::Role::Assistant, &text)
            .await
        {
            // 只记日志，**继续销账** —— 产物已经交付了。
            tracing::warn!(%err, session, "补写回收结果失败（产物已按归属落盘）");
        }
    }
    finish(&job.prompt_id).await;
}

async fn fetch_json(client: &reqwest::Client, url: &str) -> Option<Value> {
    let resp = client
        .get(url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .ok()?;
    resp.json().await.ok()
}

/// 把 ComfyUI 那边的产出拉到本地 `generated/`，返回落地后的绝对路径。
async fn download_outputs(
    client: &reqwest::Client,
    fate: &JobFate,
    generated_dir: &std::path::Path,
) -> Vec<String> {
    let JobFate::Finished(outs) = fate else {
        return Vec::new();
    };
    if let Err(err) = std::fs::create_dir_all(generated_dir) {
        tracing::warn!(%err, "建 generated 目录失败");
        return Vec::new();
    }
    let base = DEFAULT_COMFY_URL;
    let mut paths = Vec::new();
    for o in outs {
        let url = format!(
            "{base}/view?filename={}&subfolder={}&type={}",
            urlencoding::encode(&o.filename),
            urlencoding::encode(&o.subfolder),
            urlencoding::encode(&o.kind),
        );
        let Ok(resp) = client.get(&url).send().await else {
            continue;
        };
        let Ok(bytes) = resp.bytes().await else { continue };
        let target = generated_dir.join(&o.filename);
        if std::fs::write(&target, &bytes).is_ok() {
            paths.push(target.to_string_lossy().to_string());
        }
    }
    paths
}
