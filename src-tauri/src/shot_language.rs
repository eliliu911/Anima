//! 镜头语言 —— **唯一一份词表**。
//!
//! 词表在 `shared/shot-language.json`，编译期嵌进来。同一份字节还被
//! 前端 `import`、被 `scripts/gen-film-director-refs.py` 生成成
//! `skills/film-director/reference/*.md`。三边读同一个文件，
//! 所以不存在「改了一处、另两处还是旧的」。
//!
//! # 为什么值得单开一个模块
//!
//! 在这之前同一件事有三份：分镜卡片的 `scale`/`camera_movement`
//! （显示串**同时当 id 和提示词**，模型只收到「推镜头」四个字）、
//! 创作台 NSFW 选择器那十组、还有 `skills/` 下手维护的 markdown。
//! 三份的粒度、命名、取值都不一样，而它们互不相通 —— 所以一直没出事。
//! 只要「在分镜里也用那套镜头语言」这个需求一提，就必然出事。
//!
//! 设计和取舍见 `docs/PLAN_SHOT_LANGUAGE.md`。

use serde::{Deserialize, Serialize};

/// 词表原文。**路径写死是故意的** —— 路径断了直接编译失败，
/// 比运行期找不到文件好得多。
const RAW: &str = include_str!("../../shared/shot-language.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prompt {
    pub en: String,
    pub zh: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Term {
    pub id: String,
    pub name: String,
    pub prompt: Prompt,
    /// **静帧要不要用这条。** 运镜一律 false —— 静帧里没有「推」这回事。
    ///
    /// 以前这条规则散落在 `frame_subject` 的注释里，现在它是数据。
    pub still: bool,
    /// **运镜按物理分组**：`translate`（机器真的在动）/ `rotate`（原地转）/
    /// `lens`（机器不动，只有焦距或焦点变）/ `hybrid`（复合）/ `still`（静止）。
    ///
    /// # 为什么要分组
    ///
    /// 实测结论（OpenMontage 引的那篇论文）：现在的视频模型会把**平移、
    /// 旋转、纯镜头变化**混作一谈。分组之后提示词里能按组列出来，
    /// 并写明两条最容易混的界线：
    ///
    /// - **推镜头 ≠ 变焦推近**：前者机器往前走（透视变），后者只有焦距变。
    /// - **摇 ≠ 横移**：`pan` 是原地转，`truck` 是整机横着走。
    ///
    /// 只有运镜有这一栏，别的组是 `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// 这一档景别建议的画幅。只有景别有。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect: Option<String>,
    /// **画幅不对会实际出问题**，不是不够理想。
    ///
    /// `aspect` 是给单张出图用的建议（那时候画幅是自己挑的）。剧集的画幅由
    /// 项目定死，所以那边只在这一条为真时才提示 —— 否则 9:16 的竖屏短剧里
    /// 七档景别会响三档（大远景、远景、特写都「建议」别的画幅），
    /// 而竖屏剧用特写天经地义。**报太多等于没报**：用户会来问「这是什么玩意儿」。
    ///
    /// 加这个标记前先问一句：有实测吗？现在只有全景那一条有
    /// —— 1:1 下脸和手的像素预算不够，是手指畸形最常见的间接原因。
    #[serde(default, rename = "aspectHard")]
    pub aspect_hard: bool,
    /// 实测结论 / 注意事项。会被带进生成的 markdown。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// 只对这些引擎有意义。空 = 都适用。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub engines: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    pub name: String,
    pub items: Vec<Term>,
    /// 分组的显示名（只有运镜有）。界面按组分隔显示，不是一长条二十一项。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "groupNames")]
    pub group_names: Option<std::collections::BTreeMap<String, String>>,
}

/// 拼装用的固定段落。中英各一份 —— 按引擎选。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Assembly {
    pub sections: Vec<String>,
    pub consistency: Prompt,
    #[serde(rename = "outputRule")]
    pub output_rule: Prompt,
    #[serde(rename = "referenceSingle")]
    pub reference_single: Prompt,
    #[serde(rename = "referenceMulti")]
    pub reference_multi: Prompt,
    /// 每一段的标题，中英各一份。**标题只有这一个所有者** ——
    /// 上面那几句里不要再自带前缀。
    #[serde(default)]
    pub labels: std::collections::HashMap<String, Prompt>,
}

