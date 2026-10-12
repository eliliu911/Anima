//! 导入 Character Card（CCv2 / CCv3）。
//!
//! ## 为什么值得做
//!
//! 角色卡是 SillyTavern 那一套生态的事实标准，外面有海量现成角色。
//! 支持导入之后，用户不用手写 `SKILL.md` 就能拿到一个能用的形态 ——
//! 这是投入产出比最高的一条。
//!
//! ## 两种载体，PNG 才是主流
//!
//! 卡片可以是裸 JSON，但**外面流通的绝大多数是 PNG** —— 立绘本身就是卡片，
//! JSON 藏在 PNG 的 `tEXt` 块里（base64）。只认 JSON 的话，用户下载来的
//! 卡片十有八九导不进来，而报错只会说"不是合法 JSON"，他根本不知道
//! 问题在哪。
//!
//! 关键字有两个：V3 用 `ccv3`，V2 用 `chara`。**同时存在时优先 V3** ——
//! 很多卡为了兼容会两个都写，而 V2 那份是降级过的。
//!
//! ## 我们只取用得上的字段
//!
//! CCv3 有二十多个字段（lorebook、assets、多语言注释、群聊问候语……）。
//! 这里只映射 Anima 真正会用的那几个，其余原样丢掉 —— **不假装支持**。
//! 导入结果里会说明哪些字段被忽略了，用户才知道要不要手工补。

use std::path::Path;

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// 从卡片里取出来的、我们用得上的部分
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Card {
    pub name: String,
    pub description: String,
    pub personality: String,
    pub scenario: String,
    /// 卡片自带的 system prompt。有的话优先于 description 拼出来的那份
    pub system_prompt: String,
    /// 开场白
    pub first_message: String,
    /// 对话示例。放进 speaking_style.md —— 那正是"她怎么说话"的最好素材
    pub example_dialogue: String,
    pub creator: String,
    pub tags: Vec<String>,
    /// 卡片版本号（`spec_version`），只用来提示
    pub spec: String,
    /// **没被我们用上的字段名**。导入后如实告诉用户，别让他以为全都进来了
    pub ignored: Vec<String>,
}

/// 我们会用到的字段。除此之外的都算"忽略"
const USED: &[&str] = &[
    "name",
    "description",
    "personality",
    "scenario",
    "system_prompt",
    "first_mes",
    "mes_example",
    "creator",
    "tags",
];

fn text_at(data: &serde_json::Value, key: &str) -> String {
    data.get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// 解析一张卡。自动分辨 PNG 和 JSON
pub fn parse(bytes: &[u8]) -> Result<Card> {
    let json = if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        png_card_json(bytes).ok_or_else(|| {
            Error::config(
                "这张 PNG 里没有角色卡数据。\
                 它可能只是一张普通图片 —— 角色卡的 JSON 藏在 PNG 的 tEXt 块里"
                    .to_string(),
            )
        })?
    } else {
        String::from_utf8_lossy(bytes).to_string()
    };

    let root: serde_json::Value = serde_json::from_str(json.trim())
        .map_err(|e| Error::config(format!("角色卡解析失败：{e}")))?;

    // V2/V3 都是 `{spec, spec_version, data:{...}}`；也有裸 data 的老卡
    let spec = text_at(&root, "spec");
    let data = root.get("data").unwrap_or(&root);
    let obj = data
        .as_object()
        .ok_or_else(|| Error::config("角色卡的 data 不是一个对象".to_string()))?;

    let name = text_at(data, "name");
    if name.is_empty() {
        return Err(Error::config(
            "角色卡里没有 name 字段，这多半不是一张角色卡".to_string(),
        ));
    }

    let mut ignored: Vec<String> = obj
        .keys()
        .filter(|k| !USED.contains(&k.as_str()))
        // 这些是纯元信息，没被用上也不值得提
        .filter(|k| {
            !matches!(
                k.as_str(),
                "spec"
                    | "spec_version"
                    | "creator_notes"
                    | "creation_date"
                    | "modification_date"
                    | "character_version"
                    | "avatar"
            )
        })
        .cloned()
        .collect();
    ignored.sort();

    Ok(Card {
        name,
        description: text_at(data, "description"),
        personality: text_at(data, "personality"),
        scenario: text_at(data, "scenario"),
        system_prompt: text_at(data, "system_prompt"),
        first_message: text_at(data, "first_mes"),
        example_dialogue: text_at(data, "mes_example"),
        creator: text_at(data, "creator"),
        tags: data
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        spec: if spec.is_empty() {
            "chara_card_v2".into()
        } else {
            spec
        },
        ignored,
    })
}

