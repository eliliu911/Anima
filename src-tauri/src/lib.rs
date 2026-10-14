pub mod agent;
pub mod assets;
pub mod h3_prompt;
pub mod shot_table;
pub mod shot_route;
pub mod timeline_lint;
pub mod render_queue;
pub mod prompt_input;
pub mod llm_memory;
pub mod h3_spec;
pub mod prompt_route_a;
pub mod prompt_route_b;
pub mod storyboard_gen;
pub mod agent_runtime;
pub mod ai_film;
pub mod cinematic;
pub mod audio;
pub mod avatar;
pub mod character;
pub mod cinema;
pub mod episode_lock;
pub mod actor_colors;
mod control_image;
pub mod depth_capture;
pub mod canvas_ops;
pub mod cover;
pub mod app_root;
pub mod keep_awake;
pub mod long_take;
pub mod long_video;
pub mod combat;
pub mod long_shot;
pub mod one_take;
pub mod launch_path;
pub mod media_pool;
pub mod services;
pub mod updater;
pub mod episode_cover;
pub mod mcp_server;
pub mod looks;
pub mod merge_looks;
pub mod series_assets;
pub mod studio_ops;
pub mod asset_prompt;
pub mod asset_batch;
pub mod public_library;
pub mod sheet_expressions;
pub mod canvas_jobs;
pub mod world_sharp;
pub mod studio_director;
pub mod timeline_detail;
pub mod pano_cube;
mod canvas_workflows;
pub mod canvas_workflow_lint;
pub mod media_export;
mod canvas_audio;
pub mod build_gc;
pub mod comfy_progress;
pub mod script_library;
mod studio;
mod subtitle;
pub mod cinema_dna;
pub mod cinema_dub;
pub mod script_parse;
pub mod cinema_export_jianying;
pub mod cinema_export_fcp;
pub mod token_usage;
pub mod render_resume;
pub mod cinema_persona;
pub mod cinema_qc;
pub mod cinema_sheet;
pub mod character_spec;
pub mod scene_contract;
pub mod screenwriter;
pub mod comfyui;
pub mod commands;
pub mod commands_mcp;
pub mod commands_runtime;
pub mod commands_voice;
pub mod config;
pub mod diagnostics;
pub mod error;
pub mod llm;
pub mod mcp;
pub mod memory;
pub mod music;
pub mod jobs;
pub mod polish;
pub mod shot_language;
pub mod prompts;
pub mod style;
mod style_compare;
pub mod persona;
pub mod personality;
pub mod sidecar;
pub mod task_ledger;
pub mod skill;
pub mod stt;
pub mod system_tools;
pub mod tts;
pub mod voice;
pub mod windows;
pub mod workspace;

use std::path::PathBuf;
use tauri::Manager;

use config::AppConfig;
use llm::gateway::Gateway;

/// 全局应用状态。
///
/// 目前只有网关；阶段二起会加入音频管线、记忆引擎、性格状态。
/// 那几个模块都有并发写入需求，按 ARCHITECTURE.md 坑点 3 应各自用
/// actor 或显式锁管理，不要在这里堆裸共享字段。
pub struct AppState {
    pub gateway: std::sync::Arc<Gateway>,
    pub runtime: std::sync::Arc<agent_runtime::RuntimeService>,
    pub config_path: PathBuf,
    /// 配置目录。语音模型按约定放在它下面的 models/。
    pub config_dir: PathBuf,
    /// 共享的 HTTP 客户端（TTS 与音色列表用）
    pub http: reqwest::Client,
    /// 长期记忆系统。惰性初始化：它要探嵌入后端的维度才能建库，
    /// 而应用启动时后端可能还没就绪，所以不在 setup 里强行打开。
    pub memory: std::sync::Arc<tokio::sync::RwLock<Option<std::sync::Arc<memory::Memory>>>>,
    /// 已连接的 MCP server 与其工具。用 RwLock 因为用户可以在运行期
    /// 增删 server（重连时要独占写）。
    pub mcp: std::sync::Arc<tokio::sync::RwLock<mcp::McpRegistry>>,
    /// 每一轮对话的取消令牌。**和 Gateway 里那份不同** —— 见 [`agent::TurnCancels`]
    pub turn_cancels: std::sync::Arc<agent::TurnCancels>,
    /// 已加载的 Whisper 模型，按模型路径缓存。
    ///
    /// 模型文件几百 MB（small 是 465MB），每次启动语音会话都从磁盘读一遍纯属
    /// 浪费 —— 而"开语音"是个高频动作，那段延迟直接压在用户按下按钮之后。
    /// 缓存的 key 是模型路径：用户在设置里换模型时自然会重新加载，
    /// 而换回来又能命中缓存。
    pub stt_cache: std::sync::Arc<tokio::sync::Mutex<Option<CachedStt>>>,
    /// 当前语音会话。None = 未启动。
    ///
    /// 用 RwLock 而不是把它塞进 Gateway：语音会话的生命周期由用户显式的
    /// 启动/停止控制，与模型网关无关。
    pub voice: std::sync::Arc<tokio::sync::RwLock<Option<voice::VoiceSession>>>,
    /// 全局背景音乐播放器。支持 LLM 的 play_audio 和 stop_audio 工具。
    pub music_player:
        std::sync::Arc<tokio::sync::Mutex<Option<std::sync::Arc<audio::playback::Player>>>>,
}

