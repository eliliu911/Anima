//! **应用重启后接着出片**（2026-10-05 用户：「当程序关闭、进程关闭后，重启 Anima，需要有个自动恢复列队功能，
//! 并且自动恢复状态检测 ComfyUI 状态……不然那么长时间白生成」）。
//!
//! # 原来是什么样
//!
//! 启动时 [`crate::jobs::reap_stale`] 把上一次没跑完的一律判失败：
//! - 正在出的那一镜，ComfyUI 那边**照样在跑**（它是另一个进程），出完的片子没人收、不记 take —— 白跑；
//! - 批量出片后面没轮到的镜头，整批作废，要人重新点。
//!
//! # 现在
//!
//! 1. **正在出的那一镜**：出片时已经把「结果落哪、take 怎么记、ComfyUI 的 prompt_id」记在任务上
//!    （`Job::resume`，见 `cinema` 里的 `note_resume` 和 `video_tool::submit_and_fetch_video`）。
//!    重启后照它去 ComfyUI 收：出完了就下载、记 take；还在跑就接着等（和正常出片同一个等法，看进度不看时长）；
//!    ComfyUI 里没有了（ComfyUI 也重启过）就判失败，**并重新排进队列**。
//! 2. **批量出片没轮到的**：旧批量标「中断」，剩下的镜头开一条新批量。新批量先等上面那一镜收完、
//!    再等 ComfyUI 连得上，才开始 —— ComfyUI 没起来时不会一镜接一镜秒败。
//!
//! 记不全的（老任务、长镜头整段出、还没交给 ComfyUI 就关了的）照旧判失败；它们在批量里的话一样会被重新排上。
//!
//! **出首帧同一套**（同日用户：「出首帧也会列队自动恢复对吧」—— 原来不会）：正在出的那张去 ComfyUI 收图、裁好挂上；
//! 「按场出首帧」没轮到的开一条新的接着出（正在出的那张排在最后：收回来了就跳过，丢了才重出），跑完照常按首帧补合成。

use std::path::{Path, PathBuf};

use crate::jobs::{Job, JobKind};

/// 启动时要做的事（纯计算，不碰 ComfyUI，方便测）。
#[derive(Debug, Default)]
pub struct Plan {
    /// 续接得上的出片任务（有续接单、已经交给了 ComfyUI）
    pub videos: Vec<Job>,
    /// 没跑完的批量 → 还要出的镜头（按原顺序）
    pub batches: Vec<(Job, Vec<String>)>,
}

/// 这条任务续接得上吗：有续接单、已经拿到了 ComfyUI 的 prompt_id；
/// 出片还要知道结果落哪、take 怎么记，出首帧要知道是哪一镜。
fn resumable(j: &Job) -> bool {
    let Some(r) = j.resume.as_ref() else { return false };
    if !r["prompt_ids"].as_array().is_some_and(|a| !a.is_empty()) {
        return false;
    }
    match j.kind {
        JobKind::Video => r["out"].as_str().is_some_and(|s| !s.trim().is_empty()) && r.get("take").is_some(),
        JobKind::Frame => r["kind"] == "frame" && r["shot_id"].as_str().is_some_and(|s| !s.is_empty()),
        // 画布的出片（H3 / LTX，`video_tool::run_weetodd` 记的续接单）
        JobKind::Canvas => r["kind"] == "canvas_video" && r["out"].as_str().is_some_and(|s| !s.trim().is_empty()),
        _ => false,
    }
}

/// 批量是哪一类、它的每一镜是哪一类任务
fn unit_kind(batch: JobKind) -> Option<JobKind> {
    match batch {
        JobKind::VideoBatch => Some(JobKind::Video),
        JobKind::FrameBatch => Some(JobKind::Frame),
        _ => None,
    }
}