/// 从 PNG 的 tEXt / iTXt 块里挖出角色卡 JSON。
///
/// PNG 结构：8 字节签名 + 若干 `[4 长度][4 类型][数据][4 CRC]`。
/// `tEXt` 的数据是 `关键字\0内容`。
///
/// **V3 优先**：很多卡为了兼容会同时写 `ccv3` 和 `chara`，而后者是降级过的
fn png_card_json(bytes: &[u8]) -> Option<String> {
    let mut v3: Option<String> = None;
    let mut v2: Option<String> = None;

    let mut i = 8usize;
    while i + 8 <= bytes.len() {
        let len = u32::from_be_bytes(bytes.get(i..i + 4)?.try_into().ok()?) as usize;
        let kind = bytes.get(i + 4..i + 8)?;
        let start = i + 8;
        let end = start.checked_add(len)?;
        if end > bytes.len() {
            break;
        }

        if kind == b"tEXt" || kind == b"iTXt" {
            let chunk = &bytes[start..end];
            if let Some(nul) = chunk.iter().position(|&b| b == 0) {
                let keyword = String::from_utf8_lossy(&chunk[..nul]).to_lowercase();
                // iTXt 在关键字后面还有压缩标志/方法/语言/翻译名四个以 \0 分隔的字段。
                // 只处理未压缩的那种 —— 压缩过的极少见，认不出来时如实报错
                let payload = if kind == b"iTXt" {
                    let rest = &chunk[nul + 1..];
                    // 跳过 compression_flag + compression_method 两个字节，
                    // 再跳过 language 和 translated_keyword 两个 \0 结尾的串
                    let mut p = 2usize;
                    for _ in 0..2 {
                        p += rest.get(p..)?.iter().position(|&b| b == 0)? + 1;
                    }
                    rest.get(p..)?
                } else {
                    &chunk[nul + 1..]
                };

                if keyword == "ccv3" || keyword == "chara" {
                    let raw = String::from_utf8_lossy(payload);
                    if let Ok(decoded) =
                        base64::engine::general_purpose::STANDARD.decode(raw.trim().as_bytes())
                    {
                        let text = String::from_utf8_lossy(&decoded).to_string();
                        if keyword == "ccv3" {
                            v3 = Some(text);
                        } else {
                            v2 = Some(text);
                        }
                    }
                }
            }
        }
        if kind == b"IEND" {
            break;
        }
        // 数据之后还有 4 字节 CRC
        i = end + 4;
    }
    v3.or(v2)
}

/// 卡片名字 → 目录名。目录名只收 `[a-z0-9_-]`
pub fn id_from_name(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if (ch == ' ' || ch == '_' || ch == '-') && !out.ends_with('_') {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('_').to_string();
    // 全是非 ASCII 的名字（中文/日文卡很常见）会被剥成空，
    // 那时候用一个稳定的兜底名，让调用方去补
    if trimmed.is_empty() {
        "card".to_string()
    } else {
        trimmed.chars().take(40).collect()
    }
}

/// 把卡片写成 Anima 的角色目录。
///
/// 映射是有取舍的，写在这里免得以后看不懂：
///
/// | 卡片字段 | 落到哪 | 为什么 |
/// |---|---|---|
/// | `description` + `scenario` | SKILL.md 正文 | 这两个合起来才是"她是谁、在什么场景里" |
/// | `personality` | personality.yaml | 我们的性格特质会被拼进提示词 |
/// | `mes_example` | speaking_style.md | 对话示例正是"她怎么说话"最好的素材 |
/// | `system_prompt` | SKILL.md 正文最前面 | 卡片自带的优先级最高 |
/// | `first_mes` | 只存档，不用 | 我们没有"开场白"这个概念 |
pub fn import(root: &Path, id: &str, card: &Card) -> Result<super::Character> {
    let created = super::create(root, id, &card.name, &one_line(&card.description))?;
    let dir = root.join(id);

    let mut body = String::new();
    if !card.system_prompt.is_empty() {
        body.push_str(&card.system_prompt);
        body.push_str("\n\n");
    }
    if !card.description.is_empty() {
        body.push_str("## 设定\n\n");
        body.push_str(&card.description);
        body.push_str("\n\n");
    }
    if !card.scenario.is_empty() {
        body.push_str("## 场景\n\n");
        body.push_str(&card.scenario);
        body.push_str("\n\n");
    }
    if !card.first_message.is_empty() {
        // 我们没有"开场白"这个概念，但丢掉可惜 —— 存着，用户想用可以自己搬
        body.push_str("## 开场白（卡片自带，Anima 目前不会自动说）\n\n");
        body.push_str(&card.first_message);
        body.push('\n');
    }

    let meta = format!(
        "---\nid: {id}\nname: {}\nmarker: \"\"\ndescription: {}\n---\n\n",
        yaml_scalar(&card.name),
        yaml_scalar(&one_line(&card.description)),
    );
    std::fs::write(dir.join("SKILL.md"), format!("{meta}{body}"))
        .map_err(|e| Error::Io(format!("写入 SKILL.md 失败：{e}")))?;

    if !card.personality.is_empty() {
        let yaml = format!("性格: {}\n", yaml_scalar(&one_line(&card.personality)));
        std::fs::write(dir.join("personality.yaml"), yaml)
            .map_err(|e| Error::Io(format!("写入 personality.yaml 失败：{e}")))?;
    }
    if !card.example_dialogue.is_empty() {
        std::fs::write(dir.join("speaking_style.md"), &card.example_dialogue)
            .map_err(|e| Error::Io(format!("写入 speaking_style.md 失败：{e}")))?;
    }

    tracing::info!(
        character = %id, spec = %card.spec, ignored = ?card.ignored,
        "导入了一张角色卡"
    );
    let _ = created;
    super::load(root, id)
}

/// 压成一行。frontmatter 的 description 是单行标量，带换行会让 YAML 解析失败
fn one_line(text: &str) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    joined.chars().take(160).collect()
}