/// 缓存下来的 Whisper 实例，连同它是用哪份配置加载的。
///
/// 只留一个而不是做成 map：模型动辄几百 MB，同时留住两份没有意义 ——
/// 用户不会来回切模型，而缓存的收益全在"同一个模型反复开会话"上。
pub struct CachedStt {
    pub config: stt::SttConfig,
    pub stt: std::sync::Arc<stt::whisper::WhisperStt>,
}


/// 初始化日志：**同时**写终端和文件。
///
/// 为什么必须写文件：启动方式不同，输出去向就不同 —— `anima.sh` 会把 stdout
/// 重定向到文件，而用户从自己的终端跑 `npm run tauri:dev` 时输出只在那个 tty 里。
/// 排查语音问题时我因此两次读错地方、得出了错误结论。让应用自己落盘，
/// 无论谁怎么启动，日志都在同一个位置。
fn init_logging() {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "anima_lib=debug,warn".into())
        .add_directive("symphonia_bundle_mp3=error".parse().unwrap_or_default())
        .add_directive("symphonia_core=error".parse().unwrap_or_default());

    let log_dir = dirs_log_dir();
    let _ = std::fs::create_dir_all(&log_dir);

    let file = tracing_appender::rolling::never(&log_dir, "anima.log");
    let (writer, guard) = tracing_appender::non_blocking(file);
    // non_blocking 的 guard 一旦 drop 就停止刷盘，必须活到进程结束
    std::mem::forget(guard);

    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        // 文件里不要 ANSI 颜色码，否则 grep 出来全是转义序列
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(writer)
                .with_ansi(false),
        )
        .init();

    // **让 panic 也进 tracing。**
    //
    // 默认的 panic 钩子只写 stderr —— 而 stderr 去哪取决于怎么启动的：
    // `anima.sh` 捕获到 launcher.log，用户自己跑 `npm run tauri:dev` 时
    // 只在那个 tty 里。于是 tokio 工作线程里的 panic 在 anima.log 里
    // **完全不存在**。
    //
    // 实测代价：rustls 缺加密后端导致的 panic 藏了三轮才被发现，
    // 因为查的一直是 anima.log。现在它会和别的日志一起落盘。
    let previous = std::panic::take_hook();
    // **panic 的现场只有这一次机会记下来。**
    //
    // 原来只记 `info`（那是一句 `panicked at …: 消息`），丢掉了两样最要紧的：
    // 出事的线程名、以及调用栈。2026-09-18 组装页崩了两次，日志里只有
    // 「panic in a function that cannot unwind」—— 连是哪个模块都看不出来。
    //
    // 调用栈**强制打开**（不依赖 RUST_BACKTRACE）：双击启动的 .app 没有
    // 任何环境变量，指望用户去设那个变量等于永远拿不到栈。
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let name = thread.name().unwrap_or("(无名)").to_string();
        let bt = std::backtrace::Backtrace::force_capture();
        tracing::error!(thread = %name, panic = %info, backtrace = %bt, "线程 panic");
        previous(info);
    }));

    tracing::info!(dir = %log_dir.display(), "日志已同时写入终端与文件");
}

fn dirs_log_dir() -> PathBuf {
    std::env::var("HOME")
        .map(|h| PathBuf::from(h).join("Library/Logs/Anima"))
        .unwrap_or_else(|_| PathBuf::from("."))
}

/// 选定 rustls 的加密后端。**必须在任何 TLS 连接之前调用。**
///
/// ## 为什么需要它
///
/// 依赖树里同时存在 `ring` 和 `aws-lc-rs` 两个后端（reqwest 与
/// tokio-tungstenite 各带一个）。rustls 0.23 在这种情况下拒绝猜，
/// **第一次建立 TLS 连接时直接 panic**：
///
/// ```text
/// Could not automatically determine the process-level CryptoProvider
/// from Rustls crate features.
/// ```
///
/// 而这个 panic 发生在 tokio 的工作线程里，所以：
///
/// - tracing 日志里**一个字都没有**（panic 走 stderr，不走 tracing）
/// - 那一轮的生成任务当场死掉：不合成、不落库、连"一句都没说出去"
///   的探针都跑不到
/// - actor 那边只看到任务结束了，照常按"正常说完"结算情感轴
///
/// 实测的症状是「她挥了挥手，然后什么都没有」，查了三轮才定位到 ——
/// 因为它在应用自己的日志里完全不存在，只在启动器捕获 stderr 的
/// `launcher.log` 里有。
/// **公开且幂等**，因为「在 `run()` 里装一次」不够。
///
/// 实测：`tests/live.rs` 走不到 `run()`，于是 Edge TTS 的 WSS 一建连就 panic 在
/// rustls 里 —— `live_edge_tts_synthesizes_chinese_audio` 和依赖它产出音频的
/// `live_whisper_transcribes_chinese_from_tts_audio` 一起挂，而报错指向
/// rustls 内部，看不出是「provider 没装」。
///
/// 所以真正会建 TLS 的两个地方（Edge TTS 的 WSS、Gateway 的 reqwest）
/// 各自调一次，任何入口都安全，不靠「记得在 main 里调」。
pub fn ensure_crypto_provider() {
    // 已经装过就不是错误（测试进程里可能已经装过）
    if rustls::crypto::CryptoProvider::get_default().is_some() {
        return;
    }
    if rustls::crypto::ring::default_provider()
        .install_default()
        .is_err()
    {
        tracing::warn!("rustls 加密后端已经被别处装过了，沿用那一份");
    }
}


