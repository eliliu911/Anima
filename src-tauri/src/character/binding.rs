//! 每个角色自己的绑定：用哪个引擎、哪个音色、哪张脸、挂哪些技能。
//!
//! ## 为什么不是新开一个配置文件
//!
//! `characters/<id>/` 下面本来就有 `voice.json` 和 `avatar.json`，而且用户
//! 手写过内容（yae 的 `emotion_voices` 里还带着实测数据的注释）。新开一份
//! "权威配置"的结果是**两份都在、谁也不知道哪份生效** —— 那正是这个项目
//! 反复踩的那类坑。所以就读写这两份，加一份 `skills.json` 放技能。
//!
//! 写回时**保留不认识的键**（`_measured`、`_note`、`hologram_color`……）。
//! 界面上改一个音色就把用户手写的实测记录冲掉，是不可接受的。
//!
//! ## `voice` 和 `voice_id`
//!
//! 现存文件里两种键名都有（`ad_xuejie` 用 `voice_id`，另外两个用 `voice`）。
//! **读的时候两个都认**，写的时候统一成 `voice` 并删掉 `voice_id` ——
//! 留着两个键的后果是改了一个另一个还在，而生效的是哪个全靠读取顺序。
//!
//! ## 模型（LLM）不在这里
//!
//! 角色不绑大模型：换角色连带换模型会让首字延迟和工具可靠性一起跳变，
//! 而那两件事跟"她是谁"无关。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// 一个角色绑定了什么。全是 `Option` —— 没绑就跟随全局设置
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    /// TTS 引擎 id（`config.tts_engines` 里的那个 id）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    /// 音色。含义随引擎变：Edge 是 `zh-CN-XiaoxiaoNeural`，
    /// GPT-SoVITS 是音色库里的路径 `yaemiko/d`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,
    /// 面容模型 id（`avatars/` 下的文件名，或内置头的 id）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    /// 挂载的技能 id。空数组 = 不挂技能（**和"没绑"是两件事**：
    /// 没绑跟随全局，空数组是明确要求一个都不挂）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<String>>,
}

fn dir_of(root: &Path, id: &str) -> PathBuf {
    root.join(id)
}

