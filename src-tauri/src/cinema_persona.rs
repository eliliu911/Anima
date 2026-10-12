//! 人设模板：**不需要底图,直接从提示词生成角色**。
//!
//! ## 为什么这条路更好
//!
//! 参考图（IPAdapter）迁移的不只是身份 —— 实测它会把参考图的**构图、表情、
//! 服装配色**一起搬过来：
//!
//! - 要黑裙子，参考图是白上衣 → 出来银裙
//! - 要「温暖微笑」，参考图面无表情 → 出来面无表情
//! - 要全身三视图，参考图是半身肖像 → **整排塌成四个半身正面**
//!
//! 同一提示词不挂参考图那次，出来的是最干净的六视图。
//!
//! ## 底图不是不要，是往后挪
//!
//! 角色创建阶段用提示词生成设定图；**设定图的正面那格就成了后续所有分镜的
//! 参考图**。这样参考图是「全身、中性表情、目标服装」的，
//! 正好没有上面那三种副作用。
//!
//! 反过来（先要用户找一张照片）不但多一道门槛，那张照片还多半是半身肖像 ——
//! 恰好是最坏的参考图。

use serde::{Deserialize, Serialize};

/// 一套人设。中文给人看，英文给模型看。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Persona {
    pub id: &'static str,
    /// 界面上显示的名字
    pub label: &'static str,
    /// 中文描述。**这一段是给用户改的** —— 他们照着改比改英文提示词容易得多
    pub summary: &'static str,
    /// 外貌触发词（英文，送进模型）
    pub trigger: &'static str,
    /// 服装（英文）
    pub clothing: &'static str,
}

/// 内置人设。覆盖短剧常见的角色档位：现代都市男女、年龄两端、反派。
///
/// **每条的英文都是从中文那段翻过来的**，不是各写各的 ——
/// 两边不一致时用户改了中文却没效果，而且看不出为什么。
///
/// ## 人种必须显式写出来
///
/// 中文语境里「法式气质」说的是**穿搭调性**，不是长相；但直译成
/// `french elegance` 之后 RealVisXL 会照字面理解，出来是欧洲面孔。
/// 短剧要的是东亚面孔，所以每条都写死 `east asian`；
/// 「法式」这类调性词只留在服装那一栏。
pub const PERSONAS: &[Persona] = &[
    Persona {
        id: "modern_gentle_woman",
        label: "现代女主 · 温柔治愈",
        summary: "28 岁女性，法式气质，五官精致立体，皮肤白皙细腻，长发微卷自然垂落，\
                  深棕发色，侧分刘海，眼神温柔坚定，穿米色西装外套内搭白色缎面 V 领上衣，\
                  简约金色项链和耳环",
        trigger: "28 year old east asian woman, chinese, refined delicate facial features, \
                  fair smooth porcelain skin, long wavy dark brown hair, side-parted bangs, \
                  gentle confident gaze, natural light makeup",
        clothing: "beige blazer over white satin v-neck top, effortless french styling, \
                   simple thin gold necklace and small hoop earrings",
    },
    Persona {
        id: "modern_sharp_woman",
        label: "现代女主 · 凌厉御姐",
        summary: "30 岁女性，锋利气场，高颧骨，黑色长直发，红唇，眼神冷静审视，\
                  穿剪裁利落的黑色西装套裙，白衬衫，细高跟",
        trigger:
            "30 year old east asian woman, chinese, sharp commanding presence, high cheekbones, \
                  long straight black hair, red lips, cool assessing gaze",
        clothing: "sharply tailored black skirt suit, crisp white shirt, stiletto heels",
    },
    Persona {
        id: "modern_ceo_man",
        label: "现代男主 · 冷峻总裁",
        summary: "32 岁男性，轮廓分明，短黑发梳理整齐，眼神深邃克制，下颌线清晰，\
                  穿深灰色定制西装三件套，白衬衫，金属腕表",
        trigger: "32 year old east asian man, chinese, chiseled features, neat short black hair, \
                  deep restrained gaze, sharp jawline",
        clothing: "charcoal grey three-piece tailored suit, white dress shirt, metal wristwatch",
    },
    Persona {
        id: "youth_student",
        label: "少年 · 清爽学生",
        summary: "18 岁男性，干净清爽，短碎发，笑起来有点腼腆，身形偏瘦，\
                  穿白色卫衣配牛仔裤，帆布鞋",
        trigger:
            "18 year old east asian young man, chinese, clean fresh look, short layered hair, \
                  slightly shy smile, slim build",
        clothing: "white hoodie, blue jeans, canvas sneakers",
    },
    Persona {
        id: "elder_woman",
        label: "长辈 · 端庄祖母",
        summary: "65 岁女性，慈祥端庄，银白色盘发，眼角有细纹，神态从容，\
                  穿深蓝色改良旗袍，一串珍珠项链",
        trigger:
            "65 year old east asian woman, chinese, kind dignified face, silver hair in a bun, \
                  fine wrinkles around the eyes, composed expression",
        clothing: "deep blue modernized qipao, single strand pearl necklace",
    },
    Persona {
        id: "villain_woman",
        label: "反派 · 心机名媛",
        summary: "26 岁女性，明艳精致，栗色大波浪，嘴角带一点算计的笑，\
                  穿酒红色缎面连衣裙，夸张宝石耳环",
        trigger:
            "26 year old east asian woman, chinese, striking polished beauty, chestnut wavy hair, \
                  faint calculating smile",
        clothing: "wine red satin dress, statement gemstone earrings",
    },
];