/// `app_config_dir` 之外还允许读的目录。
///
/// ComfyUI 的产出落在它自己的 `output/` 下，用户工作区的
/// `.anima/generated/` 也在配置目录之外 —— 这两处的图要能在界面上显示。
pub fn media_extra_roots() -> Vec<std::path::PathBuf> {
    let mut roots = Vec::new();
    if let Some(dir) = crate::comfyui::find_comfyui_dir() {
        roots.push(dir.join("output"));
    }
    // 内置资源：风格预设的缩略图在项目根的 `resources/style_thumbs/`，
    // **不在配置目录里**。忘了这一条的表现是整页缩略图全变「读不出来」——
    // 文件在磁盘上，协议却 403。
    roots.push(crate::style::thumbs_dir());
    roots.push(crate::style::template_thumbs_dir());
    // 公共素材库（~/AI/assets/public），在应用目录之外
    roots.push(crate::public_library::root());
    // LoRA 示例图（风格窗口），在 ComfyUI 的 models/loras 下面，不在 output 里
    if let Some(d) = crate::studio::lora_thumbs_dir() {
        roots.push(d);
    }
    roots
}

/// 启动时默认露面的窗口：`"avatar"`（桌宠悬浮层）或某个面板的 label。
///
/// 读锁被占着就按默认 —— 别为了读一个设置把启动卡住。
fn startup_window(app: &tauri::AppHandle) -> String {
    app.try_state::<AppState>()
        .and_then(|s| s.gateway.config().try_read().ok().map(|c| c.startup_window.clone()))
        .unwrap_or_else(|| "avatar".to_string())
}

fn saved_geometry(app: &tauri::AppHandle, label: &str) -> Option<windows::Geometry> {
    app.try_state::<AppState>()
        .and_then(|s| s.gateway.config().try_read().ok().and_then(|c| c.window_geometry.get(label).copied()))
}

/// 又启动了一个 Anima：不开第二个，把已经开着的这个拎到前面。
///
/// 露面的是「启动时默认打开」的那个窗口 —— 双击图标想看见的就是它。
fn bring_to_front(app: &tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    let _ = app.show();
    let label = startup_window(app);
    if label == "avatar" {
        if let Some(w) = app.get_webview_window("avatar") {
            let _ = w.show();
            let _ = w.unminimize();
            let _ = w.set_focus();
            use tauri::Emitter;
            let _ = app.emit("avatar-visibility-changed", true);
        }
        return;
    }
    if let Err(err) = windows::open(app, &label, saved_geometry(app, &label)) {
        tracing::warn!(%err, window = %label, "挡下了第二个实例，但没能把已开的窗口拎到前面");
    }
}

