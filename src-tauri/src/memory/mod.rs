//! 本地记忆系统（ARCHITECTURE.md 3.4 的双轨制）。
//!
//! - [`store`]：SQLite —— 会话、逐条消息、情感/性格状态。需要精确查询的都在这。
//! - [`vector`]：LanceDB —— 长期记忆「事实」的向量存储与语义召回。
//! - [`embed`]：文本向量化，复用已配好的本地后端（LM Studio / Ollama）。
//! - [`extract`]：从对话里提取事实，用后台分层的模型异步跑，不阻塞对话。
//!
//! [`Memory`] 是给上层的门面：只暴露「记住这一轮」和「召回相关记忆」两个动作，
//! 上层不需要知道背后有几个库。

pub mod embed;
pub mod extract;
pub mod knowledge;
pub mod reconcile;
pub mod store;
pub mod vector;

pub mod summary;

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::llm::gateway::Gateway;
use crate::llm::Role;
use embed::{Embedder, HttpEmbedder};
use knowledge::{KnowledgeHit, KnowledgeStore};
use store::Store;
use vector::{Fact, Recalled, ScopeFilter, VectorStore};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    /// 总开关。关掉则完全不提取、不召回（也不会去连嵌入后端）。
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 用哪个后端做嵌入（provider id）
    #[serde(default = "default_embedding_provider")]
    pub embedding_provider: String,
    /// 嵌入模型名
    #[serde(default = "default_embedding_model")]
    pub embedding_model: String,
    /// 每次召回最多注入几条事实。
    /// 注入太多会挤占上下文、反而稀释当前话题。
    #[serde(default = "default_recall_limit")]
    pub recall_limit: usize,
    /// 相似度下限。低于它的召回结果宁可不要 —— 无关记忆比没有记忆更糟，
    /// 它会把模型往错误方向带。
    #[serde(default = "default_min_similarity")]
    pub min_similarity: f32,
}

fn default_true() -> bool {
    true
}
fn default_embedding_provider() -> String {
    // Ollama 的 /api/embed 支持批量
    "ollama".to_string()
}

/// 默认嵌入模型。
///
/// **必须是多语言/中文可用的模型。** 实测一组中文事实的 Top-1 召回命中率：
///
/// | 模型 | 维度 | 命中 |
/// | :--- | ---: | ---: |
/// | `bge-m3` | 1024 | **6/6** |
/// | `nomic-embed-text-v2-moe`（LM Studio） | 768 | 6/6 |
/// | `mxbai-embed-large` | 1024 | 2/6 |
/// | `nomic-embed-text` | 768 | 2/6 |
///
/// 后两个是英文向模型，对中文给不出有意义的排序 —— 典型症状是不管问什么，
/// 排第一的永远是最短的那条事实。
fn default_embedding_model() -> String {
    "bge-m3".to_string()
}
fn default_recall_limit() -> usize {
    5
}

/// 默认相似度下限。
///
/// **这个值与嵌入模型强相关，换模型必须重新测。** bge-m3 上实测：相关答案落在
/// 0.699–0.721，无关答案最高 0.603，所以 0.65 能干净地把两者分开。
///
/// 若换成 `nomic-embed-text-v2-moe`，它的分数整体低得多（相关答案只有
/// 0.199–0.446），沿用 0.65 会把**所有**记忆都过滤掉 —— 表现为「记忆好像没生效」
/// 且完全没有报错，极难排查。
/// 判定"这条事实已经记过了"的相似度门槛。
///
/// ## 为什么必须去重
///
/// 同一件事会被反复提取：用户第二次提到猫叫小黑，模型照样会抽出一条
/// 「养了一只叫小黑的猫」。堆积的后果不只是占地方 —— **召回会被同一条事实
/// 刷满**，挤掉本该被想起的其它内容。
///
/// 0.93 比召回阈值高很多：召回要的是"相关"，去重要的是"几乎是同一句话"。
/// 定低了会把「住在北京」和「以前住在上海」判成重复，那是丢信息。
const DEDUPE_SIMILARITY: f32 = 0.93;

fn default_min_similarity() -> f32 {
    0.65
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            embedding_provider: default_embedding_provider(),
            embedding_model: default_embedding_model(),
            recall_limit: default_recall_limit(),
            min_similarity: default_min_similarity(),
        }
    }
}

/// 时近性带来的最大加分。
///
/// **刻意取得很小。** 它的作用是**打破近似平局**，不是覆盖相关性 ——
/// 一条真正更相关的旧事实不该被一条勉强相关的新事实挤掉。
///
/// 本机在 bge-m3 上实测过的分布：相关答案落在 0.699~0.721，无关答案最高 0.603。
/// 0.03 的加分够把 0.71 和 0.70 分出先后，但翻不动 0.79 和 0.68 那种差距。
const RECENCY_BONUS: f32 = 0.03;

