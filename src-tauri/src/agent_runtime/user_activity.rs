//! 「用户是不是正在等」的进程级信号。
//!
//! 这台机器上两个模型档位指向**同一个** LM Studio 实例
//! （`realtime_provider` 和 `background_provider` 都是 lmstudio/gpt-oss-120b；
//! ollama 里只有记忆层用的 `bge-m3` 嵌入模型，没有对话模型）。
//! 所以靠档位路由把负载分开是做不到的 —— 子智能体和语音管线、主会话
//! 争的是同一个推理实例。
//!
//! `MAX_CONCURRENT_SUBAGENTS` 管的是子智能体**之间**的并发，
//! 对「子智能体 vs 用户正在进行的对话」完全没有作用：
//! 一个 4 轮带长上下文的子智能体推理跑着时，用户对数字人说的下一句话
//! 就排在它后面，而这个体验退化**不报任何错**。
//!
//! 这里给用户交互一条优先通道：Realtime 档的流在飞时（语音管线和主会话走的都是这一档），
//! 子智能体在**轮次之间**让路。让路只发生在轮与轮的缝隙 ——
//! 已经发出去的那一次推理不会被打断，那是模型服务端的事。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use tokio::sync::Notify;

static REALTIME_INFLIGHT: AtomicUsize = AtomicUsize::new(0);
static IDLE: OnceLock<Notify> = OnceLock::new();

fn idle() -> &'static Notify {
    IDLE.get_or_init(Notify::new)
}

/// 持有期间表示「用户正在等一个 Realtime 回复」。drop 时自动归还。
///
/// 做成 RAII 是因为流可能在任何一步提前结束 —— 取消、报错、上游断开、
/// 或者干脆被调用方丢掉不再 poll。手动配对 begin/end 一定会漏，
/// 而漏一次的后果是子智能体从此永远让路。
pub struct RealtimeGuard(());

impl Drop for RealtimeGuard {
    fn drop(&mut self) {
        if REALTIME_INFLIGHT.fetch_sub(1, Ordering::SeqCst) == 1 {
            idle().notify_waiters();
        }
    }
}

/// 摸这个计数器的测试之间的串行锁。
///
/// 计数器是**进程级**的（一个应用只有一套用户交互），并行跑的测试会互相
/// 把对方的前提打掉。锁必须放在模块级而不是某个 `mod tests` 里 ——
/// 因为碰它的测试**跨模块**：`user_activity` 有两条，`subagent` 还有一条
/// （`a_subagent_yields_while_the_user_is_waiting`）。
///
/// 我第一版只锁了本模块那两条，全量跑时被 `subagent` 那条撞出
/// 「测试开始时该是空闲的：left 1, right 0」—— 单跑通、全量挂，典型的漏网。
#[cfg(test)]
static SERIAL: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();

#[cfg(test)]
pub(crate) async fn exclusive_for_test() -> tokio::sync::MutexGuard<'static, ()> {
    SERIAL
        .get_or_init(|| tokio::sync::Mutex::new(()))
        .lock()
        .await
}

/// 登记一次 Realtime 推理开始。由 `Gateway::chat_stream` 调用。
pub fn begin_realtime() -> RealtimeGuard {
    REALTIME_INFLIGHT.fetch_add(1, Ordering::SeqCst);
    RealtimeGuard(())
}

pub fn realtime_inflight() -> usize {
    REALTIME_INFLIGHT.load(Ordering::SeqCst)
}

/// 等到用户那边空下来，**最多等 `max_wait`**。
///
/// 上限不是保守，是必需：万一有一条 Realtime 流因为某种原因没能正常注销
/// （被丢弃且从未 poll 到终止项之类），没有上限就是死锁 ——
/// 子智能体会永远停在轮次之间，而且不报任何错。
/// 有了上限，最坏情况只是退回到「和用户抢」，也就是加这条通道之前的行为。
pub async fn yield_to_user(max_wait: Duration) {
    if realtime_inflight() == 0 {
        return;
    }
    let _ = tokio::time::timeout(max_wait, async {
        while realtime_inflight() > 0 {
            // 先登记再复查，避免 notify 落在 await 之前的窗口里。
            let waiter = idle().notified();
            if realtime_inflight() == 0 {
                break;
            }
            waiter.await;
        }
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;


    /// 回归：有 Realtime 在飞时必须真的等；放开后立刻放行。
    #[tokio::test]
    async fn subagents_wait_while_the_user_is_waiting() {
        let _s = exclusive_for_test().await;
        assert_eq!(realtime_inflight(), 0, "测试开始时该是空闲的");

        let guard = begin_realtime();
        let waited = tokio::time::timeout(
            Duration::from_millis(300),
            yield_to_user(Duration::from_secs(30)),
        )
        .await;
        assert!(waited.is_err(), "用户还在等的时候，子智能体不该被放行");

        drop(guard);
        let released = tokio::time::timeout(
            Duration::from_millis(500),
            yield_to_user(Duration::from_secs(30)),
        )
        .await;
        assert!(released.is_ok(), "用户空下来之后该立刻放行");
        assert_eq!(realtime_inflight(), 0, "guard drop 之后计数该归零");
    }

    /// 回归：等待必须有上限 —— 否则一条没能正常注销的流会把子智能体永远卡死。
    #[tokio::test]
    async fn waiting_is_bounded_so_a_stuck_stream_cannot_deadlock() {
        let _s = exclusive_for_test().await;
        let leaked = begin_realtime();
        let started = std::time::Instant::now();
        yield_to_user(Duration::from_millis(200)).await;
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "等待没有上限，一条卡住的流就会造成死锁"
        );
        drop(leaked);
    }
}
