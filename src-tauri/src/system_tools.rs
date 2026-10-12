use crate::agent_runtime::{
    RuntimeContext, ToolCallRequest, ToolFailure, ToolFailureKind, ToolHandler, ToolKind,
    ToolOutcome, ToolRegistry, ToolSpec,
};
use crate::audio::playback::Player;
use crate::llm::{Tool, ToolCall};
use crate::mcp::Operation;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct SystemToolDeps {
    pub app: AppHandle,
    pub http: reqwest::Client,
    pub music_player: Arc<Mutex<Option<Arc<Player>>>>,
}

pub fn register_runtime_tools(registry: &mut ToolRegistry, deps: SystemToolDeps) {
    for declaration in declarations() {
        registry.register(SystemRuntimeTool {
            spec: spec_from_declaration(&declaration),
            deps: deps.clone(),
        });
    }
}

pub fn declarations() -> Vec<Tool> {
    vec![
        Tool {
            name: "open_url_or_file".to_string(),
            description: "通过系统默认程序打开 URL 或文件绝对路径（如网页、文档、应用程序）。".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "target": { "type": "string", "description": "要打开的网址(https://...)或绝对路径" }
                },
                "required": ["target"]
            }),
        },
        Tool {
            name: "play_audio".to_string(),
            // **写清楚"要有确切地址"**：不然用户说"放点音乐"时模型会调这个，
            // 然后编一个不存在的 URL。要放背景音乐用 play_radio
            description: "播放一个**已知确切地址**的音频文件（本地绝对路径，或对方给出的直链 mp3/wav/flac）。\
                          用户只是想听点音乐、没有指定文件时，用 play_radio，不要用这个编地址。\
                          注意它会把整个文件下完再播，不能用于无尽的直播流。"
                .to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "url_or_path": { "type": "string" }
                },
                "required": ["url_or_path"]
            }),
        },
        Tool {
            name: "stop_audio".to_string(),
            description: "停止背景音乐播放。".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        Tool {
            name: "search_web".to_string(),
            description: "使用搜索引擎在互联网上搜索信息。".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "搜索关键词" }
                },
                "required": ["query"]
            }),
        },
    ]
}

pub fn is_system_tool(name: &str) -> bool {
    matches!(
        name,
        "open_url_or_file" | "play_audio" | "stop_audio" | "search_web"
    )
}

#[derive(Clone)]
struct SystemRuntimeTool {
    spec: ToolSpec,
    deps: SystemToolDeps,
}

#[async_trait::async_trait]
impl ToolHandler for SystemRuntimeTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    async fn execute(&self, _ctx: &RuntimeContext, request: ToolCallRequest) -> ToolOutcome {
        let started = Instant::now();
        let call = ToolCall {
            id: request.id.clone(),
            name: request.tool_name.clone(),
            arguments: serde_json::to_string(&request.arguments)
                .unwrap_or_else(|_| "{}".to_string()),
        };
        let (content, artifacts) = execute(
            &self.deps.app,
            &call,
            &self.deps.http,
            &self.deps.music_player,
        )
        .await;
        if is_success_message(&content) {
            ToolOutcome::ok(&request, content, artifacts, started.elapsed())
        } else {
            ToolOutcome::failed(
                &request,
                ToolFailure::new(classify_system_failure(&content), content),
                started.elapsed(),
            )
        }
    }
}

fn spec_from_declaration(tool: &Tool) -> ToolSpec {
    let operation = match tool.name.as_str() {
        "search_web" => Operation::Read,
        _ => Operation::Execute,
    };
    let kind = match tool.name.as_str() {
        "search_web" => ToolKind::WebSearch,
        _ => ToolKind::BuiltIn,
    };
    ToolSpec::new(tool.name.clone(), tool.description.clone(), kind, operation)
        .with_parameters(tool.parameters.clone())
        .with_timeout(Duration::from_secs(60))
}

fn is_success_message(content: &str) -> bool {
    !content.contains("失败")
        && !content.contains("错误")
        && !content.contains("缺少")
        && !content.contains("未能")
        && !content.contains("未知")
}

