//! 守住「族裔词补齐」不被绕过。
//!
//! 这是一条**读源码**的测试，因为要守的东西没法从行为上测：
//! `cinema.rs` 里四档各自把 `trigger_words` 传进 prompt，将来加第五档时
//! 顺手写成 `&req.trigger_words` 完全自然、编译也过，
//! 只是那一档出来的人会悄悄变成欧美脸 —— 没有任何报错。
//!
//! 实测记录见 `cinema_sheet::ensure_ethnicity` 的文档注释。

use std::path::Path;

#[test]
fn 生成路径只在一处读原始触发词() {
    let src = std::fs::read_to_string(Path::new("src/cinema.rs")).expect("读不到 cinema.rs");
    let direct = src.matches("req.trigger_words").count();
    assert_eq!(
        direct, 1,
        "`req.trigger_words` 只该出现在 ensure_ethnicity 那一行；\
         其余地方都要用补过族裔词的 `trigger`。现在有 {direct} 处"
    );
    assert!(
        src.contains("ensure_ethnicity(&req.trigger_words)"),
        "那唯一一处必须是 ensure_ethnicity 的入参"
    );
}

/// 四档都得吃到补过的那份，而不是只有三视图。
#[test]
fn 四档都用补过的触发词() {
    let src = std::fs::read_to_string(Path::new("src/cinema.rs")).expect("读不到 cinema.rs");
    let used = src.matches("&trigger,").count();
    assert!(
        used >= 4,
        "三视图/表情/姿势/面部机位四档都要用，现在只有 {used} 处"
    );
}
