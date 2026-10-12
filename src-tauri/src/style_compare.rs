//! **风格广场** —— 同一个镜头，铺开多个风格，拿来挑。
//!
//! # 为什么需要它
//!
//! 风格是 **project 级**的（`style::load_art(config_dir, project_id)`），
//! 改一次全集都变。所以「这个剧本配哪个画风」这个问题，以前只能靠想象，
//! 或者改了设置整集重出一遍。
//!
//! # 两种模式，用途不同
//!
//! | 模式 | 构图 | 用途 |
//! |---|---|---|
//! | 自由（不挂 ControlNet） | 每个风格自己长 | 看「这个风格配这个剧本什么感觉」 |
//! | **锁构图**（挂 ControlNet） | 完全一致 | 严格 A/B：**只有画风这一个变量** |
//!
//! 锁构图那档必须挂 ControlNet 才成立 —— 光靠同 seed 是不够的，
//! 风格词会改变采样轨迹，同 seed 不同风格出来的构图差别很大。
//!
//! # 它还能顺便结掉一个悬案
//!
//! `style::apply_style` 里留着一句注释：长提示词把风格放前面这条规矩
//! 「**这一处本身还没在分镜上 A/B 过**」。那条结论是从资产出图搬过来的。
//! 风格广场的锁构图模式正好是做这个 A/B 的工具 —— 同构图、同 seed、
//! 只改风格词的位置，一眼就能看出来。

use crate::agent_runtime::image_engine;
use crate::error::{Error, Result};
use crate::jobs::{self, JobKind};
use crate::style::{self, ArtDirection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// 对照表里的一格。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StyleCell {
    pub preset_id: String,
    pub name_zh: String,
    pub prompt: String,
    /// 出图用的文件名前缀，方便事后对上是哪个风格
    pub filename_prefix: String,
}

/// 一次对照的配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompareSpec {
    /// 镜头提示词（不含风格词）
    pub shot_prompt: String,
    /// 要比哪些风格。空 = 全部 15 个。
    #[serde(default)]
    pub preset_ids: Vec<String>,
    pub width: u32,
    pub height: u32,
    /// **所有格子共用同一个 seed** —— 不同 seed 的对照没有意义。
    pub seed: u64,
    /// 锁构图用的控制图（ComfyUI input 下的文件名）。None = 自由模式。
    #[serde(default)]
    pub control_image: Option<String>,
}

/// 展开成一格一格。
pub fn cells(spec: &CompareSpec) -> Vec<StyleCell> {
    let wanted: Vec<&style::StylePreset> = if spec.preset_ids.is_empty() {
        style::all_presets().iter().collect()
    } else {
        spec.preset_ids
            .iter()
            .filter_map(|id| style::find_preset(id))
            .collect()
    };
    wanted
        .into_iter()
        .map(|p| {
            // 走 apply_style，不自己拼 —— 长短提示词的风格词位置规矩在那里面，
            // 在这儿重写一遍就等于埋一个会和主链路分叉的复制品。
            let art = ArtDirection {
                preset_id: p.id.clone(),
                ..Default::default()
            };
            StyleCell {
                preset_id: p.id.clone(),
                name_zh: p.name_zh.clone(),
                prompt: style::apply_style(&spec.shot_prompt, &art),
                filename_prefix: format!("anima_style_{}", p.id),
            }
        })
        .collect()
}

/// 给每一格建一张 ComfyUI 图。
///
/// **seed 全部相同**，控制图（如果有）也全部相同 —— 变量只有风格词。
pub fn graphs(spec: &CompareSpec) -> Vec<(StyleCell, Value)> {
    cells(spec)
        .into_iter()
        .map(|c| {
            let mut g = image_engine::build_zimage_graph(
                &c.prompt, spec.width, spec.height, spec.seed, 8,
            );
            if let Some(ctrl) = &spec.control_image {
                image_engine::attach_zimage_control(
                    &mut g,
                    ctrl,
                    image_engine::ZIMAGE_CONTROL_STRENGTH,
                );
            }
            if let Some(n) = g.get_mut("10").and_then(|n| n.get_mut("inputs")) {
                if let Some(o) = n.as_object_mut() {
                    o.insert("filename_prefix".into(), Value::String(c.filename_prefix.clone()));
                }
            }
            (c, g)
        })
        .collect()
}

