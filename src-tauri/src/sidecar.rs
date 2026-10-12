//! Python 后端 Sidecar 生命周期守护与自动启动管理器。
//!
//! ## 解决的核心问题
//! 1. **零手动操作**：应用启动时，Rust Core 自动探测并拉起 4D 神经数字人及本地语音（Kokoro / F5-TTS）Python 后端；
//! 2. **分发打包支持**：支持开发期（`tools/<name>/.venv`）与分发打包期（`resources/tools/<name>/.venv` 或系统 Python）自动路径定位；
//! 3. **防端口冲突**：启动前先探测健康检查端口，若用户或其他实例已在运行则无缝接管，避免重复拉起；
//! 4. **退出自动收尸**：应用退出（或托盘退出）时自动向所有拉起的子进程发送终止信号，绝不残留僵尸 Python 进程。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

/// 单个 Sidecar 服务定义
#[derive(Debug, Clone)]
pub struct SidecarDef {
    pub id: &'static str,
    pub name: &'static str,
    pub dir_rel: &'static str,
    pub script_rel: &'static str,
    pub default_port: u16,
    pub health_url: &'static str,
    pub args: &'static [&'static str],
    /// Rust 是否负责拉起它。
    ///
    /// `false` = **只监控，不托管**。有些后端的启动方式复杂得多
    /// （GPT-SoVITS 是个 clone 下来的上游仓库，由 scripts/anima.sh 拉起），
    /// 硬塞进这里只会启动失败。但它照样该出现在面板上 ——
    /// 「当前在用的引擎到底活着没有」是排查时第一个要看的。
    pub managed: bool,
}

/// 支持受控拉起的本地 Python 后端清单
pub static KNOWN_SIDECARS: &[SidecarDef] = &[
    SidecarDef {
        id: "digital-human",
        name: "4D 神经数字人推理引擎 (Metal/MPS)",
        dir_rel: "tools/digital-human",
        script_rel: "server.py",
        default_port: 8765,
        health_url: "http://127.0.0.1:8765/health",
        args: &[],
        managed: true,
    },
    SidecarDef {
        id: "kokoro-tts",
        name: "Kokoro 本地神经语音合成",
        dir_rel: "tools/kokoro-native",
        script_rel: "server.py",
        default_port: 8880,
        health_url: "http://127.0.0.1:8880/v1/models",
        args: &[],
        managed: true,
    },
    SidecarDef {
        id: "f5-tts",
        name: "F5-TTS 本地语音合成",
        dir_rel: "tools/f5-tts",
        script_rel: "server.py",
        // **端口是 8881，不是 9998；健康端点是 /v1/models，不是 /health。**
        // 写错的后果不是报错，是面板上永远停在「启动中」——
        // 健康检查一直敲一个没人应答的地址，而服务其实好好地跑着。
        // 它和 Kokoro 一样是 OpenAI 兼容接口，没有 /health 这个路由（实测 404）
        default_port: 8881,
        health_url: "http://127.0.0.1:8881/v1/models",
        args: &[],
        managed: true,
    },
    SidecarDef {
        id: "gpt-sovits",
        name: "GPT-SoVITS 零样本音色克隆",
        // 上游仓库，由 scripts/anima.sh 拉起 —— 这里只监控
        dir_rel: "tools/gpt-sovits",
        script_rel: "",
        default_port: 9880,
        // 它没有 /health，根路径也是 404；/docs 是唯一返回 2xx 的（实测）
        health_url: "http://127.0.0.1:9880/docs",
        args: &[],
        managed: false,
    },
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SidecarStatusInfo {
    pub id: String,
    pub name: String,
    pub port: u16,
    pub status: String, // "online" | "starting" | "stopped" | "external"
    pub pid: Option<u32>,
}

static RUNNING_PROCESSES: std::sync::OnceLock<Arc<Mutex<HashMap<String, Child>>>> =
    std::sync::OnceLock::new();

fn get_running_processes() -> &'static Arc<Mutex<HashMap<String, Child>>> {
    RUNNING_PROCESSES.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

/// `tools/` 的所在目录，也就是那五个 Python 服务的根。
///
/// 候选顺序和其它项目目录**共用同一份**（[`crate::app_root`]）——
/// 各写一份就是这次出事的原因：2026-09-18 我只修了这里的 `tools/`，
/// 而 `characters/` / `skills/` / `voice-library/` / `avatars/` 四处
/// 用的是同样的老写法，打包版里全都找不到。
///
/// **找不到就是 `None`，不是 `"."`** —— 返回一个必定错的路径只会让后面
/// 每一步都静默失败（原来就是这样：本地语音引擎一个都起不来，
/// 而 `start_all_sidecars` 只在 `debug!` 级别记一行）。
pub fn find_app_root(_app: Option<&AppHandle>, config_dir: Option<&Path>) -> Option<PathBuf> {
    crate::app_root::find_plain("tools", config_dir).and_then(|t| t.parent().map(Path::to_path_buf))
}

/// 找不到时给人看的话。
pub fn app_root_missing_hint() -> String {
    crate::app_root::missing_hint(
        "tools",
        "本地语音引擎（Kokoro / GPT-SoVITS / F5-TTS / 数字人）全部不可用，会退回 Edge TTS",
    )
}

/// 定位指定 Sidecar 的 Python 解释器绝对路径
fn resolve_python_interpreter(sidecar_dir: &Path) -> Option<PathBuf> {
    // 1. 优先使用专属虚拟环境 .venv
    #[cfg(windows)]
    let venv_py = sidecar_dir.join(".venv").join("Scripts").join("python.exe");
    #[cfg(not(windows))]
    let venv_py = sidecar_dir.join(".venv").join("bin").join("python");

    if venv_py.exists() {
        return Some(venv_py);
    }

    // 2. 检查 uv-managed python
    if let Ok(output) = Command::new("which").arg("python3").output() {
        if output.status.success() {
            let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path_str.is_empty() {
                return Some(PathBuf::from(path_str));
            }
        }
    }

    // 3. 兜底尝试 python3 或 python
    Some(PathBuf::from("python3"))
}

/// 快速异步探测服务健康检查端口
pub async fn check_health(url: &str) -> bool {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(600))
        .build();

    if let Ok(c) = client {
        if let Ok(resp) = c.get(url).send().await {
            return resp.status().is_success();
        }
    }
    false
}

