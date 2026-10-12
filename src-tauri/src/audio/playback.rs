//! TTS 音频播放队列。
//!
//! 这是 barge-in 的另一半：光取消 LLM 流没用，**已经排队待播的音频必须能立刻
//! 丢掉**，否则用户打断之后还会继续听到上一轮的后半句
//! （ARCHITECTURE.md 坑点 2）。
//!
//! rodio 的 `Sink` 正好提供 `clear()`/`stop()`，所以队列直接交给它管，
//! 我们只负责把 TTS 产出的 MP3 分片依次塞进去。

use std::io::Cursor;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rodio::source::Source;
use rodio::{Decoder, OutputStream, Sink};

/// 播放线程向外报的信号。
///
/// 电平和「开始出声」走**同一条 channel**，不是两条。这一点是刻意的：
/// 对口型要把词边界时间戳对齐到真实的出声时刻，如果 `Started` 走另一条
/// channel，它可能排在同一句的电平后面到达 —— 那样嘴型会比声音晚，
/// 而这种偏差**听不出来只看得出来**，最难查。同一条 channel 天然保序。
/// 一帧分析用采样的长度。
///
/// **播放层不该知道消费方要拿它干什么**，所以这个常量定义在这里，
/// 而不是从 `avatar::lipsync` 反向引进来 —— 那会让播放依赖面容。
///
/// 取 1024 是因为下游要做 FFT，需要 2 的幂。两边必须一致，
/// `avatar::lipsync` 那边有一条测试盯着这件事。
pub const ANALYSIS_FRAME: usize = 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum PlaybackSignal {
    /// 一帧原始采样。消费方拿去做什么，播放层不关心 ——
    /// 目前的消费方是音频驱动的口型（`avatar::lipsync`）。
    ///
    /// **只有没有句子 id 的音频才会发** —— 有 id 的说明它走的是文本时间线，
    /// 那条精度更高（15 个 viseme、含辅音），不该被音频这条粗的覆盖掉
    Frame(Vec<f32>, u32),
    /// 输出电平（RMS 0..1），约 30ms 一次
    Level(f32),
    /// 带这个 id 的音频**真正开始出声了**。
    ///
    /// 为什么必须由播放侧报、而不是在合成完成时报：句子是排队播的，
    /// 第 N+1 句合成完的时候第 N 句往往还在播。合成时刻和出声时刻能差好几秒，
    /// 拿合成时刻当 t=0 去对口型，嘴型会整句错开。
    Started(u64),
}

/// 透传采样、顺带算 RMS 的 Source 包装器。
///
/// 为什么需要它：界面上「AI 说话」那条音谱线要真实的输出电平，而音频是在
/// Rust 侧解码播放的，前端拿不到采样点。rodio 没有现成的电平表，
/// 所以在解码器和 Sink 之间插一层。顺带也是**唯一**能知道「这一句开始出声了」
/// 的位置 —— 第一个采样被取走的那一刻。
///
/// **不能在这里做任何会阻塞的事**：它跑在音频回调路径上，一次卡顿就是一声爆音。
/// 所以用无界 channel 的 `send`（不阻塞），发送失败直接忽略 —— 接收端没了
/// 只意味着界面不再关心电平，不该影响出声。
struct Metered<S> {
    inner: S,
    tx: std::sync::mpsc::Sender<PlaybackSignal>,
    /// 平方和与计数，凑满一窗就发一次
    acc: f32,
    count: u32,
    window: u32,
    /// 攒够一帧就发出去做口型分析。**只在没有句子 id 时攒** ——
    /// 有 id 的走文本时间线，那条更准
    frame: Vec<f32>,
    analyze: bool,
    rate: u32,
    /// 还没报过「开始」的 cue id。第一个采样取走时发出去并置空。
    ///
    /// 放在 `next()` 里而不是 `new()` 里：包装器是**入队时**构造的，
    /// 那时候前面的句子还在播，此刻报「开始」就等于报了合成时刻。
    cue: Option<u64>,
}

