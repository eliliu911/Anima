//! 一次宫格出多镜草图。
//!
//! # 和场景宫格是同一套几何、相反的语义
//!
//! 几何（行列、切分、像素预算、[`detect_grid`]）和
//! [`super::scene_sheet`] 完全共用 —— 那边已经把坑踩过一遍了，
//! 尤其是「模型不一定听行列数，要按图里实际画成什么样切」。
//!
//! 语义正好相反：
//!
//! | | 场景宫格 | 分镜宫格 |
//! |---|---|---|
//! | 一格是什么 | 同一空间的**不同机位** | 不同时刻的**不同镜头** |
//! | 必须一样 | 空间、地理、光源位置 | 画风、色调、角色长相 |
//! | 必须不一样 | 机位、可见区域 | 时刻、构图、景别 |
//!
//! 所以提示词不能照抄 —— 场景宫格反复强调「所有格子是同一个空间」，
//! 这边要是也那么说，九个镜头会出成同一个房间的九个角度。
//!
//! # 价值在哪：跨镜画风一致
//!
//! 逐张生成时，画风和光线基调**每张都在重新抽签** —— 第 3 镜偏冷、
//! 第 7 镜偏暖，各自单看都好，剪在一起就不是一部片子。
//! 同一次采样里画出来的九格天然同源。
//!
//! # 这是**草图**，不是关键帧
//!
//! 本机 3×3 每格 512×341，脸只有几十像素 —— 和设定卡那次一样的账。
//! 它能定构图、景别、人物站位和光线走向，**定不了长相**。
//! 关键帧要在草图基础上逐格 i2i 放大精修。

use super::scene_sheet::{Grid, PixelBudget};

/// 一格对应一个镜头。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShotCell {
    /// 镜头号，用来把切出来的图分配回去。**不是格子序号** ——
    /// 分页之后第二张的第 1 格是第 10 镜。
    pub shot_num: u32,
    /// 镜头 id，切出来的文件按它命名。
    pub shot_id: String,
    /// 景别（已经是词表里那句话）。
    pub scale: String,
    /// 这一格画什么。**静态描述** —— 草图是一张图，动作写进去只会浪费篇幅。
    pub visual: String,
}

/// 一次宫格最多放几个镜头。
///
/// 9 是 3×3。再多要分页 —— 不是因为画不下，是因为**每格会小到没用**：
/// 4×4 在 1536×1024 上每格只有 384×256，构图都读不准了。
pub const MAX_PER_SHEET: usize = 9;

/// 把镜头分页。每页最多 [`MAX_PER_SHEET`] 个。
pub fn paginate(shots: &[ShotCell]) -> Vec<Vec<ShotCell>> {
    if shots.is_empty() {
        return vec![];
    }
    shots.chunks(MAX_PER_SHEET).map(<[ShotCell]>::to_vec).collect()
}

/// 拼分镜宫格的提示词。
///
/// `style` 是整部片子的画风定调 —— 它**必须写在最前**并且明确说「每格都一样」，
/// 因为跨镜画风一致正是这条路唯一的价值。
pub fn grid_prompt(style: &str, shots: &[ShotCell], grid: Grid) -> String {
    // **版式用英文的摄影术语起头。** 和场景宫格同一个道理：
    // `contact sheet` / `clean white borders separating panels` 是真实的
    // 摄影术语，训练语料里出现过，比「一张精确的 N 行 M 列网格图」更容易
    // 命中版式（2026-09-11 五组对照实测）。
    //
    // 但**光有版式措辞不够** —— 只说 `multiple camera angles` 会退化成
    // 九张几乎一样的图。逐格点名那一段才是区分度的来源。
    let mut lines = vec![format!(
        "a {}x{} grid storyboard contact sheet, {} sequential film panels, \
         clean white borders separating panels, no text.\n\
         这是同一部片子的 {} 个连续分镜草图。",
        grid.cols,
        grid.rows,
        grid.cells(),
        shots.len()
    )];

    let style = style.trim();
    if !style.is_empty() {
        lines.push(format!(
            "整体画风（**每一格都必须一样**）：{style}"
        ));
    }

    // **每格逐个点名，而且带景别。**
    //
    // 只说「不同镜头」的话模型会画出九张差不多的图。景别是最容易被忽略
    // 又最影响可读性的那一项 —— 九格全是中景等于没有分镜。
    for (i, s) in shots.iter().enumerate() {
        let scale = s.scale.trim();
        let visual = s.visual.trim();
        let head = format!("第 {} 格（镜头 {}）", i + 1, s.shot_num);
        lines.push(match (scale.is_empty(), visual.is_empty()) {
            (true, true) => format!("{head}：留空"),
            (false, true) => format!("{head}：{scale}"),
            (true, false) => format!("{head}：{visual}"),
            (false, false) => format!("{head}：{scale}，{visual}"),
        });
    }

    let blanks = grid.cells() as usize - shots.len().min(grid.cells() as usize);
    if blanks > 0 {
        lines.push(format!("剩下 {blanks} 个格子留空白，不要填充内容。"));
    }

    // **这里和场景宫格说的是相反的事。**
    //
    // 场景宫格要「所有格子是同一个空间」；这边九格是九个不同时刻，
    // 照抄那句会让它出成同一个房间的九个角度。
    lines.push(
        "这九格是**不同时刻的不同镜头**，空间、构图和景别本来就该各不相同。\
         必须保持一致的只有三样：画风与笔触、整体色调与光线基调、\
         同一个角色的长相与服装。"
            .into(),
    );
    lines.push(
        "格子之间用细白线分隔，不要画边框装饰、不要给格子加编号或标题，\
         **画面里不要出现任何文字**。"
            .into(),
    );
    lines.join("\n")
}

