//! 3D 面容模型的存放与读取。
//!
//! ## 为什么模型字节要过 Rust
//!
//! 直接让前端 `fetch('file:///…/avatar.glb')` 也能跑，但那要给前端开
//! asset 协议和文件系统 scope —— 而这个项目刻意不给前端网络和文件权限
//! （见 `capabilities/`）。为一个只读的本地文件放开整类权限不划算。
//!
//! 所以前端拿到的是一段 `ArrayBuffer`，交给 `GLTFLoader.parse()`。
//!
//! ## 模型放哪
//!
//! 和音色库同一套约定（`tts::voices::voices_dir`）：优先 `ANIMA_AVATARS`
//! 环境变量，其次从工作目录往上找项目里的 `avatars/`，最后退到 app 配置目录。
//! 项目里的 `avatars/` 已 gitignore —— 模型文件几十 MB，不该进版本库。

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{Error, Result};

/// 一个可用的面容模型
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AvatarModel {
    /// 相对模型目录的路径，也是读取时的标识。可能带子目录，
    /// 例如 `alice/head.glb` 或 `live2d/Hiyori/Hiyori.model3.json`。分隔符统一用 `/`
    pub id: String,
    /// 显示名称
    pub name: String,
    /// 所在子目录（顶层的为空）。界面按它分组
    pub folder: String,
    pub bytes: u64,
    /// 模型格式类型：3d (glb/gltf/vrm) 或 live2d (model3.json/model.json)
    pub kind: String,
}

/// 往下找几层子目录。
///
/// 三层够用：例如 `live2d/Hiyori/Hiyori.model3.json` 或 `characters/ad_xuejie/`
const MAX_DEPTH: usize = 3;

/// 模型目录。与音色库同一套解析顺序，理由见模块注释。
pub fn models_dir(config_dir: &Path) -> PathBuf {
    if let Some(explicit) = std::env::var_os("ANIMA_AVATARS") {
        let path = PathBuf::from(explicit);
        if !path.as_os_str().is_empty() {
            return path;
        }
    }
    if let Some(found) = find_project_dir(config_dir) {
        return found;
    }
    config_dir.join("avatars")
}

/// 动作库目录（`.vrma`）。与模型目录同一套解析顺序。
///
/// 和面容分开放：动作是**跨模型通用**的（VRMA 绑的是 VRM 的标准骨骼名，
/// 不是某个模型的节点），一份动作所有角色都能用。混在 avatars/ 里会让
/// "这个动作属于哪个模型"变成一个不存在但看起来存在的问题。
pub fn animations_dir(config_dir: &Path) -> PathBuf {
    if let Some(explicit) = std::env::var_os("ANIMA_ANIMATIONS") {
        let path = PathBuf::from(explicit);
        if !path.as_os_str().is_empty() {
            return path;
        }
    }
    if let Some(found) = find_named_project_dir(config_dir, "animations") {
        return found;
    }
    config_dir.join("animations")
}

/// 从当前工作目录往上找某个目录名。只找几层 —— 再深就说明不在项目里跑。
fn find_named_project_dir(config_dir: &Path, name: &str) -> Option<PathBuf> {
    crate::app_root::find_plain(name, Some(config_dir))
}

/// 项目里的 `avatars/`（Live2D / 3D / 神经视频模型）。
fn find_project_dir(config_dir: &Path) -> Option<PathBuf> {
    crate::app_root::find_plain("avatars", Some(config_dir))
}