/// 时近性的衰减时间常数（天）。30 天前的事实还剩约 37% 的加分。
const RECENCY_TAU_DAYS: f32 = 30.0;

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// 按「相似度 + 时近性加分」重排召回结果。
///
/// ## 为什么需要
///
/// 原先只按相似度排，于是一条一年前的过时事实只要向量更近就赢过昨天的新事实。
/// 对陪伴型记忆无所谓（"养了只猫"几年不变），对 coding 是错的 ——
/// 关于代码的事实每次 commit 都可能失效。
///
/// ## 为什么不做成乘法权重
///
/// `similarity × recency` 会让一条半年前的高相关事实被腰斩，而它可能正是答案。
/// 加一个**有上限的小加分**才符合「时近性只是次要证据」这个判断。
///
/// 访问频次也是个有用的信号，但要记读取次数、也就要改 schema，
/// 这里先不做 —— 留在注释里而不是留在代码里。
fn rerank_by_recency(hits: &mut [Recalled], now: i64) {
    for h in hits.iter_mut() {
        let age_days = ((now - h.fact.created_at).max(0) as f32) / 86_400.0;
        h.score = h.similarity + RECENCY_BONUS * (-age_days / RECENCY_TAU_DAYS).exp();
    }
    hits.sort_by(|a, b| b.score.total_cmp(&a.score));
}

/// 记忆系统门面
pub struct Memory {
    store: Store,
    vectors: VectorStore,
    /// 项目知识库索引。和 `vectors` 同一个 LanceDB 目录、不同表 ——
    /// 「重建某个项目的索引」必须删得干净，而那个动作绝不能碰到事实。
    knowledge: KnowledgeStore,
    embedder: Arc<dyn Embedder>,
    gateway: Arc<Gateway>,
    config: MemoryConfig,
}

impl Memory {
    /// 打开记忆系统。
    ///
    /// 会先探一次嵌入模型的维度 —— 建向量表必须先知道维度，而这个值只有问过模型
    /// 才知道。也就意味着**嵌入后端不可用时记忆系统起不来**，这是有意的：
    /// 免得先建了个维度错误的库，之后所有召回都是噪声。
    pub async fn open(
        data_dir: &std::path::Path,
        config: MemoryConfig,
        gateway: Arc<Gateway>,
        http: reqwest::Client,
    ) -> Result<Self> {
        let provider = {
            let app_config = gateway.config();
            let app_config = app_config.read().await;
            app_config.provider(&config.embedding_provider)?.clone()
        };

        let embedder: Arc<dyn Embedder> = Arc::new(HttpEmbedder::new(
            provider,
            config.embedding_model.clone(),
            http,
        ));

        let dimensions = embedder.dimensions().await.map_err(|err| {
            Error::config(format!(
                "无法从嵌入后端「{}／{}」获取向量维度：{err}。\
                 记忆系统需要它才能建库；可以在设置里换一个嵌入后端，或先关掉记忆。",
                config.embedding_provider, config.embedding_model
            ))
        })?;

        tracing::info!(
            provider = %config.embedding_provider,
            model = %config.embedding_model,
            dimensions,
            "记忆系统嵌入后端就绪"
        );

        let store = Store::open(&data_dir.join("memory.db"))?;
        let vectors = VectorStore::open(&data_dir.join("vectors"), dimensions).await?;
        let knowledge = KnowledgeStore::open(&data_dir.join("vectors"), dimensions).await?;

        let me = Self {
            store,
            vectors,
            knowledge,
            embedder,
            gateway,
            config,
        };
        me.migrate_path_scopes_to_project_ids().await;
        Ok(me)
    }

