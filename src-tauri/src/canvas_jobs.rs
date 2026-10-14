//! 画布生成的后台任务。
//!
//! # 为什么要有
//!
//! 画布以前是**前端自己等结果**：点生成 → `invoke` → 一直 await 到出片。
//! 页面一重新加载（开发时改代码热更新、Cmd+R、应用重启），那个 await 就没了，
//! 节点被判「上次生成已中断」—— 而 ComfyUI 那边照样跑完，片子落在输出目录里没人认领。
//! 2026-09-14 用户在画布上跑「人物清晰」（17 分钟一条），跑一半断了，又点了一次，队列里就挂着两条。
//!
//! 现在和分镜出片一样：登记进任务表（`jobs.json`，`kind = canvas`），点完立刻回任务 id，
//! 后台跑完把**完整结果**存进任务。节点记着任务 id，页面重新加载后按 id 取回；
//! 右下角提示框和出片共用同一套事件。
//!
//! # 应用本身重启时
//!
//! 后台任务跟着进程没了。**出片（H3 / LTX）续接**（2026-10-11 起）：交给 ComfyUI 时记下结果落哪和 prompt_id
//! （`video_tool::run_weetodd` → `Job::resume`），重启后 `render_resume` 照它去 ComfyUI 收、把结果存回这条任务，
//! 节点按任务 id 取回，和没重启过一样。其余的（出图、图片处理、长镜头、还没交给 ComfyUI 的）照旧判失败并写明原因。

use serde::Deserialize;

use crate::jobs::{self, JobKind};

/// 画布能发起的几种任务。前端传 `{ kind, args }`。
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", content = "args", rename_all = "snake_case")]
pub enum CanvasTask {
    /// 出图 / 出片（和资产页、对话同一条路）
    Generate(crate::studio::GenerateArgs),
    DepthCapture(crate::depth_capture::DepthCaptureArgs),
    DepthVideo(crate::depth_capture::DepthVideoArgs),
    ImageOp(crate::canvas_ops::ImageOpArgs),
    Interpolate(crate::canvas_ops::VideoOpArgs),
    VideoUpscale(crate::canvas_ops::VideoUpscaleArgs),
    /// 画布上的长镜头：视频节点连视频节点 = 窗口链，一次提交出一个 MP4。
    /// 和分镜页那条底层是同一个 `build_h3_chain_graph`。
    LongTake(crate::canvas_ops::CanvasLongTakeArgs),
    /// 单张场景图通过 Apple SHARP 重建为 3D Gaussian Splatting 片场。
    WorldGenerate(crate::world_sharp::WorldGenerateArgs),
    PanoCube(crate::pano_cube::PanoCubeArgs),
}

#[derive(Debug, Deserialize)]
pub struct CanvasJobArgs {
    #[serde(default)]
    pub project_id: Option<String>,
    pub node_id: String,
    /// 给人看的名字，比如「动作迁移」
    pub label: String,
    pub task: CanvasTask,
}