impl<S> Metered<S>
where
    S: Source<Item = f32>,
{
    fn new(inner: S, tx: std::sync::mpsc::Sender<PlaybackSignal>, cue: Option<u64>) -> Self {
        // 约 30ms 一窗：再短了界面跟不上（也没必要），再长了谱线会显得迟钝
        let window = (inner.sample_rate() * inner.channels() as u32 / 33).max(64);
        let rate = inner.sample_rate();
        Self {
            analyze: cue.is_none(),
            frame: Vec::with_capacity(ANALYSIS_FRAME),
            rate,
            inner,
            tx,
            acc: 0.0,
            count: 0,
            window,
            cue,
        }
    }
}

impl<S> Iterator for Metered<S>
where
    S: Source<Item = f32>,
{
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let sample = self.inner.next()?;
        // 拿到第一个采样才算「开始出声」。放在 `?` 之后：解码器一上来就返回
        // None（空音频）的话不该报开始，否则前端会去对一段不存在的声音
        if let Some(id) = self.cue.take() {
            let _ = self.tx.send(PlaybackSignal::Started(id));
        }
        if self.analyze {
            self.frame.push(sample);
            if self.frame.len() >= ANALYSIS_FRAME {
                // 发不出去就丢掉，绝不能在音频线程上等
                let _ = self.tx.send(PlaybackSignal::Frame(
                    std::mem::take(&mut self.frame),
                    self.rate,
                ));
                self.frame.reserve(ANALYSIS_FRAME);
            }
        }
        self.acc += sample * sample;
        self.count += 1;
        if self.count >= self.window {
            let rms = (self.acc / self.count as f32).sqrt();
            self.acc = 0.0;
            self.count = 0;
            // 发不出去就算了，绝不能在音频线程上等
            let _ = self.tx.send(PlaybackSignal::Level(rms));
        }
        Some(sample)
    }
}

impl<S> Source for Metered<S>
where
    S: Source<Item = f32>,
{
    fn current_frame_len(&self) -> Option<usize> {
        self.inner.current_frame_len()
    }
    fn channels(&self) -> u16 {
        self.inner.channels()
    }
    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        self.inner.total_duration()
    }
}

use crate::error::{Error, Result};

/// TTS 播放器。
///
/// **为什么把音频流关在一个专属线程里**：rodio 的 `OutputStream` 内部持有绑定
/// 音频设备的裸指针，既不是 `Send` 也不是 `Sync`，所以任何持有它的结构都没法
/// 跨 tokio 任务传递 —— 而语音管线恰恰要在多个任务里入队音频。
///
/// 解法不是给它硬加 `unsafe impl Send`（那会把「不能跨线程」的真实约束掩盖成
/// 未定义行为），而是让 `OutputStream` 在它自己的线程上原地待着不动，只把
/// `Sink` 交出来 —— `Sink` 内部是 Arc 共享的，本身就是 `Send + Sync`。
pub struct Player {
    sink: Arc<Sink>,
    /// 是否正在出声。给 VAD 侧做「播放时收紧阈值」的降级方案用
    /// （CoreAudio 的回声消除在外接音箱下不可靠，见坑点 7）。
    speaking: Arc<AtomicBool>,
    /// drop 它会让守着 OutputStream 的线程退出，从而释放音频设备
    _shutdown: std::sync::mpsc::Sender<()>,
    /// 播放信号的发送端，交给每段音频的 Metered 包装器
    signal_tx: std::sync::mpsc::Sender<PlaybackSignal>,
}