fn classify_system_failure(content: &str) -> ToolFailureKind {
    if content.contains("参数") || content.contains("缺少") {
        ToolFailureKind::InvalidParams
    } else if content.contains("下载") || content.contains("请求") || content.contains("搜索")
    {
        ToolFailureKind::Transport
    } else {
        ToolFailureKind::Execution
    }
}

pub async fn execute(
    app: &AppHandle,
    call: &ToolCall,
    http: &reqwest::Client,
    music_player: &Arc<Mutex<Option<Arc<Player>>>>,
) -> (String, Vec<String>) {
    let args: serde_json::Value = match serde_json::from_str(&call.arguments) {
        Ok(v) => v,
        Err(_) => return ("参数格式错误".to_string(), vec![]),
    };

    match call.name.as_str() {
        "open_url_or_file" => {
            let target = match args.get("target").and_then(|v| v.as_str()) {
                Some(t) => t,
                None => return ("缺少 target 参数".to_string(), vec![]),
            };
            if target.starts_with("http://") || target.starts_with("https://") {
                if let Err(e) = app.opener().open_url(target, None::<&str>) {
                    return (format!("打开网址失败：{e}"), vec![]);
                }
            } else {
                if let Err(e) = app.opener().open_path(target, None::<&str>) {
                    return (format!("打开文件失败：{e}"), vec![]);
                }
            }
            ("已请求系统打开目标".to_string(), vec![])
        }
        "play_audio" => {
            let target = match args.get("url_or_path").and_then(|v| v.as_str()) {
                Some(t) => t,
                None => return ("缺少 url_or_path 参数".to_string(), vec![]),
            };
            let local_target = target.strip_prefix("file://").unwrap_or(target);
            let is_remote = target.starts_with("http://") || target.starts_with("https://");

            let bytes = if is_remote {
                match http.get(target).send().await {
                    Ok(resp) => match resp.bytes().await {
                        Ok(b) => b.to_vec(),
                        Err(e) => return (format!("下载音频流失败：{e}"), vec![]),
                    },
                    Err(e) => return (format!("请求音频流失败：{e}"), vec![]),
                }
            } else {
                match std::fs::read(local_target) {
                    Ok(b) => b,
                    Err(e) => return (format!("读取本地音频文件失败：{e}"), vec![]),
                }
            };

            let playback_path = match prepare_webview_playback_audio(
                app,
                title_target(target, local_target, is_remote),
                if is_remote { None } else { Some(local_target) },
                &bytes,
            ) {
                Ok(Some(path)) => Some(path),
                Ok(None) => None,
                Err(e) => return (format!("准备前端音频播放失败：{e}"), vec![]),
            };

            // 前端 Audio 是主播放控制器，通知前端开始播放并唤起抽屉控制器。
            // WebView 对 FLAC 支持不稳定，所以这里可能会改发一个 WAV 代理文件；
            // 原始文件路径仍保留在 artifacts 里，避免资产引用被悄悄替换。
            let title_source = if is_remote { target } else { local_target };
            let file_name = std::path::Path::new(title_source)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| {
                    title_source
                        .rsplit('/')
                        .next()
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or("本地音频")
                        .to_string()
                });
            let frontend_local_path = playback_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string());
            let payload = if let Some(path) = frontend_local_path {
                serde_json::json!({
                    "title": file_name,
                    "local_path": path,
                    "source_path": if is_remote { serde_json::Value::Null } else { serde_json::json!(local_target) },
                    "duration": 30,
                })
            } else if is_remote {
                serde_json::json!({
                    "title": file_name,
                    "audio_url": target,
                    "duration": 30,
                })
            } else {
                serde_json::json!({
                    "title": file_name,
                    "local_path": local_target,
                    "duration": 30,
                })
            };
            let _ = app.emit("runtime://music_playing", payload);

            let artifacts = if is_remote {
                vec![]
            } else {
                vec![local_target.to_string()]
            };
            ("已将音频加入播放器并唤起抽屉控制器".to_string(), artifacts)
        }
        "stop_audio" => {
            let mut lock = music_player.lock().await;
            *lock = None;
            let _ = app.emit("runtime://music_stopped", ());
            ("已停止音乐播放".to_string(), vec![])
        }
        "search_web" => {
            let query = match args.get("query").and_then(|v| v.as_str()) {
                Some(q) => q,
                None => return ("缺少 query 参数".to_string(), vec![]),
            };

            let url = format!(
                "https://html.duckduckgo.com/html/?q={}",
                urlencoding::encode(query)
            );
            let res = match http.get(&url)
                .header("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:128.0) Gecko/20100101 Firefox/128.0")
                .send().await
            {
                Ok(r) => r,
                Err(e) => return (format!("搜索请求失败: {}", e), vec![]),
            };

            let html = match res.text().await {
                Ok(text) => text,
                Err(e) => return (format!("读取搜索结果失败: {}", e), vec![]),
            };

            let mut results = Vec::new();
            for block in html.split("class=\"result__snippet\"") {
                if block.starts_with(" href=") {
                    // Extract URL
                    let url_start = block.find('"').unwrap_or(0) + 1;
                    let url_end = block[url_start..].find('"').unwrap_or(0) + url_start;
                    let mut snippet_url = &block[url_start..url_end];
                    if snippet_url.starts_with("//duckduckgo.com/l/?uddg=") {
                        snippet_url = &snippet_url[25..];
                    }

                    // Extract text
                    if let Some(text_start) = block.find('>') {
                        if let Some(text_end) = block[text_start..].find("</a>") {
                            let text = &block[text_start + 1..text_start + text_end];
                            let clean_text = text
                                .replace("<b>", "")
                                .replace("</b>", "")
                                .replace("&quot;", "\"")
                                .replace("&#x27;", "'");
                            results.push(format!(
                                "- {}: {}",
                                clean_text.trim(),
                                urlencoding::decode(snippet_url)
                                    .unwrap_or_else(|_| snippet_url.into())
                            ));
                        }
                    }
                }
            }

            if results.is_empty() {
                (
                    "未能从页面解析出搜索结果，可能被拦截或无结果。".to_string(),
                    vec![],
                )
            } else {
                // 取前 5 条
                let top_results = results.into_iter().take(5).collect::<Vec<_>>().join("\n");
                (format!("搜索结果：\n{}", top_results), vec![])
            }
        }
        _ => ("未知的系统工具".to_string(), vec![]),
    }
}

