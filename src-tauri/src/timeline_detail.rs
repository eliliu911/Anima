//! **时间线细化**：一镜一次调用，把这一镜从第 0 秒到最后一秒每一刻发生什么写满。
//!
//! # 为什么有这一步
//!
//! 用户 2026-09-27：「同样 12 秒的视频，不拆分成 4 个。而是把时间线写好，描述细致，而不是一笔带过。」
//! 依据是 动作复刻示例那几版：参考视频不变、只改描述细度，越细出来越准。
//!
//! 量出来的差距（每秒多少字）：《回收师》EP01 画面这一栏中位 15、最稀 5；
//! Kotte 母版（画面里连带写了机位和站位）中位 42、最稀 27。EP01 第 27 镜 12 秒只有一句话。
//! 这里的密度算画面 + 站位 + 运镜，**不算约束** —— 约束每段原样重复，不是细节（见 `prompt_route_b::source_chars`）。
//!
//! # 为什么不塞进镜头表生成
//!
//! 镜头表是**一场一次调用**，输出上限 16384 token；第 6 场 11 镜现在已经约 2 万字符 JSON，
//! 描述翻倍会被截断，整场解析失败。所以镜头表照旧定结构，这一步按镜补细节，失败只重试这一镜。
//!
//! # 只动时间段
//!
//! 时长、景别、机位、运镜、职责、台词、出场人物都不改 —— 写回时只换 `segments`，
//! 而且写回前核对这一镜的时间段在细化期间没被别人改过（改过就不写，不覆盖用户的手改）。

use serde::Deserialize;
use std::future::Future;
use std::path::Path;

use crate::cinema::StoryboardShot;
use crate::error::{Error, Result};
use crate::shot_table::ShotSegment;
use crate::storyboard_gen::AssetRef;

/// 目标密度（字/秒）。Kotte 母版的中位数是 42。
pub const TARGET_CPS: f32 = 40.0;
/// 低于这个算「一笔带过」。前端的提醒用同一个数（`timelineDensity.ts`）。
pub const THIN_CPS: f32 = 20.0;
/// 高于这个算写得太满（软提示，让模型删）。
///
/// 2026-09-27 第 3 镜第一次实跑写到每秒 87 字：一半是形容词和每段重复的交代。
/// Kotte 验证过有用的是 40 字左右那一档，写到它的一倍半为止。
pub const MAX_CPS: f32 = TARGET_CPS * 1.6;
/// 一镜最多重试几次（一共调用 `RETRIES + 1` 次）。
pub const RETRIES: usize = 2;

/// 一段写了多少字：画面 + 站位 + 运镜，不算约束和空白。
pub fn segment_chars(seg: &ShotSegment) -> usize {
    crate::prompt_route_b::source_chars(std::slice::from_ref(seg))
}

/// 整镜每秒多少字。
pub fn density(segments: &[ShotSegment], duration: f32) -> f32 {
    if duration <= 0.0 {
        return 0.0;
    }
    crate::prompt_route_b::source_chars(segments) as f32 / duration
}

/// 细化之后至少要到多密（字/秒）才算「写细了」—— 进反馈、让模型重写，但**不是**不写回的门槛。
///
/// 本来就够细的（≥ 目标九成）只要求别变薄；其余按现有的 1.3 倍、至少到「不算一笔带过」，
/// 封顶目标的八成 —— 一次从 9 字/秒写到 40 字/秒不现实，也不需要。
pub fn required_density(old: f32) -> f32 {
    if old >= TARGET_CPS * 0.9 {
        old * 0.95
    } else {
        (old * 1.3).clamp(THIN_CPS, TARGET_CPS * 0.8)
    }
}

/// 给模型看的这一镜：结构（不许改的那些）+ 剧本原文 + 人物道具 + 台词。
pub fn shot_brief(shot: &StoryboardShot, assets: &[AssetRef]) -> String {
    use crate::assets::AssetKind;
    let mut lines = Vec::new();
    let d = |g: &str, v: &str| crate::shot_language::display(g, v);
    lines.push(format!("时长：{} 秒（同一条视频）", fmt_secs(shot.duration)));
    lines.push(format!(
        "景别 / 机位 / 运镜：{} · {} · {}{}",
        d("shotSize", &shot.scale),
        d("angle", &shot.camera_angle),
        d("move", &shot.camera_movement),
        match shot.camera_prompt_note() {
            n if n.is_empty() => String::new(),
            n => format!("（{n}）"),
        }
    ));
    if shot.one_take {
        lines.push("这一镜是一镜出：原来时间段里写了切镜的，保留那个时刻和写法；不要新增切镜。".into());
    } else {
        lines.push("一镜之内不切镜：不许写「切至」「切回」「切到」。".into());
    }
    if !shot.duty.trim().is_empty() {
        lines.push(format!("这一镜的职责：{}", shot.duty.trim()));
    }
    if !shot.source.trim().is_empty() {
        lines.push(format!("剧本原文（这一镜拍的就是这些）：{}", shot.source.trim()));
    }
    if !shot.lighting.trim().is_empty() {
        lines.push(format!("光影：{}", shot.lighting.trim()));
    }
    // 这一场的固定布景：整场共用。人在场里的位置和灯的位置从这里推，换机位只换落在画面哪一边
    let stage = shot.stage.lines();
    if !stage.is_empty() {
        lines.push("这一场的固定布景（整场所有镜头共用；站位、朝向和光的方向都从这里推）：".into());
        lines.extend(stage.into_iter().map(|l| format!("- {l}")));
    }
    // **导演台摆过的以导演台为准**（用户 2026-09-28：「第三镜还没有做导演台，不知道导演台和细化有没有冲突」）。
    // 导演台写的是镜头级站位和机位绑定，不碰时间段；先细化、后摆导演台的话，两边说的位置可能不一样 ——
    // 摆完再细化一次，时间线就照导演台写
    if !shot.blocking.is_empty() {
        lines.push("导演台摆好的站位（以它为准：时间线里每个人在画面哪边、前后、朝向、体态都要和它一致）：".into());
        for b in &shot.blocking {
            let how = [b.zone.trim(), b.depth.trim(), b.facing.trim(), b.posture.trim()].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ");
            lines.push(format!("- {}：{how}", b.actor.trim()));
        }
    }
    if let Some(motion) = shot.director_camera.as_ref().map(|c| c.motion_note.trim()).filter(|m| !m.is_empty()) {
        lines.push(format!("导演台绑定的运镜（以它为准，camera 按它写）：{motion}"));
    }
    if !shot.scene_items.is_empty() {
        lines.push("场景里要交代的东西：".into());
        for it in &shot.scene_items {
            lines.push(format!("- {}：{}", it.name.trim(), it.detail.trim()));
        }
    }
    let used: Vec<&AssetRef> = {
        let mut ids = shot.all_character_ids();
        ids.extend(shot.scene_id.clone());
        ids.extend(shot.prop_ids.iter().cloned());
        assets.iter().filter(|a| ids.contains(&a.id)).collect()
    };
    for (kind, label) in [(AssetKind::Character, "出场人物"), (AssetKind::Scene, "场景"), (AssetKind::Prop, "道具")] {
        let of: Vec<&&AssetRef> = used.iter().filter(|a| a.kind == kind).collect();
        if of.is_empty() {
            continue;
        }
        lines.push(format!("{label}："));
        for a in of {
            lines.push(format!("- {}：{}", a.name, crate::prompt_input::short_description(a, 160)));
        }
    }
    let spoken: Vec<_> = shot.dialogue_lines.iter().filter(|l| !l.line.trim().is_empty()).collect();
    if spoken.is_empty() {
        lines.push("这一镜没有台词。".into());
    } else {
        lines.push("台词（已经在台词栏里，时间线只写谁在大约第几秒开口、说话时的样子；**不要把台词原文写进 visual 和 sound**）：".into());
        for (i, l) in spoken.iter().enumerate() {
            let how = [l.emotion.trim(), if l.off_screen { "画外音" } else { "" }].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("，");
            lines.push(format!("{}. {}{}：{}", i + 1, l.speaker.trim(), if how.is_empty() { String::new() } else { format!("（{how}）") }, l.line.trim()));
        }
    }
    lines.join("\n")
}