pub fn run() {
    init_logging();
    // 顺序要紧：TLS 在这之后才可能发生（TTS 的 WSS、云端模型的 HTTPS）
    ensure_crypto_provider();

    let mut builder = tauri::Builder::default();
    // **单例**（2026-09-29 用户截图：程序坞里两个 Anima）。
    //
    // 双击同一个 .app 系统自己会去激活已开的那个；出第二个进程的是**不同路径的包** ——
    // 当时一个从 /Applications 开、一个从 target/release/bundle 开。两个进程读写同一份
    // config.json / jobs.json / 数据库、各起一套语音和 ComfyUI 任务，互相盖。
    //
    // 必须是第一个插件：第二个实例在这个插件的 setup 里就 `exit(0)`，排在后面的话
    // 别的插件和下面的 `.setup()` 已经跑过了（开库、起服务）。
    //
    // **只管打包版。** 判定按 bundle identifier（socket 在 `/tmp/<identifier>_si.sock`），
    // dev 和打包版是同一个 —— 打包版开着的时候 `npm run tauri:dev` 会一声不吭地退出、
    // 只把打包版拎到前面，在这个仓库里干活的 agent 会以为 dev 起不来。
    if !cfg!(debug_assertions) {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| bring_to_front(app)));
    }

    builder
        .plugin(tauri_plugin_opener::init())
        // `anima-media://` —— 界面读本地图片/视频的唯一通道。
        //
        // 不走 Tauri 自带的 asset 协议：那份配置是 `scope: ["**"]`，
        // 等于把整台机器的读权限交给 webview。这里只放行
        // `app_config_dir` 子树（`library/`、`projects/`、`.anima/generated/`
        // 都在里面），越界一律 403。
        .register_asynchronous_uri_scheme_protocol(assets::MEDIA_SCHEME, |ctx, request, responder| {
            use tauri::Manager;
            let deny = |code: u16, msg: &str| {
                tauri::http::Response::builder()
                    .status(code)
                    .header("Content-Type", "text/plain; charset=utf-8")
                    .body(msg.as_bytes().to_vec())
                    .unwrap_or_else(|_| tauri::http::Response::new(Vec::new()))
            };
            let Some(requested) = assets::parse_media_request(&request.uri().to_string()) else {
                return responder.respond(deny(400, "缺少 p 参数"));
            };
            let Ok(config_dir) = ctx.app_handle().path().app_config_dir() else {
                return responder.respond(deny(500, "取不到配置目录"));
            };
            let header = |name: &str| request.headers().get(name).and_then(|v| v.to_str().ok()).map(String::from);
            let (range, if_none_match) = (header("Range"), header("If-None-Match"));
            // **读文件放到后台线程。** 同步 handler 跑在主线程上，放视频时一段一段地读会卡住整个界面。
            //
            // **不能长缓存 + immutable**：分镜首帧固定写 `shots/<镜头 id>.png`，重出后 URL 不变，
            // immutable 会让 webview 一年都不再读。所以用 ETag（mtime + 长度）+ 每次校验。
            // 越界（不在 app_config_dir 子树里）一律 403。
            //
            // **panic 必须挡在这里。** 这个回调最终跑在 ObjC 的
            // `webView:startURLSchemeTask:` 里 —— 任何逃出去的 panic 到了
            // C ABI 边界就只能 abort，表现是整个应用秒退、日志里只有一句
            // 「panic in a function that cannot unwind」。
            //
            // 2026-09-18 实测崩过两次：在组装页从 E02 切到 E01 的那一刻，
            // 视频、封面、六张缩略图的请求同时被取消。
            //
            // 兜住之后最坏也只是这一个资源回 500 —— 一张图裂了，
            // 而不是用户正在做的整集组装一起没。
            //
            // **交给固定大小的池子，不是一个请求一个线程。**
            // 崩溃报告里四个媒体线程同时卡在 `callOnMainRunLoopAndWait`
            // （`respond()` 每次都要同步往返主线程），而切一次分集会同时发出
            // 视频 + 封面 + 六张缩略图的请求。详见 `media_pool` 的模块注释。
            media_pool::submit(move || {
                let work = std::panic::AssertUnwindSafe(|| {
                    let roots = assets::media_roots(&config_dir, &media_extra_roots());
                    match assets::resolve_media_path(&roots, &requested) {
                        Ok(real) => assets::media_response(&real, range.as_deref(), if_none_match.as_deref()),
                        Err(e) => deny(403, &e.to_string()),
                    }
                });
                let resp = std::panic::catch_unwind(work).unwrap_or_else(|_| {
                    // panic 的详情由 panic hook 记（带线程名和调用栈），
                    // 这里只保证**一定要回一个响应** —— 不回的话 webview 会一直等
                    tracing::error!(path = %requested.display(), "媒体请求处理时 panic，已兜住");
                    deny(500, "读这个资源时出错了")
                });
                responder.respond(resp);
            });
        })
        .setup(|app| {
            // **必须排在任何 Command::new 之前** —— MCP server、sidecar、ffmpeg
            // 全都要靠它。双击启动不经过 shell，PATH 只有 /usr/bin:/bin:/usr/sbin:/sbin，
            // 于是 npx / node / uv 全都「No such file or directory」
            let (patched, _) = launch_path::ensure();
            if patched {
                tracing::info!("PATH 已补全（双击启动的 .app 拿不到 shell 的 PATH）");
            }

            let config_dir = app.path().app_config_dir()?;
            // **必须排在所有读 key 的代码之前。** 打包版双击启动没有任何环境变量，
            // 不先把安全文件读进来，后面每一次 get_secret 都会落空 ——
            // 表现是「模型列表拉不出来」，而且不报错。
            // AppState 会把 config_dir move 走，而下面拉 sidecar 还要用
            let sidecar_config_dir = config_dir.clone();
            // **排在任何路径解析之前。** 指针文件是双击启动唯一能找到
            // characters/ skills/ avatars/ voice-library/ tools/ resources/ 的那条路
            app_root::init(&config_dir);
            // 模型用量账本（概览页的 Token 统计）。要在网关开始干活之前
            token_usage::init(&config_dir);
            if let Err(e) = config::secrets::init(&config_dir) {
                tracing::error!(error = %e, "密钥文件没加载成功，云端后端可能都用不了");
            }
            let config_path = config_dir.join("config.json");
            // 语音模型的约定目录，提前建好，用户可以直接把 ggml-*.bin 丢进去
            let _ = std::fs::create_dir_all(stt::default_model_dir(&config_dir));

            // 老角色库里塞着 base64（现场是 86 MB / 71 张图），读一次角色列表
            // 就要解析整个文件。有体积闸门，迁过的项目连读都不会读。
            assets::migrate_legacy_projects_on_start(&config_dir);

            // 上一次退出时还挂着的渲染任务判为失败。**不做这一步的话
            // 队列上永远有几个跑不完的任务** —— 进程没了，那些 await
            // 也没了，没有任何人会再去给它们收尾，而界面上它们和真正
            // 在跑的一模一样。
            //
            // **出片例外**（2026-10-05）：交给了 ComfyUI 的那一镜重启后接着收结果，批量没轮到的接着排队 ——
            // 不然 ComfyUI 那边几个小时白跑（见 `render_resume`）。续接不上的照旧判失败。
            jobs::set_app(app.handle().clone());
            let (resumed, requeued) = render_resume::start(Some(app.handle().clone()), &config_dir);
            if resumed + requeued > 0 {
                tracing::info!(resumed, requeued, "上次退出时有片没出完：已接着收 / 接着排");
            }

            // 配置坏了不该让应用起不来：退回默认值并记一条 warn，
            // 用户还能进设置面板改回来。
            let config = match AppConfig::load(&config_path) {
                Ok(c) => c,
                Err(err) => {
                    // **先把坏掉的文件挪走再退回默认值。**
                    //
                    // 否则下一次 save 会用默认配置覆盖掉它，用户所有设置就真没了。
                    // 而"解析失败"最常见的原因恰恰是我们自己改了 schema ——
                    // 那种情况下用户的配置本身是好的，不该被我们的失误吞掉。
                    let backup = config_path.with_extension(
                        match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
                            Ok(d) => format!("broken-{}.json", d.as_secs()),
                            Err(_) => "broken.json".to_string(),
                        },
                    );
                    match std::fs::rename(&config_path, &backup) {
                        Ok(()) => tracing::error!(
                            %err,
                            backup = %backup.display(),
                            "配置解析失败，已把原文件另存备份并退回默认配置"
                        ),
                        Err(io) => tracing::error!(
                            %err, %io,
                            "配置解析失败，且备份也失败 —— 不要保存设置，先手工修复该文件"
                        ),
                    }
                    AppConfig::default()
                }
            };

            tracing::info!(
                realtime = %config.realtime_provider,
                background = %config.background_provider,
                "网关初始化"
            );

            let voice_slot = std::sync::Arc::new(tokio::sync::RwLock::new(None));
            let memory_slot = std::sync::Arc::new(tokio::sync::RwLock::new(None));
            let http = reqwest::Client::builder()
                .no_proxy()
                .build()
                .unwrap_or_else(|_| reqwest::Client::new());
            let music_player = std::sync::Arc::new(tokio::sync::Mutex::new(None));
            // 语言要在 config 被移进 Gateway **之前**取出来 —— 托盘在下面才建
            let tray_lang = config
                .language
                .clone()
                .unwrap_or_else(|| "en-US".to_string());
            let gateway = std::sync::Arc::new(Gateway::new(config)?);

            app.manage(AppState {
                gateway: gateway.clone(),
                runtime: std::sync::Arc::new(
                    agent_runtime::RuntimeService::with_app_handle_config_dir_gateway_and_system_tools(
                        app.handle().clone(),
                        config_dir.clone(),
                        http.clone(),
                        music_player.clone(),
                        gateway.clone(),
                        memory_slot.clone(),
                    ),
                ),
                config_path,
                config_dir,
                http,
                voice: voice_slot,
                // 先留空，随后在后台连接（见下面的 spawn）
                mcp: std::sync::Arc::new(tokio::sync::RwLock::new(Default::default())),
                turn_cancels: std::sync::Arc::new(agent::TurnCancels::default()),
                stt_cache: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
                memory: memory_slot,
                music_player,
            });

            // 启动时自动连接已配置的 MCP server... (this is just context)
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                use tauri::Manager;
                let state = handle.state::<AppState>();
                let servers = state.gateway.config().read().await.mcp_servers.clone();
                if servers.is_empty() {
                    return;
                }
                let mut secrets = std::collections::BTreeMap::new();
                for server in &servers {
                    for id in server.transport.secret_ids() {
                        if let Ok(Some(v)) = config::secrets::get_secret(&id) {
                            secrets.insert(id, v);
                        }
                    }
                }
                let registry = mcp::McpRegistry::connect(&servers, &secrets).await;
                let connected = registry.statuses().iter().filter(|s| s.connected).count();
                tracing::info!(
                    total = servers.len(),
                    connected,
                    tools = registry.tool_declarations().len(),
                    "MCP server 启动连接完成"
                );
                *state.mcp.write().await = registry;
            });

            // 启动时自动拉起已知本地 Python Sidecars (4D数字人、本地语音)
            crate::sidecar::start_all_sidecars(app.handle().clone(), sidecar_config_dir);

            use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
            use tauri::tray::TrayIconBuilder;

            // **托盘的语言取自配置，不是硬编码英文。**
            //
            // 这里原来写死英文，而语言只存在前端的 localStorage 里，Rust 读不到 ——
            // 于是不管用户选的什么语言，**每次启动托盘都是英文**，要进设置里
            // 再切一次才对。文案和 `update_tray_menu` 共用 `windows::` 里的同一份，
            // 各写一份必然漂。
            let (toggle_title, quit_title) = crate::windows::tray_titles(&tray_lang);

            // 「隐藏她」而不是「退出」—— 桌宠该能暂时收起来，
            // 而不是只能杀掉整个应用
            let toggle_item =
                MenuItem::with_id(app, "toggle_avatar", toggle_title, true, None::<&str>)?;
            // 五个面板。**托盘是唯一入口** —— 见 windows.rs 的模块注释
            let panel_items: Vec<_> = crate::windows::labels_for_lang(&tray_lang)
                .into_iter()
                .map(|(label, title)| {
                    MenuItem::with_id(app, format!("panel:{label}"), title, true, None::<&str>)
                })
                .collect::<std::result::Result<_, _>>()?;
            let quit_item = MenuItem::with_id(app, "quit", quit_title, true, None::<&str>)?;
            let separator = PredefinedMenuItem::separator(app)?;

            let menu = Menu::with_items(app, &{
                let mut items: Vec<&dyn tauri::menu::IsMenuItem<_>> = panel_items
                    .iter()
                    .map(|i| i as &dyn tauri::menu::IsMenuItem<_>)
                    .collect();
                items.push(&separator);
                items.push(&toggle_item);
                items.push(&quit_item);
                items
            })?;

            let mut builder = TrayIconBuilder::with_id("main_tray").menu(&menu);
            // **托盘图标不能用应用图标。**
            //
            // 应用图标按 macOS 规范留了 16% 透明边（824/1024，程序坞里才和别人一样大），
            // 那圈留白到了 22pt 的菜单栏里就是白占地方 —— 图标看着比旁边小一圈。
            // 托盘这张是满画布的 44×44（22pt @2x），见 `icons/tray.png`。
            // 自己解 PNG（tauri 的 `Image::from_bytes` 要开 `image-png` feature；`image` crate 本来就在依赖里）
            match image::load_from_memory(include_bytes!("../icons/tray.png")) {
                Ok(img) => {
                    let rgba = img.to_rgba8();
                    let (w, h) = rgba.dimensions();
                    builder = builder.icon(tauri::image::Image::new_owned(rgba.into_raw(), w, h));
                }
                // 读不出来就退回应用图标：小一点总比没有强
                Err(e) => {
                    tracing::warn!(%e, "托盘图标读不出来，退回应用图标");
                    if let Some(icon) = app.default_window_icon().cloned() {
                        builder = builder.icon(icon);
                    }
                }
            }
            builder
                .on_menu_event(move |app, event| match event.id.as_ref() {
                    "toggle_avatar" => {
                        // 角色层是独立窗口，直接开关它 —— 不用绕前端
                        if let Some(w) = app.get_webview_window("avatar") {
                            let visible = w.is_visible().unwrap_or(false);
                            let next_vis = !visible;
                            let _ = if next_vis { w.show() } else { w.hide() };
                            use tauri::Emitter;
                            let _ = app.emit("avatar-visibility-changed", next_vis);
                        }
                    }

                    "quit" => {
                        app.exit(0);
                    }
                    // 五个面板。位置从配置里取 —— 读锁可能被占着，
                    // 那时候就用默认位置开，别为了记住位置把托盘卡住
                    id if id.starts_with("panel:") => {
                        let label = id.trim_start_matches("panel:").to_string();
                        let saved = app.try_state::<AppState>().and_then(|s| {
                            s.gateway
                                .config()
                                .try_read()
                                .ok()
                                .and_then(|c| c.window_geometry.get(&label).copied())
                        });
                        if let Err(err) = windows::open(app, &label, saved) {
                            tracing::warn!(%err, window = %label, "从托盘打开面板失败");
                        }
                    }
                    _ => {}
                })
                .build(app)?;

            // 检查启动时默认打开哪个窗口："avatar"（默认桌面虚拟人）或 "chat"（对话面板）
            let startup_win = startup_window(app.handle());

            // 选了桌宠以外的形态：藏掉 avatar 悬浮层，直接开那个面板。
            //
            // **面板 label 必须真的存在**（`windows::panel` 认得），
            // 否则 `windows::open` 静默失败 —— 表现是「设置里选了，
            // 启动后什么都没开」。校验在 `commands::normalize_startup_window`。
            if startup_win != "avatar" {
                if let Some(w) = app.get_webview_window("avatar") {
                    let _ = w.hide();
                }
                let _ = windows::open(app.handle(), &startup_win, saved_geometry(app.handle(), &startup_win));
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_emotion,
            commands::get_projects,
            commands::create_project,
            commands::update_project,
            commands::delete_project,
            commands::get_sessions,
            commands::get_all_sessions,
            commands::create_session,
            commands::get_session_messages,
            commands::append_session_message,
            commands::delete_session_message,
            commands::rollback_session_to_message,
            commands::update_session_title,
            commands::archive_session,
            commands::delete_session,
            commands::start_chat,
            commands::cancel_chat,
            commands::save_chat_image_attachment,
            commands::save_generation_log,
            depth_capture::canvas_save_video,
            depth_capture::canvas_save_audio,
            depth_capture::canvas_depth_video,
            canvas_jobs::canvas_job_start,
            canvas_jobs::canvas_job_get,
            canvas_jobs::canvas_job_cancel,
            commands::load_dropped_attachment,
            commands::task_ledger_get,
            commands::task_ledger_archive,
            commands::get_config,
            commands::build_info,
            commands::set_startup_window,
            commands::set_theme_mode,
            commands::set_image_engine,
            commands::set_language,
            commands::set_sdxl_checkpoint,
            commands::set_h3_native_ref2va,
            commands::list_sdxl_checkpoints,
            commands::list_loras,
            commands::set_sheet_loras,
            commands::set_zimage_loras,
            commands::update_provider,
            commands::set_tier_provider,
            commands::set_tool_fallback,
            commands::set_route_model,
            commands::set_thinking_effort,
            commands::set_api_key,
            commands::test_provider,
            commands::provider_vision_models,
            commands::provider_context_window,
            commands::compress_history,
            commands_voice::voice_start,
            commands_voice::voice_stop,
            commands_voice::voice_interrupt,
            commands_voice::voice_status,
            commands_voice::set_voice_config,
            commands_voice::list_input_devices,
            commands_voice::list_tts_voices,
            commands_voice::probe_tts_engine,
            commands_voice::list_skills,
            commands_voice::save_skill,
            commands_voice::delete_skill,
            commands_voice::select_skills,
            commands_voice::list_slash_commands,
            commands_voice::expand_slash_command,
            commands_voice::select_character,
            commands_voice::open_panel,
            commands_voice::remember_panel_geometry,
            commands_voice::set_avatar_passthrough,
            commands_voice::get_avatar_visible,
            commands_voice::set_avatar_visible,
            commands_voice::persona_view,
            commands_voice::diagnostics,
            commands_voice::sidecar_status,
            commands_voice::create_character,
            commands_voice::update_character,
            commands_voice::delete_character,
            commands_voice::character_binding,
            commands_voice::set_character_binding,
            commands_voice::tts_engines,
            commands_voice::set_tts_engine,
            commands_voice::update_tts_engine,
            commands_voice::preview_tts_voice,
            commands_voice::memory_stats,
            commands_voice::log_ui,
            commands_voice::set_interaction_mode,
            commands_voice::update_tray_menu,
            commands_voice::avatar_models,
            commands_voice::avatar_models_dir,
            commands_voice::open_avatar_models_dir,
            commands_voice::avatar_model_bytes,
            commands_voice::avatar_animations,
            commands_voice::avatar_animation_bytes,
            commands_voice::memory_recall,
            commands_voice::memory_clear_facts,
            commands_voice::memory_list_facts,
            commands_voice::memory_forget_fact,
            commands_voice::memory_add_fact,
            commands_runtime::runtime_mcp_tool_infos,
            commands_runtime::runtime_mcp_resources,
            commands_runtime::runtime_mcp_prompts,
            commands_runtime::runtime_list_pending_approvals,
            commands_runtime::runtime_resolve_approval,
            commands_runtime::runtime_create_job,
            commands_runtime::runtime_list_jobs,
            commands_runtime::runtime_start_job_runner,
            commands_runtime::runtime_cancel_job,
            commands_runtime::runtime_pause_job,
            commands_runtime::runtime_resume_job,
            commands_runtime::runtime_export_job_log,
            commands_mcp::mcp_view,
            commands_mcp::mcp_reconnect,
            commands_mcp::mcp_upsert_server,
            commands_mcp::mcp_remove_server,
            commands_mcp::mcp_set_permissions,
            comfyui::comfyui_status,
            comfyui::comfyui_start,
            style::style_catalog,
            style::style_template_thumbs,
            style::style_get,
            style::style_save,
            style::style_analyze,
            jobs::cinema_jobs,
            jobs::cinema_jobs_running,
            cinema::cinema_synthesize_prompts,
            cinema::cinema_render_episode_videos,
            cinema::cinema_queue_shot_videos,
            render_queue::cinema_render_queue,
            render_queue::cinema_reorder_queue,
            render_queue::cinema_release_hold,
            cinema::cinema_queue_synthesize,
            cinema::cinema_render_long_take,
            cinema::cinema_repair_storyboard,
            cinema::cinema_repairable_issues,
            cinema::cinema_issue_fixes,
            keep_awake::keep_awake_status,
            cinema::cinema_regenerate_scene,
            jobs::cinema_cancel_job,
            jobs::cinema_jobs_clear,
            polish::cinema_polish_prompt,
            prompts::prompt_slots,
            prompts::prompt_save_slot,
            prompts::prompt_try,
            prompts::prompt_export,
            prompts::prompt_import,
            cinema::cinema_workspace,
            cinema::cinema_scripts,
            script_library::studio_script_list,
            script_library::studio_script_save,
            script_library::studio_script_delete,
            script_library::studio_script_projects,
            script_library::studio_script_project_save,
            script_library::studio_script_project_delete,
            script_library::studio_script_generate,
            cinema::cinema_delete_project,
            cinema::cinema_extract_entities,
            cinema::cinema_save_model_settings,
            cinema::cinema_frame_reference_pick,
            cinema::cinema_generate_shot_table,
            cinema::cinema_validate_storyboard,
            cinema::cinema_import_entities,
            cinema::cinema_link_script_project,
            cover::cinema_upload_cover,
            cinema::cinema_set_aspect_ratio,
            assets::assets_list,
            assets::assets_create,
            assets::assets_update,
            assets::assets_delete,
            assets::assets_references,
            assets::assets_add_variant,
            assets::assets_select_variant,
            assets::assets_favorite_variant,
            assets::assets_remove_variant,
            assets::assets_add_look,
            assets::assets_update_look,
            assets::assets_remove_look,
            merge_looks::assets_merge_plan,
            merge_looks::assets_merge_apply,
            series_assets::series_assets_inbox,
            series_assets::series_assets_adopt,
            series_assets::series_assets_adopt_all,
            series_assets::series_assets_keep_episode,
            series_assets::series_extract_from_texts,
            merge_looks::assets_merge_suggest,
            assets::assets_promote,
            assets::assets_read_media,
            media_export::media_save_to_downloads,
            assets::assets_resolve_refs,
            studio::studio_capabilities,
            studio::studio_generate,
            studio::studio_loras,
            studio::studio_set_lora_note,
            studio::studio_assets,
            studio::studio_delete_asset,
            studio::studio_results,
            studio::studio_save_results,
            studio::studio_templates,
            studio::studio_save_template,
            studio::studio_delete_template,
            studio::studio_history,
            studio::studio_add_history,
            cinema::cinema_list_projects,
            cinema::cinema_create_project,
            cinema::cinema_get_project_detail,
            cinema::cinema_create_episode,
            cinema::cinema_delete_episode,
            cinema::cinema_generate_ai_sequel,
            cinema::cinema_save_episode,
            cinema::cinema_assign_actor_colors,
            cinema::cinema_set_actor_color,
            cinema::cinema_reveal_file,
            cinema::cinema_stitch_episode,
            cinema::cinema_stitch_episode_start,
            cinema::cinema_export_srt,
            asset_batch::assets_batch_generate_start,
            style_compare::style_compare_start,
            style_compare::style_compare_list,
            public_library::public_library_list,
            public_library::public_library_import,
            cinema::cinema_set_default_look,
            cover::cinema_generate_cover,
            episode_cover::cinema_generate_episode_cover,
            episode_cover::cinema_episode_cover,
            services::service_status,
            updater::updater_check,
            updater::updater_build,
            updater::updater_apply,
            episode_cover::cinema_set_cover_intro,
            episode_cover::cinema_has_cover_intro,
            cinema::cinema_get_engine_config,
            cinema::cinema_save_engine_config,
            cinema::cinema_render_shot,
            cinema::cinema_shot_prompt_preview,
            cinema::cinema_adopt_frame,
            cinema::cinema_shot_routes,
            cinema::cinema_set_shot_keyframe,
            cinema::cinema_render_episode_frames,
            cinema::cinema_render_end_frame,
            world_sharp::world_read_splat,
            studio_director::cinema_generate_scene_world,
            studio_director::cinema_set_director_scene,
            studio_director::cinema_render_frame_from_director,
            timeline_detail::cinema_refine_timelines,
            cinema::cinema_save_shot,
            cinema::cinema_keyframe_preview,
            cinema::cinema_adopt_video,
            canvas_audio::canvas_speak,
            comfy_progress::comfy_progress,
            canvas_workflows::canvas_workflow_save,
            canvas_workflows::canvas_workflow_list,
            canvas_workflows::canvas_workflow_load,
            canvas_workflows::canvas_workflow_delete,
            canvas_workflows::canvas_workflow_rename,
            cinema::cinema_pick_take,
            cinema::cinema_render_shot_video,
            cinema::cinema_generate_shot_audio,
            cinema_dub::cinema_dub,
            cinema_dub::cinema_clear_dub,
            cinema_dub::cinema_dubbed_shots,
            cinema_dub::cinema_voice_templates,
            cinema::cinema_list_characters,
            cinema::cinema_generate_character_sheet_via_comfyui,
            cinema::cinema_slice_character_sheet,
            cinema::cinema_update_character_asset_slot,
            cinema::cinema_export_jianying_project,
            cinema::cinema_stale_prompts,
            cinema_export_fcp::cinema_export_fcp_start,
            cinema_export_fcp::cinema_fcp_names,
            cinema_export_fcp::cinema_choose_export_folder,
            token_usage::token_usage_summary,
            workspace::workspace_choose_folder,
            workspace::workspace_check_status,
            workspace::workspace_init,
            workspace::workspace_save_files,
            workspace::workspace_write_file,
            workspace::workspace_commit_file,
            workspace::workspace_open_in_finder,
            workspace::workspace_list_knowledge_docs,
            workspace::workspace_delete_knowledge_doc,
            workspace::workspace_get_permissions,
            workspace::workspace_save_permissions,
            workspace::workspace_reindex_memory,
            workspace::workspace_knowledge_count,
            workspace::workspace_read_file,
            workspace::workspace_list_directory_tree,
            workspace::workspace_git_status,
        ])
        .build(tauri::generate_context!())
        .expect("Tauri 应用启动失败")
        .run(|_app, event| {
            // 应用退出清理：收回电台、Python Sidecar、所有 PTY 与 LSP 服务器的子进程
            if matches!(
                event,
                tauri::RunEvent::Exit | tauri::RunEvent::ExitRequested { .. }
            ) {
                music::stop_radio();
                sidecar::stop_all_sidecars();
                agent_runtime::pty_session::stop_all_pty_sessions();
                agent_runtime::lsp_client::stop_all_lsp_servers();
            }
        });
}

