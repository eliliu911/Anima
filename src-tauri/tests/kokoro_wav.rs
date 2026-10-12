//! 用**真实的 Kokoro 输出**验证播放路径能解码。
//!
//! 需要 Kokoro 在跑，所以默认 #[ignore]。加它的理由：单元测试用的是我拼的
//! WAV，而真实服务的 header 布局（fmt → LIST → data，两处长度都是 0xFFFFFFFF）
//! 是我事后才发现的 —— 拼出来的样本永远不会替我发现下一个意外。

use std::io::Cursor;

#[tokio::test]
#[ignore = "需要 Kokoro 在 127.0.0.1:8880 运行"]
async fn real_kokoro_wav_decodes_after_header_fix() {
    let body = serde_json::json!({
        "model": "tts-1",
        "input": "测试一下解码。",
        "voice": "zf_xiaoyi",
        "response_format": "wav",
        "speed": 1.0
    });
    let bytes = reqwest::Client::new()
        .post("http://127.0.0.1:8880/v1/audio/speech")
        .json(&body)
        .send()
        .await
        .expect("Kokoro 没响应")
        .bytes()
        .await
        .expect("读取音频失败")
        .to_vec();

    assert!(bytes.len() > 1000, "音频太短，可能不是有效响应");
    assert_eq!(&bytes[0..4], b"RIFF", "应当是 WAV");

    // 未修正时 rodio 必然失败 —— 把这个前提也断言下来，
    // 否则将来 Kokoro 改了行为，这个测试会变成永远通过的空测试
    let declared = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    let raw_fails = rodio::Decoder::new(Cursor::new(bytes.clone())).is_err();
    if declared == u32::MAX {
        assert!(raw_fails, "占位长度的 WAV 本应解码失败，说明前提变了");
    }

    // 走应用真正用的那条路
    let samples = anima_lib::audio::playback::decode_for_test(bytes).expect("修正头之后应当能解码");
    assert!(samples > 0, "应当解出采样点");
    println!("解出 {samples} 个采样点");
}
