//! MCP (Model Context Protocol) 客户端。
//!
//! Anima 作为 MCP **客户端**接入外部工具：本地 server 走子进程 stdio，
//! 远程 server 走 streamable HTTP。用户自己填配置就能接入，不需要改代码
//! （对齐 Claude Desktop 的体验）。
//!
//! 模块划分：
//!   - [`permission`]：CRUD 粒度的权限闸门。**唯一的安全边界**，默认值全部往
//!     安全侧倒（见 ARCHITECTURE.md 坑点 6）。
//!   - [`guard`]：把权限、审批模式、命令白名单合成一次调用的裁决。
//!     终端类工具的风险在**参数**里而不在工具名里，所以必须有这一层。
//!   - [`config`]：server 配置与持久化。
//!
//! 网关侧的通路早就备好了：`ChatRequest.tools`、流式 tool_call 的归一化、
//! 以及"后端不支持流式工具调用时自动回退到云端"都已实现并有测试覆盖。

pub mod client;
pub mod config;
pub mod guard;
pub mod permission;
pub mod prompts;
pub mod registry;
pub mod resources;

pub use client::{McpClient, McpTool};
pub use config::{McpServerConfig, McpTransport};
pub use guard::{ApprovalMode, CommandPolicy, Guard, Outcome, Risk, Verdict};
pub use permission::{Access, Decision, DecisionReason, Operation, ServerPermissions};
pub use registry::{ApprovedCall, Authorization, McpRegistry, PendingCall, ServerStatus, ToolInfo};
