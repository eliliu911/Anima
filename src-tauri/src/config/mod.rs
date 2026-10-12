//! 应用配置。
//!
//! 配置存 JSON（`~/Library/Application Support/com.anima.desktop/config.json`），
//! API Key 单独存 Keychain（见 [`secrets`]），两者用 provider id 关联。

pub mod secrets;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::error::{Error, Result};

/// 后端协议类型。决定用哪个适配器，而不是靠猜 base_url。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// OpenAI 官方
    OpenAi,
    /// Claude / Anthropic Messages API
    Anthropic,
    /// 任意 OpenAI 兼容端点（LM Studio 默认走这个）
    OpenAiCompatible,
    /// Ollama 原生 /api/chat
    Ollama,
    /// Azure OpenAI（部署名在 URL 里、api-key 头、max_completion_tokens）
    AzureOpenAi,
}

/// 任务分层。网关据此选模型：实时链路要快，后台任务要准。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// 实时语音链路，目标端到端 1-2 秒
    Realtime,
    /// 后台复杂任务：多步 MCP 编排、记忆整理、深度推理
    Background,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    /// 唯一标识，同时作为 Keychain 的 account 名
    pub id: String,
    /// UI 上显示的名字
    pub label: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: String,
    /// 该后端流式工具调用是否可靠。MLX server 请设为 false。
    #[serde(default = "default_true")]
    pub supports_tools: bool,
    /// 是否需要 API Key。本地后端通常不需要。
    #[serde(default)]
    pub requires_key: bool,
    /// 仅 Azure OpenAI 用：对话接口的 api-version。留空则用适配器里的默认值。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_version: Option<String>,
}

fn default_tts_engine() -> String {
    "edge".to_string()
}

fn default_startup_window() -> String {
    "avatar".to_string()
}

fn default_theme_mode() -> String {
    "system".to_string()
}

/// 设定卡专用 LoRA 的一项。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SheetLora {
    /// ComfyUI 枚举里的文件名。**不猜、不模糊匹配** —— 对不上时工具会
    /// 把机器上有哪些列回去，而不是悄悄不挂。
    pub name: String,
    /// 权重。版式 LoRA（CharTurnXL）已经占了 0.55，叠上来的风格 LoRA
    /// 通常要更低，否则会把版式压掉。
    pub strength: f64,
}

/// 2026-09-26 起是 Qwen-Image 2.1（用户拍板）。理由和代价见 `image_engine::DEFAULT_ENGINE`。
pub(crate) fn default_image_engine() -> String {
    crate::agent_runtime::image_engine::DEFAULT_ENGINE.id().to_string()
}

fn default_true() -> bool {
    true
}

/// 每条链路的思考档位。
///
/// **按链路分而不是按模型分**，因为同一个模型可以同时服务两个档 ——
/// 按模型记的话，在后台档给它开了思考，切回实时档会跟着开。
///
/// 默认值来自本机实测：qwen3.6-35b-a3b-uncensored 的一句闲聊，
/// 思考开着要 6.99s 才吐第一个正文字，关掉是 0.08s。实时档等不起，
/// 而后台档（摘要压缩、事实提取、复杂推理）存在的意义正是拿时间换质量。
///
/// 模型支持哪些档、「关」该怎么发到线上，都由
/// [`crate::llm::thinking`] 决定 —— 这里只表达意图。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThinkingPolicy {
    /// 实时链路（文字对话的实时档）
    pub realtime: crate::llm::ThinkingEffort,
    /// 后台链路（文字对话的后台档）
    pub background: crate::llm::ThinkingEffort,
}

impl Default for ThinkingPolicy {
    fn default() -> Self {
        Self {
            realtime: crate::llm::ThinkingEffort::Off,
            background: crate::llm::ThinkingEffort::Auto,
        }
    }
}

