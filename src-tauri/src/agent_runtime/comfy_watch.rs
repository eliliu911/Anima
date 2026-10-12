//! 盯 ComfyUI 的任务**还在不在动**，代替「开始生成后固定等 N 小时就判失败」。
//!
//! 2026-09-30 出的事：一个三窗口长镜头（720p、挂 5 张参考图）晚上 11 点开跑，一步采样约 30 分钟，
//! 整条要十二个多小时。Anima 在第 4 小时报「开始生成后等了 4 小时还没拿到文件」、防休眠跟着撤了 ——
//! 而 ComfyUI 那边 GPU 99%、好好地在算。用户：「不能固定设置 4 小时没拿到就失败，而是应该监控 Comfy 的进程和实际进度。」
//!
//! 固定时长必然误判：耗时跟分辨率、窗口数、参考图张数、采样档位都是成倍相关的，
//! 同一个 4 小时，对一镜 5 秒是十倍余量，对一场三窗口长镜头连一半都不够。
//!
//! 现在的判据（[`judge`]）：
//! - ComfyUI 每走一步采样都广播一条 `progress`（带 `prompt_id`）—— H3 / Qwen / Z-Image 的节点都调
//!   `comfy.utils.ProgressBar`。**只要还有进度就一直等**，多久都等。
//!   2026-09-30 挂着那条长镜头实测：每 30.5 分钟一条 `{"type":"progress","data":{"value":5,"max":6,"prompt_id":…}}`，
//!   换窗口那一步（6/6 → 下一段 1/6）也只隔 31 分钟 —— 最长的静默是 [`STALL`] 的六分之一。
//! - 听得见（WebSocket 一直连着）却连着 [`STALL`] 一条进度都没有 —— 才算卡死。
//! - 听不见（ws 连不上、ComfyUI 版本不发）时退回一个很宽的上限 [`NO_SIGNAL_CEILING`]，从最后一次进度算起。
//! - HTTP 整个连不上 [`UNREACHABLE`] —— ComfyUI 进程多半退了，任务也就跟着没了（队列在内存里）。
//!
//! **为什么只能听 `progress`**：Anima 提交任务时不带 `client_id`，ComfyUI 对这种任务只广播
//! `progress` / `progress_state`；`executing` / `executed` 要有 `client_id` 才发（ComfyUI `execution.py`）。
//! 不带 `client_id` 反倒是好事：广播不认连接，ws 断了重连照样收得到。
//!
//! 全进程只连一条 ws（[`tracker`] 第一次被调时起），所有在等的任务共用。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio_tungstenite::tungstenite::Message;

/// 听得见却连着这么久没有一点进度，才算卡死。
///
/// 实测最慢的一步：720p 三窗口长镜头挂 5 张参考图，一步采样 30 分钟；
/// 两步之间还有解码、换窗口、加载 122 GB 分页权重。3 小时是它的六倍。
pub const STALL: Duration = Duration::from_secs(3 * 3600);

/// 听不见进度时的上限，**从最后一次听到进度算起**（一次都没听到就从开始生成算起）。
/// 只防死循环 —— 正常情况下 ws 一直连着，走不到这一条。
pub const NO_SIGNAL_CEILING: Duration = Duration::from_secs(24 * 3600);

/// ComfyUI 的 HTTP 连着这么久都连不上，就当它退出了。
///
/// 不能太短：机器内存吃紧、换页的时候 ComfyUI 回一个请求可能要几十秒。
pub const UNREACHABLE: Duration = Duration::from_secs(15 * 60);

/// 断线后多久重连
const RECONNECT: Duration = Duration::from_secs(10);
/// 多久发一次 ping。连着这么多个间隔什么都没收到（连 pong 都没有），就当这条连接是假的，断开重连 ——
/// 不然一条半开的连接会让人以为「听得见」，3 小时后误判成卡死
const PING_EVERY: Duration = Duration::from_secs(60);
const PING_MISSES: u32 = 3;
/// 记录只留这么久，免得跑几个月的进程里越攒越多
const KEEP: Duration = Duration::from_secs(48 * 3600);

const DEFAULT_WS_URL: &str = "ws://127.0.0.1:8188/ws";

/// 一个任务最近一次进度
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub at: Instant,
    /// 最近一条 `progress` 的 `value / max`（`progress_state` 只刷新时间，不带步数）
    pub step: Option<(u64, u64)>,
}

