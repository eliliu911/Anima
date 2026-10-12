//! 把 reqwest 的字节流解码成"一条条 payload"。
//!
//! 同时覆盖两种协议，因为本项目要接的后端刚好两种都有：
//!   - SSE（OpenAI / Claude / LM Studio）：`data: {...}\n\n`，以 `data: [DONE]` 收尾
//!   - NDJSON（Ollama 原生 `/api/chat`）：一行一个完整 JSON 对象
//!
//! 两者的共同点是"按行切分 + 忽略空行"，所以用同一个解码器处理：SSE 的 `data:`
//! 前缀在这里剥掉，其余 SSE 元字段（event:/id:/retry:）直接丢弃。

use bytes::{Bytes, BytesMut};
use futures_util::{stream::BoxStream, Stream, StreamExt};

use crate::error::{Error, Result};

/// SSE 流的终止标记
const SSE_DONE: &str = "[DONE]";

/// 单条解码结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// 一条待反序列化的 JSON 文本
    Data(String),
    /// 收到了显式的流结束标记
    Done,
}

/// 增量式行解码器。
///
/// 需要自己维护缓冲区，是因为 HTTP chunk 边界和行边界毫无关系：
/// 一个 chunk 可能只含半行，也可能含三行半。
#[derive(Default)]
pub struct LineDecoder {
    buf: String,
}

impl LineDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// 喂入一段新到的文本，取出其中所有**完整**的帧。
    pub fn push(&mut self, chunk: &str) -> Vec<Frame> {
        self.buf.push_str(chunk);
        let mut out = Vec::new();

        // 只处理已经出现换行符的部分，剩下的留在 buf 里等下一个 chunk
        while let Some(idx) = self.buf.find('\n') {
            let line: String = self.buf.drain(..=idx).collect();
            if let Some(frame) = Self::parse_line(line.trim_end_matches(['\n', '\r'])) {
                out.push(frame);
            }
        }
        out
    }

    /// 流结束时调用，取出缓冲区里最后一行没有换行符收尾的内容。
    pub fn finish(&mut self) -> Option<Frame> {
        let rest = std::mem::take(&mut self.buf);
        Self::parse_line(rest.trim())
    }

    fn parse_line(line: &str) -> Option<Frame> {
        if line.is_empty() {
            return None;
        }

        let payload = match line.split_once(':') {
            // SSE 的 data 字段；按规范去掉冒号后的一个前导空格
            Some(("data", rest)) => rest.strip_prefix(' ').unwrap_or(rest),
            // event: / id: / retry: 等 SSE 元字段，本项目不需要
            Some(("event" | "id" | "retry", _)) => return None,
            // SSE 注释行（": keep-alive"）
            Some(("", _)) => return None,
            // 没有前缀 —— NDJSON 行，整行就是 payload
            _ => line,
        };

        let payload = payload.trim();
        if payload.is_empty() {
            None
        } else if payload == SSE_DONE {
            Some(Frame::Done)
        } else {
            Some(Frame::Data(payload.to_string()))
        }
    }
}

/// 校验响应状态码，失败时把响应体一起带进错误。
///
/// 不这么做的话，调试本地推理服务时只能看到一个光秃秃的 400，
/// 而 LM Studio / Ollama 恰恰把"模型没加载""上下文超长"这类关键信息放在 body 里。
pub async fn ensure_success(resp: reqwest::Response) -> Result<reqwest::Response> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body = resp.text().await.unwrap_or_default();
    Err(Error::upstream(status.as_u16(), body))
}

/// 解码器状态。显式写成 struct 是为了让 `unfold` 出来的流保持 `Send`，
/// 可以直接丢进 tokio task。
struct DecodeState {
    bytes: BoxStream<'static, reqwest::Result<Bytes>>,
    /// 未解码的原始字节。多字节 UTF-8（中文）可能被 chunk 边界切断，
    /// 所以先按字节缓冲，只把能解出的合法前缀交给行解码器。
    raw: BytesMut,
    decoder: LineDecoder,
    pending: std::collections::VecDeque<Frame>,
    eof: bool,
}

