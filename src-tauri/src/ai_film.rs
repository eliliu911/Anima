//! **生成镜头**这一步的唯一方法来源：ai-film（62656456/ai-film-skills，Apache-2.0）的 `ai-storyboard-director` 5.7.1。
//!
//! # 分工（2026-09-29 用户定的「分工版」）
//!
//! 用户：「把全部链路换成同一套 SKILL，而不是大杂烩。」当时三个环节拼着六个来源，互相打架 ——
//! 「每一秒都要有看得见的新变化」对「安静、停住都成立」，「景别至少跳一级」对「没有固定景别阶梯」。
//! 三套技能各有分工，最后定成**每一步只认一家**：生成镜头用 ai-film（整场调度：观众什么时候看见什么、站位、轴线），
//! 细化时间线和合成提示词用 cinematic（表演和写给视频模型的提示词，见 [`crate::cinematic`]）。
//! 选型对照和实测见 `docs/plans/2026-09-29-分镜链路统一为ai-film一套技能.md`。
//!
//! # 怎么用
//!
//! 技能原文按章节原样读进来（`include_str!`），**不改写、不摘句** —— 改技能文件，提示词跟着变。
//! 我们自己只加技术硬约束（输出格式、台词逐字、H3 单次时长……），不加任何拍法规则：
//! 拍法规则一加，就又回到两套思路打架。
//!
//! 原文是写给对话代理的，里面有「读取某文件」「问用户」「创意备选」这些对话里的步骤。
//! 按钮这条路没有人在线，所以原文前面统一加 [`PREFACE`] 说明这些不适用，而不是去删原文里的句子。

const SHOT_DESIGN: &str = include_str!("../../skills/ai-storyboard-director/references/shot-design-engine.md");
const FRAMING: &str = include_str!("../../skills/ai-storyboard-director/references/framing-and-axis.md");
const CONTRACT: &str = include_str!("../../skills/ai-storyboard-director/references/production-contract.md");

/// 原文前面的一句说明。见模块注释。
pub const PREFACE: &str = "下面是分镜方法的原文（ai-film 分镜导演 5.7.1）。原文里提到的其他参考文件、和用户确认、创意备选、保存设计记录，\
是对话里的步骤，这里都不适用：没有人在线回答问题，一次交出定稿。原文说「用户锁定」的，这里指剧本原文、资产清单和下面的生产约束。";

/// 取 `## N.` 这一章（到下一个 `## ` 为止，含标题）。找不到返回空串 —— 有测试钉着每一章都取得到。
pub fn chapter(md: &str, num: u32) -> &str {
    let head = format!("## {num}.");
    // 要在行首：`### 3.` 那一行的中间也有 `## 3.`（见 `cinematic::heading_at`）
    let Some(start) = crate::cinematic::heading_at(md, &head) else { return "" };
    let rest = &md[start..];
    let end = rest[head.len()..].find("\n## ").map(|i| i + head.len() + 1).unwrap_or(rest.len());
    rest[..end].trim_end()
}

fn chapters(md: &str, nums: &[u32]) -> String {
    nums.iter().map(|n| chapter(md, *n)).collect::<Vec<_>>().join("\n\n")
}

/// 生成镜头这一问的方法：观众经历 → 调度 → 摄影 → 时间（`shot-design-engine` §1~§5），
/// 轴线（`framing-and-axis` §4~§5），台词与声音（生产合同 §5）。
///
/// 没带的章节和原因：`shot-design-engine` §6（成品反查、保存记录）是对话里交付前的自查；
/// `framing-and-axis` §2（五层构图判断）是构图细查，一场一问里放不下；
/// 生产合同 §1~§4 是「分镜转提示词」的编译规则 —— 细化和合成那两步用的是 cinematic（见 [`crate::cinematic`]）。
pub fn shot_table_method() -> String {
    format!(
        "{PREFACE}\n\n{}\n\n{}\n\n{}",
        chapters(SHOT_DESIGN, &[1, 2, 3, 4, 5]),
        chapters(FRAMING, &[4, 5]),
        chapter(CONTRACT, 5),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 章节是按标题切的，技能文件改了标题就会切空 —— 空了等于把方法整段丢掉、模型照样出东西，不报错
    #[test]
    fn 每一章都取得到() {
        for n in 1..=6 {
            assert!(chapter(SHOT_DESIGN, n).starts_with(&format!("## {n}.")), "shot-design-engine 第 {n} 章");
        }
        for n in [4, 5] {
            assert!(chapter(FRAMING, n).starts_with(&format!("## {n}.")), "framing-and-axis 第 {n} 章");
        }
        for n in 1..=6 {
            assert!(chapter(CONTRACT, n).starts_with(&format!("## {n}.")), "production-contract 第 {n} 章");
        }
    }

    #[test]
    fn 一章只到下一章之前() {
        let c = chapter(SHOT_DESIGN, 1);
        assert!(!c.contains("## 2."), "切过头了");
        assert!(chapter(SHOT_DESIGN, 6).ends_with(chapter(SHOT_DESIGN, 6).lines().last().unwrap()), "最后一章到文件尾");
        assert_eq!(chapter(SHOT_DESIGN, 9), "");
    }

    /// 原样进来：拿原文里几句有代表性的话对一下，防止有人手抄一份「精简版」
    #[test]
    fn 方法是原文() {
        let m = shot_table_method();
        for must in [
            "## 1. 先找到这场戏的变化",
            "没有最低镜数、固定秒数或远中近阶梯",
            "安静、无风、稳定光照和有意停住都成立",
            "## 4. 建立当前有效轴线",
            "## 5. 台词与声音",
            "未获改写授权时原台词逐字保留",
        ] {
            assert!(m.contains(must), "少了原文「{must}」");
        }
        assert!(m.starts_with(PREFACE));
        // 编译规则不在这一步
        assert!(!m.contains("## 2. 六模块母提示词"));
    }
}
