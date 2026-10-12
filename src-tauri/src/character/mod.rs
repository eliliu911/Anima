//! 角色模块。加载和解析 `characters/` 目录下的多文件角色定义。
//! 角色 (Character) 与技能 (Skill) 完全解耦。角色决定性格、底线、语调等“灵魂”部分，
//! 而技能决定她能使用什么工具、掌握什么具体专业知识。

pub mod binding;
pub mod card;

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Character {
    pub id: String,
    pub name: String,
    pub marker: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding: Option<crate::character::binding::Binding>,
    #[serde(skip)]
    pub personality: Option<serde_yaml::Value>,
    #[serde(skip)]
    pub speaking_style: String,
    #[serde(skip)]
    pub boundaries: String,
    #[serde(skip)]
    pub emotion_rules: Option<serde_json::Value>,
    #[serde(skip)]
    pub voice_config: Option<serde_json::Value>,
    #[serde(skip)]
    pub avatar_config: Option<serde_json::Value>,
}

impl Character {
    /// 动态编译 System Prompt 的“角色内核”部分。
    /// 这部分会被拼接到最终的 System Prompt 中，而“技能”部分会作为另一块拼接到后面。
    pub fn build_identity_prompt(&self) -> String {
        let mut prompt = String::new();

        prompt.push_str(&format!("[Character Persona & Role]\n{}\n\n", self.description));

        if !self.speaking_style.is_empty() {
            prompt.push_str(&format!("[Speaking Style & Tone]\n{}\n\n", self.speaking_style));
        }

        if !self.boundaries.is_empty() {
            prompt.push_str(&format!("[Core Boundaries & Constraints]\n{}\n\n", self.boundaries));
        }

        if let Some(traits) = &self.personality {
            if let Some(map) = traits.as_mapping() {
                prompt.push_str("[Personality Traits (Self-Awareness Reference)]\n");
                for (k, v) in map {
                    if let (Some(ks), Some(vs)) = (k.as_str(), v.as_str()) {
                        prompt.push_str(&format!("- {}: {}\n", ks, vs));
                    }
                }
                prompt.push('\n');
            }
        }

        prompt
    }
}

pub fn characters_dir(config_dir: &Path) -> PathBuf {
    if let Some(explicit) = std::env::var_os("ANIMA_CHARACTERS") {
        let path = PathBuf::from(explicit);
        if !path.as_os_str().is_empty() {
            return path;
        }
    }
    if let Some(found) = find_project_characters(config_dir) {
        return found;
    }
    config_dir.join("characters")
}

fn find_project_characters(config_dir: &Path) -> Option<PathBuf> {
    // **判据不能只看目录名** —— 同名的空目录会误命中。要求里面至少有一个
    // 带 `SKILL.md` 的子目录，那才是真的角色库。
    crate::app_root::find_dir("characters", Some(config_dir), |c| {
        std::fs::read_dir(c)
            .map(|it| it.flatten().any(|e| e.path().join("SKILL.md").is_file()))
            .unwrap_or(false)
    })
}

fn split_frontmatter(raw: &str) -> Option<(&str, &str)> {
    let rest = raw.strip_prefix("---")?.trim_start_matches(['\r', '\n']);
    let end = rest.find("\n---")?;
    let meta = &rest[..end];
    let body = rest[end + 4..].trim_start_matches(['\r', '\n']);
    Some((meta, body))
}

fn parse_skill_md(raw: &str, id_hint: &str) -> Result<Character> {
    let (meta, _body) = split_frontmatter(raw).ok_or_else(|| {
        Error::config(format!(
            "角色「{id_hint}」缺少 YAML frontmatter（文件要以 --- 开头、再用 --- 收尾）"
        ))
    })?;
    let mut character: Character = serde_yaml::from_str(meta)
        .map_err(|e| Error::config(format!("角色「{id_hint}」的 frontmatter 解析失败：{e}")))?;

    if character.id.is_empty() {
        character.id = id_hint.to_string();
    }

    Ok(character)
}

