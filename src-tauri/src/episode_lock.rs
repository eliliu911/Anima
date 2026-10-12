//! 分集文件（`episode.json`）的读-改-写锁。**所有写分集文件的地方都走这里。**
//!
//! # 为什么需要
//!
//! 这个文件的每一次写都是「整份读出来 → 改一处 → 整份写回去」。两个写入者交叠时 ——
//! 后台出片跑完回写这一镜的片子，同时导演世界在存这一镜的场面；配音回写和整集保存；
//! 批量合成提示词和用户改镜头 —— 后写的那个拿的是它读的那一刻的内容，
//! **把先写的那个的改动整个盖掉，而且不报错**。
//!
//! 以前各处靠「落盘前重读」把窗口缩小，但重读和写之间仍然没有互斥；
//! 还有几处（拼整集、配音、静帧运镜）写回的是几分钟前读的那一份。
//!
//! 这里给每个分集文件一把锁，**读、改、写三步都在锁里**。写是临时文件 + rename：
//! 不加锁的读者（界面刷新、出片前的只读检查）读到的要么是旧的完整内容，
//! 要么是新的完整内容，不会是一份半截的 JSON。
//!
//! # 用法约束
//!
//! - 闭包是同步的，锁里 `.await` 不了 —— 这是故意的。出图、出片、HTTP、TTS
//!   这种慢活在锁外做完，锁里只做「按磁盘上最新的那份改这几个字段」。
//! - 闭包没改动任何东西就不写盘：mtime 不动，也不会顺手把磁盘内容规范化一遍
//!   （比如 `drop_old_storyboard` 只在内存里清旧分镜，没真改就不该落盘）。
//! - 只管同一个进程里的写入者。手改文件、外部脚本不在这把锁里。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use crate::cinema::EpisodeData;
use crate::error::{Error, Result};

/// 按文件路径改一集：锁住，读 → `f` 改 → 有改动就原子写回。文件不在或解析不了直接报错，不调 `f`。
pub fn update_file<T>(file: &Path, f: impl FnOnce(&mut EpisodeData) -> Result<T>) -> Result<T> {
    edit(file, f)
}

/// [`update_file`]，按项目 / 分集 id 找文件。
pub fn update<T>(config_dir: &Path, project_id: &str, episode_id: &str, f: impl FnOnce(&mut EpisodeData) -> Result<T>) -> Result<T> {
    update_file(&crate::cinema::episode_file(config_dir, project_id, episode_id), f)
}

/// 同 [`update_file`]，但按原始 JSON 改 —— 给只认几个键、不经 `EpisodeData` 往返的调用方
/// （`EpisodeData` 认不出的键在这条路上原样保留）。和 [`update_file`] 是**同一把锁**。
pub fn update_json<T>(file: &Path, f: impl FnOnce(&mut Value) -> Result<T>) -> Result<T> {
    edit(file, f)
}

/// 整集保存：锁住，把磁盘上那份（不在或坏了是 `None`）交给 `f`，写 `f` 回的那份。
///
/// 给「界面送来一整份」的保存用 —— 它要和磁盘上那份合并（颜色表、导演场面），
/// 还要判断界面那份是不是比磁盘旧，这些判断必须和写在同一把锁里。
pub fn save_over(file: &Path, f: impl FnOnce(Option<EpisodeData>) -> Result<EpisodeData>) -> Result<()> {
    let lock = lock_for(file);
    let _held = lock.lock().unwrap_or_else(|p| p.into_inner());
    let disk = std::fs::read_to_string(file).ok().and_then(|t| serde_json::from_str(&t).ok());
    let doc = f(disk)?;
    write_atomic(file, to_json(&doc)?.as_bytes())
}

/// 整份写一个新分集（新建项目、加集）。也在锁里、也是原子写；目录不在就建。
pub fn put(file: &Path, doc: &impl Serialize) -> Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| Error::runtime(format!("建分集目录失败：{e}")))?;
    }
    let lock = lock_for(file);
    let _held = lock.lock().unwrap_or_else(|p| p.into_inner());
    write_atomic(file, to_json(doc)?.as_bytes())
}

fn edit<D: Serialize + DeserializeOwned, T>(file: &Path, f: impl FnOnce(&mut D) -> Result<T>) -> Result<T> {
    let lock = lock_for(file);
    // 锁里护着的是 `()`，文件本身靠原子写保证完整 —— 上一个持锁的 panic 了也照常往下走
    let _held = lock.lock().unwrap_or_else(|p| p.into_inner());
    let text = std::fs::read_to_string(file).map_err(|e| Error::config(format!("读不到分集：{e}")))?;
    let mut doc: D = serde_json::from_str(&text).map_err(|e| Error::config(format!("分集文件坏了：{e}")))?;
    let before = to_json(&doc)?;
    let out = f(&mut doc)?;
    let after = to_json(&doc)?;
    if after != before {
        write_atomic(file, after.as_bytes())?;
    }
    Ok(out)
}

