//! 分集颜色表：本集每个出场角色一个唯一颜色。
//!
//! 改写自 DramaClaw（ClaymoreLab, Elastic License 2.0）
//! `src/novelvideo/generators/episode_optimizer.py` 的 `BRIDGMAN_CHARACTER_PALETTE` /
//! `assign_sketch_colors`。出处规则见 DECISIONS.md 2026-09-16。
//!
//! # 为什么需要它
//!
//! **颜色 = 身份。** 导演世界里的人偶颜色、以后出图时认人，用的都是这一张表：
//! 下游看到画面里一个品红色的人偶，查表才知道那是谁。所以：
//!
//! - **一集一张表**，镜头之间一致（同一个人在第 3 镜和第 8 镜是同一个颜色）
//! - **已经分配的颜色永远不改** —— 改剧本、加镜头、删掉某个角色的戏份，
//!   老角色都不换色。旧的导出图里颜色是内容的一部分，换了就对不上了
//! - 只给**有身体**的角色分色（旁白、电话那头的人不会出现在画面里）
//!
//! # 和 DC 的差别
//!
//! - DC 按「身份」（角色 + 装扮）分色；这里按角色资产分色，同一个人换造型颜色不变 ——
//!   Anima 的造型是按镜头覆盖的，不是独立身份
//! - DC 按身份 id 字母序分配，这里按首次出场顺序（主角通常先出场，拿到最醒目的主色）
//! - DC 调色盘用完之后静默不分配；这里把没分到颜色的角色报出来

use serde::{Deserialize, Serialize};

/// 人物调色盘：高饱和荧光色，和场景里常见的颜色拉得开。
/// 前 [`PRIMARY_SIZE`] 个是主色，先用完主色再用后面的。
pub const PALETTE: [(&str, &str); 12] = [
    ("#FF00FF", "FLUORESCENT MAGENTA"),
    ("#00FFFF", "FLUORESCENT CYAN"),
    ("#CCFF00", "FLUORESCENT LIME"),
    ("#FF6B00", "FLUORESCENT ORANGE"),
    ("#7C4DFF", "ELECTRIC VIOLET"),
    ("#00FF66", "NEON MINT"),
    ("#00A2FF", "ELECTRIC AZURE"),
    ("#FFD400", "SIGNAL YELLOW"),
    ("#9D00FF", "NEON PURPLE"),
    ("#00FFCC", "FLUORESCENT AQUA"),
    ("#39FF14", "LASER GREEN"),
    ("#5C6BC0", "INDIGO"),
];

pub const PRIMARY_SIZE: usize = 8;

/// 新颜色和已分配颜色的色相至少差这么多度，找不到就取差得最多的。
const MIN_HUE_GAP: f64 = 60.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActorColor {
    pub asset_id: String,
    /// 角色名快照，给界面显示用。改名后下次分配时刷新
    pub name: String,
    /// `#RRGGBB`
    pub color: String,
    /// 英文色名，将来写进出图提示词（「FLUORESCENT MAGENTA 人偶是沈星晴」）
    pub color_name: String,
}

/// 分配结果。`unassigned` 非空说明调色盘不够用了 —— **要让界面看得见**。
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Assignment {
    pub colors: Vec<ActorColor>,
    pub unassigned: Vec<String>,
}

fn hue(hex: &str) -> Option<f64> {
    let h = hex.trim().trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok().map(|v| v as f64 / 255.0);
    let (r, g, b) = (p(0)?, p(2)?, p(4)?);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    if d == 0.0 {
        return Some(0.0);
    }
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    Some(h * 60.0)
}

fn hue_gap(a: f64, b: f64) -> f64 {
    let d = (a - b).abs() % 360.0;
    d.min(360.0 - d)
}

