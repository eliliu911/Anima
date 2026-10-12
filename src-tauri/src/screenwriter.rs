//! 编剧手艺 —— 按需挑几节塞进「原文 → 剧本」那一步。
//!
//! # 之前这一步背后什么都没有
//!
//! 分镜那半边有整套词库（`skills/film-director/reference/`，加上
//! `shared/shot-language.json` 那份唯一词表），而编剧这半边只有
//! [`crate::prompts::SCREENPLAY_FROM_SOURCE`] 一段提示词。
//! 于是剧本质量全看模型当天的发挥 —— 而它是下游一切的输入。
//!
//! # 手艺是收来的，不是我们写的
//!
//! `skills/screenwriter/reference/` 下那几份来自 drama-skills（MIT），
//! **原样收进来不改**。压成我们自己的提示词会分叉：上游改了跟不上，
//! 我们改了也说不清改的是谁的。镜头语言那次已经得出过同一个结论。
//!
//! # 为什么是「按需」而不是全塞
//!
//! 那几份加起来 1600 多行。全塞进去有两个坏处，而且都不报错：
//!
//! - **稀释**。提示词越长，靠后的要求越容易被忽略 —— 这一点在出图那边
//!   实测过（SDXL 的 CLIP 是 77 token 分块取平均，而长文本模型也一样
//!   有注意力预算）。真正要紧的那几条会被淹掉。
//! - **误导**。「精确倒计时怎么写」对一个没有倒计时的故事不是帮助，
//!   是**诱导** —— 上游 SKILL.md 明确写着「本技能的审查项与手艺默认
//!   只帮助实现这些要求，**不能反过来补造题面没有的**精确数字、记录、
//!   资源、程序、关系或机关」。
//!
//! 所以照抄上游的做法：默认只给核心那两节，输入里真的出现对应信号时
//! 才把那一节加进来。

/// 一份收进来的参考。
struct Reference {
    /// 显示用的名字，会写进提示词里说明这段是哪来的。
    file: &'static str,
    body: &'static str,
}

const SCRIPT_CRAFT: Reference = Reference {
    file: "script-craft.md",
    body: include_str!("../../skills/screenwriter/reference/script-craft.md"),
};
const DIALOGUE_CRAFT: Reference = Reference {
    file: "dialogue-craft.md",
    body: include_str!("../../skills/screenwriter/reference/dialogue-craft.md"),
};
const SCENE_SOUND: Reference = Reference {
    file: "scene-sound-dramaturgy.md",
    body: include_str!("../../skills/screenwriter/reference/scene-sound-dramaturgy.md"),
};
const SUBSTITUTABLE: Reference = Reference {
    file: "substitutable-realization.md",
    body: include_str!("../../skills/screenwriter/reference/substitutable-realization.md"),
};

/// 一节 —— 按 `## ` 切出来的顶层小节。
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub file: &'static str,
    /// `3. 冲突场景的发动机：目标、反对、转向、退出`
    pub heading: String,
    /// 含标题行在内的全文。
    pub text: String,
}

/// 把一份参考切成顶层小节。
///
/// 只切 `## `，不切 `### ` —— 子节是它父节的组成部分，
/// 单独拎出来会丢掉「这一条在讲什么场合」。
fn split_sections(r: &Reference) -> Vec<Section> {
    let mut out = Vec::new();
    let mut cur: Option<(String, String)> = None;
    for line in r.body.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            if let Some((heading, text)) = cur.take() {
                out.push(Section { file: r.file, heading, text });
            }
            cur = Some((h.trim().to_string(), format!("{line}\n")));
        } else if let Some((_, text)) = cur.as_mut() {
            text.push_str(line);
            text.push('\n');
        }
    }
    if let Some((heading, text)) = cur {
        out.push(Section { file: r.file, heading, text });
    }
    // 目录那一节不是手艺，别占预算
    out.retain(|s| !s.heading.starts_with("目录"));
    out
}

