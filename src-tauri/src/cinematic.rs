//! 细化时间线、合成提示词这两步的**唯一方法来源**：cinematic-video-prompt-engineer（CyberJ0605，MIT）的原文。
//!
//! # 分工（2026-09-29 用户定的「分工版」）
//!
//! | 步骤 | 技能 | 为什么 |
//! |---|---|---|
//! | 生成镜头 | ai-film（[`crate::ai_film`]） | 整场调度：观众什么时候看见什么、站位、轴线 |
//! | 细化时间线 | **cinematic** | 对白驱动表演、情绪 → 微表情、真实表演系统 |
//! | 合成提示词（线路 B） | **cinematic** | 写给视频模型的提示词，H3 是它点名的目标模型 |
//!
//! 每一步只认一家；交接处的规矩两家说法一致：后一步只把定好的镜头写细、写成提示词，不重新拆镜、改景别。
//! 同一镜两版对比（gpt-5.6，第 6 场第 13 镜）：结构一样，cinematic 在表演颗粒度上细一档
//! （眼睛先动头再跟、说话前吸气、听的人泄一口气），代价是字数翻倍 —— 所以 [`PREFACE`] 里写明「挑，不是全写」。
//!
//! 原文按标题整节读进来（`include_str!`），不改写。它是写给对话代理的：诊断、问用户、参考图阶段这些
//! 对话里的步骤不适用，前面统一加说明。

const SKILL: &str = include_str!("../../skills/cinematic-video-prompt-engineer/SKILL.md");
const STYLE: &str = include_str!("../../skills/cinematic-video-prompt-engineer/references/style_patterns.md");
const EMOTION: &str = include_str!("../../skills/cinematic-video-prompt-engineer/references/emotion_performance_prompt_library.md");

pub const PREFACE: &str = "下面是表演与镜头执行方法的原文（cinematic-video-prompt-engineer）。原文里的剧情诊断、改写策略、问用户、参考图阶段、\
选输出模式这些是对话里的步骤，这里都不适用：这一镜的结构已经定了，只把它的执行写细；原文提到的其他参考文件、最终提示词的版式也不适用 ——\
输出格式按下面的「生产约束」（JSON、全部中文）。\n\
**原文里的表演手段是用来挑的，不是全写**：每一段只挑这一刻最要紧的两三个看得见的信号（原文自己也说 take only 2-4 fitting visible signals），\
字数守住生产约束里给的数 —— 写满每一个手段，视频模型反而抓不住最关键的那几个。";

/// 合成提示词（线路 B 写手）那一步前面的说明
pub const COMPILE_PREFACE: &str = "Below is the original method text of cinematic-video-prompt-engineer. Its conversational steps (diagnosis, strategy, \
asking the user, reference-image stages, output modes) do not apply here: the shot is already designed and refined — you only write its English wording. \
Its final-prompt layout and character ceilings do not apply either: the H3 sections, tags, dialogue placeholders and word budget are fixed by the rules below. \
What applies is how to translate: keep only details that reduce ambiguity, positive action first, few scene-specific negatives, no invented events.";

/// `title` 作为**一行开头**出现的位置。
///
/// 只用 `find` 的话 `## Negative Constraints` 也能匹配到 `### Negative Constraints` 那一行的中间
/// （style_patterns 里就有）—— 现在真标题排在前面所以没事，原文挪一下顺序就会切在半行上、截错一整节（2026-10-03 审查）。
pub fn heading_at(md: &str, title: &str) -> Option<usize> {
    md.match_indices(title).map(|(i, _)| i).find(|&i| i == 0 || md[..i].ends_with('\n'))
}

/// 从 `title` 这一行到下一个同级或更高级的标题之前（含标题）。找不到返回空串 —— 有测试钉着每一节都取得到。
pub fn section<'a>(md: &'a str, title: &str) -> &'a str {
    let Some(start) = heading_at(md, title) else { return "" };
    let level = title.chars().take_while(|c| *c == '#').count();
    let rest = &md[start + title.len()..];
    let mut end = rest.len();
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        let hashes = line.chars().take_while(|c| *c == '#').count();
        if offset > 0 && hashes > 0 && hashes <= level && line.chars().nth(hashes) == Some(' ') {
            end = offset;
            break;
        }
        offset += line.len();
    }
    md[start..start + title.len() + end].trim_end()
}