impl ThinkingPolicy {
    pub fn for_tier(&self, tier: Tier) -> crate::llm::ThinkingEffort {
        match tier {
            Tier::Realtime => self.realtime,
            Tier::Background => self.background,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    /// 当前生效的技能（角色扮演）id，支持多选
    #[serde(default)]
    pub active_skills: Vec<String>,
    /// 每个面板窗口上次的位置与尺寸。**按窗口分别记** ——
    /// 每次开都弹回屏幕正中用起来很烦
    #[serde(default)]
    pub window_geometry: std::collections::HashMap<String, crate::windows::Geometry>,
    /// 全局生效的默认角色 id
    #[serde(default)]
    pub active_character: Option<String>,
    /// 界面语言，例如 `zh-CN` / `en-US`。`None` = 跟随前端默认。
    ///
    /// **前端的 localStorage 不够用。** 语言原来只存在 localStorage 里，
    /// 而 macOS 托盘菜单是 Rust 在启动时建的、读不到 localStorage ——
    /// 于是每次启动托盘都是英文，用户要进设置里再切一次才对。
    #[serde(default)]
    pub language: Option<String>,
    /// **只在角色设定卡上生效**的 LoRA。空 = 不额外挂。
    ///
    /// **刻意不做成"全局常挂"**，那样有三处会打架：
    ///
    /// 1. 和模型每次自己传的 `loras` 叠加，权重会失控；
    /// 2. 和别的路径（图生图、控姿势）混在一起，效果不可预期；
    /// 3. **最危险的一条**：LoRA 只在 SDXL 架构上有效，而默认引擎是
    ///    Z-Image —— 全局常挂会在那条路上**一声不吭地什么都不做**，
    ///    用户以为风格生效了，实际没有。
    ///
    /// 限定在设定卡上就没有第 3 条：那条路本来就强制回落 SDXL。
    #[serde(default)]
    pub sheet_loras: Vec<SheetLora>,
    /// Z-Image 那条路常挂的 LoRA。
    ///
    /// 当初否掉「全局常挂」有三条理由，其中**最危险的第 3 条**是
    /// 「LoRA 只在 SDXL 架构上有效，而默认引擎是 Z-Image，会一声不吭地什么都不做」。
    /// 那条现在不成立了 —— Z-Image 架构的 LoRA 实测生效（同 seed 对照，
    /// 99.9% 像素改变、画质不掉，见 `image_engine::lora_arch`）。
    ///
    /// 但另外两条还在，所以生效范围被卡死在**纯文生图 + 这次调用没自己传 loras**：
    ///
    /// 1. 和模型自己传的 `loras` 叠加 → 权重失控。所以只在对方没传时才挂。
    /// 2. 和图生图 / 控姿势 / 设定卡混 → 效果不可预期。那几条路本来就回落 SDXL，
    ///    挂 Z-Image 权重会退化成第 3 条那种静默失效。
    #[serde(default)]
    pub zimage_loras: Vec<SheetLora>,
    /// SDXL 那条路用哪个 checkpoint。`None` = 按内置优先级自动挑。
    ///
    /// **做成可选是因为默认值本身有倾向。** 内置优先级把 `RealVisXL` 排第一，
    /// 那是个欧美写实底模；而机器上同时装着 `AWPortrait_XL`（国内社区的系列），
    /// 做国风角色设定卡明显更合适 —— 交领、纱质外层、金边层次、玉簪都更清楚，
    /// 多视图之间的脸也更一致。谁更合适取决于用户在做什么，不该由代码替他定。
    ///
    /// SDXL 这条路是绕不开的：挂 LoRA、控姿势、出角色设定卡都强制走它。
    #[serde(default)]
    pub sdxl_checkpoint: Option<String>,
    /// H3 参考生视频 / 长镜头用**原生 Ref2VA 权重**吗。默认 `false` = 兼容模式（FL2VA 权重跑 Ref2VA 版式）。
    ///
    /// 2026-10-06 用户定的默认：原生权重装好后是「装了就自动用」，EP03 第 2 镜同种子对照三条
    /// （旧格式 + 原生、官方格式 + 原生、官方格式 + 原生再跑一次）**第二女角色都被星澜顶掉**，兼容模式那条正常。
    /// 原因还没坐实，见 `docs/reports/2026-10-06-原生Ref2VA串人排查.md`。没装原生权重时这一项不起作用。
    #[serde(default)]
    pub h3_native_ref2va: bool,
    /// 出图默认用哪个底模（`ImageEngine::id`）。默认 `qwen21`，见 `image_engine::DEFAULT_ENGINE`。
    ///
    /// 模型可以在单次调用里用 `engine` 参数覆盖它。
    ///
    /// 用到 LoRA / ControlNet / 设定卡版式时会**强制回落 SDXL** —— 那些权重
    /// 只有 SDXL 架构的，挂到别的引擎上不生效且不报错（见 `image_engine`）。
    #[serde(default = "default_image_engine")]
    pub image_engine: String,
    /// 启动时默认显示的视图或窗口："avatar"（虚拟人桌宠）或 "chat"（Chat 对话窗）
    #[serde(default = "default_startup_window")]
    pub startup_window: String,
    /// 界面主题：`"system"`（跟随系统）/ `"light"` / `"dark"`。
    ///
    /// **存在配置里而不是各窗口自己的 localStorage** ——
    /// 那样对话窗和 Studio 会各记各的，切一个另一个不动。
    #[serde(default = "default_theme_mode")]
    pub theme_mode: String,
    /// 交互模式：陪伴 / 助手。见 `voice::mode` —— 所有会打架的参数
    /// （话长、语速、动作频率、情绪强度）都挂在这一个开关上
    #[serde(default)]
    pub interaction_mode: crate::voice::InteractionMode,
    pub providers: Vec<ProviderConfig>,
    /// 实时链路用哪个 provider id
    pub realtime_provider: String,
    /// 后台任务用哪个 provider id
    pub background_provider: String,
    /// 工具调用的备用后端。
    ///
    /// 当前链路的后端不支持流式工具调用时用它。`None` = 自动挑
    /// （优先配了密钥的云端，见 `Gateway::resolve`）。
    ///
    /// 可以显式指定的理由：自动挑的规则是"任何云端"，而用户可能有多个云端
    /// 后端、只想用其中某一个（比如便宜的那个）跑工具。
    #[serde(default)]
    pub tool_fallback_provider: Option<String>,

    /// 每条链路各自用哪个模型。
    ///
    /// **模型属于链路，不属于后端。** 否则两条链路指向同一个后端时必然共用
    /// 一个模型 —— 而"实时对话用 8B、后台推理用 35B、都在本机 LM Studio 上"
    /// 是最自然的配法，本机资源也够。
    ///
    /// `None` = 沿用该后端的 `model` 字段（老配置的行为，兼容用）。
    #[serde(default)]
    pub realtime_model: Option<String>,
    #[serde(default)]
    pub background_model: Option<String>,
    #[serde(default)]
    pub tool_fallback_model: Option<String>,
    /// **已废弃**，只为读旧配置保留。新代码一律用 [`AppConfig::thinking`]。
    ///
    /// 一个布尔表达不了现实：实测同一台机器上 `gpt-oss` 族只认
    /// low/medium/high、其余族只认「关」，而实时档和后台档的需求正好相反。
    /// `load` 会把它迁移到 [`ThinkingPolicy`] 之后就不再读。
    #[serde(default)]
    pub disable_thinking: bool,
    /// 每条链路各自的思考档位。
    #[serde(default)]
    pub thinking: ThinkingPolicy,
    /// 语音识别配置
    #[serde(default)]
    pub stt: crate::stt::SttConfig,
    /// 语音管线配置（VAD 阈值、音色、设备等）
    #[serde(default)]
    pub voice: crate::voice::VoiceConfig,
    /// 记忆系统配置（嵌入后端、召回条数与阈值）
    #[serde(default)]
    pub memory: crate::memory::MemoryConfig,
    /// 可选的 TTS 引擎。默认三个（Edge / OpenAI 兼容 / GPT-SoVITS），
    /// 与 providers 同一套路：只补不改，用户改过的值不会被覆盖
    #[serde(default = "crate::tts::TtsEngineConfig::defaults")]
    pub tts_engines: Vec<crate::tts::TtsEngineConfig>,
    /// 当前选中的 TTS 引擎 id
    #[serde(default = "default_tts_engine")]
    pub tts_engine: String,
    /// 用户配置的 MCP server。默认空 —— 不预置任何 server，
    /// 因为每个 server 都是一次外部代码执行的授权，不该由我们替用户决定
    #[serde(default)]
    pub mcp_servers: Vec<crate::mcp::McpServerConfig>,
}

impl Default for AppConfig {
    /// 默认值直接对准本机已经在跑的服务（见 DECISIONS.md）：
    /// LM Studio 作默认实时后端，Ollama 作后台后端（原生 API，工具调用可靠）。
    ///
    /// 这里的 model 只是首次启动时的初始值 —— 设置面板会实时拉取各后端的真实
    /// 模型列表，所以不必、也不应该把它当成"支持的模型清单"来维护。
    fn default() -> Self {
        Self {
            image_engine: default_image_engine(),
            zimage_loras: Vec::new(),
            sdxl_checkpoint: None,
            h3_native_ref2va: false,
            sheet_loras: Vec::new(),
            language: None,
            active_skills: Vec::new(),
            window_geometry: Default::default(),
            active_character: None,
            startup_window: default_startup_window(),
            theme_mode: default_theme_mode(),
            interaction_mode: Default::default(),
            providers: vec![
                ProviderConfig {
                    id: "lmstudio".into(),
                    label: "LM Studio（本地）".into(),
                    kind: ProviderKind::OpenAiCompatible,
                    base_url: "http://127.0.0.1:1234".into(),
                    // A3B MoE：每 token 仅激活约 3B 参数，适合实时链路
                    model: "qwen/qwen3.6-35b-a3b".into(),
                    supports_tools: true,
                    // LM Studio 的 token 校验是可选的：开了就把 token 存进
                    // Keychain，连接器有 key 就带上、没有就不带。所以这里不设
                    // requires_key，避免在没开校验时也强制要求填。
                    requires_key: false,
                    api_version: None,
                },
                ProviderConfig {
                    id: "ollama".into(),
                    label: "Ollama（本地）".into(),
                    kind: ProviderKind::Ollama,
                    base_url: "http://127.0.0.1:11434".into(),
                    model: "qwen3:32b".into(),
                    supports_tools: true,
                    requires_key: false,
                    api_version: None,
                },
                ProviderConfig {
                    id: "mlx".into(),
                    label: "MLX server（本地，纯聊天）".into(),
                    kind: ProviderKind::OpenAiCompatible,
                    base_url: "http://127.0.0.1:8080".into(),
                    model: "mlx-community/Qwen3-8B-4bit".into(),
                    // 工具调用支持三套本地服务里最不成熟，显式关掉
                    supports_tools: false,
                    requires_key: false,
                    api_version: None,
                },
                ProviderConfig {
                    id: "azure".into(),
                    label: "Azure OpenAI".into(),
                    kind: ProviderKind::AzureOpenAi,
                    base_url: "https://your-resource.openai.azure.com".into(),
                    model: "gpt-4o".into(),
                    supports_tools: true,
                    requires_key: true,
                    api_version: None,
                },
                ProviderConfig {
                    id: "openai".into(),
                    label: "OpenAI（官方）".into(),
                    kind: ProviderKind::OpenAi,
                    base_url: "https://api.openai.com".into(),
                    model: "gpt-4o".into(),
                    supports_tools: true,
                    requires_key: true,
                    api_version: None,
                },
                ProviderConfig {
                    id: "claude".into(),
                    label: "Claude".into(),
                    kind: ProviderKind::Anthropic,
                    base_url: "https://api.anthropic.com".into(),
                    model: "claude-sonnet-4-5".into(),
                    supports_tools: true,
                    requires_key: true,
                    api_version: None,
                },
            ],
            realtime_provider: "lmstudio".into(),
            background_provider: "ollama".into(),
            tool_fallback_provider: None,
            realtime_model: None,
            background_model: None,
            tool_fallback_model: None,
            disable_thinking: false,
            thinking: ThinkingPolicy::default(),
            stt: crate::stt::SttConfig::default(),
            voice: crate::voice::VoiceConfig::default(),
            memory: crate::memory::MemoryConfig::default(),
            // 不预置任何 MCP server：每接一个 server 都等于授权一段外部代码
            // 在本机运行，这个决定必须由用户自己做
            tts_engines: crate::tts::TtsEngineConfig::defaults(),
            tts_engine: default_tts_engine(),
            mcp_servers: Vec::new(),
        }
    }
}

impl AppConfig {
    pub fn provider(&self, id: &str) -> Result<&ProviderConfig> {
        self.providers
            .iter()
            .find(|p| p.id == id)
            .ok_or_else(|| Error::UnknownProvider(id.to_string()))
    }

    /// 某个分层当前选中的 provider id
    pub fn provider_id_for(&self, tier: Tier) -> &str {
        match tier {
            Tier::Realtime => &self.realtime_provider,
            Tier::Background => &self.background_provider,
        }
    }

    /// 校验配置自洽：两个分层指向的 provider 必须存在。
    ///
    /// 手改过 config.json 之后很容易出现悬空引用，早点报错好过运行时才发现。
    pub fn validate(&self) -> Result<()> {
        for (tier, id) in [
            ("realtime_provider", &self.realtime_provider),
            ("background_provider", &self.background_provider),
        ] {
            if self.providers.iter().all(|p| &p.id != id) {
                return Err(Error::config(format!("{tier} 指向了不存在的后端 \"{id}\"")));
            }
        }

        let mut seen = std::collections::HashSet::new();
        for p in &self.providers {
            if !seen.insert(&p.id) {
                return Err(Error::config(format!("后端 id \"{}\" 重复", p.id)));
            }
        }
        Ok(())
    }

    /// 把默认配置里有、但已存配置里缺的后端补进来。
    ///
    /// 没有这一步的话，将来新增一个后端（比如加 Azure OpenAI），所有已经跑过一次
    /// 的机器都看不到它 —— 因为 config.json 早就落盘了，`load` 只是反序列化。
    /// 只**补**不**改**：已存在的同 id 后端保持用户改过的 base_url / 模型不动。
    ///
    /// 返回补进来的后端 id，方便日志说明发生了什么。
    fn merge_new_defaults(&mut self) -> Vec<String> {
        let defaults = Self::default();
        let mut added = Vec::new();

        for candidate in defaults.providers {
            if self.providers.iter().any(|p| p.id == candidate.id) {
                continue;
            }
            added.push(candidate.id.clone());
            self.providers.push(candidate);
        }

        // TTS 引擎同样只补不改。漏掉这段的后果是：老 config.json 里只有旧的
        // 三个引擎，新加的引擎在设置面板里**根本不出现**，看起来像没做
        for candidate in defaults.tts_engines {
            if self.tts_engines.iter().any(|e| e.id == candidate.id) {
                continue;
            }
            added.push(format!("tts:{}", candidate.id));
            self.tts_engines.push(candidate);
        }
        added
    }

    pub fn load(path: &PathBuf) -> Result<Self> {
        if !path.exists() {
            let config = Self::default();
            config.save(path)?;
            return Ok(config);
        }

        let raw = std::fs::read_to_string(path)?;
        // 有没有 `thinking` 这个键，决定要不要从旧的 disable_thinking 迁移。
        // serde 的 default 抹掉了「缺省」和「显式写了默认值」的区别，所以先看原文。
        let had_thinking_key = serde_json::from_str::<serde_json::Value>(&raw)
            .ok()
            .and_then(|v| v.get("thinking").cloned())
            .is_some();
        let mut config: Self = serde_json::from_str(&raw).map_err(|err| {
            Error::config(format!("config.json 解析失败（{}）：{err}", path.display()))
        })?;

        let added = config.merge_new_defaults();
        let migrated =
            config.seed_engine_voice_from_global() | config.migrate_thinking(had_thinking_key);
        config.validate()?;

        if !added.is_empty() || migrated {
            if !added.is_empty() {
                tracing::info!(added = ?added, "配置里补入了新增的后端");
            }
            config.save(path)?;
        }
        Ok(config)
    }

    /// 把旧的 `disable_thinking` 布尔迁移成按链路的 [`ThinkingPolicy`]。
    ///
    /// **忠实迁移，不趁机改用户的选择。** 用户原来关着思考就两个档都关，
    /// 开着就两个档都不干预 —— 想要「实时关、后台开」这种新能力，
    /// 得他自己去设。默认值（realtime=Off）只对全新配置生效。
    ///
    /// 只在配置里**没有** `thinking` 键时做一次；有了就说明已经迁移过或用户设过。
    fn migrate_thinking(&mut self, had_thinking_key: bool) -> bool {
        if had_thinking_key {
            return false;
        }
        use crate::llm::ThinkingEffort;
        let effort = if self.disable_thinking {
            ThinkingEffort::Off
        } else {
            ThinkingEffort::Auto
        };
        self.thinking = ThinkingPolicy {
            realtime: effort,
            background: effort,
        };
        tracing::info!(?effort, "已把旧的 disable_thinking 迁移成按链路的思考策略");
        true
    }

    /// 把老配置里那个**全局**音色，种进当前选中的那个引擎。
    ///
    /// 音色以前只有一份、所有引擎共用，换引擎时会把上一家的名字带过去
    /// （Kokoro 拿到 `zh-TW-HsiaoYuNeural` 去 HuggingFace 找 `.pt` 然后 404）。
    /// 现在改成每个引擎各记各的 —— 但**不能让用户已经选好的音色凭空消失**，
    /// 所以升级时把它记到当前引擎名下。只做一次：种过之后就有值了。
    fn seed_engine_voice_from_global(&mut self) -> bool {
        let global = self.voice.tts.voice.trim().to_string();
        if global.is_empty() {
            return false;
        }
        let selected = self.tts_engine.clone();
        let Some(engine) = self.tts_engines.iter_mut().find(|e| e.id == selected) else {
            return false;
        };
        if engine.voice.is_some() {
            return false;
        }
        tracing::info!(engine = %selected, voice = %global, "把全局音色迁到当前引擎名下");
        engine.voice = Some(global);
        true
    }

    pub fn save(&self, path: &PathBuf) -> Result<()> {
        self.validate()?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)?;
        Ok(())
    }
}

/// 前端需要的 provider 视图：带上"key 配了没有"，但绝不含 key 本身。
#[derive(Debug, Clone, Serialize)]
pub struct ProviderStatus {
    #[serde(flatten)]
    pub config: ProviderConfig,
    pub has_key: bool,
    /// key 来自环境变量（`.env.local`）而不是 Keychain。
    /// UI 要据此提示"在界面里改不掉"，否则用户存了 Keychain 却发现不生效。
    pub key_from_env: bool,
    /// 这条 key 是从哪儿来的。`None` = 没配。
    ///
    /// **只有 `env` 那条在界面里改不掉。** `file` 指应用数据目录里的
    /// `secrets.env`（权限 600），是现在写入的默认去处。
    pub key_source: Option<secrets::SecretSource>,
    /// 模型能力与预算分级（"local" 或 "cloud"）—— 单一事实来源
    pub model_tier: &'static str,
    /// 这个后端当前选中的模型**实际支持哪些思考档位**，按从省到费排列。
    ///
    /// 空数组表示该模型不支持任何干预 —— 前端**不要显示控件**，
    /// 给一个点了没反应的滑杆比没有滑杆更糟。
    /// 唯一来源是 [`crate::llm::thinking::supported_levels`]。
    pub thinking_levels: Vec<&'static str>,
}

impl ProviderStatus {
    pub fn of(config: &ProviderConfig) -> Self {
        Self {
            config: config.clone(),
            has_key: secrets::has_secret(&config.id),
            key_from_env: secrets::is_from_env(&config.id),
            // 比 `key_from_env` 细一档：界面要区分「环境变量（改不掉）」、
            // 「本机安全文件（可改，也是写入的默认去处）」和「Keychain（老数据）」
            key_source: secrets::secret_source(&config.id),
            // **不在这里重算。** tier 的唯一计算入口是 `agent::tier_of`，
            // agent 流式对话那条路也调它 —— 两处共用一份，改一条 match 臂
            // 也不可能只改到一半
            model_tier: crate::agent::tier_of(config),
            // 同样不在这里重算：档位能力的唯一入口是 llm::thinking
            thinking_levels: crate::llm::thinking::supported_levels(&config.model)
                .iter()
                .map(|l| l.as_str())
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::llm::ThinkingEffort;

    /// 造一份"旧版"配置文件：从默认配置出发，摘掉 `thinking` 键、
    /// 设上旧的 `disable_thinking`，再按 `extra` 覆盖若干字段。
    ///
    /// 不手写最小 JSON —— `providers` 这类字段没有 serde default，
    /// 手写的片段加载会直接失败，测到的就不是迁移逻辑了。
    fn load_legacy(
        disable_thinking: bool,
        extra: &[(&str, serde_json::Value)],
    ) -> (AppConfig, std::path::PathBuf, tempfile::TempDir) {
        let mut v = serde_json::to_value(AppConfig::default()).unwrap();
        let obj = v.as_object_mut().unwrap();
        obj.remove("thinking");
        obj.insert(
            "disable_thinking".into(),
            serde_json::json!(disable_thinking),
        );
        for (k, val) in extra {
            obj.insert((*k).into(), val.clone());
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, serde_json::to_string(&v).unwrap()).unwrap();
        let cfg = AppConfig::load(&path).expect("应当能加载");
        (cfg, path, dir)
    }

    #[test]
    fn 全新配置的默认是实时关后台开() {
        // 依据是本机实测：同一句闲聊，思考开着 6.99s 才出第一个正文字，关掉 0.08s。
        // 实时等不起；后台档存在的意义就是拿时间换质量。
        let p = ThinkingPolicy::default();
        assert_eq!(p.realtime, ThinkingEffort::Off);
        assert_eq!(p.background, ThinkingEffort::Auto);
        assert_eq!(p.for_tier(Tier::Realtime), ThinkingEffort::Off);
        assert_eq!(p.for_tier(Tier::Background), ThinkingEffort::Auto);
    }

    #[test]
    fn 旧配置的disable_thinking被忠实迁移而不是套用新默认() {
        // 用户原来开着思考（disable_thinking=false），迁移后两个档都应该是 Auto。
        // **不能趁机套用新默认的 realtime=Off** —— 那是在替用户改选择。
        let (cfg, _p, _d) = load_legacy(false, &[]);
        assert_eq!(cfg.thinking.realtime, ThinkingEffort::Auto);
        assert_eq!(cfg.thinking.background, ThinkingEffort::Auto);

        let (cfg, _p, _d) = load_legacy(true, &[]);
        assert_eq!(cfg.thinking.realtime, ThinkingEffort::Off);
        assert_eq!(cfg.thinking.background, ThinkingEffort::Off);
    }

    #[test]
    fn 迁移只做一次不会覆盖用户后来的设置() {
        // 已经有 thinking 键就说明迁移过或用户设过，这时 disable_thinking 是历史残留，
        // 再读它会把用户的选择打回去。
        let (cfg, _p, _d) = load_legacy(
            true,
            &[(
                "thinking",
                serde_json::json!({"realtime": "high", "background": "off"}),
            )],
        );
        assert_eq!(cfg.thinking.realtime, ThinkingEffort::High);
        assert_eq!(cfg.thinking.background, ThinkingEffort::Off);
    }

    #[test]
    fn 迁移结果会落盘否则下次启动又迁一遍() {
        let (_cfg, path, _d) = load_legacy(true, &[]);
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"thinking\""), "迁移后必须写回文件：{raw}");
    }

