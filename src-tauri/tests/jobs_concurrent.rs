//! `jobs.json` 的跨进程并发。**这一条必须真开多个写入者才算验过。**
//!
//! 应用本体和 `anima-mcp` 是两个进程，都往同一份 `jobs.json` 里写。
//! 进程内的 `Mutex` 管不住它们 —— 最坏的后果不是「少一条记录」，是
//! `start_exclusive` 看不见对方正在跑的任务，**于是重复发起一轮渲染**。

use std::process::Command;

/// 用真的子进程并发写，看有没有互相盖掉。
///
/// 每个子进程登记 N 个任务；全部结束后总数必须等于 进程数 × N。
/// 没有跨进程锁的话这个数会少 —— 后写的把先写的整份盖掉。
///
/// **进程数和次数是调出来的，不是随手写的。** 第一版 4×5 太少，
/// 读-改-写的窗口没重叠上：**把锁停掉它照样过** —— 假绿测试比没有测试更糟。
/// 8×20 才稳定地把竞争暴露出来（变异验证：停掉 flock 掉到个位数）。
///
/// **总数必须压在 `MAX_JOBS`（200）以下** —— 超了会被裁剪，
/// 那时断言挂掉是因为裁剪、不是因为覆盖，等于又验错了东西。
#[test]
fn 多个进程一起写不会互相盖掉() {
    let d = tempfile::tempdir().unwrap();
    let cfg = d.path().to_path_buf();

    // 用当前测试二进制自己当子进程：带上环境变量就只跑那段写入逻辑
    let exe = std::env::current_exe().unwrap();
    let kids: Vec<_> = (0..8)
        .map(|i| {
            Command::new(&exe)
                .arg("--exact")
                .arg("写入者")
                .arg("--ignored")
                .env("JOBS_WRITER", i.to_string())
                .env("JOBS_DIR", &cfg)
                .spawn()
                .expect("起不了子进程")
        })
        .collect();
    for mut k in kids {
        assert!(k.wait().unwrap().success(), "子进程失败了");
    }

    let log = anima_lib::jobs::load(&cfg);
    assert_eq!(log.jobs.len(), 8 * 20, "总数对不上 —— 有进程的写入被盖掉了：{}", log.jobs.len());
}

/// 子进程真正干的活。**默认 `#[ignore]`**，只由上面那个测试带环境变量拉起来。
#[test]
#[ignore = "由「多个进程一起写不会互相盖掉」拉起"]
fn 写入者() {
    let Ok(who) = std::env::var("JOBS_WRITER") else { return };
    let dir = std::path::PathBuf::from(std::env::var("JOBS_DIR").unwrap());
    for n in 0..20 {
        anima_lib::jobs::start_exclusive(
            &dir,
            anima_lib::jobs::JobKind::Frame,
            &format!("proj_{who}"),
            "ep_01",
            &format!("shot_{who}_{n}"),
            n,
            "并发写入测试",
        )
        .expect("登记失败");
    }
}
