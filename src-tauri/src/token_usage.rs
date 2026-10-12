//! 模型用量账本：每次调用花了多少 token，记在哪个后端、哪个模型、哪一集、哪一步上。
//!
//! # 为什么要有
//!
//! 2026-09-29 用户问「刚刚用 gpt-5.6 生成，有没有查看过 token」—— 查不了：
//! 上游每次都把 `usage` 返回来了，网关也一路传到了 `ChatChunk::Done`，但 `prompts::run` 只收正文，
//! 日志里一个数都没有。一集 EP01 花了多少只能按字数估（±30%）。
//!
//! # 在哪记
//!
//! **网关**（`Gateway::chat_stream`）：所有调用都从那儿过，看得到后端、模型和 `Done` 里的 `usage`。
//! 归到哪一集靠 task-local 的 [`Scope`]：后台任务在 spawn 时用 [`scoped`] 包一层（项目 + 分集），
//! `prompts::run` 再用 [`with_step`] 标上是哪一步（镜头表 / 细化时间线 / 线路 B……）。
//! 对话、别处的调用不在任何范围里，照样记，只是不归到某一集。
//!
//! # 怎么存
//!
//! 一个小 JSON（`token_usage.json`），按「后端 + 模型 + 项目 + 分集 + 步骤」累加，一行一个组合。
//! 一集几十次调用、一部剧几千次，文件也就几十 KB。写盘先写临时文件再改名，写一半断电不会把账写坏。

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::llm::Usage;

/// 这次调用算在谁头上
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scope {
    pub project_id: String,
    pub episode_id: String,
    pub step: String,
}

tokio::task_local! {
    static SCOPE: Scope;
}

impl Scope {
    /// 一集的范围。先建好再 `spawn(scope.run(async move { … }))` —— 参数在 async move 里被拿走之前就量好了
    pub fn episode(project_id: &str, episode_id: &str) -> Scope {
        Scope { project_id: project_id.into(), episode_id: episode_id.into(), step: String::new() }
    }
    pub fn run<F: Future>(self, fut: F) -> impl Future<Output = F::Output> {
        SCOPE.scope(self, fut)
    }
}

/// 后台任务整段算在这一集头上
///
/// **外层范围要在跑起来的时候读，不是在建 future 的时候读。** `scoped(.., with_step(.., f))` 这种写法里，
/// `with_step` 的参数先求值 —— 那时还没进 `scoped` 的范围，早读的话项目和分集记成空的（接线测试抓到过）。
pub fn scoped<F: Future>(project_id: &str, episode_id: &str, fut: F) -> impl Future<Output = F::Output> {
    let scope = Scope::episode(project_id, episode_id);
    async move { Scope { step: current().step, ..scope }.run(fut).await }
}

/// 这一段调用标上是哪一步（沿用外面的项目 / 分集）
pub fn with_step<F: Future>(step: &str, fut: F) -> impl Future<Output = F::Output> {
    let step = step.to_string();
    async move { Scope { step, ..current() }.run(fut).await }
}