/// 编码器中文原生的引擎名单。**别名要写全** —— 见 JSON 里的 note。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChineseEngines {
    pub ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShotLanguage {
    pub groups: Vec<Group>,
    pub assembly: Assembly,
    #[serde(rename = "chineseEngines")]
    pub chinese_engines: ChineseEngines,
}

/// 解析一次，之后共用。
///
/// **解析失败直接 panic。** 它是编译期嵌进来的常量，解析不了说明
/// 词表本身写坏了 —— 那种情况下让应用起不来，比让每一次出图
/// 静默丢掉镜头语言好得多。
pub fn shot_language() -> &'static ShotLanguage {
    use std::sync::OnceLock;
    static CELL: OnceLock<ShotLanguage> = OnceLock::new();
    CELL.get_or_init(|| {
        serde_json::from_str(RAW).expect("shared/shot-language.json 解析失败 —— 词表写坏了")
    })
}

pub fn group(id: &str) -> Option<&'static Group> {
    shot_language().groups.iter().find(|g| g.id == id)
}

/// 按 id 找一条。**跨组找** —— id 在整张表里唯一（有测试钉着）。
pub fn term(id: &str) -> Option<&'static Term> {
    shot_language()
        .groups
        .iter()
        .flat_map(|g| g.items.iter())
        .find(|t| t.id == id)
}

/// 这条词在这个引擎上的提示词。
///
/// **中英按引擎选**：Z-Image 的编码器是 Qwen3-4B、Chroma 是 T5-XXL，
/// 中文原生；SDXL 的 CLIP 是 77 token 分块取平均，要英文关键词。
/// 选错了不报错，只是那一段基本不生效。
pub fn text_for<'a>(term: &'a Term, engine: &str) -> &'a str {
    if prefers_chinese(engine) {
        &term.prompt.zh
    } else {
        &term.prompt.en
    }
}

/// 这个引擎的编码器吃不吃中文。
///
/// 认不出的引擎**按英文算** —— 保守方向：英文在中文原生的编码器上
/// 也能用，反过来不成立。
///
/// **名单在词表里**（`chineseEngines.ids`），不在这儿 —— 前端也要判同一件事，
/// 名单写两份就会分叉，而分叉的表现是「界面显示中文、发出去的是英文」。
pub fn prefers_chinese(engine: &str) -> bool {
    let e = engine.trim().to_ascii_lowercase();
    shot_language().chinese_engines.ids.contains(&e)
}

/// 一段拼装好的提示词 —— **每段一行，带标题**。
///
/// # 为什么不是全部用逗号连成一句
///
/// 以前出片提示词长这样：
///
/// ```text
/// <风格 40 个英文词>, <画面 200 字>, 远景…, 镜头固定不动…, 角色说：…
/// ```
///
/// 全是逗号，模型分不清哪句说的是画面、哪句说的是镜头、哪句是台词。
/// 结构是从 DramaClaw 学的（句子是我们自己的，见
/// `docs/REF_DRAMACLAW_SHOTS.md`）。
///
/// 分段之后还多出三样以前完全没有的东西：**角色一致性**（不说的话
/// 模型会让人在一镜之内变脸）、**输出要求**（闪烁 / 变形 / 跳帧）、
/// 以及**多参考图时禁止拼贴**（不禁的话它会把几张图并排画出来）。
#[derive(Debug, Default, Clone)]
pub struct Segments {
    parts: Vec<(String, String)>,
}

impl Segments {
    /// 加一段。**内容为空就整段不加** —— 一个只有标题的空行
    /// 对模型是纯噪声，而它看起来像「这里本来该有东西」。
    pub fn push(&mut self, label: &str, body: impl AsRef<str>) {
        let body = body.as_ref().trim();
        if body.is_empty() || label.trim().is_empty() {
            return;
        }
        self.parts.push((label.trim().to_string(), body.to_string()));
    }

    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    /// 拼成最终字符串。段与段之间换行。
    pub fn render(&self) -> String {
        self.parts
            .iter()
            .map(|(l, b)| if l.is_empty() { b.clone() } else { format!("{l}：{b}") })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// 某一段的标题，按引擎选中英。认不出的段返回空串 —— 空标题的段不会被加进去。
pub fn label(id: &str, engine: &str) -> String {
    match shot_language().assembly.labels.get(id) {
        Some(p) => text_for_prompt(p, engine).to_string(),
        None => String::new(),
    }
}

/// 固定段落（角色一致性 / 输出要求 / 参考图）按引擎选中英。
pub fn text_for_prompt<'a>(p: &'a Prompt, engine: &str) -> &'a str {
    if prefers_chinese(engine) {
        &p.zh
    } else {
        &p.en
    }
}