/// 细化时间线的方法（分工版）：`SKILL.md` 的「Cinematic Translation Rules」，
/// `style_patterns.md` 里的真实表演、对白驱动表演、情绪 → 微表情、微表情动作库、台词时长预算、长特写微表情时间线，
/// 加 25 种情绪的表演库。
pub fn timeline_method() -> String {
    let parts = [
        section(SKILL, "## Cinematic Translation Rules"),
        section(STYLE, "## Live Performance Realism System"),
        section(STYLE, "## Dialogue-Driven Performance Control System"),
        section(STYLE, "## Emotion-to-Micro-Expression Map"),
        section(STYLE, "## Micro-Expression Action Library"),
        section(STYLE, "### Dialogue Timing Budget"),
        section(STYLE, "## Long Facial Close-Up Micro-Expression Timeline"),
        EMOTION.trim(),
    ];
    format!("{PREFACE}\n\n{}", parts.join("\n\n"))
}

/// 合成提示词（线路 B 写手）的方法：`SKILL.md` 的「Output Format」「Cinematic Translation Rules」「Anti-Patterns and Red Flags」，
/// `style_patterns.md` 的「Prompt Sampling Range Control Principles」「Negative Constraints」。
pub fn compile_method() -> String {
    let parts = [
        section(SKILL, "## Output Format"),
        section(SKILL, "## Cinematic Translation Rules"),
        section(SKILL, "## Anti-Patterns and Red Flags"),
        section(STYLE, "## Prompt Sampling Range Control Principles"),
        section(STYLE, "## Negative Constraints"),
    ];
    format!("{COMPILE_PREFACE}\n\n{}", parts.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 标题要在行首：`### X` 那一行中间的 `## X` 不算（2026-10-03 审查）
    #[test]
    fn 标题只认行首() {
        let md = "intro\n### Negative Constraints\nsub\n## Negative Constraints\nreal\n## Next\n";
        assert_eq!(section(md, "## Negative Constraints"), "## Negative Constraints\nreal");
        assert_eq!(crate::ai_film::chapter("x\n### 3. 小节\n## 3. 真的\n正文\n## 4. 下一章", 3), "## 3. 真的\n正文");
        assert_eq!(section("## A\nbody", "## A"), "## A\nbody", "文件开头那一行也认");
    }

    #[test]
    fn 每一节都取得到_而且只到下一节之前() {
        for (md, t) in [
            (SKILL, "## Cinematic Translation Rules"),
            (STYLE, "## Live Performance Realism System"),
            (STYLE, "## Dialogue-Driven Performance Control System"),
            (STYLE, "## Emotion-to-Micro-Expression Map"),
            (STYLE, "## Micro-Expression Action Library"),
            (STYLE, "### Dialogue Timing Budget"),
            (STYLE, "## Long Facial Close-Up Micro-Expression Timeline"),
        ] {
            let s = section(md, t);
            assert!(s.starts_with(t) && s.len() > 500, "{t} 没取到");
        }
        let rules = section(SKILL, "## Cinematic Translation Rules");
        assert!(!rules.contains("\n## Visual Reference Image Prompts"), "切过头了");
        // 二级标题那一节要带上下面的三级小节
        assert!(section(STYLE, "## Dialogue-Driven Performance Control System").contains("### Emotion Barrier"));
    }

    #[test]
    fn 合成的方法是原文_带负向约束库() {
        let m = compile_method();
        assert!(m.starts_with(COMPILE_PREFACE));
        for must in ["## Output Format", "## Anti-Patterns and Red Flags", "## Prompt Sampling Range Control Principles", "### Negative Constraint Library", "### When to Omit Negative Constraints"] {
            assert!(m.contains(must), "少了「{must}」");
        }
    }

    /// 表演手段是挑的不是全写 —— 09-29 第一次实跑每秒 94 字，超过上限 64
    #[test]
    fn 细化说明写明_挑不是全写() {
        assert!(PREFACE.contains("用来挑的，不是全写") && PREFACE.contains("字数守住"));
    }

    #[test]
    fn 方法是原文() {
        let m = timeline_method();
        assert!(m.starts_with(PREFACE));
        for must in ["Convert feelings into behavior", "Dialogue as Expression Timeline", "Emotion Barrier"] {
            assert!(m.contains(must), "少了「{must}」");
        }
    }
}
