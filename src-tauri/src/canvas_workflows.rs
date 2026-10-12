//! 工作流目录：把画布上的一套接法存成可复用的文件。
//!
//! # 为什么存文件而不是 localStorage
//!
//! 画布本身存在 localStorage（`anima:canvas:project:<id>`），那是「这一集
//! 当前这张布」，刷新能恢复就够了。工作流不一样 —— 它是用户要**长期攒**的
//! 东西：localStorage 有 5MB 上限、清一次缓存就全没了、也拷不到别的机器上。
//!
//! # 按工程分目录，但不锁死在工程里
//!
//! 用户 2026-09-21：「不同工程下会混入其他工程的任务」。确实该分。
//!
//! 但**硬过滤会把这个功能的意义毁掉** —— 模板存在的全部理由就是
//! 「换个项目、换套素材还能跑」（模板里本来就不带集/镜绑定和参考图路径）。
//! 所以这里按工程分目录，界面默认只列当前工程的，另给一处能翻到别的工程 ——
//! 默认不混，要用得着的时候够得到。
//!
//! # 这一层只管存取，不懂内容
//!
//! 「哪些字段不能进模板」（产物、集/镜绑定、参考图的绝对路径）由前端的
//! `workflowTemplate.ts` 决定 —— 那些字段的含义只有前端知道。
//! 这里收到什么存什么，**但要拦住越界的文件名**。

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{Error, Result};

fn root(config_dir: &Path) -> PathBuf {
    config_dir.join("anima").join("canvas").join("workflows")
}

/// 某个工程的工作流放哪。`project_id` 为空 = 没挂工程（画布还没选工程时存的）。
fn dir(config_dir: &Path, project_id: &str) -> Result<PathBuf> {
    if project_id.is_empty() {
        return Ok(root(config_dir).join("_未归属"));
    }
    // **工程 id 也要过一遍同样的检查。** 它来自前端，同样能带 `../`
    Ok(root(config_dir).join(safe_name(project_id)?))
}

/// 文件名安全检查。
///
/// **中文要留着** —— 工作流是给人看的，`wf_1758...` 谁也认不出来。
/// 但路径分隔符、`..`、隐藏文件前缀必须拦住，否则这个接口就成了
/// 「往任意路径写任意内容」。
fn safe_name(name: &str) -> Result<String> {
    let n = name.trim();
    if n.is_empty() {
        return Err(Error::config("工作流得有个名字"));
    }
    if n.len() > 200 {
        return Err(Error::config("工作流名字太长"));
    }
    if n.contains('/') || n.contains('\\') || n.contains('\0') || n.starts_with('.') || n.contains("..") {
        return Err(Error::config(format!("工作流名字里不能有路径分隔符或 `..`：{n}")));
    }
    Ok(n.to_string())
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkflowEntry {
    pub name: String,
    /// 属于哪个工程。空 = 没挂工程
    pub project_id: String,
    /// 毫秒。列表按它倒序
    pub saved_at: u64,
    pub bytes: u64,
}

pub fn save(config_dir: &Path, project_id: &str, name: &str, json: &str) -> Result<()> {
    let name = safe_name(name)?;
    // **先确认它是合法 JSON。** 存进去一个坏文件，下次载入才发现，
    // 而那时候用户已经把画布清掉了。
    let v: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| Error::config(format!("工作流不是合法 JSON：{e}")))?;
    if v.get("nodes").and_then(|n| n.as_array()).map_or(true, |a| a.is_empty()) {
        return Err(Error::config("这套工作流一个节点都没有"));
    }
    let d = dir(config_dir, project_id)?;
    std::fs::create_dir_all(&d).map_err(|e| Error::config(format!("建不出工作流目录：{e}")))?;
    std::fs::write(d.join(format!("{name}.json")), json)
        .map_err(|e| Error::config(format!("保存工作流失败：{e}")))
}

