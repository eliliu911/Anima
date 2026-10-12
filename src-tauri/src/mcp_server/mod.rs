//! **MCP 服务端那一面** —— 让 Claude（或任何 MCP 客户端）直接调用 Anima。
//!
//! Anima 一直只有 MCP 的**客户端**那一面（`crate::mcp`，把别人的工具接进来）。
//! 这里是反过来的那一面：把 Anima 自己的工具朝外暴露。
//!
//! # 为什么几乎不用写新代码
//!
//! `ToolSpec` 已经有 `name` / `description` / `parameters`（JSON Schema）——
//! **正好就是 MCP 的工具形状**。所以这里做的是接线，不是重写：
//!
//! - [`expose`]：反向白名单，决定哪些工具能出去（安全的那一半在这里）
//! - [`protocol`]：JSON-RPC 2.0 的收发，纯函数，可离线测
//! - `bin/anima-mcp.rs`：把两者接到 stdio 上，用一个**无头**注册表
//!
//! # 无头，不是连着跑着的应用
//!
//! 用的是 `register_basic_local_tools` + `register_studio_tools(.., None, None)` ——
//! 两者都不需要 `AppHandle`。所以 `anima-mcp` 不要求 Anima 正在运行。
//!
//! **GPU 仍然是同一条队列**：出图出片最终都排进同一个本机 ComfyUI，
//! 而 ComfyUI 本身是串行的。这一条是 2026-09-16 那次内核 panic 的要求。
//!
//! # 已知短板（写在这儿免得当成 bug 查）
//!
//! - **长任务是阻塞的**：`generate_image` 单张 45s、`studio_render_episode`
//!   整集几十分钟。渲染类的走「起任务 + `studio_job_status` 轮询」，
//!   但 `generate_image` 目前是一路等到底，客户端自己的超时可能先到。
//! - **没有鉴权**：stdio 传输的信任边界就是「谁能起这个进程」。
//!   将来要是加 TCP/HTTP 传输，**必须**同时加 token，否则同机任何进程都能连。

pub mod expose;
pub mod registry;
pub mod protocol;
