//! 语音管线的 Tauri command 层。
//!
//! 与文字对话分开一个文件，是因为两者的生命周期完全不同：文字对话是「一问一答，
//! 每轮一条流」，语音是「一个长期会话，中间随时打断」。混在一起会让 commands.rs
//! 里两套状态互相纠缠。
//!
//! 事件走 [`VOICE_EVENT`] 广播。前端契约见 `docs/VOICE_API.md`。

use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::error::{Error, Result};
use crate::memory::{Memory, MemoryStats};
use crate::stt::{self, whisper::WhisperStt};
use crate::tts::{TextToSpeech, VoiceInfo};
use crate::voice::{self, VoiceConfig, VoiceDeps, VoiceEvent, VoiceState};
use crate::AppState;

/// 语音事件名。前端 `listen(VOICE_EVENT, ...)`。
pub const VOICE_EVENT: &str = "anima://voice";

#[derive(Debug, Serialize)]
pub struct VoiceStatus {
    pub state: VoiceState,
    /// 是否已加载 STT 模型
    pub stt_ready: bool,
    /// 当前使用的模型路径（空 = 未配置）
    pub model_path: String,
    pub config: VoiceConfig,
}

/// 启动语音对话。
///
/// 首次调用会加载 Whisper 模型（约 200ms），并申请麦克风权限 ——
/// 系统会弹一次授权框，这是必需且只弹一次的。
#[tauri::command]
pub async fn voice_start(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: Option<String>,
    project_id: Option<String>,
) -> Result<VoiceStatus> {
    // **启动必须串行。**
    //
    // "检查有没有在跑"和"把会话存进去"之间隔着两百多行（建 STT、组装提示词、
    // 开采集）。两个窗口同时点麦克风的话，两次调用都会看到 None、都往下走，
    // 于是同时开两个采集实例抢同一个设备 —— 表现是"语音开不起来"。
    //
    // 每个窗口是独立 webview、各有一份 store，所以前端的 busy 标志挡不住 ——
    // 那是**每个窗口自己的** busy。唯一能串行的地方在这里。
    let _starting = start_lock().lock().await;

    // 已经在跑就直接返回现状，避免重复开设备
    if state.voice.read().await.is_some() {
        return voice_status(state).await;
    }
    // 刚停过就等设备释放干净 —— 否则新流是死的，见 wait_for_capture_release
    wait_for_capture_release().await;

    let (voice_config, stt_config) = {
        let config = state.gateway.config();
        let config = config.read().await;
        let mut stt_config = config.stt.clone();

        // 没配模型路径时，去约定目录里找一个 —— 用户把 ggml-*.bin 丢进去就能用
        if stt_config.model_path.is_empty() {
            if let Some(found) = stt::discover_model(&state.config_dir) {
                tracing::info!(model = %found, "自动发现 Whisper 模型");
                stt_config.model_path = found;
            }
        }
        (config.voice.clone(), stt_config)
    };

    if stt_config.model_path.is_empty() {
        return Err(Error::config(format!(
            "未找到语音识别模型。请把 ggml-*.bin 放进 {}，或在设置里指定路径",
            stt::default_model_dir(&state.config_dir).display()
        )));
    }

    // ── Whisper：命中缓存就直接用 ──
    //
    // 模型文件几百 MB（small 是 465MB），从磁盘加载要几百毫秒，而这段延迟
    // 正好压在用户按下"开语音"之后。同一个模型没有理由反复读。
    //
    // 锁的粒度是整个"查缓存 + 加载"：并发调两次 start_voice 时，
    // 后者等前者加载完直接命中，而不是各自加载一份几百 MB 的模型。
    let stt = {
        let mut cache = state.stt_cache.lock().await;
        match cache.as_ref() {
            // 配置完全一致才算命中 —— 换模型、改线程数都要重新加载
            Some(cached) if cached.config == stt_config => {
                tracing::debug!(model = %stt_config.model_path, "复用已加载的 Whisper 模型");
                Arc::clone(&cached.stt)
            }
            _ => {
                let started = std::time::Instant::now();
                let for_load = stt_config.clone();
                // 模型加载是阻塞的，别堵住 command 所在的 async 运行时
                let loaded = tokio::task::spawn_blocking(move || WhisperStt::load(for_load))
                    .await
                    .map_err(|e| Error::Io(format!("加载模型任务异常：{e}")))??;
                let loaded = Arc::new(loaded);
                tracing::info!(
                    model = %stt_config.model_path,
                    ms = started.elapsed().as_millis(),
                    "加载 Whisper 模型"
                );
                *cache = Some(crate::CachedStt {
                    config: stt_config.clone(),
                    stt: Arc::clone(&loaded),
                });
                loaded
            }
        }
    };

    // 事件回调：把管线事件转成 Tauri 广播。
    // 管线本身不认识 Tauri，这层转换只在这里。
    let emitter = app.clone();
    let events: voice::EventSink = Arc::new(move |event: VoiceEvent| {
        if let Err(err) = emitter.emit(VOICE_EVENT, &event) {
            tracing::warn!(%err, "语音事件广播失败");
        }
    });

    // 记忆系统是可选增强：起不来（比如嵌入后端没开）也要能正常语音对话，
    // 只是不记不召。所以这里失败只记日志、不阻断启动。
    let memory = ensure_memory(&state).await;

    // 从项目里拿到自建的 system_prompt 或者是 character_id
    let mut db_project = None;
    if let Some(m) = &memory {
        if let Some(pid) = &project_id {
            if let Ok(projects) = m.store().get_projects().await {
                db_project = projects.into_iter().find(|p| p.id == *pid);
            }
        }
    }

    // **只换身份，不换能力。**
    //
    // 角色/项目提供的是"你是谁"，而动作标签、联网、语音口吻这些是
    // "你能做什么" —— 原先这里直接 `system_prompt = 角色提示词`，把能力那段
    // 整个覆盖掉了，于是换上角色之后它不会挥手，只会发 emoji。
    let mut identity = crate::voice::DEFAULT_IDENTITY.to_string();
    let mut current_character = None;
    // 项目工作区 —— 项目级技能（`<工作区>/.anima/skills/`）要靠它才找得到。
    // 这里已经查过 projects 表，不用再多一次往返。
    let project_workspace: Option<std::path::PathBuf> = db_project
        .as_ref()
        .and_then(|p| p.workspace_path.clone())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from);

    if let Some(p) = &db_project {
        if let Some(char_id) = &p.character_id {
            let chars_dir = crate::character::characters_dir(&state.config_dir);
            if let Ok(character) = crate::character::load(&chars_dir, char_id) {
                identity = character.build_identity_prompt();
                current_character = Some(character);
            } else {
                tracing::warn!("无法加载 Character ID: {}，将降级为默认 Prompt", char_id);
            }
        } else if let Some(sp) = &p.system_prompt {
            identity = sp.clone();
        }
    }

    // 如果项目没有绑定角色，则降级使用全局设置的角色
    if current_character.is_none() {
        if let Some(char_id) = &state.gateway.config().read().await.active_character {
            let chars_dir = crate::character::characters_dir(&state.config_dir);
            if let Ok(character) = crate::character::load(&chars_dir, char_id) {
                identity = character.build_identity_prompt();
                current_character = Some(character);
            }
        }
    }
    // 动作清单跟着 animations/ 目录走 —— 写死的话，用户放进去的新动作
    // 模型不知道它存在，那些文件等于白放
    let builtin: Vec<&str> = crate::voice::BUILTIN_ACTIONS
        .iter()
        .map(|(n, _)| *n)
        .collect();
    let extra_actions = crate::avatar::model::action_names_in(
        &crate::avatar::model::animations_dir(&state.config_dir),
        &builtin,
    );
    let mode = state.gateway.config().read().await.interaction_mode;
    let system_prompt = crate::voice::compose_system_prompt(&identity, &extra_actions, mode);
    tracing::info!(
        actions = ?extra_actions.iter().map(|(n, _)| n).collect::<Vec<_>>(),
        "动作库里额外可用的动作"
    );
    tracing::info!(
        character = ?current_character.as_ref().map(|c| &c.id),
        chars = system_prompt.chars().count(),
        "system prompt 组装完成（身份 + 能力）"
    );

    let session_id = match session_id {
        Some(id) => Some(id), // 前端传来了已有的 session，直接使用
        None => match &memory {
            // 没有传，则视情况创建
            Some(m) => match m
                .store()
                .create_session_for_project(project_id.as_deref())
                .await
            {
                Ok(id) => Some(id),
                Err(err) => {
                    tracing::warn!(%err, "创建记忆会话失败，本次不记录对话");
                    None
                }
            },
            None => None,
        },
    };

    // **引擎必须走形态解析，不能读全局配置。**
    //
    // 这里曾经写的是 `current_tts(&state)` —— 它读的是全局 `tts_engine`，
    // 而音色来自当前角色。角色绑了 GPT-SoVITS 的音色（`ADSister`）却拿到
    // 全局的 Edge 引擎，于是 Edge 收到一个它不认识的音色名，
    // 回一句"standard voices will no longer be supported"——
    // 报错内容和真正的问题（引擎和音色不是一家）完全对不上。
    //
    // 会话启动和运行中切换必须得出同一个答案，所以两边都走 resolve_persona。
    let persona = resolve_persona(&state).await;
    let engine_cfg = {
        let lock = state.gateway.config();
        let config = lock.read().await;
        config
            .tts_engines
            .iter()
            .find(|e| e.id == persona.engine.value)
            .cloned()
            .ok_or_else(|| Error::config(format!("找不到 TTS 引擎「{}」", persona.engine.value)))?
    };

    // **开口之前先查一遍配置。** 不查的话，引擎与音色错配要等到第一句
    // 合成失败才暴露，而那时的报错来自上游、和真正的原因对不上
    let problems = crate::tts::preflight::check(&engine_cfg, &persona.voice.value);
    for p in &problems {
        if p.fatal {
            tracing::error!(message = %p.message, "开口前检查：这套配置说不出话");
        } else {
            tracing::warn!(message = %p.message, "开口前检查");
        }
    }

    // await 不能出现在 tracing 宏的参数里 —— 那会把配置的读锁跨过 await 点，
    // 整个 command 的 future 就不再是 Send
    let active_character = persona_character(&state).await;
    tracing::info!(
        engine = %persona.engine.value,
        engine_from = ?persona.engine.origin,
        voice = %persona.voice.value,
        voice_from = ?persona.voice.origin,
        character = ?active_character,
        "本次会话用的语音引擎与音色"
    );

    // 在启动会话前就把引擎造出来：本地引擎配错（缺 base_url、缺参考音频）
    // 应该在这里报错，而不是等到第一句要说话时才发现说不出来
    let tts = crate::tts::engine(
        &engine_cfg,
        state.http.clone(),
        &crate::tts::voices::voices_dir(&state.config_dir),
    )?;
    let tts_uses_voice_library = crate::tts::uses_voice_library(engine_cfg.kind);

    let session = voice::start(
        {
            let mut c = voice_config;
            c.sentence_min_chars = mode.profile().sentence_min_chars;
            // **音色也走形态解析。** 全局那份 voice 是"默认形态的音色"，
            // 角色绑了自己的就该用角色那个
            c.tts.voice = persona.voice.value.clone();
            c
        },
        VoiceDeps {
            tts_uses_voice_library,
            http: state.http.clone(),
            config_dir: state.config_dir.clone(),
            config_path: state.config_path.clone(),
            // 技能：配置里存的是 id 数组，这里解析成完整定义。
            // 解析失败（用户删了那个目录）只记警告，成功的技能保留。
            skills: {
                let wants = state.gateway.config().read().await.active_skills.clone();
                let mut loaded = Vec::new();
                for id in wants {
                    match crate::skill::resolve_layered(
                        &state.config_dir,
                        project_workspace.as_deref(),
                        &id,
                    ) {
                        Ok(s) => loaded.push(s),
                        Err(err) => {
                            tracing::warn!(%err, "配置里的某个技能加载失败");
                        }
                    }
                }
                loaded
            },
            available_skills: crate::skill::list_layered(
                &state.config_dir,
                project_workspace.as_deref(),
            ),
            // actor 里 switch_skill 也要能命中项目技能，所以工作区得跟着会话走
            skills_workspace: project_workspace.clone(),
            gateway: Arc::clone(&state.gateway),
            stt,
            tts,
            events,
            system_prompt,
            memory,
            session_id,
            mcp: Some(state.mcp.clone()),
            runtime: Some(state.runtime.clone()),
            app: Some(app.clone()),
            music_player: state.music_player.clone(),
            extra_actions,
            mode,
            available_characters: crate::character::list_in(&crate::character::characters_dir(
                &state.config_dir,
            )),
            character: current_character,
        },
    )?;

    *state.voice.write().await = Some(session);
    voice_status(state).await
}

