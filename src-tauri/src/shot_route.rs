//! **每一镜走哪条出片链路**：首帧出片（FL2VA，只吃首帧）还是参考出片（Ref2VA，吃人物 / 场景参考图）。
//!
//! 用户 2026-10-07 定的规则（「A 加检查」）：
//!
//! 1. **实际走哪条，看有没有首帧**：首帧 / 尾帧有图 → 首帧出片，参考图不发；删掉首帧 → 参考出片。界面、批量、合成都按这一份。
//! 2. **按分镜内容建议走哪条**（[`content_route`]）：有人中途进场、或者画面里的人多到首帧挂不下他们的参考图 → 参考出片；其余首帧出片。
//!    这个建议决定「按场出首帧」给哪几镜出图，也用来提示。
//! 3. **检查**（[`keyframe_blocker`]）：已经挂了首帧、却有人中途进场 —— 首帧里没有他，H3 只能猜他长什么样 —— 拦下不出片，
//!    提示删掉首帧走参考出片。
//! 4. **接上一镜自动设**（[`auto_continuity`]）：不再只写「推荐」。用户手动改过的那一镜不动（`continuity_manual`）。
//!
//! 为什么参考出片更适合多人中途进场：参考出片每个人都有自己的参考图（`<Picture N>`），长相锁得住；
//! 首帧出片长相只认首帧（FL2VA 分区不收资产图），首帧里没画进去的人只能靠文字猜。
//! 2026-10-07 EP03 第 2 镜（6 人）参考出片衣服人数都对。

use crate::cinema::{Continuity, OpeningAbsence, StoryboardShot};
use crate::storyboard_gen::AssetRef;

/// 出片链路
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShotRoute {
    /// 首帧出片（FL2VA）：只发首帧 / 尾帧
    Keyframe,
    /// 参考出片（Ref2VA）：发人物 / 场景 / 道具参考图
    Reference,
}

/// 首帧出片最多能把几个人画进首帧 —— 出首帧最多挂 4 张参考图（场景 1 张 + 人 3 个，`cinema::FIRST_FRAME_MAX_REFS`）
pub const KEYFRAME_MAX_PEOPLE: usize = crate::cinema::FIRST_FRAME_MAX_REFS - 1;

/// 按分镜内容的建议
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct RouteAdvice {
    pub route: ShotRoute,
    /// 为什么（界面上显示）
    pub reason: String,
    /// 中途进场的人（首帧里没有他们）
    pub late: Vec<String>,
    /// 开场就在画面里的人数（不算只出声的）
    pub on_screen: usize,
}

/// 这一镜画面里的人（名字）：出场名单里有资产的角色 + **没建资产、只按文字写的人**（`unlinked_assets` 里的角色）。
///
/// **没建资产的也要算**（2026-10-11 用户：「改规则把没建资产的人也算进去」）。以前只数有资产的：
/// EP05 第 16 镜领队带两名保安破门而入，规则只看到星澜、陆川两个人，建议首帧出片 ——
/// 首帧里没有他们，H3 只能靠文字猜，就是 EP03 第 10 镜片尾多出陌生人那种。一伙人（「保安队」）按一个算。
fn people(shot: &StoryboardShot, assets: &[AssetRef]) -> Vec<String> {
    let mut out: Vec<String> = shot.all_character_ids()
        .iter()
        .filter_map(|id| assets.iter().find(|a| &a.id == id && a.kind == crate::assets::AssetKind::Character))
        .map(|a| a.name.clone())
        .collect();
    for u in &shot.unlinked_assets {
        let name = u.name.trim();
        if u.kind == "character" && !name.is_empty() && !out.iter().any(|n| n == name) {
            out.push(name.to_string());
        }
    }
    out
}

/// 开场先拍的是身体某处 / 某样东西的特写（不是人脸），这些词出现在镜头说明的第一句里
const OPENING_PARTS: &[&str] = &["靴", "腿", "脚", "腰", "胸口", "铭牌", "泡面", "碗", "膝"];
/// 首帧之后画面内容会换掉的运镜（推近也算：从腰侧推到脸，脸是推上去才有的）
const REVEALING_MOVES: &[&str] = &["tilt_up", "tilt_down", "pan_left", "pan_right", "push_in"];
/// 摇镜摇到哪：后面跟着谁，谁就是摇过去才进画面的
const PAN_TO: &[&str] = &["摇向", "摇到", "摇至", "转到", "落到", "落在"];