    /// 把老格式的 scope（规范化绝对路径）改写成项目 id。
    ///
    /// ## 为什么要迁移
    ///
    /// scope 原先就是路径，代价是**项目目录改个名，那个项目下累积的事实和
    /// 知识块就再也召不回来** —— 它们还在库里，只是 scope 对不上，没有提示。
    ///
    /// ## 三条刻意的设计
    ///
    /// **1. 先用一次 count 守住。** `scope LIKE '/%'` 为 0 就整个跳过。
    /// 不查就迁的话，每次启动都会给 LanceDB 加一个永远不会被清理的数据集版本。
    ///
    /// **2. 原地改列，不读写向量。** 读出来再写回是这类迁移最容易丢数据的
    /// 一步，而 `list()` 压根不返回向量。
    ///
    /// **3. 失败只记日志。** 迁移失败的后果是「那个项目的记忆暂时召不回来」，
    /// 而让它变成「记忆系统起不来」严重得多 —— 那会连带打掉召回、提取、
    /// 知识库检索全部。
    async fn migrate_path_scopes_to_project_ids(&self) {
        let tables: [(&str, &vector::Table); 2] = [
            ("facts", self.vectors.table()),
            ("knowledge", self.knowledge.table()),
        ];
        // 便宜守卫：两张表都没有老格式的行就直接回
        let mut needs = false;
        for (name, t) in tables {
            match vector::count_path_scoped(t).await {
                Ok(0) => {}
                Ok(n) => {
                    tracing::info!(table = name, rows = n, "发现老格式 scope，开始迁移");
                    needs = true;
                }
                Err(err) => {
                    tracing::warn!(%err, table = name, "统计老格式 scope 失败，跳过迁移");
                    return;
                }
            }
        }
        if !needs {
            return;
        }

        let projects = match self.store.get_projects().await {
            Ok(p) => p,
            Err(err) => {
                tracing::warn!(%err, "读项目表失败，跳过 scope 迁移");
                return;
            }
        };
        for pr in projects {
            let Some(path) = pr.workspace_path.as_deref() else {
                continue;
            };
            let from = knowledge::scope_of(path);
            if from.is_empty() {
                continue;
            }
            let to = format!("p:{}", pr.id);
            for (name, t) in tables {
                match vector::rescope(t, &from, &to).await {
                    Ok(0) => {}
                    Ok(n) => tracing::info!(
                        table = name, rows = n, project = %pr.name,
                        "scope 已从路径改写成项目 id"
                    ),
                    Err(err) => tracing::warn!(%err, table = name, "改写 scope 失败"),
                }
            }
        }
        // 剩下的路径 scope 属于没注册成项目的工作区 —— 它们保持路径形态，
        // 因为那种情况下没有稳定 id 可用。`scope_for` 的退路正好对上。
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn config(&self) -> &MemoryConfig {
        &self.config
    }

    /// 召回与 `query` 语义相关的记忆。
    ///
    /// 失败时返回空而不是报错：召回不到只是「这轮没有额外背景」，
    /// 不该让整轮对话失败。
    /// `scope` 决定看得见哪些事实，见 [`ScopeFilter`]。**这个参数不能有默认值** ——
    /// 用错的一侧都会出事：陪伴对话里冒出别的项目的工程约定，
    /// 或者 coding 时被塞进「我們有西瓜汁」。
    pub async fn recall(&self, query: &str, scope: &ScopeFilter) -> Vec<Recalled> {
        if !self.config.enabled || query.trim().is_empty() {
            return Vec::new();
        }

        let vector = match self.embedder.embed(&[query.to_string()]).await {
            Ok(mut v) if !v.is_empty() => v.remove(0),
            Ok(_) => return Vec::new(),
            Err(err) => {
                tracing::warn!(%err, "召回时向量化失败，本轮不注入记忆");
                return Vec::new();
            }
        };

        match self
            .vectors
            .search(
                &vector,
                self.config.recall_limit,
                self.config.min_similarity,
                scope,
            )
            .await
        {
            Ok(mut hits) => {
                if !hits.is_empty() {
                    rerank_by_recency(&mut hits, now_secs());
                    tracing::debug!(count = hits.len(), "召回到相关记忆");
                }
                hits
            }
            Err(err) => {
                tracing::warn!(%err, "记忆检索失败，本轮不注入记忆");
                Vec::new()
            }
        }
    }

    /// 把召回结果拼成可以插进 System Prompt 的一段文本。
    ///
    /// 明确标注这是「以往了解到的信息」并要求不要生硬复述 —— 否则模型很容易把
    /// 召回内容当成用户刚说的话，或者每轮都机械地重复一遍。
    pub fn format_recalled(hits: &[Recalled]) -> Option<String> {
        Self::format_recalled_for(hits, false)
    }

    /// 按用途拼召回块。`in_project` 决定措辞，而这不是文字润色。
    ///
    /// ## 为什么 coding 场景必须换一套话
    ///
    /// 陪伴场景的原话是「以下是你以往了解到的关于用户的信息，可在需要时自然地
    /// 运用」。用在 coding 上很危险：它让模型把**一条召回的旧记录**和**一份
    /// 刚读过的文件**当成同等权重。
    ///
    /// 而这两者的可靠性差着一个量级 —— 关于人的事实几年不变，关于代码的事实
    /// **每次 commit 都可能失效**。所以项目里的措辞必须明说两件事：
    /// 可能已过时、与代码冲突时以代码为准。
    pub fn format_recalled_for(hits: &[Recalled], in_project: bool) -> Option<String> {
        if hits.is_empty() {
            return None;
        }
        let mut out = String::from(if in_project {
            "以下是过去记录下来的项目约定与结论，**可能已经过时** —— \
             它们只是线索，和你实际读到的代码冲突时**以代码为准**，不要照着改：\n"
        } else {
            "以下是你以往了解到的关于用户的信息，可在需要时自然地运用，不要生硬复述：\n"
        });
        for hit in hits {
            out.push_str(&format!("- {}：{}\n", hit.fact.subject, hit.fact.fact));
        }
        Some(out)
    }

    /// 记录一轮对话，并在后台异步提取事实。
    ///
    /// 消息落库是同步等待的（很快，且要拿到 message_id）；事实提取 spawn 出去，
    /// 因为它要调一次模型，几百毫秒到数秒不等，绝不能卡住对话。
    /// `scope` 是这一轮属于哪个项目（空串 = 不在项目里）。提取出来的事实
    /// 会带上它，之后只在这个项目里被召回。
    pub async fn remember_turn(
        &self,
        session_id: &str,
        user_text: &str,
        assistant_text: &str,
        scope: &str,
    ) -> Result<()> {
        let user_message_id = self
            .store
            .append_message(session_id, Role::User, user_text)
            .await?;
        self.store
            .append_message(session_id, Role::Assistant, assistant_text)
            .await?;

        if !self.config.enabled {
            return Ok(());
        }
        // 助手没说出内容（被打断/出错）时不提取：半截对话提不出可靠事实
        if assistant_text.trim().is_empty() {
            return Ok(());
        }

        self.spawn_extraction(session_id, Some(user_message_id), user_text, assistant_text, scope);
        Ok(())
    }

    /// 后台提取 + 向量化 + 入库。任何一步失败都只记日志。
    fn spawn_extraction(
        &self,
        session_id: &str,
        user_message_id: Option<i64>,
        user_text: &str,
        assistant_text: &str,
        scope: &str,
    ) {
        let gateway = Arc::clone(&self.gateway);
        let embedder = Arc::clone(&self.embedder);
        let session_id = session_id.to_string();
        let user_text = user_text.to_string();
        let assistant_text = assistant_text.to_string();
        let scope = scope.to_string();
        // 查重要在「本项目可见的范围」里做，也就是全局 + 本项目。
        // 只在本项目里查的话，一条已经作为全局事实存在的信息会被
        // 每个项目各记一份。
        let dedupe_scope = if scope.is_empty() {
            ScopeFilter::GlobalOnly
        } else {
            ScopeFilter::Project(scope.clone())
        };
        // Store / VectorStore 的 Clone 是共享同一份底层句柄，不是新开连接
        let store = self.store.clone();
        let vectors = self.vectors.clone();

        tokio::spawn(async move {
            // 已提取过就跳过 —— 提取要花一次模型调用，不该重复付。
            // `None` 是 agent 路径：那条链路的历史由前端管，消息不落 SQLite，
            // 所以没有 message_id 可作为记账键（去重靠向量那一层）。
            if let Some(id) = user_message_id {
                if store.is_extracted(id).await.unwrap_or(false) {
                    return;
                }
            }

            // 在项目里就换一套白名单：陪伴型白名单（个人信息/偏好/长期目标）
            // 在 coding 会话里的提取率实测接近零，而那是正确行为 ——
            // 「这个项目用 node --test」不属于它列的任何一类。
            let facts = match extract::extract(
                &gateway,
                &user_text,
                &assistant_text,
                !scope.is_empty(),
            )
            .await
            {
                Ok(f) => f,
                Err(err) => {
                    tracing::warn!(%err, "事实提取失败");
                    return;
                }
            };

            if facts.is_empty() {
                // 标记成已处理，避免下次又花一次模型调用得出同样的空结果
                if let Some(id) = user_message_id {
                    let _ = store.mark_extracted(id, 0).await;
                }
                return;
            }

            let texts: Vec<String> = facts.iter().map(|f| f.fact.clone()).collect();
            let embedded = match embedder.embed(&texts).await {
                Ok(v) => v,
                Err(err) => {
                    tracing::warn!(%err, "事实向量化失败");
                    return;
                }
            };

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);

            // ── 写入时收敛 ──
            //
            // 三段处理，只有中间那段要花一次模型调用：
            //
            //   相似度 ≥ 0.93        确定重复，直接跳过（不问模型）
            //   0.80 ~ 0.93         问模型：重复 / 矛盾 / 细化 / 新的
            //   < 0.80              新的，直接入库
            //
            // 中间那段是原先漏掉的。0.93 挡得住措辞几乎相同的重复，挡不住
            // **同一件事的两种说法** —— 本机真实库里就有「姓名是刘松公子」和
            // 「我叫刘松，可以叫我主人/公子/亲爱的」两条，以及《凡人修仙传》
            // 两条。而 recall_limit = 5，这两组已经吃掉 4 个位置。
            //
            // 更要紧的是**矛盾**：只做「重复就跳过」的话，矛盾的两条会同时留在
            // 库里、同时被召回，而模型没有依据去选。那是「表意不准」的最大来源。
            let mut records: Vec<Fact> = Vec::new();
            let mut kept_vectors: Vec<Vec<f32>> = Vec::new();
            let mut skipped = 0usize;
            let mut superseded: Vec<String> = Vec::new();
            for (f, vector) in facts.iter().zip(embedded.iter()) {
                // 用向量而不是字符串比对，因为「养了只猫叫小黑」和「有一只猫，
                // 名字是小黑」是同一件事，但字面上不一样。
                let neighbors = match vectors
                    .search(
                        vector,
                        reconcile::MAX_NEIGHBORS,
                        reconcile::RECONCILE_SIMILARITY,
                        &dedupe_scope,
                    )
                    .await
                {
                    Ok(hits) => hits,
                    // 查邻居失败就当没邻居：宁可多记一条，也不要因为一次检索
                    // 失败把新事实丢掉
                    Err(err) => {
                        tracing::warn!(%err, "查近邻失败，按新事实处理");
                        Vec::new()
                    }
                };

                // 确定重复：不问模型
                if neighbors
                    .first()
                    .is_some_and(|h| h.similarity >= DEDUPE_SIMILARITY)
                {
                    tracing::debug!(new = %f.fact, "措辞几乎相同，跳过");
                    skipped += 1;
                    continue;
                }

                let texts: Vec<String> =
                    neighbors.iter().map(|h| h.fact.fact.clone()).collect();
                let verdict = if texts.is_empty() {
                    reconcile::Verdict::New
                } else {
                    match reconcile::judge(&gateway, &f.fact, &texts).await {
                        Ok(v) => v,
                        // 判定失败当新事实。让一次模型调用失败去删掉一条已有
                        // 事实是不可接受的
                        Err(err) => {
                            tracing::warn!(%err, "收敛判定失败，按新事实处理");
                            reconcile::Verdict::New
                        }
                    }
                };

                let (text_to_store, replace) = match verdict {
                    reconcile::Verdict::New => (f.fact.clone(), None),
                    reconcile::Verdict::Duplicate { index } => {
                        tracing::debug!(
                            new = %f.fact,
                            existing = %neighbors[index].fact.fact,
                            "同一件事，跳过"
                        );
                        skipped += 1;
                        continue;
                    }
                    reconcile::Verdict::Contradicts { index } => {
                        // **新的赢。** 关于代码的事实每次 commit 都可能失效，
                        // 而关于人的事实变了也该按新的来。
                        //
                        // 这个删除有风险：新提取如果是误读，会毁掉一条正确的
                        // 旧事实。所以两条原文都记进 info —— 出问题时能查。
                        tracing::info!(
                            new = %f.fact,
                            superseded = %neighbors[index].fact.fact,
                            "矛盾，旧事实被取代"
                        );
                        (f.fact.clone(), Some(neighbors[index].fact.id.clone()))
                    }
                    reconcile::Verdict::Refines { index, text } => {
                        tracing::info!(
                            merged = %text,
                            replaced = %neighbors[index].fact.fact,
                            "细化，合并成一条"
                        );
                        (text, Some(neighbors[index].fact.id.clone()))
                    }
                };

                if let Some(old_id) = replace {
                    superseded.push(old_id);
                }
                // Refines 改了文本 → 必须重新向量化，否则向量和文本对不上，
                // 之后这条永远召不回来（而且完全没有迹象）
                let vec_for_store = if text_to_store == f.fact {
                    vector.clone()
                } else {
                    match embedder.embed(std::slice::from_ref(&text_to_store)).await {
                        Ok(mut v) if !v.is_empty() => v.remove(0),
                        _ => {
                            tracing::warn!("合并文本向量化失败，按原文入库");
                            vector.clone()
                        }
                    }
                };
                records.push(Fact {
                    id: uuid::Uuid::new_v4().to_string(),
                    subject: f.subject.clone(),
                    fact: text_to_store,
                    session_id: session_id.clone(),
                    created_at: now,
                    scope: scope.clone(),
                });
                kept_vectors.push(vec_for_store);
            }

            // **先写新的再删旧的。** 顺序反了的话，写入失败会同时丢掉两条 ——
            // 旧的已经删了，新的没进去。
            if records.is_empty() {
                tracing::debug!(skipped, "提取到的事实全部已存在");
                // 仍要标记成已处理 —— 否则下次又花一次模型调用得出同样的结果
                if let Some(id) = user_message_id {
                    let _ = store.mark_extracted(id, 0).await;
                }
                return;
            }
            let embedded = kept_vectors;

            match vectors.add(&records, &embedded).await {
                Ok(()) => {
                    // 新的进去了，才删被取代的那些
                    for id in &superseded {
                        if let Err(err) = vectors.delete(id).await {
                            // 删不掉只是多留一条旧记录，比丢数据好得多
                            tracing::warn!(%err, %id, "被取代的事实删除失败");
                        }
                    }
                    tracing::info!(
                        count = records.len(),
                        skipped,
                        superseded = superseded.len(),
                        "已写入长期记忆"
                    );
                    for f in &records {
                        tracing::debug!(subject = %f.subject, fact = %f.fact, "记住");
                    }
                    if let Some(id) = user_message_id {
                        let _ = store.mark_extracted(id, records.len()).await;
                    }
                }
                Err(err) => tracing::warn!(%err, "写入长期记忆失败"),
            }
        });
    }