/// 给本集出场角色补齐颜色。
///
/// `cast` 是按首次出场顺序排好的 `(asset_id, name)`。
/// `existing` 里的条目**原样保留**（名字按 `cast` 刷新），包括这次没出场的角色。
pub fn assign(existing: &[ActorColor], cast: &[(String, String)]) -> Assignment {
    let mut colors: Vec<ActorColor> = existing
        .iter()
        .filter(|c| !c.asset_id.trim().is_empty() && hue(&c.color).is_some())
        .cloned()
        .collect();
    for c in colors.iter_mut() {
        if let Some((_, name)) = cast.iter().find(|(id, _)| *id == c.asset_id) {
            c.name = name.clone();
        }
    }

    let mut used: Vec<String> = colors.iter().map(|c| c.color.to_uppercase()).collect();
    let mut unassigned = Vec::new();

    for (id, name) in cast {
        if colors.iter().any(|c| &c.asset_id == id) {
            continue;
        }
        let free: Vec<usize> = (0..PALETTE.len()).filter(|&i| !used.contains(&PALETTE[i].0.to_string())).collect();
        let primary: Vec<usize> = free.iter().copied().filter(|&i| i < PRIMARY_SIZE).collect();
        let candidates = if primary.is_empty() { free } else { primary };
        if candidates.is_empty() {
            unassigned.push(name.clone());
            continue;
        }
        let used_hues: Vec<f64> = used.iter().filter_map(|c| hue(c)).collect();
        let mut best = candidates[0];
        let mut best_gap = -1.0;
        for &i in &candidates {
            let h = hue(PALETTE[i].0).unwrap_or(0.0);
            let gap = used_hues.iter().map(|&u| hue_gap(h, u)).fold(f64::INFINITY, f64::min);
            if used_hues.is_empty() || gap >= MIN_HUE_GAP {
                best = i;
                break;
            }
            if gap > best_gap {
                best_gap = gap;
                best = i;
            }
        }
        let (hex, color_name) = PALETTE[best];
        used.push(hex.to_string());
        colors.push(ActorColor {
            asset_id: id.clone(),
            name: name.clone(),
            color: hex.to_string(),
            color_name: color_name.to_string(),
        });
    }
    Assignment { colors, unassigned }
}

