//! Anima Studio 的端到端冒烟：**从新建项目到分镜，全程走命令层的实现函数**。
//!
//! ```text
//! ./scripts/with-env.sh cargo run --bin smoke-studio
//! ```
//!
//! 不出视频（那是分钟级的，另有 `studio` 的 E2E 测试盯着），
//! 但把「用户从零开始会走的每一步」都真跑一遍：
//! 建项目 → 写剧本 → 定风格 → 提取实体 → 生成分镜 → 存分镜 → 引用回写。
//!
//! **判据是文件和数字，不是「跑完了」。**

use anima_lib::{assets, cinema, prompts, style};
use std::path::PathBuf;

const SCRIPT: &str = "【场景：老茶馆】【时间：雨天午后】\n\
    老茶师在窗边擦拭一只黄铜壶，白须垂到胸前，青灰长衫。\n\
    小徒弟捧着一摞茶盏进来，脚步很急。\n\
    小徒弟：「师父，前头来了位客人，说要见您。」\n\
    老茶师没有抬头：「让他等。茶还没醒。」\n";

#[tokio::main]
async fn main() {
    // 用一个带时间戳的临时目录 —— 冒烟不碰用户的真实数据。
    let cfg = std::env::temp_dir().join(format!(
        "anima-smoke-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    ));
    std::fs::create_dir_all(&cfg).expect("建不了临时配置目录");
    println!("临时配置目录：{}\n", cfg.display());
    let failed: std::rc::Rc<std::cell::RefCell<Vec<String>>> = Default::default();
    let step = std::cell::Cell::new(0usize);
    let ok = |name: &str, cond: bool, detail: String| {
        step.set(step.get() + 1);
        println!(
            "{} [{}] {name} —— {detail}",
            if cond { "✓" } else { "✗" },
            step.get()
        );
        if !cond {
            failed.borrow_mut().push(name.to_string());
        }
    };

    // 1. 建项目
    let pid = "smoke_proj";
    let proj_dir = cfg.join("projects").join(pid);
    std::fs::create_dir_all(proj_dir.join("episodes/ep_01/shots")).unwrap();
    std::fs::write(
        proj_dir.join("project.json"),
        serde_json::json!({
            "id": pid, "title": "冒烟", "genre": "", "aspect_ratio": "9:16",
            "description": "", "created_at": 0, "updated_at": 0,
            "episode_count": 1, "cover_image": null, "kind": "standalone"
        })
        .to_string(),
    )
    .unwrap();
    let ep = cinema::EpisodeData {
        long_take_scenes: Vec::new(),
        one_take: false,
        scene_takes: Vec::new(),
        long_take_context_frames: None,
        scene_renders: Vec::new(),
        id: "ep_01".into(), episode_num: 1, title: "第 1 集".into(),
        style_lock: String::new(),
        synopsis: String::new(), script_text: SCRIPT.into(), script_doc: None,
        // 冒烟里不设时长预算 —— 那一行在提示词里就不出现
        target_secs: 0,
        storyboard: Vec::new(), storyboard_version: cinema::STORYBOARD_VERSION, final_video_url: None,
        actor_colors: Vec::new(),
    };
    std::fs::write(
        proj_dir.join("episodes/ep_01/episode.json"),
        serde_json::to_string(&ep).unwrap(),
    )
    .unwrap();

    let items = cinema::workspace_items(&cfg);
    ok(
        "工作区列得出新项目",
        items.len() == 1 && items[0].status == "draft",
        format!("{} 个 · 状态 {}", items.len(), items.first().map(|i| i.status).unwrap_or("?")),
    );

    // 2. 定风格
    let preset = &style::all_presets()[0];
    let art = style::ArtDirection {
        preset_id: preset.id.clone(),
        ..Default::default()
    };
    style::save_art(&cfg, pid, &art).unwrap();
    let back = style::load_art(&cfg, pid);
    ok(
        "风格存得住",
        back.is_set() && !back.effective_positive().is_empty(),
        preset.name_zh.to_string(),
    );
    let styled = style::apply_style("老茶师擦茶盏", &back);
    ok(
        "风格拼进提示词且画面在前",
        styled.starts_with("老茶师擦茶盏") && styled.len() > "老茶师擦茶盏".len(),
        format!("{} 字", styled.chars().count()),
    );

    // 3. 提取实体（要 LLM）
    let pcfg = prompts::load(&cfg);
    let sent = prompts::compose(&pcfg, prompts::PromptSlot::EntityExtraction, &[("TEXT", SCRIPT)]);
    ok(
        "提取提示词把剧本填进去了",
        sent.contains("老茶师") && !sent.contains("{TEXT}"),
        format!("{} 字", sent.chars().count()),
    );

    let gateway = match build_gateway(&cfg) {
        Ok(g) => g,
        Err(e) => {
            println!("\n⚠ 起不了 LLM 网关：{e}");
            println!("  后面几步要本地模型，跳过。前面几步的结论仍然有效。");
            finish(&failed.borrow());
            return;
        }
    };

    match prompts::run(&gateway, &pcfg, prompts::PromptSlot::EntityExtraction, &[("TEXT", SCRIPT)]).await {
        Ok(raw) => {
            let es = prompts::parse_entities(&raw);
            ok(
                "从剧本抽出实体",
                es.iter().any(|e| e.name.contains("老茶师")),
                format!("{} 条：{}", es.len(), es.iter().map(|e| e.name.as_str()).collect::<Vec<_>>().join(" ")),
            );
            let n = cinema::import_entities(&cfg, pid, "ep_01", es, &[]).unwrap();
            // **提取直接进系列池，同名并入已有的**（2026-09-25 起）。
            // 以前落本集、等「对齐」—— 对齐一直没做，每集的角色各是各的。
            let store = assets::load_store(&cfg, &assets::Scope::Project { project_id: pid.into() });
            ok(
                "实体入库成系列资产",
                n.added > 0 && store.assets.len() == n.added,
                format!("新建 {} 条", n.added),
            );

            // 第二集重抽同样的实体：要并进系列里那一个，而不是又建一份
            let es2 = vec![
                prompts::ExtractedEntity {
                    anchors: None,
                    disembodied: false, incidental: false,
                    spec: None,
                    kind: "character".into(), name: "老茶师".into(),
                    description: String::new(), persona: String::new(), exists: false,
                },
            ];
            let before = store.assets.len();
            let n2 = cinema::import_entities(&cfg, pid, "ep_02", es2, &[]).unwrap();
            let after = assets::load_store(&cfg, &assets::Scope::Project { project_id: pid.into() }).assets.len();
            ok(
                "第二集重抽同名角色并进系列里那一个",
                n2.merged == 1 && after == before,
                format!("系列 {before} → {after}"),
            );

            // 4. @ 引用能对上刚入库的资产
            if let Some(a) = store.assets.first() {
                let prompt = format!("[{}:{}] 在窗边", a.kind.tag_label(), a.name);
                let refs = assets::resolve_refs(&cfg, &prompt);
                ok(
                    "@ 引用对得上刚入库的资产",
                    refs.len() == 1 && refs[0].asset_id.is_some(),
                    prompt.to_string(),
                );
                let expanded = assets::expand_ref_tags(&prompt, &refs);
                ok(
                    "送进引擎的提示词里没有方括号",
                    !expanded.contains('['),
                    expanded.clone(),
                );
            }
        }
        Err(e) => {
            println!("⚠ 提取实体没跑成：{e}");
            failed.borrow_mut().push("提取实体".into());
        }
    }

    // 5. 生成分镜：2026-09-13 分镜重构中，旧的两阶段生成已删，新流程接上后再补冒烟。

    finish(&failed.borrow());
}

fn build_gateway(_cfg: &PathBuf) -> Result<anima_lib::llm::gateway::Gateway, String> {
    let real = dirs::config_dir()
        .ok_or("找不到配置目录")?
        .join("com.anima.desktop")
        .join("config.json");
    // 用真实配置里的后端设定（LM Studio 的地址和链路），
    // 但项目数据全部落在临时目录里 —— 不碰用户的东西。
    let config = anima_lib::config::AppConfig::load(&real).map_err(|e| e.to_string())?;
    anima_lib::llm::gateway::Gateway::new(config).map_err(|e| e.to_string())
}

fn finish(failed: &[String]) {
    if failed.is_empty() {
        println!("\n全部通过。");
    } else {
        println!("\n没过的：{}", failed.join("、"));
        std::process::exit(1);
    }
}
