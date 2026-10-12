//! 媒体请求的工作线程池。**固定几个线程，不是一个请求一个线程。**
//!
//! # 为什么需要它
//!
//! 2026-09-18 的崩溃报告直接指出了问题。用户在组装页反复切分集，
//! 崩溃时的线程快照是这样的：
//!
//! ```text
//! Thread 25 (崩溃)  ← abort，走的是媒体请求这条路
//! Thread 26  WTF::callOnMainRunLoopAndWait ← -[WKURLSchemeTaskImpl didFinish]
//! Thread 27  WTF::callOnMainRunLoopAndWait ← -[WKURLSchemeTaskImpl didReceiveData:]
//! Thread 28  WTF::callOnMainRunLoopAndWait ← -[WKURLSchemeTaskImpl didReceiveData:]
//! Thread 29  WTF::callOnMainRunLoopAndWait ← -[WKURLSchemeTaskImpl didReceiveData:]
//! ```
//!
//! **四个媒体线程同时卡在「同步等主线程」上**，第五个崩了。
//!
//! 原因是 `responder.respond()` 内部会调 WebKit 的 `didReceiveData:` /
//! `didFinish`，而它们**每次都要同步往返主线程一次**
//! （`getExceptionTypeFromMainRunLoop`）。切一次分集，视频 + 封面 +
//! 六张缩略图的请求成批涌进来，原来的写法是**每个请求 `std::thread::spawn`
//! 一个线程，没有任何上限**，于是十几个线程一起抢主线程。
//!
//! # 为什么限制并发不会更慢
//!
//! 那些线程最终都在主线程上排队 —— 主线程只有一个，怎么并发都得串行过。
//! 所以限制并发不减少吞吐，只是**不再为每个请求付一个线程的代价**，
//! 也不会在快速切页时把线程数顶到几十个。
//!
//! # 这里不判断「请求还有没有效」
//!
//! 那是 wry 的事（它每步都 `check_task_is_valid` + `objc2::exception::catch`）。
//! 这里只负责别把线程用爆。

use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock};

/// 同时最多几个媒体请求在处理。
///
/// **4 是按「主线程是瓶颈」定的**：再多也只是在主线程门口多排几个，
/// 而每多一个就多一个线程。太少的话一段大视频的 Range 请求会挡住缩略图。
pub const WORKERS: usize = 4;

type Job = Box<dyn FnOnce() + Send + 'static>;

fn sender() -> &'static Sender<Job> {
    static S: OnceLock<Sender<Job>> = OnceLock::new();
    S.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Job>();
        let rx = std::sync::Arc::new(Mutex::new(rx));
        for i in 0..WORKERS {
            let rx = std::sync::Arc::clone(&rx);
            let _ = std::thread::Builder::new()
                .name(format!("anima-media-{i}"))
                .spawn(move || {
                    loop {
                        // **锁只用来取一个任务，不覆盖执行。**
                        // 覆盖执行的话四个工人会互相串行，等于只有一个
                        let job = { rx.lock().unwrap_or_else(|e| e.into_inner()).recv() };
                        match job {
                            Ok(job) => job(),
                            Err(_) => break, // 发送端没了，退出
                        }
                    }
                });
        }
        tx
    })
}

/// 把一个媒体请求交给池子。
///
/// **队列满不了**（无界通道），所以这里不会阻塞调用方 ——
/// 调用方是 webview 的协议回调，阻塞它等于卡住整个界面。
pub fn submit(job: impl FnOnce() + Send + 'static) {
    let _ = sender().send(Box::new(job));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// 提交的任务**每一个都要跑到**。丢任务的表现是「某张图永远转圈」。
    #[test]
    fn 提交的任务都会执行() {
        let done = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = mpsc::channel::<()>();
        for _ in 0..50 {
            let done = Arc::clone(&done);
            let tx = tx.clone();
            submit(move || {
                done.fetch_add(1, Ordering::SeqCst);
                let _ = tx.send(());
            });
        }
        drop(tx);
        for _ in 0..50 {
            rx.recv_timeout(std::time::Duration::from_secs(10)).expect("有任务没跑完");
        }
        assert_eq!(done.load(Ordering::SeqCst), 50);
    }

    /// **并发数不能超过 `WORKERS`。**
    ///
    /// 这正是崩溃报告里出问题的地方：原来一个请求一个线程，
    /// 快速切页时十几个线程一起去抢主线程。
    #[test]
    fn 并发数不超过上限() {
        let now = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = mpsc::channel::<()>();
        for _ in 0..40 {
            let (now, peak, tx) = (Arc::clone(&now), Arc::clone(&peak), tx.clone());
            submit(move || {
                let n = now.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(n, Ordering::SeqCst);
                // 停一下，制造重叠 —— 不停的话每个任务瞬间结束，量不出并发
                std::thread::sleep(std::time::Duration::from_millis(20));
                now.fetch_sub(1, Ordering::SeqCst);
                let _ = tx.send(());
            });
        }
        drop(tx);
        for _ in 0..40 {
            rx.recv_timeout(std::time::Duration::from_secs(20)).expect("有任务没跑完");
        }
        let p = peak.load(Ordering::SeqCst);
        assert!(p <= WORKERS, "并发到了 {p}，超过上限 {WORKERS}");
        assert!(p > 1, "峰值只有 {p} —— 池子退化成单线程了，那会让一段大视频挡住所有缩略图");
    }

    /// 任务 panic 不能把工人线程带走 —— 带走的话池子会越来越小，最后全死。
    ///
    /// **调用方负责 `catch_unwind`**（见 `lib.rs` 的媒体协议处理器），
    /// 这里验证的是「就算真的漏出来，后面的任务还跑得了」。
    #[test]
    fn 一个任务炸了不影响后面的() {
        let (tx, rx) = mpsc::channel::<()>();
        for i in 0..12 {
            let tx = tx.clone();
            submit(move || {
                let r = std::panic::catch_unwind(|| {
                    if i % 3 == 0 {
                        panic!("故意的");
                    }
                });
                let _ = r;
                let _ = tx.send(());
            });
        }
        drop(tx);
        for _ in 0..12 {
            rx.recv_timeout(std::time::Duration::from_secs(10)).expect("炸完之后池子停了");
        }
    }
}