#[cfg(test)]
mod single_instance_tests {
    /// 单例插件必须排在最前：第二个实例在它的 setup 里退出，排后面的话
    /// 开库、起服务这些已经做过一遍了。见 `run()` 里的注释。
    #[test]
    fn 单例插件排在别的插件和_setup_前面() {
        let src = include_str!("lib.rs");
        let body = &src[src.find("pub fn run()").unwrap()..];
        let at = |needle: &str| body.find(needle).unwrap_or_else(|| panic!("run() 里没有 {needle}"));
        let single = at("tauri_plugin_single_instance::init(");
        assert!(single < at("tauri_plugin_opener::init()"));
        assert!(single < at(".setup(|app|"));
        // 挡下之后要把已开的那个拎到前面，不然用户看到的是「点了没反应」
        assert!(body[single..].starts_with("tauri_plugin_single_instance::init(|app, _args, _cwd| bring_to_front(app))"));
    }
}

#[cfg(test)]
mod icon_tests {
    /// **托盘图标必须满画布，应用图标必须留白** —— 两张图的规矩相反，混用哪一张都不报错，
    /// 只是菜单栏里小一圈（2026-09-15 用户截图）或者程序坞里大一圈（同日，反方向）。
    #[test]
    fn 托盘图标满画布_应用图标按_macos_规范留白() {
        let edge = |bytes: &[u8]| -> (u32, f32) {
            let img = image::load_from_memory(bytes).expect("图标解不开").to_rgba8();
            let (w, h) = img.dimensions();
            let opaque = |x: u32| (0..h).any(|y| img.get_pixel(x, y)[3] > 8);
            let left = (0..w).find(|&x| opaque(x)).unwrap_or(0);
            let right = (0..w).rev().find(|&x| opaque(x)).unwrap_or(w - 1);
            (w, (right - left + 1) as f32 / w as f32)
        };

        let (tw, tray) = edge(include_bytes!("../icons/tray.png"));
        assert_eq!(tw, 44, "菜单栏是 22pt，@2x 就是 44px");
        assert!(tray > 0.98, "托盘图标留白了：主体只占 {:.1}%，菜单栏里会比邻居小一圈", tray * 100.0);

        let (aw, app) = edge(include_bytes!("../icons/icon.png"));
        assert!(aw >= 512);
        // macOS 规范 824/1024 = 80.5%；缩放和投影会带来几个点的浮动
        assert!((0.78..0.88).contains(&app), "应用图标的画布占比跑了：{:.1}%（规范 80.5%，Dock 里按这个和别人对齐）", app * 100.0);
    }
}