/// 草图能不能直接当关键帧用。
///
/// **基本都是不能。** 这个函数存在的意义是把那句话说出口 ——
/// 设定卡那次就是没人算这笔账。
pub fn sketch_note(budget: PixelBudget) -> String {
    format!(
        "{}\n这是**分镜草图**：构图、景别、站位和光线走向可信，\
         脸和材质不可信（每格的人脸通常只有几十像素）。\
         要当关键帧用，得在草图基础上逐格 i2i 放大精修。",
        budget.note()
    )
}

/// 打真实 ComfyUI 的验证。**默认 `#[ignore]`**。
///
/// ```bash
/// cd src-tauri && cargo test --lib shot_sheet_live -- --ignored --nocapture
/// ```
///
/// **要看的是跨镜画风一致性** —— 那是这条路唯一的价值。
/// 九格构图各不相同是应该的；九格画风、色调、人物长相要是不一致，
/// 这条路就没有意义。
#[cfg(test)]
mod live {
    use super::*;
    use crate::agent_runtime::scene_sheet::{detect_grid, grid_for, split_sheet, Viewpoint};

    #[tokio::test]
    #[ignore = "要本机 ComfyUI"]
    async fn shot_sheet_live_出九宫格分镜草图() {
        let shots = vec![
            cell_of(1, "远景", "破旧大巴停在乡间公路边，扬起尘土"),
            cell_of(2, "中景", "年轻女子拖着行李箱从车门走下，四下张望"),
            cell_of(3, "特写", "她的手握紧行李箱拉杆，指节发白"),
            cell_of(4, "全景", "田埂上一个老农弯腰拔草，远处是青山"),
            cell_of(5, "中景", "她走近田埂，站在老农身后开口"),
            cell_of(6, "特写", "老农抬头，眯眼打量她，脸上沟壑纵横"),
            cell_of(7, "近景", "两人隔着田埂对视，她微微低头"),
            cell_of(8, "中景", "老农抬手指向路的尽头"),
            cell_of(9, "远景", "她拖着箱子沿路远去，风吹乱头发"),
        ];
        let grid = grid_for(shots.len());
        let (w, h) = (1536u32, 1024u32);
        let prompt = grid_prompt("写实电影感，冷调，自然光，胶片颗粒", &shots, grid);
        println!("\n=== 分镜宫格提示词 ===\n{prompt}\n");
        println!(
            "=== 草图说明 ===\n{}\n",
            sketch_note(PixelBudget::of(w, h, grid))
        );

        let out = std::path::PathBuf::from("/tmp/anima-shot-sheet/sheet.png");
        std::fs::create_dir_all(out.parent().unwrap()).unwrap();
        let graph =
            crate::agent_runtime::image_engine::build_zimage_graph(&prompt, w, h, 20_260_912, 8);

        println!("提交中（Z-Image 约 45 秒）…");
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        const URL: &str = "http://127.0.0.1:8188";
        let res: serde_json::Value = client
            .post(format!("{URL}/prompt"))
            .json(&serde_json::json!({ "prompt": graph }))
            .send().await.expect("提交失败")
            .json().await.expect("提交返回不是 JSON");
        let id = res["prompt_id"].as_str().expect("没拿到 prompt_id").to_string();

        let mut hist = serde_json::Value::Null;
        for _ in 0..600 {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            let h: serde_json::Value = client
                .get(format!("{URL}/history/{id}"))
                .send().await.expect("查历史失败")
                .json().await.unwrap_or(serde_json::Value::Null);
            if h.get(&id).is_some() {
                hist = h;
                break;
            }
        }
        let entry = hist.get(&id).expect("等超时了");
        let img = entry["outputs"].as_object()
            .and_then(|o| o.values().find_map(|v| v["images"].as_array()?.first().cloned()))
            .expect("历史里没有图");
        let bytes = client
            .get(format!("{URL}/view?filename={}&subfolder={}&type={}",
                img["filename"].as_str().unwrap_or(""),
                img["subfolder"].as_str().unwrap_or(""),
                img["type"].as_str().unwrap_or("output")))
            .send().await.expect("取图失败")
            .bytes().await.expect("取图失败");
        std::fs::write(&out, &bytes).unwrap();
        println!("联合图：{} ({:.1} MB)", out.display(), bytes.len() as f64 / 1e6);

        // **按实测布局切** —— 模型不一定听行列数，场景宫格那边已经撞过
        let actual = detect_grid(&image::open(&out).unwrap());
        println!("实测布局：{actual:?}（要求 {grid:?}）");

        let views: Vec<Viewpoint> = shots
            .iter()
            .map(|s| Viewpoint::new(format!("镜{}", s.shot_num), ""))
            .collect();
        match split_sheet(&out, grid, &views) {
            Ok(cells) => {
                for c in &cells {
                    println!("  {}", c.display());
                }
                println!("\n**要看的是跨镜画风一致性** —— 构图各不相同是应该的，");
                println!("画风、色调、人物长相不一致才是这条路失败。");
            }
            Err(e) => println!("切分没做：{e}"),
        }
    }

