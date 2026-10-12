//! 「服务」页：这台机器上 Anima 依赖的东西，现在谁在跑。
//!
//! # 为什么单独做一页
//!
//! 这份信息原来只有 `anima status` 打得出来 —— 也就是说**只有开着终端的人
//! 看得到**。而它恰恰是排查的第一步：出图失败多半是 ComfyUI 没开，
//! 语音没声音多半是 Kokoro 没起，「模型列表空的」多半是 LM Studio 没开。
//!
//! # 这里是唯一的清单
//!
//! 端口这种东西一旦抄在两处就会漂。原来至少有三份：`anima.sh` 的
//! `*_PORT` 变量、[`crate::sidecar::KNOWN_SIDECARS`]、TTS 引擎表里的
//! `base_url`。这里是 Rust 侧的唯一来源，并且有测试：
//!
//! - [`tests::端口和anima_sh一致`] 直接读 `scripts/anima.sh`，对不上就炸
//! - [`tests::sidecar都在清单里`] 读 `KNOWN_SIDECARS`，漏一个就炸
//!
//! # 判据是「端口上有人听」，不是「HTTP 打得通」
//!
//! 各家的健康路径五花八门（GPT-SoVITS 的 `/` 是 404、只有 `/docs` 是 200），
//! 而这一页要回答的是「它起来没有」。**连得上 TCP 就算在线** ——
//! 比逐个记健康路径可靠，也不会因为某家改了路由就误报离线。

use serde::Serialize;
use std::time::Duration;

/// 探测一个端口最多等多久。**本机回环，超过这个数基本就是没人听**，
/// 而这一页有十个服务，串起来等会很难受。
const PROBE_TIMEOUT: Duration = Duration::from_millis(400);

/// 服务分组。顺序就是界面上的顺序 —— 按「坏了影响多大」排。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Group {
    /// 大模型后端
    Llm,
    /// 出图 / 出片
    Vision,
    /// 数字人
    Avatar,
    /// 本地语音合成
    Voice,
}

impl Group {
    pub fn label(self) -> &'static str {
        match self {
            Group::Llm => "大模型后端",
            Group::Vision => "视觉工坊",
            Group::Avatar => "数字人",
            Group::Voice => "本地语音合成",
        }
    }
}

pub struct ServiceDef {
    pub id: &'static str,
    pub name: &'static str,
    pub group: Group,
    pub port: u16,
    /// **它是干什么的** —— 一句话，让人知道要不要在意它
    pub what: &'static str,
    /// **不在线会怎么样。** 这是这一页最有用的一列：
    /// 「离线」本身不说明问题，有些服务本来就不用开
    pub degraded: &'static str,
    /// 起不来时怎么办。空 = 没有一句话能说清的办法
    pub how: &'static str,
    /// 没开是不是正常状态。`true` = 界面不标成警告
    pub optional: bool,
}

/// **唯一的清单。** 加服务只加这里。
pub static SERVICES: &[ServiceDef] = &[
    ServiceDef {
        id: "lmstudio",
        name: "LM Studio",
        group: Group::Llm,
        port: 1234,
        what: "本地大模型，对话和出提示词都可能走它",
        degraded: "选了本地模型的话对话直接失败；模型列表也拉不出来",
        how: "打开 LM Studio 并在里面 Start Server",
        optional: false,
    },
    ServiceDef {
        id: "ollama",
        name: "Ollama",
        group: Group::Llm,
        port: 11434,
        what: "后台小模型 + 向量记忆的嵌入（bge-m3）",
        degraded: "记忆召回和后台抽取停摆，对话本身还能用",
        how: "终端跑 `ollama serve`",
        optional: false,
    },
    ServiceDef {
        id: "comfyui",
        name: "ComfyUI",
        group: Group::Vision,
        port: 8188,
        what: "所有出图、出片、超分都排进它的队列",
        degraded: "出图出片全部不可用（工具会明确报错，不是静默失败）",
        how: "`anima start` 会拉起它；手工起记得带 --fp32-unet",
        optional: false,
    },
    ServiceDef {
        id: "digital-human",
        name: "4D 神经数字人",
        group: Group::Avatar,
        port: 8765,
        what: "神经超写实那种数字人形态的推理服务（Metal/MPS）",
        degraded: "回退到 3D / Live2D / 电子云形态",
        how: "应用启动时会自动拉起；起不来多半是找不到 tools/",
        optional: true,
    },
    ServiceDef {
        id: "kokoro-tts",
        name: "Kokoro",
        group: Group::Voice,
        port: 8880,
        what: "本地语音合成，54 个音色，不需要参考音频",
        degraded: "选了它的话每句话都会报错；可以退回 Edge（云端）",
        how: "应用启动时会自动拉起",
        optional: true,
    },
    ServiceDef {
        id: "f5-tts",
        name: "F5-TTS",
        group: Group::Voice,
        port: 8881,
        what: "零样本音色克隆 —— 给一段参考音频就能模仿",
        degraded: "选了它的话每句话都会报错",
        how: "应用启动时会自动拉起",
        optional: true,
    },
    ServiceDef {
        id: "gpt-sovits",
        name: "GPT-SoVITS",
        group: Group::Voice,
        port: 9880,
        what: "另一个零样本克隆引擎，中文表现更稳",
        degraded: "选了它的话每句话都会报错",
        how: "由 `anima start` 拉起，应用不托管它",
        optional: true,
    },
    ServiceDef {
        id: "cosyvoice2",
        name: "CosyVoice2",
        group: Group::Voice,
        port: 50000,
        what: "零样本克隆引擎（没常用）",
        degraded: "没选它就没影响",
        how: "",
        optional: true,
    },
    ServiceDef {
        id: "indextts2",
        name: "IndexTTS2",
        group: Group::Voice,
        port: 8000,
        what: "零样本克隆引擎（没常用）",
        degraded: "没选它就没影响",
        how: "",
        optional: true,
    },
];

