//! 取回一个 URL 的内容，转成纯文本交给模型。
//!
//! ## 为什么需要它
//!
//! 在这之前 Anima 的网络工具只有三个：`web_search`（搜索）、`search_web`、
//! `open_url_or_file`（在**系统浏览器里打开**，内容根本回不到模型手里）。
//! 少了「把这个网址的内容拿回来」这一把。
//!
//! 后果是实打实的：用户让模型对比几个 GitHub 上的六爻项目，模型唯一能走的路
//! 是 shell 里 `git clone`。于是撞上网络抖动、目录冲突、`rm -rf` 重来，
//! **18 条命令里一半在跟 clone 较劲**，而且克隆到哪儿了没人说、谁清理也没人管。
//!
//! 有了这把工具，读一个远端文件就是一次请求，不落盘、没有清理问题、
//! 失败就是一条明确的错误，不会演变成 shell 重试循环。
//!
//! ## 刻意的取舍
//!
//! - **不引 HTML 解析依赖。** 剥标签 + 解实体足够应付文档和源码页；
//!   为了少数排版精细的页面引一整个 DOM 解析器不划算。
//! - **GitHub 页面自动换成 raw。** `github.com/o/r/blob/main/x.ts` 抓回来是
//!   一整页应用外壳，真正的源码埋在里面；`raw.githubusercontent.com` 直接给源码。
//!   这是模型最常想读的东西，值得special-case。
//! - **有大小上限。** 一个网页可以有几 MB，整个塞进上下文会把对话挤爆。

use std::time::Duration;

use serde_json::json;

use super::tool::{ToolCallRequest, ToolFailure, ToolFailureKind, ToolOutcome, ToolSpec};
use super::{RuntimeContext, ToolHandler, ToolKind};
use crate::mcp::Operation;

/// 单次抓取的字符上限。超出就截断并说明。
const MAX_CHARS: usize = 60_000;
const TIMEOUT: Duration = Duration::from_secs(20);

pub struct FetchUrlTool;

/// 把 GitHub 的网页地址换成能直接拿到源码的 raw 地址。
///
/// `https://github.com/o/r/blob/main/src/x.ts`
///   → `https://raw.githubusercontent.com/o/r/main/src/x.ts`
///
/// 不这么做的话抓回来的是一整页应用外壳（几十 KB 的脚本和样式），
/// 而模型真正要的那几十行源码埋在里面。
pub fn rewrite_github_blob(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))?;
    let (owner_repo, tail) = rest.split_once("/blob/")?;
    Some(format!(
        "https://raw.githubusercontent.com/{owner_repo}/{tail}"
    ))
}

/// 剥掉 HTML 标签，留下可读文本。
///
/// `<script>` 和 `<style>` 的**内容**要整段丢掉 —— 只剥标签的话，
/// 一页 React 应用会剩下几十 KB 的压缩 JS，那比不剥还糟。
pub fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let bytes = html.as_bytes();
    let lower = html.to_ascii_lowercase();
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i] == b'<' {
            // script / style：连内容一起跳过
            let mut skipped = false;
            for tag in ["script", "style"] {
                let open = format!("<{tag}");
                if lower[i..].starts_with(&open) {
                    let close = format!("</{tag}>");
                    match lower[i..].find(&close) {
                        Some(rel) => i += rel + close.len(),
                        None => i = bytes.len(),
                    }
                    skipped = true;
                    break;
                }
            }
            if skipped {
                continue;
            }
            // 普通标签：跳到 '>'，块级标签留个换行
            let block = ["</p", "</div", "</li", "</tr", "</h", "<br", "</pre"]
                .iter()
                .any(|t| lower[i..].starts_with(t));
            match html[i..].find('>') {
                Some(rel) => i += rel + 1,
                None => break,
            }
            if block && !out.ends_with('\n') {
                out.push('\n');
            }
            continue;
        }
        let ch = html[i..].chars().next().unwrap_or(' ');
        out.push(ch);
        i += ch.len_utf8();
    }

    decode_entities(&collapse_blank_lines(&out))
}

fn decode_entities(s: &str) -> String {
    s.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&") // 必须最后，否则 &amp;lt; 会被解成 <
}

fn collapse_blank_lines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blanks = 0;
    for line in s.lines() {
        let t = line.trim_end();
        if t.trim().is_empty() {
            blanks += 1;
            if blanks > 1 {
                continue;
            }
            out.push('\n');
        } else {
            blanks = 0;
            out.push_str(t);
            out.push('\n');
        }
    }
    out.trim().to_string()
}

