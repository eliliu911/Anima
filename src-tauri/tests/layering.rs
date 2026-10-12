//! 分层约束：**谁能依赖谁**。
//!
//! ## 为什么要用测试守
//!
//! 反向依赖是悄悄长出来的：写 `playback` 时顺手用了 `tts` 里的 WAV 封装、
//! 加口型时顺手引了 `avatar::lipsync::FRAME` —— 每一次都只是一行，
//! 每一次都能编译，累积起来就成了一团谁也说不清方向的东西。
//!
//! 这条测试只查一件事：**音频层不许依赖任何上层**。
//! 它是整条链路的地基（采集、播放、VAD、WAV），一旦它反过来依赖
//! TTS 或面容，就没法单独测、也没法单独换。
//!
//! 上层之间的依赖是允许的（`voice` 是编排层，依赖所有人是它的职责）。

use std::path::Path;

/// 音频层不许依赖的模块
const FORBIDDEN_FOR_AUDIO: &[&str] = &["crate::tts", "crate::avatar", "crate::voice", "crate::stt"];

fn rust_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(rust_files(&p));
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
    out
}

#[test]
fn the_audio_layer_depends_on_nothing_above_it() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/audio");
    let mut violations = Vec::new();

    for file in rust_files(&root) {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        for (n, line) in text.lines().enumerate() {
            // 注释里提到上层是可以的（"消费方目前是 avatar::lipsync"），
            // 只查真正的代码引用
            let code = line.split("//").next().unwrap_or("");
            for bad in FORBIDDEN_FOR_AUDIO {
                if code.contains(bad) {
                    violations.push(format!(
                        "{}:{} 依赖了 {bad}\n    {}",
                        file.file_name().unwrap().to_string_lossy(),
                        n + 1,
                        line.trim()
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "音频层是地基，不能反过来依赖上层。\n\
         需要上层的某个常量/函数时，把它挪下来或者在音频层自己定义。\n\n{}",
        violations.join("\n")
    );
}

/// **语音会话的引擎必须走形态解析，不能读全局配置。**
///
/// 这条丢过一次，代价很大：`voice_start` 里写的是 `current_tts(&state)`，
/// 它读全局 `tts_engine`（Edge），而音色来自当前角色（`ADSister`，
/// GPT-SoVITS 的音色库路径）。于是 Edge 收到一个不认识的音色名，
/// 回一句 "standard voices will no longer be supported" ——
/// **报错内容和真正的问题（引擎与音色不是一家）完全对不上**，
/// 顺着那句报错查会一路查到微软的文档上去。
///
/// 会话启动和运行中切换必须得出同一个答案。各算各的迟早在某个组合上
/// 分叉，而那种分叉的症状是"重启之后才对"。
#[test]
fn voice_start_resolves_the_engine_through_the_persona() {
    let src = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands_voice.rs"),
    )
    .expect("读不到 commands_voice.rs");

    // 取 voice_start 函数体：从签名到下一个 #[tauri::command]
    let start = src
        .find("pub async fn voice_start")
        .expect("找不到 voice_start");
    let body_end = src[start..]
        .find("#[tauri::command]")
        .map(|i| start + i)
        .unwrap_or(src.len());
    let body = &src[start..body_end];

    assert!(
        body.contains("resolve_persona"),
        "voice_start 必须走 resolve_persona 取引擎与音色"
    );
    // 只看代码，不看注释 —— 上面那段说明里就提到了 `current_tts`
    let code: String = body
        .lines()
        .map(|l| l.split("//").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !code.contains("current_tts("),
        "voice_start 不能用 current_tts —— 那读的是全局引擎，\
         会和角色绑定的音色错配"
    );
}
