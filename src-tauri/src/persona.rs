//! 形态解析：**当下这一刻,引擎/音色/面容/技能到底该用哪个,以及为什么**。
//!
//! ## 为什么必须有这一层
//!
//! 在它之前,这四样东西有**两个平级的写入口** —— 全局设置面板和角色绑定,
//! 谁后写谁赢。实际后果是一串查不明白的"没声音":
//!
//! - 角色绑定把 `gpt_sovits` 写进全局 config,切回默认形态**不还原** ——
//!   设置里显示 Edge、实际跑 SoVITS,报错说"需要参考音频";
//! - 音色只有一份、所有引擎共用,换引擎时把上一家的名字带过去 ——
//!   Kokoro 拿到 Edge 的音色名,去 HuggingFace 找 `.pt` 然后 404。
//!
//! 两次都不是"某处写错了",是**没有一个地方说得清最终值该是什么**。
//!
//! ## 关系
//!
//! ```text
//! ① 模型中枢（LLM）  完全独立,不参与形态。换角色不动它
//! ② 形态             第二权威
//!    ├─ 默认形态      特殊的**兜底**形态：它的配置就是全局那份
//!    └─ 具体角色      绑了什么用什么,没绑的那项落回默认形态
//! ```
//!
//! **模型不进形态**是有意的：换角色连带换模型会让首字延迟和工具可靠性
//! 一起跳变,而那两件事跟"她是谁"无关。
//!
//! ## 每一项都带来源
//!
//! 界面要在选中角色时把面板禁用、并显示当前值和"这项由谁决定"。
//! 只给最终值是不够的 —— 用户看到一个灰掉的下拉框却不知道为什么,
//! 只会以为坏了。所以 [`Resolved`] 里每一项都配一个 [`Origin`]。

use serde::{Deserialize, Serialize};

use crate::character::binding::Binding;

/// 这一项的值是谁定的
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    /// 当前角色绑定的
    Character,
    /// 落回默认形态（全局设置）
    Default,
}

impl Origin {
    /// 界面上能不能改这一项。**角色定的就不给改** ——
    /// 允许改的话又会出现两个真相
    pub fn editable(self) -> bool {
        matches!(self, Origin::Default)
    }
}

/// 默认形态那一份配置。它就是兜底 —— 没有更下面一层了
#[derive(Debug, Clone, PartialEq)]
pub struct Fallback {
    pub engine: String,
    /// 这个引擎自己记的音色（`TtsEngineConfig::voice` 解析之后的值）
    pub voice: String,
    pub avatar: Option<String>,
    pub skills: Vec<String>,
}

/// 一项值 + 它的来源
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Field<T> {
    pub value: T,
    pub origin: Origin,
}

impl<T> Field<T> {
    fn from_character(value: T) -> Self {
        Self {
            value,
            origin: Origin::Character,
        }
    }
    fn from_default(value: T) -> Self {
        Self {
            value,
            origin: Origin::Default,
        }
    }
}

/// 解析结果：现在到底用什么
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Resolved {
    pub engine: Field<String>,
    pub voice: Field<String>,
    pub avatar: Field<Option<String>>,
    pub skills: Field<Vec<String>>,
    /// 当前是不是默认形态。界面据此决定整块面板给不给改
    pub is_default_form: bool,
}

impl Resolved {
    /// 面容和语音**只在默认形态下可编辑**。
    ///
    /// 这是这次重构的核心约束：选了具体角色之后,这两块归角色管,
    /// 在全局面板上改只会造出第二个真相
    pub fn editable(&self) -> bool {
        self.is_default_form
    }
}