fn title_target<'a>(target: &'a str, local_target: &'a str, is_remote: bool) -> &'a str {
    if is_remote { target } else { local_target }
}

fn prepare_webview_playback_audio(
    app: &AppHandle,
    title_source: &str,
    local_path: Option<&str>,
    bytes: &[u8],
) -> std::result::Result<Option<PathBuf>, String> {
    if is_webview_playable_audio(bytes) {
        return Ok(None);
    }
    if !is_flac_audio(bytes, title_source) {
        return Ok(None);
    }

    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("audio-playback");
    std::fs::create_dir_all(&cache_dir).map_err(|e| e.to_string())?;

    let stem = playback_cache_stem(title_source, bytes);
    let wav_path = cache_dir.join(format!("{stem}.wav"));
    if is_existing_playable_wav(&wav_path) {
        return Ok(Some(wav_path));
    }

    let input_path = if let Some(path) = local_path {
        PathBuf::from(path)
    } else {
        let source_path = cache_dir.join(format!("{stem}.flac"));
        std::fs::write(&source_path, bytes).map_err(|e| e.to_string())?;
        source_path
    };

    match transcode_to_webview_wav(&input_path, &wav_path) {
        Ok(()) => Ok(Some(wav_path)),
        Err(ffmpeg_err) => {
            let (samples, sample_rate) = crate::audio::playback::decode_mono(bytes.to_vec())
                .map_err(|decode_err| format!("{ffmpeg_err}; Rust 解码兜底也失败：{decode_err}"))?;
            let wav = crate::audio::playback::encode_mono_wav(&samples, sample_rate);
            std::fs::write(&wav_path, wav).map_err(|e| e.to_string())?;
            Ok(Some(wav_path))
        }
    }
}