/// 列出 `root` 下的模型文件，**含子目录**。
///
/// 支持子目录是因为模型常常是一整个文件夹（`.gltf` + `.bin` + `textures/` 或 Live2D 资源包），
/// 而且按角色分目录才管得过来。
pub fn list_in(root: &Path) -> Vec<AvatarModel> {
    let mut out = Vec::new();
    walk(root, root, 0, &mut out);
    // 排序固定：不排的话 read_dir 的顺序随文件系统变，
    // 「默认选第一个」就会时而不同
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// 列出动作库里的 `.vrma`。
pub fn list_animations_in(root: &Path) -> Vec<AvatarModel> {
    let mut paths: Vec<PathBuf> = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Ok(inner) = std::fs::read_dir(&path) {
                paths.extend(inner.flatten().map(|e| e.path()));
            }
        } else {
            paths.push(path);
        }
    }
    let mut out: Vec<AvatarModel> = paths
        .into_iter()
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("vrma"))
        })
        .filter_map(|p| {
            let name = p.file_stem()?.to_str()?.to_string();
            let id = p.strip_prefix(root).ok()?.to_str()?.replace('\\', "/");
            Some(AvatarModel {
                id,
                name,
                folder: String::new(),
                bytes: p.metadata().map(|m| m.len()).unwrap_or(0),
                kind: "3d".to_string(),
            })
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// 动作库里可用的动作名。返回 `(动作名, 原文件名)`。
pub fn action_names_in(root: &Path, builtin: &[&str]) -> Vec<(String, String)> {
    list_animations_in(root)
        .into_iter()
        .filter_map(|a| {
            let normalized = a.name.to_lowercase().replace([' ', '-'], "_");
            if builtin.contains(&normalized.as_str()) {
                return None;
            }
            Some((normalized, a.name))
        })
        .collect()
}

fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<AvatarModel>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            // 贴图与动作子目录里没有模型入口文件，进去只是白扫
            let skip = name.starts_with('.')
                || name == "textures"
                || name == "motions"
                || name.ends_with(".2048")
                || name.ends_with(".4096")
                || name.ends_with(".1024");
            if !skip && depth < MAX_DEPTH {
                walk(root, &path, depth + 1, out);
            }
            continue;
        }
        if let Some(model) = describe(root, &path) {
            out.push(model);
        }
    }
}

fn describe(root: &Path, path: &Path) -> Option<AvatarModel> {
    let file_name = path.file_name()?.to_str()?.to_ascii_lowercase();
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    let (_is_3d, is_live2d, is_neural) =
        if file_name.ends_with(".model3.json") || file_name.ends_with(".model.json") {
            (false, true, false)
        } else if matches!(ext.as_str(), "glb" | "gltf" | "vrm") {
            (true, false, false)
        } else if matches!(
            ext.as_str(),
            "jpg" | "jpeg" | "png" | "webp" | "mp4" | "mov" | "webm"
        ) {
            (false, false, true)
        } else {
            return None;
        };

    let rel = path.strip_prefix(root).ok()?;
    let id = rel
        .components()
        .map(|c| c.as_os_str().to_str().unwrap_or_default())
        .collect::<Vec<_>>()
        .join("/");
    let folder = rel
        .parent()
        .map(|p| {
            p.components()
                .map(|c| c.as_os_str().to_str().unwrap_or_default())
                .collect::<Vec<_>>()
                .join("/")
        })
        .unwrap_or_default();

    let name = if is_live2d {
        if let Some(parent) = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
        {
            if parent != "live2d" && parent != "avatars" {
                parent.to_string()
            } else {
                path.file_stem()?
                    .to_str()?
                    .trim_end_matches(".model3")
                    .trim_end_matches(".model")
                    .to_string()
            }
        } else {
            path.file_stem()?
                .to_str()?
                .trim_end_matches(".model3")
                .trim_end_matches(".model")
                .to_string()
        }
    } else {
        path.file_stem()?.to_str()?.to_string()
    };

    let kind = if is_live2d {
        "live2d".to_string()
    } else if is_neural {
        "neural".to_string()
    } else {
        "3d".to_string()
    };

    Some(AvatarModel {
        id,
        name,
        folder,
        bytes: std::fs::metadata(path).ok()?.len(),
        kind,
    })
}

/// 读出模型目录下某个文件的字节。
///
/// `id` 是相对路径，可以带子目录（`alice/head.glb`），也用于读取 `.gltf`
/// 引用的 `.bin` 和贴图。
pub fn read_in(root: &Path, id: &str) -> Result<Vec<u8>> {
    let path = resolve(root, id)?;
    std::fs::read(&path).map_err(|e| Error::Io(format!("读取 {} 失败：{e}", path.display())))
}