#[derive(Default)]
pub struct Watcher {
    seen: Mutex<HashMap<String, Progress>>,
    /// 当前这条 ws 是什么时候连上的；断着为 `None`
    connected_since: Mutex<Option<Instant>>,
    alive: AtomicBool,
}

impl Watcher {
    fn record(&self, prompt_id: String, step: Option<(u64, u64)>, now: Instant) {
        let mut seen = self.seen.lock().unwrap();
        let prev = seen.get(&prompt_id).and_then(|p| p.step);
        seen.insert(prompt_id, Progress { at: now, step: step.or(prev) });
        seen.retain(|_, p| now.duration_since(p.at) < KEEP);
    }

    pub fn last(&self, prompt_id: &str) -> Option<Progress> {
        self.seen.lock().unwrap().get(prompt_id).copied()
    }

    pub fn listening_since(&self) -> Option<Instant> {
        *self.connected_since.lock().unwrap()
    }
}

/// 从一条 ws 文本消息里取出「哪个任务动了一下」。不是进度消息返回 `None`
pub fn parse_event(text: &str) -> Option<(String, Option<(u64, u64)>)> {
    let v: Value = serde_json::from_str(text).ok()?;
    let data = v.get("data")?;
    let prompt_id = data.get("prompt_id")?.as_str()?.to_string();
    let num = |k: &str| data.get(k).and_then(Value::as_f64).map(|f| f.round() as u64);
    match v.get("type")?.as_str()? {
        "progress" => Some((prompt_id, num("value").zip(num("max")))),
        // 节点开始 / 结束也算动了一下（解码、换窗口那几段没有步数）
        "progress_state" | "executing" | "executed" | "execution_start" | "execution_cached" => Some((prompt_id, None)),
        _ => None,
    }
}

/// 一直连着 ComfyUI 的 ws，把每条进度记进 `w`。断了就重连，永不退出（除非运行时关了）
pub async fn listen(url: String, w: Arc<Watcher>) {
    listen_every(url, w, RECONNECT).await
}

async fn listen_every(url: String, w: Arc<Watcher>, reconnect: Duration) {
    // 运行时关掉时这个 future 直接被丢弃，跑不到循环后面 —— 靠 Drop 把「活着」清掉，下次要用时重起
    struct Alive(Arc<Watcher>);
    impl Drop for Alive {
        fn drop(&mut self) {
            *self.0.connected_since.lock().unwrap() = None;
            self.0.alive.store(false, Ordering::SeqCst);
        }
    }
    let _alive = Alive(w.clone());
    loop {
        if let Ok((ws, _)) = tokio_tungstenite::connect_async(url.as_str()).await {
            *w.connected_since.lock().unwrap() = Some(Instant::now());
            let (mut tx, mut rx) = ws.split();
            let mut tick = tokio::time::interval(PING_EVERY);
            tick.tick().await;
            let mut quiet_ticks = 0u32;
            loop {
                tokio::select! {
                    msg = rx.next() => match msg {
                        Some(Ok(m)) => {
                            quiet_ticks = 0;
                            if let Message::Text(t) = m {
                                if let Some((id, step)) = parse_event(&t) {
                                    w.record(id, step, Instant::now());
                                }
                            }
                        }
                        _ => break,
                    },
                    _ = tick.tick() => {
                        quiet_ticks += 1;
                        if quiet_ticks > PING_MISSES || tx.send(Message::Ping(Vec::new())).await.is_err() {
                            break;
                        }
                    }
                }
            }
            *w.connected_since.lock().unwrap() = None;
        }
        tokio::time::sleep(reconnect).await;
    }
}

static GLOBAL: OnceLock<Arc<Watcher>> = OnceLock::new();

/// 全进程共用的那一个。第一次调（或上一个监听任务随运行时一起没了）时起监听
fn global() -> Arc<Watcher> {
    let w = GLOBAL.get_or_init(|| Arc::new(Watcher::default())).clone();
    if !w.alive.swap(true, Ordering::SeqCst) {
        match tokio::runtime::Handle::try_current() {
            Ok(h) => {
                h.spawn(listen(format!("{DEFAULT_WS_URL}?clientId=anima-watch-{}", std::process::id()), w.clone()));
            }
            Err(_) => w.alive.store(false, Ordering::SeqCst),
        }
    }
    w
}