/// 风格广场出的图落在**项目目录**下，不是 `resources/style_thumbs/`。
///
/// 那个目录是**内置资源、进版本库**的（`scripts/render-style-thumbs.sh` 出的
/// 通用样例）。往里写等于把某个项目的画面混进全局资源，换个项目还看得见。
pub fn dir(config_dir: &Path, project_id: &str) -> PathBuf {
    crate::cinema::cinema_projects_dir(config_dir)
        .join(project_id)
        .join("style_compare")
}

/// 这个项目已经出好的风格对照图：`preset_id → 绝对路径`。
pub fn existing(config_dir: &Path, project_id: &str) -> Vec<(String, String)> {
    let d = dir(config_dir, project_id);
    let Ok(rd) = std::fs::read_dir(&d) else {
        return Vec::new();
    };
    let mut out: Vec<(String, String)> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("png") {
                return None;
            }
            let id = p.file_stem()?.to_str()?.to_string();
            Some((id, p.to_string_lossy().to_string()))
        })
        .collect();
    out.sort();
    out
}

/// 出一格并存进项目目录。
async fn run_one(
    config_dir: &Path,
    project_id: &str,
    cell: &StyleCell,
    spec: &CompareSpec,
    engine: &str,
) -> std::result::Result<String, String> {
    // 走 generate_with 而不是自己提交裸 graph —— 引擎选择、回落理由、超时
    // 那一整套都在它里面，绕过去等于复制一份会和主链路分叉的实现。
    let args: crate::studio::GenerateArgs = serde_json::from_value(json!({
        "mode": "t2i",
        "engine": engine,
        "prompt": cell.prompt,
        "negative_prompt": "",
        "width": spec.width,
        "height": spec.height,
        // **每一格同一个 seed** —— 不同 seed 的「对照」什么也没对照
        "seed": spec.seed,
        "images": [],
        // 常挂 LoRA 会把画风拽走，风格对照必须关掉 —— 和资产出图同一个理由
        "loras": [],
        "auto_loras": false,
    }))
    .map_err(|e| format!("参数拼装失败：{e}"))?;
    let out = crate::studio::generate_with(config_dir, args)
        .await
        .map_err(|e| e.to_string())?;
    if !out.ok {
        return Err(if out.message.trim().is_empty() {
            "引擎没有返回图片".into()
        } else {
            out.message
        });
    }
    let src = out
        .paths
        .first()
        .cloned()
        .or(out.path)
        .ok_or_else(|| "没有产出文件".to_string())?;
    let d = dir(config_dir, project_id);
    std::fs::create_dir_all(&d).map_err(|e| format!("建不了目录：{e}"))?;
    let dst = d.join(format!("{}.png", cell.preset_id));
    std::fs::copy(&src, &dst).map_err(|e| format!("存不进项目目录：{e}"))?;
    Ok(dst.to_string_lossy().to_string())
}

/// 登记并后台开跑。按**项目**登记 —— 风格是项目级的，一个项目同时只跑一批。
pub fn start(
    app: Option<tauri::AppHandle>,
    config_dir: &Path,
    project_id: &str,
    spec: CompareSpec,
) -> Result<jobs::Job> {
    if spec.shot_prompt.trim().is_empty() {
        return Err(Error::config("先选一个镜头 —— 风格广场要拿你自己的画面去试"));
    }
    let cells = cells(&spec);
    if cells.is_empty() {
        return Err(Error::config("没有选中任何风格"));
    }
    let title = format!("风格广场 · {} 个风格", cells.len());
    let job_id = jobs::start_exclusive(
        config_dir,
        JobKind::StyleCompare,
        project_id,
        "",
        "",
        0,
        &title,
    )
    .map_err(|_| Error::config("这个项目已经在跑风格广场了 —— 进度在右下角"))?;
    let targets: Vec<String> = cells.iter().map(|c| c.preset_id.clone()).collect();
    jobs::set_targets(config_dir, &job_id, &targets);
    jobs::set_progress(config_dir, &job_id, 0, cells.len(), &cells[0].name_zh);
    let started =
        jobs::get(config_dir, &job_id).ok_or_else(|| Error::runtime("任务登记后读不回来"))?;
    if let Some(app) = &app {
        jobs::emit(Some(app), Some(&started));
    }

    let config_dir = config_dir.to_path_buf();
    let project_id_owned = project_id.to_string();
    let engine = crate::asset_batch::project_engine(&config_dir, &project_id_owned);
    tauri::async_runtime::spawn(async move {
        let emit = |job: Option<jobs::Job>| {
            if let Some(app) = &app {
                jobs::emit(Some(app), job.as_ref());
            }
        };
        let mut failed = Vec::new();
        let mut attempted = 0usize;
        for (i, c) in cells.iter().enumerate() {
            if jobs::cancel_requested(&config_dir, &job_id) {
                break;
            }
            attempted += 1;
            emit(jobs::set_progress(
                &config_dir,
                &job_id,
                i,
                cells.len(),
                &c.name_zh,
            ));
            match run_one(&config_dir, &project_id_owned, c, &spec, &engine).await {
                Ok(_) => {
                    if let Some(app) = &app {
                        crate::studio_ops::emit_changed(app, "style", Some(&project_id_owned));
                    }
                }
                Err(e) => failed.push(format!("{}：{e}", c.name_zh)),
            }
        }
        let summary = finish_summary(cells.len(), attempted, &failed);
        let job = if !failed.is_empty() && failed.len() == attempted {
            jobs::finish_with(
                &config_dir,
                &job_id,
                None,
                Some(&summary),
                &failed,
                Some(&failed[0]),
            )
        } else {
            jobs::finish_with(&config_dir, &job_id, None, Some(&summary), &failed, None)
        };
        emit(job);
    });
    Ok(started)
}