/// 把前端传来的相对路径解析成真实路径，**并确认它没跑出模型目录**。
///
/// 这里是唯一把前端字符串变成文件路径的地方，所以路径穿越必须在这里挡死：
/// 直接拼的话 `../../../../etc/passwd` 就能读到库外的任何文件，
/// 而这个项目的前提正是「前端不持有文件权限」。
///
/// 挡两层：
///
/// 1. **先看字面**。任何 `..`、绝对路径、Windows 分隔符一律拒绝。
/// 2. **再看实际落点**。canonicalize 之后必须仍在 root 之下 ——
///    光看字面挡不住软链接：目录里放一个指向 `/etc` 的软链，
///    路径里一个 `..` 都没有，照样能读出去。
fn resolve(root: &Path, id: &str) -> Result<PathBuf> {
    let bad = id.is_empty()
        || id.starts_with('/')
        || id.contains('\\')
        || Path::new(id)
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)));
    if bad {
        return Err(Error::config(format!("模型路径不合法：{id}")));
    }

    let path = root.join(id);
    let (Ok(real), Ok(real_root)) = (path.canonicalize(), root.canonicalize()) else {
        // canonicalize 失败通常就是文件不存在。交给调用方去读并报真正的错，
        // 这里不把"不存在"伪装成"不合法"
        return Ok(path);
    };
    if !real.starts_with(&real_root) {
        return Err(Error::config(format!(
            "模型路径跑出了模型目录：{id}（软链接？）"
        )));
    }
    Ok(real)
}

#[cfg(test)]
mod animation_tests {
    use super::*;