/// 现在的时间段，原样给模型（它要在这上面写细，不是从头编）。**带段号 `n`**：输出每段要用
/// `source_segments` 说明它细化的是原来哪一段，代码据此核对原动作一个不少、顺序不乱（见 [`conservation`]）。
pub fn segments_json(segments: &[ShotSegment]) -> String {
    let numbered: Vec<serde_json::Value> = segments
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let mut v = serde_json::to_value(s).unwrap_or_default();
            if let Some(o) = v.as_object_mut() {
                o.insert("n".into(), serde_json::json!(i + 1));
            }
            v
        })
        .collect();
    serde_json::to_string_pretty(&serde_json::json!({ "segments": numbered })).unwrap_or_default()
}

/// 「写到什么程度」那一句。
pub fn target_line(duration: f32, old: f32) -> String {
    let want = (TARGET_CPS * duration).round() as usize;
    let most = (MAX_CPS * duration).round() as usize;
    format!(
        "这一镜 {} 秒，画面 + 站位 + 运镜合计写到约 {want} 字（每秒约 {} 字），不超过 {most} 字；现在只有每秒约 {:.0} 字",
        fmt_secs(duration),
        TARGET_CPS as usize,
        old
    )
}

/// 上一镜怎么结束的：最后一段的画面。
pub fn prev_line(prev: Option<&StoryboardShot>) -> String {
    match prev.and_then(|p| p.segments.last().map(|s| (p.shot_num, s.visual.trim().to_string()))) {
        Some((n, v)) if !v.is_empty() => format!("第 {n} 镜最后是：{v}"),
        _ => "这是本集第一镜，或上一镜没有画面描述".into(),
    }
}

pub fn vars(shot: &StoryboardShot, assets: &[AssetRef], prev: Option<&StoryboardShot>, feedback: &str) -> Vec<(String, String)> {
    let old = density(&shot.segments, shot.duration);
    vec![
        ("SECS".into(), fmt_secs(shot.duration)),
        ("SHOT".into(), shot_brief(shot, assets)),
        ("SEGMENTS".into(), segments_json(&shot.segments)),
        ("PREV".into(), prev_line(prev)),
        // 拍法只有这一个来源：cinematic 的表演与执行原文（分工版，见 `cinematic` 模块注释）
        ("METHOD".into(), crate::cinematic::timeline_method()),
        ("TARGET".into(), target_line(shot.duration, old)),
        ("FEEDBACK".into(), feedback.to_string()),
    ]
}

/// 细化出来的一段 + 它细化的是原来哪几段（段号从 1 数）。
#[derive(Debug, Clone, Deserialize)]
pub struct Refined {
    #[serde(flatten)]
    pub seg: ShotSegment,
    #[serde(default)]
    pub source_segments: Vec<usize>,
}

#[derive(Deserialize)]
struct Out {
    segments: Vec<Refined>,
}

/// 解析模型输出：`{"segments":[...]}`，按起点排序，首尾差不到 0.05 秒的对齐，秒数取到 0.01。
///
/// 只动边界的格式，**不补内容** —— 缺段、断档由 [`problems`] 报出来重写。
pub fn parse(raw: &str) -> std::result::Result<Vec<Refined>, String> {
    let body = crate::script_library::strip_code_fence(raw);
    let start = body.find('{').ok_or("输出里没有 JSON")?;
    let end = body.rfind('}').ok_or("输出里的 JSON 没闭合")?;
    let out: Out = serde_json::from_str(&body[start..=end]).map_err(|e| format!("JSON 解析失败：{e}"))?;
    let mut segs = out.segments;
    if segs.is_empty() {
        return Err("segments 是空的".into());
    }
    let r = |v: f32| (v * 100.0).round() / 100.0;
    segs.sort_by(|a, b| a.seg.start.total_cmp(&b.seg.start));
    for s in &mut segs {
        s.seg.start = r(s.seg.start);
        s.seg.end = r(s.seg.end);
    }
    for i in 1..segs.len() {
        if (segs[i].seg.start - segs[i - 1].seg.end).abs() <= 0.05 {
            segs[i].seg.start = segs[i - 1].seg.end;
        }
    }
    Ok(segs)
}

/// 细化后的时间段和原段对得上的容差（秒）。
const SOURCE_TIME_TOLERANCE: f32 = 0.5;