/// `attempted` < `total` 说明中途停了。
pub fn finish_summary(total: usize, attempted: usize, failed: &[String]) -> String {
    let ok = attempted - failed.len();
    let head = if failed.is_empty() && attempted == total {
        format!("{ok} 个风格都出好了，去风格页挑")
    } else if failed.is_empty() {
        format!("出了 {ok} 个风格")
    } else {
        format!("出了 {ok} 个，{} 个没出来", failed.len())
    };
    if attempted < total {
        format!("{head}（中途停止，还有 {} 个没轮到）", total - attempted)
    } else {
        head
    }
}

/// 风格页「用本片试」
#[tauri::command]
pub fn style_compare_start(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
    project_id: String,
    spec: CompareSpec,
) -> Result<jobs::Job> {
    start(Some(app), &state.config_dir, &project_id, spec)
}

/// 风格页读已出的对照图
#[tauri::command]
pub fn style_compare_list(
    state: tauri::State<'_, crate::AppState>,
    project_id: String,
) -> Result<Vec<(String, String)>> {
    Ok(existing(&state.config_dir, &project_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> CompareSpec {
        CompareSpec {
            shot_prompt: "a swordsman on a rooftop at dusk".into(),
            preset_ids: vec![],
            width: 1024,
            height: 1024,
            seed: 7,
            control_image: None,
        }
    }

    /// 默认铺开全部内置风格，一个不漏。
    #[test]
    fn 默认展开全部风格() {
        assert_eq!(cells(&spec()).len(), style::all_presets().len());
    }

    /// **每一格的 seed 必须相同。** 不同 seed 的「对照」其实什么也没对照。
    #[test]
    fn 所有格子共用同一个seed() {
        for (_, g) in graphs(&spec()) {
            assert_eq!(g["8"]["inputs"]["seed"], 7);
        }
    }

    /// 每一格的提示词必须真的带上**自己那个**风格的词，而且互不相同。
    #[test]
    fn 每格带各自的风格词() {
        let cs = cells(&spec());
        for c in &cs {
            let p = style::find_preset(&c.preset_id).unwrap();
            assert!(
                c.prompt.contains(p.positive_prompt.trim()),
                "{} 的提示词里没有它自己的风格词",
                c.name_zh
            );
            assert!(c.prompt.contains("swordsman"), "{} 把画面主体弄丢了", c.name_zh);
        }
        let mut seen = std::collections::HashSet::new();
        for c in &cs {
            assert!(seen.insert(c.prompt.clone()), "{} 和别人的提示词一模一样", c.name_zh);
        }
    }

    /// 锁构图模式：每一格都要挂上 ControlNet，而且用的是**同一张**控制图。
    /// 漏挂一格，那一格的构图就会自己长 —— 而且不报错。
    #[test]
    fn 锁构图时每格都挂上controlnet() {
        let mut s = spec();
        s.control_image = Some("lock.png".into());
        let gs = graphs(&s);
        assert!(!gs.is_empty());
        for (c, g) in gs {
            assert_eq!(
                g["8"]["inputs"]["model"],
                serde_json::json!(["22", 0]),
                "{} 没挂 ControlNet",
                c.name_zh
            );
            assert_eq!(g["21"]["inputs"]["image"], "lock.png");
        }
    }

    /// 自由模式不许偷偷挂上 ControlNet。
    #[test]
    fn 自由模式不挂controlnet() {
        for (_, g) in graphs(&spec()) {
            assert!(g.get("22").is_none(), "自由模式不该有 ControlNet 节点");
        }
    }

    /// 文件名前缀要能认出是哪个风格 —— 不然出完一堆图分不清谁是谁。
    #[test]
    fn 文件名带得出风格id() {
        for (c, g) in graphs(&spec()) {
            assert!(c.filename_prefix.contains(&c.preset_id));
            assert_eq!(g["10"]["inputs"]["filename_prefix"], c.filename_prefix);
        }
    }

    /// 出的图必须落在**项目目录**下，不能污染内置的 `resources/style_thumbs/`
    /// —— 那个目录进版本库，是全局通用样例，混进某个项目的画面换项目还看得见。
    #[test]
    fn 结果落在项目目录不碰内置资源() {
        let d = tempfile::tempdir().unwrap();
        let p = dir(d.path(), "proj_a");
        let s = p.to_string_lossy();
        assert!(s.contains("proj_a"), "路径里没有项目 id：{s}");
        assert!(s.ends_with("style_compare"));
        assert!(!s.contains("style_thumbs"), "不能写进内置缩略图目录");
        assert!(!s.contains("resources"), "不能写进 resources");
        // 不同项目互不干扰
        assert_ne!(p, dir(d.path(), "proj_b"));
    }

    /// 还没出过时返回空，不报错。
    #[test]
    fn 没出过时返回空() {
        let d = tempfile::tempdir().unwrap();
        assert!(existing(d.path(), "nope").is_empty());
    }

    /// 只认 png，而且 key 是风格 id。
    #[test]
    fn 读回来的是风格id对路径() {
        let d = tempfile::tempdir().unwrap();
        let p = dir(d.path(), "proj_a");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("cinematic_realism.png"), b"x").unwrap();
        std::fs::write(p.join("graphic_novel.png"), b"x").unwrap();
        std::fs::write(p.join("readme.txt"), b"x").unwrap();
        let got = existing(d.path(), "proj_a");
        assert_eq!(got.len(), 2, "txt 不该被算进来");
        assert_eq!(got[0].0, "cinematic_realism");
        assert!(got[0].1.ends_with("cinematic_realism.png"));
    }

    /// 空提示词直接拒绝 —— 风格广场的意义就是拿**本片的画面**去试。
    #[test]
    fn 空提示词拒绝开跑() {
        let d = tempfile::tempdir().unwrap();
        let mut sp = spec();
        sp.shot_prompt = "   ".into();
        let err = start(None, d.path(), "p1", sp).unwrap_err().to_string();
        assert!(err.contains("镜头"), "错误提示没说清要干嘛：{err}");
    }

    /// 人眼验收用：把对照图的 graph 导出成 JSON，拿去 ComfyUI 实跑。
    /// 默认 `#[ignore]` —— 它写文件，不是断言。
    #[test]
    #[ignore]
    fn 导出对照graph供实跑() {
        let s = CompareSpec {
            shot_prompt: "a lone swordsman standing on a stone plaza at dusk, a tower behind him"
                .into(),
            preset_ids: vec![
                "cinematic_realism".into(),
                "modern_cel_anime".into(),
                "chinese_ink_fantasy".into(),
                "graphic_novel".into(),
            ],
            width: 1024,
            height: 1024,
            seed: 424242,
            control_image: Some("anima_depth_full_duo.png".into()),
        };
        let out: Vec<_> = graphs(&s)
            .into_iter()
            .map(|(c, g)| serde_json::json!({"cell": c, "graph": g}))
            .collect();
        let path = std::env::temp_dir().join("anima_style_compare.json");
        std::fs::write(&path, serde_json::to_string(&out).unwrap()).unwrap();
        println!("{}", path.display());
    }

    /// 指定子集时只出那几个，且顺序稳定。
    #[test]
    fn 可以只比一部分() {
        let mut s = spec();
        s.preset_ids = vec!["cinematic_realism".into(), "chinese_ink_fantasy".into()];
        let cs = cells(&s);
        assert_eq!(cs.len(), 2);
        assert_eq!(cs[0].preset_id, "cinematic_realism");
        assert_eq!(cs[1].preset_id, "chinese_ink_fantasy");
    }
}
