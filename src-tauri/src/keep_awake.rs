//! 出片期间别让 Mac 睡过去。
//!
//! # 为什么需要这个
//!
//! 2026-09-20 实测，E05 第 2 镜（15 秒，576×1024）：
//!
//! | | |
//! |---|---:|
//! | 任务面板显示的耗时 | **496.9 分钟** |
//! | H3 自己记的分段计时之和 | **67.9 分钟** |
//!
//! 差的 429 分钟在 `pmset -g log` 里：任务 `00:34:35` 起跑，
//! `00:36` 关屏，**`01:13:08` 进入 Idle Sleep** —— 开跑 38 分钟就睡了，
//! 然后整夜在 Sleep / DarkWake 之间循环，早上用户唤醒机器才接着跑完。
//!
//! 也就是说：**挂一个整集的批量出片去睡觉，第二天早上它才刚开始跑第二镜。**
//! 而界面上看不出任何异常 —— 任务是「运行中」，耗时一路往上涨，
//! 和「这一镜真的很慢」长得一模一样。这是这个项目最典型的失败形状。
//!
//! # 按住什么，不按住什么
//!
//! `caffeinate -s -i`：挡**系统**睡眠和**空闲**睡眠。
//!
//! **不加 `-d`。** 那个挡的是显示器睡眠 —— 挂一夜出片不该让屏幕亮一夜。
//! 关屏不影响 GPU，上面那条日志里 `00:36` 就关屏了，真正让它停下来的是
//! `01:13` 那次 Idle Sleep。
//!
//! # 为什么是子进程而不是 IOPMAssertion
//!
//! `IOPMAssertionCreateWithName` 要么引一个 objc/IOKit 的绑定，要么自己写
//! `extern "C"` 声明加手动管理 CFString。`caffeinate` 是系统自带的命令、
//! 做的就是这一件事，而且**它接受 `-w <pid>`**：
//! 我们的进程一没，它自己就退出了。
//!
//! 这一条很要紧 —— 子进程在 Unix 上不会跟着父进程死。没有 `-w` 的话，
//! 应用被强杀之后会留下一个永远按着的 `caffeinate`，
//! 而那正是「一个悄悄生效的东西比没有更糟」。
//!
//! # 什么时候松手：看有没有动静，不看按了多久
//!
//! 任务记录说「运行中」不等于真的在跑（应用被强杀过、某个环节挂住了），所以要有个松手的条件。
//!
//! **原来是「最多按住 24 小时」，2026-10-02 改掉了。**它有两个洞：
//! - 正常的长批量被它放掉：《回收师》EP01 一集 32 镜、720P，整批要 40 多个小时。
//!   跑到第 24 小时它松了手，后半夜全靠用户自己在终端开的 `caffeinate` 撑着。
//! - 真挂住的它反而管不到：判断原来只在写 `jobs.json` 时做，挂住的任务不再写文件，上限永远轮不到。
//!
//! 现在：**连续 [`STALL_SECS`] 没有任何动静才松手**。动静有两种 ——
//! `jobs.json` 被写（起任务、报进度、跑完），以及 ComfyUI 广播的采样进度
//! （[`crate::agent_runtime::comfy_watch::since_last_progress`]；单镜出片期间 `jobs.json`
//! 可以几个小时不写，最慢的一步采样也只隔 30 分钟左右）。
//! 另有一个 [`TICK`] 一次的自查，不靠写文件触发，挂住的任务才查得出来。
//! 松手之后又有了动静，就重新按住。
//!
//! 应用启动时 [`crate::jobs::reap_stale`] 会把上一次没跑完的判为失败，
//! 所以这里只兜「同一次运行里挂住」这一种情况。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

/// 连续这么久没有任何动静，才当任务挂住了、松手。
///
/// 和 ComfyUI 那边「听不见进度时最多等多久」是同一个数 —— 等任务的那一侧都还认它活着，
/// 防休眠就不该先放手。听得见进度时 ComfyUI 3 小时没进度就把任务判失败，
/// 任务一结束这里就跟着松手，用不着等满这个数。
pub const STALL_SECS: u64 = crate::agent_runtime::comfy_watch::NO_SIGNAL_CEILING.as_secs();