impl Player {
    /// `signal_tx` 用于上报播放电平和「某一句开始出声」。
    /// 不关心时传一个丢弃端即可 —— 发送失败会被忽略。
    pub fn new(signal_tx: std::sync::mpsc::Sender<PlaybackSignal>) -> Result<Self> {
        // 用 sync_channel(0) 让「设备是否打开成功」同步回传，
        // 这样 new() 的语义仍然是「返回即可用」，而不是延迟到第一次播放才报错
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel::<Result<Arc<Sink>>>(0);
        let (shutdown_tx, shutdown_rx) = std::sync::mpsc::channel::<()>();

        std::thread::Builder::new()
            .name("anima-audio-out".into())
            .spawn(move || {
                let opened = OutputStream::try_default()
                    .map_err(|e| Error::Io(format!("打开音频输出设备失败：{e}")))
                    .and_then(|(stream, handle)| {
                        let sink = Sink::try_new(&handle)
                            .map_err(|e| Error::Io(format!("创建播放队列失败：{e}")))?;
                        Ok((stream, sink))
                    });

                match opened {
                    Ok((stream, sink)) => {
                        if ready_tx.send(Ok(Arc::new(sink))).is_err() {
                            return; // 调用方已经放弃了
                        }
                        // 阻塞在这里：只要 Player 还活着，stream 就必须活着。
                        // Player 被 drop 时 Sender 关闭，recv 返回 Err，线程退出。
                        let _ = shutdown_rx.recv();
                        drop(stream);
                    }
                    Err(err) => {
                        let _ = ready_tx.send(Err(err));
                    }
                }
            })
            .map_err(|e| Error::Io(format!("创建音频输出线程失败：{e}")))?;

        let sink = ready_rx
            .recv()
            .map_err(|_| Error::Io("音频输出线程意外退出".into()))??;

        Ok(Self {
            sink,
            speaking: Arc::new(AtomicBool::new(false)),
            _shutdown: shutdown_tx,
            signal_tx,
        })
    }

    /// 把一段合成好的音频追加到播放队列。
    ///
    /// **格式必须嗅探，不能写死。** 原先这里是 `Decoder::new_mp3` ——
    /// 那是 Edge TTS 的输出格式。接入本地引擎之后 Kokoro / CosyVoice /
    /// GPT-SoVITS 返回的都是 WAV，用 MP3 解码器去解会得到一串
    /// "invalid mpeg audio header" 然后 "end of stream"：
    /// 文字回答正常生成、只是听不到声音，表现成"它不说话了"。
    ///
    /// 不等它播完 —— 语音管线是「一句合成好就先播，后面的边播边合成」，
    /// 这样用户听到第一个字的时间只取决于第一句。
    pub fn enqueue_audio(&self, bytes: Vec<u8>) -> Result<()> {
        self.enqueue_cued(bytes, None)
    }

    /// 同 [`Player::enqueue_audio`]，但这段音频**开始出声时**会通过 signal
    /// channel 报一个 `Started(cue)`。
    ///
    /// 对口型需要它：词边界时间戳的 t=0 是「这一句开始出声」，
    /// 而那个时刻只有播放侧知道（见 [`PlaybackSignal::Started`]）。
    ///
    /// 打断时不用做任何额外处理 —— `interrupt()` 清空队列，被丢掉的音频
    /// 永远取不到第一个采样，也就永远不会报 `Started`。这个正确性是
    /// 「在第一个采样上报」这个设计白送的，不是额外写出来的。
    pub fn enqueue_cued(&self, bytes: Vec<u8>, cue: Option<u64>) -> Result<()> {
        // RIFF 头就是 WAV。显式分支而不是一律交给 Decoder::new 去探：
        // 出错时的信息能指明"我按什么格式解的"，排查快得多
        // 流式 WAV 的头要先修 —— 见 fix_streaming_wav
        let bytes = fix_streaming_wav(bytes);

        // 把格式记进错误信息：rodio 只说"end of stream"，
        // 不告诉你它按什么格式解的 —— 而这正是排查的关键
        let kind = if bytes.len() > 4 && &bytes[0..4] == b"RIFF" {
            "WAV"
        } else {
            "MP3"
        };
        // Decoder::new 在已启用的解码器之间自动探测（见 Cargo.toml 的 features）
        let decoder = Decoder::new(Cursor::new(bytes))
            .map_err(|e| Error::decode(format!("解码 TTS 音频失败（看起来是 {kind}）：{e}")))?;
        // 转成 f32 再插电平表：i16 上算 RMS 要额外做归一化，
        // 而 rodio 的混音链路本来就会转 f32
        self.sink.append(Metered::new(
            decoder.convert_samples::<f32>(),
            self.signal_tx.clone(),
            cue,
        ));
        self.speaking.store(true, Ordering::Relaxed);
        Ok(())
    }