/// 跑一个任务，回可序列化的结果。**失败时回的是给人看的原文**。
/// `gateway`：后台模型（超长视频的提示词没写时间时拆段用，见 [`crate::long_video`]）。没有就只能按时间码拆
pub async fn run(config_dir: &std::path::Path, gateway: Option<std::sync::Arc<crate::Gateway>>, task: CanvasTask) -> std::result::Result<serde_json::Value, String> {
    fn to_json<T: serde::Serialize>(v: T) -> std::result::Result<serde_json::Value, String> {
        serde_json::to_value(v).map_err(|e| format!("结果序列化失败：{e}"))
    }
    match task {
        CanvasTask::Generate(args) => {
            let out = crate::studio::generate_with_gateway(config_dir, gateway.as_deref(), args).await.map_err(|e| e.to_string())?;
            // 工具跑完但没出东西 —— 当失败处理，原因照抄
            if !out.ok {
                return Err(if out.message.trim().is_empty() { "引擎没有返回结果".into() } else { out.message });
            }
            to_json(out)
        }
        CanvasTask::DepthCapture(args) => to_json(crate::depth_capture::depth_capture_with(config_dir, args).await.map_err(|e| e.to_string())?),
        CanvasTask::DepthVideo(args) => to_json(crate::depth_capture::depth_video_with(config_dir, args).await.map_err(|e| e.to_string())?),
        CanvasTask::ImageOp(args) => to_json(crate::canvas_ops::image_op_with(config_dir, args).await.map_err(|e| e.to_string())?),
        CanvasTask::Interpolate(args) => to_json(crate::canvas_ops::interpolate_with(config_dir, args).await.map_err(|e| e.to_string())?),
        CanvasTask::VideoUpscale(args) => to_json(crate::canvas_ops::video_upscale_with(config_dir, args).await.map_err(|e| e.to_string())?),
        CanvasTask::LongTake(args) => to_json(crate::canvas_ops::run_canvas_long_take(config_dir, args).await?),
        CanvasTask::WorldGenerate(args) => to_json(crate::world_sharp::generate(config_dir, args).await?),
        CanvasTask::PanoCube(args) => to_json(crate::pano_cube::split(config_dir, args).await?),
    }
}

/// 登记并在后台开跑，立刻回任务 id。
#[tauri::command]
pub fn canvas_job_start(app: tauri::AppHandle, state: tauri::State<'_, crate::AppState>, args: CanvasJobArgs) -> std::result::Result<String, String> {
    let config_dir = state.config_dir.clone();
    let gateway = state.gateway.clone();
    let project = args.project_id.clone().unwrap_or_default();
    let id = jobs::start(&config_dir, JobKind::Canvas, &project, "", &args.node_id, 0, &args.label);
    jobs::emit(Some(&app), jobs::get(&config_dir, &id).as_ref());
    let job_id = id.clone();
    tauri::async_runtime::spawn(async move {
        // **整个任务跑在 `JOB_ID` 的作用域里。** 底下 `submit_and_fetch_video`
        // 拿到 prompt_id 之后 `comfy_jobs::attach` 就能把它记到这个任务名下 ——
        // 取消时才找得到该跟 ComfyUI 撤哪一个。
        //
        // **也跑在 `jobs::scoped` 里**（2026-10-11 用户截图：两个视频节点显示同一个「第 3/4 步 · 约还要 15 分钟」，
        // 排队的那个也是）。出片的等待循环本来就按自己这个 prompt 算进度 —— 排队时「排队中 · 前面还有 N 个」、
        // 轮到了「第 N/M 步 · 约还要 X 分钟」—— 但只写进「当前任务」（`jobs::note_progress` / `note_pipeline`），
        // 画布任务以前不在这个作用域里，于是什么都没记，节点只好读全局的 ComfyUI 日志，每个节点都显示正在跑的那一个
        let out = crate::jobs::scoped(&config_dir, &job_id, crate::agent_runtime::comfy_jobs::JOB_ID.scope(job_id.clone(), run(&config_dir, Some(gateway), args.task))).await;
        crate::agent_runtime::comfy_jobs::forget(&job_id);
        let job = match out {
            Ok(result) => jobs::finish_canvas(&config_dir, &job_id, Some(result), None),
            Err(e) => jobs::finish_canvas(&config_dir, &job_id, None, Some(&e)),
        };
        jobs::emit(Some(&app), job.as_ref());
    });
    Ok(id)
}