/// 手动改一个角色的颜色。**必须是调色盘里的、且没被别人占用** ——
/// 两个人同色，下游就认不出谁是谁。
pub fn set_color(colors: &[ActorColor], asset_id: &str, hex: &str) -> Result<Vec<ActorColor>, String> {
    let hex_up = hex.trim().to_uppercase();
    let Some((pal_hex, pal_name)) = PALETTE.iter().find(|(h, _)| *h == hex_up) else {
        return Err(format!("{hex} 不在人物调色盘里"));
    };
    if let Some(other) = colors.iter().find(|c| c.asset_id != asset_id && c.color.to_uppercase() == hex_up) {
        return Err(format!("这个颜色已经分给了「{}」", other.name));
    }
    let mut out = colors.to_vec();
    let Some(me) = out.iter_mut().find(|c| c.asset_id == asset_id) else {
        return Err("这个角色还没有颜色，先分配一次".into());
    };
    me.color = pal_hex.to_string();
    me.color_name = pal_name.to_string();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cast(names: &[&str]) -> Vec<(String, String)> {
        names.iter().map(|n| (format!("id-{n}"), n.to_string())).collect()
    }

    #[test]
    fn 调色盘颜色互不相同() {
        let mut hexes: Vec<_> = PALETTE.iter().map(|(h, _)| *h).collect();
        hexes.sort();
        hexes.dedup();
        assert_eq!(hexes.len(), PALETTE.len());
    }

    #[test]
    fn 每个角色一个颜色且互不相同() {
        let a = assign(&[], &cast(&["沈星晴", "陆临川", "掌柜", "小二"]));
        assert_eq!(a.colors.len(), 4);
        assert!(a.unassigned.is_empty());
        let mut hexes: Vec<_> = a.colors.iter().map(|c| c.color.clone()).collect();
        hexes.dedup();
        assert_eq!(hexes.len(), 4);
        assert_eq!(a.colors[0].name, "沈星晴");
    }

    /// 第一个出场的拿第一个主色；第二个要和它色相拉开
    #[test]
    fn 先出场的拿主色且相邻角色色相拉开() {
        let a = assign(&[], &cast(&["甲", "乙"]));
        assert_eq!(a.colors[0].color, PALETTE[0].0);
        let gap = hue_gap(hue(&a.colors[0].color).unwrap(), hue(&a.colors[1].color).unwrap());
        assert!(gap >= MIN_HUE_GAP, "gap = {gap}");
    }

    /// **核心不变量**：改剧本之后老角色不换色
    #[test]
    fn 已分配的颜色不变() {
        let first = assign(&[], &cast(&["甲", "乙", "丙"])).colors;
        // 新加一个角色、乙的戏份删了、顺序也变了
        let second = assign(&first, &cast(&["丁", "丙", "甲"]));
        for old in &first {
            let now = second.colors.iter().find(|c| c.asset_id == old.asset_id).unwrap();
            assert_eq!(now.color, old.color, "{} 换色了", old.name);
        }
        let ding = second.colors.iter().find(|c| c.name == "丁").unwrap();
        assert!(first.iter().all(|c| c.color != ding.color));
    }

    #[test]
    fn 重复分配是幂等的() {
        let once = assign(&[], &cast(&["甲", "乙"]));
        let twice = assign(&once.colors, &cast(&["甲", "乙"]));
        assert_eq!(once.colors, twice.colors);
    }

    #[test]
    fn 改名后刷新名字但不换色() {
        let first = assign(&[], &cast(&["甲"])).colors;
        let renamed = assign(&first, &[("id-甲".into(), "甲甲".into())]).colors;
        assert_eq!(renamed[0].name, "甲甲");
        assert_eq!(renamed[0].color, first[0].color);
    }

    #[test]
    fn 主色用完才用后面的() {
        let names: Vec<String> = (0..PRIMARY_SIZE).map(|i| format!("人{i}")).collect();
        let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        let a = assign(&[], &cast(&refs));
        for c in &a.colors {
            let idx = PALETTE.iter().position(|(h, _)| *h == c.color).unwrap();
            assert!(idx < PRIMARY_SIZE);
        }
    }

    /// DC 在这里静默不分配；我们报出来
    #[test]
    fn 调色盘不够时报出没分到的角色() {
        let names: Vec<String> = (0..PALETTE.len() + 2).map(|i| format!("人{i}")).collect();
        let refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        let a = assign(&[], &cast(&refs));
        assert_eq!(a.colors.len(), PALETTE.len());
        assert_eq!(a.unassigned, vec!["人12".to_string(), "人13".to_string()]);
    }

    #[test]
    fn 手动改色不能撞色也不能用调色盘外的颜色() {
        let colors = assign(&[], &cast(&["甲", "乙"])).colors;
        let taken = colors[1].color.clone();
        assert!(set_color(&colors, "id-甲", &taken).unwrap_err().contains("乙"));
        assert!(set_color(&colors, "id-甲", "#123456").is_err());
        let free = PALETTE.iter().map(|(h, _)| *h).find(|h| colors.iter().all(|c| c.color != *h)).unwrap();
        let next = set_color(&colors, "id-甲", &free.to_lowercase()).unwrap();
        assert_eq!(next[0].color, free);
    }

    #[test]
    fn 坏数据丢掉() {
        let bad = vec![
            ActorColor { asset_id: "".into(), name: "x".into(), color: "#FF00FF".into(), color_name: "".into() },
            ActorColor { asset_id: "id-y".into(), name: "y".into(), color: "red".into(), color_name: "".into() },
        ];
        let a = assign(&bad, &cast(&["甲"]));
        assert_eq!(a.colors.len(), 1);
        assert_eq!(a.colors[0].asset_id, "id-甲");
    }
}
