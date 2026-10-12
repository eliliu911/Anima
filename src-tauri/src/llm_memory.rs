//! 出片和后台模型抢内存：出片前卸载 LM Studio 里的模型，要用后台模型时再加载回来。
//!
//! # 为什么
//!
//! 2026-09-14 整集批量出片：LM Studio 的后台模型（qwen3.6 35B）出片期间一直常驻 26 GB，
//! 系统交换区用到 10.8 / 12 GB；H3 参考模式峰值约 54 GB。一镜 6 秒要 30 分钟。
//!
//! # 为什么还要「加载回来」
//!
//! 本机 LM Studio 关着「按需加载」（`justInTimeModelLoading: false`）：卸载之后，
//! 生成镜头表、合成提示词会直接报「模型没加载」。所以这两件事开工前先确认模型在内存里。
//!
//! 用 LM Studio 自带的 `lms` 命令行。找不到它、后台模型不走 LM Studio 时什么都不做 ——
//! 这是省内存的优化，不是出片的前提，失败只记日志不报错。

use std::path::{Path, PathBuf};
use std::process::Command;

fn lms_bin() -> Option<PathBuf> {
    let p = dirs::home_dir()?.join(".lmstudio/bin/lms");
    p.is_file().then_some(p)
}

/// 后台模型是不是走 LM Studio；是的话返回模型名。
fn background_lmstudio_model(config_dir: &Path) -> Option<String> {
    let cfg = crate::config::AppConfig::load(&config_dir.join("config.json")).ok()?;
    (cfg.background_provider == "lmstudio").then_some(())?;
    cfg.background_model.filter(|m| !m.trim().is_empty())
}

/// Chat（实时）模型是不是走 LM Studio；是的话返回模型名。出片前**不卸它**。
fn realtime_lmstudio_model(config_dir: &Path) -> Option<String> {
    let cfg = crate::config::AppConfig::load(&config_dir.join("config.json")).ok()?;
    (cfg.realtime_provider == "lmstudio").then_some(())?;
    cfg.realtime_model.filter(|m| !m.trim().is_empty())
}

/// 模型名里写的参数量（B）。「ornith-1.5-35b-a3b」→ 35（取最大的那个，`a3b` 是激活量不算），认不出回 `None`
pub(crate) fn param_billions(model: &str) -> Option<f64> {
    model
        .to_lowercase()
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '.'))
        .filter_map(|t| t.strip_suffix('b').filter(|n| n.chars().next().is_some_and(|c| c.is_ascii_digit())).and_then(|n| n.parse::<f64>().ok()))
        .fold(None, |m: Option<f64>, x| Some(m.map_or(x, |m| m.max(x))))
}

/// Chat 用的是本机大模型（名字上 ≥ 14B）时回模型名。
///
/// 出片前只卸后台模型、保留 Chat 的，于是出片峰值（H3 参考模式约 54 GB）要和它挤。用户定的做法是出片期间
/// Chat 换小模型或 Ollama Cloud —— 不自动切，由 Chat 的出片工具在结果里提醒一句（方案 §4.4）。
pub fn big_local_chat_model(config_dir: &Path) -> Option<String> {
    let cfg = crate::config::AppConfig::load(&config_dir.join("config.json")).ok()?;
    let model = cfg.realtime_model.filter(|m| !m.trim().is_empty())?;
    let local = matches!(cfg.realtime_provider.as_str(), "lmstudio" | "ollama") && !model.to_lowercase().contains("cloud");
    (local && param_billions(&model).is_some_and(|b| b >= 14.0)).then_some(model)
}

/// 从 `lms ps --json` 里挑出要卸的模型：除了 Chat 模型全卸。
pub(crate) fn models_to_unload(ps: &serde_json::Value, keep: Option<&str>) -> Vec<String> {
    let Some(list) = ps.as_array() else { return Vec::new() };
    list.iter()
        .filter(|m| !keep.is_some_and(|k| is_loaded(&serde_json::json!([m]), k)))
        .filter_map(|m| m.get("identifier").and_then(|v| v.as_str()).map(String::from))
        .collect()
}

/// 出片前调用：卸载 LM Studio 里 **Chat 模型以外** 的模型。
///
/// 以前是 `lms unload --all`，把 Chat 自己的模型一起卸了 —— Chat 接 Studio 之后，
/// 在 Chat 里发起出片等于把 Chat 的模型卸掉（2026-09-15 核对）。Chat 和后台本来是两个模型，
/// 用户的做法是 Chat 用小模型或 Ollama Cloud，所以只卸后台模型和闲置的其他模型。
///
/// **有生成镜头表 / 合成提示词的任务在跑时不卸** —— 卸了它们会半路报错。
pub fn free_for_video(config_dir: &Path) {
    let busy = crate::jobs::all_running(config_dir)
        .iter()
        .any(|j| matches!(j.kind, crate::jobs::JobKind::ShotTable | crate::jobs::JobKind::Prompts));
    if busy {
        tracing::info!("有生成镜头表 / 合成提示词的任务在跑，出片前不卸载后台模型");
        return;
    }
    if background_lmstudio_model(config_dir).is_none() {
        return;
    }
    let Some(lms) = lms_bin() else { return };
    let ps = Command::new(&lms)
        .args(["ps", "--json"])
        .output()
        .ok()
        .and_then(|o| serde_json::from_slice::<serde_json::Value>(&o.stdout).ok())
        .unwrap_or_default();
    let keep = realtime_lmstudio_model(config_dir);
    for id in models_to_unload(&ps, keep.as_deref()) {
        match Command::new(&lms).args(["unload", &id]).output() {
            Ok(out) => tracing::info!(%id, ok = out.status.success(), "出片前卸载 LM Studio 模型（Chat 模型保留）"),
            Err(e) => tracing::warn!(%id, "出片前卸载 LM Studio 模型失败：{e}"),
        }
    }
}