/// 启动全部已知 Sidecar 服务
pub fn start_all_sidecars(app: AppHandle, config_dir: PathBuf) {
    tauri::async_runtime::spawn(async move {
        // **找不到就明说。** 原来这里会拿到 `"."` 然后一路 `debug!` 跳过每一个，
        // 表现是「本地语音引擎装了就是用不了，没人知道为什么」
        let Some(root) = find_app_root(Some(&app), Some(&config_dir)) else {
            tracing::warn!("{}", app_root_missing_hint());
            return;
        };
        tracing::info!(root = %root.display(), "Sidecar 管理器初始化");

        for def in KNOWN_SIDECARS {
            if !def.managed {
                tracing::debug!(id = def.id, "只监控不托管，跳过拉起");
                continue;
            }
            let sidecar_dir = root.join(def.dir_rel);
            let script_path = sidecar_dir.join(def.script_rel);

            if !sidecar_dir.exists() || !script_path.exists() {
                tracing::debug!(id = def.id, path = %script_path.display(), "Sidecar 目录或入口脚本不存在，跳过拉起");
                continue;
            }

            // 1. 先检查是否已经在运行
            if check_health(def.health_url).await {
                tracing::info!(
                    id = def.id,
                    port = def.default_port,
                    "Sidecar 服务已在线（沿用外部运行实例）"
                );
                continue;
            }

            // 2. 解析 Python 解释器
            let py_bin = match resolve_python_interpreter(&sidecar_dir) {
                Some(p) => p,
                None => {
                    tracing::warn!(id = def.id, "未找到有效的 Python 解释器环境，跳过自动拉起");
                    continue;
                }
            };

            tracing::info!(
                id = def.id,
                name = def.name,
                py = %py_bin.display(),
                script = %script_path.display(),
                "正在自动启动本地 Sidecar 后端..."
            );

            // 3. 构建子进程命令
            let mut cmd = Command::new(&py_bin);
            cmd.arg(def.script_rel)
                .args(def.args)
                .current_dir(&sidecar_dir)
                .stdout(Stdio::null())
                .stderr(Stdio::null());

            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                cmd.process_group(0);
            }

            match cmd.spawn() {
                Ok(child) => {
                    let pid = child.id();
                    tracing::info!(id = def.id, pid, "Sidecar 子进程已成功拉起");

                    if let Ok(mut guard) = get_running_processes().lock() {
                        guard.insert(def.id.to_string(), child);
                    }

                    // 4. 后台等待服务就绪日志
                    let health_url = def.health_url.to_string();
                    let def_id = def.id.to_string();
                    let def_name = def.name.to_string();
                    let def_port = def.default_port;

                    tauri::async_runtime::spawn(async move {
                        for _ in 0..25 {
                            tokio::time::sleep(Duration::from_millis(400)).await;
                            if check_health(&health_url).await {
                                tracing::info!(id = %def_id, name = %def_name, port = def_port, "Sidecar 后端就绪并正常提供服务");
                                return;
                            }
                        }
                        tracing::warn!(id = %def_id, "Sidecar 启动 10 秒后仍未响应健康检查，请检查依赖或端口占用");
                    });
                }
                Err(err) => {
                    tracing::error!(id = def.id, %err, "拉起 Sidecar 子进程失败");
                }
            }
        }
    });
}