/// **首帧拍不到人脸**：文字里写到了，首帧画面里却看不见（2026-10-11 用户：「改规则」）。
///
/// E05 有 8 镜是这样：从机械靴 / 泡面 / 胸口铭牌 / 腰侧特写起拍，再上摇或推近才露出脸；或者横摇过去才拍到圆婶、陆川。
/// 首帧出片的首帧就是第 0 秒那一格 —— 那时脸不在画面里，后面摇出来的脸 H3 只能猜，参考图又不送。按镜头说明（`camera_note`）判：
/// - 运镜是上下摇 / 横摇 / 推近，第一句从身体某处或某样东西拍起 → 回「开场先拍 X」；
/// - 横摇的说明里，摇到的目标后面写着某个人 → 回「X 是摇过去才拍到的」。
pub fn revealed_by_camera(shot: &StoryboardShot, people: &[String]) -> Option<String> {
    if !REVEALING_MOVES.contains(&shot.camera_movement.as_str()) {
        return None;
    }
    let note = shot.camera_note.as_str();
    let first = note.split(['，', '。', '；', ',', ';']).next().unwrap_or_default();
    if let Some(part) = OPENING_PARTS.iter().find(|p| first.contains(**p)) {
        return Some(format!("开场先拍{part}的特写、再{}才露出人脸 —— 首帧里看不到脸，要靠参考图锁长相", if shot.camera_movement == "push_in" { "推近" } else { "摇镜" }));
    }
    if shot.camera_movement.starts_with("pan_") {
        for kw in PAN_TO {
            if let Some(i) = note.find(kw) {
                let after = &note[i..];
                if let Some(who) = people.iter().find(|n| after.contains(n.as_str())) {
                    return Some(format!("{who}是镜头摇过去才拍到的 —— 首帧里没有这个人，要靠参考图锁长相"));
                }
            }
        }
    }
    None
}

/// 按分镜内容建议走哪条（见模块说明第 2 条）。镜头详情里手动选过的（`route_choice`）照选的来
pub fn content_route(shot: &StoryboardShot, assets: &[AssetRef]) -> RouteAdvice {
    let advice = content_route_auto(shot, assets);
    match shot.route_choice {
        Some(route) if route != advice.route => RouteAdvice {
            route,
            reason: format!("手动选了{}出片（按内容会建议{}：{}）", route_label(route), route_label(advice.route), advice.reason),
            ..advice
        },
        Some(_) => RouteAdvice { reason: format!("手动选了{}出片（和内容建议一致）", route_label(advice.route)), ..advice },
        None => advice,
    }
}

fn route_label(r: ShotRoute) -> &'static str {
    match r {
        ShotRoute::Keyframe => "首帧",
        ShotRoute::Reference => "参考",
    }
}

fn content_route_auto(shot: &StoryboardShot, assets: &[AssetRef]) -> RouteAdvice {
    let people = people(shot, assets);
    let late: Vec<String> = people
        .iter()
        .filter(|n| shot.absent_from_opening(n) == Some(OpeningAbsence::EntersLater))
        .cloned()
        .collect();
    let on_screen = people.iter().filter(|n| shot.absent_from_opening(n).is_none()).count();
    let (route, reason) = if !late.is_empty() {
        (ShotRoute::Reference, format!("{}中途进场，首帧里没有，要靠参考图锁长相", late.join("、")))
    } else if on_screen > KEYFRAME_MAX_PEOPLE {
        (ShotRoute::Reference, format!("开场就有 {on_screen} 个人，首帧最多挂 {KEYFRAME_MAX_PEOPLE} 个人的参考图"))
    } else if on_screen == 0 {
        (ShotRoute::Reference, "画面里没有人物，按场景 / 道具参考出".to_string())
    } else if let Some(why) = revealed_by_camera(shot, &people) {
        (ShotRoute::Reference, why)
    } else {
        (ShotRoute::Keyframe, format!("开场 {on_screen} 个人都在首帧里，中途没人进场，首帧定构图"))
    };
    RouteAdvice { route, reason, late, on_screen }
}

/// 实际走哪条（见模块说明第 1 条）。长镜头窗口不吃首帧，一律参考
pub fn actual_route(shot: &StoryboardShot, long_take: bool) -> ShotRoute {
    if shot.keyframes.drives_video() && !long_take { ShotRoute::Keyframe } else { ShotRoute::Reference }
}