/// 当前所在的范围；不在任何范围里是空的
pub fn current() -> Scope {
    SCOPE.try_with(|s| s.clone()).unwrap_or_default()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Row {
    pub provider: String,
    /// 设置里显示的后端名（「Azure OpenAI」），记下来是因为后端以后可能被删掉
    #[serde(default)]
    pub provider_label: String,
    pub model: String,
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub episode_id: String,
    #[serde(default)]
    pub step: String,
    pub calls: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// 输出里花在思考上的（已经算在 `completion_tokens` 里，单独列出来看）
    #[serde(default)]
    pub reasoning_tokens: u64,
    #[serde(default)]
    pub last_at: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Ledger {
    rows: Vec<Row>,
}

struct Store {
    path: PathBuf,
    ledger: Option<Ledger>,
}

static STORE: OnceLock<Mutex<Store>> = OnceLock::new();

/// 应用启动时调一次。没调过的话（测试、命令行工具）记账什么都不做
pub fn init(config_dir: &Path) {
    let _ = STORE.set(Mutex::new(Store { path: config_dir.join("token_usage.json"), ledger: None }));
}

fn load(path: &Path) -> Ledger {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save(path: &Path, ledger: &Ledger) {
    let tmp = path.with_extension("json.tmp");
    let Ok(text) = serde_json::to_string(ledger) else { return };
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

fn add(ledger: &mut Ledger, provider: &str, provider_label: &str, model: &str, scope: &Scope, usage: &Usage, now: u64) {
    let row = match ledger.rows.iter_mut().position(|r| {
        r.provider == provider && r.model == model && r.project_id == scope.project_id && r.episode_id == scope.episode_id && r.step == scope.step
    }) {
        Some(i) => &mut ledger.rows[i],
        None => {
            ledger.rows.push(Row {
                provider: provider.into(),
                model: model.into(),
                project_id: scope.project_id.clone(),
                episode_id: scope.episode_id.clone(),
                step: scope.step.clone(),
                ..Default::default()
            });
            ledger.rows.last_mut().unwrap()
        }
    };
    row.provider_label = provider_label.into();
    row.calls += 1;
    row.prompt_tokens += usage.prompt_tokens as u64;
    row.completion_tokens += usage.completion_tokens as u64;
    row.reasoning_tokens += usage.reasoning_tokens.unwrap_or(0) as u64;
    row.last_at = now;
}

/// 记一次调用。网关在流结束、拿到 `usage` 时调
pub fn record(provider: &str, provider_label: &str, model: &str, scope: &Scope, usage: &Usage) {
    let Some(store) = STORE.get() else { return };
    let Ok(mut st) = store.lock() else { return };
    let path = st.path.clone();
    let ledger = st.ledger.get_or_insert_with(|| load(&path));
    add(ledger, provider, provider_label, model, scope, usage, crate::jobs::now());
    save(&path, ledger);
}

/// 全部行（给汇总用）
fn rows() -> Vec<Row> {
    let Some(store) = STORE.get() else { return Vec::new() };
    let Ok(mut st) = store.lock() else { return Vec::new() };
    let path = st.path.clone();
    st.ledger.get_or_insert_with(|| load(&path)).rows.clone()
}

// ---------------------------------------------------------------------------
// 汇总
// ---------------------------------------------------------------------------

/// 每百万 token 的美元价（输入, 输出）。**只收官方公布过的**，认不出的不估 —— 估错比不估更糟。
/// 本地后端（LM Studio / Ollama / MLX）不花钱，也不估。
/// 数字来自 OpenAI 官方模型页（2026-09-29 查）；Azure 标准部署一般同价，以账单为准。
pub fn price_per_million(provider_kind_is_cloud: bool, model: &str) -> Option<(f64, f64)> {
    if !provider_kind_is_cloud {
        return None;
    }
    let m = model.to_lowercase();
    // 长的先匹配：`gpt-5.6-sol` 不能被当成别的
    let table: [(&str, (f64, f64)); 3] = [("gpt-5.6-sol", (4.0, 20.0)), ("gpt-5.6-terra", (2.0, 12.0)), ("gpt-5.6-luna", (0.2, 1.2))];
    table.iter().find(|(k, _)| m.contains(k)).map(|(_, p)| *p)
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Totals {
    pub calls: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub reasoning_tokens: u64,
    /// 估算的美元数；有任何一部分认不出价就是 `None` 那部分不算（见 `priced_all`）
    pub usd: f64,
    /// 是不是每一笔都认得出价。`false` 时 `usd` 只是认得出的那部分
    pub priced_all: bool,
}

impl Totals {
    fn add(&mut self, r: &Row, price: Option<(f64, f64)>, cloud: bool) {
        self.calls += r.calls;
        self.prompt_tokens += r.prompt_tokens;
        self.completion_tokens += r.completion_tokens;
        self.reasoning_tokens += r.reasoning_tokens;
        match price {
            Some((i, o)) => self.usd += (r.prompt_tokens as f64 * i + r.completion_tokens as f64 * o) / 1_000_000.0,
            // 本地模型不花钱，不影响「每一笔都认得出价」
            None if cloud => self.priced_all = false,
            None => {}
        }
    }
    fn new() -> Self {
        Totals { priced_all: true, ..Default::default() }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelTotals {
    pub provider: String,
    pub provider_label: String,
    pub model: String,
    pub cloud: bool,
    /// 每百万 token 的价（输入, 输出）；认不出是 `None`
    pub price: Option<(f64, f64)>,
    pub totals: Totals,
}

#[derive(Debug, Clone, Serialize)]
pub struct StepTotals {
    pub step: String,
    pub totals: Totals,
}

#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    /// 这个项目里，按后端 + 模型
    pub models: Vec<ModelTotals>,
    /// 这个项目里，按步骤
    pub steps: Vec<StepTotals>,
    /// 这个项目里，按分集（键是分集 id；项目级、没归到某一集的在 `""` 下）
    pub episodes: std::collections::BTreeMap<String, Totals>,
    pub project: Totals,
    /// 所有项目 + 对话等不归项目的，按后端 + 模型
    pub all_models: Vec<ModelTotals>,
    pub all: Totals,
}

fn by_model(rows: &[&Row], is_cloud: &dyn Fn(&str) -> bool) -> Vec<ModelTotals> {
    let mut out: Vec<ModelTotals> = Vec::new();
    for r in rows {
        let cloud = is_cloud(&r.provider);
        let price = price_per_million(cloud, &r.model);
        let i = match out.iter().position(|m| m.provider == r.provider && m.model == r.model) {
            Some(i) => i,
            None => {
                out.push(ModelTotals { provider: r.provider.clone(), provider_label: r.provider_label.clone(), model: r.model.clone(), cloud, price, totals: Totals::new() });
                out.len() - 1
            }
        };
        out[i].totals.add(r, price, cloud);
    }
    out.sort_by_key(|m| std::cmp::Reverse(m.totals.prompt_tokens + m.totals.completion_tokens));
    out
}

pub fn summarize(all_rows: &[Row], project_id: &str, is_cloud: &dyn Fn(&str) -> bool) -> Summary {
    let all: Vec<&Row> = all_rows.iter().collect();
    let mine: Vec<&Row> = all_rows.iter().filter(|r| r.project_id == project_id).collect();
    let mut steps: Vec<StepTotals> = Vec::new();
    let mut episodes = std::collections::BTreeMap::new();
    let mut project = Totals::new();
    let mut total = Totals::new();
    for r in &all {
        let cloud = is_cloud(&r.provider);
        total.add(r, price_per_million(cloud, &r.model), cloud);
    }
    for r in &mine {
        let cloud = is_cloud(&r.provider);
        let price = price_per_million(cloud, &r.model);
        project.add(r, price, cloud);
        episodes.entry(r.episode_id.clone()).or_insert_with(Totals::new).add(r, price, cloud);
        let i = match steps.iter().position(|s| s.step == r.step) {
            Some(i) => i,
            None => {
                steps.push(StepTotals { step: r.step.clone(), totals: Totals::new() });
                steps.len() - 1
            }
        };
        steps[i].totals.add(r, price, cloud);
    }
    Summary { models: by_model(&mine, is_cloud), steps, episodes, project, all_models: by_model(&all, is_cloud), all: total }
}

/// 概览页用：这个项目的用量，外加全部的
#[tauri::command]
pub async fn token_usage_summary(state: tauri::State<'_, crate::AppState>, project_id: String) -> crate::error::Result<Summary> {
    let config = state.gateway.config();
    let config = config.read().await.clone();
    let is_cloud = move |id: &str| {
        use crate::config::ProviderKind::*;
        config.providers.iter().find(|p| p.id == id).is_some_and(|p| matches!(p.kind, OpenAi | AzureOpenAi | Anthropic))
    };
    Ok(summarize(&rows(), &project_id, &is_cloud))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(p: u32, c: u32) -> Usage {
        Usage { prompt_tokens: p, completion_tokens: c, reasoning_tokens: None }
    }

    #[test]
    fn 同一个组合累加_不同组合各记一行() {
        let mut l = Ledger::default();
        let ep = Scope { project_id: "p".into(), episode_id: "ep_01".into(), step: "镜头表".into() };
        add(&mut l, "azure", "Azure OpenAI", "gpt-5.6-sol", &ep, &usage(10_000, 2_000), 1);
        add(&mut l, "azure", "Azure OpenAI", "gpt-5.6-sol", &ep, &usage(5_000, 1_000), 2);
        add(&mut l, "azure", "Azure OpenAI", "gpt-5.6-sol", &Scope { step: "线路 B".into(), ..ep.clone() }, &usage(1, 1), 3);
        add(&mut l, "lmstudio", "LM Studio", "ornith", &Scope::default(), &usage(7, 7), 4);
        assert_eq!(l.rows.len(), 3);
        assert_eq!((l.rows[0].calls, l.rows[0].prompt_tokens, l.rows[0].completion_tokens, l.rows[0].last_at), (2, 15_000, 3_000, 2));
    }

    #[test]
    fn 汇总_按模型按集按步骤_费用只算认得出价的() {
        let mut l = Ledger::default();
        let s = |ep: &str, step: &str| Scope { project_id: "p".into(), episode_id: ep.into(), step: step.into() };
        add(&mut l, "azure", "Azure OpenAI", "gpt-5.6-sol", &s("ep_01", "镜头表"), &usage(1_000_000, 100_000), 1);
        add(&mut l, "azure", "Azure OpenAI", "gpt-5.6-sol", &s("ep_02", "线路 B"), &usage(500_000, 0), 1);
        add(&mut l, "lmstudio", "LM Studio", "ornith", &s("ep_01", "细化时间线"), &usage(9, 9), 1);
        add(&mut l, "azure", "Azure OpenAI", "gpt-5.6-sol", &Scope { project_id: "别的剧".into(), ..Default::default() }, &usage(1_000_000, 0), 1);
        let cloud = |id: &str| id == "azure";
        let sum = summarize(&l.rows, "p", &cloud);
        // 本剧：1.5M 输入 × $4 + 0.1M 输出 × $20 = $8
        assert!((sum.project.usd - 8.0).abs() < 1e-9, "{}", sum.project.usd);
        assert!(sum.project.priced_all, "本地模型不花钱，不算认不出价");
        assert_eq!(sum.episodes["ep_01"].prompt_tokens, 1_000_009);
        assert!((sum.episodes["ep_02"].usd - 2.0).abs() < 1e-9);
        assert_eq!(sum.models.len(), 2);
        assert_eq!(sum.models[0].model, "gpt-5.6-sol", "按用量从多到少");
        assert_eq!(sum.steps.len(), 3);
        // 全部：多出别的剧那 1M 输入
        assert!((sum.all.usd - 12.0).abs() < 1e-9);
        // 认不出价的云端模型：不瞎估，标出来
        add(&mut l, "azure", "Azure OpenAI", "某个新模型", &s("ep_01", "镜头表"), &usage(1, 1), 1);
        assert!(!summarize(&l.rows, "p", &cloud).project.priced_all);
    }

    #[test]
    fn 价目只认官方公布过的() {
        assert_eq!(price_per_million(true, "gpt-5.6-sol"), Some((4.0, 20.0)));
        assert_eq!(price_per_million(true, "GPT-5.6-Terra"), Some((2.0, 12.0)));
        assert_eq!(price_per_million(true, "gpt-4o"), None);
        assert_eq!(price_per_million(false, "gpt-5.6-sol"), None, "本地跑的不估");
    }

    #[tokio::test]
    async fn 任务范围_里面标步骤沿用外面的集() {
        assert_eq!(current(), Scope::default());
        let got = scoped("p", "ep_01", async { with_step("镜头表", async { current() }).await }).await;
        assert_eq!(got, Scope { project_id: "p".into(), episode_id: "ep_01".into(), step: "镜头表".into() });
        assert_eq!(current(), Scope::default(), "出了范围就不算了");
    }

    #[test]
    fn 账本写坏了从空的开始_不拖垮调用() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("token_usage.json");
        std::fs::write(&p, "{坏的").unwrap();
        assert!(load(&p).rows.is_empty());
        let mut l = Ledger::default();
        add(&mut l, "azure", "Azure OpenAI", "m", &Scope::default(), &usage(1, 2), 1);
        save(&p, &l);
        assert_eq!(load(&p).rows.len(), 1);
    }
}