pub fn plan(running: &[Job]) -> Plan {
    let videos: Vec<Job> = running.iter().filter(|j| resumable(j)).cloned().collect();
    let batches = running
        .iter()
        .filter(|j| unit_kind(j.kind).is_some())
        .map(|b| {
            let unit = unit_kind(b.kind);
            // 正在出的那一镜的下标；「排队中」时一镜都没开始
            let current = b.progress.as_ref().filter(|p| !p.label.starts_with("排队中")).map_or(0, |p| p.done);
            let rest: Vec<String> = b
                .targets
                .iter()
                .enumerate()
                .filter(|(i, id)| {
                    if *i < current {
                        return false;
                    }
                    // 正在出的那一镜续接得上 → 由续接收尾，不再排
                    !(*i == current && videos.iter().any(|v| Some(v.kind) == unit && v.project_id == b.project_id && v.episode_id == b.episode_id && &v.shot_id == *id))
                })
                .map(|(_, id)| id.clone())
                .collect();
            (b.clone(), rest)
        })
        .collect();
    Plan { videos, batches }
}

/// 进程启动时调（取代原来的直接 `reap_stale`）。回续接的出片数和重新排上的镜头数。
pub fn start(app: Option<tauri::AppHandle>, config_dir: &Path) -> (usize, usize) {
    let p = plan(&crate::jobs::all_running(config_dir));
    let keep: Vec<String> = p.videos.iter().map(|j| j.id.clone()).collect();
    for (b, rest) in &p.batches {
        let msg = if rest.is_empty() {
            "应用退出时中断 —— 正在出的那一镜重启后接着收，没有别的镜头要排".to_string()
        } else {
            format!("应用退出时中断 —— 剩下的 {} 镜重启后已自动接着排队（新任务）", rest.len())
        };
        crate::jobs::finish_with(config_dir, &b.id, None, None, &[], Some(&msg));
    }
    let stale = crate::jobs::reap_stale_except(config_dir, &keep);
    if stale > 0 {
        tracing::warn!(stale, "上次退出时有任务没跑完，续接不上的已判为失败");
    }
    let requeued = p.batches.iter().map(|(_, r)| r.len()).sum();
    if p.videos.is_empty() && requeued == 0 {
        return (0, 0);
    }
    tracing::info!(videos = p.videos.len(), requeued, "上次退出时有片没出完：续接正在出的，剩下的接着排队");
    let dir = config_dir.to_path_buf();
    let n = p.videos.len();
    tauri::async_runtime::spawn(async move {
        // 正在出的首帧，属于某个「按场出首帧」、而那一批后面还有要出的 —— 由新的那一批收尾（补合成、丢了重出）
        let frame_batch_shots: Vec<(String, String, String)> = p
            .batches
            .iter()
            .filter(|(b, rest)| b.kind == JobKind::FrameBatch && !rest.is_empty())
            .flat_map(|(b, _)| b.targets.iter().map(move |t| (b.project_id.clone(), b.episode_id.clone(), t.clone())))
            .collect();
        let mut pending_frames: Vec<(String, String, String)> = Vec::new();
        for v in p.videos {
            let (app, dir) = (app.clone(), dir.clone());
            if v.kind == JobKind::Frame {
                let key = (v.project_id.clone(), v.episode_id.clone(), v.shot_id.clone());
                let in_batch = frame_batch_shots.contains(&key);
                if in_batch {
                    pending_frames.push(key);
                }
                tauri::async_runtime::spawn(async move { resume_frame(app, dir, v, !in_batch).await });
            } else if v.kind == JobKind::Canvas {
                tauri::async_runtime::spawn(async move { resume_canvas(app, dir, v).await });
            } else {
                tauri::async_runtime::spawn(async move { resume_one(app, dir, v).await });
            }
        }
        for (b, rest) in p.batches {
            if rest.is_empty() {
                continue;
            }
            if b.kind == JobKind::FrameBatch {
                let pending: Vec<String> = pending_frames.iter().filter(|(p, e, _)| p == &b.project_id && e == &b.episode_id).map(|(_, _, s)| s.clone()).collect();
                let mut ids = rest;
                ids.extend(pending.iter().cloned());
                let Some(app) = app.clone() else { continue };
                let Some((http, gateway)) = app_services(&app).await else { continue };
                match crate::cinema::start_frame_batch_with(app.clone(), dir.clone(), http, gateway, b.project_id.clone(), b.episode_id.clone(), ids, None, Vec::new(), true, pending) {
                    Ok(job) => crate::jobs::emit(Some(&app), Some(&job)),
                    Err(e) => tracing::warn!(error = %e, "重启后接着出首帧没排上"),
                }
                continue;
            }
            match crate::cinema::queue_render(app.clone(), dir.clone(), b.project_id.clone(), b.episode_id.clone(), rest) {
                Ok(job) => crate::jobs::emit(app.as_ref(), Some(&job)),
                Err(e) => tracing::warn!(error = %e, "重启后接着排队没排上"),
            }
        }
    });
    (n, requeued)
}