    fn library() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for f in ["Greeting.vrma", "Peace sign.vrma", "wave.vrma", "notes.txt"] {
            std::fs::write(dir.path().join(f), b"x").unwrap();
        }
        std::fs::create_dir(dir.path().join("pack")).unwrap();
        std::fs::write(dir.path().join("pack/Spin.vrma"), b"x").unwrap();
        // 一整包 Mixamo 的 fbx —— 这条管线吃不了，不该出现在清单里
        std::fs::write(dir.path().join("pack/Waving.fbx"), b"x").unwrap();
        dir
    }

    #[test]
    fn only_vrma_is_listed_including_one_level_of_subdirs() {
        let dir = library();
        let ids: Vec<String> = list_animations_in(dir.path())
            .into_iter()
            .map(|a| a.id)
            .collect();
        assert!(ids.contains(&"Greeting.vrma".to_string()));
        // 子目录里的要带相对路径，否则读的时候找不到
        assert!(ids.contains(&"pack/Spin.vrma".to_string()), "实际：{ids:?}");
        assert!(!ids.iter().any(|i| i.ends_with(".fbx")), "fbx 不该进清单");
        assert!(!ids.iter().any(|i| i.ends_with(".txt")));
    }

    /// 动作名要和模型写的标签用同一套规则：小写 + 空格换下划线。
    /// 不一致的话 `Peace sign.vrma` 永远匹配不上 `<action:peace_sign>`
    #[test]
    fn names_are_normalised_for_the_prompt() {
        let dir = library();
        let names: Vec<String> = action_names_in(dir.path(), &["wave"])
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert!(names.contains(&"peace_sign".to_string()), "实际：{names:?}");
        assert!(names.contains(&"greeting".to_string()));
        assert!(names.contains(&"spin".to_string()));
        // 和内置手势同名的文件是"替换它"，不是新动作 —— 不能在清单里出现两次
        assert!(!names.contains(&"wave".to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("anima.glb"), b"GLB-BYTES").unwrap();
        std::fs::write(dir.path().join("alt.gltf"), b"{}").unwrap();
        std::fs::write(dir.path().join("vroid.vrm"), b"GLB-ISH").unwrap();
        std::fs::write(dir.path().join("readme.txt"), b"not a model").unwrap();
        // 一个角色一个目录：模型 + 它引用的 bin + 贴图
        std::fs::create_dir_all(dir.path().join("alice/textures")).unwrap();
        std::fs::write(dir.path().join("alice/head.gltf"), b"{}").unwrap();
        std::fs::write(dir.path().join("alice/head.bin"), b"BIN").unwrap();
        std::fs::write(dir.path().join("alice/textures/skin.png"), b"PNG").unwrap();
        dir
    }

    #[test]
    fn lists_models_including_subfolders() {
        let dir = fixture();
        let found = list_in(dir.path());
        assert_eq!(
            found.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["alice/head.gltf", "alt.gltf", "anima.glb", "vroid.vrm"],
            "glb/gltf/vrm 都要列出来，子目录里的也要，而且顺序固定"
        );
    }

    #[test]
    fn reports_the_folder_so_the_ui_can_group() {
        let dir = fixture();
        let found = list_in(dir.path());
        let alice = found.iter().find(|m| m.name == "head").unwrap();
        assert_eq!(alice.folder, "alice");
        let top = found.iter().find(|m| m.name == "anima").unwrap();
        assert_eq!(top.folder, "", "顶层的没有分组");
    }

    #[test]
    fn lists_live2d_models() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("live2d/Hiyori")).unwrap();
        std::fs::write(dir.path().join("live2d/Hiyori/Hiyori.model3.json"), b"{}").unwrap();
        let found = list_in(dir.path());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "live2d/Hiyori/Hiyori.model3.json");
        assert_eq!(found[0].name, "Hiyori");
        assert_eq!(found[0].kind, "live2d");
    }

    #[test]
    fn does_not_descend_into_texture_folders() {
        // 贴图目录里没有模型，进去只是白扫。而且贴图动辄几百个文件
        let dir = fixture();
        assert!(
            list_in(dir.path())
                .iter()
                .all(|m| !m.id.contains("textures")),
            "不该扫 textures/"
        );
    }

    #[test]
    fn missing_directory_is_empty_not_an_error() {
        // 用户还没放模型是正常状态，不该报错 —— 界面要能平静地说「还没有模型」
        assert!(list_in(Path::new("/nonexistent/anima/avatars")).is_empty());
    }

    #[test]
    fn reads_model_bytes() {
        let dir = fixture();
        assert_eq!(read_in(dir.path(), "anima.glb").unwrap(), b"GLB-BYTES");
    }

    #[test]
    fn reads_sibling_files_of_a_gltf() {
        // .gltf 会引用 .bin 和贴图。读不到它们的话模型会加载成
        // 一个没有贴图、甚至没有几何体的东西，而且不报错
        let dir = fixture();
        assert_eq!(read_in(dir.path(), "alice/head.bin").unwrap(), b"BIN");
        assert_eq!(
            read_in(dir.path(), "alice/textures/skin.png").unwrap(),
            b"PNG"
        );
    }

    /// 路径穿越必须被挡住。
    ///
    /// id 是前端传进来的字符串，直接拼进路径就等于把整个文件系统开给了前端 ——
    /// 而这个项目的前提正是「前端不持有文件权限」。
    #[test]
    fn rejects_path_traversal() {
        let dir = fixture();
        for evil in [
            "../secret",
            "../../etc/passwd",
            "..",
            "",
            "alice/../../etc/passwd",
            "/etc/passwd",
            "a\\..\\b",
        ] {
            assert!(read_in(dir.path(), evil).is_err(), "应当拒绝：{evil:?}");
        }
    }

    /// 软链接也要挡住。
    ///
    /// **只看字面挡不住这个**：路径里一个 `..` 都没有，但落点在库外。
    /// 所以还要 canonicalize 之后确认仍在 root 之下。
    #[cfg(unix)]
    #[test]
    fn rejects_symlink_escaping_the_library() {
        let dir = fixture();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.glb"), b"SECRET").unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("escape")).unwrap();

        let err = read_in(dir.path(), "escape/secret.glb").unwrap_err();
        assert!(
            format!("{err}").contains("跑出了模型目录"),
            "软链接必须被挡住，实际报的是：{err}"
        );
    }

    #[test]
    fn env_override_wins() {
        // 环境变量优先级最高 —— 换模型目录不用改代码
        let dir = tempfile::tempdir().unwrap();
        temp_env::with_var("ANIMA_AVATARS", Some(dir.path()), || {
            assert_eq!(models_dir(Path::new("/tmp/cfg")), dir.path());
        });
    }

    #[test]
    fn falls_back_to_config_dir() {
        // 空字符串等于没设 —— 否则 `ANIMA_AVATARS=` 会让它去读根目录
        temp_env::with_var("ANIMA_AVATARS", Some(""), || {
            let resolved = models_dir(Path::new("/tmp/cfg"));
            // 在项目里跑测试时会命中项目的 avatars/，两种都算对
            assert!(
                resolved == Path::new("/tmp/cfg/avatars") || resolved.ends_with("avatars"),
                "{resolved:?}"
            );
        });
    }
}