    /// 立刻停止并清空队列 —— barge-in 时调用。
    pub fn interrupt(&self) {
        self.sink.clear();
        self.speaking.store(false, Ordering::Relaxed);
        // clear() 会让 sink 进入暂停态，要重新 play 才能接受后续音频
        self.sink.play();
    }

    /// 队列是否已播完
    pub fn is_idle(&self) -> bool {
        let idle = self.sink.empty();
        if idle {
            self.speaking.store(false, Ordering::Relaxed);
        }
        idle
    }

    /// 是否正在出声（供 VAD 侧收紧阈值）
    pub fn is_speaking(&self) -> bool {
        self.speaking.load(Ordering::Relaxed) && !self.sink.empty()
    }

    /// 共享「正在出声」标志给采集侧。
    ///
    /// 采集线程用它决定要不要收紧 VAD 阈值。用 Arc<AtomicBool> 而不是回调或
    /// 加锁查询：这个标志会在音频线程的每帧循环里读，必须是无锁且极廉价的。
    pub fn speaking_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.speaking)
    }

    /// 队列播完后把标志落下来。
    ///
    /// rodio 不提供「播完」回调，所以由管线定期调用它同步状态 ——
    /// 否则播完之后标志一直是 true，VAD 会永远停在钝化状态、再也打断不了。
    pub fn sync_speaking_state(&self) -> bool {
        let speaking = !self.sink.empty();
        self.speaking.store(speaking, Ordering::Relaxed);
        speaking
    }

    pub fn set_volume(&self, volume: f32) {
        self.sink.set_volume(volume.clamp(0.0, 1.0));
    }
}

/// 把流式文本切成适合送 TTS 的句子。
///
/// 为什么需要它：TTS 按整段合成的话，用户要等模型把整段说完才听到声音。
/// 按句切分能让「第一句合成」和「后续生成」并行，首字延迟只取决于第一句。
///
/// 规则与用户已有的 Python 原型保持一致（`STREAM_SENTENCE_ENDINGS` /
/// `STREAM_MIN_CHARS`）：遇到句末标点且长度够就切。
pub struct SentenceSplitter {
    buffer: String,
    min_chars: usize,
}

/// 触发切句的标点。中英文都要覆盖。
const SENTENCE_ENDINGS: &[char] = &['。', '！', '？', '；', '\n', '.', '!', '?', ';'];

impl SentenceSplitter {
    pub fn new(min_chars: usize) -> Self {
        Self {
            buffer: String::new(),
            min_chars,
        }
    }

    /// 喂入一段增量文本，取出所有已经成形的句子。
    pub fn push(&mut self, delta: &str) -> Vec<String> {
        self.buffer.push_str(delta);
        let mut out = Vec::new();

        loop {
            // 找到第一个「够长 + 落在句末标点」的切点
            let cut = self
                .buffer
                .char_indices()
                .scan(0usize, |count, (idx, ch)| {
                    *count += 1;
                    Some((idx, ch, *count))
                })
                .find(|(_, ch, count)| *count >= self.min_chars && SENTENCE_ENDINGS.contains(ch));

            match cut {
                Some((idx, ch, _)) => {
                    // idx 是标点的起始字节位置，要连标点一起切走
                    let end = idx + ch.len_utf8();
                    let sentence: String = self.buffer.drain(..end).collect();
                    let trimmed = sentence.trim().to_string();
                    if !trimmed.is_empty() {
                        out.push(trimmed);
                    }
                }
                None => break,
            }
        }
        out
    }