fn read_json(path: &Path) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn str_at(v: &Option<serde_json::Value>, keys: &[&str]) -> Option<String> {
    let obj = v.as_ref()?.as_object()?;
    for k in keys {
        if let Some(s) = obj.get(*k).and_then(|x| x.as_str()) {
            let s = s.trim();
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

/// 读一个角色现在绑了什么
pub fn read(root: &Path, id: &str) -> Binding {
    let dir = dir_of(root, id);
    let voice = read_json(&dir.join("voice.json"));
    let avatar = read_json(&dir.join("avatar.json"));
    let skills = read_json(&dir.join("skills.json"));

    Binding {
        engine: str_at(&voice, &["engine"]),
        // 两种键名都认 —— 见模块注释
        voice: str_at(&voice, &["voice", "voice_id"]),
        avatar: str_at(&avatar, &["model_id", "model", "avatar"]),
        skills: skills
            .as_ref()
            .and_then(|v| v.get("skills"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str())
                    .map(str::to_string)
                    .collect()
            }),
    }
}

/// 把一个键写进 JSON 文件，**保留文件里其它所有内容**。
///
/// 文件不存在就新建一个只有这个键的对象。值为 `None` 表示删掉这个键
/// （界面上"跟随全局"就是删掉，而不是写个空字符串 —— 空字符串会被
/// 当成"绑了一个叫空的东西"）
fn patch(path: &Path, edits: &[(&str, Option<serde_json::Value>)]) -> Result<()> {
    let mut doc = read_json(path).unwrap_or_else(|| serde_json::json!({}));
    // 早期几个角色的 `skills.json` 是 `[]`（hinata / rice / wanko / ziling）—— 空的就当没写，
    // 否则界面上一改绑定就报「不是一个 JSON 对象」。有内容的数组照旧拒，不吞掉别人的东西
    if doc.is_null() || doc.as_array().is_some_and(|a| a.is_empty()) {
        doc = serde_json::json!({});
    }
    let obj = doc
        .as_object_mut()
        .ok_or_else(|| Error::config(format!("{} 不是一个 JSON 对象", path.display())))?;
    for (key, value) in edits {
        match value {
            Some(v) => {
                obj.insert((*key).to_string(), v.clone());
            }
            None => {
                obj.remove(*key);
            }
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::Io(format!("建目录失败：{e}")))?;
    }
    let text =
        serde_json::to_string_pretty(&doc).map_err(|e| Error::Io(format!("序列化失败：{e}")))?;
    std::fs::write(path, text).map_err(|e| Error::Io(format!("写入失败：{e}")))?;
    Ok(())
}

fn as_value(s: &Option<String>) -> Option<serde_json::Value> {
    s.as_ref()
        .map(|x| x.trim())
        .filter(|x| !x.is_empty())
        .map(|x| serde_json::Value::String(x.to_string()))
}

/// 写回绑定。**只改这四项，文件里其它内容原样保留**
pub fn write(root: &Path, id: &str, binding: &Binding) -> Result<()> {
    let dir = dir_of(root, id);
    if !dir.is_dir() {
        return Err(Error::config(format!("找不到角色「{id}」的目录")));
    }

    patch(
        &dir.join("voice.json"),
        &[
            ("engine", as_value(&binding.engine)),
            ("voice", as_value(&binding.voice)),
            // 统一成一个键。两个键并存时"生效的是哪个"全靠读取顺序
            ("voice_id", None),
        ],
    )?;
    patch(
        &dir.join("avatar.json"),
        &[("model_id", as_value(&binding.avatar))],
    )?;
    patch(
        &dir.join("skills.json"),
        &[(
            "skills",
            binding.skills.as_ref().map(|s| {
                serde_json::Value::Array(
                    s.iter()
                        .map(|x| serde_json::Value::String(x.clone()))
                        .collect(),
                )
            }),
        )],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("anima-binding-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("yae")).unwrap();
        dir
    }

    #[test]
    fn reads_what_is_bound() {
        let root = scratch("read");
        std::fs::write(
            root.join("yae/voice.json"),
            r#"{"engine":"gpt_sovits","voice":"yaemiko/d"}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("yae/avatar.json"),
            r#"{"model_id":"Yae miko.glb"}"#,
        )
        .unwrap();
        std::fs::write(root.join("yae/skills.json"), r#"{"skills":["nurse"]}"#).unwrap();

        let b = read(&root, "yae");
        assert_eq!(b.engine.as_deref(), Some("gpt_sovits"));
        assert_eq!(b.voice.as_deref(), Some("yaemiko/d"));
        assert_eq!(b.avatar.as_deref(), Some("Yae miko.glb"));
        assert_eq!(b.skills.as_deref(), Some(&["nurse".to_string()][..]));
    }

    /// 现存文件里两种键名都有。**只认一个的话，另一半角色的音色读不出来**
    #[test]
    fn accepts_both_voice_and_voice_id() {
        let root = scratch("keys");
        std::fs::write(
            root.join("yae/voice.json"),
            r#"{"engine":"edge","voice_id":"zh-TW-HsiaoChenNeural"}"#,
        )
        .unwrap();
        assert_eq!(
            read(&root, "yae").voice.as_deref(),
            Some("zh-TW-HsiaoChenNeural")
        );
    }

    /// 没绑就是没绑，不是空字符串
    #[test]
    fn nothing_bound_reads_as_none() {
        let root = scratch("empty");
        let b = read(&root, "yae");
        assert_eq!(b, Binding::default());
    }

    /// **写回不能冲掉用户手写的内容。** yae 的 voice.json 里有实测数据的注释，
    /// 界面上改个音色就把它冲掉是不可接受的
    #[test]
    fn writing_preserves_everything_else() {
        let root = scratch("preserve");
        std::fs::write(
            root.join("yae/voice.json"),
            r#"{"engine":"gpt_sovits","voice":"yaemiko/d",
                "_measured":{"yaemiko/d":"基频 258Hz"},
                "emotion_voices":{"up":"yaemiko/greeting"}}"#,
        )
        .unwrap();

        write(
            &root,
            "yae",
            &Binding {
                voice: Some("yaemiko/greeting".into()),
                ..Default::default()
            },
        )
        .unwrap();

        let doc = read_json(&root.join("yae/voice.json")).unwrap();
        assert_eq!(doc["voice"], "yaemiko/greeting");
        assert!(doc["_measured"].is_object(), "实测记录被冲掉了");
        assert_eq!(doc["emotion_voices"]["up"], "yaemiko/greeting");
    }

    /// 早期角色的 `skills.json` 是 `[]`：界面改绑定原来报「不是一个 JSON 对象」（2026-10-04 外部审查指出）。
    /// 空数组当没写；有内容的数组照旧拒，不吞掉
    #[test]
    fn empty_array_skills_file_can_be_written() {
        let root = scratch("array");
        std::fs::write(root.join("yae/skills.json"), "[]").unwrap();
        assert_eq!(read(&root, "yae").skills, None);
        write(&root, "yae", &Binding { skills: Some(vec!["nurse".into()]), ..Default::default() }).unwrap();
        assert_eq!(read(&root, "yae").skills, Some(vec!["nurse".to_string()]));
        std::fs::write(root.join("yae/skills.json"), r#"["别人的东西"]"#).unwrap();
        assert!(write(&root, "yae", &Binding { skills: Some(vec!["nurse".into()]), ..Default::default() }).is_err());
    }

    /// 写完只留一个 `voice` 键 —— 两个并存时生效的是哪个全靠读取顺序
    #[test]
    fn writing_collapses_the_duplicate_key() {
        let root = scratch("collapse");
        std::fs::write(root.join("yae/voice.json"), r#"{"voice_id":"old"}"#).unwrap();
        write(
            &root,
            "yae",
            &Binding {
                voice: Some("new".into()),
                ..Default::default()
            },
        )
        .unwrap();

        let doc = read_json(&root.join("yae/voice.json")).unwrap();
        assert_eq!(doc["voice"], "new");
        assert!(doc.get("voice_id").is_none(), "重复的键还在");
    }

    /// 解绑（选"跟随全局"）要把键删掉，而不是写个空串 ——
    /// 空串会被当成"绑了一个叫空的东西"
    #[test]
    fn unbinding_removes_the_key() {
        let root = scratch("unbind");
        std::fs::write(
            root.join("yae/voice.json"),
            r#"{"engine":"edge","voice":"x"}"#,
        )
        .unwrap();
        write(&root, "yae", &Binding::default()).unwrap();

        let doc = read_json(&root.join("yae/voice.json")).unwrap();
        assert!(doc.get("engine").is_none());
        assert!(doc.get("voice").is_none());
        assert_eq!(read(&root, "yae"), Binding::default());
    }

    /// 空数组和"没绑"是两件事：前者是"一个技能都别挂"，后者是"跟随全局"
    #[test]
    fn an_empty_skill_list_is_not_the_same_as_unbound() {
        let root = scratch("skills");
        write(
            &root,
            "yae",
            &Binding {
                skills: Some(vec![]),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(read(&root, "yae").skills, Some(vec![]));

        write(&root, "yae", &Binding::default()).unwrap();
        assert_eq!(read(&root, "yae").skills, None);
    }

    #[test]
    fn writing_to_a_missing_character_is_an_error() {
        let root = scratch("missing");
        assert!(write(&root, "nobody", &Binding::default()).is_err());
    }
}