    /// 从 agent 的一轮里提取项目事实。**不往 SQLite 落消息。**
    ///
    /// ## 为什么和 `remember_turn` 分开
    ///
    /// agent 链路（`start_chat` → `agent::run`）的对话历史由前端的 store 管，
    /// 带自己的字符预算与折叠规则。再往 `messages` 表写一份就是**第二个真相源** ——
    /// 这个项目已经在「两处描述同一件事」上栽过好几次，而这一次的后果是
    /// 两边的历史会静默分叉（前端折叠了旧工具结果，SQLite 里还是全的）。
    ///
    /// 代价是没有 `message_id` 可作为「已提取过」的记账键，所以同一轮如果
    /// 被重放会重复付一次模型调用。去重靠向量那一层（`DEDUPE_SIMILARITY`），
    /// 结果不会重复入库。
    ///
    /// `scope` 为空时直接返回：不在项目里就不该产生项目事实，
    /// 而 agent 链路的全局事实由陪伴链路负责记。
    pub fn remember_project_turn(&self, user_text: &str, assistant_text: &str, scope: &str) {
        if !self.config.enabled || scope.is_empty() || assistant_text.trim().is_empty() {
            return;
        }
        // session_id 用一个可辨认的常量，和手动添加的 "manual" 一样便于溯源
        self.spawn_extraction("agent", None, user_text, assistant_text, scope);
    }