fn read_optional_string(path: &Path) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn read_optional_yaml(path: &Path) -> Option<serde_yaml::Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_yaml::from_str(&raw).ok())
}

fn read_optional_json(path: &Path) -> Option<serde_json::Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
}

pub fn load(root: &Path, id: &str) -> Result<Character> {
    let dir = root.join(id);
    if !dir.is_dir() {
        // 自动创建与自愈角色目录：若角色目录被意外删除或尚未初始化，自动为其创建目录
        if let Err(e) = std::fs::create_dir_all(&dir) {
            return Err(Error::config(format!("无法创建角色「{id}」目录：{e}")));
        }
    }

    let skill_file = dir.join("SKILL.md");
    if !skill_file.is_file() {
        let display_name = match id {
            "yumiko" => "由美子",
            "xiaoya" => "小雅",
            "ayami" => "绚美",
            "hanyu" => "韩羽",
            "ziling" => "子苓",
            "seeu" => "小柔",
            _ => id,
        };
        let default_skill = format!(
            "---\nid: {id}\nname: {display_name}\nmarker: {display_name}\ndescription: 4D 动态数字人伴侣\n---\n\n你是「{display_name}」，拥有 4D 动态体态与专属声音的数字人陪伴伴侣。\n"
        );
        let _ = std::fs::write(&skill_file, default_skill);
        let _ = std::fs::write(
            dir.join("avatar.json"),
            serde_json::json!({
                "default_mode": "neural_stream",
                "model_id": format!("neural/{id}_idle.mp4")
            })
            .to_string(),
        );
        let _ = std::fs::write(
            dir.join("voice.json"),
            serde_json::json!({
                "engine": "edge",
                "voice": "zh-CN-XiaoxiaoNeural"
            })
            .to_string(),
        );
    }

    let raw = std::fs::read_to_string(&skill_file)
        .map_err(|e| Error::config(format!("读不了 {}：{e}", skill_file.display())))?;

    let mut character = parse_skill_md(&raw, id)?;

    // 加载附属文件
    character.personality = read_optional_yaml(&dir.join("personality.yaml"));
    character.speaking_style = read_optional_string(&dir.join("speaking_style.md"));
    character.boundaries = read_optional_string(&dir.join("boundaries.md"));
    character.emotion_rules = read_optional_json(&dir.join("emotion_rules.json"));
    character.voice_config = read_optional_json(&dir.join("voice.json"));
    character.avatar_config = read_optional_json(&dir.join("avatar.json"));
    character.binding = Some(crate::character::binding::read(root, id));

    Ok(character)
}

pub fn list_in(root: &Path) -> Vec<Character> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();

    let mut out = Vec::new();
    for dir in dirs {
        let id_hint = dir
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .to_string();

        match load(root, &id_hint) {
            Ok(char) => out.push(char),
            Err(err) => tracing::warn!(%err, character = %id_hint, "角色加载失败，跳过"),
        }
    }
    out
}

pub const BUILTIN_NAME: &str = "switch_character";

pub fn is_switch_call(name: &str) -> bool {
    name == BUILTIN_NAME
}

pub fn character_from_arguments(raw: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(raw).ok()?;
    Some(value.get("character")?.as_str()?.trim().to_string())
}

pub fn declaration(available: &[Character]) -> crate::llm::Tool {
    let list = available
        .iter()
        .map(|s| format!("{}（{}）", s.id, s.name))
        .collect::<Vec<_>>()
        .join("、");
    crate::llm::Tool {
        name: BUILTIN_NAME.to_string(),
        description: format!(
            "切换到另一个角色灵魂（性格/外貌/声音设定），或退出当前角色。当用户说\"换回默认角色\"、\
             \"让AD学姐来\"这类话的时候调用它。\
             可选角色：{list}。退出请传空字符串。"
        ),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "character": {
                    "type": "string",
                    "description": "要切换到的角色 id；空字符串表示退回默认无特定角色的状态"
                }
            },
            "required": ["character"]
        }),
    }
}

