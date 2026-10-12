//! 无头工具注册表。**测试和 `anima-mcp` 二进制用的是同一个函数** ——
//! 各建各的话，测试通过而线上少一个工具，谁也看不出来。

use std::path::Path;

use crate::agent_runtime::ToolRegistry;

/// 建一个不需要 `AppHandle` 的注册表。
///
/// 三组来源，和应用内那套完全一致 —— 只是 `app` 传 `None`：
/// 基础工具（含出图出片）、Studio 工具、ComfyUI 只读状态。
///
/// **危险工具照样注册进来**（shell / 文件），因为这是同一份注册表；
/// 朝外暴露哪些由 [`super::expose`] 单独把关。两件事分开，
/// 是为了让「暴露与否」只有一个判断点。
pub fn headless(config_dir: &Path, gateway: Option<std::sync::Arc<crate::llm::gateway::Gateway>>) -> ToolRegistry {
    let mut reg = ToolRegistry::new();
    crate::agent_runtime::local_tools::register_basic_local_tools(&mut reg);
    crate::agent_runtime::studio_provider::register_studio_tools(&mut reg, config_dir, None, gateway);
    crate::agent_runtime::comfyui_provider::register_comfyui_tools(&mut reg, config_dir);
    reg
}