/// 算出当下该用什么。
///
/// `binding` 是 `None` 表示默认形态；是 `Some` 但某一项没绑时,那一项
/// **落回默认形态** —— 这就是"默认形态是兜底形态"的含义。
///
/// 音色跟着引擎走：角色绑了引擎却没绑音色时,用**那个引擎自己**的音色,
/// 而不是默认形态当前引擎的音色 —— 后者是另一套命名空间,给过去就是 404。
///
/// `voice_of_engine`：按引擎 id 取**它自己**记的音色。跨引擎沿用音色名
/// 是之前那一串 404 的根源,所以这个查询必须由调用方按引擎给。
pub fn resolve(
    binding: Option<&Binding>,
    fallback: &Fallback,
    voice_of_engine: &dyn Fn(&str) -> String,
) -> Resolved {
    let Some(b) = binding else {
        return Resolved {
            engine: Field::from_default(fallback.engine.clone()),
            voice: Field::from_default(fallback.voice.clone()),
            avatar: Field::from_default(fallback.avatar.clone()),
            skills: Field::from_default(fallback.skills.clone()),
            is_default_form: true,
        };
    };

    let engine = match b.engine.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(e) => Field::from_character(e.to_string()),
        None => Field::from_default(fallback.engine.clone()),
    };

    let voice = match b.voice.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(v) => Field::from_character(v.to_string()),
        // 角色换了引擎却没绑音色：**必须取新引擎自己的音色**。
        // 沿用默认形态那个是另一套命名空间的名字
        None if engine.origin == Origin::Character => {
            Field::from_character(voice_of_engine(&engine.value))
        }
        None => Field::from_default(fallback.voice.clone()),
    };

    Resolved {
        engine,
        voice,
        avatar: match b.avatar.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(a) => Field::from_character(Some(a.to_string())),
            None => Field::from_default(fallback.avatar.clone()),
        },
        skills: match &b.skills {
            // 空数组是"一个都别挂",和"没绑"不是一回事
            Some(s) => Field::from_character(s.clone()),
            None => Field::from_default(fallback.skills.clone()),
        },
        is_default_form: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fallback() -> Fallback {
        Fallback {
            engine: "edge".into(),
            voice: "zh-CN-XiaoxiaoNeural".into(),
            avatar: Some("默认脸.vrm".into()),
            skills: vec!["general".into()],
        }
    }

    /// 每个引擎自己记的音色。跨引擎沿用是之前一串 404 的根源
    fn voices(engine: &str) -> String {
        match engine {
            "edge" => "zh-CN-XiaoxiaoNeural".into(),
            "gpt_sovits" => "yaemiko/d".into(),
            "openai_compatible" => "zf_xiaobei".into(),
            _ => String::new(),
        }
    }

    fn bind(engine: Option<&str>, voice: Option<&str>, avatar: Option<&str>) -> Binding {
        Binding {
            engine: engine.map(str::to_string),
            voice: voice.map(str::to_string),
            avatar: avatar.map(str::to_string),
            skills: None,
        }
    }

    #[test]
    fn default_form_uses_the_global_config_and_is_editable() {
        let r = resolve(None, &fallback(), &voices);
        assert_eq!(r.engine.value, "edge");
        assert_eq!(r.voice.value, "zh-CN-XiaoxiaoNeural");
        assert!(r.is_default_form);
        assert!(r.editable(), "默认形态下面板必须可编辑");
        for o in [
            r.engine.origin,
            r.voice.origin,
            r.avatar.origin,
            r.skills.origin,
        ] {
            assert_eq!(o, Origin::Default);
        }
    }

    /// **这是这次重构的核心约束。** 选了具体角色之后,语音和面容归角色管
    #[test]
    fn picking_a_character_locks_the_panels() {
        let b = bind(
            Some("gpt_sovits"),
            Some("yaemiko/greeting"),
            Some("yae.glb"),
        );
        let r = resolve(Some(&b), &fallback(), &voices);
        assert!(!r.is_default_form);
        assert!(!r.editable(), "选了角色就不该还能在全局面板上改");
        assert_eq!(r.engine.origin, Origin::Character);
        assert!(!r.engine.origin.editable());
    }

    /// 没绑的那项落回默认形态 —— 这就是"默认形态是兜底形态"
    #[test]
    fn unbound_fields_fall_back_to_the_default_form() {
        let b = bind(None, None, Some("yae.glb"));
        let r = resolve(Some(&b), &fallback(), &voices);
        assert_eq!(r.engine.value, "edge");
        assert_eq!(r.engine.origin, Origin::Default);
        assert_eq!(r.avatar.value.as_deref(), Some("yae.glb"));
        assert_eq!(r.avatar.origin, Origin::Character);
    }

    /// **角色换了引擎却没绑音色时,音色必须跟着新引擎走。**
    ///
    /// 沿用默认形态那个音色名是另一套命名空间：Kokoro 拿到
    /// `zh-CN-XiaoxiaoNeural` 会去 HuggingFace 找 `.pt` 然后 404,
    /// 而用户看到的只是"没声音"。这条是真实事故的回归测试
    #[test]
    fn switching_engine_without_a_voice_takes_the_new_engines_own_voice() {
        let b = bind(Some("openai_compatible"), None, None);
        let r = resolve(Some(&b), &fallback(), &voices);
        assert_eq!(r.engine.value, "openai_compatible");
        assert_eq!(r.voice.value, "zf_xiaobei", "沿用了上一个引擎的音色名");
        assert_eq!(r.voice.origin, Origin::Character);
    }

    /// 绑了引擎也绑了音色时，用绑的那个
    #[test]
    fn an_explicit_voice_wins_over_the_engine_default() {
        let b = bind(Some("gpt_sovits"), Some("furina/b"), None);
        let r = resolve(Some(&b), &fallback(), &voices);
        assert_eq!(r.voice.value, "furina/b");
    }

    /// 空字符串等于没绑 —— 不是"绑了一个叫空的东西"
    #[test]
    fn blank_strings_count_as_unbound() {
        let b = bind(Some("  "), Some(""), Some(" "));
        let r = resolve(Some(&b), &fallback(), &voices);
        assert_eq!(r.engine.value, "edge");
        assert_eq!(r.engine.origin, Origin::Default);
        assert_eq!(r.avatar.value.as_deref(), Some("默认脸.vrm"));
    }

    /// 空技能数组是"一个都别挂",和"没绑"不是一回事
    #[test]
    fn an_empty_skill_list_is_a_real_choice() {
        let mut b = bind(None, None, None);
        b.skills = Some(vec![]);
        let r = resolve(Some(&b), &fallback(), &voices);
        assert!(r.skills.value.is_empty());
        assert_eq!(r.skills.origin, Origin::Character);

        b.skills = None;
        let r = resolve(Some(&b), &fallback(), &voices);
        assert_eq!(r.skills.value, vec!["general".to_string()]);
        assert_eq!(r.skills.origin, Origin::Default);
    }

    /// **退出角色必须完全回到默认形态。** 之前的 bug 正是"退出了但引擎没还原"
    #[test]
    fn leaving_a_character_restores_everything() {
        let b = bind(Some("gpt_sovits"), Some("yaemiko/d"), Some("yae.glb"));
        let with = resolve(Some(&b), &fallback(), &voices);
        let without = resolve(None, &fallback(), &voices);
        assert_ne!(with.engine.value, without.engine.value);
        assert_eq!(without.engine.value, "edge");
        assert_eq!(without.voice.value, "zh-CN-XiaoxiaoNeural");
        assert_eq!(without.avatar.value.as_deref(), Some("默认脸.vrm"));
    }

    /// 解析必须是纯的：同样的输入永远同样的输出,而且不改动入参
    #[test]
    fn resolving_twice_gives_the_same_answer() {
        let b = bind(Some("gpt_sovits"), None, None);
        let f = fallback();
        let a = resolve(Some(&b), &f, &voices);
        let c = resolve(Some(&b), &f, &voices);
        assert_eq!(a, c);
        assert_eq!(f, fallback(), "解析改动了兜底配置");
    }
}