/// ComfyUI 那边不在了（重启过、取消了）、或者一直连不上 —— 这一镜该重新排，而不是只报失败。
fn should_requeue(error: &str) -> bool {
    ["没有这个任务", "连不上", "重启过", "没有响应", "unreachable"].iter().any(|k| error.contains(k))
}

async fn resume_one(app: Option<tauri::AppHandle>, config_dir: PathBuf, job: Job) {
    let r = job.resume.clone().unwrap_or_default();
    let out = PathBuf::from(r["out"].as_str().unwrap_or_default());
    let prompt_id = r["prompt_ids"].as_array().and_then(|a| a.last()).and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let generate_audio = r["generate_audio"].as_bool().unwrap_or(true);
    let burn = r["burn_subtitles"].as_bool().unwrap_or(false);
    let take: Option<crate::cinema::VideoTake> = serde_json::from_value(r["take"].clone()).ok();
    let label = crate::jobs::set_progress(&config_dir, &job.id, 0, 1, "重启后接着收 ComfyUI 的结果");
    crate::jobs::emit(app.as_ref(), label.as_ref());

    // 开机对账（`comfy_jobs::reconcile_on_startup`）可能已经把文件放回原位了 —— 那就只差记 take
    let fetched = if out.is_file() {
        Ok(())
    } else {
        // 在这条任务名下等：ComfyUI 的进度（第几步、还要多久）照样推到任务卡片上
        crate::jobs::scoped(&config_dir, &job.id, crate::agent_runtime::video_tool::resume_shot_video(&prompt_id, &out, generate_audio)).await
    };
    let result = fetched.map_err(crate::error::Error::runtime).and_then(|()| {
        let take = take.ok_or_else(|| crate::error::Error::runtime("续接单里的 take 读不出来"))?;
        crate::cinema::record_resumed_take(&config_dir, &job.project_id, &job.episode_id, &job.shot_id, take, burn)
    });
    let done = match &result {
        Ok(_) => crate::jobs::finish_with(&config_dir, &job.id, Some(&out.to_string_lossy()), Some("重启后从 ComfyUI 收回"), &[], None),
        Err(e) => crate::jobs::finish(&config_dir, &job.id, None, Some(&format!("重启后接着收 ComfyUI 的结果，没收到：{e}"))),
    };
    crate::jobs::emit(app.as_ref(), done.as_ref());
    if let Err(e) = result {
        if should_requeue(&e.to_string()) {
            match crate::cinema::queue_render(app.clone(), config_dir.clone(), job.project_id.clone(), job.episode_id.clone(), vec![job.shot_id.clone()]) {
                Ok(b) => crate::jobs::emit(app.as_ref(), Some(&b)),
                Err(e) => tracing::warn!(error = %e, shot = %job.shot_id, "续接失败后重新排队没排上"),
            }
        }
    }
}