fn to_json(doc: &impl Serialize) -> Result<String> {
    serde_json::to_string_pretty(doc).map_err(|e| Error::runtime(format!("序列化分集失败：{e}")))
}

/// 每个分集文件一把锁。
///
/// 键用**规范化过的目录** + 文件名：同一个文件有人拼成 `cinema_projects_dir(..)/p/episodes/e/episode.json`，
/// 有人从 `read_dir` 的结果拼，macOS 的临时目录还有 `/var` → `/private/var` 的符号链接 ——
/// 键不一样就是两把锁，等于没锁。
fn lock_for(file: &Path) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
    let key = match (file.parent().and_then(|d| d.canonicalize().ok()), file.file_name()) {
        (Some(dir), Some(name)) => dir.join(name),
        _ => file.to_path_buf(),
    };
    let mut locks = LOCKS.get_or_init(Default::default).lock().unwrap_or_else(|p| p.into_inner());
    locks.entry(key).or_default().clone()
}

/// 先写同目录的临时文件再 rename。临时文件名每次不同 —— 应用外的进程（`mut.sh` 的副本、
/// 测试）不在这把锁里，共用一个固定的 `.tmp` 名字会互相截断。
fn write_atomic(file: &Path, bytes: &[u8]) -> Result<()> {
    let name = file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let tmp = file.with_file_name(format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple()));
    std::fs::write(&tmp, bytes).map_err(|e| Error::runtime(format!("保存分集失败：{e}")))?;
    std::fs::rename(&tmp, file).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        Error::runtime(format!("保存分集失败：{e}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn seed(dir: &Path) {
        let ep_dir = crate::cinema::cinema_projects_dir(dir).join("p1/episodes/e1");
        std::fs::create_dir_all(&ep_dir).unwrap();
        let shot = json!({"id":"s1","shot_num":1,"scale":"","camera_movement":"","dialogue":"","prompt":"","duration":5.0,"status":"pending"});
        std::fs::write(
            ep_dir.join("episode.json"),
            json!({"id":"e1","episode_num":1,"title":"t","synopsis":"","script_text":"","storyboard_version":crate::cinema::STORYBOARD_VERSION,"storyboard":[shot]}).to_string(),
        )
        .unwrap();
    }

    /// 三个写入者交叉改同一集的不同字段，各几百次：**一次都不能丢**。
    ///
    /// - 「出片回写」：`update` 往 `long_take_scenes` 里追加
    /// - 「导演世界存场面」：真的 `set_director_scene`，每次加一个新键
    /// - 「按原始 JSON 改」的那条路（系列资产合并那类）：`update_json` 往剧本后面追加一个字
    ///
    /// 同时一个读者不加锁地一直读：原子写保证它永远读到完整的 JSON。
    #[test]
    fn concurrent_writers_never_lose_an_update() {
        const N: usize = 150;
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().to_path_buf();
        seed(&cfg);
        let file = crate::cinema::episode_file(&cfg, "p1", "e1");
        let done = std::sync::atomic::AtomicBool::new(false);

        std::thread::scope(|s| {
            let reader = s.spawn(|| {
                let mut reads = 0;
                while !done.load(std::sync::atomic::Ordering::Relaxed) {
                    crate::cinema::read_episode(&cfg, "p1", "e1").expect("读到了半截的分集文件");
                    reads += 1;
                }
                reads
            });
            let writers = [
                s.spawn(|| {
                    for i in 0..N {
                        update(&cfg, "p1", "e1", |ep| {
                            ep.long_take_scenes.push(i as u32);
                            Ok(())
                        })
                        .unwrap();
                    }
                }),
                s.spawn(|| {
                    for i in 0..N {
                        crate::studio_director::set_director_scene(&cfg, "p1", "e1", "s1", json!({ format!("k{i}"): i })).unwrap();
                    }
                }),
                s.spawn(|| {
                    for _ in 0..N {
                        update_json(&file, |v| {
                            v["script_text"] = Value::String(format!("{}字", v["script_text"].as_str().unwrap_or_default()));
                            Ok(())
                        })
                        .unwrap();
                    }
                }),
            ];
            for w in writers {
                w.join().unwrap();
            }
            done.store(true, std::sync::atomic::Ordering::Relaxed);
            assert!(reader.join().unwrap() > 0);
        });

        let ep = crate::cinema::read_episode(&cfg, "p1", "e1").unwrap();
        assert_eq!(ep.long_take_scenes, (0..N as u32).collect::<Vec<_>>(), "出片回写丢了");
        let scene = ep.storyboard[0].director_scene.as_ref().and_then(|v| v.as_object()).cloned().unwrap_or_default();
        assert_eq!(scene.len(), N, "导演世界的场面丢了键");
        assert_eq!(ep.script_text.chars().count(), N, "按原始 JSON 改的那条路丢了");
        // 临时文件都换上去了，没留下来
        let left: Vec<_> = std::fs::read_dir(file.parent().unwrap()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert_eq!(left, vec![std::ffi::OsString::from("episode.json")], "留下了临时文件");
    }

    /// 钉子：写分集文件的地方不许再绕开这把锁直接 `fs::write`。
    ///
    /// 认两种写法（调用本身 + 下一行）：路径一看就是分集文件，或者写的内容是一个分集变量
    /// （`&ep` / `&latest` / `&updated_ep` / `&ep1` —— 路径变量叫 `file`、`f` 时只能靠这个认）。
    /// 拿改之前的代码实测：23 处直接写认出 21 处。漏的两处是 `merge_looks` / `series_assets`，
    /// 写的是叫 `value` / `v` 的原始 JSON，认不出来 —— 那两个文件改的时候要自己记着。
    #[test]
    fn nobody_writes_episode_json_directly() {
        let path_hints = ["ep_file", "episode_file(", "episode.json"];
        let body_hints = ["(&ep)", ", &ep)", "(&latest)", "(&updated_ep)", "(&ep1)", "(&episode)"];
        let mut hits = Vec::new();
        for (name, src) in [
            ("cinema.rs", include_str!("cinema.rs")),
            ("studio_director.rs", include_str!("studio_director.rs")),
            ("studio_ops.rs", include_str!("studio_ops.rs")),
            ("merge_looks.rs", include_str!("merge_looks.rs")),
            ("series_assets.rs", include_str!("series_assets.rs")),
            ("cinema_dub.rs", include_str!("cinema_dub.rs")),
            ("agent_runtime/cinema_provider.rs", include_str!("agent_runtime/cinema_provider.rs")),
        ] {
            let prod = src.split("#[cfg(test)]").next().unwrap();
            let lines: Vec<&str> = prod.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                let Some(at) = line.find("fs::write(").or_else(|| line.find("write_json(")) else { continue };
                let call = format!("{} {}", &line[at..], lines.get(i + 1).unwrap_or(&""));
                if path_hints.iter().chain(&body_hints).any(|k| call.contains(k)) {
                    hits.push(format!("{name}:{} {}", i + 1, line.trim()));
                }
            }
        }
        assert!(hits.is_empty(), "直接写了分集文件 —— 改走 crate::episode_lock：\n{}", hits.join("\n"));
    }

    /// 同一个文件换一种拼法（多一段 `..`、经过符号链接）也得是同一把锁。
    #[test]
    fn same_file_spelled_differently_shares_one_lock() {
        let dir = tempfile::tempdir().unwrap();
        seed(dir.path());
        let file = crate::cinema::episode_file(dir.path(), "p1", "e1");
        let ep_dir = file.parent().unwrap();
        let detour = ep_dir.join("../e1/episode.json");
        assert!(Arc::ptr_eq(&lock_for(&file), &lock_for(&detour)));
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(ep_dir, &link).unwrap();
        assert!(Arc::ptr_eq(&lock_for(&file), &lock_for(&link.join("episode.json"))));
    }

    /// 闭包没改东西就不写（mtime 不动、磁盘内容不被规范化）；闭包报错也不写。
    #[test]
    fn no_change_or_error_leaves_the_file_alone() {
        let dir = tempfile::tempdir().unwrap();
        seed(dir.path());
        let file = crate::cinema::episode_file(dir.path(), "p1", "e1");
        let raw = std::fs::read_to_string(&file).unwrap();
        update_file(&file, |ep| Ok(ep.title.len())).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), raw, "没改也写了一遍");
        let err = update_file(&file, |ep| -> Result<()> {
            ep.title = "改了一半".into();
            Err(Error::config("不干了"))
        });
        assert!(err.is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), raw, "闭包报错了还写盘");
        assert!(update(dir.path(), "p1", "nope", |_| Ok(())).is_err(), "分集不在要报错，不能凭空建一个");
    }

    /// 整集保存拿得到磁盘上那份（没有就是 None），写的是闭包回的那份；闭包报错不写。
    #[test]
    fn save_over_sees_disk_and_writes_result() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("e1/episode.json");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let blank: EpisodeData = serde_json::from_value(json!({"id":"e1","episode_num":1,"title":"一","synopsis":"","script_text":"","storyboard":[]})).unwrap();
        save_over(&file, |disk| {
            assert!(disk.is_none());
            Ok(blank.clone())
        })
        .unwrap();
        assert!(save_over(&file, |_| Err(Error::config("旧的"))).is_err());
        save_over(&file, |disk| {
            let mut ep = disk.expect("磁盘上有了");
            assert_eq!(ep.title, "一");
            ep.title = "二".into();
            Ok(ep)
        })
        .unwrap();
        let on_disk: EpisodeData = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(on_disk.title, "二");
    }
}
