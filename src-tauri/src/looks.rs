//! 角色形象怎么选：**单镜覆盖 → 项目默认 → 主形象**。
//!
//! 数据结构在 [`crate::assets::Look`]；方案见 `docs/plans/2026-09-15-角色形象方案.md`。
//!
//! # 接进出片链路的方式
//!
//! 出片链路到处按资产取描述（`AssetRef.description`）和图（`has_image`、参考图路径）。
//! 不去改那几十处，而是在源头换掉：
//!
//! - [`crate::storyboard_gen::load_assets`] 读完资产就按**项目默认形象**换好
//! - 按镜处理的地方（合成提示词、提示词指纹、参考图路径）先过一遍 [`for_shot`]，按**单镜覆盖**再换一次
//!
//! 换形象 → 描述变 → 提示词指纹变 → 这一镜自动标「要重新合成」，不会拿旧提示词出新形象的片。

use std::collections::BTreeMap;
use std::path::Path;

#[cfg(test)]
use crate::assets::MAIN_LOOK;
use crate::cinema::StoryboardShot;
use crate::storyboard_gen::AssetRef;

/// 项目的默认形象表。读不到就是空（全部主形象）
pub fn project_default_looks(config_dir: &Path, project_id: &str) -> BTreeMap<String, String> {
    std::fs::read_to_string(crate::cinema::cinema_projects_dir(config_dir).join(project_id).join("project.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("default_looks").cloned())
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

/// 按「资产 id → 形象 id」表换形象。表里的形象不存在（被删了）就不换，留在原来那个。
pub fn apply(assets: &[AssetRef], choice: &BTreeMap<String, String>) -> Vec<AssetRef> {
    assets
        .iter()
        .map(|a| {
            let Some(look) = choice.get(&a.id).and_then(|id| a.looks.iter().find(|l| &l.id == id)) else {
                return a.clone();
            };
            AssetRef {
                description: look.description.clone(),
                has_image: look.has_image,
                // **显式选了主形象留 `main`，不清空**（2026-10-10 EP04 第 19 镜）：空 = 没选、跟着默认走，
                // `main` = 用户明说这一镜穿主形象。清空的话两者分不开，`outfit_mismatch` 把选过的也当「换了装没选造型」
                look_id: look.id.clone(),
                ..a.clone()
            }
        })
        .collect()
}

/// 这一镜实际用的资产：在（已经按项目默认换过的）资产上再套这一镜的覆盖。
pub fn for_shot(assets: &[AssetRef], shot: &StoryboardShot) -> Vec<AssetRef> {
    if shot.look_overrides.is_empty() {
        return assets.to_vec();
    }
    apply(assets, &shot.look_overrides)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::AssetKind;
    use crate::storyboard_gen::LookRef;

    fn shen() -> AssetRef {
        let look = |id: &str, d: &str, img: bool| LookRef { id: id.into(), name: id.into(), description: d.into(), has_image: img };
        AssetRef {
            id: "char_shen".into(), kind: AssetKind::Character, name: "林婉清".into(), has_image: true,
            description: "女\n白衬衫".into(), look_id: String::new(),
            looks: vec![look(MAIN_LOOK, "女\n白衬衫", true), look("look_tang", "女\n男装官服", true), look("look_ball", "女\n礼服", false)],
            h3_voice: String::new(),
        }
    }

    #[test]
    fn 单镜覆盖_项目默认_主形象三级回落() {
        let assets = vec![shen()];
        let mut defaults = BTreeMap::new();
        // 没有默认：主形象
        assert_eq!(apply(&assets, &defaults)[0].description, "女\n白衬衫");
        // 项目默认盛唐
        defaults.insert("char_shen".into(), "look_tang".into());
        let projected = apply(&assets, &defaults);
        assert_eq!((projected[0].description.as_str(), projected[0].look_id.as_str()), ("女\n男装官服", "look_tang"));
        // 这一镜换礼服（没图）
        let mut shot: StoryboardShot = serde_json::from_value(serde_json::json!({
            "id": "s1", "shot_num": 1, "scale": "", "camera_movement": "", "dialogue": "", "prompt": "", "duration": 3.0, "status": "pending"
        })).unwrap();
        assert_eq!(for_shot(&projected, &shot)[0].look_id, "look_tang", "没覆盖用项目默认");
        shot.look_overrides.insert("char_shen".into(), "look_ball".into());
        let s = &for_shot(&projected, &shot)[0];
        assert_eq!((s.description.as_str(), s.has_image, s.look_id.as_str()), ("女\n礼服", false, "look_ball"));
        // 覆盖成主形象：描述换回主形象，look_id 留 `main`（显式选过，和「没选」分开）
        shot.look_overrides.insert("char_shen".into(), MAIN_LOOK.into());
        let s = &for_shot(&projected, &shot)[0];
        assert_eq!((s.description.as_str(), s.look_id.as_str()), ("女\n白衬衫", MAIN_LOOK));
        // 指着被删掉的形象：不换
        shot.look_overrides.insert("char_shen".into(), "look_gone".into());
        assert_eq!(for_shot(&projected, &shot)[0].look_id, "look_tang");
    }

    #[test]
    fn 读项目默认形象表() {
        let d = tempfile::tempdir().unwrap();
        let proj = d.path().join("projects").join("p1");
        std::fs::create_dir_all(&proj).unwrap();
        assert!(project_default_looks(d.path(), "p1").is_empty());
        std::fs::write(proj.join("project.json"), r#"{"default_looks":{"char_shen":"look_tang"}}"#).unwrap();
        let got = project_default_looks(d.path(), "p1");
        assert_eq!(got.get("char_shen").map(String::as_str), Some("look_tang"));
    }
}
