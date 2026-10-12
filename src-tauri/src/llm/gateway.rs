//! 模型网关。对上层只暴露"按分层发起对话"，隐藏后端差异与选路逻辑。
//!
//! 两个关键职责：
//!   1. **选路**：按 [`Tier`] 挑 provider，并在后端不支持流式工具调用时降级
//!      （见 ARCHITECTURE.md 3.2.1 / 坑点 8）。
//!   2. **可取消**：每条流配一个 [`CancellationToken`]，barge-in 时能立刻掐掉
//!      整条 LLM 生成流，而不是等它自己跑完（见 ARCHITECTURE.md 坑点 2）。

use futures_util::StreamExt;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use super::anthropic::AnthropicConnector;
use super::azure::AzureOpenAiConnector;
use super::ollama::OllamaConnector;
use super::openai::OpenAiConnector;
use super::{ChatChunk, ChatRequest, ChatStream, FinishReason, LlmConnector, Message, Role};
use crate::config::{secrets, AppConfig, ProviderConfig, ProviderKind, Tier};
use crate::error::{Error, Result};

/// 云端请求的超时。本地大模型首 token 可能很慢（模型冷加载），
/// 所以只设连接超时，不设整体超时 —— 整体超时由 CancellationToken 负责。
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// 换档时塞给后台模型的一句交代。前台小模型可能已经说了一句开场白（「好，我帮你查一下」），
/// 语音链路里那句已经念出去了 —— 后台模型不能再说一遍，也不能当没发生过。
pub const TOOL_ESCALATION_NOTE: &str = "（前台助手已经对用户说了上面那句开场白。接下来需要调用工具，由你接手：\
直接调用需要的工具并完成回答，不要重复开场白。）";

pub struct Gateway {
    config: Arc<RwLock<AppConfig>>,
    http: reqwest::Client,
    /// stream_id -> 取消令牌
    inflight: Arc<RwLock<HashMap<String, CancellationToken>>>,
}

/// 把散落的 system 消息合并成开头的一条。
///
/// 顺序保持不变（拼接时按原顺序），其余消息的相对次序也不变。
/// 一条都没有时什么也不做 —— 不要凭空造一条空的 system。
///
/// 为什么必须做：模型的 chat 模板普遍只接受**一条**、且必须是第一条。
/// Anima 有三个地方会各自加 system —— 项目指令（`commands.rs`）、
/// 通用人设和历史折叠提示（前端 `buildBoundedMessages`）——
/// 谁都不知道别人加过，攒到两条就 500。
pub(crate) fn merge_system_messages(messages: &mut Vec<Message>) {
    // **只有一条路径。** 早先给「已经只有一条」开了条早退分支，结果那条路
    // 不过滤空白 —— 一条只有空格的 system 会被留下，而某些模板同样拒绝它。
    // 两条路径就会有两种行为，统一走这一条。
    let mut merged: Vec<String> = Vec::new();
    let mut rest = Vec::with_capacity(messages.len());
    for m in messages.drain(..) {
        if m.role == Role::System {
            if !m.content.trim().is_empty() {
                merged.push(m.content);
            }
        } else {
            rest.push(m);
        }
    }
    *messages = rest;
    if !merged.is_empty() {
        messages.insert(0, Message::system(merged.join("\n\n")));
    }
}

