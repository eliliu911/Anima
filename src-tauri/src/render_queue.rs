//! **批量出片的排队名单：看、调顺序、插暂停位。**
//!
//! 2026-10-10 用户：「我们的 Anima 里缺少列队插队功能……14/23 个任务的列队，但是完全没办法调整顺序」。
//!
//! 名单就是 `jobs.json` 里批量任务的 `targets`。已经出完的和正在出的那一镜（下标 ≤ `progress.done`）不能动；
//! 后面没轮到的可以重排。`cinema::render_batch` 每一镜开始前重读名单，前缀没变就认新顺序；
//! 应用重启后续接（`render_resume::plan`）本来就按 `targets` 的顺序排剩下的。
//!
//! # 接上一镜的不能拆开
//!
//! 一镜要接上一镜（`effective_continuity() != None`、不是首帧锁画面），而上一镜**也在没轮到的名单里** ——
//! 这一镜必须紧跟在上一镜后面出，不然接的是上一镜的旧片（或者根本还没有片）。界面上这种连着的几镜合成一张卡片一起拖。
//! 上一镜已经出完了（不在没轮到的名单里）的，接的就是那条片，可以随便挪。
//!
//! # 暂停位
//!
//! `hold:<标签>` 是名单里的一个占位：批量走到它就停下等（不交 ComfyUI 任何东西），直到它被撤掉（[`release_hold`]）。
//! 用途是给名单外的东西让出 GPU —— 比如插一条冒烟测试：插暂停位 → 等前一镜出完 → 单独提交测试 → 测完撤掉，批量接着出。

use crate::cinema::{Continuity, StoryboardShot};
use crate::jobs::{Job, JobKind, JobStatus};
use serde::Serialize;
use std::path::Path;

pub const HOLD_PREFIX: &str = "hold:";

pub fn is_hold(id: &str) -> bool {
    id.starts_with(HOLD_PREFIX)
}

/// 这一镜出片时要接上一镜的画面（和 `cinema::chain_blocker` 同一个判据）
fn needs_prev(shot: &StoryboardShot) -> bool {
    shot.effective_continuity() != Continuity::None && !shot.keyframes.drives_video()
}

/// 本集里紧挨在它前面的那一镜（按镜号）
fn prev_id<'a>(shots: &'a [StoryboardShot], shot: &StoryboardShot) -> Option<&'a str> {
    shots.iter().filter(|s| s.shot_num < shot.shot_num).max_by_key(|s| s.shot_num).map(|s| s.id.as_str())
}

/// 没轮到的名单按「必须连着出」分组：一镜要接上一镜、上一镜也在没轮到的名单里且紧挨在它前面 → 和上一镜同组。
pub fn groups(shots: &[StoryboardShot], pending: &[String]) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = Vec::new();
    for (i, id) in pending.iter().enumerate() {
        let chained = i > 0
            && shots.iter().find(|s| &s.id == id).is_some_and(|s| needs_prev(s) && prev_id(shots, s) == Some(pending[i - 1].as_str()));
        match out.last_mut() {
            Some(g) if chained => g.push(id.clone()),
            _ => out.push(vec![id.clone()]),
        }
    }
    out
}

