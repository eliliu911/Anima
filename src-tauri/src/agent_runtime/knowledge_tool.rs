//! `search_knowledge`：让 agent 在项目知识库里做语义检索。
//!
//! ## 为什么必须是工具，而不是每轮注入
//!
//! `.anima/knowledge/` 是用户主动放进去的领域文档（需求说明、接口定义、
//! 角色设定），量可以很大。每轮都往 System Prompt 里塞一段，会同时坏两件事：
//! 首字延迟（几千字符的 prefill）和相关性（大多数轮次根本不需要它）。
//!
//! 做成工具则是模型按需去取 —— 代价是它得先知道有这个工具，
//! 所以 `description` 里明确写了"在动手写代码之前"这个时机。
//!
//! ## 为什么工具自己持有记忆句柄
//!
//! 记忆系统是**惰性初始化**的（要先探嵌入后端的维度才能建库，而应用启动时
//! 后端可能还没起来），所以 `AppState` 里存的是一个
//! `Arc<RwLock<Option<Arc<Memory>>>>` 空格子。
//!
//! 这个工具持有**同一个格子**而不是 `Arc<Memory>`：注册发生在启动期、
//! 那时格子还是空的，而调用发生在对话期、那时已经填好了。
//! 换成在 `RuntimeContext` 上加字段也能做到，但那要动 `RuntimeContext::new`
//! 的每一个调用点（含大量测试），而收益只是把同一个格子换个地方放。

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::json;

use super::local_tools::{invalid_params, string_arg};
use super::{
    RuntimeContext, ToolCallRequest, ToolFailure, ToolFailureKind, ToolHandler, ToolKind,
    ToolOutcome, ToolSpec,
};
use crate::mcp::Operation;
use crate::memory::Memory;

/// 惰性初始化的记忆系统格子。与 `AppState::memory` 是同一个。
pub type MemoryCell = Arc<tokio::sync::RwLock<Option<Arc<Memory>>>>;

/// 一次最多返回几块。
///
/// 3 块 × 800 字符上限 ≈ 2400 字符 ≈ 1000 token。再多就开始和
/// 工具结果预算（`RECENT_TURN_TOOL_BUDGET = 16000`）抢地方，
/// 而第 4 块之后的相似度通常已经掉到没有价值。
const DEFAULT_LIMIT: usize = 3;
const MAX_LIMIT: usize = 8;

pub struct SearchKnowledgeTool {
    memory: MemoryCell,
}

impl SearchKnowledgeTool {
    pub fn new(memory: MemoryCell) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl ToolHandler for SearchKnowledgeTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(
            "search_knowledge",
            "Perform semantic vector search in the project's local knowledge base (.anima/knowledge/). \
             Retrieves the most relevant document chunks, specs, guidelines, and context. \
             Use natural language query describing what you need to know before writing code or making design decisions.",
            ToolKind::FileSystem,
            Operation::Read,
        )
        .with_parameters(json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural language query describing what information is needed (e.g. 'error code conventions for login api')"
                },
                "limit": {
                    "type": "integer",
                    "default": DEFAULT_LIMIT,
                    "description": "Maximum number of chunks to return (default 3, max 8)"
                }
            },
            "required": ["query"]
        }))
        .with_idempotent(true)
        .with_timeout(Duration::from_secs(30))
    }

    async fn execute(&self, ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let started = std::time::Instant::now();
        let Some(query) = string_arg(&request.arguments, "query") else {
            return invalid_params(&request, "缺少 query 参数");
        };
        let limit = request
            .arguments
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(DEFAULT_LIMIT)
            .clamp(1, MAX_LIMIT);

        let memory = { self.memory.read().await.clone() };
        let Some(memory) = memory else {
            // **说清楚是哪一环没起来。** 只说"检索失败"的话，模型会当成
            // "知识库里没有"，然后接着按自己的假设往下做。
            return ToolOutcome::failed(
                &request,
                ToolFailure::new(
                    ToolFailureKind::MissingDependency,
                    "记忆系统尚未初始化（嵌入后端不可用或记忆已关闭），知识库检索不可用。\
                     这不代表知识库为空 —— 不要据此假定项目没有相关文档，\
                     改用 view_file 直接读 .anima/knowledge/ 下的文件。",
                ),
                started.elapsed(),
            );
        };

        let workspace = ctx.workspace_root.to_string_lossy().to_string();
        match memory.search_knowledge(&workspace, query, limit).await {
            Ok(hits) if hits.is_empty() => ToolOutcome::ok(
                &request,
                // 区分"没索引"和"索引了但不相关"：前者要去点重建，后者是真的没有。
                // 混成一句"未找到"会让用户以为知识库坏了。
                format!(
                    "知识库里没有与「{query}」相关的内容。\n\n\
                     可能是还没建索引（在项目设置里点「重建向量索引」），\
                     也可能是 .anima/knowledge/ 里确实没有相关文档 —— \
                     可以用 list_directory 看一眼那个目录里有什么。"
                ),
                Vec::new(),
                started.elapsed(),
            ),
            Ok(hits) => {
                let mut out = format!("知识库命中 {} 块：\n", hits.len());
                for h in &hits {
                    // 出处和相似度都给出来：模型才能判断该多信这一块，
                    // 而用户在工具结果里也能看出召回质量
                    out.push_str(&format!(
                        "\n── {} #{} （相似度 {:.2}）──\n{}\n",
                        h.chunk.doc, h.chunk.index, h.similarity, h.chunk.text
                    ));
                }
                ToolOutcome::ok(&request, out, Vec::new(), started.elapsed())
            }
            Err(err) => ToolOutcome::failed(
                &request,
                ToolFailure::new(ToolFailureKind::Execution, format!("知识库检索失败：{err}")),
                started.elapsed(),
            ),
        }
    }
}