/// 切换交互模式（陪伴 / 助手）。
///
/// 会话在跑就热切换（提示词、语速、分句阈值一起变）；没在跑就只写配置，
/// 下次启动生效。两种情况都持久化 —— 否则重启就退回默认那档。
#[tauri::command]
pub async fn set_interaction_mode(
    state: State<'_, AppState>,
    mode: voice::InteractionMode,
) -> Result<voice::InteractionMode> {
    {
        let config = state.gateway.config();
        let mut config = config.write().await;
        config.interaction_mode = mode;
        config.save(&state.config_path)?;
    }
    if let Some(session) = state.voice.read().await.as_ref() {
        session.set_mode(mode)?;
    }
    tracing::info!(mode = %mode.label(), "交互模式已切换");
    Ok(mode)
}

/// 停止语音对话，释放麦克风与音频设备。
#[tauri::command]
pub async fn voice_stop(state: State<'_, AppState>) -> Result<VoiceStatus> {
    // 和启动共用同一把锁：否则"停"和"开"会交错 ——
    // 停到一半时另一个窗口开起来，新会话会被随后的 take() 摘掉
    let _serialized = start_lock().lock().await;
    if let Some(session) = state.voice.write().await.take() {
        session.stop();
        // drop 掉 session 会让桥接线程退出、cpal 流停止、设备释放
        mark_capture_released();
    }
    voice_status(state).await
}

/// 语音会话的启动锁。
///
/// 存在的理由见 `voice_start` 里那段注释：检查与存入之间隔着很长一段异步操作，
/// 不串行的话并发启动会开出多个抢同一个麦克风的采集实例。
fn start_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// 上一次释放麦克风设备的时刻。
///
/// **CoreAudio 释放输入设备不是同步的。** 停掉之后立刻重开，新流会变成
/// 僵尸流：不报任何错，但一个采样都收不到 —— 表现是"麦克风开着但她听不见"。
/// 实测撞到过：连点几次开关，之后就再也收不到声音。
fn capture_released_at() -> &'static std::sync::Mutex<Option<std::time::Instant>> {
    static AT: std::sync::OnceLock<std::sync::Mutex<Option<std::time::Instant>>> =
        std::sync::OnceLock::new();
    AT.get_or_init(|| std::sync::Mutex::new(None))
}

fn mark_capture_released() {
    *capture_released_at().lock().unwrap() = Some(std::time::Instant::now());
}

/// 设备释放得够久了吗；不够就等一下。
///
/// 400ms 是实测留的余量。宁可让"开麦克风"慢 0.4 秒，也不要给一个
/// 永远收不到声音的流 —— 后者用户完全看不出原因。
async fn wait_for_capture_release() {
    let since = capture_released_at().lock().unwrap().map(|t| t.elapsed());
    if let Some(elapsed) = since {
        const SETTLE: std::time::Duration = std::time::Duration::from_millis(400);
        if elapsed < SETTLE {
            let wait = SETTLE - elapsed;
            tracing::debug!(?wait, "等麦克风设备释放干净再开");
            tokio::time::sleep(wait).await;
        }
    }
}

/// 手动打断当前回答（界面上的停止按钮）。
///
/// 与语音打断走同一条路径，所以行为完全一致：取消 LLM 流 + 清空播放队列。
#[tauri::command]
pub async fn voice_interrupt(state: State<'_, AppState>) -> Result<bool> {
    match state.voice.read().await.as_ref() {
        Some(session) => {
            session.interrupt();
            Ok(true)
        }
        None => Ok(false),
    }
}

#[tauri::command]
pub async fn voice_status(state: State<'_, AppState>) -> Result<VoiceStatus> {
    let session_state = match state.voice.read().await.as_ref() {
        Some(session) => session.state().await,
        None => VoiceState::Idle,
    };

    let config = state.gateway.config();
    let config = config.read().await;
    let mut model_path = config.stt.model_path.clone();
    if model_path.is_empty() {
        model_path = stt::discover_model(&state.config_dir).unwrap_or_default();
    }

    Ok(VoiceStatus {
        state: session_state,
        stt_ready: !model_path.is_empty(),
        model_path,
        config: config.voice.clone(),
    })
}