/// 多久自查一次。只影响「挂住之后多久松手」，不影响按住。
pub const TICK: Duration = Duration::from_secs(10 * 60);

/// 现在按着没有、最近一次看到动静是什么时候。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HoldState {
    /// 开始按住的时刻（秒）。`None` = 没按。
    pub since: Option<u64>,
    /// 最近一次看到任务还活着的时刻（秒）
    pub last_alive: u64,
    /// 因为太久没动静松的手。**有了新动静就清掉、重新按住**；任务跑完也清掉。
    pub stalled: bool,
}

/// 该做什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// 开始按住
    Hold,
    /// 松手
    Release,
    /// 什么都不做
    Nothing,
}

/// 纯判断，不碰进程。**拎出来是为了能测** —— 真按住要起一个子进程。
///
/// `last_alive` 是最近一次看到动静的时刻。**按了多久不进判断** —— 那正是原来的洞。
pub fn decide(running: bool, holding: bool, now: u64, last_alive: u64) -> Action {
    if !running {
        return if holding { Action::Release } else { Action::Nothing };
    }
    let quiet = now.saturating_sub(last_alive) >= STALL_SECS;
    match (holding, quiet) {
        (false, false) => Action::Hold,
        (true, true) => Action::Release,
        _ => Action::Nothing,
    }
}

/// 把 [`decide`] 的结果写回状态。回 `(新状态, 这次做了什么)`。
///
/// `alive_at` 是这一次看到的动静（写 `jobs.json` 那次就是 `now`；自查那次是 ComfyUI 最近一条进度），
/// 没看到就传 0。和之前记下的取较新的那个。
pub fn apply(running: bool, st: HoldState, now: u64, alive_at: u64) -> (HoldState, Action) {
    let last_alive = st.last_alive.max(alive_at);
    let act = decide(running, st.since.is_some(), now, last_alive);
    let next = match act {
        Action::Hold => HoldState { since: Some(now), last_alive, stalled: false },
        // 任务还在跑却松了手 = 太久没动静；任务跑完松的手不算
        Action::Release => HoldState { since: None, last_alive, stalled: running },
        Action::Nothing if running => HoldState { last_alive, ..st },
        Action::Nothing => HoldState { since: None, last_alive, stalled: false },
    };
    (next, act)
}

static HELD: Mutex<Option<std::process::Child>> = Mutex::new(None);
static STATE: Mutex<HoldState> = Mutex::new(HoldState { since: None, last_alive: 0, stalled: false });
/// 最近一次写 `jobs.json` 时有没有任务在跑。自查线程靠它决定还要不要接着查
static RUNNING: AtomicBool = AtomicBool::new(false);
/// 自查线程在不在
static TICKING: AtomicBool = AtomicBool::new(false);

/// 现在按着没有。给诊断用。
pub fn holding() -> bool {
    STATE.lock().map(|s| s.since.is_some()).unwrap_or(false)
}

/// 系统里一个正在挡睡眠的进程（`pmset -g assertions` 里的一条）。
#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct SleepBlocker {
    pub pid: u32,
    /// 进程名，如 `caffeinate`、`Claude`
    pub process: String,
    /// 断言名，如 `caffeinate command-line tool`
    pub name: String,
    /// 还剩多少秒自动松手；`None` = 没有时限（跟着某个进程，或者一直按着）
    pub secs_left: Option<u64>,
    /// `caffeinate -w <pid>` 代哪个进程按着
    pub on_behalf_of: Option<u32>,
    /// 是 Anima 自己按的
    pub anima: bool,
}