/// 终止所有由 Anima 拉起的 Sidecar 子进程
pub fn stop_all_sidecars() {
    if let Ok(mut guard) = get_running_processes().lock() {
        for (id, mut child) in guard.drain() {
            tracing::info!(id = %id, pid = child.id(), "正在终止 Sidecar 子进程...");
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// 获取当前所有 Sidecar 的运行状态
pub async fn get_sidecar_statuses() -> Vec<SidecarStatusInfo> {
    // 1. 先同步拷贝一份当前记录的 PID 映射，随后立即释放互斥锁（避免跨 await）
    let pids: HashMap<String, u32> = {
        if let Ok(mut guard) = get_running_processes().lock() {
            // **先把死掉的收掉。** 子进程退出后不清理的话，PID 一直挂在表里，
            // 而健康检查又不过 —— 状态就永远停在「启动中」，
            // 界面上还显示一个早就不存在的 PID。
            // 这个失败**不报错**：看起来像"启动很慢"，其实是已经死了
            guard.retain(|id, child| match child.try_wait() {
                Ok(Some(code)) => {
                    tracing::warn!(id = %id, ?code, "Sidecar 子进程已退出，从运行表移除");
                    false
                }
                Ok(None) => true,
                // 查不到状态就当它还活着，宁可多留一会儿也别误报"没启动"
                Err(_) => true,
            });
            guard.iter().map(|(k, v)| (k.clone(), v.id())).collect()
        } else {
            HashMap::new()
        }
    };

    let mut results = Vec::new();
    for def in KNOWN_SIDECARS {
        let is_online = check_health(def.health_url).await;
        let spawned_pid = pids.get(def.id).copied();

        let status = if is_online {
            if spawned_pid.is_some() {
                "online"
            } else {
                "external"
            }
        } else if spawned_pid.is_some() {
            "starting"
        } else {
            "stopped"
        };

        results.push(SidecarStatusInfo {
            id: def.id.to_string(),
            name: def.name.to_string(),
            port: def.default_port,
            status: status.to_string(),
            pid: spawned_pid,
        });
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    /// sidecar id → 它对应的 TTS 引擎 id。没有对应引擎的（数字人）不在表里。
    const ENGINE_OF: &[(&str, &str)] = &[
        ("kokoro-tts", "openai_compatible"),
        ("f5-tts", "f5_tts"),
        ("gpt-sovits", "gpt_sovits"),
    ];

    #[test]
    fn 监控端口必须和引擎配置里的端口一致() {
        // **这条不变量正是 F5-TTS 卡在「启动中」的原因。**
        // 注册表写 9998，而引擎配置里是 8881 —— 健康检查一直敲一个
        // 没人应答的端口，服务其实好好跑着。这个失败不报错、不超时，
        // 面板上就是永远「启动中」，没有任何线索指向端口写错了。
        let engines = crate::tts::TtsEngineConfig::defaults();
        for (sidecar_id, engine_id) in ENGINE_OF {
            let def = KNOWN_SIDECARS
                .iter()
                .find(|d| d.id == *sidecar_id)
                .unwrap_or_else(|| panic!("注册表里没有 {sidecar_id}"));
            let engine = engines
                .iter()
                .find(|e| e.id == *engine_id)
                .unwrap_or_else(|| panic!("默认引擎里没有 {engine_id}"));

            let port = engine
                .base_url
                .rsplit(':')
                .next()
                .and_then(|p| p.trim_end_matches('/').parse::<u16>().ok())
                .unwrap_or_else(|| {
                    panic!("{engine_id} 的 base_url 解析不出端口：{}", engine.base_url)
                });

            assert_eq!(
                def.default_port, port,
                "{sidecar_id} 监控的是 {} 端口，而引擎 {engine_id} 配的是 {port}",
                def.default_port
            );
            assert!(
                def.health_url.contains(&format!(":{port}")),
                "{sidecar_id} 的健康地址 {} 和端口 {port} 对不上",
                def.health_url
            );
        }
    }

    #[test]
    fn 托管的后端必须有入口脚本名() {
        // managed=true 却没有 script_rel 的话，拉起时会静默跳过 ——
        // 面板上显示「未运行」，看着像服务挂了
        for def in KNOWN_SIDECARS {
            if def.managed {
                assert!(
                    !def.script_rel.is_empty(),
                    "{} 声明托管但没有入口脚本",
                    def.id
                );
            } else {
                assert!(
                    def.script_rel.is_empty(),
                    "{} 不托管就不该写入口脚本，避免误解",
                    def.id
                );
            }
        }
    }






    /// 提示要**能照着做**，不是「未找到」四个字。
    #[test]
    fn 找不到时的提示能照着做() {
        let h = app_root_missing_hint();
        assert!(h.contains("anima install"), "没告诉用户怎么办：{h}");
        assert!(h.contains("ANIMA_APP_ROOT"));
        assert!(h.contains("Edge TTS"), "没说会退回到什么：{h}");
    }
}