/// **原动作守恒**（2026-09-28 审查：「细化器只验证字变多，没有验证原有事件、动作顺序和结束状态被保留」）。
///
/// 不引入新数据模型，靠模型回填的 `source_segments` 核对四件代码能判断的事，任何一条不过都**不写回**：
///
/// 1. 原来每一段都被用到（没有被删掉的动作）；
/// 2. 顺序不倒退（后一段引用的原段不早于前一段）；
/// 3. 时间对得上（细化段落在它引用的原段的时间范围里，差不超过半秒）；
/// 4. 原段里点了名的人和道具（`names`），在引用它的细化段里还在 —— 写了一大段和原动作无关的描写，这一条过不了。
///
/// 内容翻得对不对代码判断不了；这四条保证的是**原来的每一件事都还在、先后没乱**。
pub fn conservation(old: &[ShotSegment], new: &[Refined], names: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let n_old = old.len();
    let mut used = vec![false; n_old];
    let mut prev: Option<(usize, usize)> = None;
    for (k, r) in new.iter().enumerate() {
        let src: Vec<usize> = r.source_segments.iter().copied().filter(|n| (1..=n_old).contains(n)).collect();
        if src.len() != r.source_segments.len() {
            out.push(format!("细化后第 {} 段的 source_segments 里有不存在的原段号 —— 原来只有 1~{n_old} 段", k + 1));
        }
        if src.is_empty() {
            out.push(format!("细化后第 {} 段没写 source_segments —— 每一段都要写明它细化的是原来第几段", k + 1));
            continue;
        }
        let (lo, hi) = (*src.iter().min().unwrap(), *src.iter().max().unwrap());
        for &n in &src {
            used[n - 1] = true;
        }
        if let Some((plo, phi)) = prev {
            if lo < plo || hi < phi {
                out.push(format!("细化后第 {} 段引用了原第 {lo} 段，排在引用原第 {phi} 段的后面 —— 原来的先后顺序不能改", k + 1));
            }
        }
        prev = Some((lo, hi));
        let (s0, s1) = (old[lo - 1].start, old[hi - 1].end);
        if r.seg.start > s1 + SOURCE_TIME_TOLERANCE || r.seg.end < s0 - SOURCE_TIME_TOLERANCE {
            out.push(format!(
                "细化后第 {} 段（{}~{} 秒）说它来自原第 {lo}{} 段（{}~{} 秒），时间对不上",
                k + 1,
                fmt_secs(r.seg.start),
                fmt_secs(r.seg.end),
                if hi > lo { format!("~{hi}") } else { String::new() },
                fmt_secs(s0),
                fmt_secs(s1)
            ));
        }
    }
    for (i, seg) in old.iter().enumerate() {
        if !used[i] {
            out.push(format!("原来的第 {} 段（{}~{} 秒「{}」）没写进去 —— 原来的每一段都要保留", i + 1, fmt_secs(seg.start), fmt_secs(seg.end), seg.title.trim()));
            continue;
        }
        let was = format!("{}{}", seg.visual, seg.blocking);
        let now: String = new
            .iter()
            .filter(|r| r.source_segments.contains(&(i + 1)))
            .map(|r| format!("{}{}{}", r.seg.visual, r.seg.blocking, r.seg.camera))
            .collect();
        for name in names.iter().filter(|n| !n.trim().is_empty() && was.contains(n.as_str())) {
            if !now.contains(name.as_str()) {
                out.push(format!("原第 {} 段里的「{name}」，细化后引用这一段的段落里不见了 —— 原来写到的人和道具都要保留", i + 1));
            }
        }
    }
    out
}

/// 细化结果的问题。`blocking` 非空就不能写回；`soft` 进反馈重写，但重试用完照样写回。
#[derive(Debug, Default)]
pub struct Problems {
    pub blocking: Vec<String>,
    pub soft: Vec<String>,
}

/// 核对细化结果：时间段完整（镜头表同一套校验）、真的变细了、没有变薄。
pub fn problems(shot: &StoryboardShot, new: &[ShotSegment], known_assets: &std::collections::BTreeSet<String>) -> Problems {
    let mut out = Problems::default();
    let mut probe = shot.clone();
    probe.segments = new.to_vec();
    let asset_ids: Vec<&str> = Vec::new();
    let names: Vec<&str> = Vec::new();
    let view = crate::shot_table::ShotView {
        shot_num: probe.shot_num,
        duration: probe.duration,
        segments: &probe.segments,
        lines: probe.dialogue_lines.iter().map(|d| (d.speaker.as_str(), d.line.as_str(), d.off_screen)).collect(),
        asset_ids,
        character_names: names,
        ref_slots: &[],
        lighting: "",
        one_take: probe.one_take,
    };
    // 只要时间段、声音这两类 —— 时长 / 台词容量 / 资产那些和这一步无关，也不是它能改的
    for i in crate::shot_table::validate(&view, known_assets) {
        if i.field == "segments" || i.field == "sound" {
            out.blocking.push(i.message);
        }
    }
    let old = density(&shot.segments, shot.duration);
    let now = density(new, shot.duration);
    if now <= old * 1.05 {
        out.blocking.push(format!("细化之后每秒约 {now:.0} 字，原来每秒约 {old:.0} 字 —— 没有写细，要把每一刻看得见的动作、表情、视线、手、镜头写出来"));
    } else if now < required_density(old) {
        out.soft.push(format!("每秒约 {now:.0} 字，还偏薄 —— 写到每秒约 {:.0} 字：每个人每一刻的动作过程、表情、视线、手，镜头从哪到哪", required_density(old)));
    } else if now > MAX_CPS {
        // **逐段给数**：只说「压到每秒 64 字」模型改不动（它数不准字）。2026-09-29 cinematic 版第一次每秒 94、
        // 第二次 71 —— 告诉它哪几段超了、各压到多少，才改得动
        let over: Vec<String> = new
            .iter()
            .enumerate()
            .filter_map(|(i, seg)| {
                let cap = ((seg.end - seg.start) * MAX_CPS).round() as usize;
                let n = segment_chars(seg);
                (n > cap).then(|| format!("第 {} 段 {} 字 → {} 字以内", i + 1, n, cap))
            })
            .collect();
        out.soft.push(format!(
            "每秒约 {now:.0} 字，写得太满 —— 压到每秒 {} 字以内（{}）：删掉形容词堆砌和每段重复的交代，每一刻只留最要紧的两三个看得见的动作、表情、视线、手和镜头",
            MAX_CPS as usize,
            over.join("；")
        ));
    }
    for (i, seg) in new.iter().enumerate() {
        let len = seg.end - seg.start;
        if len >= 2.0 && (segment_chars(seg) as f32) / len < THIN_CPS {
            out.soft.push(format!("第 {} 段 {}~{} 秒只写了 {} 字 —— 一笔带过，把这段时间里的过程写出来", i + 1, fmt_secs(seg.start), fmt_secs(seg.end), segment_chars(seg)));
        }
    }
    out
}