pub fn find(id: &str) -> Option<&'static Persona> {
    PERSONAS.iter().find(|p| p.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id不重复() {
        let mut ids: Vec<&str> = PERSONAS.iter().map(|p| p.id).collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n, "有重复的 id");
    }

    #[test]
    fn 每条都四样俱全() {
        for p in PERSONAS {
            assert!(!p.label.trim().is_empty(), "{} 缺 label", p.id);
            assert!(!p.summary.trim().is_empty(), "{} 缺中文描述", p.id);
            assert!(!p.trigger.trim().is_empty(), "{} 缺触发词", p.id);
            assert!(!p.clothing.trim().is_empty(), "{} 缺服装", p.id);
        }
    }

    #[test]
    fn 英文提示词里不能混中文() {
        // 送进 SDXL 的是英文 CLIP，混了中文那几个 token 会被当成噪声 ——
        // **不报错**，只是那部分描述悄悄失效
        for p in PERSONAS {
            for (field, text) in [("trigger", p.trigger), ("clothing", p.clothing)] {
                assert!(
                    !text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)),
                    "{} 的 {field} 里混了中文：{text}",
                    p.id
                );
            }
        }
    }

    #[test]
    fn 中文描述里要有中文() {
        // 反过来：summary 是给用户看和改的，写成英文就失去意义了
        for p in PERSONAS {
            assert!(
                p.summary
                    .chars()
                    .any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)),
                "{} 的中文描述没有中文",
                p.id
            );
        }
    }

    #[test]
    fn 每条都写明了人种() {
        // 中文里「法式气质」说的是穿搭调性，直译成 french elegance 之后
        // RealVisXL 会照字面理解成欧洲面孔 —— 实测就是这样。
        // 短剧要东亚面孔，所以人种必须显式写在触发词里
        for p in PERSONAS {
            assert!(
                p.trigger.contains("east asian"),
                "{} 的触发词没写明人种，出来会是欧美长相：{}",
                p.id,
                p.trigger
            );
        }
    }

    #[test]
    fn 触发词里不留会被当成长相的地域词() {
        // 「french」留在服装栏可以（那是调性），放进长相触发词就会变脸
        for p in PERSONAS {
            for bad in ["french", "european", "caucasian"] {
                assert!(
                    !p.trigger.to_lowercase().contains(bad),
                    "{} 的触发词里有 {bad}，会把长相带偏",
                    p.id
                );
            }
        }
    }

    #[test]
    fn 覆盖到不同性别和年龄档() {
        // 界面上标着「男/女/老/少」，少一档就是标错了
        let all: String = PERSONAS
            .iter()
            .map(|p| p.trigger)
            .collect::<Vec<_>>()
            .join(" ");
        assert!(all.contains("woman"), "缺女性");
        assert!(all.contains("man"), "缺男性");
        assert!(PERSONAS.iter().any(|p| p.trigger.contains("18 ")), "缺少年");
        assert!(PERSONAS.iter().any(|p| p.trigger.contains("65 ")), "缺长辈");
    }
}
