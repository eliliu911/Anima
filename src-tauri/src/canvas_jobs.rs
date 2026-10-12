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
}