    /// 流结束时把剩下的残句取出来（可能没有句末标点）。
    pub fn flush(&mut self) -> Option<String> {
        let rest = std::mem::take(&mut self.buffer).trim().to_string();
        (!rest.is_empty()).then_some(rest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 固定长度的静音源，用来喂 [`Metered`]。
    ///
    /// 不用真实解码器：这里要验的是「什么时候报开始」，那是纯粹的取样时序问题，
    /// 和音频格式无关。真跑一遍播放还要开声卡、还要等实时时间，又慢又飘。
    struct Silence {
        left: usize,
    }

    impl Iterator for Silence {
        type Item = f32;
        fn next(&mut self) -> Option<f32> {
            if self.left == 0 {
                return None;
            }
            self.left -= 1;
            Some(0.5)
        }
    }

    impl Source for Silence {
        fn current_frame_len(&self) -> Option<usize> {
            None
        }
        fn channels(&self) -> u16 {
            1
        }
        fn sample_rate(&self) -> u32 {
            24_000
        }
        fn total_duration(&self) -> Option<std::time::Duration> {
            None
        }
    }

    /// 走 `Metered` 跑一遍，收集它发出的信号
    fn drain_signals(cue: Option<u64>, samples: usize) -> Vec<PlaybackSignal> {
        let (tx, rx) = std::sync::mpsc::channel();
        let src = Silence { left: samples };
        let mut m = Metered::new(src, tx, cue);
        while m.next().is_some() {}
        drop(m);
        rx.try_iter().collect()
    }

    /// **带句子 id 的音频不能发原始帧。**
    ///
    /// 有 id 说明它走的是文本时间线（15 个 viseme、含辅音），比音频驱动那条准。
    /// 两条同时发的话，粗的那条会把准的那条盖掉 —— 而且看不出是被盖了
    #[test]
    fn audio_with_a_sentence_id_does_not_emit_analysis_frames() {
        let with_cue = drain_signals(Some(7), ANALYSIS_FRAME * 3);
        assert!(
            !with_cue
                .iter()
                .any(|s| matches!(s, PlaybackSignal::Frame(..))),
            "走文本时间线的音频不该再发音频帧"
        );
        assert_eq!(started_ids(&with_cue), vec![7]);
    }

    /// 没有 id 的音频（play_audio、电台）才发帧 ——
    /// 补的正是"这些场景嘴一动不动"
    #[test]
    fn audio_without_a_sentence_id_emits_analysis_frames() {
        let no_cue = drain_signals(None, ANALYSIS_FRAME * 3);
        let frames: Vec<_> = no_cue
            .iter()
            .filter_map(|s| match s {
                PlaybackSignal::Frame(v, rate) => Some((v.len(), *rate)),
                _ => None,
            })
            .collect();
        assert!(frames.len() >= 2, "只发了 {} 帧", frames.len());
        for (len, rate) in frames {
            assert_eq!(len, ANALYSIS_FRAME);
            assert_eq!(
                rate, 24_000,
                "采样率要跟着音源走，写死会让 mel 滤波器组对不上"
            );
        }
    }

    fn started_ids(signals: &[PlaybackSignal]) -> Vec<u64> {
        signals
            .iter()
            .filter_map(|s| match s {
                PlaybackSignal::Started(id) => Some(*id),
                PlaybackSignal::Level(_) | PlaybackSignal::Frame(..) => None,
            })
            .collect()
    }

    /// **构造包装器不等于开始出声。**
    ///
    /// 这是整个对口型时间对齐的地基。句子是排队播的，包装器在入队那一刻就构造好，
    /// 但那时前一句往往还在播。如果构造时就报开始，前端会把嘴型时间线的 t=0
    /// 设在合成时刻 —— 差几秒，整句嘴型全错开。
    #[test]
    fn does_not_report_start_until_first_sample_is_taken() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut m = Metered::new(Silence { left: 10 }, tx, Some(7));

        // 构造完、还没取样
        assert!(
            rx.try_recv().is_err(),
            "入队时就报开始的话，嘴型会对着还没播的句子动"
        );

        m.next();
        assert_eq!(
            started_ids(&[rx.try_recv().expect("取到第一个采样就该报开始")]),
            vec![7]
        );
    }

    /// 一段音频只报一次开始，不是每个采样都报。
    #[test]
    fn reports_start_exactly_once() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut m = Metered::new(Silence { left: 5_000 }, tx, Some(3));
        while m.next().is_some() {}
        let all: Vec<_> = rx.try_iter().collect();
        assert_eq!(started_ids(&all), vec![3], "开始只能报一次");
        // 顺带确认电平还在正常产出 —— 加 cue 不该把原来的电平链路弄坏
        assert!(
            all.iter()
                .any(|s| matches!(s, PlaybackSignal::Level(l) if *l > 0.0)),
            "电平没了：{all:?}"
        );
    }