/// 细化一镜：带着问题重写，最多 [`RETRIES`] 次。回 `(新的时间段, 给人看的提示)`。
///
/// 选哪一份：没有 blocking 的里面 soft 最少的，一样少取更细的。全都有 blocking 就报错、不写回。
/// 核对「原段里点了名的人和道具还在」用的名字：这一镜能用的人物、道具资产。
fn anchor_names(assets: &[AssetRef]) -> Vec<String> {
    use crate::assets::AssetKind;
    assets.iter().filter(|a| matches!(a.kind, AssetKind::Character | AssetKind::Prop)).map(|a| a.name.clone()).collect()
}

pub async fn refine<F, Fut>(
    shot: &StoryboardShot,
    assets: &[AssetRef],
    prev: Option<&StoryboardShot>,
    known_assets: &std::collections::BTreeSet<String>,
    call: &F,
) -> std::result::Result<(Vec<ShotSegment>, Vec<String>), String>
where
    F: Fn(crate::prompts::PromptSlot, Vec<(String, String)>) -> Fut,
    Fut: Future<Output = Result<String>>,
{
    let names = anchor_names(assets);
    let mut feedback = String::new();
    let mut best: Option<(Vec<ShotSegment>, Vec<String>)> = None;
    let mut last = Vec::new();
    for _ in 0..=RETRIES {
        let got = call(crate::prompts::PromptSlot::TimelineDetail, vars(shot, assets, prev, &feedback))
            .await
            .map_err(|e| format!("模型调用失败：{e}"))
            .and_then(|raw| parse(&raw).inspect_err(|e| tracing::warn!(shot = shot.shot_num, error = %e, raw = %raw, "时间线细化解析失败，模型原文附后")));
        match got {
            Ok(refined) => {
                let segs: Vec<ShotSegment> = refined.iter().map(|r| r.seg.clone()).collect();
                let mut p = problems(shot, &segs, known_assets);
                p.blocking.extend(conservation(&shot.segments, &refined, &names));
                if p.blocking.is_empty() {
                    // 问题一样多时挑**字数离目标最近**的一份。以前挑写得最多的 —— 对「写得太少」对，
                    // 对「写得太满」正好反了（09-29 cinematic 版每秒 94 字，几次都超，留下的是最满的那份）
                    let off = |s: &[ShotSegment]| (density(s, shot.duration) - TARGET_CPS).abs();
                    let better = best.as_ref().map_or(true, |(b, notes)| {
                        p.soft.len() < notes.len() || (p.soft.len() == notes.len() && off(&segs) < off(b))
                    });
                    if better {
                        best = Some((segs, p.soft.clone()));
                    }
                    if p.soft.is_empty() {
                        break;
                    }
                }
                last = p.blocking.iter().chain(p.soft.iter()).cloned().collect();
            }
            Err(e) => {
                // 被内容过滤拦了不重试（见 `prompts::is_content_filter`）
                let filtered = crate::prompts::is_content_filter(&e);
                last = vec![e];
                if filtered {
                    break;
                }
            }
        }
        feedback = format!("## 上次的问题，这次必须改掉\n{}", last.join("\n"));
    }
    best.ok_or_else(|| format!("第 {} 镜细化没通过检查（重试过 {RETRIES} 次）：{}", shot.shot_num, last.join("；")))
}

/// 写回：**只换 `segments`**，而且只在这一镜的时间段和开始细化时一样的情况下写（在写锁里核对）。
pub fn write_back(config_dir: &Path, project_id: &str, episode_id: &str, shot_id: &str, before: &[ShotSegment], after: Vec<ShotSegment>) -> Result<()> {
    crate::episode_lock::update(config_dir, project_id, episode_id, |ep| {
        let shot = ep.storyboard.iter_mut().find(|s| s.id == shot_id).ok_or_else(|| Error::config(format!("找不到分镜「{shot_id}」")))?;
        if shot.segments != before {
            return Err(Error::config("这一镜的时间段在细化期间被改过，没写入 —— 你的改动留着，要细化就再点一次"));
        }
        shot.segments = after;
        Ok(())
    })
}

fn fmt_secs(v: f32) -> String {
    let r = (v * 10.0).round() / 10.0;
    if r.fract() == 0.0 { format!("{}", r as i64) } else { format!("{r}") }
}

