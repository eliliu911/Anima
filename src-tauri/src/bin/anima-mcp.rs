//! `anima-mcp` —— 让 Claude（或任何 MCP 客户端）直接调用 Anima。
//!
//! 一个 stdio 的 MCP 服务端：每行一个 JSON-RPC 请求进来、每行一个响应出去。
//!
//! ```bash
//! claude mcp add anima -- /path/to/anima-mcp
//! ```
//!
//! # 三件写错就会静默出事的
//!
//! **一、stdout 是协议信道，一个字节都不能污染。** 任何 `println!`、
//! 任何写到 stdout 的日志都会被客户端当成半条 JSON-RPC 消息，
//! 表现是「服务器无响应」或者「协议错误」，而不是「日志多了一行」。
//! 所以日志**全部走 stderr**。
//!
//! **二、无头，不需要 Anima 在跑。** 用的是
//! [`anima_lib::mcp_server::registry::headless`]，不要 `AppHandle`。
//! 但有 5 个工具确实需要界面或者会往共享的 `jobs.json` 里写，
//! 它们由 `expose::HEADLESS_BLOCKED` 挡掉（有测试读源码核对）。
//!
//! **三、密钥要先加载。** 这个进程是被 MCP 客户端拉起来的，
//! 很可能没有任何环境变量 —— 和打包版双击启动是同一种处境。
//! 不先 `secrets::init` 的话所有云端后端都用不了，而且不报错。
//!
//! # GPU 仍然是同一条队列
//!
//! 出图出片最终都排进同一个本机 ComfyUI，而 ComfyUI 本身串行执行。
//! 这一条是 2026-09-16 那次内核 panic 之后定的要求。

use std::sync::Arc;

use anima_lib::agent_runtime::{RuntimeContext, ToolCallRequest, ToolRegistry, ToolRouter, ToolStatus};
use anima_lib::mcp_server::{expose, protocol, registry};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// 这个进程永远是无头的。留成常量是为了让读代码的人一眼看到前提。
const HEADLESS: bool = true;

struct AnimaTools {
    router: ToolRouter,
    registry: ToolRegistry,
    ctx: RuntimeContext,
}

#[async_trait::async_trait]
impl protocol::Tools for AnimaTools {
    fn list(&self) -> Vec<(String, String, Value)> {
        let mut out: Vec<(String, String, Value)> = self
            .registry
            .specs()
            .into_iter()
            .filter(|s| expose::exposed_when(s, HEADLESS))
            .map(|s| (s.name, s.description, s.parameters))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    async fn call(&self, name: &str, args: Value) -> (String, bool) {
        // **再查一遍。** 列表里没列不等于对面不会调 —— 见 expose::check_call
        if let Err(why) = expose::check_call(self.registry.spec(name).as_ref(), HEADLESS) {
            return (why, true);
        }
        let outcome = self
            .router
            .dispatch(&self.ctx, ToolCallRequest::new(name, args))
            .await;
        let failed = outcome.status != ToolStatus::Ok;
        let mut text = outcome.content;
        if let Some(f) = outcome.failure {
            // 失败原因要带出去。**只回一句「失败」的话模型只能瞎改参数重试**
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&format!("{:?}: {}", f.kind, f.message));
            if let Some(h) = &f.hint {
                text.push_str(&format!("\n提示：{h}"));
            }
        }
        if !outcome.artifacts.is_empty() {
            text.push_str(&format!("\n产物：{}", outcome.artifacts.join(", ")));
        }
        (text, failed)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // **日志只走 stderr。** 写到 stdout 会污染协议信道
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let config_dir = std::env::var("ANIMA_CONFIG_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::data_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join("com.anima.desktop")
        });

    // 这个进程是被 MCP 客户端拉起来的，很可能一个环境变量都没有
    if let Err(e) = anima_lib::config::secrets::init(&config_dir) {
        tracing::error!(error = %e, "密钥没加载成功，需要 key 的后端会用不了");
    }

    // 网关给那些要后台模型的工具用（抽实体、合成提示词）。建不起来也照常开，
    // 只是那几个工具会失败 —— 比整个服务端起不来好
    let gateway = anima_lib::config::AppConfig::load(&config_dir.join("config.json"))
        .ok()
        .and_then(|c| anima_lib::llm::gateway::Gateway::new(c).ok())
        .map(Arc::new);
    if gateway.is_none() {
        tracing::warn!("网关没建起来，要后台模型的工具会失败");
    }

    let reg = registry::headless(&config_dir, gateway);
    let tools = AnimaTools {
        router: ToolRouter::new(reg.clone()),
        registry: reg,
        // **工作区就是 Anima 自己的数据目录，而且是只读策略。**
        // 暴露出去的工具一个都不是文件类（白名单挡着），这里给只读是第三道保险 ——
        // 万一将来有人往白名单里加了个会写文件的工具，它也出不了这个目录。
        ctx: RuntimeContext::new(&config_dir, anima_lib::agent_runtime::RuntimePolicy::read_only(&config_dir)),
    };
    tracing::info!(
        config_dir = %config_dir.display(),
        exposed = tools_count(&tools),
        "anima-mcp 起来了"
    );

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut out = tokio::io::stdout();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let reply = match protocol::parse_line(&line) {
            Ok(req) => protocol::handle(req, &tools, env!("CARGO_PKG_VERSION")).await,
            Err(e) => Some(e),
        };
        // 通知没有回复 —— 回了严格的客户端会报协议错误
        if let Some(reply) = reply {
            out.write_all(serde_json::to_string(&reply)?.as_bytes()).await?;
            out.write_all(b"\n").await?;
            out.flush().await?;
        }
    }
    Ok(())
}

fn tools_count(t: &AnimaTools) -> usize {
    protocol::Tools::list(t).len()
}