    /// 空音频不报开始。
    ///
    /// 「整句都是标点」这种情况合成出来就是空的。对一段不存在的声音报开始，
    /// 前端会让嘴动一段却没有任何声音 —— 比不动更假。
    #[test]
    fn empty_audio_never_reports_start() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut m = Metered::new(Silence { left: 0 }, tx, Some(9));
        assert!(m.next().is_none());
        assert!(
            started_ids(&rx.try_iter().collect::<Vec<_>>()).is_empty(),
            "空音频不该报开始"
        );
    }

    /// 没给 cue 的音频（比如电台）不报开始，也不影响电平。
    #[test]
    fn uncued_audio_only_reports_levels() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut m = Metered::new(Silence { left: 5_000 }, tx, None);
        while m.next().is_some() {}
        let all: Vec<_> = rx.try_iter().collect();
        assert!(started_ids(&all).is_empty());
        assert!(all.iter().any(|s| matches!(s, PlaybackSignal::Level(_))));
    }

    #[test]
    fn splits_on_chinese_punctuation() {
        let mut s = SentenceSplitter::new(4);
        assert_eq!(
            s.push("今天天气不错。我们出去走走吧！"),
            vec!["今天天气不错。", "我们出去走走吧！"]
        );
    }

    #[test]
    fn waits_for_min_chars_before_cutting() {
        let mut s = SentenceSplitter::new(8);
        // 「好。」太短，不该单独成句 —— 否则 TTS 会蹦出一个突兀的单字音频
        assert!(s.push("好。").is_empty());
        assert_eq!(s.push("我明白你的意思了。"), vec!["好。我明白你的意思了。"]);
    }

    #[test]
    fn accumulates_across_streaming_deltas() {
        let mut s = SentenceSplitter::new(4);
        assert!(s.push("我是").is_empty());
        assert!(s.push("Anima").is_empty());
        assert_eq!(
            s.push("，很高兴认识你。"),
            vec!["我是Anima，很高兴认识你。"]
        );
    }

    #[test]
    fn flush_returns_trailing_fragment() {
        let mut s = SentenceSplitter::new(4);
        assert!(s.push("没有标点收尾的残句").is_empty());
        assert_eq!(s.flush().as_deref(), Some("没有标点收尾的残句"));
        // flush 之后缓冲区应当空了
        assert_eq!(s.flush(), None);
    }

    #[test]
    fn does_not_split_inside_multibyte_characters() {
        // 切点用字节索引，中文必须按字符边界切，否则会 panic
        let mut s = SentenceSplitter::new(2);
        let out = s.push("啊。哦。呃。");
        assert_eq!(out, vec!["啊。", "哦。", "呃。"]);
    }

    #[test]
    fn handles_english_punctuation_and_newlines() {
        let mut s = SentenceSplitter::new(4);
        assert_eq!(
            s.push("Hello there. How are you?\n"),
            vec!["Hello there.", "How are you?"]
        );
    }
}