/// 取消一个画布任务：**真的去 ComfyUI 撤**，不是只把状态改成已取消。
///
/// `jobs.rs` 的模块注释里立过这条规矩 ——「点了状态变成已取消、ComfyUI 照跑」
/// 是这个项目最典型的病，所以在能真取消之前一个入口都不给。现在能了。
///
/// 撤两次：排队中的 `POST /queue {"delete":[…]}`，正在跑的 `POST /interrupt`。
/// 然后打上取消标记 —— 轮询循环每轮自己看一眼，不用等 ComfyUI 那边有反应。
#[tauri::command]
pub async fn canvas_job_cancel(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
    job_id: String,
) -> std::result::Result<String, String> {
    use crate::agent_runtime::comfy_jobs;
    let config_dir = state.config_dir.clone();
    comfy_jobs::mark_cancelled(&job_id);
    let ids = comfy_jobs::prompt_ids_of(&job_id);
    let n = comfy_jobs::cancel_prompts(&ids).await;
    // **状态也要自己落。** 任务可能卡在一个没有取消检查点的 await 上
    // （下载、写盘），等它自己发现可能要好几秒 —— 而用户点完就想看到它停。
    let job = jobs::finish_canvas(&config_dir, &job_id, None, Some("已取消"));
    jobs::emit(Some(&app), job.as_ref());
    Ok(if n > 0 {
        format!("已取消，并让 ComfyUI 撤掉了 {n} 个任务")
    } else {
        // 没有 prompt_id 有两种可能：还没提交到 ComfyUI，或者这条路径没调 attach。
        // 两种都得说出来 —— 默默说「已取消」而 ComfyUI 还在跑，就是上面那条病。
        "已取消。没找到对应的 ComfyUI 任务（可能还没提交出去）—— 如果 ComfyUI 那边还在跑，要去它的界面上撤".into()
    })
}