/// 更新语音配置。会话在跑时需要重启才能生效（设备/VAD 参数在启动时固化）。
#[tauri::command]
pub async fn set_voice_config(
    state: State<'_, AppState>,
    config: VoiceConfig,
) -> Result<VoiceStatus> {
    {
        let stored = state.gateway.config();
        let mut stored = stored.write().await;
        stored.voice = config;
        stored.save(&state.config_path)?;
    }
    voice_status(state).await
}

/// 列出可用的麦克风设备
#[tauri::command]
pub async fn list_input_devices() -> Result<Vec<String>> {
    crate::audio::capture::input_devices()
}

/// 按当前配置造 TTS 引擎。
///
/// 三处用到（语音会话、音色列表、试听），逻辑一样就抽出来 ——
/// 分散写的话切引擎时很容易漏掉一处，表现成"设置里切了但试听还是老引擎"。
async fn current_tts(state: &State<'_, AppState>) -> Result<std::sync::Arc<dyn TextToSpeech>> {
    let config = state.gateway.config();
    let config = config.read().await;
    let selected = config
        .tts_engines
        .iter()
        .find(|e| e.id == config.tts_engine)
        .ok_or_else(|| {
            crate::error::Error::config(format!(
                "找不到 TTS 引擎「{}」，请在设置里重新选择",
                config.tts_engine
            ))
        })?;
    crate::tts::engine(
        selected,
        state.http.clone(),
        &crate::tts::voices::voices_dir(&state.config_dir),
    )
}

/// TTS 引擎面板要的数据
#[derive(Debug, serde::Serialize)]
pub struct TtsEngineView {
    pub engines: Vec<crate::tts::TtsEngineConfig>,
    pub selected: String,
}

#[tauri::command]
pub async fn tts_engines(state: State<'_, AppState>) -> Result<TtsEngineView> {
    let config = state.gateway.config();
    let config = config.read().await;
    Ok(TtsEngineView {
        engines: config.tts_engines.clone(),
        selected: config.tts_engine.clone(),
    })
}

/// 切换当前引擎。
///
/// **只检查引擎存在，不检查配置是否完整。**
///
/// 原先这里会立刻构造实例、构造失败就拒绝切换。出发点是"别让用户切完才发现
/// 说不出话"，但造成了死锁：参考音频那一栏只在**选中该引擎后**才显示，
/// 而缺参考音频又切不过去 —— 用户根本没有机会填。
///
/// 现在把校验推迟到真正要合成的时候（`current_tts` 构造实例时），
/// 那里的错误信息一样明确，而且用户已经能进到配置界面了。
#[tauri::command]
pub async fn set_tts_engine(state: State<'_, AppState>, engine_id: String) -> Result<()> {
    {
        let config = state.gateway.config();
        let mut config = config.write().await;
        let selected = config
            .tts_engines
            .iter()
            .find(|e| e.id == engine_id)
            .ok_or_else(|| crate::error::Error::config(format!("找不到 TTS 引擎「{engine_id}」")))?
            .clone();
        // 配置不完整时切换仍然成功，但留一条日志 —— 这样"切过去了却说不出话"
        // 在日志里有迹可循，不用靠猜
        if let Err(err) = crate::tts::engine(
            &selected,
            state.http.clone(),
            &crate::tts::voices::voices_dir(&state.config_dir),
        ) {
            tracing::warn!(
                engine = %selected.id,
                %err,
                "已切换引擎，但当前配置还不完整，需要补齐后才能合成"
            );
        }
        config.tts_engine = engine_id;
        config.save(&state.config_path)?;
    }
    apply_persona_to_session(&state).await?;
    Ok(())
}

/// 改一个引擎的连接配置（base_url / 参考音频等）。
#[tauri::command]
pub async fn update_tts_engine(
    state: State<'_, AppState>,
    engine: crate::tts::TtsEngineConfig,
) -> Result<()> {
    let config = state.gateway.config();
    let mut config = config.write().await;
    let slot = config
        .tts_engines
        .iter_mut()
        .find(|e| e.id == engine.id)
        .ok_or_else(|| crate::error::Error::config(format!("找不到 TTS 引擎「{}」", engine.id)))?;
    *slot = engine;
    config.save(&state.config_path)?;
    drop(config);
    // 改 base_url / 参考音频之后，正在跑的会话也得换成新配置建的引擎
    apply_persona_to_session(&state).await?;
    Ok(())
}

/// 把当前选中的引擎推给正在跑的语音会话。
/// 列出可用的 TTS 音色（从服务端实时拉取，不写死）
#[tauri::command]
pub async fn list_tts_voices(state: State<'_, AppState>) -> Result<Vec<VoiceInfo>> {
    current_tts(&state).await?.voices().await
}

/// 探一个引擎现在能不能用。
///
/// ## 为什么需要它
///
/// 设置面板里列着六个引擎，但**能用的取决于对应服务有没有跑起来**，
/// 而这一点在切换之前完全看不出来。用户只能靠"切过去、说一句话、看它报错"
/// 来发现服务没开 —— 这一路上撞过好几次（Kokoro 没起来时每句话都报
/// "TTS 请求失败"，而界面上没有任何迹象）。
///
/// 探活的方式是拉音色列表：那是唯一一个所有适配器都实现、且不产生副作用
/// （不合成音频、不占用播放队列）的调用。
#[derive(Debug, serde::Serialize)]
pub struct EngineProbe {
    pub id: String,
    /// 能连上并拿到回应
    pub reachable: bool,
    /// 拿到几个音色。零样本克隆的引擎可能是 0 但仍然可用
    pub voices: usize,
    /// 连不上时的原因，原样带给界面
    pub error: Option<String>,
}

#[tauri::command]
pub async fn probe_tts_engine(
    state: State<'_, AppState>,
    engine_id: String,
) -> Result<EngineProbe> {
    let selected = {
        let config = state.gateway.config();
        let config = config.read().await;
        config
            .tts_engines
            .iter()
            .find(|e| e.id == engine_id)
            .cloned()
            .ok_or_else(|| Error::config(format!("没有这个引擎：{engine_id}")))?
    };

    // 建引擎本身就可能失败（缺 base_url、缺参考音频），那也是"不可用"
    let tts = match crate::tts::engine(
        &selected,
        state.http.clone(),
        &crate::tts::voices::voices_dir(&state.config_dir),
    ) {
        Ok(t) => t,
        Err(err) => {
            return Ok(EngineProbe {
                id: engine_id,
                reachable: false,
                voices: 0,
                error: Some(err.to_string()),
            })
        }
    };

    // 加超时：服务挂在半死状态时（端口开着但不回应）不能让界面一直转圈
    match tokio::time::timeout(std::time::Duration::from_secs(6), tts.voices()).await {
        Ok(Ok(list)) if !list.is_empty() => Ok(EngineProbe {
            id: engine_id,
            reachable: true,
            voices: list.len(),
            error: None,
        }),
        // **空列表不等于可达。**
        //
        // 零样本克隆的引擎（GPT-SoVITS / CosyVoice2 / IndexTTS2）的 voices()
        // 直接返回空数组、**根本不碰网络** —— 音色 = 参考音频，没有列表这回事。
        // 所以只看 voices() 会把一个已经死掉的服务报成可达，比没有探活更糟：
        // 给死服务亮绿灯。这里必须真的去连一次。
        Ok(Ok(_)) => Ok(probe_base_url(&state.http, engine_id, &selected.base_url).await),
        Ok(Err(err)) => Ok(EngineProbe {
            id: engine_id,
            reachable: false,
            voices: 0,
            error: Some(err.to_string()),
        }),
        Err(_) => Ok(EngineProbe {
            id: engine_id,
            reachable: false,
            voices: 0,
            error: Some("6 秒没有回应".into()),
        }),
    }
}