/// **所有**工程的工作流，各自带着 `project_id`。
///
/// 不在这里按工程筛：筛了的话界面就没法给出「别的工程里还有 N 套」这句话，
/// 而那正是模板可复用的地方。筛选留给界面。
pub fn list(config_dir: &Path) -> Vec<WorkflowEntry> {
    let Ok(dirs) = std::fs::read_dir(root(config_dir)) else { return Vec::new() };
    let mut out: Vec<WorkflowEntry> = Vec::new();
    for d in dirs.flatten().filter(|e| e.path().is_dir()) {
        let owner = d.file_name().to_string_lossy().to_string();
        let project_id = if owner == "_未归属" { String::new() } else { owner };
        let Ok(files) = std::fs::read_dir(d.path()) else { continue };
        out.extend(files.flatten().filter_map(|e| {
            let p = e.path();
            if p.extension()? != "json" {
                return None;
            }
            let m = e.metadata().ok()?;
            Some(WorkflowEntry {
                name: p.file_stem()?.to_string_lossy().to_string(),
                project_id: project_id.clone(),
                saved_at: m.modified().ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as u64).unwrap_or(0),
                bytes: m.len(),
            })
        }));
    }
    out.sort_by_key(|w| std::cmp::Reverse(w.saved_at));
    out
}

pub fn load(config_dir: &Path, project_id: &str, name: &str) -> Result<String> {
    let name = safe_name(name)?;
    std::fs::read_to_string(dir(config_dir, project_id)?.join(format!("{name}.json")))
        .map_err(|e| Error::config(format!("读不出工作流「{name}」：{e}")))
}

pub fn delete(config_dir: &Path, project_id: &str, name: &str) -> Result<()> {
    let name = safe_name(name)?;
    let p = dir(config_dir, project_id)?.join(format!("{name}.json"));
    if !p.is_file() {
        return Err(Error::config(format!("没有这套工作流：{name}")));
    }
    std::fs::remove_file(&p).map_err(|e| Error::config(format!("删不掉：{e}")))
}

pub fn rename(config_dir: &Path, project_id: &str, from: &str, to: &str) -> Result<()> {
    let (from, to) = (safe_name(from)?, safe_name(to)?);
    let d = dir(config_dir, project_id)?;
    let src = d.join(format!("{from}.json"));
    let dst = d.join(format!("{to}.json"));
    if !src.is_file() {
        return Err(Error::config(format!("没有这套工作流：{from}")));
    }
    // **不静默覆盖同名的。** 改个名把别人的工作流冲掉是不可逆的。
    if dst.exists() && from != to {
        return Err(Error::config(format!("已经有一套叫「{to}」的了")));
    }
    std::fs::rename(&src, &dst).map_err(|e| Error::config(format!("改名失败：{e}")))
}

// ── Tauri 命令 ────────────────────────────────────────────────────

use tauri::State;

use crate::AppState;

#[tauri::command]
pub fn canvas_workflow_save(state: State<'_, AppState>, project_id: String, name: String, json: String) -> Result<()> {
    save(&state.config_dir, &project_id, &name, &json)
}

#[tauri::command]
pub fn canvas_workflow_list(state: State<'_, AppState>) -> Vec<WorkflowEntry> {
    list(&state.config_dir)
}

#[tauri::command]
pub fn canvas_workflow_load(state: State<'_, AppState>, project_id: String, name: String) -> Result<String> {
    load(&state.config_dir, &project_id, &name)
}

#[tauri::command]
pub fn canvas_workflow_delete(state: State<'_, AppState>, project_id: String, name: String) -> Result<()> {
    delete(&state.config_dir, &project_id, &name)
}