    /// 把工作区路径解析成 scope。
    ///
    /// ## 为什么不直接用路径
    ///
    /// 原先 scope 就是规范化的绝对路径。代价是**把项目目录改名或挪走，
    /// 那个项目下累积的事实和知识块就再也召不回来** —— 它们还在库里，
    /// 只是 scope 对不上，而且没有任何提示。
    ///
    /// 改成注册过的项目用它的 id（`projects` 表的主键），改名挪位置都无感。
    ///
    /// 没注册成项目的工作区仍然退回规范化路径 —— 那种情况下没有稳定 id 可用，
    /// 这是诚实的降级而不是隐藏问题。
    pub async fn scope_for(&self, workspace_path: &str) -> String {
        let path_scope = knowledge::scope_of(workspace_path);
        if path_scope.is_empty() {
            return String::new();
        }
        match self.store.get_projects().await {
            Ok(projects) => projects
                .iter()
                .find(|pr| {
                    pr.workspace_path
                        .as_deref()
                        .map(knowledge::scope_of)
                        .is_some_and(|s| s == path_scope)
                })
                .map(|pr| format!("p:{}", pr.id))
                .unwrap_or(path_scope),
            // 查不到项目表就退回路径。宁可 scope 不理想，也不要因为一次
            // 数据库读失败让这一轮完全没有 scope（那会让项目事实进全局池）
            Err(err) => {
                tracing::warn!(%err, "读项目表失败，scope 退回路径");
                path_scope
            }
        }
    }