/// 直接连一次 base_url，确认那个端口真的有人在听。
///
/// **任何 HTTP 回应都算可达**，包括 404 和 500 —— 我们要判断的是"服务在不在"，
/// 而不是"这个路径对不对"。GPT-SoVITS 的 `/` 就会回 404，它照样是活的。
async fn probe_base_url(http: &reqwest::Client, engine_id: String, base_url: &str) -> EngineProbe {
    let url = base_url.trim_end_matches('/');
    if url.is_empty() {
        // 没有 base_url 的只有 Edge，而它的 voices() 会返回真实列表，
        // 走不到这里。走到了说明配置不完整
        return EngineProbe {
            id: engine_id,
            reachable: false,
            voices: 0,
            error: Some("没有配置 base_url".into()),
        };
    }
    match tokio::time::timeout(std::time::Duration::from_secs(5), http.get(url).send()).await {
        Ok(Ok(_)) => EngineProbe {
            id: engine_id,
            reachable: true,
            // 零样本克隆没有音色列表，0 是准确的信息
            voices: 0,
            error: None,
        },
        Ok(Err(err)) => EngineProbe {
            id: engine_id,
            reachable: false,
            voices: 0,
            error: Some(format!("连不上 {url}：{err}")),
        },
        Err(_) => EngineProbe {
            id: engine_id,
            reachable: false,
            voices: 0,
            error: Some(format!("{url} 5 秒没有回应")),
        },
    }
}

/// 试听某个音色。返回 MP3 字节，由前端播放 —— 这样试听不占用语音管线的播放队列。
#[tauri::command]
pub async fn preview_tts_voice(
    state: State<'_, AppState>,
    voice: String,
    text: Option<String>,
) -> Result<Vec<u8>> {
    let config = crate::tts::TtsConfig {
        voice,
        ..Default::default()
    };
    let text = text.unwrap_or_else(|| "你好，我是 Anima。".to_string());
    let speech = current_tts(&state)
        .await?
        .synthesize(&text, &config)
        .await?;
    Ok(speech.audio)
}

/// 惰性打开记忆系统。已打开则复用。
///
/// 返回 None 表示这次用不了记忆（配置关了，或嵌入后端连不上），
/// 调用方应当照常继续 —— 记忆是增强而不是前提。
pub(crate) async fn ensure_memory(state: &State<'_, AppState>) -> Option<Arc<Memory>> {
    if let Some(existing) = state.memory.read().await.as_ref() {
        return Some(Arc::clone(existing));
    }

    // 先把 Arc 拿出来再借读锁，否则临时的 Arc 在 await 处就被丢掉了
    let stored = state.gateway.config();
    let config = stored.read().await.memory.clone();
    if !config.enabled {
        return None;
    }

    match Memory::open(
        &state.config_dir,
        config,
        Arc::clone(&state.gateway),
        state.http.clone(),
    )
    .await
    {
        Ok(memory) => {
            let memory = Arc::new(memory);
            *state.memory.write().await = Some(Arc::clone(&memory));

            // 记忆一就绪就把 ComfyUI 挂账对上 —— 上次没收回的图/视频在这里补回来。
            //
            // 只跑一次：`ensure_memory` 会被很多路径调到，重复对账会把同一个
            // 结果往会话里补好几遍。
            crate::agent_runtime::comfy_jobs::set_memory(Arc::clone(&memory)).await;
            static RECONCILED: std::sync::atomic::AtomicBool =
                std::sync::atomic::AtomicBool::new(false);
            if !RECONCILED.swap(true, std::sync::atomic::Ordering::SeqCst) {
                let http = state.http.clone();
                let dir = state.config_dir.join(".anima").join("generated");
                tokio::spawn(async move {
                    crate::agent_runtime::comfy_jobs::reconcile_on_startup(http, dir).await;
                });
            }

            Some(memory)
        }
        Err(err) => {
            // 最常见的原因是嵌入后端没起来。语音本身不该因此不可用。
            tracing::warn!(%err, "记忆系统未能启动，本次将不记不召");
            None
        }
    }
}

/// 记忆统计，给设置面板显示「记了多少东西」
#[tauri::command]
pub async fn memory_stats(state: State<'_, AppState>) -> Result<Option<MemoryStats>> {
    match ensure_memory(&state).await {
        Some(memory) => Ok(Some(memory.stats().await?)),
        None => Ok(None),
    }
}