/// YAML 标量转义。**卡片里的名字什么都可能有**（冒号、引号、`#`），
/// 不转义会让整个 frontmatter 解析失败，而那时角色是加载不出来的
fn yaml_scalar(text: &str) -> String {
    let escaped = text.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v3_json() -> String {
        serde_json::json!({
            "spec": "chara_card_v3",
            "spec_version": "3.0",
            "data": {
                "name": "Seraphina",
                "description": "森林守护者。",
                "personality": "温柔、坚定",
                "scenario": "你在森林里醒来",
                "first_mes": "你醒了？",
                "mes_example": "<START>\n{{user}}: 你好\n{{char}}: 你好呀",
                "system_prompt": "你要始终以 Seraphina 的身份说话。",
                "creator": "someone",
                "tags": ["fantasy", "healer"],
                "character_book": { "entries": [] },
                "alternate_greetings": ["嗨"],
                "assets": []
            }
        })
        .to_string()
    }

    #[test]
    fn parses_a_v3_card() {
        let c = parse(v3_json().as_bytes()).unwrap();
        assert_eq!(c.name, "Seraphina");
        assert_eq!(c.personality, "温柔、坚定");
        assert_eq!(c.system_prompt, "你要始终以 Seraphina 的身份说话。");
        assert_eq!(c.tags, vec!["fantasy", "healer"]);
        assert_eq!(c.spec, "chara_card_v3");
    }

    /// **没用上的字段要如实报出来。** 不报的话用户以为 lorebook 也导进来了
    #[test]
    fn reports_the_fields_it_does_not_use() {
        let c = parse(v3_json().as_bytes()).unwrap();
        assert!(c.ignored.contains(&"character_book".to_string()));
        assert!(c.ignored.contains(&"alternate_greetings".to_string()));
        assert!(!c.ignored.contains(&"name".to_string()));
    }

    /// 老卡是裸 data，没有 spec/data 外层
    #[test]
    fn parses_a_bare_v1_style_card() {
        let raw = serde_json::json!({ "name": "旧卡", "description": "很老" }).to_string();
        let c = parse(raw.as_bytes()).unwrap();
        assert_eq!(c.name, "旧卡");
    }

    #[test]
    fn a_card_without_a_name_is_rejected() {
        let raw = serde_json::json!({ "data": { "description": "无名" } }).to_string();
        let err = parse(raw.as_bytes()).unwrap_err().to_string();
        assert!(err.contains("name"), "报错要说清楚缺什么：{err}");
    }

    /// 造一张带 tEXt 的 PNG
    fn png_with(keyword: &str, payload: &str) -> Vec<u8> {
        let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        let b64 = base64::engine::general_purpose::STANDARD.encode(payload);
        let mut data = keyword.as_bytes().to_vec();
        data.push(0);
        data.extend_from_slice(b64.as_bytes());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(b"tEXt");
        out.extend_from_slice(&data);
        out.extend_from_slice(&[0, 0, 0, 0]); // CRC，解析时不校验
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(b"IEND");
        out.extend_from_slice(&[0, 0, 0, 0]);
        out
    }

    /// **外面流通的绝大多数卡片是 PNG。** 只认 JSON 等于大部分卡导不进来
    #[test]
    fn parses_a_card_embedded_in_a_png() {
        let c = parse(&png_with("ccv3", &v3_json())).unwrap();
        assert_eq!(c.name, "Seraphina");
    }

    #[test]
    fn parses_the_v2_keyword_too() {
        let c = parse(&png_with("chara", &v3_json())).unwrap();
        assert_eq!(c.name, "Seraphina");
    }

    /// 很多卡为了兼容两个关键字都写，**V2 那份是降级过的** —— 必须优先 V3
    #[test]
    fn prefers_v3_when_both_keywords_are_present() {
        let v2 = serde_json::json!({ "data": { "name": "降级版" } }).to_string();
        let mut png = png_with("chara", &v2);
        // 把 V3 的块插在 IEND 之前
        let iend = png.len() - 12;
        let v3_chunk = png_with("ccv3", &v3_json());
        let body = &v3_chunk[8..v3_chunk.len() - 12];
        png.splice(iend..iend, body.iter().copied());

        assert_eq!(parse(&png).unwrap().name, "Seraphina");
    }

    /// 普通图片要给一句人话，而不是"不是合法 JSON"
    #[test]
    fn a_plain_png_says_what_is_wrong() {
        let png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        let err = parse(&png).unwrap_err().to_string();
        assert!(err.contains("没有角色卡数据"), "{err}");
    }

    /// 截断的 PNG 不能让解析 panic
    #[test]
    fn a_truncated_png_does_not_panic() {
        let full = png_with("ccv3", &v3_json());
        for cut in [9, 12, 20, full.len() - 3] {
            let _ = parse(&full[..cut]);
        }
    }

    #[test]
    fn makes_safe_directory_ids() {
        assert_eq!(id_from_name("Seraphina"), "seraphina");
        assert_eq!(id_from_name("Mio  Honda"), "mio_honda");
        assert_eq!(id_from_name("A/B:C"), "abc");
        // 纯中文/日文名会被剥空，要有兜底
        assert_eq!(id_from_name("八重神子"), "card");
        assert_eq!(id_from_name("  "), "card");
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("anima-card-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn imports_into_a_loadable_character() {
        let root = scratch("import");
        let card = parse(v3_json().as_bytes()).unwrap();
        let c = import(&root, "seraphina", &card).unwrap();

        assert_eq!(c.name, "Seraphina");
        // 导入完必须能被正常加载 —— 导出一个加载不了的目录等于没导
        assert_eq!(
            super::super::load(&root, "seraphina").unwrap().name,
            "Seraphina"
        );

        let md = std::fs::read_to_string(root.join("seraphina/SKILL.md")).unwrap();
        assert!(md.contains("你要始终以 Seraphina 的身份说话"));
        assert!(md.contains("森林守护者"));
        assert!(md.contains("你在森林里醒来"));
        // 开场白我们不用，但要存着
        assert!(md.contains("你醒了？"));

        assert!(
            std::fs::read_to_string(root.join("seraphina/personality.yaml"))
                .unwrap()
                .contains("温柔")
        );
        assert!(
            std::fs::read_to_string(root.join("seraphina/speaking_style.md"))
                .unwrap()
                .contains("你好呀")
        );
    }

    /// **卡片里的名字什么都可能有。** 不转义会让 frontmatter 解析失败，
    /// 而那时角色是加载不出来的 —— 又一个不报错的失败
    #[test]
    fn names_with_yaml_metacharacters_still_load() {
        let root = scratch("yaml");
        let raw = serde_json::json!({
            "data": { "name": "A: \"B\" #c", "description": "带\n换行\n的设定" }
        })
        .to_string();
        let card = parse(raw.as_bytes()).unwrap();
        let c = import(&root, "tricky", &card).unwrap();
        assert_eq!(c.name, "A: \"B\" #c");
        assert!(!c.description.contains('\n'), "description 必须压成一行");
    }

    /// 卡片没有 personality / mes_example 时不该建出空文件
    #[test]
    fn a_sparse_card_does_not_create_empty_files() {
        let root = scratch("sparse");
        let raw = serde_json::json!({ "data": { "name": "简卡" } }).to_string();
        let card = parse(raw.as_bytes()).unwrap();
        import(&root, "simple", &card).unwrap();
        assert!(!root.join("simple/personality.yaml").exists());
        assert!(!root.join("simple/speaking_style.md").exists());
    }
}