/// 挂了首帧、却有人中途进场 → 不能出片（见模块说明第 3 条）。回拦下的原因
pub fn keyframe_blocker(shot: &StoryboardShot, assets: &[AssetRef], long_take: bool) -> Option<String> {
    if actual_route(shot, long_take) != ShotRoute::Keyframe {
        return None;
    }
    let advice = content_route(shot, assets);
    (!advice.late.is_empty()).then(|| {
        format!(
            "{}中途进场，首帧里没有，首帧出片只能猜他的长相 —— 删掉首帧走参考出片（每个人都有参考图）",
            advice.late.join("、")
        )
    })
}

/// 接上一镜自动设成什么（见模块说明第 4 条）
///
/// - 第一镜、跨场、长镜头：不接（跨场接了会把上一场的光线布景带进来；长镜头窗口之间靠重叠帧）；
/// - 同一场、参考出片：运镜与光线（上一镜最后 1 秒当视频参考，2026-09-20 实测跨镜比同镜内部还稳）；
/// - 同一场、首帧出片：不接（构图由首帧定；「运镜与光线」在首帧出片里本来就不起作用）。
///
/// **不自动选「接住最后一帧」**：它要求两镜姿势严丝合缝，接不上开头会跳 —— 只能人判断。
pub fn auto_continuity(shot: &StoryboardShot, prev: Option<&StoryboardShot>, route: ShotRoute, long_take: bool) -> Continuity {
    let same_scene = prev.is_some_and(|p| shot.scene_no != 0 && shot.scene_no == p.scene_no);
    match (same_scene && !long_take, route) {
        (true, ShotRoute::Reference) => Continuity::RefVideo,
        _ => Continuity::None,
    }
}

/// 这一镜现在该按哪条走：有首帧按首帧，没有按内容建议（还没出首帧的首帧出片镜头，接上一镜也按首帧出片算）
pub fn planned_route(shot: &StoryboardShot, assets: &[AssetRef], long_take: bool) -> ShotRoute {
    if long_take {
        return ShotRoute::Reference;
    }
    if shot.keyframes.drives_video() {
        return ShotRoute::Keyframe;
    }
    content_route(shot, assets).route
}