/// 故事里的「发动机」。**只在输入真的以它为发动机时才启用** ——
/// 上游 SKILL.md 的原话是：不能反过来补造题面没有的机关。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    /// 艰难选择：两个方向都要活着。
    Choice,
    /// 精确死线 / 倒计时。
    Deadline,
    /// 有限次数 / 轮次 / 呼吸 / 节拍。
    LimitedUnit,
    /// 会改变人物判断的关键物证。
    Evidence,
    /// 对白密集。
    DialogueHeavy,
    /// 声音承担戏剧功能。
    Sound,
    /// **心理活动多。**
    ///
    /// 上游把这一节的触发条件写成「同一故事义务的不同可拍实现」。
    /// 原文里的内心戏正是最需要它的地方：「他很愤怒」拍不出来，
    /// 必须换成一个可拍的实现，而换法不止一种。
    ///
    /// 我们的提示词里本来就有一条「原文里的心理活动、旁白式叙述不要
    /// 直接变成台词」—— 这一节是那条规则背后的手艺。
    Interiority,
}

impl Engine {
    /// 命中它要看到什么词。
    ///
    /// **宁可漏，不可滥。** 漏了只是少给一节手艺；滥了会让模型去
    /// 补造原文里根本没有的机关，而那种失败看起来像「模型自己加戏」。
    fn words(self) -> &'static [&'static str] {
        match self {
            Engine::Choice => &["两难", "抉择", "只能选", "要么", "取舍", "二选一"],
            // **不只是「倒计时」这三个字。** 实跑第一章时漏了 ——
            // 原文写的是「剩余续航 231.4 小时」「比昨天少了 6.8 小时」，
            // 一个带数字、会减少的量，而词表里只有「小时内」。
            // 所以另外还看「数字 + 时间/资源单位」，见 `has_counted_unit`。
            Engine::Deadline => &["倒计时", "死线", "最后期限", "来不及", "截止", "期限前", "撑不到"],
            Engine::LimitedUnit => &["最后一次", "只剩", "仅剩", "一次机会", "每一轮", "三次机会"],
            Engine::Evidence => &["证据", "录音", "物证", "监控", "证明他", "证明她", "日记", "证词"],
            Engine::Sound => &["脚步声", "呼吸声", "枪声", "铃声", "音效", "寂静无声"],
            Engine::Interiority => &["心里想", "意识到", "回忆起", "在心中", "脑海里", "暗自", "她知道", "他知道"],
            // 对白密集不靠词表，靠引号密度 —— 见 `detect`
            Engine::DialogueHeavy => &[],
        }
    }

    fn section_of(self) -> (&'static Reference, &'static str) {
        match self {
            Engine::Choice | Engine::LimitedUnit | Engine::Deadline => (&SCRIPT_CRAFT, "6."),
            Engine::Evidence => (&SCRIPT_CRAFT, "5."),
            Engine::DialogueHeavy => (&DIALOGUE_CRAFT, "3."),
            Engine::Sound => (&SCENE_SOUND, ""),
            Engine::Interiority => (&SUBSTITUTABLE, "功能与实现分开写"),
        }
    }
}

pub const ALL_ENGINES: [Engine; 7] = [
    Engine::Choice,
    Engine::Deadline,
    Engine::LimitedUnit,
    Engine::Evidence,
    Engine::DialogueHeavy,
    Engine::Sound,
    Engine::Interiority,
];

/// 对白算「密集」的门槛：每千字多少对引号。
///
/// 中文小说里对白用「」或“”。这个数是**保守**的 —— 宁可漏。
pub const DIALOGUE_PER_1K: f64 = 6.0;

