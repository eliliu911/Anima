//! 角色视觉基因锚点系统 (Character Visual DNA)
//!
//! 解决跨镜头生成时单纯依赖视觉参考图导致的面部特征漂移问题。
//! 结构化的语义特征拼成紧凑的自然语言锚点，与 IP-Adapter/PuLID
//! 形成【视觉 + 语义】双通道强锁。
//!
//! # 判别性强的锚点排在前面
//!
//! 原来是 8 维五官描述。后来补了两类，**它们比五官描述好用得多**：
//!
//! - **辨识标记**（`marks`）：「左眼下方两厘米一颗小痣」。
//! - **Hex 色值**（`color_anchors`）：`#3D2314`。
//!
//! 道理是一样的：`#3D2314` 和「左眼下方两厘米的痣」都是**判别性极强的
//! token**，模型抓得住；「清秀的东方女性」抓不住 —— 它对应的是一大片分布，
//! 每次采样落在哪儿全看运气，这就是长相漂移的来源。
//!
//! 所以 [`to_prompt_block`](CharacterVisualDna::to_prompt_block) 把这两类
//! **排在五官描述前面** —— 提示词越靠后越容易被稀释。

use serde::{Deserialize, Serialize};

/// 颜色锚点。**写 Hex 值，不写色彩名。**
///
/// 「棕色头发」覆盖的色域大得离谱，两镜之间差出一个色系都算「符合」。
/// `#1A1A1A` 没有这个问题。
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct ColorAnchors {
    /// 虹膜色，如 `#3D2314`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iris: Option<String>,
    /// 发色，如 `#1A1A1A`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hair: Option<String>,
    /// 肤色，如 `#E8C4A0`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skin: Option<String>,
    /// 唇色，如 `#C4727E`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lips: Option<String>,
}