    /// 重建某个项目 `.anima/knowledge/` 的向量索引，返回索引到的块数。
    ///
    /// **整体替换语义**：先删掉这个项目的全部旧块再写新的。用户改了文档、
    /// 删了文档、重命名了文档，重建之后索引都必须与磁盘一致 ——
    /// 只追加的话，删掉的文档会永远留在召回结果里，而这种错很难被发现：
    /// 模型引用了一份已经不存在的文档，看起来只像是它记错了。
    ///
    /// 目录不存在或没有可读文本时返回 0，并且**仍然清空旧索引**。
    pub async fn reindex_knowledge(&self, workspace_path: &str) -> Result<usize> {
        let scope = self.scope_for(workspace_path).await;
        if scope.is_empty() {
            return Err(Error::config("重建索引需要先指定项目工作区"));
        }
        // **目录来自路径，scope 来自项目 id** —— 这两件事分开之后就不能再
        // 拿 scope 当路径用了
        let dir = std::path::Path::new(&knowledge::scope_of(workspace_path))
            .join(".anima")
            .join("knowledge");
        let chunks = knowledge::collect_chunks(&dir, &scope);

        if chunks.is_empty() {
            // 空也要走一遍 replace，否则删掉文档之后旧块还在
            let n = self.knowledge.replace_scope(&scope, &[], &[]).await?;
            tracing::info!(scope = %scope, "知识库为空，已清空该项目索引");
            return Ok(n);
        }

        // **分批向量化。** 一次把几百块塞给嵌入后端，Ollama 侧会因为请求体
        // 过大或超时整批失败，而失败之后我们只知道"重建失败"、不知道是量的问题。
        const BATCH: usize = 32;
        let mut vectors: Vec<Vec<f32>> = Vec::with_capacity(chunks.len());
        for group in chunks.chunks(BATCH) {
            let texts: Vec<String> = group.iter().map(|c| c.text.clone()).collect();
            vectors.extend(self.embedder.embed(&texts).await?);
        }
        if vectors.len() != chunks.len() {
            return Err(Error::decode(format!(
                "嵌入后端返回 {} 个向量，但有 {} 块待索引",
                vectors.len(),
                chunks.len()
            )));
        }

        let n = self.knowledge.replace_scope(&scope, &chunks, &vectors).await?;
        tracing::info!(scope = %scope, chunks = n, "知识库索引已重建");
        Ok(n)
    }