/// 界面上「防休眠」那个标记要的东西。
///
/// 用户 2026-09-30：「我不知道 Anima 的防休眠进程生没生效，因为界面里没地方查看」。
/// 只报 Anima 自己的不够 —— 用户自己在终端开的 `caffeinate`、别的应用按着的也算数，一起列出来，
/// 才回答得了「今晚这台机器会不会睡」。
#[derive(Debug, Clone, serde::Serialize)]
pub struct KeepAwakeStatus {
    /// Anima 因为有任务在跑正按着
    pub anima_holding: bool,
    /// 从什么时候开始按的（秒）
    pub since: Option<u64>,
    /// 任务记录还说在跑，但连续 [`STALL_SECS`] 没有任何动静，松手了（多半挂住了；一有新进度会重新按住）
    pub stalled: bool,
    /// 最近一次看到任务还活着（秒）。从没看到过为 `None`
    pub last_alive: Option<u64>,
    /// 连续多久没动静才松手（秒）
    pub stall_secs: u64,
    /// 系统里所有在挡**系统睡眠**的（不含 powerd 自己「亮屏期间不睡」那条 —— 关屏就没了，不算数）
    pub blockers: Vec<SleepBlocker>,
}

const BLOCKING_KINDS: &[&str] = &["PreventSystemSleep", "PreventUserIdleSystemSleep", "NoIdleSleepAssertion"];

/// 解析 `pmset -g assertions` 的「Listed by owning process」那一段。纯函数，测试喂真实输出。
pub fn parse_assertions(text: &str, anima_pid: u32, anima_child: Option<u32>) -> Vec<SleepBlocker> {
    let mut out: Vec<SleepBlocker> = Vec::new();
    let mut cur: Option<usize> = None;
    let mut in_list = false;
    for line in text.lines() {
        if line.starts_with("Listed by owning process:") {
            in_list = true;
            continue;
        }
        if !in_list {
            continue;
        }
        // 下一个段落（Kernel Assertions 等）顶格开头
        if !line.starts_with(char::is_whitespace) && !line.trim().is_empty() {
            in_list = false;
            continue;
        }
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix("pid ") {
            cur = None;
            let Some((pid, rest)) = rest.split_once('(') else { continue };
            let Ok(pid) = pid.trim().parse::<u32>() else { continue };
            let Some((process, rest)) = rest.split_once("):") else { continue };
            if process == "powerd" || !BLOCKING_KINDS.iter().any(|k| rest.contains(k)) {
                continue;
            }
            let name = rest.split_once("named: \"").map(|(_, n)| n.trim_end().trim_end_matches('"').to_string()).unwrap_or_default();
            cur = Some(match out.iter().position(|b| b.pid == pid) {
                Some(i) => i,
                None => {
                    out.push(SleepBlocker { pid, process: process.to_string(), name, secs_left: None, on_behalf_of: None, anima: false });
                    out.len() - 1
                }
            });
        } else if let Some(i) = cur {
            if let Some(r) = t.strip_prefix("Timeout will fire in ") {
                if let Some(n) = r.split_whitespace().next().and_then(|n| n.parse::<u64>().ok()) {
                    out[i].secs_left = Some(out[i].secs_left.map_or(n, |m| m.min(n)));
                }
            }
            if let Some((_, r)) = t.split_once("on behalf of Process ID ") {
                out[i].on_behalf_of = r.split(|c: char| !c.is_ascii_digit()).next().and_then(|n| n.parse().ok());
            }
        }
    }
    for b in &mut out {
        b.anima = b.on_behalf_of == Some(anima_pid) || Some(b.pid) == anima_child;
    }
    out
}