impl ColorAnchors {
    fn parts(&self) -> Vec<String> {
        [
            ("iris", &self.iris),
            ("hair", &self.hair),
            ("skin", &self.skin),
            ("lips", &self.lips),
        ]
        .into_iter()
        .filter_map(|(k, v)| {
            let v = v.as_deref()?.trim();
            if v.is_empty() { None } else { Some(format!("{k} {v}")) }
        })
        .collect()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct CharacterVisualDna {
    /// 眼睛与眼睑细节 (e.g. "almond, slightly upturned double eyelid")
    pub eyes: Option<String>,
    /// 下颌与下巴轮廓 (e.g. "soft oval, delicate defined chin")
    pub jaw: Option<String>,
    /// 鼻梁与鼻头 (e.g. "straight bridge with delicate nose tip")
    pub nose: Option<String>,
    /// 唇形与唇色 (e.g. "natural curved thin lips, rose pink tone")
    pub mouth: Option<String>,
    /// 发型与刘海 (e.g. "short wavy chestnut bob with curtain bangs")
    pub hair: Option<String>,
    /// 发色与高光 (e.g. "warm chestnut brown with subtle golden highlights")
    pub hair_color: Option<String>,
    /// 肤色与质感 (e.g. "fair porcelain skin, warm undertone, smooth texture")
    pub skin: Option<String>,
    /// 标志性服饰配饰 (e.g. "black silk velvet evening gown, diamond pendant necklace")
    pub signature_outfit: Option<String>,
    /// **辨识标记 —— 这一层最强。**
    ///
    /// 位置要写死：`"small mole 2cm below the left eye"`、
    /// `"faint scar across the right eyebrow"`。
    /// 「有颗痣」这种没有位置的写法约束不住任何东西。
    #[serde(default)]
    pub marks: Vec<String>,
    /// Hex 颜色锚点。见 [`ColorAnchors`]。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_anchors: Option<ColorAnchors>,
    /// **这个角色专属的负向词。**
    ///
    /// 「金发」「蓝眼」「胡子」这种 —— 不是画质负向（那是全局的），
    /// 是「这个人不该长这样」。
    ///
    /// **注意它不是到处都有效**：Z-Image 和 Krea 2 跑在 cfg=1 上，
    /// 负向分支被抵消掉，传了也不生效（见 CLAUDE.md）。
    /// 所以调用方要按引擎判断发不发。
    #[serde(default)]
    pub avoid: Vec<String>,
}

impl CharacterVisualDna {
    /// 有没有任何内容。
    pub fn is_empty(&self) -> bool {
        self.to_prompt_block("x").is_empty()
    }

    /// 这个角色专属的负向词，逗号连起来。空 = 没有。
    pub fn avoid_prompt(&self) -> String {
        self.avoid
            .iter()
            .map(|a| a.trim())
            .filter(|a| !a.is_empty())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// **一致性锁面** —— 跨镜不变、必须**每条提示词都带**的那几个字。
    ///
    /// # 和 `to_prompt_block` 的分工
    ///
    /// `to_prompt_block` 是**识别锚点**：写给人读的事实 + 长描述，
    /// 只在没发参考图那条路上补（发了图再写一遍会和图打架、挤爆提示词）。
    ///
    /// 锁面不一样：**参考图补不上这一层**。一张角色板只能约束它自己画到的
    /// 部分 —— 被手挡住的道具、每镜重新构图的服装细节、只成形一半的东西，
    /// 都落在身份参考「默认控不住」的一侧。同一件毛衣第一镜蓝色第二镜红色，
    /// 通常不是资产写错了，是**没有任何一条提示词把颜色带进去**。
    ///
    /// 所以锁面**发了图也要发**。它能这么用的前提是它足够短：
    ///
    /// - 只收判别性最强、且**跨语言同形**的那些：Hex 色值、辨识标记。
    ///   色值在中英正文里是同一串字符，三处正文都能原样命中。
    /// - **不收五官描述** —— 那是识别锚点，一集里的锁通常是个位数，
    ///   把每条锚点都升级成锁，每条提示词都会被同一串名词短语撑满，
    ///   反而挤掉本镜真正要执行的动作。
    pub fn lock_line(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        for m in &self.marks {
            let m = m.trim();
            if !m.is_empty() {
                parts.push(m.to_string());
            }
        }
        if let Some(c) = &self.color_anchors {
            parts.extend(c.parts());
        }
        parts.join(", ")
    }

    /// 归一化并生成可直接注入到提示词末尾的 DNA Prompt 描述块。
    ///
    /// **辨识标记和 Hex 色值排在最前** —— 它们判别性最强，而提示词越靠后
    /// 越容易被稀释。
    pub fn to_prompt_block(&self, character_name: &str) -> String {
        let mut parts = Vec::new();

        // ① 辨识标记 —— 最强的锚点放第一位
        for m in &self.marks {
            let m = m.trim();
            if !m.is_empty() {
                parts.push(m.to_string());
            }
        }
        // ② Hex 色值
        if let Some(c) = &self.color_anchors {
            parts.extend(c.parts());
        }

        if let Some(ref e) = self.eyes {
            if !e.trim().is_empty() {
                parts.push(format!("eyes: {}", e.trim()));
            }
        }
        if let Some(ref j) = self.jaw {
            if !j.trim().is_empty() {
                parts.push(format!("jaw: {}", j.trim()));
            }
        }
        if let Some(ref n) = self.nose {
            if !n.trim().is_empty() {
                parts.push(format!("nose: {}", n.trim()));
            }
        }
        if let Some(ref m) = self.mouth {
            if !m.trim().is_empty() {
                parts.push(format!("mouth: {}", m.trim()));
            }
        }
        if let Some(ref h) = self.hair {
            if !h.trim().is_empty() {
                parts.push(format!("hair: {}", h.trim()));
            }
        }
        if let Some(ref hc) = self.hair_color {
            if !hc.trim().is_empty() {
                parts.push(format!("hair color: {}", hc.trim()));
            }
        }
        if let Some(ref s) = self.skin {
            if !s.trim().is_empty() {
                parts.push(format!("skin: {}", s.trim()));
            }
        }
        if let Some(ref o) = self.signature_outfit {
            if !o.trim().is_empty() {
                parts.push(format!("outfit: {}", o.trim()));
            }
        }

        if parts.is_empty() {
            return String::new();
        }

        let joined = parts.join(", ");
        format!("{character_name} visual DNA: {joined}")
    }

    /// 内置由美子的标准 Visual DNA 预设
    pub fn preset_yumiko() -> Self {
        Self {
            eyes: Some("almond eyes, refined double eyelid, sharp confident gaze".into()),
            jaw: Some("soft oval face, delicate chin".into()),
            nose: Some("straight elegant bridge".into()),
            mouth: Some("natural curved lips".into()),
            hair: Some("short wavy chestnut brown bob with curtain bangs".into()),
            hair_color: Some("warm chestnut brown".into()),
            skin: Some("fair porcelain east asian skin".into()),
            signature_outfit: Some("black silk velvet evening gown, diamond pendant".into()),
            marks: vec!["a small mole 2cm below the left eye".into()],
            color_anchors: Some(ColorAnchors {
                iris: Some("#4A2C17".into()), hair: Some("#6B4423".into()),
                skin: Some("#F0DCC8".into()), lips: Some("#C4727E".into()),
            }),
            avoid: vec!["blonde hair".into(), "blue eyes".into()],
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    /// **判别性强的排在最前。** 提示词越靠后越容易被稀释，
    /// 而辨识标记和 Hex 色值正是最不该被稀释的那两类。
    #[test]
    fn 辨识标记和色值排在五官描述前面() {
        let dna = CharacterVisualDna::preset_yumiko();
        let block = dna.to_prompt_block("Yumiko");
        let mark = block.find("mole 2cm below").expect("没有辨识标记");
        let hex = block.find("#4A2C17").expect("没有虹膜色值");
        let eyes = block.find("eyes:").expect("没有五官描述");
        assert!(mark < eyes, "辨识标记该排在五官前面：{block}");
        assert!(hex < eyes, "色值该排在五官前面：{block}");
    }

    /// 空字段一个都不发 —— 半个锚点进提示词只是噪声。
    #[test]
    fn 空的锚点不产生任何内容() {
        let mut dna = CharacterVisualDna {
            marks: vec!["  ".into()],
            color_anchors: Some(ColorAnchors::default()),
            ..Default::default()
        };
        assert_eq!(dna.to_prompt_block("空"), "", "全空时不该发东西");
        assert!(dna.is_empty());
        dna.marks = vec!["左眉一道疤".into()];
        assert!(dna.to_prompt_block("甲").contains("左眉一道疤"));
        assert!(!dna.is_empty());
    }

    #[test]
    fn 色值只发填了的那几个() {
        let dna = CharacterVisualDna {
            color_anchors: Some(ColorAnchors {
                iris: Some("#3D2314".into()),
                hair: None,
                skin: Some("  ".into()),
                lips: None,
            }),
            ..Default::default()
        };
        let block = dna.to_prompt_block("甲");
        assert!(block.contains("iris #3D2314"));
        assert!(!block.contains("hair"), "没填的不该出现：{block}");
        assert!(!block.contains("skin"), "只有空白的不该出现：{block}");
    }

    /// **锁面只收跨语言同形的那些，不收五官描述。**
    ///
    /// 锁面要逐字出现在每一条受影响的提示词里，所以必须短。
    /// 一集里的锁通常是个位数 —— 把每条识别锚点都升级成锁，
    /// 每条提示词都会被同一串名词短语撑满，反而挤掉本镜真正要执行的动作。
    #[test]
    fn 锁面只收标记和色值不收五官描述() {
        let dna = CharacterVisualDna::preset_yumiko();
        let lock = dna.lock_line();
        assert!(lock.contains("mole 2cm below"), "辨识标记要在：{lock}");
        assert!(lock.contains("#4A2C17"), "Hex 色值要在：{lock}");
        // 五官描述不进锁面 —— 那是识别锚点，长，而且各语言写法不同
        assert!(!lock.contains("almond"), "五官描述不该进锁面：{lock}");
        assert!(!lock.contains("jaw"), "{lock}");
        // 锁面要比整块锚点短得多
        assert!(lock.len() * 2 < dna.to_prompt_block("Yumiko").len(), "锁面太长了：{lock}");
    }

    #[test]
    fn 没有标记和色值时锁面为空() {
        let dna = CharacterVisualDna {
            eyes: Some("almond".into()),
            ..Default::default()
        };
        assert_eq!(dna.lock_line(), "", "只有五官描述时不该有锁面");
    }

    #[test]
    fn 角色专属负向拼成逗号串() {
        let dna = CharacterVisualDna {
            avoid: vec!["金发".into(), "  ".into(), "蓝眼".into()],
            ..Default::default()
        };
        assert_eq!(dna.avoid_prompt(), "金发, 蓝眼");
        assert_eq!(CharacterVisualDna::default().avoid_prompt(), "");
    }

    #[test]
    fn dna_prompt_block_formats_correctly() {
        let dna = CharacterVisualDna::preset_yumiko();
        let block = dna.to_prompt_block("Yumiko");
        assert!(block.starts_with("Yumiko visual DNA:"));
        assert!(block.contains("eyes: almond eyes"));
        assert!(block.contains("hair: short wavy chestnut"));
    }

    #[test]
    fn empty_dna_returns_empty_string() {
        let dna = CharacterVisualDna::default();
        let block = dna.to_prompt_block("Empty");
        assert_eq!(block, "");
    }
}