/// 带数字的时间 / 资源读数 —— 倒计时的真正形状。
///
/// # 为什么不能只认「倒计时」这三个字
///
/// 实跑《第01章-电量还剩多少》时漏了：原文写的是
/// 「剩余续航 **231.4 小时**」「比昨天少了 **6.8 小时**」 ——
/// 一个带数字、会减少的量。那才是死线的形状，而「倒计时」这个词
/// 一次都没出现。
///
/// 判据是**数字紧跟着单位**，不是单看单位词。单看「小时」会把
/// 「过了几个小时」这种闲笔也算进来，而那不是发动机。
fn has_counted_unit(text: &str) -> bool {
    const UNITS: [&str; 8] = ["小时", "分钟", "秒", "天", "电量", "续航", "剩余", "存量"];
    let bytes: Vec<char> = text.chars().collect();
    for (i, c) in bytes.iter().enumerate() {
        if !c.is_ascii_digit() {
            continue;
        }
        // 跳过这一串数字（含小数点）
        let mut j = i + 1;
        while j < bytes.len() && (bytes[j].is_ascii_digit() || bytes[j] == '.') {
            j += 1;
        }
        // **数字和单位之间可能有空格** —— 「231.4 小时」是中文排版惯例，
        // 贴着数字末尾去比会整条漏掉（实跑第一章时就是这么漏的）。
        while j < bytes.len() && bytes[j].is_whitespace() {
            j += 1;
        }
        let tail: String = bytes[j..bytes.len().min(j + 4)].iter().collect();
        if UNITS.iter().any(|u| tail.starts_with(u)) {
            return true;
        }
    }
    false
}

/// 输入里真的有哪些发动机。
pub fn detect(text: &str) -> Vec<Engine> {
    let chars = text.chars().count().max(1);
    let mut out = Vec::new();
    for e in ALL_ENGINES {
        let hit = match e {
            Engine::DialogueHeavy => {
                let quotes = text.matches('「').count() + text.matches('“').count();
                (quotes as f64) / (chars as f64 / 1000.0) >= DIALOGUE_PER_1K
            }
            Engine::Deadline => {
                e.words().iter().any(|w| text.contains(w)) || has_counted_unit(text)
            }
            _ => e.words().iter().any(|w| text.contains(w)),
        };
        if hit {
            out.push(e);
        }
    }
    out
}

/// 不管写什么故事都要遵守的那几节。
///
/// **就两节**：一节管「这场戏凭什么留着」，一节管「台词是行动不是解释」。
/// 这两条是所有短剧剧本最常见的两种失败，而且和题材无关。
fn core_sections() -> Vec<Section> {
    let mut out = Vec::new();
    for (r, prefix) in [(&SCRIPT_CRAFT, "2."), (&DIALOGUE_CRAFT, "3.")] {
        if let Some(s) = split_sections(r).into_iter().find(|s| s.heading.starts_with(prefix)) {
            out.push(s);
        }
    }
    out
}

/// 手艺段的字数上限。
///
/// 超了就停，不截断某一节 —— 半节手艺比没有更糟：它会给出一个
/// 看起来完整、实际缺了前提的规则。
pub const CRAFT_BUDGET_CHARS: usize = 7000;