/// 细化选中的镜头。**后台任务**（同一集同时只能有一个），逐镜跑、可停，某一镜失败记下接着做。
#[tauri::command]
pub fn cinema_refine_timelines(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
    project_id: String,
    episode_id: String,
    shot_ids: Vec<String>,
) -> Result<crate::jobs::Job> {
    let config_dir = state.config_dir.clone();
    let ep = crate::cinema::read_episode(&config_dir, &project_id, &episode_id)?;
    let mut ordered: Vec<&StoryboardShot> = ep.storyboard.iter().filter(|s| shot_ids.contains(&s.id)).collect();
    ordered.sort_by_key(|s| s.shot_num);
    if ordered.is_empty() {
        return Err(Error::config("没有要细化的镜头"));
    }
    let ids: Vec<String> = ordered.iter().map(|s| s.id.clone()).collect();
    let title = if ids.len() == 1 { format!("细化时间线 · 第 {} 镜", ordered[0].shot_num) } else { format!("细化时间线 · {} 镜", ids.len()) };
    let job_id = crate::jobs::start_exclusive(&config_dir, crate::jobs::JobKind::Timeline, &project_id, &episode_id, "", 0, &title)
        .map_err(|_| Error::config("这一集已经在细化时间线了 —— 进度在右下角和 QUEUE 里"))?;
    let started = crate::jobs::set_targets(&config_dir, &job_id, &ids).ok_or_else(|| Error::runtime("任务登记后读不回来"))?;
    crate::jobs::emit(Some(&app), Some(&started));
    let gateway = state.gateway.clone();
    let usage_scope = crate::token_usage::Scope::episode(&project_id, &episode_id);
    tauri::async_runtime::spawn(usage_scope.run(async move {
        let dir = config_dir.clone();
        let loaded = tauri::async_runtime::spawn_blocking(move || crate::llm_memory::ensure_background_loaded(&dir)).await.unwrap_or_else(|e| Err(e.to_string()));
        if let Err(e) = loaded {
            let done = crate::jobs::finish_with(&config_dir, &job_id, None, None, &[], Some(&e));
            crate::jobs::emit(Some(&app), done.as_ref());
            return;
        }
        let cfg = crate::prompts::load_for_project(&config_dir, &project_id);
        let call = |slot: crate::prompts::PromptSlot, vars: Vec<(String, String)>| {
            let gateway = gateway.clone();
            let cfg = cfg.clone();
            async move {
                let vars: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                crate::prompts::run(&gateway, &cfg, slot, &vars).await
            }
        };
        let assets = crate::storyboard_gen::load_assets(&config_dir, &project_id, &episode_id);
        let known: std::collections::BTreeSet<String> = assets.iter().map(|a| a.id.clone()).collect();
        let total = ids.len();
        // notes = 没做成的（界面上当失败显示）；remarks = 做成了、有话说的（密度变化、还偏薄 / 太满）
        let (mut done, mut notes, mut remarks) = (0usize, Vec::new(), Vec::new());
        for (i, id) in ids.iter().enumerate() {
            if crate::jobs::cancel_requested(&config_dir, &job_id) {
                notes.push(format!("已停止，后面 {} 镜没有细化", total - i));
                break;
            }
            // **每一镜都重读**：前面几镜细化的几分钟里，用户可能改了后面的镜头
            let ep = match crate::cinema::read_episode(&config_dir, &project_id, &episode_id) {
                Ok(ep) => ep,
                Err(e) => {
                    notes.push(format!("读不出分集：{e}"));
                    break;
                }
            };
            let Some(shot) = ep.storyboard.iter().find(|s| &s.id == id) else {
                notes.push("有一镜在细化期间被删了".into());
                continue;
            };
            let job = crate::jobs::set_progress(&config_dir, &job_id, i, total, &format!("第 {} 镜", shot.shot_num));
            crate::jobs::emit(Some(&app), job.as_ref());
            let prev = ep.storyboard.iter().filter(|s| s.shot_num < shot.shot_num).max_by_key(|s| s.shot_num);
            let shot_assets = crate::looks::for_shot(&assets, shot);
            match refine(shot, &shot_assets, prev, &known, &call).await {
                Ok((segs, soft)) => {
                    let (before, after) = (density(&shot.segments, shot.duration), density(&segs, shot.duration));
                    match write_back(&config_dir, &project_id, &episode_id, id, &shot.segments, segs) {
                        Ok(()) => {
                            done += 1;
                            remarks.push(format!("第 {} 镜：每秒 {before:.0} → {after:.0} 字", shot.shot_num));
                            remarks.extend(soft.into_iter().map(|m| format!("第 {} 镜：{m}", shot.shot_num)));
                        }
                        Err(e) => notes.push(format!("第 {} 镜：{e}", shot.shot_num)),
                    }
                }
                Err(e) => notes.push(e),
            }
            let job = crate::jobs::set_progress(&config_dir, &job_id, i + 1, total, &format!("第 {} 镜", shot.shot_num));
            crate::jobs::emit(Some(&app), job.as_ref());
        }
        let summary = format!("细化时间线 {done}/{total} 镜 —— 细化过的要重新合成提示词");
        let finished = if done == 0 {
            crate::jobs::finish_with(&config_dir, &job_id, None, None, &notes, Some(&format!("{total} 镜一镜都没细化成：{}", notes.join("；"))))
        } else {
            crate::jobs::finish_with_remarks(&config_dir, &job_id, None, Some(&summary), &notes, &remarks, None)
        };
        crate::jobs::emit(Some(&app), finished.as_ref());
    }));
    Ok(started)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn seg(start: f32, end: f32, visual: &str) -> ShotSegment {
        ShotSegment { start, end, title: "t".into(), visual: visual.into(), ..Default::default() }
    }

    fn shot(segments: Vec<ShotSegment>) -> StoryboardShot {
        let mut s: StoryboardShot = serde_json::from_value(serde_json::json!({
            "id": "s27", "shot_num": 27, "scale": "medium", "camera_movement": "static", "dialogue": "",
            "prompt": "", "duration": 12.0, "status": "pending"
        }))
        .unwrap();
        s.segments = segments;
        s
    }

    /// EP01 第 27 镜那种：12 秒一句话
    fn thin() -> StoryboardShot {
        shot(vec![seg(0.0, 12.0, &"字".repeat(62))])
    }

    #[test]
    fn 密度按画面站位运镜约束算() {
        let s = ShotSegment { start: 0.0, end: 4.0, visual: "字 字".into(), blocking: "站".into(), camera: "推".into(), constraints: "保".into(), sound: "不算".into(), ..Default::default() };
        // 约束和声音不算：约束每段原样重复，不是细节
        assert_eq!(segment_chars(&s), 4);
        assert!((density(&[s], 4.0) - 1.0).abs() < 1e-6);
        assert_eq!(density(&[], 0.0), 0.0);
    }

    #[test]
    fn 要求写到多细() {
        // 很稀的：至少到「不算一笔带过」
        assert_eq!(required_density(5.2), THIN_CPS);
        // 中等的：1.3 倍，封顶目标八成
        assert!((required_density(20.0) - 26.0).abs() < 1e-4);
        assert!((required_density(30.0) - TARGET_CPS * 0.8).abs() < 1e-4);
        // 本来就够细的：别变薄就行
        assert!((required_density(42.0) - 39.9).abs() < 1e-4);
    }

    #[test]
    fn 解析排序对齐边界不补内容() {
        let raw = "```json\n{\"segments\":[{\"start\":6.0,\"end\":12.0,\"visual\":\"b\"},{\"start\":0.0,\"end\":5.97,\"visual\":\"a\"}]}\n```";
        let segs = parse(raw).unwrap();
        assert_eq!(segs.iter().map(|s| (s.seg.start, s.seg.end)).collect::<Vec<_>>(), vec![(0.0, 5.97), (5.97, 12.0)]);
        // 段号跟着回来；没写就是空的（守恒那一步会拦）
        let with_src = parse("{\"segments\":[{\"start\":0,\"end\":12,\"source_segments\":[1,2],\"visual\":\"a\"}]}").unwrap();
        assert_eq!(with_src[0].source_segments, vec![1, 2]);
        assert!(segs[0].source_segments.is_empty());
        assert!(parse("{\"segments\":[]}").is_err());
        assert!(parse("没有 JSON").is_err());
    }

    #[test]
    fn 写细了才算过() {
        let old = thin();
        let known = BTreeSet::new();
        // 同样一句话：没写细，不能写回
        let same = problems(&old, &old.segments, &known);
        assert!(same.blocking.iter().any(|m| m.contains("没有写细")), "{:?}", same.blocking);
        // 四段、每段写满：通过，没有软问题
        let rich: Vec<ShotSegment> = (0..4).map(|i| seg(i as f32 * 3.0, i as f32 * 3.0 + 3.0, &"字".repeat(100))).collect();
        let ok = problems(&old, &rich, &known);
        assert!(ok.blocking.is_empty() && ok.soft.is_empty(), "{ok:?}");
        // 写细了一点但还偏薄：能写回，带提示
        let bit: Vec<ShotSegment> = vec![seg(0.0, 6.0, &"字".repeat(80)), seg(6.0, 12.0, &"字".repeat(80))];
        let p = problems(&old, &bit, &known);
        assert!(p.blocking.is_empty(), "{:?}", p.blocking);
        assert!(p.soft.iter().any(|m| m.contains("偏薄")), "{:?}", p.soft);
        // 写得太满（第 3 镜第一次实跑每秒 87 字）：能写回，但让它删
        let stuffed: Vec<ShotSegment> = (0..4).map(|i| seg(i as f32 * 3.0, i as f32 * 3.0 + 3.0, &"字".repeat(260))).collect();
        let p = problems(&old, &stuffed, &known);
        assert!(p.blocking.is_empty(), "{:?}", p.blocking);
        assert!(p.soft.iter().any(|m| m.contains("太满")), "{:?}", p.soft);
        // 逐段给数：每段 3 秒上限 192 字，这里每段都超
        let msg = p.soft.iter().find(|m| m.contains("太满")).unwrap();
        assert!(msg.contains("第 1 段") && msg.contains("字以内"), "{msg}");
    }

    /// 问题一样多时挑离目标最近的一份，不是最满的 —— 写得太满时挑最满的等于越改越满（09-29）
    #[test]
    fn 同样有问题时挑字数离目标最近的() {
        let src = include_str!("timeline_detail.rs");
        let body = &src[src.find("pub async fn refine").unwrap()..];
        assert!(body.contains("(density(s, shot.duration) - TARGET_CPS).abs()") && body.contains("off(&segs) < off(b)"), "挑选又回到了「挑最满的」");
    }

    #[test]
    fn 时间段断开或没写到镜尾不能写回() {
        let old = thin();
        let known = BTreeSet::new();
        let gap = vec![seg(0.0, 5.0, &"字".repeat(200)), seg(6.0, 12.0, &"字".repeat(200))];
        assert!(problems(&old, &gap, &known).blocking.iter().any(|m| m.contains("断开")));
        let short = vec![seg(0.0, 10.0, &"字".repeat(500))];
        assert!(problems(&old, &short, &known).blocking.iter().any(|m| m.contains("对不上")));
        // 切镜词：一镜之内不切
        let cut = vec![seg(0.0, 6.0, &format!("{}切至特写", "字".repeat(200))), seg(6.0, 12.0, &"字".repeat(200))];
        assert!(problems(&old, &cut, &known).blocking.iter().any(|m| m.contains("切")));
    }

    #[test]
    fn 台词原文不许写进画面() {
        let mut old = thin();
        old.dialogue_lines = vec![crate::cinema::DialogueLine { speaker: "陆川".into(), line: "拆完就不疼了".into(), ..Default::default() }];
        let dup = vec![seg(0.0, 12.0, &format!("{}陆川说拆完就不疼了", "字".repeat(500)))];
        assert!(!problems(&old, &dup, &BTreeSet::new()).blocking.is_empty());
    }

    /// 重试：第一次没写细 → 带着问题重写 → 第二次写细了就停；全都不过就报错、不写回。
    #[tokio::test]
    async fn 重试到写细为止() {
        let old = thin();
        let known = BTreeSet::new();
        let calls = std::sync::Mutex::new(Vec::<String>::new());
        let rich = serde_json::json!({"segments": (0..4).map(|i| serde_json::json!({"start": i as f32 * 3.0, "end": i as f32 * 3.0 + 3.0, "source_segments": [1], "visual": "字".repeat(100)})).collect::<Vec<_>>()}).to_string();
        let answers = [serde_json::json!({"segments": [{"start": 0.0, "end": 12.0, "source_segments": [1], "visual": "字".repeat(62)}]}).to_string(), rich];
        let call = |_slot: crate::prompts::PromptSlot, vars: Vec<(String, String)>| {
            let fb = vars.iter().find(|(k, _)| k == "FEEDBACK").map(|(_, v)| v.clone()).unwrap_or_default();
            let n = { let mut c = calls.lock().unwrap(); c.push(fb); c.len() - 1 };
            let out = answers[n.min(1)].clone();
            async move { Ok(out) }
        };
        let (segs, soft) = refine(&old, &[], None, &known, &call).await.unwrap();
        assert_eq!(segs.len(), 4);
        assert!(soft.is_empty());
        let fb = calls.lock().unwrap().clone();
        assert_eq!(fb.len(), 2, "写细了就该停");
        assert!(fb[1].contains("没有写细"), "第二次要带着问题：{}", fb[1]);

        let never = |_s: crate::prompts::PromptSlot, _v: Vec<(String, String)>| async { Ok("{\"segments\":[{\"start\":0,\"end\":12,\"source_segments\":[1],\"visual\":\"一句\"}]}".to_string()) };
        let err = refine(&old, &[], None, &known, &never).await.unwrap_err();
        assert!(err.contains("第 27 镜") && err.contains("没有写细"), "{err}");
    }

    fn refined(start: f32, end: f32, src: &[usize], visual: &str) -> Refined {
        Refined { seg: seg(start, end, visual), source_segments: src.to_vec() }
    }

    /// 原来三段：陆川放下扳手 → 星澜睁眼 → 陆川后退
    fn three() -> Vec<ShotSegment> {
        vec![seg(0.0, 4.0, "陆川放下扳手"), seg(4.0, 8.0, "星澜睁眼看向陆川"), seg(8.0, 12.0, "陆川后退一步")]
    }
    fn names() -> Vec<String> {
        vec!["陆川".into(), "星澜".into(), "扳手".into()]
    }

    #[test]
    fn 原动作守恒_拆细合法() {
        let new = vec![
            refined(0.0, 2.0, &[1], "陆川右手握着扳手慢慢放低"),
            refined(2.0, 4.0, &[1], "扳手搁到台边"),
            refined(4.0, 8.0, &[2], "星澜睁眼，视线落到陆川脸上"),
            refined(8.0, 12.0, &[3], "陆川后退一步"),
        ];
        assert!(conservation(&three(), &new, &names()).is_empty(), "{:?}", conservation(&three(), &new, &names()));
    }

    /// 审查要的那条：写得又长又细、但和原动作无关 —— 必须不过
    #[test]
    fn 高密度但和原动作无关的描述不过() {
        let off = "一只猫从窗台跳下，窗外霓虹闪烁，雨水顺着玻璃流下".repeat(4);
        let new = vec![refined(0.0, 4.0, &[1], &off), refined(4.0, 8.0, &[2], &off), refined(8.0, 12.0, &[3], &off)];
        let p = conservation(&three(), &new, &names());
        assert!(p.iter().any(|m| m.contains("「陆川」")) && p.iter().any(|m| m.contains("「星澜」")) && p.iter().any(|m| m.contains("「扳手」")), "{p:?}");
    }

    #[test]
    fn 漏段_倒序_时间对不上_没写来源都不过() {
        // 漏了第 2 段（星澜睁眼）
        let missing = vec![refined(0.0, 6.0, &[1], "陆川放下扳手"), refined(6.0, 12.0, &[3], "陆川后退一步")];
        assert!(conservation(&three(), &missing, &names()).iter().any(|m| m.contains("原来的第 2 段")));
        // 先写睁眼再写放扳手
        let reversed = vec![refined(0.0, 4.0, &[2], "星澜睁眼看陆川"), refined(4.0, 8.0, &[1], "陆川放下扳手"), refined(8.0, 12.0, &[3], "陆川后退")];
        assert!(conservation(&three(), &reversed, &names()).iter().any(|m| m.contains("顺序")));
        // 说自己来自第 3 段（8~12 秒），却放在 0~4 秒
        let wrong_time = vec![refined(0.0, 4.0, &[1, 3], "陆川放下扳手后退"), refined(4.0, 8.0, &[2], "星澜睁眼看陆川"), refined(8.0, 12.0, &[3], "陆川后退")];
        let p = conservation(&three(), &wrong_time, &names());
        assert!(p.iter().any(|m| m.contains("顺序")), "{p:?}");
        let late = vec![refined(0.0, 4.0, &[1], "陆川放下扳手"), refined(4.0, 8.0, &[2], "星澜睁眼看陆川"), refined(9.0, 12.0, &[2], "星澜睁眼陆川"), refined(8.0, 9.0, &[3], "陆川后退")];
        assert!(conservation(&three(), &late, &names()).iter().any(|m| m.contains("时间对不上") || m.contains("顺序")));
        let unsourced = vec![refined(0.0, 12.0, &[], "陆川放下扳手，星澜睁眼看陆川，陆川后退")];
        let p = conservation(&three(), &unsourced, &names());
        assert!(p.iter().any(|m| m.contains("没写 source_segments")), "{p:?}");
        let bogus = vec![refined(0.0, 12.0, &[1, 2, 3, 9], "陆川放下扳手，星澜睁眼看陆川，陆川后退")];
        assert!(conservation(&three(), &bogus, &names()).iter().any(|m| m.contains("不存在的原段号")));
    }

    /// 给模型的时间段带段号
    #[test]
    fn 时间段带段号() {
        let j: serde_json::Value = serde_json::from_str(&segments_json(&three())).unwrap();
        assert_eq!(j["segments"][1]["n"], 2);
        assert_eq!(j["segments"][1]["visual"], "星澜睁眼看向陆川");
    }

    /// 写回只换时间段；细化期间被改过就不写。
    #[test]
    fn 写回前核对没被改过() {
        let dir = tempfile::tempdir().unwrap();
        let ep_dir = crate::cinema::cinema_projects_dir(dir.path()).join("p1/episodes/e1");
        std::fs::create_dir_all(&ep_dir).unwrap();
        let mut s = thin();
        s.id = "s1".into();
        s.duty = "职责不动".into();
        let ep = serde_json::json!({"id":"e1","episode_num":1,"title":"t","synopsis":"","script_text":"","storyboard_version":crate::cinema::STORYBOARD_VERSION,"storyboard":[s]});
        std::fs::write(ep_dir.join("episode.json"), ep.to_string()).unwrap();
        let rich: Vec<ShotSegment> = (0..4).map(|i| seg(i as f32 * 3.0, i as f32 * 3.0 + 3.0, "细")).collect();

        write_back(dir.path(), "p1", "e1", "s1", &s.segments, rich.clone()).unwrap();
        let got = crate::cinema::read_episode(dir.path(), "p1", "e1").unwrap();
        assert_eq!(got.storyboard[0].segments, rich);
        assert_eq!(got.storyboard[0].duty, "职责不动");

        // 现在盘上已经是 rich 了，拿旧的 before 再写 —— 当成被别人改过
        let err = write_back(dir.path(), "p1", "e1", "s1", &s.segments, vec![seg(0.0, 12.0, "x")]).unwrap_err();
        assert!(err.to_string().contains("被改过"), "{err}");
        assert_eq!(crate::cinema::read_episode(dir.path(), "p1", "e1").unwrap().storyboard[0].segments, rich);
    }

    #[test]
    fn 摆过导演台的_站位和运镜以导演台为准() {
        let mut s = thin();
        assert!(!shot_brief(&s, &[]).contains("导演台"), "没摆过就不提");
        s.blocking = vec![crate::cinema::StageSubject { actor: "陆川".into(), zone: "左".into(), depth: "中景".into(), facing: "面向右".into(), posture: "坐".into() }];
        s.director_camera = Some(serde_json::from_value(serde_json::json!({"name":"机位 1","camera":{},"motion_note":"向前推近约 1.2 米"})).unwrap());
        let b = shot_brief(&s, &[]);
        assert!(b.contains("导演台摆好的站位（以它为准") && b.contains("- 陆川：左 · 中景 · 面向右 · 坐"), "{b}");
        assert!(b.contains("导演台绑定的运镜（以它为准，camera 按它写）：向前推近约 1.2 米"), "{b}");
    }

    #[test]
    fn 这一场的固定布景进细化说明_没有就不提() {
        let mut s = thin();
        assert!(!shot_brief(&s, &[]).contains("固定布景"), "老镜头没有就不提");
        s.stage = crate::shot_table::SceneStage {
            facing: "从拆机间正门往里看".into(),
            lights: vec!["暖黄工作灯：拆机台上方，斜照台面".into()],
            axis: "陆川 ↔ 竹竿：机位在拆机台前方".into(),
            ..Default::default()
        };
        let b = shot_brief(&s, &[]);
        assert!(b.contains("这一场的固定布景（整场所有镜头共用"), "{b}");
        assert!(b.contains("- 方向基准：从拆机间正门往里看（下面的左右都按这个方向）"), "{b}");
        assert!(b.contains("- 光源（整场固定，换机位只换从哪一侧看）：暖黄工作灯：拆机台上方，斜照台面"), "{b}");
        assert!(b.contains("- 主关系与机位侧：陆川 ↔ 竹竿：机位在拆机台前方"), "{b}");
    }

    /// 细化这一步的拍法只有 cinematic 一个来源（2026-09-29 用户定的分工版）。
    /// 方法整块走 `{METHOD}`，不再在这一镜的说明里另外拼运镜库、表演库
    #[test]
    fn 细化说明不再夹别家的运镜库和表演库() {
        let mut s = thin();
        s.camera_movement = "push_in".into();
        s.dialogue_lines = vec![crate::cinema::DialogueLine { speaker: "陆川".into(), line: "你是谁".into(), emotion: "震惊，说到一半停住".into(), ..Default::default() }];
        let b = shot_brief(&s, &[]);
        assert!(!b.contains("cinematic-video-prompt-engineer") && !b.contains("写法参考"), "{b}");
        let v = vars(&s, &[], None, "");
        let method = &v.iter().find(|(k, _)| k == "METHOD").unwrap().1;
        assert!(method.starts_with(crate::cinematic::PREFACE) && method.contains("Dialogue as Expression Timeline"), "{method}");
    }

    #[test]
    fn 给模型的这一镜带上原文台词和不切镜() {
        let mut s = thin();
        s.source = "星澜歪头解释协议".into();
        s.duty = "交代协议".into();
        s.dialogue_lines = vec![crate::cinema::DialogueLine { speaker: "星澜".into(), line: "协议第一条".into(), emotion: "平静".into(), ..Default::default() }];
        let b = shot_brief(&s, &[]);
        assert!(b.contains("时长：12 秒") && b.contains("星澜歪头解释协议") && b.contains("交代协议"), "{b}");
        assert!(b.contains("1. 星澜（平静）：协议第一条") && b.contains("不要把台词原文写进 visual"), "{b}");
        assert!(b.contains("不许写「切至」"), "{b}");
        let v = vars(&s, &[], None, "");
        let keys: Vec<&str> = v.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(keys, ["SECS", "SHOT", "SEGMENTS", "PREV", "METHOD", "TARGET", "FEEDBACK"]);
        assert!(v[5].1.contains("约 480 字") && v[5].1.contains("不超过 768 字") && v[5].1.contains("每秒约 5 字"), "{}", v[5].1);
    }
}