/// 新建一个角色形态。
///
/// **只建最小骨架**：`SKILL.md` 带 frontmatter，其余文件用户按需再加。
/// 建一堆空文件的话，用户分不清哪些是他填过的
/// YAML 单行标量转义。裸写的话，名字里一个冒号就能让整份 frontmatter 解析失败
fn yaml_scalar(text: &str) -> String {
    let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

pub fn create(root: &Path, id: &str, name: &str, description: &str) -> Result<Character> {
    let id = id.trim();
    // id 是目录名，必须能安全落到文件系统上
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(Error::config(
            "角色 id 只能用英文字母、数字、下划线和短横线（它就是目录名）".to_string(),
        ));
    }
    let dir = root.join(id);
    if dir.exists() {
        return Err(Error::config(format!("角色「{id}」已经存在了")));
    }
    std::fs::create_dir_all(&dir).map_err(|e| Error::Io(format!("建目录失败：{e}")))?;

    let name = if name.trim().is_empty() {
        id
    } else {
        name.trim()
    };
    // **名字要按 YAML 标量转义。** 导入进来的角色卡名字什么都可能有
    // （冒号、引号、`#`），不转义会让 frontmatter 解析失败 ——
    // 而那时角色是加载不出来的，属于不报错的失败
    let md = format!(
        "---\nid: {id}\nname: {}\nmarker: \"\"\ndescription: {}\n---\n\n\
         在这里写这个角色的设定。同目录下还可以放：\n\n\
         - `speaking_style.md` 说话风格\n\
         - `boundaries.md` 绝对底线\n\
         - `personality.yaml` 性格特质\n\
         - `voice.json` / `avatar.json` / `skills.json` 绑定（在界面里改更方便）\n",
        yaml_scalar(name),
        yaml_scalar(if description.trim().is_empty() {
            "（还没写设定）"
        } else {
            description.trim()
        })
    );
    std::fs::write(dir.join("SKILL.md"), md).map_err(|e| Error::Io(format!("写入失败：{e}")))?;
    load(root, id)
}

/// 改角色的名字和设定。**只动 frontmatter，正文原样保留** ——
/// 用户可能在正文里写了很多东西
pub fn update_meta(root: &Path, id: &str, name: &str, description: &str) -> Result<Character> {
    let path = root.join(id).join("SKILL.md");
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| Error::config(format!("读不了 {}：{e}", path.display())))?;
    let (meta, body) = split_frontmatter(&raw)
        .ok_or_else(|| Error::config(format!("角色「{id}」的 SKILL.md 缺少 frontmatter")))?;

    let mut doc: serde_yaml::Mapping = serde_yaml::from_str(meta)
        .map_err(|e| Error::config(format!("frontmatter 解析失败：{e}")))?;
    if !name.trim().is_empty() {
        doc.insert("name".into(), name.trim().into());
    }
    doc.insert("description".into(), description.trim().into());
    // id 跟着目录走，不允许从这里改 —— 改了会和目录名对不上
    doc.insert("id".into(), id.into());

    let meta_out =
        serde_yaml::to_string(&doc).map_err(|e| Error::Io(format!("序列化失败：{e}")))?;
    std::fs::write(&path, format!("---\n{meta_out}---\n\n{body}"))
        .map_err(|e| Error::Io(format!("写入失败：{e}")))?;
    load(root, id)
}

/// 删掉一个角色形态。整个目录一起删 —— **这是不可逆的**
pub fn delete(root: &Path, id: &str) -> Result<()> {
    let dir = root.join(id);
    if !dir.is_dir() {
        return Err(Error::config(format!("找不到角色「{id}」")));
    }
    // 防呆：只删 characters/ 下面直属的一层，不接受 `../` 这种
    if id.contains('/') || id.contains("..") {
        return Err(Error::config("非法的角色 id".to_string()));
    }
    std::fs::remove_dir_all(&dir).map_err(|e| Error::Io(format!("删除失败：{e}")))?;
    tracing::info!(character = %id, "已删除角色形态");
    Ok(())
}

