//! 打真实 ComfyUI 的精度体检。默认 `#[ignore]`，需要本机 ComfyUI 在跑。
//!
//! 存在的理由：`running_argv()` 依赖 ComfyUI `/system_stats` 里的 `argv` 字段。
//! 那个字段是 ComfyUI 自己的实现细节，某次升级把它改名或去掉，
//! 精度体检就会**静默退化成「ComfyUI 没在跑」**，什么都不报。
//! 这条测试是唯一能发现那件事的地方。
//!
//! ```bash
//! cargo test --test comfy_precision_live -- --ignored --nocapture
//! ```

#[tokio::test]
#[ignore = "需要本机 ComfyUI 在 127.0.0.1:8188"]
async fn 能从跑着的实例读到命令行并判出精度() {
    let argv = anima_lib::comfyui::running_argv()
        .await
        .expect("ComfyUI 没在跑，或 /system_stats 不再暴露 argv");
    println!("  argv = {argv:?}");

    assert!(
        argv.iter().any(|a| a.contains("main.py")),
        "argv 看着不像 ComfyUI 的命令行：{argv:?}"
    );

    let input = anima_lib::diagnostics::Input {
        comfy_argv: Some(argv.clone()),
        ..Default::default()
    };
    let check = anima_lib::diagnostics::checks(&input)
        .into_iter()
        .find(|c| c.id == "comfy_precision")
        .expect("argv 拿到了就该有这一项");
    println!("  体检结果 = {} / {:?}", check.value, check.level);

    assert_eq!(
        check.level,
        anima_lib::diagnostics::Level::Ok,
        "这个实例没带 --fp32-unet，FLUX 会静默出黑图。重启时加上它。"
    );
}