    fn cell_of(n: u32, scale: &str, visual: &str) -> ShotCell {
        ShotCell {
            shot_num: n,
            shot_id: format!("s{n}"),
            scale: scale.into(),
            visual: visual.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_runtime::scene_sheet::grid_for;

    fn cell(n: u32, scale: &str, visual: &str) -> ShotCell {
        ShotCell {
            shot_num: n,
            shot_id: format!("s{n}"),
            scale: scale.into(),
            visual: visual.into(),
        }
    }

    #[test]
    fn 超过九个镜头分页() {
        let shots: Vec<ShotCell> = (1..=20).map(|n| cell(n, "中景", "x")).collect();
        let pages = paginate(&shots);
        assert_eq!(pages.len(), 3);
        assert_eq!(pages[0].len(), 9);
        assert_eq!(pages[2].len(), 2);
        // **分页之后格子序号和镜头号不是一回事**
        assert_eq!(pages[1][0].shot_num, 10, "第二页第一格该是第 10 镜");
        assert!(paginate(&[]).is_empty());
    }

    /// **这里和场景宫格说的是相反的事。**
    ///
    /// 场景宫格要「所有格子是同一个空间」；照抄那句，九个镜头会出成
    /// 同一个房间的九个角度。
    #[test]
    fn 明说九格是不同时刻而不是同一个空间() {
        let shots: Vec<ShotCell> = (1..=4).map(|n| cell(n, "中景", "x")).collect();
        let p = grid_prompt("水彩插画", &shots, grid_for(4));
        assert!(p.contains("不同时刻的不同镜头"), "{p}");
        assert!(p.contains("空间、构图和景别本来就该各不相同"), "{p}");
        assert!(!p.contains("同一个空间"), "照抄了场景宫格那句：{p}");
    }

    /// 画风要写在最前，而且明说每格都一样 —— 跨镜画风一致是这条路唯一的价值。
    #[test]
    fn 画风写在最前并明说每格都一样() {
        let shots = vec![cell(1, "特写", "她的眼睛")];
        let p = grid_prompt("冷调写实，胶片颗粒", &shots, grid_for(1));
        let style_at = p.find("冷调写实").unwrap();
        let cell_at = p.find("第 1 格").unwrap();
        assert!(style_at < cell_at, "画风该排在格子前面：{p}");
        // 版式用英文摄影术语起头 —— 实测比中文「N 行 M 列」更容易命中
        assert!(p.starts_with("a 1x1 grid storyboard contact sheet"), "{p}");
        assert!(p.contains("每一格都必须一样"), "{p}");
    }

    /// 每格带景别。**九格全是中景等于没有分镜。**
    #[test]
    fn 每格带上景别和镜头号() {
        let shots = vec![cell(3, "特写", "她的手"), cell(4, "全景", "整条街")];
        let p = grid_prompt("", &shots, grid_for(2));
        assert!(p.contains("第 1 格（镜头 3）：特写，她的手"), "{p}");
        assert!(p.contains("第 2 格（镜头 4）：全景，整条街"), "{p}");
    }

    #[test]
    fn 空字段不产生半句话() {
        let shots = vec![
            cell(1, "", "只有画面"),
            cell(2, "中景", ""),
            cell(3, "", ""),
        ];
        let p = grid_prompt("", &shots, grid_for(3));
        assert!(p.contains("第 1 格（镜头 1）：只有画面"), "{p}");
        assert!(p.contains("第 2 格（镜头 2）：中景"), "{p}");
        assert!(p.contains("第 3 格（镜头 3）：留空"), "{p}");
        assert!(!p.contains("：，"), "拼出了半句话：{p}");
    }

    #[test]
    fn 格子没填满时要求留空() {
        let shots: Vec<ShotCell> = (1..=7).map(|n| cell(n, "中景", "x")).collect();
        let p = grid_prompt("", &shots, Grid { rows: 3, cols: 3 });
        assert!(p.contains("剩下 2 个格子留空白"), "{p}");
    }

    /// **草图不是关键帧，这句话必须说出口。**
    /// 设定卡那次就是没人算这笔账。
    #[test]
    fn 草图说明点出脸不可信() {
        let note = sketch_note(PixelBudget::of(1536, 1024, Grid { rows: 3, cols: 3 }));
        assert!(note.contains("分镜草图"), "{note}");
        assert!(note.contains("脸和材质不可信"), "{note}");
        assert!(note.contains("精修"), "得给下一步：{note}");
        // 像素预算那一层也要在
        assert!(note.contains("512"), "{note}");
    }
}
