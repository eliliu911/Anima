//! 极简 mock HTTP 服务，用于在不依赖真实模型服务的前提下验证各适配器的协议处理。
//!
//! 手写 HTTP/1.1 而不是引 wiremock 之类：只需要"按节奏吐 chunked 响应"这一个能力，
//! 而且要能精确控制 chunk 边界（正是流式解析最容易出错的地方）。
//!
//! 这个模块被多个测试二进制共享，每个只用到其中一部分，所以整体允许 dead_code。

#![allow(dead_code)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// mock 服务的行为描述
pub struct MockSpec {
    /// 响应状态码
    pub status: u16,
    /// Content-Type
    pub content_type: &'static str,
    /// 逐个写出的 chunk。刻意允许一个 chunk 里含半行，
    /// 用来验证解码器能跨 chunk 拼接。
    pub chunks: Vec<String>,
    /// 每个 chunk 之间的间隔
    pub delay: Duration,
}

impl MockSpec {
    /// SSE 响应：每个元素自动补上 `data: ` 前缀与空行
    pub fn sse(events: &[&str]) -> Self {
        Self {
            status: 200,
            content_type: "text/event-stream",
            chunks: events.iter().map(|e| format!("data: {e}\n\n")).collect(),
            delay: Duration::from_millis(1),
        }
    }

    /// NDJSON 响应（Ollama 原生协议）
    pub fn ndjson(lines: &[&str]) -> Self {
        Self {
            status: 200,
            content_type: "application/x-ndjson",
            chunks: lines.iter().map(|l| format!("{l}\n")).collect(),
            delay: Duration::from_millis(1),
        }
    }

    /// 原样写出给定 chunk，用于自定义 chunk 边界
    pub fn raw(chunks: &[&str]) -> Self {
        Self {
            status: 200,
            content_type: "text/event-stream",
            chunks: chunks.iter().map(|c| c.to_string()).collect(),
            delay: Duration::from_millis(1),
        }
    }

    pub fn error(status: u16, body: &str) -> Self {
        Self {
            status,
            content_type: "application/json",
            chunks: vec![body.to_string()],
            delay: Duration::ZERO,
        }
    }

    pub fn with_delay(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

/// 已启动的 mock 服务
pub struct MockServer {
    pub base_url: String,
    /// 最后一次收到的请求体，供断言"我们发出去的 JSON 长什么样"
    captured: Arc<Mutex<Option<String>>>,
    /// 按顺序记录的所有请求体
    all: Arc<Mutex<Vec<String>>>,
}

impl MockServer {
    /// 取出捕获到的请求体并解析成 JSON
    pub fn request_body(&self) -> serde_json::Value {
        let raw = self
            .captured
            .lock()
            .unwrap()
            .clone()
            .expect("mock 服务没有收到请求");
        serde_json::from_str(&raw).expect("请求体不是合法 JSON")
    }

    /// 按顺序取出所有请求体
    pub fn request_bodies(&self) -> Vec<serde_json::Value> {
        self.all
            .lock()
            .unwrap()
            .iter()
            .map(|raw| serde_json::from_str(raw).expect("请求体不是合法 JSON"))
            .collect()
    }
}

/// 启动 mock 服务，按顺序用给定的 spec 依次响应多个请求。
///
/// 需要多个 spec 是为了测「先被拒、去掉参数重试」这类两次请求的路径。
pub async fn spawn_many(specs: Vec<MockSpec>) -> MockServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let captured = Arc::new(Mutex::new(None));
    let all = Arc::new(Mutex::new(Vec::new()));
    let captured_clone = Arc::clone(&captured);
    let all_clone = Arc::clone(&all);

    tokio::spawn(async move {
        for spec in specs {
            let Ok((socket, _)) = listener.accept().await else {
                return;
            };
            let all = Arc::clone(&all_clone);
            let last = Arc::clone(&captured_clone);
            serve_one(socket, &spec, &move |body: String| {
                all.lock().unwrap().push(body.clone());
                *last.lock().unwrap() = Some(body);
            })
            .await;
        }
    });

    MockServer {
        base_url: format!("http://{addr}"),
        captured,
        all,
    }
}

/// 启动 mock 服务，只处理一个连接然后退出。
pub async fn spawn(spec: MockSpec) -> MockServer {
    // 端口 0 让内核分配，避免并行跑测试时端口冲突
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let captured = Arc::new(Mutex::new(None));
    let captured_clone = Arc::clone(&captured);

    tokio::spawn(async move {
        let Ok((socket, _)) = listener.accept().await else {
            return;
        };
        serve_one(socket, &spec, &move |body| {
            *captured_clone.lock().unwrap() = Some(body);
        })
        .await;
    });

    MockServer {
        base_url: format!("http://{addr}"),
        captured,
        all: Arc::new(Mutex::new(Vec::new())),
    }
}

/// 处理一个连接：读完请求、按 spec 写响应。
///
/// `record` 在**读完请求体、还没开始写响应**时就被调用。这个时机很关键：
/// 若等整个响应写完才记录，客户端可能已经读完流并开始断言了，
/// 于是断言看到的还是空的 —— 一个只在并发下偶发的假失败。
async fn serve_one(
    mut socket: tokio::net::TcpStream,
    spec: &MockSpec,
    record: &(dyn Fn(String) + Send + Sync),
) {
    {
        // ---- 读请求 ----
        let mut buf = Vec::new();
        let mut tmp = [0u8; 4096];
        let mut header_end = None;

        // 先读到 header 结束
        while header_end.is_none() {
            match socket.read(&mut tmp).await {
                Ok(0) | Err(_) => return,
                Ok(n) => {
                    buf.extend_from_slice(&tmp[..n]);
                    header_end = find_header_end(&buf);
                }
            }
        }
        let header_end = header_end.unwrap();

        // 再按 Content-Length 把 body 读完，否则客户端可能拿到 broken pipe
        let headers = String::from_utf8_lossy(&buf[..header_end]).to_string();
        let content_length = parse_content_length(&headers);
        while buf.len() < header_end + content_length {
            match socket.read(&mut tmp).await {
                Ok(0) | Err(_) => break,
                Ok(n) => buf.extend_from_slice(&tmp[..n]),
            }
        }

        if content_length > 0 {
            record(String::from_utf8_lossy(&buf[header_end..]).to_string());
        }

        // ---- 写响应 ----
        let head = format!(
            "HTTP/1.1 {} {}\r\n\
             Content-Type: {}\r\n\
             Transfer-Encoding: chunked\r\n\
             Cache-Control: no-cache\r\n\
             \r\n",
            spec.status,
            if spec.status == 200 { "OK" } else { "Error" },
            spec.content_type,
        );
        if socket.write_all(head.as_bytes()).await.is_err() {
            return;
        }

        for chunk in &spec.chunks {
            // HTTP chunked 编码：十六进制长度 + CRLF + 数据 + CRLF
            let framed = format!("{:x}\r\n{}\r\n", chunk.len(), chunk);
            if socket.write_all(framed.as_bytes()).await.is_err() {
                return; // 客户端已断开（取消测试里的正常路径）
            }
            let _ = socket.flush().await;
            if !spec.delay.is_zero() {
                tokio::time::sleep(spec.delay).await;
            }
        }

        let _ = socket.write_all(b"0\r\n\r\n").await;
        let _ = socket.flush().await;
    }
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|i| i + 4)
}

fn parse_content_length(headers: &str) -> usize {
    headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())?
        })
        .unwrap_or(0)
}