#[cfg(test)]
mod live {
    /// **实跑，只读**：拿真实分集里的一镜细化一次，打印前后和耗时，**不写回**。
    ///
    /// `SHOT=3 cargo test --lib timeline_detail::live -- --ignored --nocapture`
    /// （`PROJECT` / `EPISODE` 默认《回收师》EP01）
    #[tokio::test]
    #[ignore = "要本机后台模型和真实项目"]
    async fn 细化实跑只读() {
        let config_dir = dirs::config_dir().unwrap().join("com.anima.desktop");
        let project = std::env::var("PROJECT").unwrap_or_else(|_| "drama_1790397900".into());
        let episode = std::env::var("EPISODE").unwrap_or_else(|_| "ep_01".into());
        let num: u32 = std::env::var("SHOT").ok().and_then(|v| v.parse().ok()).unwrap_or(3);
        let mut app_cfg = crate::config::AppConfig::load(&config_dir.join("config.json")).unwrap();
        // 换后端 / 模型跑（比较用，不改用户配置）：`ANIMA_LIVE_PROVIDER`、`ANIMA_LIVE_MODEL`
        if let Ok(p) = std::env::var("ANIMA_LIVE_PROVIDER") {
            app_cfg.background_provider = p;
        }
        match std::env::var("ANIMA_LIVE_MODEL") {
            Ok(m) => app_cfg.background_model = Some(m),
            Err(_) => crate::llm_memory::ensure_background_loaded(&config_dir).unwrap(),
        }
        println!("后台：{} · {:?}", app_cfg.background_provider, app_cfg.background_model);
        let gateway = crate::llm::gateway::Gateway::new(app_cfg).unwrap();
        let cfg = crate::prompts::load(&config_dir);
        // `SHOT_FILE`：从一份镜头数组 JSON 里取这一镜（拿实跑出来、还没写进项目的镜头表比较）
        let storyboard: Vec<crate::cinema::StoryboardShot> = match std::env::var("SHOT_FILE") {
            Ok(f) => serde_json::from_str(&std::fs::read_to_string(f).unwrap()).unwrap(),
            Err(_) => crate::cinema::read_episode(&config_dir, &project, &episode).unwrap().storyboard,
        };
        let shot = storyboard.iter().find(|s| s.shot_num == num).expect("没有这一镜");
        let prev = storyboard.iter().filter(|s| s.shot_num < num).max_by_key(|s| s.shot_num);
        let assets = crate::storyboard_gen::load_assets(&config_dir, &project, &episode);
        let known: std::collections::BTreeSet<String> = assets.iter().map(|a| a.id.clone()).collect();
        let call = |slot: crate::prompts::PromptSlot, vars: Vec<(String, String)>| {
            let (gateway, cfg) = (&gateway, &cfg);
            async move {
                let v: Vec<(&str, &str)> = vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
                crate::prompts::run(gateway, cfg, slot, &v).await
            }
        };
        let show = |segs: &[crate::shot_table::ShotSegment]| {
            for s in segs {
                println!("  {:>5.2}~{:<5.2} 「{}」({} 字)\n    画面：{}\n    站位：{}\n    运镜：{}\n    约束：{}  禁止：{}  声音：{}", s.start, s.end, s.title, super::segment_chars(s), s.visual, s.blocking, s.camera, s.constraints, s.forbid, s.sound);
            }
        };
        println!("== 第 {num} 镜 {} 秒，原来每秒 {:.1} 字", shot.duration, super::density(&shot.segments, shot.duration));
        show(&shot.segments);
        let t = std::time::Instant::now();
        let out = super::refine(shot, &crate::looks::for_shot(&assets, shot), prev, &known, &call).await;
        println!("== 用时 {:.0} 秒", t.elapsed().as_secs_f32());
        match out {
            Ok((segs, soft)) => {
                println!("== 细化后每秒 {:.1} 字，提示：{soft:?}", super::density(&segs, shot.duration));
                show(&segs);
                if let Ok(out) = std::env::var("ANIMA_LIVE_OUT") {
                    std::fs::write(out, serde_json::to_string_pretty(&segs).unwrap()).unwrap();
                }
            }
            Err(e) => println!("== 失败：{e}"),
        }
    }
}