/// 全进程最近一次听到**任何**任务有进度，离现在多久。
///
/// 给防休眠用（[`crate::keep_awake`]）：单镜出片期间 `jobs.json` 可以几个小时不写，
/// 这里的进度才是「还在跑」的证据。**只读，不起监听** —— 没在听、或一条都没听到过，返回 `None`。
pub fn since_last_progress() -> Option<Duration> {
    let w = GLOBAL.get()?;
    let seen = w.seen.lock().ok()?;
    seen.values().map(|p| p.at.elapsed()).min()
}

/// 测试用：往全进程那一个里记一条进度（不起监听）
#[cfg(test)]
pub(crate) fn record_for_test(prompt_id: &str, ago: Duration) {
    let w = GLOBAL.get_or_init(|| Arc::new(Watcher::default())).clone();
    let at = Instant::now().checked_sub(ago).unwrap_or_else(Instant::now);
    w.record(prompt_id.to_string(), None, at);
}

/// 这一轮查 ComfyUI 看到的样子
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seen {
    /// 在队列里排着，还没轮到
    Queued,
    /// 在跑（或者刚离开队列、history 还没写完）
    Running,
    /// HTTP 连不上
    Unreachable,
}

/// 还等不等
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Wait,
    /// 听得见，却连着这么久没有一点进度
    Stalled(Duration),
    /// 听不见进度，已经这么久了
    NoSignal(Duration),
    /// ComfyUI 整个连不上这么久了
    Unreachable(Duration),
}

impl Verdict {
    /// 给用户看的一句话。**任务没取消** —— 说清去哪儿看，别说成「失败了」让人以为白跑
    pub fn explain(self, prompt_id: &str) -> Option<String> {
        let h = |d: Duration| {
            let m = d.as_secs() / 60;
            if m >= 60 { format!("{} 小时 {} 分", m / 60, m % 60) } else { format!("{m} 分钟") }
        };
        match self {
            Verdict::Wait => None,
            Verdict::Stalled(d) => Some(format!(
                "ComfyUI 已经 {} 没有任何进度了（prompt_id `{prompt_id}`）—— 多半卡死了。\
                 Anima 不再等它，但没有取消：去 ComfyUI 看一眼，真卡住了就在那边中断",
                h(d)
            )),
            Verdict::NoSignal(d) => Some(format!(
                "{} 没收到 ComfyUI 的进度消息，也没拿到文件（prompt_id `{prompt_id}`）。\
                 ComfyUI 那边可能还在跑 —— 跑完之后下次打开 Anima 会自动把文件收回来",
                h(d)
            )),
            Verdict::Unreachable(d) => Some(format!(
                "ComfyUI 已经 {} 连不上了（prompt_id `{prompt_id}`）—— 进程多半退出了，队列里的任务跟着没了。重开 ComfyUI 后重新出",
                h(d)
            )),
        }
    }
}

/// 判定。纯函数，所有时刻都由调用方给 —— 测试靠它造「3 小时以后」
///
/// - `run_started`：第一次看到它不在排队的时刻；`None` = 还在排队（排队另有上限，不归这里管）
/// - `last_progress`：最近一次听到它的进度
/// - `listening_since`：当前这条 ws 连上的时刻；`None` = 没连着
/// - `unreachable_since`：HTTP 从什么时候开始一直连不上；`None` = 这一轮连得上
pub fn judge(
    now: Instant,
    run_started: Option<Instant>,
    last_progress: Option<Instant>,
    listening_since: Option<Instant>,
    unreachable_since: Option<Instant>,
) -> Verdict {
    if let Some(u) = unreachable_since {
        let d = now.saturating_duration_since(u);
        if d >= UNREACHABLE {
            return Verdict::Unreachable(d);
        }
    }
    let Some(started) = run_started else { return Verdict::Wait };
    let quiet_from = last_progress.map_or(started, |p| p.max(started));
    let quiet = now.saturating_duration_since(quiet_from);
    // 「没有进度」得是听得见时的没有 —— 这条连接连着的时间够长，才说明真的没人说话
    let heard = listening_since.is_some_and(|l| now.saturating_duration_since(l) >= STALL);
    if quiet >= STALL && heard {
        return Verdict::Stalled(quiet);
    }
    if quiet >= NO_SIGNAL_CEILING {
        return Verdict::NoSignal(quiet);
    }
    Verdict::Wait
}

/// 一个任务的等待状态。轮询循环每轮调一次 [`Tracker::observe`]
pub struct Tracker {
    prompt_id: String,
    watcher: Arc<Watcher>,
    run_started: Option<Instant>,
    unreachable_since: Option<Instant>,
}