/// 拼出要塞进提示词的那一段。空串 = 一节都没挑上（不该发生，核心两节
/// 总在，但文件被改坏时要能退化成「什么都不加」而不是 panic）。
pub fn craft_block(text: &str) -> String {
    let mut picked = core_sections();
    for e in detect(text) {
        let (r, prefix) = e.section_of();
        let found = split_sections(r)
            .into_iter()
            .find(|s| prefix.is_empty() || s.heading.starts_with(prefix));
        if let Some(s) = found {
            if !picked.iter().any(|p| p.file == s.file && p.heading == s.heading) {
                picked.push(s);
            }
        }
    }

    let mut used = 0usize;
    let mut parts: Vec<String> = Vec::new();
    for s in picked {
        let n = s.text.chars().count();
        if used + n > CRAFT_BUDGET_CHARS {
            continue;
        }
        used += n;
        parts.push(format!("<!-- {} -->\n{}", s.file, s.text.trim_end()));
    }
    if parts.is_empty() {
        return String::new();
    }
    format!(
        "以下是编剧手艺参考（来自 drama-skills，MIT）。\
         **它只帮你实现题面已经给出的东西，不要反过来补造原文里没有的\
         精确数字、记录、资源、程序或机关。**\n\n{}",
        parts.join("\n\n---\n\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 参考切得出小节而且不含目录() {
        let s = split_sections(&SCRIPT_CRAFT);
        assert!(s.len() > 8, "切出来太少：{}", s.len());
        assert!(!s.iter().any(|x| x.heading.starts_with("目录")), "目录不是手艺");
        // 每一节都带着自己的标题行，否则塞进提示词里认不出在讲什么
        for x in &s {
            assert!(x.text.starts_with("## "), "「{}」丢了标题行", x.heading);
        }
    }

    /// 子节不单独拎 —— 拎出来会丢掉「这一条在讲什么场合」。
    #[test]
    fn 只切顶层不切子节() {
        let s = split_sections(&SCRIPT_CRAFT);
        assert!(!s.iter().any(|x| x.heading.starts_with("3.1")), "子节被当成顶层切了");
        let engine = s.iter().find(|x| x.heading.starts_with("3.")).unwrap();
        assert!(engine.text.contains("### 3.1"), "子节应该留在父节里");
    }

    #[test]
    fn 核心那两节总在() {
        let c = core_sections();
        assert_eq!(c.len(), 2, "{:?}", c.iter().map(|x| &x.heading).collect::<Vec<_>>());
        assert!(c.iter().any(|x| x.file == "script-craft.md"));
        assert!(c.iter().any(|x| x.file == "dialogue-craft.md"));
    }

    mod 认发动机 {
        use super::*;

        #[test]
        fn 认得出艰难选择() {
            assert!(detect("他只能选一个：救妹妹，还是交出名单。").contains(&Engine::Choice));
        }

        #[test]
        fn 认得出倒计时() {
            assert!(detect("离最后期限还有二十分钟。").contains(&Engine::Deadline));
            assert!(detect("三小时内必须把钱凑齐，来不及了。").contains(&Engine::Deadline));
        }

        /// **实跑漏过一次。**
        ///
        /// 《第01章-电量还剩多少》里写的是「剩余续航 231.4 小时」，
        /// 一个带数字、会减少的量 —— 而「倒计时」这三个字一次都没出现。
        /// 方案那一步自己认出了「精确死线」，我们的检测器没认出来。
        #[test]
        fn 带数字的读数也算死线() {
            // **数字和单位之间有空格** —— 中文排版惯例，贴着数字比会整条漏掉
            assert!(detect("剩余续航 231.4 小时。").contains(&Engine::Deadline));
            assert!(detect("剩余续航231.4小时。").contains(&Engine::Deadline));
            assert!(detect("比昨天少了 6.8 小时。").contains(&Engine::Deadline));
            assert!(detect("电量还剩 12%，撑不到天亮。").contains(&Engine::Deadline));
        }

        /// **判据是数字紧跟单位，不是单看单位词。**
        /// 单看「小时」会把「过了几个小时」这种闲笔也算成发动机。
        #[test]
        fn 没有数字的时间词不算死线() {
            assert!(!detect("过了几个小时，天慢慢亮了。").contains(&Engine::Deadline));
            assert!(!detect("她在那里坐了很久，久到忘了时间。").contains(&Engine::Deadline));
        }

        #[test]
        fn 认得出有限单位() {
            assert!(detect("只剩最后一次机会。").contains(&Engine::LimitedUnit));
        }

        #[test]
        fn 认得出关键物证() {
            assert!(detect("她把那支录音笔推到桌子中间。").contains(&Engine::Evidence));
        }

        /// 对白密集不靠词表 —— 靠引号密度。
        #[test]
        fn 对白密集看引号密度() {
            let 密 = "「你来了。」「嗯。」「坐。」「不坐。」「那就站着说。」「说什么？」".repeat(3);
            assert!(detect(&密).contains(&Engine::DialogueHeavy), "{}", 密.chars().count());
            let 疏 = format!("{}「就一句。」", "他沿着河堤一直走，天色慢慢暗下来。".repeat(40));
            assert!(!detect(&疏).contains(&Engine::DialogueHeavy));
        }

        /// **宁可漏，不可滥。**
        ///
        /// 漏了只是少给一节手艺；滥了会让模型去补造原文里根本没有的机关 ——
        /// 上游 SKILL.md 的原话：「不能反过来补造题面没有的精确数字、
        /// 记录、资源、程序、关系或机关」。那种失败看起来像「模型自己加戏」。
        #[test]
        fn 平淡的原文一个发动机都不认() {
            let 平 = "午后的阳光落在木地板上。她泡了茶，坐下来，翻开那本旧相册。\
                      窗外有孩子在喊，声音远远的。她想起很多年前的一个下午。";
            assert!(detect(平).is_empty(), "认多了：{:?}", detect(平));
        }

        /// 心理活动拍不出来，必须换成可拍的实现 —— 而换法不止一种。
        #[test]
        fn 认得出心理活动() {
            assert!(detect("她意识到自己其实一直都知道答案。").contains(&Engine::Interiority));
        }

        #[test]
        fn 空输入不炸() {
            assert!(detect("").is_empty());
        }
    }

    #[test]
    fn 没有发动机时只给核心两节() {
        let 平 = "午后的阳光落在木地板上。她泡了茶，坐下来，翻开那本旧相册。";
        let block = craft_block(平);
        assert!(block.contains("script-craft.md"));
        assert!(block.contains("dialogue-craft.md"));
        // 倒计时那一节不该出现 —— 原文里没有死线
        assert!(!block.contains("倒计时先算动作"), "给了原文里没有的机关");
    }

    #[test]
    fn 有倒计时时才加那一节() {
        let block = craft_block("离最后期限还有二十分钟，他还没找到钥匙。");
        assert!(block.contains("6."), "没加上节奏/有限单位那一节");
    }

    /// **要带上「不要补造」那句话。**
    ///
    /// 手艺参考本身是「怎么写好」，而模型很容易把它读成「应该有这些东西」。
    /// 这句话是上游 SKILL.md 里最要紧的一条约束。
    #[test]
    fn 手艺段带着不要补造的告诫() {
        let block = craft_block("他只能选一个。");
        assert!(block.contains("不要反过来补造"), "{block}");
        assert!(block.contains("drama-skills"), "要说清这是谁的东西");
    }

    /// 超预算就停，**不截断某一节** —— 半节手艺比没有更糟，
    /// 它会给出一个看起来完整、实际缺了前提的规则。
    #[test]
    fn 不超预算而且不切半节() {
        // 六个发动机全命中
        let 全 = "他只能选一个。最后期限还有二十分钟。只剩最后一次机会。\
                  录音笔在桌上。脚步声由远及近。\
                  「你来了。」「嗯。」「坐。」「不坐。」「说吧。」「说什么？」";
        let block = craft_block(全);
        assert!(block.chars().count() <= CRAFT_BUDGET_CHARS + 400, "超了：{}", block.chars().count());
        // 每一段都从标题行开始 = 没有被从中间切开
        for part in block.split("\n---\n\n").skip(1) {
            assert!(part.contains("## "), "有一节被切断了");
        }
    }

    /// 每条参考都要真的有路径被读到 —— **收进来没人读的文件
    /// 就是「看着有实际没有」**，这一轮已经清过好几个。
    #[test]
    fn 每份收进来的参考都有触发条件() {
        use std::collections::HashSet;
        let reachable: HashSet<&str> =
            ALL_ENGINES.iter().map(|e| e.section_of().0.file).collect();
        for f in ["script-craft.md", "dialogue-craft.md", "scene-sound-dramaturgy.md", "substitutable-realization.md"] {
            assert!(reachable.contains(f) || core_sections().iter().any(|s| s.file == f),
                    "{f} 收进来了但没有任何路径会读它");
        }
    }

    #[test]
    fn 同一节不会被加两次() {
        // 选择和有限单位指向同一节
        let block = craft_block("他只能选一个，而且只剩最后一次机会。");
        let engine_marks = block.matches("<!-- script-craft.md -->").count();
        assert!(engine_marks <= 3, "重复塞了同一份：{engine_marks}");
    }
}