    /// 在某个项目的知识库里语义检索。
    ///
    /// 复用 `min_similarity`（和事实召回同一个阈值）：无关内容比没有内容更糟，
    /// 它会把模型往错误方向带，而这里的后果更重 —— agent 会照着一份
    /// 不相关的文档去改代码。
    pub async fn search_knowledge(
        &self,
        workspace_path: &str,
        query: &str,
        limit: usize,
    ) -> Result<Vec<KnowledgeHit>> {
        let scope = self.scope_for(workspace_path).await;
        if scope.is_empty() || query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let mut embedded = self.embedder.embed(&[query.to_string()]).await?;
        if embedded.is_empty() {
            return Ok(Vec::new());
        }
        let vector = embedded.remove(0);
        self.knowledge
            .search(&scope, &vector, limit, self.config.min_similarity)
            .await
    }

    /// 某个项目索引了多少块。给设置面板显示「已索引 N 块」用 ——
    /// 这个数字必须来自库本身，不能是"数了几个文件"。
    pub async fn knowledge_count(&self, workspace_path: &str) -> Result<usize> {
        let scope = self.scope_for(workspace_path).await;
        if scope.is_empty() {
            return Ok(0);
        }
        self.knowledge.count(&scope).await
    }

    /// 统计信息，给设置面板显示
    pub async fn stats(&self) -> Result<MemoryStats> {
        let (sessions, messages) = self.store.counts().await?;
        let facts = self.vectors.count().await?;
        Ok(MemoryStats {
            sessions,
            messages,
            facts,
            dimensions: self.vectors.dimensions(),
        })
    }

    /// 清空长期记忆（事实），保留对话记录。
    pub async fn clear_facts(&self) -> Result<()> {
        self.vectors.clear().await
    }

    /// 浏览事实（按时间倒序）。给"看看它记住了我什么"的界面用。
    pub async fn list_facts(&self, limit: usize, offset: usize) -> Result<Vec<Fact>> {
        self.vectors.list(limit, offset).await
    }

    /// 删掉一条事实。记错了要能删 —— 只有"清空全部"的话没人会真的去用。
    pub async fn forget_fact(&self, id: &str) -> Result<()> {
        self.vectors.delete(id).await
    }