/// 角色一致性那一句。**没有角色就不要这一段** ——
/// 「保持  的外观一致」比不写更糟。
pub fn consistency_line(names: &[String], engine: &str) -> String {
    let names: Vec<&str> = names.iter().map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    if names.is_empty() {
        return String::new();
    }
    let sep = if prefers_chinese(engine) { "、" } else { ", " };
    text_for_prompt(&shot_language().assembly.consistency, engine)
        .replace("{names}", &names.join(sep))
}

/// 参考图那一段。张数决定用哪一句 —— **多张时那句里有「不要拼贴」**，
/// 不禁的话模型会把几张图并排画成一张多格画面。
pub fn reference_line(count: usize, engine: &str) -> String {
    let a = &shot_language().assembly;
    match count {
        0 => String::new(),
        1 => text_for_prompt(&a.reference_single, engine).to_string(),
        _ => text_for_prompt(&a.reference_multi, engine).to_string(),
    }
}

/// 组内按 id 找。
///
/// **不用跨组的 [`term`]** —— 景别和运镜各存各的字段，拿运镜 id 去查景别
/// 应该查不到，而不是碰巧命中另一组里同名的一条。
pub fn term_in(group_id: &str, id: &str) -> Option<&'static Term> {
    group(group_id)?.items.iter().find(|t| t.id == id.trim())
}

/// 界面上显示什么。
///
/// **认不出的原样显示。** 那多半是旧数据里用户自己打的词
/// （见 [`migrate`]），显示成「大特写」是在骗人。
pub fn display(group_id: &str, value: &str) -> String {
    match term_in(group_id, value) {
        Some(t) => t.name.clone(),
        None => value.trim().to_string(),
    }
}

/// 提示词里写什么。认不出的原样发出去 —— 用户打的词本来就该进提示词。
pub fn prompt_text(group_id: &str, value: &str, engine: &str) -> String {
    match term_in(group_id, value) {
        Some(t) => text_for(t, engine).to_string(),
        None => value.trim().to_string(),
    }
}

/// 把旧数据里的**显示串**换成 id。
///
/// # 为什么需要它
///
/// 分镜卡片的 `scale` / `camera_movement` 一直存的是
/// `"中景 (Medium)"` 这样的串，而且这个串**同时充当 id 和提示词** ——
/// 模型收到的字面就是「中景 (Medium)」四个汉字加一个英文括号。
/// 换成 id 之后显示名、中文提示词、英文提示词各归各的。
///
/// # 认不出的**原样返回**
///
/// 这条是硬的。旧库里可能有用户手打的景别（「过肩半身」之类），
/// 强行归一到「中景」是**静默改掉用户明确设过的东西** ——
/// 界面上还显示得好好的，只是内容变了。宁可留一个不在词表里的值
/// （界面按原文显示、提示词按原文发），也不能悄悄换掉。
///
/// 这一点和 `normalize_scale` 不同：那个吃的是模型刚吐出来的东西，
/// 模型被明确要求只能用给定的几个词，兜底到中景是对的。
pub fn migrate(group_id: &str, raw: &str) -> String {
    let r = raw.trim();
    if r.is_empty() {
        return String::new();
    }
    let Some(g) = group(group_id) else {
        return r.to_string();
    };
    // 1. 已经是 id 了 —— 迁移要幂等，不然跑两遍就出事。
    //
    // 当前词表下这一步其实碰不到 —— 没有任何 id 撞上同组的显示名或别名
    // （`没有_id_撞上同组的显示名或别名` 钉着这条）。留着是因为那是**数据的性质，
    // 不是代码的性质**：以后加一条英文别名（比如 `("static", …)`）就会撞上，
    // 而那时的表现是「跑两遍迁移得到另一个 id」。
    if g.items.iter().any(|t| t.id == r) {
        return r.to_string();
    }
    // 2. 按显示名匹配。**长的先比** —— 「大特写」含着「特写」，
    //    短的先命中就会把大特写归成特写（旧代码里同一个坑）。
    let mut by_name: Vec<&Term> = g.items.iter().collect();
    by_name.sort_by_key(|t| std::cmp::Reverse(t.name.chars().count()));
    for t in &by_name {
        if r.contains(&t.name) {
            return t.id.clone();
        }
    }
    // 3. 旧词表里那几个和新名字对不上的写法
    for (k, id) in legacy_aliases(group_id) {
        if r.contains(k) {
            return (*id).to_string();
        }
    }
    // 4. 认不出 —— 原样留着
    r.to_string()
}

