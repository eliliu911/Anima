//! 空间契约 → 宫格 → 切格 → 色温核对，**端到端跑一次真图**。
//!
//! 单元测试只证明了「契约编译出了那段文字」，证明不了**出来的九个格子
//! 是同一个环境**。这一块所有的失败都不报错：视角描述太笼统只是让两格
//! 出成同一个机位，地理错位只是让柜台在这一格有下一格没有 ——
//! 两种都能正常跑完全部采样步，任务状态都是 `success`。
//!
//! ```bash
//! cargo test --test scene_contract_live -- --ignored --nocapture
//! ```
//!
//! **判定「跑通了」必须真的看图。** 这个测试只做能自动做的那几项
//! （版式对不对、切得出几格、色温散不散），剩下的它把文件路径打出来。

use anima_lib::agent_runtime::scene_sheet;
use anima_lib::scene_contract::{Anchor, AnchorSize, Entrance, SceneContract, SceneView};

fn 面馆() -> SceneContract {
    SceneContract {
        shape: "东西进深的狭长一层旧面馆，层高偏矮".into(),
        zones: vec![
            "东侧卷帘门入口".into(),
            "南侧两排客桌".into(),
            "北侧柜台".into(),
            "柜台后的后厨".into(),
        ],
        entrances: vec![Entrance {
            name: "东侧卷帘门".into(),
            connects_to: "临街人行道".into(),
        }],
        anchors: vec![
            Anchor {
                name: "整面北墙的旧木柜台".into(),
                zone: "北墙".into(),
                size: AnchorSize::WallDominant,
                must_not_become: vec!["现代吧台".into(), "玻璃展示柜".into()],
            },
            Anchor {
                name: "柜台正后方的后厨出餐口".into(),
                zone: "北墙柜台后".into(),
                size: AnchorSize::Medium,
                must_not_become: vec![],
            },
            Anchor {
                name: "南墙的手写菜单板".into(),
                zone: "南墙".into(),
                size: AnchorSize::Small,
                must_not_become: vec![],
            },
        ],
        relations: vec![
            "后厨出餐口在柜台正后方".into(),
            "卷帘门和柜台隔着整条店堂正对".into(),
        ],
        materials: vec!["磨损的深色木柜台".into(), "浅米色旧瓷砖墙".into(), "灰绿色水磨石地面".into()],
        views: vec![
            SceneView {
                name: "入口正打".into(),
                from: "东侧卷帘门内侧".into(),
                toward: "店堂深处".into(),
                far: "靠北墙的整面旧木柜台".into(),
                visible: vec!["整面北墙的旧木柜台".into()],
                light: "主光来自身后的街口，柜台区靠暖色吊灯补光".into(),
            },
            SceneView {
                name: "柜台反打".into(),
                from: "柜台内侧".into(),
                toward: "东侧卷帘门".into(),
                far: "卷帘门和门外的街".into(),
                visible: vec!["南墙的手写菜单板".into()],
                light: "同一个街口光，现在从正面来，柜台这一侧压暗".into(),
            },
            SceneView {
                name: "客桌侧看".into(),
                from: "南侧客桌之间".into(),
                toward: "北侧柜台".into(),
                far: "柜台后的后厨出餐口".into(),
                visible: vec!["柜台正后方的后厨出餐口".into()],
                light: "暖色吊灯为主，街口光斜着从画面右后方进来".into(),
            },
            SceneView {
                name: "出餐口近处".into(),
                from: "柜台外侧靠北".into(),
                toward: "后厨出餐口".into(),
                far: "出餐口里的后厨灶台".into(),
                visible: vec!["整面北墙的旧木柜台".into()],
                light: "出餐口里透出的白色厨房光，和店堂的暖光形成冷暖对比".into(),
            },
        ],
        inferred: vec!["天花板的管线走向剧本没写".into()],
        no_extra: vec!["第二扇门".into(), "楼梯".into(), "任何招牌文字".into()],
        empty_stage: anima_lib::scene_contract::EmptyStage::Required,
        pano_calibration: None,
        world: None,
    }
}

const COMFY: &str = "http://127.0.0.1:8188";

#[tokio::test]
#[ignore = "需要本机 ComfyUI（带 --fp32-unet）+ Z-Image Turbo"]
async fn 契约出的宫格能切开而且色温不散() {
    let c = 面馆();
    assert!(c.issues().is_empty(), "契约本身就有问题：{:?}", c.issues());

    let views = c.viewpoints();
    let grid = scene_sheet::grid_for(views.len());
    let prompt = scene_sheet::grid_prompt(
        "安和面馆",
        &c.to_prompt_block(""),  // 名字不发 —— 会被画成招牌
        &views,
        grid,
    );
    eprintln!("=== 提示词（{} 字）===\n{prompt}\n", prompt.chars().count());

    let (w, h) = (1536u32, 1024u32);
    let seed = 20260911;
    let graph = anima_lib::agent_runtime::image_engine::build_zimage_graph(&prompt, w, h, seed, 8);

    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let bytes = render(&http, graph).await.expect("出图失败");

    let out_dir = std::env::temp_dir().join("anima_scene_contract_live");
    std::fs::create_dir_all(&out_dir).unwrap();
    let sheet = out_dir.join("sheet.png");
    std::fs::write(&sheet, &bytes).unwrap();
    eprintln!("联合图：{}", sheet.display());

    let img = image::load_from_memory(&bytes).unwrap();
    let detected = scene_sheet::detect_grid(&img);
    eprintln!("要的版式 {}x{}，量出来 {detected:?}", grid.cols, grid.rows);

    let cells = scene_sheet::split_sheet(&sheet, grid, &views).expect("切不开");
    for p in &cells {
        eprintln!("  格子：{}", p.display());
    }
    assert_eq!(cells.len(), views.len(), "格子数和视角数对不上");

    let temp = anima_lib::cinema_qc::check_color_temp_paths(&cells);
    for cell in &temp.cells {
        eprintln!("  {} 色温比 {:.3}", cell.name, cell.ratio);
    }
    eprintln!("色温离散 {:.1}%", temp.spread * 100.0);
    match &temp.note {
        Some(n) => eprintln!("⚠️ {n}"),
        None => eprintln!("色温核对通过"),
    }

    let budget = scene_sheet::PixelBudget::of(w, h, grid);
    eprintln!("{}", budget.note());
    eprintln!("\n**去看图**：{}", out_dir.display());
}