/// 开始盯一个任务（顺带把 ws 监听起起来）
pub fn tracker(prompt_id: &str) -> Tracker {
    Tracker::with_watcher(prompt_id, global())
}

impl Tracker {
    pub fn with_watcher(prompt_id: &str, watcher: Arc<Watcher>) -> Self {
        Self { prompt_id: prompt_id.to_string(), watcher, run_started: None, unreachable_since: None }
    }

    pub fn observe(&mut self, seen: Seen) -> Verdict {
        self.observe_at(seen, Instant::now())
    }

    pub fn observe_at(&mut self, seen: Seen, now: Instant) -> Verdict {
        match seen {
            Seen::Queued => self.unreachable_since = None,
            Seen::Running => {
                self.unreachable_since = None;
                self.run_started.get_or_insert(now);
            }
            Seen::Unreachable => {
                self.unreachable_since.get_or_insert(now);
            }
        }
        let last = self.watcher.last(&self.prompt_id).map(|p| p.at);
        judge(now, self.run_started, last, self.watcher.listening_since(), self.unreachable_since)
    }

    /// 真正开始跑（离开排队）之后过了多少秒；还在排队是 0
    pub fn run_secs(&self) -> u64 {
        self.run_started.map_or(0, |t| t.elapsed().as_secs())
    }

    /// 最近一次进度，给进度文字用
    pub fn progress(&self) -> Option<Progress> {
        self.watcher.last(&self.prompt_id)
    }