    /// 绕过模型提取，直接写入一条事实。
    ///
    /// 给联调测试用：验证「向量化 + 召回」这一段时不想把模型提取的不确定性
    /// 混进来。也可用于将来「让用户手动添加一条记忆」的功能。
    pub async fn add_fact(
        &self,
        session_id: &str,
        subject: &str,
        fact: &str,
        scope: &str,
    ) -> Result<()> {
        let embedded = self.embedder.embed(&[fact.to_string()]).await?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let record = Fact {
            id: uuid::Uuid::new_v4().to_string(),
            subject: subject.to_string(),
            fact: fact.to_string(),
            session_id: session_id.to_string(),
            created_at: now,
            scope: scope.to_string(),
        };
        self.vectors.add(&[record], &embedded).await
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryStats {
    pub sessions: usize,
    pub messages: usize,
    pub facts: usize,
    pub dimensions: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use vector::Fact;

    fn recalled(subject: &str, fact: &str, similarity: f32) -> Recalled {
        Recalled {
            fact: Fact {
                id: "x".into(),
                subject: subject.into(),
                fact: fact.into(),
                session_id: "s".into(),
                created_at: 0,
                scope: String::new(),
            },
            similarity,
            score: similarity,
        }
    }

    #[test]
    fn no_recall_means_no_injected_prompt() {
        // 没召回到就不该往 System Prompt 里塞空段落
        assert!(Memory::format_recalled(&[]).is_none());
    }

    #[test]
    fn formats_recalled_facts_with_guidance() {
        let hits = [
            recalled("用户", "养了一只叫小黑的猫", 0.9),
            recalled("用户", "住在上海", 0.7),
        ];
        let text = Memory::format_recalled(&hits).expect("应当有内容");

        assert!(text.contains("小黑"));
        assert!(text.contains("上海"));
        // 必须提示模型别生硬复述，否则每轮都会机械重复一遍记忆
        assert!(text.contains("不要生硬复述"), "实际：{text}");
    }

    #[test]
    fn memory_config_fills_defaults_from_partial_json() {
        let config: MemoryConfig =
            serde_json::from_str(r#"{"recall_limit":3}"#).expect("应当能补齐默认值");
        assert_eq!(config.recall_limit, 3);
        assert!(config.enabled);
        assert_eq!(config.embedding_model, "bge-m3");
        // 阈值必须高到能挡住无关记忆：bge-m3 上无关答案最高约 0.60
        assert!(
            config.min_similarity > 0.6,
            "默认阈值 {} 太低，会把无关记忆也注入上下文",
            config.min_similarity
        );
    }

    fn hit(text: &str, sim: f32, age_days: i64) -> Recalled {
        Recalled {
            fact: Fact {
                id: text.into(),
                subject: "项目".into(),
                fact: text.into(),
                session_id: "s".into(),
                created_at: 1_800_000_000 - age_days * 86_400,
                scope: String::new(),
            },
            similarity: sim,
            score: sim,
        }
    }

    #[test]
    fn 时近性只打破近似平局不覆盖相关性() {
        let now = 1_800_000_000;
        // 一条今天的、一条一年前的，相似度接近
        let mut v = vec![hit("旧但略高", 0.71, 365), hit("新", 0.70, 0)];
        rerank_by_recency(&mut v, now);
        assert_eq!(v[0].fact.id, "新", "近似平局时新的该赢：{v:#?}");

        // 但差距大的时候翻不动 —— 一条真正更相关的旧事实不该被挤掉
        let mut v = vec![hit("旧且明显更相关", 0.79, 365), hit("新但勉强相关", 0.68, 0)];
        rerank_by_recency(&mut v, now);
        assert_eq!(v[0].fact.id, "旧且明显更相关", "{v:#?}");
    }

    #[test]
    fn similarity_保持原值不被混合分覆盖() {
        // 界面要能解释「为什么这条排在前面」，只有一个混合分就看不出来了
        let mut v = vec![hit("a", 0.70, 0)];
        rerank_by_recency(&mut v, 1_800_000_000);
        assert_eq!(v[0].similarity, 0.70);
        assert!(v[0].score > 0.70, "加分没生效");
        assert!(v[0].score <= 0.70 + RECENCY_BONUS + 1e-6, "加分超上限");
    }

    #[test]
    fn 未来时间戳不会算出负年龄() {
        // 机器时钟回拨、或者手工塞进来的 created_at 都可能大于 now
        let mut v = vec![hit("未来", 0.70, -100)];
        rerank_by_recency(&mut v, 1_800_000_000);
        assert!(v[0].score.is_finite());
        assert!(v[0].score <= 0.70 + RECENCY_BONUS + 1e-6);
    }

    #[test]
    fn 项目里的召回块必须说可能过时且以代码为准() {
        let hits = vec![hit("测试用 npm test", 0.8, 1)];
        let chat = Memory::format_recalled_for(&hits, false).unwrap();
        let proj = Memory::format_recalled_for(&hits, true).unwrap();
        assert_ne!(chat, proj);

        // 关于代码的事实每次 commit 都可能失效，这两句是它和陪伴型的分界
        assert!(proj.contains("可能已经过时"), "{proj}");
        assert!(proj.contains("以代码为准"), "{proj}");
        // 陪伴那套不该带这些 —— 「养了只猫」不会因为 commit 失效
        assert!(!chat.contains("以代码为准"));
        // 两套都要带上事实本体
        for b in [&chat, &proj] {
            assert!(b.contains("测试用 npm test"), "{b}");
        }
        assert!(Memory::format_recalled_for(&[], true).is_none());
    }
}