/// 旧库里出现过、但和新词表的显示名对不上的写法。
///
/// 只有这几条，因为旧词表本身只有 5 档景别 + 6 个运镜。
fn legacy_aliases(group_id: &str) -> &'static [(&'static str, &'static str)] {
    match group_id {
        // 旧的「全景 (Wide)」按名字就命中新的「全景」= full_body，不用别名。
        "shotSize" => &[("Wide", "full_body"), ("Medium", "medium"), ("ECU", "extreme_close_up")],
        "move" => &[
            // 旧的「摇镜头 (Pan)」没分左右。**映射到右摇是为了和现有出片一致** ——
            // ffmpeg 那条分支里 `contains("摇")` 走的就是从左往右扫。
            ("摇", "pan_right"),
            // 旧的「俯视 (Tilt Down)」名义上是运镜，实际用户拿它当机位用。
            // 这里按它自己的标签迁成 tilt_down；机位那一组是新加的，
            // 旧数据里没有对应的字段可以搬过去。
            ("俯视", "tilt_down"),
            ("跟", "follow"),
            ("推", "push_in"),
            ("拉", "pull_out"),
            ("Zoom In", "push_in"),
            ("Zoom Out", "pull_out"),
            ("Tracking", "follow"),
            ("Static", "static"),
        ],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 词表能解析且不是空的() {
        let sl = shot_language();
        assert!(sl.groups.len() >= 9, "组数不对：{}", sl.groups.len());
        let n: usize = sl.groups.iter().map(|g| g.items.len()).sum();
        assert!(n >= 60, "条目太少，词表可能被截断：{n}");
    }

    /// **id 必须全表唯一。** `term()` 是跨组找的，撞了就会随机命中一个 ——
    /// 而那表现是「选了 A 出来是 B」，极难查。
    #[test]
    fn id_全表唯一() {
        let mut seen = std::collections::HashSet::new();
        for g in &shot_language().groups {
            for t in &g.items {
                assert!(seen.insert(t.id.as_str()), "id 撞了：{}", t.id);
            }
        }
    }

    /// 中英**两份都不能空**。空的那一份在对应引擎上就是整段丢掉，不报错。
    #[test]
    fn 每条都有中英两份() {
        for g in &shot_language().groups {
            for t in &g.items {
                assert!(!t.prompt.en.trim().is_empty(), "{} 缺英文", t.id);
                assert!(!t.prompt.zh.trim().is_empty(), "{} 缺中文", t.id);
                assert!(!t.name.trim().is_empty(), "{} 缺显示名", t.id);
            }
        }
    }

    /// **运镜一律不进静帧。** 静帧里没有「推」这回事 ——
    /// 塞进去模型只会照着那几个字画点奇怪的东西。
    #[test]
    fn 运镜全部标记为不进静帧() {
        let moves = group("move").expect("没有运镜这一组");
        assert!(moves.items.len() >= 12, "运镜条数不对");
        for t in &moves.items {
            assert!(!t.still, "{} 被标成静帧可用了", t.id);
        }
    }

    /// **每条运镜都要标分组，而且分组名要能查到。**
    ///
    /// 漏标一条的后果不是排版难看：提示词按组列出来，没有组的那条
    /// **整条不会出现在给模型的清单里** —— 它在下拉框里选得到，
    /// 抽分镜时却永远不会被选中，而且不报错。
    #[test]
    fn 每条运镜都标了分组且分组名查得到() {
        let moves = group("move").expect("没有运镜这一组");
        let names = moves.group_names.as_ref().expect("运镜没有分组显示名");
        for t in &moves.items {
            let g = t.group.as_deref().unwrap_or_else(|| panic!("{} 没标分组", t.id));
            assert!(names.contains_key(g), "{} 的分组「{g}」没有显示名", t.id);
        }
        // **五组都要有人**。少一组说明某一类运动根本没词可用 ——
        // `lens` 那一组就是这么补上的：以前一条都没有，
        // 想要「机位不动、画面拉近」时只能错用推镜头。
        let used: std::collections::BTreeSet<&str> =
            moves.items.iter().filter_map(|t| t.group.as_deref()).collect();
        for g in ["translate", "rotate", "lens", "hybrid", "still"] {
            assert!(used.contains(g), "「{g}」这一组一条词都没有");
        }
    }

    /// **别的组不许有 `group`。** 它是运镜专用的物理分类，
    /// 景别/光位标上它只会让「按组列出来」那段渲染出莫名其妙的小标题。
    #[test]
    fn 只有运镜带分组() {
        for g in &shot_language().groups {
            if g.id == "move" {
                continue;
            }
            for t in &g.items {
                assert!(t.group.is_none(), "{}/{} 不该有分组", g.id, t.id);
            }
        }
    }

    /// **推镜头和变焦推近必须说清区别。**
    ///
    /// 这是这次分组要解决的核心混淆：前者机器往前走（透视变），
    /// 后者只有焦距变（透视不变）。两条的提示词长得一样的话，
    /// 分组就白分了 —— 模型收到的还是同一句话。
    #[test]
    fn 推镜头和变焦推近的提示词说得清区别() {
        let push = term_in("move", "push_in").expect("没有推镜头");
        let zoom = term_in("move", "zoom_in").expect("没有变焦推近");
        assert_eq!(push.group.as_deref(), Some("translate"));
        assert_eq!(zoom.group.as_deref(), Some("lens"));
        assert_ne!(push.prompt.zh, zoom.prompt.zh, "两条中文提示词一模一样");
        assert_ne!(push.prompt.en, zoom.prompt.en, "两条英文提示词一模一样");
        assert!(zoom.prompt.zh.contains("不动"), "变焦那条没说机器不动：{}", zoom.prompt.zh);
    }

    /// 「固定」的严格定义要写在它自己的说明里 —— 零移动、零变焦、零对焦变化。
    /// 不写的话「固定 + 轻微推近」这种自相矛盾的组合没人拦。
    #[test]
    fn 固定写明了是严格的() {
        let st = term_in("move", "static").expect("没有固定");
        let note = st.note.as_deref().unwrap_or("");
        assert!(note.contains("零移动") && note.contains("零变焦"), "固定的说明不够严格：{note}");
    }

    /// 除了运镜，其余组都该能进静帧 —— 景别、机位、光线都是静帧的事。
    #[test]
    fn 非运镜的组都能进静帧() {
        for g in &shot_language().groups {
            if g.id == "move" {
                continue;
            }
            for t in &g.items {
                assert!(t.still, "{}/{} 被标成静帧不可用", g.id, t.id);
            }
        }
    }

    /// **每一档景别都要有建议画幅。** 这条是让 `lens.md` 里那句实测
    /// （全身必须竖幅，否则脸和手的像素预算不够）第一次真正进链路的前提。
    #[test]
    fn 每档景别都带建议画幅() {
        let sizes = group("shotSize").expect("没有景别这一组");
        assert_eq!(sizes.items.len(), 7, "景别应该是 7 档");
        for t in &sizes.items {
            let a = t.aspect.as_deref().unwrap_or("");
            assert!(
                matches!(a, "portrait" | "landscape" | "square"),
                "{} 的建议画幅不对：{a:?}",
                t.id
            );
        }
        // 全身那一档必须是竖的 —— 这是那条实测的核心
        assert_eq!(
            term("full_body").and_then(|t| t.aspect.as_deref()),
            Some("portrait"),
            "全身没标竖画幅 —— lens.md 那条实测会失效"
        );
    }

    #[test]
    fn 按引擎选中英文() {
        let t = term("medium").expect("找不到中景");
        assert_eq!(text_for(t, "zimage"), t.prompt.zh);
        assert_eq!(text_for(t, "qwen21"), t.prompt.zh);
        assert_eq!(text_for(t, "sdxl"), t.prompt.en);
        // 认不出的按英文算 —— 英文在中文原生的编码器上也 work，反过来不行
        assert_eq!(text_for(t, "谁知道这是什么"), t.prompt.en);
    }

    /// 拼装用的固定段落也要中英双份，且带占位符。
    #[test]
    fn 固定段落齐全() {
        let a = &shot_language().assembly;
        assert!(a.consistency.zh.contains("{names}"), "一致性段少了占位符");
        assert!(a.consistency.en.contains("{names}"));
        for p in [&a.output_rule, &a.reference_single, &a.reference_multi] {
            assert!(!p.zh.trim().is_empty());
            assert!(!p.en.trim().is_empty());
        }
        // 多图那段必须明确禁止拼贴 —— 那是这条链路会踩的实际失败
        assert!(a.reference_multi.zh.contains("拼贴"));
    }

    /// **`sections` 必须和真实存在的组对得上。** 对不上的话那一段
    /// 静默不出现 —— 拼出来的提示词少一块，而且不报错。
    #[test]
    fn 拼装顺序里的每一段都真的存在() {
        for id in &shot_language().assembly.sections {
            assert!(group(id).is_some(), "拼装顺序里的「{id}」没有对应的组");
        }
    }

    // ---- 迁移 ----

    /// 旧的五档景别显示串都要落到对应的 id 上。
    /// **大特写在特写之前** —— 短名先命中会把大特写归成特写。
    #[test]
    fn 旧景别显示串迁移到_id() {
        for (old, want) in [
            ("大特写 (ECU)", "extreme_close_up"),
            ("特写 (Close-Up)", "close_up"),
            ("中景 (Medium)", "medium"),
            ("全景 (Wide)", "full_body"),
            ("远景 (ELS)", "wide"),
            // 只写中文名（模型经常这么吐）
            ("大特写", "extreme_close_up"),
            ("中景", "medium"),
        ] {
            assert_eq!(migrate("shotSize", old), want, "{old} 迁移错了");
        }
    }

    #[test]
    fn 旧运镜显示串迁移到_id() {
        for (old, want) in [
            ("推镜头 (Zoom In)", "push_in"),
            ("拉镜头 (Zoom Out)", "pull_out"),
            ("摇镜头 (Pan)", "pan_right"),
            ("固定 (Static)", "static"),
            ("跟拍 (Tracking)", "follow"),
            ("俯视 (Tilt Down)", "tilt_down"),
        ] {
            assert_eq!(migrate("move", old), want, "{old} 迁移错了");
        }
    }

    /// **认不出的原样留着。** 强行归一是静默改掉用户设过的东西 ——
    /// 界面看着正常，内容变了。这条比「全都能认出来」重要。
    #[test]
    fn 认不出的值原样返回() {
        for raw in ["过肩半身", "斜四十五度俯冲", "whip pan", "???"] {
            assert_eq!(migrate("shotSize", raw), raw, "景别把认不出的改掉了");
        }
        assert_eq!(migrate("move", "螺旋升天"), "螺旋升天");
        // 组名认不出时也不能改值
        assert_eq!(migrate("根本没这一组", "中景 (Medium)"), "中景 (Medium)");
    }

    /// **迁移必须幂等。** 存盘时机不确定，同一条数据会被迁移多次；
    /// 第二遍把 id 又当显示串去 contains 匹配，会得到别的 id。
    #[test]
    fn 迁移是幂等的() {
        for g in ["shotSize", "move"] {
            for t in &group(g).unwrap().items {
                let once = migrate(g, &t.id);
                assert_eq!(once, t.id, "{}/{} 第一遍就变了", g, t.id);
                assert_eq!(migrate(g, &once), t.id, "{}/{} 第二遍变了", g, t.id);
            }
        }
    }

    /// 每一条新词表的显示名自己也要能迁回自己 —— 界面上曾经存过显示名。
    #[test]
    fn 新词表的显示名也能迁回_id() {
        for g in ["shotSize", "move", "angle"] {
            for t in &group(g).unwrap().items {
                assert_eq!(migrate(g, &t.name), t.id, "{}/{} 显示名迁不回去", g, t.name);
            }
        }
    }

    /// 让上面那个「已经是 id」早返回**当前**碰不到的前提。
    ///
    /// 它一旦不成立，`migrate` 就不再幂等 —— 而失败的样子是
    /// 「存了一次没事，再存一次景别自己变了」。这条测试比那时候去查快得多。
    #[test]
    fn 没有_id_撞上同组的显示名或别名() {
        for gid in ["shotSize", "move", "angle"] {
            let g = group(gid).unwrap();
            for t in &g.items {
                for other in &g.items {
                    assert!(!t.id.contains(&other.name), "{} 撞上显示名 {}", t.id, other.name);
                }
                for (alias, _) in legacy_aliases(gid) {
                    assert!(!t.id.contains(alias), "{} 撞上别名 {alias}", t.id);
                }
            }
        }
    }

    /// **视频渲染链路认哪些别名，名单里就得有哪些。**
    ///
    /// `cinema.rs` 的分流写的是 `"minimax_h3" | "h3" | "ltx" | "ltx25" | "ltx2.5"`。
    /// 名单里漏一条，那个别名就静默走英文分支 —— H3 的编码器是 Qwen3-VL，
    /// 中文原生，发英文不报错，只是镜头语言那一段基本不生效。
    #[test]
    fn 渲染链路的引擎别名都在中文名单里() {
        // `krea2` 在这条里：Krea 2 的编码器是 Qwen3-VL，2026-09-08 实测中文原生
        // （同一段中文提示词元素全中、「三个青苹果」数量都对，Z-Image 反而出成
        // 四个）。漏掉它的表现和 H3 一样 —— 静默走英文分支，图照出、不报错。
        for e in ["minimax_h3", "h3", "ltx", "ltx25", "ltx2.5", "zimage", "krea2", "qwen21"] {
            assert!(prefers_chinese(e), "{e} 不在中文引擎名单里");
        }
        // 大小写和空白不该影响判断
        assert!(prefers_chinese("  MiniMax_H3 "));
        // 英文编码器的必须留在英文分支
        for e in ["sdxl", "wan", ""] {
            assert!(!prefers_chinese(e), "{e} 被当成中文引擎了");
        }
    }

    /// **一组的词不能钻进另一组。**
    ///
    /// 真出过：大特写那条的英文原本是 `macro close-up, eye level detail` ——
    /// `eye level` 是机位词。用户选了「仰拍 + 大特写」，两个词一起发出去，
    /// 模型两头都听一点，出来既不仰也不特。不报错。
    #[test]
    fn 机位词不许出现在别的组里() {
        let angle_words = ["eye level", "low angle", "high angle", "bird", "dutch", "over the shoulder"];
        for g in &shot_language().groups {
            if g.id == "angle" {
                continue;
            }
            for t in &g.items {
                let en = t.prompt.en.to_lowercase();
                for w in angle_words {
                    assert!(!en.contains(w), "{}/{} 的英文里有机位词 {w}：{}", g.id, t.id, t.prompt.en);
                }
            }
        }
    }

    /// 景别是**有序的一档一档**，英文不能重复 —— 两档发同一句话，
    /// 等于用户选了两个不同的档、模型收到同一个词。
    #[test]
    fn 景别的英文两两不同() {
        let sizes = group("shotSize").unwrap();
        for (i, a) in sizes.items.iter().enumerate() {
            for b in &sizes.items[i + 1..] {
                assert_ne!(a.prompt.en, b.prompt.en, "{} 和 {} 的英文一样", a.id, b.id);
                assert_ne!(a.prompt.zh, b.prompt.zh, "{} 和 {} 的中文一样", a.id, b.id);
            }
        }
    }

    /// **标题只有一个所有者。**
    ///
    /// 这四句以前自带「角色一致性：」这种前缀，而拼装时又会加一遍标题 ——
    /// 拼出来是「角色一致性：角色一致性：保持…」。
    #[test]
    fn 固定段落不许自带标题() {
        let a = &shot_language().assembly;
        for (key, p) in [
            ("consistency", &a.consistency),
            ("outputRule", &a.output_rule),
            ("referenceSingle", &a.reference_single),
            ("referenceMulti", &a.reference_multi),
        ] {
            for (lang, t) in [("zh", &p.zh), ("en", &p.en)] {
                // 标题是拼装时加的，句子自己不能再带
                let head: String = t.chars().take(24).collect();
                assert!(
                    !head.contains('：') && !head.contains(": "),
                    "{key}.{lang} 自带了标题：{t}"
                );
                // **这些字符串是直接发给模型的**，markdown 的 ** 会原样进提示词
                assert!(!t.contains("**"), "{key}.{lang} 里有 markdown 强调符：{t}");
            }
        }
        // 每一段都要有中英两份标题
        for id in ["style", "scene", "camera", "consistency", "reference", "output", "dialogue"] {
            let l = a.labels.get(id).unwrap_or_else(|| panic!("缺 {id} 的标题"));
            assert!(!l.zh.trim().is_empty() && !l.en.trim().is_empty(), "{id} 标题不全");
        }
    }

    // ---- 分段拼装 ----

    #[test]
    fn 分段拼出来每段一行带标题() {
        let mut seg = Segments::default();
        seg.push(&label("scene", "h3"), "一只猫扒着箱子");
        seg.push(&label("camera", "h3"), "中景，镜头缓慢推近");
        let out = seg.render();
        assert_eq!(out, "画面：一只猫扒着箱子\n镜头：中景，镜头缓慢推近");
        assert_eq!(out.lines().count(), 2);
    }

    /// **空内容整段不加。** 只有标题的空行对模型是纯噪声，
    /// 而它看起来像「这里本来该有东西」。
    #[test]
    fn 空内容不留一个光秃秃的标题() {
        let mut seg = Segments::default();
        seg.push(&label("scene", "h3"), "有内容");
        seg.push(&label("camera", "h3"), "   ");
        seg.push(&label("dialogue", "h3"), "");
        // 认不出的段 id → 标题为空 → 整段不加
        seg.push(&label("根本没这一段", "h3"), "有内容但没标题");
        assert_eq!(seg.render(), "画面：有内容");
    }

    #[test]
    fn 段落标题按引擎选中英() {
        assert_eq!(label("scene", "h3"), "画面");
        assert_eq!(label("scene", "sdxl"), "Scene");
        assert_eq!(label("consistency", "zimage"), "角色一致性");
        assert_eq!(label("consistency", "ltx"), "角色一致性");
    }

    /// **没有角色就不要这一段。**「保持  的外观一致」比不写更糟。
    #[test]
    fn 角色一致性没有名字时整段消失() {
        assert_eq!(consistency_line(&[], "h3"), "");
        assert_eq!(consistency_line(&["".into(), "   ".into()], "h3"), "");
    }

    #[test]
    fn 角色一致性把名字填进去() {
        let names = vec!["小花".to_string(), "大橘".to_string()];
        let zh = consistency_line(&names, "h3");
        assert!(zh.contains("小花、大橘"), "中文用顿号分隔：{zh}");
        assert!(!zh.contains("{names}"), "占位符没被替换：{zh}");
        let en = consistency_line(&names, "sdxl");
        assert!(en.contains("小花, 大橘"), "英文用逗号分隔：{en}");
        assert!(!en.contains("{names}"));
    }

    /// **多张参考图那一句里必须有「不要拼贴」。**
    /// 不禁的话模型会把几张图并排画成一张多格画面 —— 图出了、不报错。
    #[test]
    fn 参考图按张数换句子() {
        assert_eq!(reference_line(0, "h3"), "", "没有参考图就不该有这一段");
        let one = reference_line(1, "h3");
        let many = reference_line(4, "h3");
        assert_ne!(one, many, "单张和多张用了同一句");
        assert!(one.contains("不要把它锁成第一帧"), "{one}");
        assert!(many.contains("拼贴"), "多参考没禁拼贴：{many}");
        assert!(!one.contains("拼贴"), "单张不该提拼贴：{one}");
        // 英文那份同样要禁
        assert!(reference_line(4, "sdxl").to_lowercase().contains("collage"));
    }

    /// **硬约束必须带上理由。**
    ///
    /// 没有实测就不该标 —— 标了就会在界面上占一个位置，而占位置的提示
    /// 每多一条，人对全部提示的信任就少一分。
    #[test]
    fn 硬画幅约束都写了理由() {
        let sizes = group("shotSize").unwrap();
        let hard: Vec<&Term> = sizes.items.iter().filter(|t| t.aspect_hard).collect();
        assert!(!hard.is_empty(), "一条硬约束都没有？那提示永远不会出现");
        for t in &hard {
            assert!(t.aspect.is_some(), "{} 标了硬约束却没说要哪种画幅", t.id);
            let note = t.note.as_deref().unwrap_or("");
            assert!(!note.trim().is_empty(), "{} 标了硬约束却没写理由", t.id);
        }
        // 全景那一条是实测来的，必须在名单里
        assert!(term("full_body").unwrap().aspect_hard, "全景那条实测结论被摘掉了");
    }

    /// **绝大多数景别不该是硬约束。** 全标上等于每一镜都在报警。
    #[test]
    fn 硬约束是少数() {
        let sizes = group("shotSize").unwrap();
        let hard = sizes.items.iter().filter(|t| t.aspect_hard).count();
        assert!(
            hard * 2 < sizes.items.len(),
            "{hard}/{} 的景别标成了硬约束 —— 报太多等于没报",
            sizes.items.len()
        );
    }

    #[test]
    fn 空值迁移后还是空() {
        assert_eq!(migrate("shotSize", ""), "");
        assert_eq!(migrate("move", "   "), "");
    }

    /// 迁移出来的 id 必须真的在那一组里 —— 别名表里写错一个字，
    /// 结果是个查不到的 id，界面上那一段直接空掉。
    #[test]
    fn 别名表指向的_id_都存在() {
        for g in ["shotSize", "move"] {
            for (alias, id) in legacy_aliases(g) {
                assert!(
                    group(g).unwrap().items.iter().any(|t| t.id == *id),
                    "{g} 的别名 {alias} 指向不存在的 id：{id}"
                );
            }
        }
    }
}