/// 手动召回，用于调试「它到底记住了什么」
#[tauri::command]
pub async fn memory_recall(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<crate::memory::vector::Recalled>> {
    match ensure_memory(&state).await {
        // **不过滤项目。** 这个命令是给"看看它记住了我什么"的界面用的，
        // 而那个界面的意义正是看见全部 —— 包括本来不该被记住的那些。
        // 按项目过滤会让"我明明记过"和"这里查不到"同时成立。
        Some(memory) => Ok(memory
            .recall(&query, &crate::memory::vector::ScopeFilter::Any)
            .await),
        None => Ok(Vec::new()),
    }
}

/// 清空长期记忆（事实），保留对话记录
#[tauri::command]
pub async fn memory_clear_facts(state: State<'_, AppState>) -> Result<bool> {
    match ensure_memory(&state).await {
        Some(memory) => {
            memory.clear_facts().await?;
            Ok(true)
        }
        None => Ok(false),
    }
}

// ---------------------------------------------------------------------------
// 技能（角色扮演）
// ---------------------------------------------------------------------------

/// 设置面板要展示的技能列表 + 当前选中项。
#[derive(Debug, serde::Serialize)]
pub struct SkillView {
    pub skills: Vec<crate::skill::Skill>,
    /// 当前生效的技能 id 列表。
    pub active: Vec<String>,
    /// **没能加载的那些。** 文件在磁盘上、看着像装好了，而解析失败了 ——
    /// 以前这件事只有一行 WARN 日志，界面上完全看不出来。
    #[serde(default)]
    pub broken: Vec<crate::skill::SkillLoadError>,
}

/// 列出所有技能。
///
/// **每次都从磁盘读**，不缓存：技能内容在 `skills/<id>/SKILL.md` 里，用户改完
/// 应该立刻看到，而不用重启。文件只有几个、每个几 KB，读一遍是白菜价。
#[tauri::command]
pub async fn list_skills(
    state: State<'_, AppState>,
    workspace_path: Option<String>,
) -> Result<SkillView> {
    let active = state.gateway.config().read().await.active_skills.clone();
    let ws = workspace_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(std::path::Path::new);
    let project = ws.map(crate::skill::project_skills_dir);
    let (skills, broken) = crate::skill::list_layered_in_reporting(
        project.as_deref(),
        &crate::skill::skills_dir(&state.config_dir),
    );
    Ok(SkillView { skills, active, broken })
}

#[derive(Debug, serde::Deserialize)]
pub struct SaveSkillArgs {
    pub id: String,
    pub name: String,
    pub marker: Option<String>,
    pub description: Option<String>,
    pub prefer_tier: Option<crate::config::Tier>,
    pub thinking: Option<bool>,
    pub tools: Option<Vec<String>>,
    pub source: Option<String>,
    pub kind: Option<crate::skill::SkillKind>,
    pub surface: Option<crate::skill::SkillSurface>,
    pub prompt: Option<String>,
    pub is_project_level: Option<bool>,
    pub workspace_path: Option<String>,
}

/// 创建或更新一个技能
#[tauri::command]
pub async fn save_skill(
    state: State<'_, AppState>,
    args: SaveSkillArgs,
) -> Result<crate::skill::Skill> {
    let ws = args
        .workspace_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(std::path::Path::new);

    let scope = if args.is_project_level.unwrap_or(false) && ws.is_some() {
        crate::skill::SkillScope::Project
    } else {
        crate::skill::SkillScope::Global
    };

    crate::skill::save_skill(
        &state.config_dir,
        ws,
        scope,
        &args.id,
        &args.name,
        args.marker.as_deref().unwrap_or(""),
        args.description.as_deref().unwrap_or(""),
        args.prefer_tier,
        args.thinking,
        args.tools.unwrap_or_default(),
        args.source.as_deref().unwrap_or(""),
        args.kind.unwrap_or_default(),
        args.surface.unwrap_or_default(),
        args.prompt.as_deref().unwrap_or(""),
    )
}

/// 删除一个技能
#[tauri::command]
pub async fn delete_skill(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    id: String,
    workspace_path: Option<String>,
) -> Result<()> {
    let ws = workspace_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(std::path::Path::new);

    crate::skill::delete_skill(&state.config_dir, ws, &id)?;

    // 若删掉的是当前激活的技能，自动移除激活状态并应用
    let is_active = {
        let config = state.gateway.config();
        let mut config = config.write().await;
        if config.active_skills.contains(&id) {
            config.active_skills.retain(|s| s != &id);
            let _ = config.save(&state.config_path);
            true
        } else {
            false
        }
    };

    if is_active {
        apply_persona(&state, &app).await?;
    }

    Ok(())
}

/// `/` 弹层里的一个条目。
///
/// 两类合在一张表里，因为**用户看到的是一个列表**。分成两个接口的话，
/// 排序、过滤、键盘导航都得写两遍，而它们必须表现一致。
#[derive(Debug, Clone, Serialize)]
pub struct SlashEntry {
    /// 敲进去的名字（不含斜杠）
    pub id: String,
    pub title: String,
    pub description: String,
    /// `"skill"`：展开成一段提示词发出去。`"builtin"`：前端执行一个本地动作
    pub kind: &'static str,
    /// 要不要参数。界面据此提示「/review <文件>」
    pub takes_args: bool,
    /// builtin 的动作标识，前端 switch 它。skill 为 None
    pub action: Option<&'static str>,
}

/// 内置动作的注册表。
///
/// **在 Rust 侧列一份而不是前端硬编码**：`/` 的候选列表要和技能合在一起
/// 排序过滤，两边各存一半的话，「有哪些命令」这个问题就有两个答案了 ——
/// 这个仓库在「两处描述同一件事」上栽过好几次。
///
/// 动作**本身**在前端执行（清空对话、开面板都是前端状态），
/// 绕一趟 Tauri 只会让它变慢且多一个失败点。
fn builtin_slash_entries() -> Vec<SlashEntry> {
    vec![
        SlashEntry {
            id: "clear".into(),
            title: "清空对话".into(),
            description: "清掉当前会话的全部消息。不影响已落库的历史。".into(),
            kind: "builtin",
            takes_args: false,
            action: Some("clear"),
        },
        SlashEntry {
            id: "init".into(),
            title: "补齐项目脚手架".into(),
            description: "生成缺失的 .anima/（project.json、rules.md、知识库、示例技能）与 PROMPT.md。已有的文件不覆盖。".into(),
            kind: "builtin",
            takes_args: false,
            action: Some("init"),
        },
        SlashEntry {
            id: "reindex".into(),
            title: "重建知识库索引".into(),
            description: "把 .anima/knowledge/ 重新切块入库。改完文档要点一次。".into(),
            kind: "builtin",
            takes_args: false,
            action: Some("reindex"),
        },
        SlashEntry {
            // **技能的开关搬进对话。**
            //
            // 身份技能持续生效，而它的开关原来只在设置面板里 —— 那个面板是
            // 全局的，而身份技能可以是项目层的（`.anima/skills/`）。
            // 用户的原话：「那个页面是全局设计的，不应该作为单独的项目
            // 或者 conversation 来使用」。
            //
            // 这条命令修不了那个错位（写的还是全局配置），但它把开关放到了
            // 手边，并且在列表里**明说这是全局的**。
            id: "skill".into(),
            title: "技能".into(),
            description: "/skill 看列表 · /skill add <id> 挂载 · /skill remove <id> 卸载。任务型技能不用挂载，直接打 / 选它。".into(),
            kind: "builtin",
            takes_args: true,
            action: Some("skill"),
        },
        SlashEntry {
            id: "settings".into(),
            title: "打开设置".into(),
            description: "模型、记忆、面容、技能都在里面。".into(),
            kind: "builtin",
            takes_args: false,
            action: Some("settings"),
        },
        SlashEntry {
            id: "mcp".into(),
            title: "打开 MCP 面板".into(),
            description: "查看与重连 MCP server。".into(),
            kind: "builtin",
            takes_args: false,
            action: Some("mcp"),
        },
    ]
}

/// `/` 弹层的候选列表：任务技能 + 内置动作。
///
/// 身份型技能**不在这里** —— 它们是勾选后常驻的，在设置面板里选。
/// 混进来会让「敲一次」和「一直生效」两种语义长得一样。
#[tauri::command]
pub async fn list_slash_commands(
    state: State<'_, AppState>,
    workspace_path: Option<String>,
) -> Result<Vec<SlashEntry>> {
    let ws = workspace_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let all = crate::skill::list_layered(&state.config_dir, ws.map(std::path::Path::new));
    let mut out: Vec<SlashEntry> = crate::skill::tasks_for(&all, ws.is_some())
        .into_iter()
        .map(|s| SlashEntry {
            takes_args: crate::skill::expand::placeholders(&s.prompt).takes_args(),
            id: s.id,
            title: s.name,
            description: s.description,
            kind: "skill",
            action: None,
        })
        .collect();
    out.extend(builtin_slash_entries());
    Ok(out)
}

/// 把一个技能命令展开成要发出去的那段文本。
///
/// **在后端展开而不是前端**，理由和 `PROMPT.md` 一样：用户改完 SKILL.md
/// 应该立刻生效。前端缓存一份的话，文件里写的和模型收到的会静默对不上。
#[tauri::command]
pub async fn expand_slash_command(
    state: State<'_, AppState>,
    id: String,
    args: String,
    workspace_path: Option<String>,
) -> Result<String> {
    let ws = workspace_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(std::path::Path::new);
    let skill = crate::skill::resolve_layered(&state.config_dir, ws, &id)?;
    if skill.kind != crate::skill::SkillKind::Task {
        return Err(crate::error::Error::config(format!(
            "「{}」是身份型技能，请在设置面板里勾选，不能用 / 触发。",
            skill.name
        )));
    }
    Ok(crate::skill::expand_task(&skill, &args))
}

#[tauri::command]
pub async fn select_skills(
    state: State<'_, AppState>,
    skills: Vec<String>,
    workspace_path: Option<String>,
) -> Result<()> {
    let wants: Vec<String> = skills
        .into_iter()
        .filter(|s| !s.trim().is_empty())
        .collect();

    let mut resolved = Vec::new();
    for id in &wants {
        // 分层解析：项目技能的 id 也必须能被接受，否则界面上能勾、
        // 一保存就报"没有这个技能"
        resolved.push(crate::skill::resolve_layered(
            &state.config_dir,
            workspace_path
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(std::path::Path::new),
            id,
        )?);
    }

    {
        let config = state.gateway.config();
        let mut config = config.write().await;
        config.active_skills = resolved.iter().map(|s| s.id.clone()).collect();
        config.save(&state.config_path)?;
    }

    if let Some(session) = state.voice.read().await.as_ref() {
        if let Err(e) = session.switch_skills(resolved.clone()) {
            tracing::warn!(%e, "无法向运行中的语音会话推送新技能");
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn select_character(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    character: Option<String>,
) -> Result<()> {
    let want = character.filter(|s| !s.trim().is_empty());

    if let Some(id) = &want {
        let root = crate::character::characters_dir(&state.config_dir);
        crate::character::load(&root, id)?;
    }

    let loaded = match &want {
        Some(id) => {
            let root = crate::character::characters_dir(&state.config_dir);
            Some(crate::character::load(&root, id)?)
        }
        None => None,
    };

    {
        let config = state.gateway.config();
        let mut config = config.write().await;
        config.active_character = want.clone();
        config.save(&state.config_path)?;
    }

    // **必须通知正在跑的会话。** 只写 config.json 的话，运行中的语音会话
    // 用的还是旧角色 —— 身份提示词和情绪音色都不变，表现就是
    // "切了默认形态，但人还是原来那个、声音也没换"。
    // `set_interaction_mode` 一直是这么做的，这里当初漏了
    if let Some(session) = state.voice.read().await.as_ref() {
        session.set_character(loaded)?;
    }

    // 引擎/音色/面容/技能统一走形态解析。**切回默认形态也要走** ——
    // 那一遍的作用就是让每一项都落回兜底层
    if let Some(session) = state.voice.read().await.as_ref() {
        // 旧的"角色音色覆盖"已经并进形态解析了，这里清掉免得两处打架
        session.set_character_voice(None)?;
    }
    apply_persona(&state, &app).await?;
    tracing::info!(character = ?want, "角色已切换");

    Ok(())
}

/// 浏览长期记忆（按时间倒序）。
///
/// 和 `memory_recall` 是两件事：召回要先猜一个查询词，而**"它记住了我什么"这个
/// 问题没有列表就永远回答不了**。记忆系统不可见时用户无法建立信任。
///
/// 倒序是因为新记的最可能是错的 —— 刚说过的话最容易被提取歪。
#[tauri::command]
pub async fn memory_list_facts(
    state: State<'_, AppState>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> Result<Vec<crate::memory::vector::Fact>> {
    match ensure_memory(&state).await {
        // 默认 50 条：一屏能翻完，而且这张表通常也就几百条
        Some(memory) => {
            memory
                .list_facts(limit.unwrap_or(50), offset.unwrap_or(0))
                .await
        }
        None => Ok(Vec::new()),
    }
}

/// 删掉一条记忆。
///
/// 存在的意义就是**记错了要能删**。原先只有"清空全部"，粒度太粗 —— 为了删掉
/// 一条错的而丢掉全部正确的，没人会真的去用。
#[tauri::command]
pub async fn memory_forget_fact(state: State<'_, AppState>, id: String) -> Result<bool> {
    match ensure_memory(&state).await {
        Some(memory) => {
            memory.forget_fact(&id).await?;
            tracing::info!(%id, "删除了一条记忆");
            Ok(true)
        }
        None => Ok(false),
    }
}

/// 手动记一条。
///
/// 有它是因为**提取是模型做的，会漏**。用户明确想让它记住某件事的时候，
/// 不该只能等模型下次刚好提取到。
#[tauri::command]
pub async fn memory_add_fact(
    state: State<'_, AppState>,
    subject: String,
    fact: String,
) -> Result<bool> {
    let fact = fact.trim().to_string();
    if fact.is_empty() {
        return Err(Error::config("要记的内容是空的".to_string()));
    }
    let subject = {
        let s = subject.trim();
        // 主体留空就当"用户" —— 绝大多数手动添加的记忆都是关于用户自己的
        if s.is_empty() {
            "用户".to_string()
        } else {
            s.to_string()
        }
    };
    match ensure_memory(&state).await {
        Some(memory) => {
            // session_id 用一个可辨认的常量，便于将来区分"手动记的"和"提取的"
            // 手动记的一律是全局：用户在设置面板里加的是"关于我"的事，
            // 那个界面上没有项目这个概念
            memory.add_fact("manual", &subject, &fact, "").await?;
            tracing::info!(%subject, %fact, "手动记了一条");
            Ok(true)
        }
        None => Ok(false),
    }
}

/// 列出可用的 3D 面容模型。
#[tauri::command]
pub fn avatar_models(state: State<'_, AppState>) -> Vec<crate::avatar::model::AvatarModel> {
    crate::avatar::model::list_in(&crate::avatar::model::models_dir(&state.config_dir))
}

/// 模型目录的绝对路径。界面要显示它 —— 「把模型放哪」是用户唯一需要知道的事，
/// 让他自己去猜等于让这个功能不可用。
#[tauri::command]
pub fn avatar_models_dir(state: State<'_, AppState>) -> String {
    crate::avatar::model::models_dir(&state.config_dir)
        .display()
        .to_string()
}

/// 在系统文件管理器（访达/资源管理器）中直接打开素材目录
#[tauri::command]
pub fn open_avatar_models_dir(state: State<'_, AppState>) -> Result<()> {
    let dir = crate::avatar::model::models_dir(&state.config_dir);
    if !dir.exists() {
        let _ = std::fs::create_dir_all(&dir);
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(&dir).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer").arg(&dir).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(&dir).spawn();
    }
    Ok(())
}

/// 读出一个模型的字节，交给前端的 GLTFLoader.parse()。
///
/// 走 command 而不是让前端读文件：这个项目刻意不给前端文件系统权限
/// （见 `capabilities/`），为一个只读文件放开整类权限不划算。
#[tauri::command]
pub fn avatar_model_bytes(state: State<'_, AppState>, id: String) -> Result<Vec<u8>> {
    crate::avatar::model::read_in(&crate::avatar::model::models_dir(&state.config_dir), &id)
}

/// 列出动作库（`.vrma`）。
///
/// VRMA 是 VRM 官方的动画格式，绑的是**标准骨骼名**而不是某个模型的节点，
/// 所以一份动作所有角色通用。用户从 VRoid 之类的地方下回来直接丢进目录。
#[tauri::command]
pub fn avatar_animations(state: State<'_, AppState>) -> Vec<crate::avatar::model::AvatarModel> {
    crate::avatar::model::list_animations_in(&crate::avatar::model::animations_dir(
        &state.config_dir,
    ))
}

/// 读一个 `.vrma` 的字节，交给前端解析。路径穿越由 `read_in` 挡。
#[tauri::command]
pub fn avatar_animation_bytes(state: State<'_, AppState>, id: String) -> Result<Vec<u8>> {
    crate::avatar::model::read_in(
        &crate::avatar::model::animations_dir(&state.config_dir),
        &id,
    )
}

/// 让前端往 anima.log 里写一行。
///
/// 加它的理由很实际：Tauri 的 webview 没有可读的控制台，前端出问题只能靠
/// 界面上显示什么去猜。而"模型加载了但不显示"这类问题，画面上什么都没有 ——
/// 猜了好几轮都没结果。有了这条，前端的诊断能直接进日志，`anima logs` 就能看。
#[tauri::command]
pub fn log_ui(scope: String, message: String) {
    tracing::info!(target: "ui", %scope, %message, "前端诊断");
}

/// 角色绑定面板要的一切：这个角色绑了什么，以及**每一项有哪些可选**。
///
/// 选项和绑定一起返回，是因为它们必须对得上：界面只显示"绑了 `yaemiko/d`"
/// 而不知道有哪些可选，用户就只能去翻目录 —— 那等于没做这个面板。
#[derive(Debug, serde::Serialize)]
pub struct CharacterBindingView {
    pub binding: crate::character::binding::Binding,
    pub engines: Vec<crate::tts::TtsEngineConfig>,
    /// 选中引擎能提供的音色。**拉不到时是空数组而不是报错** ——
    /// 服务没起来是常态，不该让整个面板打不开
    pub voices: Vec<VoiceInfo>,
    /// 拉音色失败的原因，原样带给界面。空数组 + 有原因 = 服务没起来；
    /// 空数组 + 没原因 = 这个引擎本来就不提供列表（零样本克隆的都是）
    pub voices_error: Option<String>,
    pub avatars: Vec<crate::avatar::model::AvatarModel>,
    pub skills: Vec<crate::skill::Skill>,
}

/// 读一个角色的绑定 + 所有可选项。
///
/// `engine` 传了就按它拉音色列表（界面上刚改过引擎、还没保存时用）；
/// 没传就按这个角色已经绑的引擎，再没有就按全局当前引擎。
#[tauri::command]
pub async fn character_binding(
    state: State<'_, AppState>,
    id: String,
    engine: Option<String>,
) -> Result<CharacterBindingView> {
    let root = crate::character::characters_dir(&state.config_dir);
    let binding = crate::character::binding::read(&root, &id);

    let (engines, fallback_engine) = {
        let lock = state.gateway.config();
        let config = lock.read().await;
        (config.tts_engines.clone(), config.tts_engine.clone())
    };

    let wanted = engine
        .filter(|s| !s.trim().is_empty())
        .or_else(|| binding.engine.clone())
        .unwrap_or(fallback_engine);

    let (voices, voices_error) = match engines.iter().find(|e| e.id == wanted) {
        Some(cfg) => match crate::tts::engine(
            cfg,
            state.http.clone(),
            &crate::tts::voices::voices_dir(&state.config_dir),
        ) {
            Ok(tts) => match tts.voices().await {
                Ok(v) => (v, None),
                Err(err) => (Vec::new(), Some(err.to_string())),
            },
            Err(err) => (Vec::new(), Some(err.to_string())),
        },
        None => (Vec::new(), Some(format!("找不到引擎「{wanted}」"))),
    };

    Ok(CharacterBindingView {
        binding,
        engines,
        voices,
        voices_error,
        avatars: crate::avatar::model::list_in(&crate::avatar::model::models_dir(
            &state.config_dir,
        )),
        skills: crate::skill::list_in(&crate::skill::skills_dir(&state.config_dir)),
    })
}

/// 保存一个角色的绑定。
///
/// **改的正好是当前角色时要立刻生效**，否则又是"设了没反应" ——
/// 这个项目刚在切引擎/切角色上栽过一次，同一个形状不要再来一遍。
#[tauri::command]
pub async fn set_character_binding(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    id: String,
    binding: crate::character::binding::Binding,
) -> Result<()> {
    let root = crate::character::characters_dir(&state.config_dir);
    crate::character::binding::write(&root, &id, &binding)?;
    tracing::info!(character = %id, ?binding, "保存角色绑定");

    let is_active = state
        .gateway
        .config()
        .read()
        .await
        .active_character
        .as_deref()
        == Some(id.as_str());
    if is_active {
        apply_persona(&state, &app).await?;
    }
    Ok(())
}

/// 读出当前的形态解析结果。**所有"现在该用什么"的问题都问这里。**
///
/// 兜底层就是全局配置（默认形态那一份）。角色绑了什么就盖住哪一项,
/// 没绑的落回兜底 —— 见 [`crate::persona`] 的模块注释。
/// 当前角色 id，只用于日志
async fn persona_character(state: &State<'_, AppState>) -> Option<String> {
    state.gateway.config().read().await.active_character.clone()
}

pub async fn resolve_persona(state: &State<'_, AppState>) -> crate::persona::Resolved {
    let (fallback, engines, active) = {
        let lock = state.gateway.config();
        let config = lock.read().await;
        let voice = config
            .tts_engines
            .iter()
            .find(|e| e.id == config.tts_engine)
            .map(crate::tts::effective_voice)
            .unwrap_or_default();
        (
            crate::persona::Fallback {
                engine: config.tts_engine.clone(),
                voice,
                // 默认形态的面容由前端自己存（three 的场景在前端手里）,
                // 所以兜底层这里是 None —— 前端不收到换脸事件就保持原样
                avatar: None,
                skills: config.active_skills.clone(),
            },
            config.tts_engines.clone(),
            config.active_character.clone(),
        )
    };

    let binding = active.as_deref().map(|id| {
        let root = crate::character::characters_dir(&state.config_dir);
        crate::character::binding::read(&root, id)
    });

    let voice_of_engine = |id: &str| {
        engines
            .iter()
            .find(|e| e.id == id)
            .map(crate::tts::effective_voice)
            .unwrap_or_default()
    };
    crate::persona::resolve(binding.as_ref(), &fallback, &voice_of_engine)
}

/// 把解析结果推给正在跑的会话 + 前端。
///
/// **只推,不写全局 config。** 写全局是上一版最大的错误：角色绑的引擎
/// 被永久写进配置,切回默认形态不还原 —— 设置里显示 Edge、实际跑 SoVITS。
pub async fn apply_persona(state: &State<'_, AppState>, app: &tauri::AppHandle) -> Result<()> {
    apply_persona_inner(state, Some(app)).await
}

/// 同上，但不广播面容。改引擎配置这类场景没有 AppHandle，
/// 而且那时候面容本来也没变
pub async fn apply_persona_to_session(state: &State<'_, AppState>) -> Result<()> {
    apply_persona_inner(state, None).await
}

async fn apply_persona_inner(
    state: &State<'_, AppState>,
    app: Option<&tauri::AppHandle>,
) -> Result<()> {
    let resolved = resolve_persona(state).await;

    let engine_cfg = {
        let lock = state.gateway.config();
        let config = lock.read().await;
        config
            .tts_engines
            .iter()
            .find(|e| e.id == resolved.engine.value)
            .cloned()
    };

    if let Some(session) = state.voice.read().await.as_ref() {
        match engine_cfg {
            Some(cfg) => match crate::tts::engine(
                &cfg,
                state.http.clone(),
                &crate::tts::voices::voices_dir(&state.config_dir),
            ) {
                Ok(engine) => {
                    session.set_tts(
                        engine,
                        resolved.voice.value.clone(),
                        crate::tts::uses_voice_library(cfg.kind),
                    )?;
                    tracing::info!(
                        engine = %resolved.engine.value,
                        voice = %resolved.voice.value,
                        from = ?resolved.engine.origin,
                        "形态已应用到会话"
                    );
                }
                // 建不起来时**继续用旧引擎**,比换成一个说不出话的强
                Err(err) => tracing::warn!(
                    engine = %resolved.engine.value, %err,
                    "新引擎建不起来，会话继续用旧的"
                ),
            },
            None => tracing::warn!(
                engine = %resolved.engine.value,
                "形态指向了一个不存在的引擎"
            ),
        }

        let root = crate::skill::skills_dir(&state.config_dir);
        let mut loaded = Vec::new();
        for sid in &resolved.skills.value {
            match crate::skill::resolve(&root, sid) {
                Ok(sk) => loaded.push(sk),
                Err(err) => tracing::warn!(skill = %sid, %err, "形态绑的技能加载失败"),
            }
        }
        session.switch_skills(loaded)?;
    }

    // 面容与形态状态：广播给前端所有窗口
    if let Some(app) = app {
        if let Some(model) = resolved.avatar.value.as_deref() {
            let _ = app.emit("anima://avatar-model", model);
        } else if resolved.is_default_form {
            let _ = app.emit("anima://avatar-model-clear", ());
        }
        let _ = app.emit("anima://persona-changed", ());
    }
    Ok(())
}

/// 形态面板要的一切：当前解析结果（含每项来源）+ 所有可选项。
///
/// **来源必须一起给。** 选中角色时面板是禁用的，只显示一个灰掉的值
/// 而不说"这项由角色决定"，用户只会以为坏了。
#[derive(Debug, serde::Serialize)]
pub struct PersonaView {
    pub resolved: crate::persona::Resolved,
    /// 当前角色 id。None = 默认形态
    pub active: Option<String>,
    pub characters: Vec<crate::character::Character>,
    pub engines: Vec<crate::tts::TtsEngineConfig>,
    pub avatars: Vec<crate::avatar::model::AvatarModel>,
    pub skills: Vec<crate::skill::Skill>,
    /// **开口之前就能查出来的配置问题。**
    ///
    /// 这个项目"没声音"查了好几轮，每次根因都是配置不匹配，而且每次都要
    /// 等到真的要说话时才暴露。这里把判断提前 —— 见 `tts::preflight`
    pub problems: Vec<crate::tts::preflight::Problem>,
}

#[tauri::command]
pub async fn persona_view(state: State<'_, AppState>) -> Result<PersonaView> {
    let resolved = resolve_persona(&state).await;
    let (active, engines) = {
        let lock = state.gateway.config();
        let config = lock.read().await;
        (config.active_character.clone(), config.tts_engines.clone())
    };
    let problems = engines
        .iter()
        .find(|e| e.id == resolved.engine.value)
        .map(|e| crate::tts::preflight::check(e, &resolved.voice.value))
        .unwrap_or_else(|| {
            vec![crate::tts::preflight::Problem {
                message: format!("找不到引擎「{}」", resolved.engine.value),
                fatal: true,
            }]
        });

    let root = crate::character::characters_dir(&state.config_dir);
    Ok(PersonaView {
        problems,
        resolved,
        active,
        characters: crate::character::list_in(&root),
        engines,
        avatars: crate::avatar::model::list_in(&crate::avatar::model::models_dir(
            &state.config_dir,
        )),
        skills: crate::skill::list_in(&crate::skill::skills_dir(&state.config_dir)),
    })
}

#[tauri::command]
pub async fn create_character(
    state: State<'_, AppState>,
    id: String,
    name: String,
    description: String,
) -> Result<crate::character::Character> {
    let root = crate::character::characters_dir(&state.config_dir);
    let c = crate::character::create(&root, &id, &name, &description)?;
    tracing::info!(character = %c.id, "新建角色形态");
    Ok(c)
}

#[tauri::command]
pub async fn update_character(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    id: String,
    name: String,
    description: String,
) -> Result<crate::character::Character> {
    let root = crate::character::characters_dir(&state.config_dir);
    let c = crate::character::update_meta(&root, &id, &name, &description)?;
    // 改的正好是当前角色时要立刻生效 —— 身份提示词是从这些字段拼的
    let is_active = state
        .gateway
        .config()
        .read()
        .await
        .active_character
        .as_deref()
        == Some(id.as_str());
    if is_active {
        if let Some(session) = state.voice.read().await.as_ref() {
            session.set_character(Some(c.clone()))?;
        }
        apply_persona(&state, &app).await?;
    }
    Ok(c)
}

/// 删掉一个角色形态。**删的如果正是当前形态，要退回默认形态** ——
/// 不退的话运行中的会话还挂着一个已经不存在的角色
#[tauri::command]
pub async fn delete_character(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    id: String,
) -> Result<()> {
    let mut deleted = false;
    let candidates = [
        crate::character::characters_dir(&state.config_dir),
        state.config_dir.join("characters"),
        std::path::PathBuf::from("characters"),
    ];

    for root in &candidates {
        if root.join(&id).is_dir() && crate::character::delete(root, &id).is_ok() {
            deleted = true;
        }
    }

    if !deleted {
        return Err(Error::config(format!("找不到角色「{id}」")));
    }

    // 联动清理生成时产生的肖像与底模文件
    let _ = std::fs::remove_file(
        state
            .config_dir
            .join("public")
            .join("portraits")
            .join(format!("{}.jpg", id)),
    );
    let _ = std::fs::remove_file(
        std::path::Path::new("public")
            .join("portraits")
            .join(format!("{}.jpg", id)),
    );
    let _ = std::fs::remove_file(
        std::path::Path::new("avatars")
            .join("neural")
            .join(format!("{}.jpg", id)),
    );

    let was_active = {
        let lock = state.gateway.config();
        let mut config = lock.write().await;
        let was = config.active_character.as_deref() == Some(id.as_str());
        if was {
            config.active_character = None;
            config.save(&state.config_path)?;
        }
        was
    };
    if was_active {
        if let Some(session) = state.voice.read().await.as_ref() {
            session.set_character(None)?;
            session.set_character_voice(None)?;
        }
        apply_persona(&state, &app).await?;
        tracing::info!(character = %id, "删掉的是当前形态，已退回默认形态");
    }
    Ok(())
}

/// 让角色层窗口忽略（或恢复接收）鼠标事件。
///
/// 桌宠必须同时满足两件互相矛盾的事：不能吃掉你点向下面应用的点击，
/// 又要在光标停到她身上时能拖能点。判定与滞后在前端
/// （`avatar/passthrough.ts`，那里有测试），这里只负责调系统 API。
///
/// **窗口不存在不是错误** —— 角色层可能被用户隐藏了
#[tauri::command]
pub fn set_avatar_passthrough(app: tauri::AppHandle, ignore: bool) -> Result<()> {
    use tauri::Manager;
    let Some(window) = app.get_webview_window("avatar") else {
        return Ok(());
    };
    window
        .set_ignore_cursor_events(ignore)
        .map_err(|e| Error::Io(format!("设置鼠标穿透失败：{e}")))?;
    Ok(())
}

/// 查询角色层当前是否可见
#[tauri::command]
pub fn get_avatar_visible(app: tauri::AppHandle) -> Result<bool> {
    use tauri::Manager;
    let Some(window) = app.get_webview_window("avatar") else {
        return Ok(false);
    };
    Ok(window.is_visible().unwrap_or(false))
}

/// 显示 / 隐藏角色层。托盘里的「隐藏她」及 Chat 侧边栏开关用这个
#[tauri::command]
pub fn set_avatar_visible(app: tauri::AppHandle, visible: bool) -> Result<()> {
    use tauri::{Emitter, Manager};
    let Some(window) = app.get_webview_window("avatar") else {
        return Ok(());
    };
    let r = if visible {
        window.show()
    } else {
        window.hide()
    };
    let _ = app.emit("avatar-visibility-changed", visible);
    r.map_err(|e| Error::Io(format!("切换角色层可见性失败：{e}")))
}

/// 打开（或聚焦）一个面板窗口。位置和尺寸会用上次记住的
#[tauri::command]
pub async fn open_panel(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    label: String,
) -> Result<()> {
    let saved = state
        .gateway
        .config()
        .read()
        .await
        .window_geometry
        .get(&label)
        .copied();
    crate::windows::open(&app, &label, saved)
}

/// 记住一个窗口现在的位置与尺寸。
///
/// **由前端在窗口关闭前调**：Tauri 的窗口销毁事件里已经拿不到几何信息了，
/// 那时候读出来是 0 或者屏幕外的坐标，存下去下次会开到看不见的地方
#[tauri::command]
pub async fn remember_panel_geometry(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    label: String,
) -> Result<()> {
    let Some(geo) = crate::windows::geometry_of(&app, &label) else {
        return Ok(());
    };
    let lock = state.gateway.config();
    let mut config = lock.write().await;
    if config.window_geometry.get(&label) == Some(&geo) {
        return Ok(());
    }
    config.window_geometry.insert(label, geo);
    config.save(&state.config_path)?;
    Ok(())
}

/// 体检：把当前**实际生效**的配置读出来。
///
/// 每一项都对应一种查过很久的失败 —— 见 `diagnostics.rs` 的模块注释。
/// 判断逻辑全在那个纯函数里，这里只负责去各处捞数据。
#[tauri::command]
pub async fn diagnostics(state: State<'_, AppState>) -> Result<Vec<crate::diagnostics::Check>> {
    let resolved = resolve_persona(&state).await;
    let engine_id = resolved.engine.value.clone();

    let (embedding_model, recall_threshold, mut stt_model) = {
        let lock = state.gateway.config();
        let config = lock.read().await;
        (
            config.memory.embedding_model.clone(),
            config.memory.min_similarity,
            config.stt.model_path.clone(),
        )
    };
    if stt_model.is_empty() {
        stt_model = stt::discover_model(&state.config_dir).unwrap_or_default();
    }

    // 记忆层是惰性的：这里**不主动把它拉起来**。体检要报的是"现在是什么样"，
    // 顺手启动一个重量级子系统会把被观测的东西改掉
    let (memory_ready, embedding_dimensions) = match state.memory.read().await.as_ref() {
        Some(memory) => (
            true,
            memory.stats().await.map(|s| s.dimensions).unwrap_or(0),
        ),
        None => (false, 0),
    };

    let animation_count = crate::avatar::model::list_animations_in(
        &crate::avatar::model::animations_dir(&state.config_dir),
    )
    .len();
    let silero_present = crate::audio::silero::model_path(&state.config_dir).is_file();

    // ComfyUI 没在跑就是 None，那一条体检项直接不出现
    let comfy_argv = crate::comfyui::running_argv().await;

    // `probe_tts_engine` 会把 state 拿走，后面还要用 config_dir，先留一份
    let app_root = crate::sidecar::find_app_root(None, Some(&state.config_dir))
        .map(|p| p.to_string_lossy().to_string());

    // 探活放最后：它可能要等 6 秒超时，前面那些不该被它拖着
    let probe = probe_tts_engine(state, engine_id.clone()).await.ok();

    Ok(crate::diagnostics::checks(&crate::diagnostics::Input {
        silero_present,
        embedding_model,
        embedding_dimensions,
        recall_threshold,
        memory_ready,
        animation_count,
        tts_engine: engine_id,
        tts_reachable: probe.as_ref().is_some_and(|p| p.reachable),
        tts_voices: probe.as_ref().map(|p| p.voices).unwrap_or(0),
        tts_error: probe.and_then(|p| p.error),
        stt_model,
        comfy_argv,
        // 找不到时回 None —— 诊断页会把「本地引擎为什么全离线」说清楚
        app_root,
    }))
}

/// 查询所有本地 Python Sidecar（4D数字人、本地语音）的运行状态
#[tauri::command]
pub async fn sidecar_status() -> Result<Vec<crate::sidecar::SidecarStatusInfo>> {
    Ok(crate::sidecar::get_sidecar_statuses().await)
}

/// 动态更新系统托盘菜单的语言与文字
#[tauri::command]
pub fn update_tray_menu(app: tauri::AppHandle, lang: String) -> Result<()> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};

    // 文案和启动时那份共用 —— 各写一份必然漂
    let (toggle_title, quit_title) = crate::windows::tray_titles(&lang);

    let toggle_item = MenuItem::with_id(&app, "toggle_avatar", toggle_title, true, None::<&str>)
        .map_err(|e| Error::Io(format!("创建菜单项失败: {e}")))?;

    let panel_items: Vec<_> = crate::windows::labels_for_lang(&lang)
        .into_iter()
        .map(|(label, title)| {
            MenuItem::with_id(&app, format!("panel:{label}"), title, true, None::<&str>)
        })
        .collect::<std::result::Result<_, _>>()
        .map_err(|e| Error::Io(format!("创建面板菜单项失败: {e}")))?;

    let quit_item = MenuItem::with_id(&app, "quit", quit_title, true, None::<&str>)
        .map_err(|e| Error::Io(format!("创建退出菜单项失败: {e}")))?;
    let separator = PredefinedMenuItem::separator(&app)
        .map_err(|e| Error::Io(format!("创建分隔线失败: {e}")))?;

    let menu = Menu::with_items(&app, &{
        let mut items: Vec<&dyn tauri::menu::IsMenuItem<_>> = panel_items
            .iter()
            .map(|i| i as &dyn tauri::menu::IsMenuItem<_>)
            .collect();
        items.push(&separator);
        items.push(&toggle_item);
        items.push(&quit_item);
        items
    })
    .map_err(|e| Error::Io(format!("构建菜单失败: {e}")))?;

    if let Some(tray) = app.tray_by_id("main_tray") {
        let _ = tray.set_menu(Some(menu));
    }

    Ok(())
}