/// 修正"流式写出"的 WAV 头。
///
/// Kokoro（以及其它边生成边返回的 TTS 服务）在写 WAV 头时还不知道音频总长，
/// 于是把 RIFF 和 data 两处长度都填成占位值 `0xFFFFFFFF`。rodio/hound 相信这个
/// 声明，去读 4GB 数据，读到实际结尾就报 `end of stream` —— 表现是
/// **文字回答正常、完全听不到声音**。
///
/// 我在 `tts::wav_duration_ms` 里处理过同一个坑（那里注释也写着"size 可能是
/// 流式占位"），但播放路径走的是 rodio，它不会替我们兜。所以在入队前修一遍。
///
/// 只在确实需要时复制：正常的 WAV 直接原样返回，不白花一次内存拷贝。
fn fix_streaming_wav(bytes: Vec<u8>) -> Vec<u8> {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return bytes;
    }

    // 找 data chunk。必须遍历 —— fmt 和 data 之间常夹着 LIST（Kokoro 就有）
    let mut pos = 12usize;
    let mut data_at = None;
    while pos + 8 <= bytes.len() {
        let size = u32::from_le_bytes([
            bytes[pos + 4],
            bytes[pos + 5],
            bytes[pos + 6],
            bytes[pos + 7],
        ]);
        if &bytes[pos..pos + 4] == b"data" {
            data_at = Some((pos, size));
            break;
        }
        // chunk 按偶数对齐；用 checked 防止畸形 size 让 pos 溢出后回绕
        match pos.checked_add(8 + size as usize + (size % 2) as usize) {
            Some(next) if next > pos => pos = next,
            _ => break,
        }
    }

    let Some((data_pos, declared)) = data_at else {
        return bytes;
    };
    let actual = (bytes.len() - data_pos - 8) as u32;
    // 声明值正确就别动
    if declared == actual {
        return bytes;
    }

    let mut fixed = bytes;
    fixed[data_pos + 4..data_pos + 8].copy_from_slice(&actual.to_le_bytes());
    let riff = (fixed.len() - 8) as u32;
    fixed[4..8].copy_from_slice(&riff.to_le_bytes());
    tracing::debug!(declared, actual, "修正了流式 WAV 头的长度字段");
    fixed
}

#[cfg(test)]
mod wav_header_tests {
    use super::*;

    /// 拼一个带 LIST chunk、且长度字段是流式占位的 WAV（Kokoro 的真实形状）
    fn streaming_wav(data_len: usize) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend(b"RIFF");
        b.extend(u32::MAX.to_le_bytes()); // 占位
        b.extend(b"WAVE");
        b.extend(b"fmt ");
        b.extend(16u32.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(24_000u32.to_le_bytes());
        b.extend(48_000u32.to_le_bytes());
        b.extend(2u16.to_le_bytes());
        b.extend(16u16.to_le_bytes());
        b.extend(b"LIST");
        b.extend(26u32.to_le_bytes());
        b.extend(std::iter::repeat_n(0u8, 26));
        b.extend(b"data");
        b.extend(u32::MAX.to_le_bytes()); // 占位
        b.extend(std::iter::repeat_n(0u8, data_len));
        b
    }

    #[test]
    fn placeholder_sizes_are_rewritten_to_actual_lengths() {
        let fixed = fix_streaming_wav(streaming_wav(1000));
        let riff = u32::from_le_bytes(fixed[4..8].try_into().unwrap());
        assert_eq!(riff as usize, fixed.len() - 8, "RIFF 长度应当被修正");

        // data chunk 在 LIST 之后
        let data_pos = fixed
            .windows(4)
            .position(|w| w == b"data")
            .expect("找得到 data chunk");
        let declared = u32::from_le_bytes(fixed[data_pos + 4..data_pos + 8].try_into().unwrap());
        assert_eq!(declared, 1000, "data 长度应当等于实际字节数");
    }