/// 新顺序能不能用：要接上一镜、上一镜也在没轮到的名单里的，必须紧跟在上一镜后面。`Err` 是给人看的原因。
pub fn check_order(shots: &[StoryboardShot], pending: &[String]) -> Result<(), String> {
    let num = |id: &str| shots.iter().find(|s| s.id == id).map(|s| s.shot_num);
    for (i, id) in pending.iter().enumerate() {
        let Some(shot) = shots.iter().find(|s| &s.id == id) else { continue };
        if !needs_prev(shot) {
            continue;
        }
        let Some(prev) = prev_id(shots, shot) else { continue };
        if !pending.iter().any(|p| p == prev) {
            continue;
        }
        if i == 0 || pending[i - 1] != prev {
            return Err(format!(
                "第 {} 镜要接第 {} 镜的画面，第 {} 镜也还没出 —— 它必须紧跟在第 {} 镜后面",
                shot.shot_num,
                num(prev).unwrap_or_default(),
                num(prev).unwrap_or_default(),
                num(prev).unwrap_or_default()
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct QueueItem {
    pub id: String,
    /// 镜头的镜号；暂停位、长镜头整场是 `None`
    pub shot_num: Option<u32>,
    pub label: String,
    pub hold: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct QueueView {
    pub job_id: String,
    pub project_id: String,
    pub episode_id: String,
    /// 已经出完的个数
    pub done: usize,
    /// 正在出的那一个（「排队中」时为空）
    pub current: Option<QueueItem>,
    /// 没轮到的，按「必须连着出」分好组
    pub pending: Vec<Vec<QueueItem>>,
}

/// 名单里正在出的下标；还在「排队中」（一镜都没开始）是 `None`。和 `jobs::append_targets` 同一个判据
pub fn current_index(job: &Job) -> Option<usize> {
    job.progress.as_ref().filter(|p| !p.label.starts_with("排队中")).map(|p| p.done)
}

fn item(shots: &[StoryboardShot], id: &str) -> QueueItem {
    if let Some(label) = id.strip_prefix(HOLD_PREFIX) {
        return QueueItem { id: id.into(), shot_num: None, label: format!("暂停 · {label}"), hold: true };
    }
    if let Some(n) = id.strip_prefix("scene_") {
        return QueueItem { id: id.into(), shot_num: None, label: format!("第 {n} 场 · 长镜头"), hold: false };
    }
    match shots.iter().find(|s| s.id == id) {
        Some(s) => QueueItem { id: id.into(), shot_num: Some(s.shot_num), label: format!("第 {} 镜 · {} 秒", s.shot_num, s.duration), hold: false },
        None => QueueItem { id: id.into(), shot_num: None, label: "（已不在这一集）".into(), hold: false },
    }
}

fn load_shots(config_dir: &Path, project_id: &str, episode_id: &str) -> Vec<StoryboardShot> {
    crate::cinema::read_episode_synced(config_dir, project_id, episode_id).map(|e| e.storyboard).unwrap_or_default()
}

pub fn view(config_dir: &Path, job: &Job) -> QueueView {
    let shots = load_shots(config_dir, &job.project_id, &job.episode_id);
    let cur = current_index(job);
    let start = cur.map_or(0, |c| c + 1);
    let pending: Vec<String> = job.targets.iter().skip(start).cloned().collect();
    QueueView {
        job_id: job.id.clone(),
        project_id: job.project_id.clone(),
        episode_id: job.episode_id.clone(),
        done: cur.unwrap_or(0),
        current: cur.and_then(|c| job.targets.get(c)).map(|id| item(&shots, id)),
        pending: groups(&shots, &pending).into_iter().map(|g| g.iter().map(|id| item(&shots, id)).collect()).collect(),
    }
}

/// 这一集正在跑的出片批量
pub fn running_batch(config_dir: &Path, project_id: &str, episode_id: &str) -> Option<Job> {
    crate::jobs::all_running(config_dir)
        .into_iter()
        .find(|j| j.kind == JobKind::VideoBatch && j.project_id == project_id && j.episode_id == episode_id)
}

/// 换掉没轮到的那段名单（顺序可以变，也可以加暂停位）。先查「接上一镜的不能拆开」。
pub fn reorder(config_dir: &Path, job_id: &str, pending: &[String]) -> Result<Job, String> {
    let job = crate::jobs::get(config_dir, job_id).filter(|j| j.status == JobStatus::Running).ok_or("这个批量已经不在跑了")?;
    let shots = load_shots(config_dir, &job.project_id, &job.episode_id);
    check_order(&shots, pending)?;
    crate::jobs::replace_pending(config_dir, job_id, pending).ok_or_else(|| "这个批量已经不在跑了".to_string())
}

/// 在某个位置插一个暂停位。`after` = 插在这一项后面；`None` = 插在正在出的那一镜后面（下一个就停）
pub fn insert_hold(config_dir: &Path, job_id: &str, label: &str, after: Option<&str>) -> Result<(Job, String), String> {
    let job = crate::jobs::get(config_dir, job_id).filter(|j| j.status == JobStatus::Running).ok_or("这个批量已经不在跑了")?;
    let start = current_index(&job).map_or(0, |c| c + 1);
    let mut pending: Vec<String> = job.targets.iter().skip(start).cloned().collect();
    let hold = format!("{HOLD_PREFIX}{}", label.trim());
    let at = match after {
        None => 0,
        Some(a) => pending.iter().position(|p| p == a).map(|i| i + 1).ok_or_else(|| format!("没轮到的名单里没有 {a}"))?,
    };
    pending.insert(at, hold.clone());
    let shots = load_shots(config_dir, &job.project_id, &job.episode_id);
    check_order(&shots, &pending)?;
    let job = crate::jobs::replace_pending(config_dir, job_id, &pending).ok_or("这个批量已经不在跑了")?;
    Ok((job, hold))
}

/// 撤掉暂停位（正在等的那个也能撤：批量下一次看名单时发现它没了，接着出）
pub fn release_hold(config_dir: &Path, job_id: &str, hold: &str) -> Result<Job, String> {
    crate::jobs::remove_target(config_dir, job_id, hold).ok_or_else(|| format!("名单里没有 {hold}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shot(num: u32, cont: &str) -> StoryboardShot {
        serde_json::from_value(serde_json::json!({
            "id": format!("s{num}"), "shot_num": num, "scale": "", "camera_movement": "", "dialogue": "", "prompt": "",
            "duration": 5.0, "status": "pending", "continuity": cont, "continuity_manual": true
        }))
        .unwrap()
    }
    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// 16 接 15、15 也没出 → 一组；17 不接 → 单独一组；暂停位单独一组
    #[test]
    fn 接上一镜的和上一镜分在一组() {
        let shots = vec![shot(15, "none"), shot(16, "ref_video"), shot(17, "none")];
        assert_eq!(groups(&shots, &ids(&["s15", "s16", "hold:测试", "s17"])), vec![ids(&["s15", "s16"]), ids(&["hold:测试"]), ids(&["s17"])]);
        // 15 不在没轮到的名单里（已经出完）：16 接的是那条片，自己一组
        assert_eq!(groups(&shots, &ids(&["s16", "s17"])), vec![ids(&["s16"]), ids(&["s17"])]);
    }

    #[test]
    fn 把接上一镜的拆开_拒绝() {
        let shots = vec![shot(15, "none"), shot(16, "ref_video"), shot(17, "none")];
        // 整组一起挪：可以
        assert!(check_order(&shots, &ids(&["s17", "s15", "s16"])).is_ok());
        // 中间插东西：不行
        let e = check_order(&shots, &ids(&["s15", "hold:测试", "s16", "s17"])).unwrap_err();
        assert!(e.contains("第 16 镜要接第 15 镜"), "{e}");
        // 放到上一镜前面：不行
        assert!(check_order(&shots, &ids(&["s16", "s15", "s17"])).is_err());
        // 上一镜已经出完（不在名单里）：随便放
        assert!(check_order(&shots, &ids(&["s17", "s16"])).is_ok());
    }
}

// —— 界面用的命令 ——

fn err(e: String) -> crate::error::Error {
    crate::error::Error::runtime(e)
}

/// 任务队列里点开一个出片批量：看名单（已出 / 正在出 / 没轮到的分组）
#[tauri::command]
pub fn cinema_render_queue(state: tauri::State<'_, crate::AppState>, job_id: String) -> crate::error::Result<QueueView> {
    let job = crate::jobs::get(&state.config_dir, &job_id).ok_or_else(|| err("找不到这个任务".into()))?;
    Ok(view(&state.config_dir, &job))
}

/// 拖完之后存新顺序（没轮到的那段，平铺成一列）
#[tauri::command]
pub fn cinema_reorder_queue(state: tauri::State<'_, crate::AppState>, job_id: String, pending: Vec<String>) -> crate::error::Result<QueueView> {
    let job = reorder(&state.config_dir, &job_id, &pending).map_err(err)?;
    Ok(view(&state.config_dir, &job))
}

/// 撤掉暂停位
#[tauri::command]
pub fn cinema_release_hold(state: tauri::State<'_, crate::AppState>, job_id: String, hold: String) -> crate::error::Result<QueueView> {
    let job = release_hold(&state.config_dir, &job_id, &hold).map_err(err)?;
    Ok(view(&state.config_dir, &job))
}