impl Gateway {
    pub fn new(config: AppConfig) -> Result<Self> {
        // 云端模型走 HTTPS，同样要先有 rustls 的加密后端。
        // 幂等，见 `crate::ensure_crypto_provider`
        crate::ensure_crypto_provider();
        let http = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            // 本地回环连接不需要代理，也避免系统代理把 127.0.0.1 的请求劫走
            .no_proxy()
            .build()?;

        Ok(Self {
            config: Arc::new(RwLock::new(config)),
            http,
            inflight: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    pub fn config(&self) -> Arc<RwLock<AppConfig>> {
        Arc::clone(&self.config)
    }

    /// 按 provider 配置造一个连接器。
    ///
    /// 每次请求都新建而不是缓存：连接器本身是无状态的（`reqwest::Client`
    /// 内部有连接池，clone 是廉价的），这样改配置立刻生效，不用做失效逻辑。
    pub fn connector(&self, provider: &ProviderConfig) -> Result<Box<dyn LlmConnector>> {
        let key = if provider.requires_key {
            let key = secrets::get_secret(&provider.id)?.filter(|k| !k.is_empty());
            match key {
                Some(k) => Some(k),
                None => {
                    return Err(Error::Credential(format!(
                        "{} 需要 API Key，请先在设置里填写",
                        provider.label
                    )))
                }
            }
        } else {
            // 不强制要求，但配了就带上（例如开了 token 校验的 LM Studio）
            secrets::get_secret(&provider.id)?.filter(|k| !k.is_empty())
        };

        let http = self.http.clone();

        Ok(match provider.kind {
            ProviderKind::Anthropic => Box::new(AnthropicConnector::new(
                &provider.base_url,
                &provider.model,
                key.unwrap_or_default(),
                http,
            )),
            ProviderKind::AzureOpenAi => Box::new(AzureOpenAiConnector::new(
                &provider.base_url,
                &provider.model,
                provider.api_version.clone(),
                key.unwrap_or_default(),
                provider.supports_tools,
                http,
            )),
            ProviderKind::Ollama => Box::new(OllamaConnector::new(
                &provider.base_url,
                &provider.model,
                http,
            )),
            ProviderKind::OpenAi | ProviderKind::OpenAiCompatible => {
                Box::new(OpenAiConnector::new(
                    &provider.id,
                    &provider.base_url,
                    &provider.model,
                    key,
                    provider.supports_tools,
                    http,
                ))
            }
        })
    }

    /// **实时档想调工具时，换成哪一档来干。** `None` = 不换。
    ///
    /// # 为什么（2026-09-26 用户定的，推翻了 2026-08-24「两档只分用户等不等」的一部分）
    ///
    /// 实时档配的是小模型（gemma-4-e4b），为的是语音秒回（关思考首字 ~0.09s）。
    /// 但小模型调工具会犯傻：同一个工具反复调、不调工具就宣布完成、在循环里卡死 ——
    /// 这正是当初做后台档的原因。所以：**闲聊留在实时档，一要调工具就交给后台档**。
    ///
    /// 只在实时档、且两档解析出来**不是同一个后端 + 同一个模型**时才换 ——
    /// 两档配成同一个模型时换了也是它自己，白白多一次首字。
    pub async fn tool_escalation(&self, tier: Tier) -> Option<Tier> {
        if tier != Tier::Realtime {
            return None;
        }
        let realtime = self.resolve(Tier::Realtime, true).await.ok()?;
        let background = self.resolve(Tier::Background, true).await.ok()?;
        (realtime.id != background.id || realtime.model != background.model).then_some(Tier::Background)
    }

    /// 解析某个分层实际要用的 provider。
    ///
    /// 如果请求带了工具、而该分层的后端不支持流式工具调用，就回退到
    /// 另一个支持的后端（优先云端）。这正是 ARCHITECTURE.md 坑点 8 说的
    /// "复杂/高风险工具调用回退到云端模型"。
    /// 选出本轮要用的后端。`needs_tools=true` 时会跳过不支持工具调用的后端。
    ///
    /// 公开是因为 command 层要在建流之前就把实际后端告诉前端 ——
    /// 带工具的对话由 agent 循环自己建流（要跑多轮），command 拿不到那个结果。
    pub async fn resolve(&self, tier: Tier, needs_tools: bool) -> Result<ProviderConfig> {
        let config = self.config.read().await;
        let primary = config.provider(config.provider_id_for(tier))?;

        // 链路自己的模型覆盖后端的默认值。
        //
        // **模型属于链路，不属于后端** —— 否则两条链路指向同一个后端时必然共用
        // 一个模型，而"实时用 8B、后台用 35B、都在本机 LM Studio 上"是最自然的
        // 配法。覆盖放在这里做是因为这是**唯一**决定"本轮用哪个模型"的地方，
        // 下游一律读返回值的 `model`，不用改。
        let with_model = |p: &ProviderConfig, override_model: Option<&String>| {
            let mut out = p.clone();
            if let Some(m) = override_model.filter(|m| !m.trim().is_empty()) {
                out.model = m.clone();
            }
            out
        };
        let tier_model = match tier {
            Tier::Realtime => config.realtime_model.as_ref(),
            Tier::Background => config.background_model.as_ref(),
        };

        if !needs_tools || primary.supports_tools {
            return Ok(with_model(primary, tier_model));
        }

        // 回退顺序：用户指定的优先，其次云端（工具调用最可靠），
        // 最后任何支持工具的本地后端
        let explicit = config
            .tool_fallback_provider
            .as_deref()
            .and_then(|id| config.providers.iter().find(|p| p.id == id))
            .filter(|p| p.supports_tools);
        let cloud_with_key = || {
            config.providers.iter().find(|p| {
                p.supports_tools
                    && matches!(
                        p.kind,
                        ProviderKind::Anthropic | ProviderKind::OpenAi | ProviderKind::AzureOpenAi
                    )
                    && secrets::has_secret(&p.id)
            })
        };
        let fallback = explicit
            .or_else(cloud_with_key)
            .or_else(|| config.providers.iter().find(|p| p.supports_tools));

        match fallback {
            Some(p) => {
                tracing::info!(
                    from = %primary.id,
                    to = %p.id,
                    "当前后端不支持流式工具调用，本轮回退"
                );
                // 回退后用备用链路自己的模型，不是被回退掉的那条链路的
                Ok(with_model(p, config.tool_fallback_model.as_ref()))
            }
            // 没有可回退的后端时不静默丢工具 —— 那会让模型看起来"忘了用工具"，
            // 排查成本极高。直接报错，让用户知道要去配一个。
            None => Err(Error::config(format!(
                "{} 不支持流式工具调用，且没有可回退的后端",
                primary.label
            ))),
        }
    }

    /// 发起一次流式对话。
    ///
    /// 返回的流在 `cancel(stream_id)` 被调用后会尽快结束，
    /// 并以 `Done { reason: Cancelled }` 收尾 —— 上层据此清空 TTS 播放队列。
    pub async fn chat_stream(
        &self,
        stream_id: String,
        tier: Tier,
        req: ChatRequest,
    ) -> Result<(ProviderConfig, ChatStream)> {
        let provider = self.resolve(tier, !req.tools.is_empty()).await?;
        let connector = self.connector(&provider)?;

        // **多条 system 消息必须先合并。** 很多模型的 chat 模板只允许一条、
        // 且必须是第一条，多一条就直接 raise_exception：
        //   Jinja Exception: System message must be at the beginning.
        // 请求会拿到 500，用户看到的是一句莫名的「上游服务返回 200」。
        //
        // 会攒出多条的地方不止一处（项目指令 + 前端人设 + 历史折叠提示），
        // 所以合并放在这个唯一收口，而不是让每个调用方自己小心。
        let mut req = req;
        merge_system_messages(&mut req.messages);
        // 出错时要报的规模。**在这里量**：`req` 下面会被 move 进 connector。
        let msg_count_chars = (
            req.messages.len(),
            req.messages.iter().map(|m| m.content.chars().count()).sum::<usize>(),
        );

        // 登记「用户正在等」。Realtime 是语音管线和主会话所在的档
        // —— 子智能体（Background）会在轮次之间给它让路。
        // 用 RAII：流可能在任何一步提前结束（取消、报错、被丢弃不再 poll），
        // 手动配对一定会漏，而漏一次的后果是子智能体从此永远让路。
        let activity = (tier == Tier::Realtime)
            .then(crate::agent_runtime::user_activity::begin_realtime);

        let token = CancellationToken::new();
        self.inflight
            .write()
            .await
            .insert(stream_id.clone(), token.clone());

        // 建连本身也要可取消：本地模型冷加载时这一步可能卡好几秒，
        // 用户这时候开口打断，不应该等建连结束才响应。
        let upstream = tokio::select! {
            biased;
            _ = token.cancelled() => {
                self.finish(&stream_id).await;
                return Ok((provider, cancelled_stream().boxed()));
            }
            result = connector.chat_stream(req) => result,
        };

        let upstream = match upstream {
            Ok(s) => s,
            Err(err) => {
                // **上游报错必须落日志。**
                //
                // 原来这里是直接 `return Err(err)` —— 错误只到界面上闪一下，
                // 关掉窗口就没了。用户问「看看日志怎么回事」，而日志里
                // 一条都没有，只能靠猜是上下文爆了还是模型崩了。
                //
                // 带上 provider / model / 这次发了多少条消息和多少字符 ——
                // 撞上下文窗口是这条路最常见的失败，而那三个数就够判断。
                let chars: usize = msg_count_chars.1;
                tracing::error!(
                    %err,
                    provider = %provider.id,
                    model = %provider.model,
                    messages = msg_count_chars.0,
                    chars,
                    "上游请求失败"
                );
                self.finish(&stream_id).await;
                return Err(err);
            }
        };

        // 记账：流结束时 `Done` 里带着这次的 usage。算在谁头上看调用方所在的范围（见 `token_usage`）
        let scope = crate::token_usage::current();
        let (pid, plabel, model) = (provider.id.clone(), provider.label.clone(), provider.model.clone());
        let upstream = upstream
            .inspect(move |item| {
                if let Ok(ChatChunk::Done { usage: Some(u), .. }) = item {
                    crate::token_usage::record(&pid, &plabel, &model, &scope, u);
                }
            })
            .boxed();
        let stream = self.wrap_cancellable(stream_id, token, upstream, activity);
        Ok((provider, stream))
    }

    /// 给上游流套一层取消检查。
    fn wrap_cancellable(
        &self,
        stream_id: String,
        token: CancellationToken,
        upstream: ChatStream,
        activity: Option<crate::agent_runtime::user_activity::RealtimeGuard>,
    ) -> ChatStream {
        let inflight = Arc::clone(&self.inflight);

        struct State {
            upstream: ChatStream,
            token: CancellationToken,
            inflight: Arc<RwLock<HashMap<String, CancellationToken>>>,
            stream_id: String,
            closed: bool,
            /// 只是拿着不用 —— 它一 drop，「用户正在等」的登记就撤销了。
            _activity: Option<crate::agent_runtime::user_activity::RealtimeGuard>,
        }

        let state = State {
            upstream,
            token,
            inflight,
            stream_id,
            closed: false,
            _activity: activity,
        };

        futures_util::stream::unfold(state, |mut st| async move {
            if st.closed {
                return None;
            }

            let next = tokio::select! {
                // biased：优先检查取消，避免上游一直有数据时永远轮不到取消分支
                biased;
                _ = st.token.cancelled() => None,
                item = st.upstream.next() => Some(item),
            };

            match next {
                // 被取消
                None => {
                    st.closed = true;
                    st.inflight.write().await.remove(&st.stream_id);
                    Some((
                        Ok(ChatChunk::Done {
                            reason: FinishReason::Cancelled,
                            usage: None,
                        }),
                        st,
                    ))
                }
                // 上游正常结束
                Some(None) => {
                    st.closed = true;
                    st.inflight.write().await.remove(&st.stream_id);
                    None
                }
                Some(Some(item)) => {
                    // Done / Err 都意味着这条流到此为止，顺手清理登记
                    let terminal = matches!(item, Ok(ChatChunk::Done { .. }) | Err(_));
                    if terminal {
                        st.closed = true;
                        st.inflight.write().await.remove(&st.stream_id);
                    }
                    Some((item, st))
                }
            }
        })
        .boxed()
    }

    /// 取消指定的流。用于 barge-in，以及用户手动点"停止"。
    ///
    /// 返回是否真的取消了某条流（false = 该流已经结束）。
    pub async fn cancel(&self, stream_id: &str) -> bool {
        let token = self.inflight.write().await.remove(stream_id);
        match token {
            Some(t) => {
                t.cancel();
                true
            }
            None => false,
        }
    }

    /// 取消所有在途流。切换后端、退出应用时用。
    pub async fn cancel_all(&self) {
        let mut inflight = self.inflight.write().await;
        for (_, token) in inflight.drain() {
            token.cancel();
        }
    }

    async fn finish(&self, stream_id: &str) {
        self.inflight.write().await.remove(stream_id);
    }

    /// 探活指定后端，返回其可用模型列表。
    /// 这个后端有哪些模型能吃图。
    ///
    /// `None` = **查不到**（这个后端没有那个接口），不是"没有视觉模型"。
    /// 两者必须分开：前端拿到 None 时应该什么都不说，而不是谎报"不支持视觉"。
    ///
    /// 目前只有 OpenAI 兼容那条路能查（走 LM Studio 的 `/api/v0/models` 读
    /// `type: "vlm"`）。云端后端的视觉能力是按模型型号定的，硬编码一张表迟早
    /// 过期，不如老实说不知道。
    pub async fn vision_models(&self, provider_id: &str) -> Result<Option<Vec<String>>> {
        let provider = {
            let config = self.config.read().await;
            config.provider(provider_id)?.clone()
        };
        if !matches!(
            provider.kind,
            crate::config::ProviderKind::OpenAiCompatible
        ) {
            return Ok(None);
        }
        let key = secrets::get_secret(&provider.id)?.filter(|k| !k.is_empty());
        let connector = OpenAiConnector::new(
            &provider.id,
            &provider.base_url,
            &provider.model,
            key,
            provider.supports_tools,
            self.http.clone(),
        );
        Ok(connector.vision_models().await)
    }

    /// 这个后端当前加载的模型有多大上下文窗口（token）。
    ///
    /// `None` = **查不到**，不是"窗口是 0"。调用方据此回落到默认预算。
    /// 只有 OpenAI 兼容那条路能查（LM Studio 的 `/api/v0/models`）——
    /// Ollama / MLX / 云端都没有这个字段，硬猜不如老实说不知道。
    ///
    /// 见 [`OpenAiConnector::context_window`]：读的是 `loaded_context_length`
    /// （这次实际加载的），不是 `max_context_length`（模型能力上限）。
    pub async fn context_window(&self, provider_id: &str) -> Result<Option<u32>> {
        let provider = {
            let config = self.config.read().await;
            config.provider(provider_id)?.clone()
        };
        if !matches!(
            provider.kind,
            crate::config::ProviderKind::OpenAiCompatible
        ) {
            return Ok(None);
        }
        let key = secrets::get_secret(&provider.id)?.filter(|k| !k.is_empty());
        let connector = OpenAiConnector::new(
            &provider.id,
            &provider.base_url,
            &provider.model,
            key,
            provider.supports_tools,
            self.http.clone(),
        );
        Ok(connector.context_window(&provider.model).await)
    }

    pub async fn health_check(&self, provider_id: &str) -> Result<Vec<String>> {
        let provider = {
            let config = self.config.read().await;
            config.provider(provider_id)?.clone()
        };
        self.connector(&provider)?.health_check().await
    }
}

/// 建连阶段就被取消时返回的流：只有一个 Cancelled 收尾帧。
fn cancelled_stream() -> impl futures_util::Stream<Item = Result<ChatChunk>> + Send {
    futures_util::stream::once(async {
        Ok(ChatChunk::Done {
            reason: FinishReason::Cancelled,
            usage: None,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **链路的模型必须能覆盖后端的默认值。**
    ///
    /// 少了这一条，两条链路指向同一个后端时只能共用一个模型 —— 而
    /// "实时用 8B、后台用 35B、都在本机 LM Studio 上"是最自然的配法，
    /// 本机资源也够。
    ///
    /// 而且这种错**不报任何错**：请求照样成功，只是悄悄用了另一个模型，
    /// 表现成"后台推理怎么这么笨"或者"实时对话怎么这么慢"。
    #[tokio::test]
    async fn route_model_overrides_provider_default() {
        // 两条链路刻意指向同一个本地后端，各自要不同的模型
        let config = AppConfig {
            realtime_provider: "lmstudio".into(),
            background_provider: "lmstudio".into(),
            realtime_model: Some("small-8b".into()),
            background_model: Some("big-35b".into()),
            ..AppConfig::default()
        };
        let gateway = Gateway::new(config).expect("网关初始化失败");

        let realtime = gateway.resolve(Tier::Realtime, false).await.unwrap();
        let background = gateway.resolve(Tier::Background, false).await.unwrap();

        assert_eq!(realtime.id, background.id, "本例刻意让两条链路共用后端");
        assert_eq!(realtime.model, "small-8b");
        assert_eq!(background.model, "big-35b");
    }

    /// 实时档小模型、后台档大模型：想调工具就换到后台
    #[tokio::test]
    async fn 实时档想调工具时换到后台档() {
        let config = AppConfig {
            realtime_provider: "lmstudio".into(),
            background_provider: "lmstudio".into(),
            realtime_model: Some("gemma-4-e4b".into()),
            background_model: Some("ornith-35b".into()),
            ..AppConfig::default()
        };
        let gateway = Gateway::new(config).expect("网关初始化失败");
        assert_eq!(gateway.tool_escalation(Tier::Realtime).await, Some(Tier::Background));
        // 已经在后台档就不用再换
        assert_eq!(gateway.tool_escalation(Tier::Background).await, None);
    }

    /// 两档配成同一个模型：换了也是它自己，不换
    #[tokio::test]
    async fn 两档同一个模型时不换() {
        let config = AppConfig {
            realtime_provider: "lmstudio".into(),
            background_provider: "lmstudio".into(),
            realtime_model: Some("ornith-35b".into()),
            background_model: Some("ornith-35b".into()),
            ..AppConfig::default()
        };
        let gateway = Gateway::new(config).expect("网关初始化失败");
        assert_eq!(gateway.tool_escalation(Tier::Realtime).await, None);
    }

    /// 没设链路模型时沿用后端自己的 —— 老配置的行为不能变
    #[tokio::test]
    async fn unset_route_model_falls_back_to_provider_default() {
        let config = AppConfig {
            realtime_provider: "lmstudio".into(),
            realtime_model: None,
            ..AppConfig::default()
        };
        let expected = config.provider("lmstudio").unwrap().model.clone();
        let gateway = Gateway::new(config).expect("网关初始化失败");

        let resolved = gateway.resolve(Tier::Realtime, false).await.unwrap();
        assert_eq!(resolved.model, expected);
    }

    /// 空字符串等于没设 —— 界面把下拉清成"跟随后端"时传的就是空串
    #[tokio::test]
    async fn blank_route_model_is_treated_as_unset() {
        let config = AppConfig {
            realtime_provider: "lmstudio".into(),
            realtime_model: Some("   ".into()),
            ..AppConfig::default()
        };
        let expected = config.provider("lmstudio").unwrap().model.clone();
        let gateway = Gateway::new(config).expect("网关初始化失败");

        let resolved = gateway.resolve(Tier::Realtime, false).await.unwrap();
        assert_eq!(resolved.model, expected);
    }
}

#[cfg(test)]
mod system_merge_tests {
    use super::*;

    fn roles(ms: &[Message]) -> Vec<&'static str> {
        ms.iter()
            .map(|m| match m.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "tool",
            })
            .collect()
    }

    #[test]
    fn 多条system合并成开头一条() {
        // 这是线上真的撞到的形状：项目指令（后端插的）+ 通用人设（前端的）。
        // 模型模板直接 raise_exception: "System message must be at the beginning."
        let mut ms = vec![
            Message::system("项目指令"),
            Message::system("通用人设"),
            Message::user("你好"),
        ];
        merge_system_messages(&mut ms);
        assert_eq!(roles(&ms), vec!["system", "user"]);
        assert_eq!(ms[0].content, "项目指令\n\n通用人设", "顺序要保持");
    }

    #[test]
    fn 历史折叠提示那条也会被合并() {
        // 前端 buildBoundedMessages 在历史超预算时会再插一条 system。
        // 长对话早就有这个雷，只是没人撞到过。
        let mut ms = vec![
            Message::system("人设"),
            Message::system("[提示：更早的对话历史已折叠]"),
            Message::user("继续"),
            Message::assistant("好"),
            Message::user("再来"),
        ];
        merge_system_messages(&mut ms);
        assert_eq!(roles(&ms), vec!["system", "user", "assistant", "user"]);
        assert!(ms[0].content.contains("人设"));
        assert!(ms[0].content.contains("已折叠"));
    }

    #[test]
    fn 只有一条时不动内容() {
        let mut ms = vec![Message::system("人设"), Message::user("你好")];
        merge_system_messages(&mut ms);
        assert_eq!(roles(&ms), vec!["system", "user"]);
        assert_eq!(ms[0].content, "人设");
    }

    #[test]
    fn 唯一的一条不在开头时挪到开头() {
        let mut ms = vec![Message::user("你好"), Message::system("人设")];
        merge_system_messages(&mut ms);
        assert_eq!(roles(&ms), vec!["system", "user"]);
    }

    #[test]
    fn 一条都没有时不要凭空造一条() {
        // 造一条空 system 同样会被某些模板拒绝
        let mut ms = vec![Message::user("你好")];
        merge_system_messages(&mut ms);
        assert_eq!(roles(&ms), vec!["user"]);
    }

    #[test]
    fn 空白的system不参与拼接也不留空行() {
        let mut ms = vec![
            Message::system("  "),
            Message::system("人设"),
            Message::user("你好"),
        ];
        merge_system_messages(&mut ms);
        assert_eq!(roles(&ms), vec!["system", "user"]);
        assert_eq!(ms[0].content, "人设", "不能拼出前导空行");
    }

    #[test]
    fn 全是空白的system直接去掉() {
        let mut ms = vec![Message::system("   "), Message::user("你好")];
        merge_system_messages(&mut ms);
        assert_eq!(roles(&ms), vec!["user"]);
    }

    #[test]
    fn 工具往返里的消息次序不能被打乱() {
        // agent 的多轮工具循环会攒出 assistant/tool 交错的长序列，
        // 合并 system 时把它们的相对次序弄乱会让请求变成非法结构。
        let mut ms = vec![
            Message::system("a"),
            Message::user("查一下"),
            Message::assistant("我调工具"),
            Message {
                role: Role::Tool,
                content: "结果".into(),
                images: vec![],
                tool_calls: vec![],
                tool_call_id: Some("c1".into()),
            },
            Message::system("b"),
            Message::assistant("答案"),
        ];
        merge_system_messages(&mut ms);
        assert_eq!(
            roles(&ms),
            vec!["system", "user", "assistant", "tool", "assistant"]
        );
        assert_eq!(ms[0].content, "a\n\nb");
        assert_eq!(
            ms[3].tool_call_id.as_deref(),
            Some("c1"),
            "tool_call_id 不能丢"
        );
    }

    /// **上游报错必须落日志。**
    ///
    /// 原来是直接 `return Err(err)` —— 错误只在界面上闪一下，关掉窗口就没了。
    /// 用户问「看看日志怎么回事」而日志里一条都没有，只能靠猜是上下文爆了
    /// 还是模型崩了。
    ///
    /// 这条钉的是**规模三件套一起报**：provider / model / 这次多少条消息
    /// 多少字符。撞上下文窗口是这条路最常见的失败，而那三个数就够判断。
    #[test]
    fn 上游失败那一支会落日志并带上规模() {
        // 切掉测试模块再找，否则 `find` 会撞上这条测试自己的字面量
        let src = include_str!("gateway.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("源码是空的");
        let i = src
            .find("let upstream = match upstream {")
            .expect("找不到上游错误分支");
        let arm = &src[i..i + 1200];
        assert!(arm.contains("tracing::error!"), "上游失败没落日志");
        for field in ["provider =", "model =", "messages =", "chars"] {
            assert!(arm.contains(field), "日志里缺 {field}");
        }
        // 规模要在 req 被 move 之前量
        assert!(
            src.find("let msg_count_chars").expect("没量规模") < i,
            "规模在 req 被 move 之后才量，拿不到"
        );
    }
}