/// **画布的出片**：去 ComfyUI 收回来、按画布任务的结果格式（和 `studio::GenerateOutput` 同一个形状）落回任务 ——
/// 节点重新加载后按任务 id 取回，和没重启过一样。ComfyUI 那边没有了就判失败（画布不自动重排：节点上点一下就重出）。
async fn resume_canvas(app: Option<tauri::AppHandle>, config_dir: PathBuf, job: Job) {
    let r = job.resume.clone().unwrap_or_default();
    let out = PathBuf::from(r["out"].as_str().unwrap_or_default());
    let prompt_id = r["prompt_ids"].as_array().and_then(|a| a.last()).and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let generate_audio = r["generate_audio"].as_bool().unwrap_or(true);
    let label = crate::jobs::set_progress(&config_dir, &job.id, 0, 1, "重启后接着收 ComfyUI 的结果");
    crate::jobs::emit(app.as_ref(), label.as_ref());
    // 开机对账（`comfy_jobs::reconcile_on_startup`）可能已经把文件放回原位了
    let fetched = if out.is_file() {
        Ok(())
    } else {
        // 在这条任务名下等（进度照样推到节点上）；取消也认 —— 节点上点取消靠 `comfy_jobs::JOB_ID`
        crate::jobs::scoped(
            &config_dir,
            &job.id,
            crate::agent_runtime::comfy_jobs::JOB_ID.scope(job.id.clone(), async {
                crate::agent_runtime::comfy_jobs::attach(&prompt_id);
                crate::agent_runtime::video_tool::resume_shot_video(&prompt_id, &out, generate_audio).await
            }),
        )
        .await
    };
    crate::agent_runtime::comfy_jobs::forget(&job.id);
    let done = match fetched {
        Ok(()) => crate::jobs::finish_canvas(&config_dir, &job.id, Some(canvas_result(&out, r["summary"].as_str().unwrap_or_default())), None),
        Err(e) => crate::jobs::finish_canvas(&config_dir, &job.id, None, Some(&format!("重启后接着收 ComfyUI 的结果，没收到：{e}"))),
    };
    crate::jobs::emit(app.as_ref(), done.as_ref());
}

/// 画布任务的结果（节点按 `studio::GenerateOutput` 的形状读）
fn canvas_result(out: &Path, summary: &str) -> serde_json::Value {
    let p = out.to_string_lossy().to_string();
    let message = if summary.is_empty() { format!("应用重启后从 ComfyUI 收回。\n文件：{p}") } else { format!("{summary}\n文件：{p}") };
    serde_json::to_value(crate::studio::GenerateOutput { path: Some(p.clone()), paths: vec![p], message, notes: vec!["应用重启过，这条是从 ComfyUI 收回来的".into()], ok: true }).unwrap_or_default()
}