    #[test]
    fn well_formed_wav_is_returned_untouched() {
        let mut good = streaming_wav(1000);
        let data_pos = good.windows(4).position(|w| w == b"data").unwrap();
        good[data_pos + 4..data_pos + 8].copy_from_slice(&1000u32.to_le_bytes());
        let riff = (good.len() - 8) as u32;
        good[4..8].copy_from_slice(&riff.to_le_bytes());
        let before = good.clone();
        assert_eq!(fix_streaming_wav(good), before, "正常的 WAV 不该被改动");
    }

    #[test]
    fn non_wav_input_passes_through() {
        // mp3 之类原样返回，别去动它
        let mp3 = b"ID3\x04\x00\x00\x00\x00\x00\x00 rest of the frames".to_vec();
        assert_eq!(fix_streaming_wav(mp3.clone()), mp3);
        assert_eq!(fix_streaming_wav(vec![1, 2, 3]), vec![1, 2, 3]);
    }

    #[test]
    fn malformed_chunk_size_does_not_loop_or_panic() {
        // 畸形 size 会让 pos 溢出回绕，进而死循环 —— checked_add 挡住
        let mut b = streaming_wav(100);
        b[16..20].copy_from_slice(&u32::MAX.to_le_bytes()); // fmt chunk 声明成 4GB
        let out = fix_streaming_wav(b.clone());
        assert_eq!(out.len(), b.len(), "找不到 data 就原样返回");
    }
}

/// 解码成单声道 f32 采样，返回 (采样, 采样率)。
///
/// VAD 自检、`play_audio` 的时长探测这些要拿到采样点本身，而 `enqueue_audio`
/// 只负责丢给播放队列。走的是**同一条解码路径**（含流式 WAV 头修正），
/// 所以不会出现"能播但读不出"或反之。
///
/// 多声道会被下混成单声道：TTS 输出基本都是单声道，而下游的分析按单声道写
/// 简单得多；真遇到立体声，下混比只取左声道更保险。
pub fn decode_mono(bytes: Vec<u8>) -> Result<(Vec<f32>, u32)> {
    use rodio::Source;
    let bytes = fix_streaming_wav(bytes);
    let decoder = Decoder::new(Cursor::new(bytes))
        .map_err(|e| Error::decode(format!("解码音频失败：{e}")))?;
    let sample_rate = decoder.sample_rate();
    let channels = decoder.channels().max(1) as usize;
    let samples: Vec<f32> = decoder.convert_samples::<f32>().collect();

    if channels == 1 {
        return Ok((samples, sample_rate));
    }
    let mono = samples
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect();
    Ok((mono, sample_rate))
}

/// 把单声道 f32 采样编码成 16 位 WAV，交给播放队列。
///
/// 之所以编码回 WAV 而不是直接送采样：播放路径统一走 `enqueue_audio`，
/// 只有一条路就只有一处可能出错。多一次编解码的开销远小于维护两条路径的代价。
pub fn encode_mono_wav(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let pcm: Vec<u8> = samples
        .iter()
        .flat_map(|&s| {
            // 先夹再转：超出 [-1,1] 的值直接转 i16 会回绕成反相的爆音
            let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            v.to_le_bytes()
        })
        .collect();
    // 复用 tts 里那个 —— 它是当初为 CosyVoice 的裸 PCM 写的，
    // 头部字段的处理（含长度修正）已经被测试盖住，没必要再写一遍
    crate::audio::wav::wrap_pcm_s16le(&pcm, sample_rate, 1)
}

/// 只给集成测试用：确认真实服务的输出能被解码。
#[doc(hidden)]
pub fn decode_for_test(bytes: Vec<u8>) -> Result<usize> {
    decode_mono(bytes).map(|(s, _)| s.len())
}