    #[test]
    fn provider_status_下发的档位来自能力表而不是写死() {
        let mut c = ProviderConfig {
            id: "x".into(),
            label: "x".into(),
            kind: ProviderKind::OpenAiCompatible,
            base_url: "http://127.0.0.1:1234".into(),
            model: "openai/gpt-oss-120b".into(),
            supports_tools: true,
            requires_key: false,
            api_version: None,
        };
        // gpt-oss 有梯度但没有「关」
        let s = ProviderStatus::of(&c);
        assert_eq!(s.thinking_levels, vec!["low", "medium", "high"]);

        // 二态族只有开关
        c.model = "ornith-1.5-35b-a3b".into();
        assert_eq!(ProviderStatus::of(&c).thinking_levels, vec!["off", "auto"]);

        // 不思考的模型给空数组，前端据此不显示控件
        c.model = "qwen/qwen3-coder-next".into();
        assert!(ProviderStatus::of(&c).thinking_levels.is_empty());
    }

    #[test]
    fn default_config_is_self_consistent() {
        AppConfig::default().validate().expect("默认配置应当自洽");
    }

    #[test]
    fn rejects_dangling_tier_reference() {
        let config = AppConfig {
            realtime_provider: "nope".into(),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_duplicate_provider_ids() {
        let mut config = AppConfig::default();
        let dup = config.providers[0].clone();
        config.providers.push(dup);
        assert!(config.validate().is_err());
    }

    #[test]
    fn merge_adds_missing_providers_without_touching_existing_ones() {
        // 模拟一份"老版本"配置：只有 lmstudio，且用户改过它的模型
        let mut old = AppConfig {
            zimage_loras: Vec::new(),
            image_engine: default_image_engine(),
            sdxl_checkpoint: None,
            h3_native_ref2va: false,
            sheet_loras: Vec::new(),
            language: None,
            interaction_mode: Default::default(),
            active_skills: Vec::new(),
            window_geometry: Default::default(),
            active_character: None,
            startup_window: default_startup_window(),
            theme_mode: default_theme_mode(),
            providers: vec![ProviderConfig {
                id: "lmstudio".into(),
                label: "LM Studio（本地）".into(),
                kind: ProviderKind::OpenAiCompatible,
                base_url: "http://127.0.0.1:9999".into(),
                model: "我自己挑的模型".into(),
                supports_tools: true,
                requires_key: false,
                api_version: None,
            }],
            realtime_provider: "lmstudio".into(),
            background_provider: "lmstudio".into(),
            tool_fallback_provider: None,
            realtime_model: None,
            background_model: None,
            tool_fallback_model: None,
            disable_thinking: false,
            thinking: ThinkingPolicy::default(),
            stt: crate::stt::SttConfig::default(),
            voice: crate::voice::VoiceConfig::default(),
            memory: crate::memory::MemoryConfig::default(),
            tts_engines: crate::tts::TtsEngineConfig::defaults(),
            tts_engine: default_tts_engine(),
            mcp_servers: Vec::new(),
        };

        let added = old.merge_new_defaults();

        // 新后端补进来了
        assert!(added.contains(&"azure".to_string()));
        assert!(old.provider("azure").is_ok());
        // 用户改过的值一个都没被覆盖
        let lmstudio = old.provider("lmstudio").unwrap();
        assert_eq!(lmstudio.base_url, "http://127.0.0.1:9999");
        assert_eq!(lmstudio.model, "我自己挑的模型");
        old.validate().expect("合并后应当仍然自洽");
    }

    #[test]
    fn merge_is_idempotent() {
        let mut config = AppConfig::default();
        assert!(
            config.merge_new_defaults().is_empty(),
            "默认配置不该再补出任何东西"
        );
    }

    #[test]
    fn mlx_defaults_to_tools_disabled() {
        let config = AppConfig::default();
        assert!(!config.provider("mlx").unwrap().supports_tools);
    }
}

#[cfg(test)]
mod voice_migration_tests {
    use super::*;

    /// 升级不能让用户已经选好的音色凭空消失 —— 把全局那份种到当前引擎名下
    #[test]
    fn the_global_voice_moves_into_the_selected_engine() {
        let mut c = AppConfig::default();
        c.tts_engine = "edge".into();
        c.voice.tts.voice = "zh-TW-HsiaoYuNeural".into();

        assert!(c.seed_engine_voice_from_global());
        let edge = c.tts_engines.iter().find(|e| e.id == "edge").unwrap();
        assert_eq!(edge.voice.as_deref(), Some("zh-TW-HsiaoYuNeural"));
    }

    /// **只种到当前那个引擎。** 种给所有引擎的话就等于没修 ——
    /// Kokoro 又会拿到一个 Edge 的音色名
    #[test]
    fn other_engines_are_left_alone() {
        let mut c = AppConfig::default();
        c.tts_engine = "edge".into();
        c.voice.tts.voice = "zh-TW-HsiaoYuNeural".into();
        c.seed_engine_voice_from_global();

        for e in c.tts_engines.iter().filter(|e| e.id != "edge") {
            assert!(e.voice.is_none(), "{} 被塞了别家的音色名", e.id);
        }
    }

    /// 只做一次：引擎已经有自己的音色就别覆盖
    #[test]
    fn it_never_overwrites_a_voice_the_engine_already_has() {
        let mut c = AppConfig::default();
        c.tts_engine = "edge".into();
        c.voice.tts.voice = "新的".into();
        if let Some(e) = c.tts_engines.iter_mut().find(|e| e.id == "edge") {
            e.voice = Some("原来的".into());
        }
        assert!(!c.seed_engine_voice_from_global());
        let edge = c.tts_engines.iter().find(|e| e.id == "edge").unwrap();
        assert_eq!(edge.voice.as_deref(), Some("原来的"));
    }

    #[test]
    fn nothing_to_migrate_is_not_a_change() {
        let mut c = AppConfig::default();
        c.voice.tts.voice = "  ".into();
        assert!(!c.seed_engine_voice_from_global());
    }
}