/// 提交一张图并等它出来。轮询上限按本机 Z-Image 的实测耗时给（45s）留足余量。
async fn render(http: &reqwest::Client, graph: serde_json::Value) -> Result<Vec<u8>, String> {
    let body = serde_json::json!({ "prompt": graph });
    let r: serde_json::Value = http
        .post(format!("{COMFY}/prompt"))
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let id = r
        .get("prompt_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("提交被拒：{r}"))?
        .to_string();
    eprintln!("提交了 {id}，等出图…");

    for _ in 0..600 {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        let h: serde_json::Value = http
            .get(format!("{COMFY}/history/{id}"))
            .send()
            .await
            .map_err(|e| e.to_string())?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        let Some(entry) = h.get(&id) else { continue };
        let images = entry
            .pointer("/outputs")
            .and_then(|o| o.as_object())
            .and_then(|o| o.values().find_map(|v| v.get("images")))
            .and_then(|v| v.as_array());
        let Some(first) = images.and_then(|a| a.first()) else {
            continue;
        };
        let name = first.get("filename").and_then(|v| v.as_str()).unwrap_or("");
        let sub = first.get("subfolder").and_then(|v| v.as_str()).unwrap_or("");
        let kind = first.get("type").and_then(|v| v.as_str()).unwrap_or("output");
        let bytes = http
            .get(format!(
                "{COMFY}/view?filename={}&subfolder={}&type={kind}",
                urlencoding(name),
                urlencoding(sub)
            ))
            .send()
            .await
            .map_err(|e| e.to_string())?
            .bytes()
            .await
            .map_err(|e| e.to_string())?;
        return Ok(bytes.to_vec());
    }
    Err("等超时了".into())
}

/// 只够用的 URL 编码。ComfyUI 的文件名是它自己生成的（`ComfyUI_00001_.png`），
/// 不会有奇怪字符 —— 为这个拉一个依赖不值。
fn urlencoding(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' | '/' => c.to_string(),
            _ => format!("%{:02X}", c as u32),
        })
        .collect()
}

/// 出一张真全景，给前端的查看器 / 导演世界当输入。
///
/// 提示词由前端的 `panoramaPlan` 编出来（`PANO_PLAN` 指向它写的 JSON）——
/// **不在这儿重写一份**：重写的那份必然和界面用的分叉，而分叉了不报错。
///
/// ```bash
/// PANO_PLAN=/tmp/pano_plan.json PANO_OUT=/tmp/pano.png \
///   cargo test --test scene_contract_live 全景 -- --ignored --nocapture
/// ```
#[tokio::test]
#[ignore = "需要本机 ComfyUI + Z-Image；提示词由前端编译器给"]
async fn 全景出图给查看器用() {
    let plan_path = std::env::var("PANO_PLAN").expect("要 PANO_PLAN");
    let raw = std::fs::read_to_string(&plan_path).unwrap();
    let plan: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let prompt = plan["prompt"].as_str().unwrap();
    let w = plan["width"].as_u64().unwrap() as u32;
    let h = plan["height"].as_u64().unwrap() as u32;
    assert_eq!(w, h * 2, "等距圆柱必须 2:1");
    eprintln!("{w}×{h}，提示词 {} 字", prompt.chars().count());

    let graph = anima_lib::agent_runtime::image_engine::build_zimage_graph(prompt, w, h, 20260911, 8);
    let http = reqwest::Client::builder().no_proxy().build().unwrap();
    let bytes = render(&http, graph).await.expect("出图失败");

    let out = std::env::var("PANO_OUT").unwrap_or_else(|_| "/tmp/anima_pano.png".into());
    std::fs::write(&out, &bytes).unwrap();
    let img = image::load_from_memory(&bytes).unwrap();
    eprintln!("落盘 {out}  实际 {}×{}", img.width(), img.height());

    // **接缝要能对上。** 等距圆柱的左右边缘在球面上是同一条线 ——
    // 差太多的话转到正后方会看见一条明显的接缝。
    // 这里只做最粗的核对：左右各一列的平均色差。
    let rgb = img.to_rgb8();
    let col = |x: u32| {
        let mut acc = [0u64; 3];
        for y in 0..rgb.height() {
            let p = rgb.get_pixel(x, y).0;
            for i in 0..3 { acc[i] += p[i] as u64; }
        }
        acc.map(|v| (v / rgb.height() as u64) as i64)
    };
    let (l, r) = (col(0), col(rgb.width() - 1));
    let diff: i64 = (0..3).map(|i| (l[i] - r[i]).abs()).sum::<i64>() / 3;
    eprintln!("左右边缘平均色差 {diff}/255 —— 越小接缝越顺");
}