impl DecodeState {
    /// 从 `raw` 头部取出最长的合法 UTF-8 前缀，剩余的不完整字符留在缓冲区。
    fn drain_utf8(&mut self) -> String {
        let valid_len = match std::str::from_utf8(&self.raw) {
            Ok(_) => self.raw.len(),
            Err(err) => err.valid_up_to(),
        };
        let head = self.raw.split_to(valid_len);
        // 安全：valid_up_to() 保证这一段是合法 UTF-8
        String::from_utf8(head.to_vec()).unwrap_or_default()
    }
}

/// 把响应体解码成 `Frame` 流。调用前应先过 [`ensure_success`]。
pub fn decode_response(resp: reqwest::Response) -> impl Stream<Item = Result<Frame>> + Send {
    let state = DecodeState {
        bytes: resp.bytes_stream().boxed(),
        raw: BytesMut::new(),
        decoder: LineDecoder::new(),
        pending: std::collections::VecDeque::new(),
        eof: false,
    };

    futures_util::stream::unfold(state, |mut st| async move {
        loop {
            // 先把上一轮解出来但还没吐完的帧发出去
            if let Some(frame) = st.pending.pop_front() {
                return Some((Ok(frame), st));
            }
            if st.eof {
                return None;
            }

            match st.bytes.next().await {
                Some(Ok(chunk)) => {
                    st.raw.extend_from_slice(&chunk);
                    let text = st.drain_utf8();
                    if !text.is_empty() {
                        let frames = st.decoder.push(&text);
                        st.pending.extend(frames);
                    }
                }
                Some(Err(err)) => {
                    st.eof = true;
                    return Some((Err(Error::Network(err.to_string())), st));
                }
                None => {
                    st.eof = true;
                    if let Some(frame) = st.decoder.finish() {
                        st.pending.push_back(frame);
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_sse_frames() {
        let mut d = LineDecoder::new();
        let frames = d.push("data: {\"a\":1}\n\ndata: {\"a\":2}\n\n");
        assert_eq!(
            frames,
            vec![
                Frame::Data("{\"a\":1}".into()),
                Frame::Data("{\"a\":2}".into())
            ]
        );
    }

    #[test]
    fn handles_frame_split_across_chunks() {
        let mut d = LineDecoder::new();
        assert!(d.push("data: {\"partial\"").is_empty());
        assert_eq!(
            d.push(":true}\n"),
            vec![Frame::Data("{\"partial\":true}".into())]
        );
    }

    #[test]
    fn recognises_done_marker() {
        let mut d = LineDecoder::new();
        assert_eq!(d.push("data: [DONE]\n"), vec![Frame::Done]);
    }

    #[test]
    fn passes_through_ndjson_lines() {
        let mut d = LineDecoder::new();
        assert_eq!(
            d.push("{\"done\":false}\n{\"done\":true}\n"),
            vec![
                Frame::Data("{\"done\":false}".into()),
                Frame::Data("{\"done\":true}".into())
            ]
        );
    }

    #[test]
    fn drops_sse_metadata_fields() {
        let mut d = LineDecoder::new();
        assert!(d.push("event: message_start\n: keep-alive\n").is_empty());
    }

    #[test]
    fn emits_trailing_line_without_newline() {
        let mut d = LineDecoder::new();
        assert!(d.push("{\"x\":1}").is_empty());
        assert_eq!(d.finish(), Some(Frame::Data("{\"x\":1}".into())));
    }

    #[test]
    fn json_with_colon_in_string_is_not_mistaken_for_sse_field() {
        // NDJSON 行以 `{` 开头，split_once(':') 会切出 `{"role"` 这种前缀，
        // 必须落到 fallback 分支而不是被当成未知 SSE 字段丢掉
        let mut d = LineDecoder::new();
        assert_eq!(
            d.push("{\"role\":\"assistant\"}\n"),
            vec![Frame::Data("{\"role\":\"assistant\"}".into())]
        );
    }
}
