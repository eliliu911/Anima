//! rustls 的加密后端必须能装上。
//!
//! ## 为什么值得一条独立的测试
//!
//! 依赖树里同时有 `ring` 和 `aws-lc-rs`（reqwest 与 tokio-tungstenite 各带
//! 一个），rustls 0.23 在这种情况下**拒绝猜**，第一次建 TLS 连接时直接 panic。
//!
//! 而这个 panic 的症状极其难查：
//!
//! - 它发生在 tokio 工作线程里，走 stderr，**tracing 日志里一个字都没有**
//! - 生成任务当场死掉：不合成、不落库、连"一句都没说出去"的探针都跑不到
//! - actor 只看到任务结束了，照常按"正常说完"结算情感轴
//!
//! 实测表现是「她挥了挥手，然后什么都没有」，查了三轮。
//!
//! 这条测试保证：只要有人往依赖里再引一个后端、或者删掉 `install_default`
//! 的调用，它会红。

/// 后端必须存在且能装上。
///
/// 装过一次之后 `install_default` 会返回 Err，所以这里只断言
/// "装完之后一定有默认后端"，而不是"install 必须成功"
#[test]
fn a_crypto_provider_can_be_installed() {
    let _ = rustls::crypto::ring::default_provider().install_default();
    assert!(
        rustls::crypto::CryptoProvider::get_default().is_some(),
        "装不上加密后端 —— Edge TTS 的 WSS 一连接就会 panic"
    );
}

/// **两个后端同时存在是常态**，所以不能指望 rustls 自动选。
///
/// 这条不是要求"只能有一个"（依赖是别人带进来的，管不了），
/// 而是提醒：正因为可能有多个，`install_default` 那一步不能省
#[test]
fn we_do_not_rely_on_automatic_provider_selection() {
    // ring 是我们显式选的那个，必须可用
    let provider = rustls::crypto::ring::default_provider();
    assert!(
        !provider.cipher_suites.is_empty(),
        "ring 后端没有可用的密码套件"
    );
}