#[tauri::command]
pub fn canvas_workflow_rename(state: State<'_, AppState>, project_id: String, from: String, to: String) -> Result<()> {
    rename(&state.config_dir, &project_id, &from, &to)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn 一份() -> String {
        serde_json::json!({"version":1,"name":"x","nodes":[{"id":"a"}],"edges":[],"slots":[]}).to_string()
    }

    #[test]
    fn 存了能列能读能删() {
        let d = tempfile::tempdir().unwrap();
        save(d.path(), "drama_1", "首帧→视频→回填", &一份()).unwrap();
        let l = list(d.path());
        assert_eq!(l.len(), 1);
        // **中文名要原样留着** —— 工作流是给人看的
        assert_eq!(l[0].name, "首帧→视频→回填");
        assert!(load(d.path(), "drama_1", "首帧→视频→回填").unwrap().contains("\"nodes\""));
        delete(d.path(), "drama_1", "首帧→视频→回填").unwrap();
        assert!(list(d.path()).is_empty());
        // 删不存在的要报错，不是静默成功
        assert!(delete(d.path(), "drama_1", "首帧→视频→回填").is_err());
    }

    /// **按工程分开存，但列表给全部。**
    ///
    /// 用户 2026-09-21：「不同工程下会混入其他工程的任务」。
    /// 分是要分的 —— 但筛选留给界面：后端筛了的话，界面就没法说
    /// 「别的工程里还有 N 套」，而模板存在的全部理由就是换个项目还能跑。
    #[test]
    fn 按工程分开存_列表带着归属() {
        let d = tempfile::tempdir().unwrap();
        save(d.path(), "drama_1", "同名的", &一份()).unwrap();
        save(d.path(), "drama_2", "同名的", &一份()).unwrap();
        save(d.path(), "", "没挂工程的", &一份()).unwrap();

        // **同名不互相覆盖** —— 不分目录的话第二次 save 会把第一个冲掉
        let l = list(d.path());
        assert_eq!(l.len(), 3, "同名的被覆盖了：{l:?}");
        let mut owned: Vec<(String, String)> =
            l.iter().map(|w| (w.project_id.clone(), w.name.clone())).collect();
        owned.sort();
        assert_eq!(owned, vec![
            (String::new(), "没挂工程的".into()),
            ("drama_1".into(), "同名的".into()),
            ("drama_2".into(), "同名的".into()),
        ]);

        // 读也按工程走，取不到别人的那一份
        assert!(load(d.path(), "drama_1", "同名的").is_ok());
        assert!(load(d.path(), "drama_3", "同名的").is_err());
        // 删只删自己工程的
        delete(d.path(), "drama_1", "同名的").unwrap();
        assert_eq!(list(d.path()).len(), 2);
        assert!(load(d.path(), "drama_2", "同名的").is_ok(), "删错了别的工程的");
    }

    /// **工程 id 也来自前端，同样能带 `../`。**
    /// 只检查工作流名字的话，`project_id` 就成了那条没人看的路。
    #[test]
    fn 工程id也不许带路径() {
        let d = tempfile::tempdir().unwrap();
        for bad in ["../跑出去", "a/b", "a\\b", ".隐藏", ".."] {
            assert!(save(d.path(), bad, "x", &一份()).is_err(), "放过了工程 id：{bad}");
            assert!(load(d.path(), bad, "x").is_err(), "放过了工程 id：{bad}");
            assert!(delete(d.path(), bad, "x").is_err(), "放过了工程 id：{bad}");
        }
        // 空工程 id 是合法的（画布还没选工程）—— 落在 `_未归属`
        assert!(save(d.path(), "", "x", &一份()).is_ok());
    }

    /// **名字里带路径就是往任意位置写文件。** 这个接口收的是用户输入。
    #[test]
    fn 名字里不许有路径分隔符或上跳() {
        let d = tempfile::tempdir().unwrap();
        for bad in ["../跑出去", "a/b", "a\\b", ".隐藏", "..", ""] {
            assert!(save(d.path(), "drama_1", bad, &一份()).is_err(), "放过了：{bad}");
            assert!(load(d.path(), "drama_1", bad).is_err(), "放过了：{bad}");
            assert!(delete(d.path(), "drama_1", bad).is_err(), "放过了：{bad}");
        }
    }

    /// 坏 JSON 存进去，要等到下次载入才发现 —— 那时候画布可能已经清掉了。
    #[test]
    fn 坏内容当场拦下不落盘() {
        let d = tempfile::tempdir().unwrap();
        assert!(save(d.path(), "drama_1", "坏的", "{不是 json").is_err());
        assert!(save(d.path(), "drama_1", "空的", r#"{"nodes":[]}"#).is_err());
        assert!(list(d.path()).is_empty(), "坏内容落盘了");
    }

    #[test]
    fn 改名不静默覆盖同名的() {
        let d = tempfile::tempdir().unwrap();
        save(d.path(), "drama_1", "甲", &一份()).unwrap();
        save(d.path(), "drama_1", "乙", &一份()).unwrap();
        assert!(rename(d.path(), "drama_1", "甲", "乙").is_err(), "把「乙」冲掉了");
        rename(d.path(), "drama_1", "甲", "丙").unwrap();
        assert_eq!(list(d.path()).len(), 2);
        assert!(load(d.path(), "drama_1", "丙").is_ok());
    }

    #[test]
    fn 列表按时间倒序_没有目录时给空() {
        let d = tempfile::tempdir().unwrap();
        assert!(list(d.path()).is_empty(), "没有目录时该给空表而不是崩");
        save(d.path(), "drama_1", "老的", &一份()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        save(d.path(), "drama_1", "新的", &一份()).unwrap();
        assert_eq!(list(d.path())[0].name, "新的");
    }
}