/// 一个服务现在的样子。
#[derive(Debug, Clone, Serialize)]
pub struct ServiceStatus {
    pub id: &'static str,
    pub name: &'static str,
    pub group: &'static str,
    pub group_label: &'static str,
    pub port: u16,
    pub online: bool,
    pub what: &'static str,
    pub degraded: &'static str,
    pub how: &'static str,
    pub optional: bool,
}

/// 端口上有没有人听。**判据是 TCP 连得上**，不是 HTTP 打得通 —— 理由见模块注释。
pub fn port_open(port: u16) -> bool {
    use std::net::{Ipv4Addr, SocketAddr, TcpStream};
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    TcpStream::connect_timeout(&addr, PROBE_TIMEOUT).is_ok()
}

/// 全部探一遍。**并发** —— 十个串起来最坏要 4 秒，界面会空一下。
pub async fn probe_all() -> Vec<ServiceStatus> {
    let handles: Vec<_> = SERVICES
        .iter()
        .map(|d| {
            let port = d.port;
            (d, tokio::task::spawn_blocking(move || port_open(port)))
        })
        .collect();
    let mut out = Vec::with_capacity(handles.len());
    for (d, h) in handles {
        out.push(ServiceStatus {
            id: d.id,
            name: d.name,
            group: match d.group {
                Group::Llm => "llm",
                Group::Vision => "vision",
                Group::Avatar => "avatar",
                Group::Voice => "voice",
            },
            group_label: d.group.label(),
            port: d.port,
            online: h.await.unwrap_or(false),
            what: d.what,
            degraded: d.degraded,
            how: d.how,
            optional: d.optional,
        });
    }
    out
}

#[tauri::command]
pub async fn service_status() -> Vec<ServiceStatus> {
    probe_all().await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **端口抄在两处就会漂。** 直接读 `scripts/anima.sh` 的 `*_PORT` 变量对一遍。
    ///
    /// 漂了的后果很难查：界面说 Kokoro 离线，终端说在线，
    /// 而两边都「没错」—— 它们查的是不同的端口。
    #[test]
    fn 端口和anima_sh一致() {
        let sh = include_str!("../../scripts/anima.sh");
        // 变量名 → 我们的 id
        let pairs = [
            ("LMSTUDIO_PORT", "lmstudio"),
            ("OLLAMA_PORT", "ollama"),
            ("COMFY_PORT", "comfyui"),
            ("DIGITAL_HUMAN_PORT", "digital-human"),
            ("KOKORO_PORT", "kokoro-tts"),
            ("F5_PORT", "f5-tts"),
            ("SOVITS_PORT", "gpt-sovits"),
            ("COSY_PORT", "cosyvoice2"),
            ("INDEX_PORT", "indextts2"),
        ];
        for (var, id) in pairs {
            let needle = format!("\n{var}=");
            let at = sh.find(&needle).unwrap_or_else(|| panic!("anima.sh 里没有 {var}"));
            let rest = &sh[at + needle.len()..];
            let val: u16 = rest
                .split(|c: char| !c.is_ascii_digit())
                .next()
                .unwrap_or("")
                .parse()
                .unwrap_or_else(|_| panic!("{var} 读不出端口"));
            let ours = SERVICES.iter().find(|s| s.id == id).unwrap_or_else(|| panic!("清单里没有 {id}"));
            assert_eq!(ours.port, val, "{id} 的端口和 anima.sh 的 {var} 对不上");
        }
    }

    /// **`KNOWN_SIDECARS` 里的每一个都要在这一页出现。**
    /// 漏一个的表现是「那个服务的状态谁也看不到」。
    #[test]
    fn sidecar都在清单里() {
        for sc in crate::sidecar::KNOWN_SIDECARS {
            let hit = SERVICES.iter().find(|s| s.id == sc.id);
            assert!(hit.is_some(), "sidecar {} 不在服务清单里", sc.id);
            assert_eq!(hit.unwrap().port, sc.default_port, "{} 的端口对不上", sc.id);
        }
    }

    /// 每一条都要说清楚**离线了会怎么样** —— 这一页最有用的就是这一列。
    /// 光说「离线」没有信息量：有些服务本来就不用开。
    #[test]
    fn 每条都说得出离线的后果() {
        for s in SERVICES {
            assert!(!s.what.trim().is_empty(), "{} 没写是干什么的", s.id);
            assert!(s.degraded.chars().count() >= 6, "{} 的「离线会怎样」太敷衍：{}", s.id, s.degraded);
        }
    }

    /// id 不能重复 —— 前端拿它当 key。
    #[test]
    fn id不重复() {
        let mut ids: Vec<&str> = SERVICES.iter().map(|s| s.id).collect();
        ids.sort_unstable();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n, "服务 id 有重复");
    }

    /// 探一个**几乎不可能有人听**的端口，确认判据本身是工作的。
    /// 恒真的探测比没有探测更糟 —— 它会给每个服务都亮绿灯。
    #[test]
    fn 探测不是恒真的() {
        assert!(!port_open(1), "端口 1 都说在线，探测逻辑坏了");
    }
}