fn transcode_to_webview_wav(input_path: &Path, wav_path: &Path) -> std::result::Result<(), String> {
    let output = Command::new(ffmpeg_binary())
        .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
        .arg(input_path)
        .args(["-vn", "-acodec", "pcm_s16le"])
        .arg(wav_path)
        .output()
        .map_err(|e| format!("调用 ffmpeg 转码失败：{e}"))?;
    if output.status.success() && is_existing_playable_wav(wav_path) {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(if stderr.is_empty() {
        "ffmpeg 转码失败：没有错误输出".to_string()
    } else {
        format!("ffmpeg 转码失败：{stderr}")
    })
}

fn ffmpeg_binary() -> &'static str {
    for candidate in ["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg"] {
        if Path::new(candidate).is_file() {
            return candidate;
        }
    }
    "ffmpeg"
}

fn is_webview_playable_audio(bytes: &[u8]) -> bool {
    is_wav_audio(bytes) || is_mp3_audio(bytes)
}

fn is_flac_audio(bytes: &[u8], source: &str) -> bool {
    bytes.starts_with(b"fLaC") || source.to_ascii_lowercase().ends_with(".flac")
}

fn is_mp3_audio(bytes: &[u8]) -> bool {
    bytes.starts_with(b"ID3") || matches!(bytes.get(0..2), Some([0xFF, b]) if b & 0xE0 == 0xE0)
}

fn is_wav_audio(bytes: &[u8]) -> bool {
    bytes.starts_with(b"RIFF") && matches!(bytes.get(8..12), Some(chunk) if chunk == b"WAVE")
}

fn is_existing_playable_wav(path: &Path) -> bool {
    std::fs::read(path)
        .map(|bytes| bytes.len() > 44 && is_wav_audio(&bytes))
        .unwrap_or(false)
}

fn playback_cache_stem(source: &str, bytes: &[u8]) -> String {
    let raw = Path::new(source)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("audio");
    let clean = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    bytes.len().hash(&mut hasher);
    bytes.get(..bytes.len().min(4096)).hash(&mut hasher);
    let tail_start = bytes.len().saturating_sub(4096);
    bytes.get(tail_start..).hash(&mut hasher);
    format!("{clean}_{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_declarations_map_to_runtime_specs() {
        let specs = declarations()
            .into_iter()
            .map(|tool| spec_from_declaration(&tool))
            .collect::<Vec<_>>();

        let search = specs
            .iter()
            .find(|spec| spec.name == "search_web")
            .expect("search_web spec");
        assert_eq!(search.kind, ToolKind::WebSearch);
        assert_eq!(search.operation, Operation::Read);

        let open = specs
            .iter()
            .find(|spec| spec.name == "open_url_or_file")
            .expect("open_url_or_file spec");
        assert_eq!(open.kind, ToolKind::BuiltIn);
        assert_eq!(open.operation, Operation::Execute);
    }

    #[test]
    fn flac_needs_a_webview_playback_proxy() {
        let wav = b"RIFF\x24\x00\x00\x00WAVEfmt ";
        let mp3 = b"ID3\x04\x00\x00\x00\x00\x00\x00";
        let flac = b"fLaC\x00\x00\x00\x22";

        assert!(is_webview_playable_audio(wav));
        assert!(is_webview_playable_audio(mp3));
        assert!(!is_webview_playable_audio(flac));
        assert!(is_flac_audio(flac, "music.flac"));
    }

    #[test]
    fn playback_cache_stem_is_safe_and_stable() {
        let stem = playback_cache_stem("甜歌 / demo.flac", b"fLaC bytes");

        assert!(stem.contains("demo_"), "{stem}");
        assert!(
            stem.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "{stem}"
        );
        assert_eq!(stem, playback_cache_stem("甜歌 / demo.flac", b"fLaC bytes"));
    }
}
