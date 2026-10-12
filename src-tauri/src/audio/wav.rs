//! WAV 容器的读写。
//!
//! **放在音频层而不是 TTS 层**：给裸 PCM 包个头和"文字怎么变成声音"
//! 没有关系。原先它在 `tts/mod.rs` 里，于是播放层为了用它反过来依赖了
//! TTS —— 播放不该知道 TTS 的存在。

/// 给裸 PCM（16 位小端）包一个 WAV 头。
///
/// CosyVoice2 / IndexTTS 这类服务返回的是没有头的采样点。不包头的话
/// rodio 解不出来 —— 播放直接失败，而不是"音质差一点"。
pub fn wrap_pcm_s16le(pcm: &[u8], sample_rate: u32, channels: u16) -> Vec<u8> {
    let bits = 16u16;
    let byte_rate = sample_rate * channels as u32 * (bits as u32 / 8);
    let block_align = channels * bits / 8;
    // 采样点必须成对，奇数长度说明尾部被截断了一半，丢掉那个字节，
    // 否则最后一个采样会是噪声（听起来是一声爆音）
    let data = &pcm[..pcm.len() - pcm.len() % block_align as usize];

    let mut out = Vec::with_capacity(44 + data.len());
    out.extend(b"RIFF");
    out.extend(((36 + data.len()) as u32).to_le_bytes());
    out.extend(b"WAVE");
    out.extend(b"fmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes()); // PCM
    out.extend(channels.to_le_bytes());
    out.extend(sample_rate.to_le_bytes());
    out.extend(byte_rate.to_le_bytes());
    out.extend(block_align.to_le_bytes());
    out.extend(bits.to_le_bytes());
    out.extend(b"data");
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(data);
    out
}