/// 现在的防休眠状态。**读系统真实的断言**，不只看我们自己记的状态 —— 「记着按了」不等于子进程还活着。
#[tauri::command]
pub fn keep_awake_status() -> KeepAwakeStatus {
    let st = STATE.lock().map(|s| *s).unwrap_or_default();
    let child = {
        let mut held = match HELD.lock() {
            Ok(h) => h,
            Err(e) => e.into_inner(),
        };
        // 子进程被别人杀了的话，别再说「按着」
        match held.as_mut() {
            Some(c) => matches!(c.try_wait(), Ok(None)).then(|| c.id()),
            None => None,
        }
    };
    let text = std::process::Command::new("pmset")
        .args(["-g", "assertions"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();
    let blockers = parse_assertions(&text, std::process::id(), child);
    KeepAwakeStatus {
        anima_holding: st.since.is_some() && (child.is_some() || blockers.iter().any(|b| b.anima)),
        since: st.since,
        stalled: st.stalled,
        last_alive: (st.last_alive > 0).then_some(st.last_alive),
        stall_secs: STALL_SECS,
        blockers,
    }
}

/// 按任务队列的状态同步一次。**在每次写 `jobs.json` 之后调**。
///
/// 非 macOS 上只更新状态、不起进程 —— 这个项目以 Apple Silicon 为第一目标，
/// 别的平台没有等价的一行命令，留空比留一个半生不熟的实现好。
pub fn sync(running: bool) {
    RUNNING.store(running, Ordering::SeqCst);
    // 写 `jobs.json` 本身就是一次动静
    let now = crate::jobs::now();
    step(running, now, now);
    if running {
        ensure_ticker();
    }
}

/// 算一次、动一次手。`sync` 和自查线程共用
fn step(running: bool, now: u64, alive_at: u64) {
    let mut st = match STATE.lock() {
        Ok(s) => s,
        Err(e) => e.into_inner(),
    };
    let (next, act) = apply(running, *st, now, alive_at);
    *st = next;
    drop(st);
    match act {
        Action::Hold => start(),
        Action::Release => stop(running),
        Action::Nothing => {}
    }
}

/// ComfyUI 最近一条进度的时刻（秒）；没在听或一条都没有返回 0
fn comfy_alive_at(now: u64) -> u64 {
    crate::agent_runtime::comfy_watch::since_last_progress()
        .map(|d| now.saturating_sub(d.as_secs()))
        .unwrap_or(0)
}

/// 有任务在跑时每 [`TICK`] 自查一次。**不靠写 `jobs.json` 触发** —— 挂住的任务不会再写，
/// 只靠写文件触发的话永远发现不了（原来的上限就是这么失效的）。任务都跑完它自己退出。
fn ensure_ticker() {
    if TICKING.swap(true, Ordering::SeqCst) {
        return;
    }
    let spawned = std::thread::Builder::new().name("keep-awake-tick".into()).spawn(|| loop {
        std::thread::sleep(TICK);
        if !RUNNING.load(Ordering::SeqCst) {
            TICKING.store(false, Ordering::SeqCst);
            // 退出前的这一瞬间又有任务开跑、而它看到「还有人在查」没起新线程 —— 那就接着查
            if RUNNING.load(Ordering::SeqCst) && !TICKING.swap(true, Ordering::SeqCst) {
                continue;
            }
            return;
        }
        let now = crate::jobs::now();
        step(true, now, comfy_alive_at(now));
    });
    if spawned.is_err() {
        TICKING.store(false, Ordering::SeqCst);
        tracing::warn!("起不来防休眠的自查线程：任务挂住时不会自动松手");
    }
}

#[cfg(target_os = "macos")]
fn start() {
    let mut held = match HELD.lock() {
        Ok(h) => h,
        Err(e) => e.into_inner(),
    };
    if held.is_some() {
        return;
    }
    // `-w <我们自己的 pid>`：应用一没，它自己退出。见模块注释。
    let pid = std::process::id().to_string();
    match std::process::Command::new("caffeinate")
        .args(["-s", "-i", "-w", &pid])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
    {
        Ok(c) => {
            tracing::info!("有任务在跑，已阻止系统休眠（caffeinate -s -i）");
            *held = Some(c);
        }
        // 起不来不是致命的：最坏就是回到以前的行为（可能睡过去）
        Err(e) => tracing::warn!(error = %e, "起不来 caffeinate，出片期间机器可能会睡过去"),
    }
}

#[cfg(not(target_os = "macos"))]
fn start() {}

/// `stalled` = 因为太久没动静才松的手（不是因为任务跑完）。
fn stop(stalled: bool) {
    let mut held = match HELD.lock() {
        Ok(h) => h,
        Err(e) => e.into_inner(),
    };
    if let Some(mut c) = held.take() {
        let _ = c.kill();
        let _ = c.wait();
    }
    if stalled {
        tracing::warn!(
            hours = STALL_SECS / 3600,
            "任务记录还说在跑，但这么久没有任何进度，松手了 —— 去队列里看看是不是挂住了；一有新进度会重新按住"
        );
    } else {
        tracing::info!("任务跑完了，不再阻止系统休眠");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: HoldState = HoldState { since: None, last_alive: 0, stalled: false };

    #[test]
    fn 有任务才按住_跑完就松手() {
        let (st, act) = apply(true, NONE, 100, 100);
        assert_eq!(act, Action::Hold);
        assert_eq!(st.since, Some(100));

        // 还在跑：不重复起进程
        let (st, act) = apply(true, st, 200, 200);
        assert_eq!(act, Action::Nothing);
        assert_eq!(st.since, Some(100));

        let (st, act) = apply(false, st, 300, 300);
        assert_eq!(act, Action::Release);
        assert_eq!((st.since, st.stalled), (None, false));

        // 没任务也没按着：什么都不做
        assert_eq!(apply(false, st, 400, 400).1, Action::Nothing);
    }

    /// **2026-10-02 的回归。**《回收师》EP01 一集 32 镜要跑 40 多个小时，原来的「最多按住 24 小时」
    /// 在第 24 小时松了手 —— 任务好好地在跑，后半夜全靠用户自己开的 caffeinate。
    /// 只要一直有动静（ComfyUI 每 30 分钟左右一步进度），按多久都不松。
    #[test]
    fn 一直有动静就一直按着_按多久都不松手() {
        let (mut st, _) = apply(true, NONE, 0, 0);
        let half_hour = 1800;
        for i in 1..=(48 * 2) {
            let t = i * half_hour;
            let (next, act) = apply(true, st, t, t);
            assert_eq!(act, Action::Nothing, "第 {} 小时松手了", t / 3600);
            st = next;
        }
        assert_eq!(st.since, Some(0), "按住的起点被刷新了");
    }

    /// 任务记录说「在跑」不等于真在跑。挂住的话不能按着不放一整周 ——
    /// 但判据是**多久没动静**，不是按了多久。
    #[test]
    fn 太久没动静才松手_有了新动静重新按住() {
        let (st, _) = apply(true, NONE, 0, 0);
        // 差一秒不算到
        assert_eq!(apply(true, st, STALL_SECS - 1, 0).1, Action::Nothing);

        let (st, act) = apply(true, st, STALL_SECS, 0);
        assert_eq!(act, Action::Release);
        assert!(st.stalled);

        // 松手之后没有新动静：不能每次自查都按回去
        let (st, act) = apply(true, st, STALL_SECS + 600, 0);
        assert_eq!(act, Action::Nothing);
        assert!(st.stalled);

        // 又有进度了（比如 ComfyUI 只是一步特别慢）：重新按住
        let t = STALL_SECS + 1200;
        let (st, act) = apply(true, st, t, t);
        assert_eq!(act, Action::Hold);
        assert!(!st.stalled);
    }

    /// 旧的动静不能把「最近一次」往回拨 —— 自查时 ComfyUI 那边可能比刚写的 jobs.json 还旧
    #[test]
    fn 最近一次动静只往前走() {
        let (st, _) = apply(true, NONE, 1000, 1000);
        let (st, _) = apply(true, st, 2000, 500);
        assert_eq!(st.last_alive, 1000);
        // 所以按 1000 算，到 1000 + STALL 才松
        assert_eq!(apply(true, st, 1000 + STALL_SECS - 1, 0).1, Action::Nothing);
        assert_eq!(apply(true, st, 1000 + STALL_SECS, 0).1, Action::Release);
    }

    /// 这一轮任务跑完，`stalled` 要清掉，下一批照常按得住
    #[test]
    fn 任务跑完清掉挂住标记() {
        let (st, _) = apply(true, NONE, 0, 0);
        let (st, _) = apply(true, st, STALL_SECS, 0);
        assert!(st.stalled);
        let (st, act) = apply(false, st, STALL_SECS + 1, STALL_SECS + 1);
        assert_eq!(act, Action::Nothing, "本来就没按着，不用松手");
        assert!(!st.stalled, "stalled 没清掉 —— 界面会一直说「多半挂住了」");
        let t = STALL_SECS + 2;
        assert_eq!(apply(true, st, t, t).1, Action::Hold);
    }

    /// 自查线程靠这条拿 ComfyUI 的进度。它读成 0 的话，单镜出片的几个小时里就只剩 jobs.json 那条证据 ——
    /// 而那几个小时 jobs.json 一次都不写
    #[test]
    fn 自查读得到comfy最近一条进度() {
        use crate::agent_runtime::comfy_watch::record_for_test;
        record_for_test("keep-awake-test-old", Duration::from_secs(5 * 3600));
        record_for_test("keep-awake-test-new", Duration::from_secs(90));
        let now = 10_000_000;
        let at = comfy_alive_at(now);
        // 取的是所有任务里**最新**的那条（90 秒前），不是最旧的
        assert!(at >= now - 120 && at <= now, "{}", now - at);
    }

    /// 松手的等待和 ComfyUI「听不见进度时最多等多久」是同一个数，别各改各的
    #[test]
    fn 松手等待和comfy那边一致() {
        assert_eq!(STALL_SECS, crate::agent_runtime::comfy_watch::NO_SIGNAL_CEILING.as_secs());
        assert!(STALL_SECS > crate::agent_runtime::comfy_watch::STALL.as_secs(), "听得见进度时任务先被判失败、再松手");
        assert!(TICK.as_secs() * 6 <= STALL_SECS, "自查太稀，挂住后松手会拖太久");
    }

    /// 2026-09-30 本机 `pmset -g assertions` 的真实输出（截取）：一个 18 小时的手动 caffeinate、Anima 自己的（-w 12355）、
    /// Claude 的 Electron；powerd「亮屏期间不睡」那条不算（关屏就没了）
    #[test]
    fn 读得出系统里谁在挡睡眠() {
        let text = "Assertion status system-wide:\n   PreventSystemSleep             1\nListed by owning process:\n   pid 70392(Claude): [0x00127b2900018e83] 00:00:16 NoIdleSleepAssertion named: \"Electron\"  \n   pid 13988(caffeinate): [0x00127b5300018e88] 00:00:01 PreventUserIdleSystemSleep named: \"caffeinate command-line tool\"  \n\tDetails: caffeinate asserting for 64800 secs\n\tTimeout will fire in 64799 secs Action=TimeoutActionRelease\n   pid 13988(caffeinate): [0x00127b5300078e88] 00:00:01 PreventSystemSleep named: \"caffeinate command-line tool\"  \n\tTimeout will fire in 64799 secs Action=TimeoutActionRelease\n   pid 110(powerd): [0x000f3a6100018a3c] 59:13:28 PreventUserIdleSystemSleep named: \"Powerd - Prevent sleep while display is on\"  \n   pid 169(WindowServer): [0x0000001900098087] 00:00:03 UserIsActive named: \"com.apple.iohideventsystem.queue.tickle\"  \n   pid 13766(caffeinate): [0x00127ada00018e55] 00:01:35 PreventUserIdleSystemSleep named: \"caffeinate command-line tool\"  \n\tDetails: caffeinate asserting on behalf of Process ID 12355\n\tCreated for PID: 12355. \nKernel Assertions: 0x4=USB\n   id=500  level=255 0x4=USB mod=10/1/26, 9:00 AM description=com.apple.usb.externaldevice.14000000 owner=Mac Studio\n";
        let got = parse_assertions(text, 12355, None);
        let pids: Vec<u32> = got.iter().map(|b| b.pid).collect();
        assert_eq!(pids, vec![70392, 13988, 13766], "powerd、WindowServer（UserIsActive）不算，一个进程只列一条：{got:?}");
        let manual = &got[1];
        assert_eq!((manual.secs_left, manual.on_behalf_of, manual.anima), (Some(64799), None, false));
        let ours = &got[2];
        assert_eq!((ours.secs_left, ours.on_behalf_of, ours.anima), (None, Some(12355), true));
        assert_eq!(got[0].process, "Claude");
        // 子进程 pid 对得上也算 Anima 的
        assert!(parse_assertions(text, 1, Some(13766)).iter().any(|b| b.pid == 13766 && b.anima));
    }

    /// 读本机此刻的防休眠状态（`pmset -g assertions`），打印出来对一眼。只读
    #[test]
    #[ignore = "读本机系统状态"]
    fn 本机防休眠状态() {
        for b in keep_awake_status().blockers {
            println!("{b:?}");
        }
    }
}