#[async_trait::async_trait]
impl ToolHandler for FetchUrlTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "fetch_url",
            "Fetch a URL and return its content as plain text. Use this to read \
             documentation, source files, or any web page — NEVER shell out to \
             `git clone`/`curl`/`wget` just to read something. GitHub blob URLs are \
             automatically rewritten to raw so you get the source, not the page shell.",
            ToolKind::WebSearch,
            Operation::Read,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "The absolute http(s) URL to fetch" }
            },
            "required": ["url"]
        }))
        .with_timeout(Duration::from_secs(25))
    }

    async fn execute(&self, _ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let started = std::time::Instant::now();
        let Some(url) = request.arguments.get("url").and_then(|v| v.as_str()) else {
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(ToolFailureKind::InvalidParams, "缺少 url 参数"),
                Duration::from_millis(0),
            );
        };
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::InvalidParams,
                    format!("只接受 http(s) 地址，收到的是：{url}"),
                ),
                started.elapsed(),
            );
        }

        let effective = rewrite_github_blob(url).unwrap_or_else(|| url.to_string());
        let client = match reqwest::Client::builder().timeout(TIMEOUT).build() {
            Ok(c) => c,
            Err(e) => {
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(ToolFailureKind::Execution, format!("构造 HTTP 客户端失败：{e}")),
                    started.elapsed(),
                )
            }
        };

        let resp = match client
            .get(&effective)
            .header("User-Agent", "Anima/0.8 (+local agent)")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(ToolFailureKind::Execution, format!("请求失败：{e}"))
                        // 明确告诉模型别退回 shell —— 那正是它上次做的事
                        .with_hint("网络问题请直接如实告诉用户，不要改用 git clone / curl 反复重试。"),
                    started.elapsed(),
                )
            }
        };

        let status = resp.status();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        if !status.is_success() {
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::Execution,
                    format!("HTTP {status}：{effective}"),
                ),
                started.elapsed(),
            );
        }

        let body = match resp.text().await {
            Ok(b) => b,
            Err(e) => {
                return ToolOutcome::failed(
                    &request,
                    ToolFailure::new(ToolFailureKind::Execution, format!("读取响应体失败：{e}")),
                    started.elapsed(),
                )
            }
        };

        let text = if content_type.contains("text/html") {
            html_to_text(&body)
        } else {
            body
        };

        let total = text.chars().count();
        let (content, note) = if total > MAX_CHARS {
            (
                text.chars().take(MAX_CHARS).collect::<String>(),
                format!("\n\n[已截断：共 {total} 字符，只返回前 {MAX_CHARS} 个]"),
            )
        } else {
            (text, String::new())
        };

        let header = if effective != url {
            format!("# {effective}\n（由 {url} 自动改写为 raw）\n\n")
        } else {
            format!("# {effective}\n\n")
        };

        ToolOutcome::ok(
            &request,
            format!("{header}{content}{note}"),
            Vec::new(),
            started.elapsed(),
        )
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    /// GitHub 的 blob 地址必须换成 raw —— 否则抓回来的是一整页应用外壳，
    /// 模型真正要的源码埋在几十 KB 的脚本里。
    #[test]
    fn github_blob_换成_raw() {
        assert_eq!(
            rewrite_github_blob("https://github.com/catterchy/liuyao/blob/main/src/paipan.ts"),
            Some("https://raw.githubusercontent.com/catterchy/liuyao/main/src/paipan.ts".into())
        );
        // 正名：不是 blob 的一律不动
        assert_eq!(rewrite_github_blob("https://github.com/catterchy/liuyao"), None);
        assert_eq!(rewrite_github_blob("https://example.com/blob/x"), None);
    }

    /// `<script>` / `<style>` 要**连内容一起**丢掉。
    /// 只剥标签的话，一页 React 应用会剩下几十 KB 压缩 JS，比不剥还糟。
    #[test]
    fn 脚本和样式整段丢掉() {
        let html = "<html><head><style>body{color:red}</style>\
                    <script>var a=1;function f(){return 2}</script></head>\
                    <body><p>正文一</p><p>正文二</p></body></html>";
        let text = html_to_text(html);
        assert!(!text.contains("color:red"), "样式内容漏出来了：{text}");
        assert!(!text.contains("var a=1"), "脚本内容漏出来了：{text}");
        assert!(text.contains("正文一") && text.contains("正文二"), "正文丢了：{text}");
    }

    /// 块级标签之间要断行，否则整页挤成一行没法读。
    #[test]
    fn 块级标签之间断行() {
        let text = html_to_text("<p>第一段</p><p>第二段</p>");
        assert!(text.contains('\n'), "没有断行：{text:?}");
    }

    /// 实体解码顺序：`&amp;` 必须最后，否则 `&amp;lt;` 会被解成 `<`。
    #[test]
    fn 实体解码顺序不能反() {
        assert_eq!(html_to_text("<p>a &amp;lt; b</p>").trim(), "a &lt; b");
        assert_eq!(html_to_text("<p>x &lt; y &amp; z</p>").trim(), "x < y & z");
    }
}