/// 要用后台模型前调用：没加载就加载上。返回给用户看的错误（加载失败时后面的调用一定也会失败，早点说清楚）。
pub fn ensure_background_loaded(config_dir: &Path) -> std::result::Result<(), String> {
    let Some(model) = background_lmstudio_model(config_dir) else { return Ok(()) };
    let Some(lms) = lms_bin() else { return Ok(()) };
    let loaded = Command::new(&lms)
        .args(["ps", "--json"])
        .output()
        .ok()
        .and_then(|o| serde_json::from_slice::<serde_json::Value>(&o.stdout).ok())
        .is_some_and(|v| is_loaded(&v, &model));
    if loaded {
        return Ok(());
    }
    tracing::info!(%model, "后台模型不在内存里，加载");
    let out = Command::new(&lms).args(["load", &model, "--yes"]).output().map_err(|e| format!("加载后台模型失败：{e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("加载后台模型「{model}」失败：{}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// `lms ps --json` 的输出里有没有这个模型。按 identifier / modelKey / path 任一对上就算。
fn is_loaded(ps: &serde_json::Value, model: &str) -> bool {
    ps.as_array().is_some_and(|list| {
        list.iter().any(|m| {
            ["identifier", "modelKey", "path"].iter().any(|k| m.get(k).and_then(|v| v.as_str()).is_some_and(|v| v == model || v.ends_with(&format!("/{model}"))))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 认得出已加载的模型() {
        let ps = serde_json::json!([{"identifier": "qwen3.6-35b-a3b-uncensored-hauhaucs-aggressive", "modelKey": "qwen3.6-35b-a3b-uncensored-hauhaucs-aggressive", "path": "hauhaucs/qwen3.6-35b-a3b-uncensored-hauhaucs-aggressive"}]);
        assert!(is_loaded(&ps, "qwen3.6-35b-a3b-uncensored-hauhaucs-aggressive"));
        assert!(!is_loaded(&ps, "qwen/qwen3.8-27b"));
        assert!(!is_loaded(&serde_json::json!([]), "x"));
        assert!(!is_loaded(&serde_json::json!({"error": "x"}), "x"));
    }

    #[test]
    fn 从模型名认参数量() {
        assert_eq!(param_billions("ornith-1.5-35b-a3b"), Some(35.0));
        assert_eq!(param_billions("qwen3:32b"), Some(32.0));
        assert_eq!(param_billions("gemma-3-4b-it"), Some(4.0));
        assert_eq!(param_billions("gpt-oss-120b"), Some(120.0));
        assert_eq!(param_billions("llama3.1:8b-instruct"), Some(8.0));
        assert_eq!(param_billions("deepseek-v3"), None);
    }

    #[test]
    fn 出片前只卸_chat_以外的模型() {
        let ps = serde_json::json!([
            {"identifier": "openai/gpt-oss-120b", "modelKey": "openai/gpt-oss-120b"},
            {"identifier": "ornith-1.5-35b-a3b", "modelKey": "ornith-1.5-35b-a3b"},
            {"identifier": "qwen3.6-35b", "modelKey": "qwen3.6-35b", "path": "x/qwen3.6-35b"}
        ]);
        assert_eq!(models_to_unload(&ps, Some("ornith-1.5-35b-a3b")), ["openai/gpt-oss-120b", "qwen3.6-35b"]);
        assert_eq!(models_to_unload(&ps, None).len(), 3, "Chat 不走 LM Studio 时全卸");
        assert!(models_to_unload(&serde_json::json!({"error": 1}), None).is_empty());
    }

    /// 出片前卸载、用模型前加载，都要接在对的地方。
    #[test]
    fn 接线在出片和后台模型任务开工前() {
        let cinema = include_str!("cinema.rs");
        let reg = &cinema[cinema.find("async fn render_video_registered").unwrap()..];
        let reg = &reg[..reg.find("\n}\n").unwrap()];
        assert!(reg.contains("crate::llm_memory::free_for_video(config_dir);"));
        for f in ["pub fn start_shot_table(", "pub fn start_synthesize("] {
            let body = &cinema[cinema.find(f).unwrap()..];
            let body = &body[..body.find("\n}\n").unwrap()];
            assert!(body.contains("spawn_blocking(move || crate::llm_memory::ensure_background_loaded(&dir))"), "{f} 开工前没在后台确认后台模型已加载");
        }
    }
}