/// 整集按规则把「接上一镜」设好（手动改过的不动）。回改了哪几镜 `(镜号, 新值)`
pub fn apply_auto_continuity(ep: &mut crate::cinema::EpisodeData, assets: &[AssetRef]) -> Vec<(u32, Continuity)> {
    let mut order: Vec<usize> = (0..ep.storyboard.len()).collect();
    order.sort_by_key(|&i| ep.storyboard[i].shot_num);
    let mut changed = Vec::new();
    for (k, &i) in order.iter().enumerate() {
        let prev = (k > 0).then(|| ep.storyboard[order[k - 1]].clone());
        let shot = &ep.storyboard[i];
        if shot.continuity_manual {
            continue;
        }
        let long_take = ep.long_take_scenes.contains(&shot.scene_no);
        let want = auto_continuity(shot, prev.as_ref(), planned_route(shot, assets, long_take), long_take);
        if shot.effective_continuity() != want {
            let s = &mut ep.storyboard[i];
            s.continuity = want;
            s.continues_previous = false;
            changed.push((s.shot_num, want));
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::AssetKind;

    fn asset(id: &str, name: &str) -> AssetRef {
        AssetRef { id: id.into(), kind: AssetKind::Character, name: name.into(), has_image: true, description: String::new(), look_id: String::new(), looks: Vec::new(), h3_voice: String::new() }
    }

    fn shot(num: u32, scene: u32, chars: &[&str], segs: &[(&str, &str)], people: &[&str]) -> StoryboardShot {
        let mut s: StoryboardShot = serde_json::from_value(serde_json::json!({
            "id": format!("s{num}"), "shot_num": num, "scale": "", "camera_movement": "", "dialogue": "", "prompt": "", "duration": 5.0, "status": "pending"
        }))
        .unwrap();
        s.scene_no = scene;
        s.character_ids = chars.iter().map(|c| c.to_string()).collect();
        s.segments = segs.iter().map(|(v, b)| crate::shot_table::ShotSegment { visual: v.to_string(), blocking: b.to_string(), ..Default::default() }).collect();
        s.stage.people = people.iter().map(|p| p.to_string()).collect();
        s
    }

    fn assets() -> Vec<AssetRef> {
        vec![asset("a", "星澜"), asset("b", "陆川"), asset("c", "苏晴"), asset("d", "胖子"), asset("e", "大妈")]
    }

    #[test]
    fn 按内容判断链路() {
        let a = assets();
        // 两个人开场都在 → 首帧
        let two = shot(1, 1, &["a", "b"], &[("星澜贴着陆川", "")], &[]);
        assert_eq!(content_route(&two, &a).route, ShotRoute::Keyframe);
        // 苏晴中途进场 → 参考
        let late = shot(2, 1, &["a", "b", "c"], &[("星澜贴着陆川", ""), ("苏晴从巷口走来", "")], &["苏晴：尚未出现，第 2 段从巷口进"]);
        let adv = content_route(&late, &a);
        assert_eq!((adv.route, adv.late.clone()), (ShotRoute::Reference, vec!["苏晴".to_string()]));
        // 开场 4 个人 → 参考（首帧挂不下 4 个人的图）
        let crowd = shot(3, 1, &["a", "b", "c", "d"], &[("星澜、陆川、苏晴、胖子站在巷口", "")], &[]);
        assert_eq!(content_route(&crowd, &a).route, ShotRoute::Reference);
        assert!(content_route(&crowd, &a).reason.contains("4 个人"));
        // 3 个人正好挂得下 → 首帧
        let three = shot(4, 1, &["a", "b", "c"], &[("星澜、陆川、苏晴站在巷口", "")], &[]);
        assert_eq!(content_route(&three, &a).route, ShotRoute::Keyframe);
    }

    /// 2026-10-11 EP05 第 16 镜：领队带保安破门而入，两人都没建资产 —— 以前只数到星澜、陆川，建议首帧出片
    #[test]
    fn 没建资产的人也算() {
        let a = assets();
        let unlinked = |names: &[&str]| names.iter().map(|n| crate::shot_table::UnlinkedAsset { kind: "character".into(), name: n.to_string() }).collect::<Vec<_>>();
        // 开场就在：星澜、陆川、领队、保安队 = 4 个 → 参考（首帧挂不下）
        let mut burst = shot(16, 3, &["a", "b"], &[("屋门弹开，领队带着保安队跨进门槛，星澜和陆川僵住", "")], &[]);
        burst.unlinked_assets = unlinked(&["领队", "保安队"]);
        assert_eq!(content_route(&burst, &a).route, ShotRoute::Reference);
        // 中途才进来（站位写着尚未出现）→ 参考，并且挂了首帧要拦
        let mut late = shot(17, 3, &["a", "b"], &[("星澜贴着陆川", ""), ("领队推门进来", "")], &["领队：尚未出现，第 2 段进门"]);
        late.unlinked_assets = unlinked(&["领队"]);
        assert_eq!(content_route(&late, &a).late, vec!["领队".to_string()]);
        late.keyframes.first = true;
        assert!(keyframe_blocker(&late, &a, false).unwrap().contains("领队"));
        // 没建资产的道具不算人
        let mut prop = shot(18, 3, &["a", "b"], &[("星澜递出名片", "")], &[]);
        prop.unlinked_assets = vec![crate::shot_table::UnlinkedAsset { kind: "prop".into(), name: "手写名片".into() }];
        assert_eq!(content_route(&prop, &a).on_screen, 2);
    }

    /// 2026-10-11 E05：从靴子 / 泡面 / 胸口 / 腰侧特写起拍再摇上去，或者横摇过去才拍到人 —— 首帧里看不到脸
    #[test]
    fn 运镜才把人露出来的_建议参考() {
        let a = assets();
        let cam = |mv: &str, note: &str| {
            let mut s = shot(7, 2, &["a", "b"], &[("星澜侧坐床沿，陆川坐在对面", "")], &[]);
            s.camera_movement = mv.into();
            s.camera_note = note.into();
            s
        };
        let boots = cam("tilt_up", "从机械靴近景沿交叠双腿上摇至双人中景，保持在主轴西侧");
        let adv = content_route(&boots, &a);
        assert_eq!(adv.route, ShotRoute::Reference);
        assert!(adv.reason.contains("靴"), "{}", adv.reason);
        assert_eq!(content_route(&cam("push_in", "低位贴近星澜腰侧起幅，随她第二次呼唤推向脸"), &a).route, ShotRoute::Reference);
        // 横摇过去才拍到陆川
        let pan = cam("pan_left", "从星澜方向越过前景肩部，随扫描仪由东向西转到陆川");
        assert!(content_route(&pan, &a).reason.contains("陆川是镜头摇过去才拍到的"));
        // 普通推近（从两人近景推到脸）、摇到的是东西不是人、固定机位：照旧首帧
        assert_eq!(content_route(&cam("push_in", "窗外同轴低机位，从两人近景缓慢推向星澜礼貌微笑的脸"), &a).route, ShotRoute::Keyframe);
        assert_eq!(content_route(&cam("pan_right", "先留在窗前双人中景；镜头随他的视线小幅右摇到窗口，将楼下泡面和野猫作为结尾"), &a).route, ShotRoute::Keyframe);
        assert_eq!(content_route(&cam("static", "从机械靴起拍"), &a).route, ShotRoute::Keyframe, "不动的机位，开场拍什么首帧就是什么");
    }

    /// 镜头详情里手动选的出片方式压过内容建议
    #[test]
    fn 手动选的出片方式优先() {
        let a = assets();
        let mut two = shot(1, 1, &["a", "b"], &[("星澜贴着陆川", "")], &[]);
        two.route_choice = Some(ShotRoute::Reference);
        let adv = content_route(&two, &a);
        assert_eq!(adv.route, ShotRoute::Reference);
        assert!(adv.reason.starts_with("手动选了参考出片"), "{}", adv.reason);
        // 选了参考：按场出首帧不给它出（planned_route 跟建议走），接上一镜按参考设
        assert_eq!(planned_route(&two, &a, false), ShotRoute::Reference);
        let mut crowd = shot(3, 1, &["a", "b", "c", "d"], &[("星澜、陆川、苏晴、胖子站在巷口", "")], &[]);
        crowd.route_choice = Some(ShotRoute::Keyframe);
        assert_eq!(content_route(&crowd, &a).route, ShotRoute::Keyframe);
    }

    #[test]
    fn 挂了首帧却有人中途进场要拦() {
        let a = assets();
        let mut late = shot(2, 1, &["a", "b", "c"], &[("星澜贴着陆川", ""), ("苏晴从巷口走来", "")], &["苏晴：尚未出现"]);
        assert_eq!(keyframe_blocker(&late, &a, false), None, "没首帧：本来就走参考");
        late.keyframes.first = true;
        assert!(keyframe_blocker(&late, &a, false).unwrap().contains("苏晴"));
        assert_eq!(keyframe_blocker(&late, &a, true), None, "长镜头不吃首帧");
        let mut two = shot(1, 1, &["a", "b"], &[("星澜贴着陆川", "")], &[]);
        two.keyframes.first = true;
        assert_eq!(keyframe_blocker(&two, &a, false), None, "人都在首帧里：不拦");
    }

    #[test]
    fn 接上一镜自动设() {
        let a = assets();
        let first = shot(1, 1, &["a", "b"], &[("星澜贴着陆川", "")], &[]);
        let same_ref = shot(2, 1, &["a", "b", "c", "d"], &[("四个人站在巷口", "")], &[]);
        let mut same_kf = shot(3, 1, &["a", "b"], &[("星澜贴着陆川", "")], &[]);
        same_kf.keyframes.first = true;
        let other_scene = shot(4, 2, &["a", "b", "c", "d"], &[("四个人站在巷口", "")], &[]);
        assert_eq!(auto_continuity(&first, None, ShotRoute::Keyframe, false), Continuity::None, "第一镜");
        assert_eq!(auto_continuity(&same_ref, Some(&first), ShotRoute::Reference, false), Continuity::RefVideo, "同场参考");
        assert_eq!(auto_continuity(&same_kf, Some(&same_ref), ShotRoute::Keyframe, false), Continuity::None, "同场首帧");
        assert_eq!(auto_continuity(&other_scene, Some(&same_kf), ShotRoute::Reference, false), Continuity::None, "跨场");
        assert_eq!(auto_continuity(&same_ref, Some(&first), ShotRoute::Reference, true), Continuity::None, "长镜头");

        let mut ep: crate::cinema::EpisodeData = serde_json::from_value(serde_json::json!({
            "id": "e", "episode_num": 1, "title": "t", "synopsis": "", "script_text": "", "storyboard": []
        }))
        .unwrap();
        let mut manual = same_ref.clone();
        manual.id = "m".into();
        manual.shot_num = 5;
        manual.continuity_manual = true;
        manual.continuity = Continuity::FirstFrame;
        ep.storyboard = vec![same_kf.clone(), same_ref.clone(), first.clone(), manual];
        let changed = apply_auto_continuity(&mut ep, &a);
        assert_eq!(changed, vec![(2, Continuity::RefVideo)], "按镜号排、只改该改的");
        assert_eq!(ep.storyboard.iter().find(|s| s.id == "m").unwrap().continuity, Continuity::FirstFrame, "手动改过的不动");
        assert!(apply_auto_continuity(&mut ep, &a).is_empty(), "再跑一遍不变");
    }
}