/// 界面那边的 http 客户端和后台模型网关（出首帧、补合成要用）。启动时 `AppState` 还没登记好就等一会儿
async fn app_services(app: &tauri::AppHandle) -> Option<(reqwest::Client, std::sync::Arc<crate::Gateway>)> {
    use tauri::Manager;
    for _ in 0..60 {
        if let Some(state) = app.try_state::<crate::AppState>() {
            return Some((state.http.clone(), state.gateway.clone()));
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    tracing::warn!("等不到应用状态，重启后接着出首帧没排上");
    None
}

/// 去 ComfyUI 收一张图（重启前交出去的）：出完了下载回来；还在排队 / 在跑就等；没了（连着两次查不到）回错。
async fn fetch_image(prompt_id: &str) -> Result<Vec<u8>, String> {
    const BASE: &str = "http://127.0.0.1:8188";
    let client = reqwest::Client::builder().no_proxy().timeout(std::time::Duration::from_secs(30)).build().unwrap_or_default();
    let started = std::time::Instant::now();
    let mut unreachable_since: Option<std::time::Instant> = None;
    let mut lost = 0;
    loop {
        if started.elapsed() > std::time::Duration::from_secs(6 * 3600) {
            return Err(format!("等了 6 小时 ComfyUI 还没出这张图（prompt_id `{prompt_id}`）—— 去 ComfyUI 看一眼队列"));
        }
        let get = |url: String| {
            let client = client.clone();
            async move { client.get(url).send().await.ok()?.json::<serde_json::Value>().await.ok() }
        };
        let (Some(history), Some(queue)) = (get(format!("{BASE}/history/{prompt_id}")).await, get(format!("{BASE}/queue")).await) else {
            let since = *unreachable_since.get_or_insert_with(std::time::Instant::now);
            if since.elapsed() > std::time::Duration::from_secs(30 * 60) {
                return Err("连不上 ComfyUI 已经 30 分钟（重启过的话这张图也没了）".into());
            }
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            continue;
        };
        unreachable_since = None;
        if let Some(entry) = history.get(prompt_id) {
            if entry.pointer("/status/status_str").and_then(|v| v.as_str()) == Some("error") {
                return Err(format!("ComfyUI 执行出错（prompt_id `{prompt_id}`），去 ComfyUI 看日志"));
            }
            let Some((file, sub)) = output_image(entry) else {
                return Err(format!("ComfyUI 跑完了但没有产出图片（prompt_id `{prompt_id}`）"));
            };
            let url = format!("{BASE}/view?filename={}&subfolder={}&type=output", urlencoding::encode(&file), urlencoding::encode(&sub));
            let bytes = client.get(url).send().await.and_then(|r| r.error_for_status()).map_err(|e| format!("下载失败：{e}"))?;
            return bytes.bytes().await.map(|b| b.to_vec()).map_err(|e| format!("下载失败：{e}"));
        }
        if crate::agent_runtime::comfy_jobs::queue_contains(&queue, prompt_id) {
            lost = 0;
        } else {
            lost += 1;
            if lost >= 2 {
                return Err(format!("ComfyUI 那边已经没有这个任务了（prompt_id `{prompt_id}`）—— 多半是 ComfyUI 重启过"));
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    }
}

/// history 一条记录里第一张真正的产出图（`type: output`；预览节点是 `temp`、载入的是 `input`，都不算）
fn output_image(entry: &serde_json::Value) -> Option<(String, String)> {
    let outputs = entry.get("outputs")?.as_object()?;
    let mut nodes: Vec<(&String, &serde_json::Value)> = outputs.iter().collect();
    nodes.sort_by_key(|(k, _)| k.parse::<u64>().unwrap_or(u64::MAX));
    nodes.into_iter().find_map(|(_, v)| {
        v.get("images")?.as_array()?.iter().find_map(|img| {
            if img.get("type").and_then(|t| t.as_str()).unwrap_or("output") != "output" {
                return None;
            }
            let file = img.get("filename")?.as_str()?.to_string();
            Some((file, img.get("subfolder").and_then(|s| s.as_str()).unwrap_or_default().to_string()))
        })
    })
}

/// 续接一张首帧：收图 → 裁成项目画幅、挂到这一镜（和正常出完同一个函数）。
/// `own`：不属于还要接着出的「按场出首帧」—— 那就自己收尾：成功了按首帧补合成，丢了自己重排一条。
async fn resume_frame(app: Option<tauri::AppHandle>, config_dir: PathBuf, job: Job, own: bool) {
    let r = job.resume.clone().unwrap_or_default();
    let prompt_id = r["prompt_ids"].as_array().and_then(|a| a.last()).and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let want = (r["width"].as_u64().unwrap_or(0) as u32, r["height"].as_u64().unwrap_or(0) as u32);
    let started = crate::jobs::set_progress(&config_dir, &job.id, 0, 1, "重启后接着收 ComfyUI 的首帧");
    crate::jobs::emit(app.as_ref(), started.as_ref());
    let ep_file = crate::cinema::cinema_projects_dir(&config_dir).join(&job.project_id).join("episodes").join(&job.episode_id).join("episode.json");
    let shots_dir = ep_file.with_file_name("shots");
    let result = match fetch_image(&prompt_id).await {
        Ok(bytes) => crate::cinema::install_first_frame(&ep_file, &shots_dir, &job.shot_id, &bytes, want, &[]).map_err(|e| e.to_string()),
        Err(e) => Err(e),
    };
    let done = match &result {
        Ok(r) => crate::jobs::finish_with(&config_dir, &job.id, Some(&r.image_url), Some("重启后从 ComfyUI 收回"), &[], None),
        Err(e) => crate::jobs::finish(&config_dir, &job.id, None, Some(&format!("重启后接着收 ComfyUI 的首帧，没收到：{e}"))),
    };
    crate::jobs::emit(app.as_ref(), done.as_ref());
    if !own {
        return;
    }
    let Some(app) = app else { return };
    let Some((http, gateway)) = app_services(&app).await else { return };
    match result {
        // 和按场出首帧跑完一样：有了首帧，出片线路的提示词按首帧合成一次
        Ok(_) => {
            let targets = crate::cinema::frame_resynth_targets_pub(&config_dir, &job.project_id, &job.episode_id, std::slice::from_ref(&job.shot_id));
            if !targets.is_empty() {
                if let Err(e) = crate::cinema::start_synthesize(Some(app.clone()), config_dir.clone(), gateway, job.project_id.clone(), job.episode_id.clone(), targets, None) {
                    tracing::warn!(error = %e, "续接收回首帧后补合成没开始");
                }
            }
        }
        Err(e) if should_requeue(&e) => {
            match crate::cinema::start_frame_batch(app.clone(), config_dir.clone(), http, gateway, job.project_id.clone(), job.episode_id.clone(), vec![job.shot_id.clone()], None, Vec::new(), true) {
                Ok(b) => crate::jobs::emit(Some(&app), Some(&b)),
                Err(e) => tracing::warn!(error = %e, "续接首帧失败后重新出没排上"),
            }
        }
        Err(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{JobProgress, JobStatus};

    fn job(id: &str, kind: JobKind, shot: &str) -> Job {
        serde_json::from_value(serde_json::json!({
            "id": id, "kind": kind, "status": "running", "project_id": "p", "episode_id": "e",
            "shot_id": shot, "shot_num": 0, "prompt": "", "created_at": 0
        }))
        .unwrap()
    }

    fn with_resume(mut j: Job, prompt_ids: &[&str]) -> Job {
        j.resume = Some(serde_json::json!({"out": "/x/s2-b.mp4", "prompt_ids": prompt_ids, "take": {"route": "b", "path": "/x/s2-b.mp4"}}));
        j
    }

    #[test]
    fn 续接得上的才续接_批量剩下的接着排() {
        let mut batch = job("b1", JobKind::VideoBatch, "");
        batch.targets = ["s1", "s2", "s3", "s4"].iter().map(|s| s.to_string()).collect();
        batch.progress = Some(JobProgress { done: 1, total: 4, label: "第 2 镜".into(), eta_at: None });
        let v_ok = with_resume(job("v2", JobKind::Video, "s2"), &["pid-2"]);
        let p = plan(&[batch.clone(), v_ok.clone()]);
        assert_eq!(p.videos.iter().map(|j| j.id.as_str()).collect::<Vec<_>>(), ["v2"]);
        assert_eq!(p.batches[0].1, ["s3", "s4"], "s1 出过了；s2 正在出、续接收尾，不再排");

        // 还没交给 ComfyUI 就关了（没有 prompt_id）：续接不上，那一镜要重新排
        let v_none = with_resume(job("v2", JobKind::Video, "s2"), &[]);
        let p = plan(&[batch.clone(), v_none]);
        assert!(p.videos.is_empty());
        assert_eq!(p.batches[0].1, ["s2", "s3", "s4"]);

        // 老任务没有续接单：同上
        let p = plan(&[batch.clone(), job("v2", JobKind::Video, "s2")]);
        assert!(p.videos.is_empty() && p.batches[0].1.len() == 3);

        // 还在「排队中」（一镜都没开始）：全部要排
        batch.progress = Some(JobProgress { done: 0, total: 4, label: "排队中 · 等前一镜出完".into(), eta_at: None });
        assert_eq!(plan(&[batch]).batches[0].1.len(), 4);
    }

    /// 出首帧同一套：正在出的那张续接，按场出首帧剩下的接着排
    #[test]
    fn 出首帧也续接() {
        let mut batch = job("fb", JobKind::FrameBatch, "");
        batch.targets = ["s1", "s2", "s3"].iter().map(|s| s.to_string()).collect();
        batch.progress = Some(JobProgress { done: 1, total: 3, label: "第 2 镜".into(), eta_at: None });
        let mut frame = job("f2", JobKind::Frame, "s2");
        frame.resume = Some(serde_json::json!({"kind": "frame", "shot_id": "s2", "width": 1344, "height": 768, "prompt_ids": ["pid"]}));
        let p = plan(&[batch.clone(), frame.clone()]);
        assert_eq!(p.videos.iter().map(|j| j.id.as_str()).collect::<Vec<_>>(), ["f2"]);
        assert_eq!(p.batches[0].1, ["s3"]);
        // 出片的续接不算到出首帧的批量上
        let mut video = with_resume(job("v2", JobKind::Video, "s2"), &["pid"]);
        video.shot_id = "s2".into();
        assert_eq!(plan(&[batch.clone(), video]).batches[0].1, ["s2", "s3"]);
        // 没拿到 prompt_id 的首帧：续接不上
        frame.resume = Some(serde_json::json!({"kind": "frame", "shot_id": "s2", "prompt_ids": []}));
        assert!(plan(&[batch, frame]).videos.is_empty());
    }

    /// 画布出片也续接；没拿到 prompt_id、或者不是出片（图片处理之类）的画布任务照旧判失败
    #[test]
    fn 画布出片也续接() {
        let mut c = job("c1", JobKind::Canvas, "node-1");
        c.resume = Some(serde_json::json!({"kind": "canvas_video", "out": "/w/.anima/generated/video_1.mp4", "prompt_ids": ["pid"]}));
        assert_eq!(plan(std::slice::from_ref(&c)).videos.len(), 1);
        c.resume = Some(serde_json::json!({"kind": "canvas_video", "out": "/w/v.mp4", "prompt_ids": []}));
        assert!(plan(std::slice::from_ref(&c)).videos.is_empty(), "还没交给 ComfyUI");
        assert!(plan(&[job("c2", JobKind::Canvas, "node-2")]).videos.is_empty(), "没有续接单");
        // 结果是节点认得的形状
        let v = canvas_result(Path::new("/w/v.mp4"), "已用 H3 生成");
        assert_eq!(v["path"], "/w/v.mp4");
        assert_eq!(v["paths"][0], "/w/v.mp4");
        assert_eq!(v["ok"], true);
        // 画布三条出片路都记续接单（10-11 窗口表长镜头漏了，星澜 30 秒跑到一半才发现重启接不回来）
        let prod = |src: &'static str| &src[..src.find("#[cfg(test)]").unwrap_or(src.len())];
        for (name, src) in [
            ("单条出片 run_weetodd", prod(include_str!("agent_runtime/video_tool.rs"))),
            ("超长视频", prod(include_str!("long_video.rs"))),
            // canvas_ops.rs 中间夹着测试模块，只看长镜头那个函数体
            ("窗口表长镜头", {
                let src = include_str!("canvas_ops.rs");
                let i = src.find("pub async fn run_canvas_long_take(").unwrap();
                &src[i..i + src[i..].find("\n}\n").unwrap()]
            }),
        ] {
            assert!(src.contains("\"kind\": \"canvas_video\""), "{name} 没记续接单 —— 重启后接不回来");
        }
    }

    #[test]
    fn 从_history_里认出产出图() {
        let entry = serde_json::json!({"outputs": {
            "12": {"images": [{"filename": "prev.png", "subfolder": "", "type": "temp"}]},
            "9": {"images": [{"filename": "anima_img_00039_.png", "subfolder": "", "type": "output"}]},
            "3": {"images": [{"filename": "ref.png", "subfolder": "", "type": "input"}]}
        }});
        assert_eq!(output_image(&entry), Some(("anima_img_00039_.png".into(), String::new())));
        assert_eq!(output_image(&serde_json::json!({"outputs": {}})), None);
    }

    #[test]
    fn 什么时候重新排队() {
        assert!(should_requeue("ComfyUI 那边已经没有这个任务了（prompt_id `x`）"));
        assert!(should_requeue("连不上 ComfyUI: error"));
        assert!(!should_requeue("ComfyUI 执行出错：CUDA out of memory"), "ComfyUI 自己报错的，重排还是错");
    }

    /// 启动时：续接的留着在跑，续接不上的判失败，批量标「中断」
    #[test]
    fn 启动时续接的不判失败() {
        let d = tempfile::tempdir().unwrap();
        let ok = crate::jobs::start(d.path(), JobKind::Video, "p", "e", "s2", 2, "");
        let lost = crate::jobs::start(d.path(), JobKind::Video, "p", "e", "s5", 5, "");
        // 给 ok 写续接单（正常是出片时 note_resume / note_prompt_id 写的）
        crate::jobs::set_resume_for_test(d.path(), &ok, serde_json::json!({"out": "/x/s2-b.mp4", "prompt_ids": ["pid"], "take": {"route": "b", "path": "/x/s2-b.mp4"}}));
        let running = crate::jobs::all_running(d.path());
        let p = plan(&running);
        let keep: Vec<String> = p.videos.iter().map(|j| j.id.clone()).collect();
        assert_eq!(keep, std::slice::from_ref(&ok));
        crate::jobs::reap_stale_except(d.path(), &keep);
        assert_eq!(crate::jobs::get(d.path(), &ok).unwrap().status, JobStatus::Running);
        assert_eq!(crate::jobs::get(d.path(), &lost).unwrap().status, JobStatus::Failed);
    }

    /// 出片时真的把 prompt_id 记进了续接单（在任务 scope 里提交）
    #[tokio::test]
    async fn 出片任务里提交时记下_prompt_id() {
        let d = tempfile::tempdir().unwrap();
        let id = crate::jobs::start(d.path(), JobKind::Video, "p", "e", "s2", 2, "");
        crate::jobs::scoped(d.path(), &id, async {
            crate::jobs::note_resume(serde_json::json!({"out": "/x/a.mp4", "take": {}}));
            crate::jobs::note_prompt_id("pid-1");
        })
        .await;
        crate::jobs::note_prompt_id("不在任务里的不记");
        // 开跑时刻只记第一次；续接时读得回来（「已跑」不因重启清零，2026-10-06）
        crate::jobs::scoped(d.path(), &id, async {
            assert_eq!(crate::jobs::current_run_started_at(), None);
            crate::jobs::note_run_started(1000);
            crate::jobs::note_run_started(2000);
            assert_eq!(crate::jobs::current_run_started_at(), Some(1000));
        })
        .await;
        assert_eq!(crate::jobs::current_run_started_at(), None, "不在任务里读不到");
        let r = crate::jobs::get(d.path(), &id).unwrap().resume.unwrap();
        assert_eq!(r["prompt_ids"], serde_json::json!(["pid-1"]));
        assert!(resumable(&crate::jobs::get(d.path(), &id).unwrap()));
        // 源码：两处提交点和出片都接上了
        let vt = include_str!("agent_runtime/video_tool.rs");
        assert!(vt.contains("crate::jobs::note_prompt_id(&prompt_id);"));
        assert!(vt.contains("let mut run_since = crate::jobs::current_run_started_at();"));
        assert!(vt.contains("crate::jobs::note_run_started(t);"));
        let cinema = include_str!("cinema.rs");
        let prod = &cinema[..cinema.find("#[cfg(test)]").unwrap()];
        assert_eq!(prod.matches("note_resume(pictures, &picture_paths);").count(), 4, "四条出片路（超长镜 / 首帧 / 参考 / LTX）都要记续接单");
        // 出首帧：记续接单、单镜和按场出首帧都在任务 scope 里、出图提交点记 prompt_id
        assert!(prod.contains("crate::jobs::note_resume(json!({\"kind\": \"frame\""));
        assert!(prod.contains("crate::jobs::scoped(&state.config_dir, &job, cinema_render_shot_with_deps("));
        assert!(prod.contains("crate::jobs::scoped(&config_dir, &one, cinema_render_shot_with_deps("));
        assert!(include_str!("agent_runtime/image_tool.rs").contains("crate::jobs::note_prompt_id(prompt_id);"));
        assert!(prod.contains("crate::jobs::scoped(config_dir, &job, async {"));
        let lib = include_str!("lib.rs");
        assert!(lib.contains("render_resume::start(") && !lib.contains("let stale = jobs::reap_stale(&config_dir);"));
    }
}