    /// 「采样 4/6 · 12 分钟前」这样一句；还没听到过进度返回 `None`
    pub fn describe(&self) -> Option<String> {
        let p = self.progress()?;
        let ago = p.at.elapsed().as_secs() / 60;
        let when = if ago == 0 { "刚刚".to_string() } else { format!("{ago} 分钟前") };
        Some(match p.step {
            Some((v, m)) if m > 0 => format!("采样 {v}/{m} · {when}"),
            _ => format!("最近一次进度 {when}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: Duration = Duration::from_secs(3600);

    #[test]
    fn 读得出进度消息_别的消息不算() {
        assert_eq!(
            parse_event(r#"{"type":"progress","data":{"value":4,"max":6,"prompt_id":"p1","node":"12"}}"#),
            Some(("p1".into(), Some((4, 6))))
        );
        assert_eq!(
            parse_event(r#"{"type":"progress_state","data":{"prompt_id":"p1","nodes":{}}}"#),
            Some(("p1".into(), None))
        );
        // 连上时 ComfyUI 先发一条 status —— 不带 prompt_id，不是谁的进度
        assert_eq!(parse_event(r#"{"type":"status","data":{"status":{"exec_info":{"queue_remaining":1}},"sid":"x"}}"#), None);
        assert_eq!(parse_event("不是 json"), None);
    }

    #[test]
    fn 有进度就一直等_十二小时也等() {
        let t0 = Instant::now();
        let now = t0 + 12 * H;
        // 一直连着，半小时前还有一步进度 —— 这就是那条三窗口长镜头
        assert_eq!(judge(now, Some(t0), Some(now - H / 2), Some(t0), None), Verdict::Wait);
    }

    #[test]
    fn 听得见却三小时没进度_才算卡死() {
        let t0 = Instant::now();
        let last = t0 + H;
        assert_eq!(judge(last + STALL - Duration::from_secs(60), Some(t0), Some(last), Some(t0), None), Verdict::Wait);
        assert_eq!(judge(last + STALL, Some(t0), Some(last), Some(t0), None), Verdict::Stalled(STALL));
    }

    #[test]
    fn 听不见的时候不判卡死_退回宽上限() {
        let t0 = Instant::now();
        // ws 一直没连上：3 小时没进度不能说它卡了 —— 可能只是我们没听到
        assert_eq!(judge(t0 + 5 * H, Some(t0), None, None, None), Verdict::Wait);
        // 刚重连上 10 分钟也不够：这 10 分钟里没进度说明不了什么
        let now = t0 + 5 * H;
        assert_eq!(judge(now, Some(t0), None, Some(now - Duration::from_secs(600)), None), Verdict::Wait);
        assert_eq!(judge(t0 + NO_SIGNAL_CEILING, Some(t0), None, None, None), Verdict::NoSignal(NO_SIGNAL_CEILING));
        // 宽上限从最后一次进度算，不是从开始算 —— 在动的任务永远不会撞上它
        let last = t0 + 20 * H;
        assert_eq!(judge(t0 + NO_SIGNAL_CEILING, Some(t0), Some(last), None, None), Verdict::Wait);
    }

    #[test]
    fn 排队的时候不归这里管() {
        let t0 = Instant::now();
        assert_eq!(judge(t0 + 30 * H, None, None, Some(t0), None), Verdict::Wait);
    }

    #[test]
    fn 连不上十五分钟算进程退了_中间连上一次就清零() {
        let w = Arc::new(Watcher::default());
        let mut t = Tracker::with_watcher("p1", w);
        let t0 = Instant::now();
        assert_eq!(t.observe_at(Seen::Running, t0), Verdict::Wait);
        assert_eq!(t.observe_at(Seen::Unreachable, t0 + Duration::from_secs(60)), Verdict::Wait);
        assert_eq!(t.observe_at(Seen::Unreachable, t0 + Duration::from_secs(60) + UNREACHABLE - Duration::from_secs(1)), Verdict::Wait);
        assert_eq!(t.observe_at(Seen::Running, t0 + UNREACHABLE), Verdict::Wait);
        assert_eq!(t.observe_at(Seen::Unreachable, t0 + UNREACHABLE + Duration::from_secs(3)), Verdict::Wait);
        assert!(matches!(
            t.observe_at(Seen::Unreachable, t0 + 2 * UNREACHABLE + Duration::from_secs(3)),
            Verdict::Unreachable(_)
        ));
    }

    #[test]
    fn 排队的时间不算进安静期() {
        let w = Arc::new(Watcher::default());
        *w.connected_since.lock().unwrap() = Some(Instant::now());
        let mut t = Tracker::with_watcher("p1", w);
        let t0 = Instant::now() + H;
        // 排了 5 小时（前面的长镜头在跑），轮到它之后才开始算
        assert_eq!(t.observe_at(Seen::Queued, t0), Verdict::Wait);
        assert_eq!(t.observe_at(Seen::Running, t0 + 5 * H), Verdict::Wait);
        assert_eq!(t.observe_at(Seen::Running, t0 + 5 * H + STALL - Duration::from_secs(1)), Verdict::Wait);
        assert!(matches!(t.observe_at(Seen::Running, t0 + 5 * H + STALL), Verdict::Stalled(_)));
    }

    /// 真起一个 ws 服务：收得到进度、断了会重连、重连后接着收
    #[tokio::test]
    async fn 连着真的_ws_收进度_断线重连() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let progress = |v: u64| Message::Text(format!(r#"{{"type":"progress","data":{{"value":{v},"max":6,"prompt_id":"p1","node":"12"}}}}"#));
        tokio::spawn(async move {
            let (s, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(s).await.unwrap();
            ws.send(Message::Text(r#"{"type":"status","data":{"status":{},"sid":"x"}}"#.into())).await.unwrap();
            ws.send(progress(2)).await.unwrap();
            ws.close(None).await.ok();
            drop(ws);
            // 重连上来的第二条
            let (s, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(s).await.unwrap();
            ws.send(progress(3)).await.unwrap();
            // 连着不断，读掉 ping
            while ws.next().await.is_some() {}
        });
        let w = Arc::new(Watcher::default());
        let task = tokio::spawn(listen_every(format!("ws://{addr}/ws"), w.clone(), Duration::from_millis(50)));
        let deadline = Instant::now() + Duration::from_secs(10);
        while w.last("p1").and_then(|p| p.step) != Some((3, 6)) {
            assert!(Instant::now() < deadline, "没收到重连后的进度：{:?}", w.last("p1"));
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(w.listening_since().is_some(), "连着的时候要算「听得见」");
        assert!(w.last("x").is_none(), "status 不是谁的进度");
        task.abort();
        let _ = task.await;
        assert!(w.listening_since().is_none() && !w.alive.load(Ordering::SeqCst), "监听没了要清掉，下次用时才会重起");
    }

    #[test]
    fn 进度文字带步数() {
        let w = Arc::new(Watcher::default());
        w.record("p1".into(), Some((4, 6)), Instant::now());
        // progress_state 不带步数，但不能把上一条的步数冲掉
        w.record("p1".into(), None, Instant::now());
        let t = Tracker::with_watcher("p1", w);
        assert_eq!(t.describe().as_deref(), Some("采样 4/6 · 刚刚"));
    }
}