#[cfg(test)]
mod crud_tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("anima-crud-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn creates_a_loadable_character() {
        let root = scratch("create");
        let c = create(&root, "nurse", "小护士", "医院场景的陪伴角色").unwrap();
        assert_eq!(c.id, "nurse");
        assert_eq!(c.name, "小护士");
        // 建完必须能被正常加载 —— 建出一个加载不了的骨架等于没建
        assert_eq!(load(&root, "nurse").unwrap().name, "小护士");
        assert_eq!(list_in(&root).len(), 1);
    }

    /// id 就是目录名。**不校验的话能写出 `../` 跳出目录**
    #[test]
    fn rejects_ids_that_are_not_safe_directory_names() {
        let root = scratch("badid");
        for bad in ["", "  ", "../evil", "a/b", "带中文", "with space"] {
            assert!(
                create(&root, bad, "x", "").is_err(),
                "接受了非法 id：{bad:?}"
            );
        }
    }

    #[test]
    fn refuses_to_overwrite_an_existing_character() {
        let root = scratch("dup");
        create(&root, "nurse", "小护士", "").unwrap();
        assert!(create(&root, "nurse", "另一个", "").is_err());
    }

    /// 名字空着就用 id，别建出一个没名字的角色
    #[test]
    fn a_blank_name_falls_back_to_the_id() {
        let root = scratch("noname");
        assert_eq!(create(&root, "nurse", "   ", "").unwrap().name, "nurse");
    }

    /// **改元信息不能动正文。** 用户可能在正文里写了很多设定
    #[test]
    fn updating_meta_keeps_the_body() {
        let root = scratch("update");
        create(&root, "nurse", "小护士", "旧设定").unwrap();
        let path = root.join("nurse/SKILL.md");
        let raw = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!("{raw}\n这是我手写的一大段设定，不能丢。\n")).unwrap();

        let c = update_meta(&root, "nurse", "大护士", "新设定").unwrap();
        assert_eq!(c.name, "大护士");
        assert_eq!(c.description, "新设定");
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("这是我手写的一大段设定"),
            "正文被冲掉了"
        );
    }

    /// id 跟着目录走，不允许从 frontmatter 改 —— 改了就和目录名对不上
    #[test]
    fn updating_meta_cannot_change_the_id() {
        let root = scratch("keepid");
        create(&root, "nurse", "小护士", "").unwrap();
        assert_eq!(
            update_meta(&root, "nurse", "改名了", "").unwrap().id,
            "nurse"
        );
    }

    #[test]
    fn deletes_the_whole_directory() {
        let root = scratch("delete");
        create(&root, "nurse", "小护士", "").unwrap();
        std::fs::write(root.join("nurse/voice.json"), r#"{"engine":"edge"}"#).unwrap();

        delete(&root, "nurse").unwrap();
        assert!(!root.join("nurse").exists());
        assert!(list_in(&root).is_empty());
        assert!(delete(&root, "nurse").is_err(), "删不存在的应该报错");
    }

    /// 防呆：`..` 之类不能穿出 characters/
    #[test]
    fn delete_refuses_path_traversal() {
        let root = scratch("traversal");
        assert!(delete(&root, "../..").is_err());
        assert!(delete(&root, "a/b").is_err());
    }

    /// 坏掉的角色不该让整个列表挂掉 —— 跳过它，其余照常
    #[test]
    fn a_broken_character_does_not_break_the_list() {
        let root = scratch("broken");
        create(&root, "good", "好的", "").unwrap();
        std::fs::create_dir_all(root.join("bad")).unwrap();
        std::fs::write(root.join("bad/SKILL.md"), "没有 frontmatter").unwrap();

        let list = list_in(&root);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "good");
    }
}