/// 按 id 查一条任务。页面重新加载后节点靠它取回结果。
#[tauri::command]
pub fn canvas_job_get(state: tauri::State<'_, crate::AppState>, job_id: String) -> Option<jobs::Job> {
    jobs::get(&state.config_dir, &job_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 前端传的是 `{ kind, args }`，每一种都要认得。认不出来会整条拒收，节点上只看到一句反序列化错误。
    #[test]
    fn 前端的任务格式都认得() {
        let cases = [
            r#"{"kind":"generate","args":{"mode":"t2i","engine":"zimage","prompt":"猫","width":1024,"height":1024}}"#,
            r#"{"kind":"depth_capture","args":{"video_path":"/a.mp4"}}"#,
            r#"{"kind":"depth_video","args":{"depth_path":"/d.mp4","image_path":"/g.png","prompt":"跳舞","engine":"h3","hd":true,"keep_background":true,"source_video_path":"/a.mp4"}}"#,
            r#"{"kind":"image_op","args":{"image_path":"/g.png","op":"remove_bg"}}"#,
            r#"{"kind":"interpolate","args":{"video_path":"/a.mp4"}}"#,
            r#"{"kind":"video_upscale","args":{"video_path":"/a.mp4","refine":false}}"#,
            r#"{"kind":"world_generate","args":{"image_path":"/a.png","device":"mps"}}"#,
            r#"{"kind":"pano_cube","args":{"image_path":"/a.png","face_size":1536}}"#,
            r#"{"kind":"long_take","args":{"windows":[{"prompt":"a","images":[]},{"prompt":"b","images":[]}],"window_secs":15,"width":960,"height":544,"generate_audio":true}}"#,
        ];
        for c in cases {
            let t: std::result::Result<CanvasTask, _> = serde_json::from_str(c);
            assert!(t.is_ok(), "{c} → {:?}", t.err());
        }
        let args: CanvasJobArgs = serde_json::from_str(
            r#"{"project_id":null,"node_id":"n1","label":"动作迁移","task":{"kind":"interpolate","args":{"video_path":"/a.mp4"}}}"#,
        )
        .unwrap();
        assert_eq!(args.node_id, "n1");

        // 长镜头带上节点选的加速档 —— 以前不收，一律 Standard
        let t: CanvasTask = serde_json::from_str(
            r#"{"kind":"long_take","args":{"windows":[],"window_secs":15,"width":960,"height":544,"h3_quality":"lightx2v"}}"#,
        )
        .unwrap();
        let CanvasTask::LongTake(a) = t else { panic!("不是长镜头") };
        assert_eq!(a.h3_quality.map(|q| q.tier()), Some(crate::agent_runtime::weetodd::H3Tier::LightX2V));
        // 以及节点选的 LoRA（以前不收）
        let t: CanvasTask = serde_json::from_str(
            r#"{"kind":"long_take","args":{"windows":[],"window_secs":15,"width":960,"height":544,"loras":[{"name":"h3/HMBreastsV2.safetensors","strength":0.8}]}}"#,
        )
        .unwrap();
        let CanvasTask::LongTake(a) = t else { panic!("不是长镜头") };
        let picked = crate::agent_runtime::lora::parse_requested(&serde_json::json!({ "loras": a.loras }));
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].strength, 0.8);
    }

    /// 成功时整条结果进任务，页面重新加载后按 id 取得回来。
    #[test]
    fn 画布任务收尾时存下完整结果() {
        let dir = std::env::temp_dir().join(format!("anima-canvas-job-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let id = jobs::start(&dir, JobKind::Canvas, "p1", "", "node-1", 0, "动作迁移");
        let result = serde_json::json!({"path": "/v.mp4", "first_frame": "/f.png", "notes": ["768×512"]});
        let job = jobs::finish_canvas(&dir, &id, Some(result.clone()), None).unwrap();
        assert_eq!(job.status, jobs::JobStatus::Done);
        assert_eq!(job.output.as_deref(), Some("/v.mp4"));
        assert_eq!(jobs::get(&dir, &id).unwrap().result, Some(result));
        let failed = jobs::finish_canvas(&dir, &id, None, Some("ComfyUI 没开")).unwrap();
        assert_eq!(failed.status, jobs::JobStatus::Failed);
        assert_eq!(failed.error.as_deref(), Some("ComfyUI 没开"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod long_take_live {
    use super::*;
    /// **画布长镜头整条路走一遍**：任务按前端发出的 JSON 原样反序列化，进 `run`（和后台任务同一个入口），
    /// 一路到 ComfyUI 出片。两个 2.5 秒窗口 / 640×384 / Larry 4 步，几分钟。
    /// 2026-10-11 这条路从改成后台任务起就没跑通过，交给用户之前先自己跑一遍。
    /// `ANIMA_LT_IMAGES=/a.png,/b.png cargo test --lib 画布长镜头整条路 -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "要本机 ComfyUI + H3，跑几分钟"]
    async fn 画布长镜头整条路() {
        let cfg = dirs::config_dir().unwrap().join("com.anima.desktop");
        let images: Vec<String> = std::env::var("ANIMA_LT_IMAGES").expect("要 ANIMA_LT_IMAGES").split(',').map(String::from).collect();
        let w = |n: usize| format!("[reference generation] <Subject 1> from <Picture 1> stands on the ruined rooftop of <Picture 3> at dawn, window {n}. The camera slowly pushes in. Her hair moves in the wind.");
        let payload = serde_json::json!({
            "kind": "long_take",
            "args": {
                "windows": [{"prompt": w(1), "images": images}, {"prompt": w(2), "images": images}],
                "window_secs": 2.5, "width": 640, "height": 384, "generate_audio": true,
                "h3_quality": "standard", "loras": []
            }
        });
        let task: CanvasTask = serde_json::from_value(payload).unwrap();
        let t0 = std::time::Instant::now();
        let out = run(&cfg, None, task).await.expect("长镜头没出来");
        println!("用时 {:.0} 秒，结果：{out}", t0.elapsed().as_secs_f64());
        let path = out["video_path"].as_str().expect("结果里没有 video_path");
        assert!(std::path::Path::new(path).is_file(), "文件不在：{path}");
    }

    /// **画布超长视频整条路**：和画布「生成」发出的同一个 `generate` 任务，时长 17 秒 → 后台按时间码拆 2 段、串成一条。
    /// `ANIMA_LT_IMAGES=… cargo test --lib 画布超长视频整条路 -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "要本机 ComfyUI + H3，跑十来分钟"]
    async fn 画布超长视频整条路() {
        let cfg = dirs::config_dir().unwrap().join("com.anima.desktop");
        let images: Vec<String> = std::env::var("ANIMA_LT_IMAGES").expect("要 ANIMA_LT_IMAGES").split(',').map(String::from).collect();
        let prompt = "subject_definitions:\n<Subject 1> is Baiye, a single silver-haired combat android in white ceramic armor holding a silver-white rifle, as shown in <Picture 1> and <Picture 4>. <Subject 2> is the ruined rooftop at dawn, as shown in <Picture 3>.\n\n\
                      summary:\n[reference generation] <Subject 1> walks across <Subject 2> at dawn and looks out over the city.\n\n\
                      detailed_description:\nSemi-realistic Chinese-animation 3D CG, warm dawn backlight. Exactly one woman.\n\
                      [Shot 1] A wide shot: <Subject 1> walks slowly across the puddles of <Subject 2>, the rifle lowered at her side.\n\
                      [Shot 2] At 00:06.000, the camera cuts to a medium shot from behind as she reaches the broken edge and stops.\n\
                      [Shot 3] At 00:11.000, the camera cuts to a close-up of her face; she turns her head toward the rising sun. Around the 12.0-second mark, <Subject 1> says, <d>[Chinese] 天亮了。</d>\n\n\
                      overall_soundscape: Wind over the rooftop, footsteps in shallow water.\n\nnon_diegetic_music: N/A";
        let payload = serde_json::json!({
            "kind": "generate",
            "args": {"mode": "r2v", "engine": "h3", "prompt": prompt, "width": 640, "height": 384, "duration": 17.0,
                     "images": images, "generate_audio": true, "h3_quality": "standard"}
        });
        let task: CanvasTask = serde_json::from_value(payload).unwrap();
        let t0 = std::time::Instant::now();
        let out = run(&cfg, None, task).await.expect("超长视频没出来");
        println!("用时 {:.0} 秒，结果：{out}", t0.elapsed().as_secs_f64());
        let path = out["path"].as_str().expect("结果里没有 path");
        assert!(std::path::Path::new(path).is_file(), "文件不在：{path}");
    }

    /// 画布视频节点 → 它点「生成」时发出的那个 `generate` 任务（和 `dagExecutor` 单条出片同一组字段）。
    /// 参考图按连线顺序给（= `<Picture N>` 的编号）。触发词不另拼：A/B 模板 `injectTriggers: false`、触发词手写在正文里
    fn node_task(d: &serde_json::Value, images: &[String]) -> serde_json::Value {
        let mut args = serde_json::json!({
            "mode": d["mode"].as_str().unwrap_or("r2v"), "engine": d["engine"].as_str().unwrap_or("h3"),
            "prompt": d["prompt"], "loras": d.get("loras").cloned().unwrap_or(serde_json::json!([])),
            "negative_prompt": "", "width": d["width"], "height": d["height"],
            "duration": d["durationSecs"].as_f64().unwrap_or(5.0), "generate_audio": d["generateAudio"].as_bool().unwrap_or(false),
            "h3_official_format": d["h3OfficialFormat"].as_bool().unwrap_or(false), "images": images,
        });
        if let Some(q) = d["h3Quality"].as_str() { args["h3_quality"] = q.into(); }
        if let Some(s) = d["h3Sampling"].as_str().filter(|s| !s.is_empty()) { args["h3_sampling"] = s.into(); }
        if let Some(seed) = d["seed"].as_u64().filter(|s| *s > 0) { args["seed"] = seed.into(); }
        serde_json::json!({"kind": "generate", "args": args})
    }

    #[test]
    fn 画布节点转成生成任务() {
        let d = serde_json::json!({"engine": "h3", "mode": "r2v", "durationSecs": 5, "generateAudio": true, "width": 896, "height": 512,
            "seed": 20261012, "h3Quality": "staged", "h3Sampling": "res_multistep", "prompt": "p", "loras": [{"name": "h3-combat/a.safetensors", "strength": 0}]});
        let t: CanvasTask = serde_json::from_value(node_task(&d, &["/1.png".into()])).expect("前端格式反序列化不了");
        let v = serde_json::to_value(node_task(&d, &["/1.png".into()])).unwrap();
        assert_eq!(v["args"]["seed"], 20261012);
        assert_eq!(v["args"]["h3_sampling"], "res_multistep");
        assert_eq!(v["args"]["loras"][0]["strength"], 0, "零对照的强度 0 要原样发");
        let _ = t;
    }

    /// ComfyUI 正在跑 + 排着的任务数
    async fn comfy_busy() -> Option<usize> {
        let text = reqwest::get("http://127.0.0.1:8188/queue").await.ok()?.text().await.ok()?;
        let v: serde_json::Value = serde_json::from_str(&text).ok()?;
        Some(v["queue_running"].as_array().map_or(0, Vec::len) + v["queue_pending"].as_array().map_or(0, Vec::len))
    }

    /// **按画布工作流里的节点逐个出片**（A/B 对照用，和在画布上一个个点「生成」发的是同一个任务）。
    /// 先等 ComfyUI 连续空闲 `ANIMA_AB_IDLE_SECS`（默认 180）秒 —— 批量出片在跑时不插进去抢；之后一个出完再提交下一个。
    /// 出的片拷成 `<ANIMA_AB_OUT>/<节点 id>.mp4`，每条一行记进 `results.jsonl`。一条失败接着下一条。
    /// `ANIMA_AB_PROJECT=drama_… ANIMA_AB_WORKFLOW=… ANIMA_AB_NODES=ab0,abA ANIMA_AB_IMAGES=/1.png,/2.png,/3.png,/4.png ANIMA_AB_OUT=/dir
    ///  cargo test --lib 画布工作流逐个出 -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "要本机 ComfyUI + H3，一个节点半小时上下"]
    async fn 画布工作流逐个出() {
        let cfg = dirs::config_dir().unwrap().join("com.anima.desktop");
        let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("要 {k}"));
        let wf: serde_json::Value = serde_json::from_str(&crate::canvas_workflows::load(&cfg, &env("ANIMA_AB_PROJECT"), &env("ANIMA_AB_WORKFLOW")).expect("工作流读不到")).unwrap();
        let images: Vec<String> = env("ANIMA_AB_IMAGES").split(',').map(String::from).collect();
        for p in &images {
            assert!(std::path::Path::new(p).is_file(), "参考图不在：{p}");
        }
        let out_dir = std::path::PathBuf::from(env("ANIMA_AB_OUT"));
        std::fs::create_dir_all(&out_dir).unwrap();
        let idle_need: u64 = std::env::var("ANIMA_AB_IDLE_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(180);
        let mut idle = 0u64;
        while idle < idle_need {
            match comfy_busy().await {
                Some(0) => idle += 15,
                Some(n) => { if idle > 0 || n > 0 { println!("ComfyUI 还有 {n} 个任务，等它空下来"); } idle = 0; }
                None => panic!("连不上 ComfyUI（:8188）"),
            }
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
        }
        let nodes = wf["nodes"].as_array().unwrap();
        for id in env("ANIMA_AB_NODES").split(',') {
            let Some(node) = nodes.iter().find(|n| n["id"] == id) else { println!("[{id}] 工作流里没有这个节点，跳过"); continue };
            let task: CanvasTask = serde_json::from_value(node_task(&node["data"], &images)).unwrap();
            let t0 = std::time::Instant::now();
            println!("[{id}] 开始（unix {}）", crate::jobs::now());
            let (ok, detail) = match run(&cfg, None, task).await {
                Ok(out) => match out["path"].as_str() {
                    Some(path) => {
                        let dest = out_dir.join(format!("{id}.mp4"));
                        let copied = std::fs::copy(path, &dest).is_ok();
                        (copied, serde_json::json!({"src": path, "dest": dest, "result": out}))
                    }
                    None => (false, serde_json::json!({"result": out})),
                },
                Err(e) => (false, serde_json::json!({"error": e.to_string()})),
            };
            let mins = t0.elapsed().as_secs_f64() / 60.0;
            println!("[{id}] {} 用时 {mins:.1} 分钟", if ok { "完成" } else { "失败" });
            let line = serde_json::json!({"node": id, "ok": ok, "minutes": mins, "detail": detail});
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(out_dir.join("results.jsonl")).unwrap();
            let _ = writeln!(f, "{line}");
        }
    }
}
