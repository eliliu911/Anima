//! 体检：把这套系统里**不报错的失败**摆到台面上。
//!
//! ## 为什么单独有这一页
//!
//! CHANGELOG 里那几十条修复有一个共同点 —— 它们**全都不报错**。
//! 嘴不动、没声音、动作没反应、记忆好像没生效，界面上什么都不说，
//! 日志里也往往什么都没有。用户唯一能做的是"感觉不太对"。
//!
//! 所以这一页不做"有没有异常"的判断，它做的是**把当前实际生效的东西读出来**。
//!
//! ## 一条规矩：说清楚退回到了什么
//!
//! 缺东西本身不重要，重要的是**缺了之后现在跑的是哪一套** —— 那才是你此刻
//! 听到、看到的东西。比如 Silero 模型不在会退回能量 VAD，症状是"话没说完
//! 就被切断"；只说"缺模型"，用户还是对不上自己遇到的现象。
//!
//! 所以每条 [`Check`] 都带 `detail`：不是修复建议，是**后果**。

use serde::Serialize;
use std::path::Path;

/// 一条体检结果。
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    /// 前端做 key 用，稳定不变
    pub id: &'static str,
    pub label: &'static str,
    /// 一眼能看懂的当前值，例如「bge-m3 · 1024 维」
    pub value: String,
    pub level: Level,
    /// **退回到了什么 / 会怎么失败**，而不是修复步骤
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Ok,
    /// 界面上唯一的告警色（陶色）。**能用但不是你以为的那样**
    Warn,
}

/// 体检的输入。全部由命令层去各处捞，这样判断本身可以脱离运行时测。
#[derive(Debug, Clone, Default)]
pub struct Input {
    /// Silero 模型文件在不在
    pub silero_present: bool,
    pub embedding_model: String,
    pub embedding_dimensions: usize,
    /// 召回相似度下限
    pub recall_threshold: f32,
    /// 记忆层是否已经起来（惰性初始化，没起来时维度读不到）
    pub memory_ready: bool,
    pub animation_count: usize,
    pub tts_engine: String,
    pub tts_reachable: bool,
    pub tts_voices: usize,
    pub tts_error: Option<String>,
    /// Whisper 模型路径，空 = 没配上
    pub stt_model: String,
    /// 跑着的 ComfyUI 的命令行（`/system_stats` 的 `argv`）。
    /// `None` = 没在跑，这时不体检它（没开图像功能是正常状态，不是故障）。
    pub comfy_argv: Option<Vec<String>>,
    /// `tools/` 的所在目录。`None` = 找不到。
    ///
    /// **打包版最容易断的就是这一条**，而且它的默认失败方式是静默的：
    /// 本地语音引擎一个都起不来，界面上只表现为「全部离线」。
    pub app_root: Option<String>,
}

/// 实测过对中文可用的嵌入模型。
///
/// 名单是**实测**出来的，不是按"听起来像多语言"挑的 —— 见 `memory/mod.rs`
/// 里那张召回命中率表。用前缀匹配是因为 Ollama 会带 tag（`bge-m3:latest`）。
const MULTILINGUAL_EMBEDDINGS: &[&str] = &[
    "bge-m3",
    "nomic-embed-text-v2",
    "multilingual-e5",
    "qwen3-embedding",
];

fn is_multilingual(model: &str) -> bool {
    let model = model.to_ascii_lowercase();
    MULTILINGUAL_EMBEDDINGS.iter().any(|m| model.starts_with(m))
}

/// 把当前状态翻译成人话。**纯函数** —— 不碰文件系统也不碰网络。
pub fn checks(input: &Input) -> Vec<Check> {
    let mut out = Vec::new();

    // ── tools/ 找不找得到 ────────────────────────────────────
    //
    // **排在语音引擎前面**：它一断，下面那些引擎全都会显示离线，
    // 而真正的原因在这一条上。顺序反了的话人会去查引擎本身。
    out.push(match &input.app_root {
        Some(root) => Check {
            id: "app_root",
            label: "本地服务目录",
            value: root.clone(),
            level: Level::Ok,
            detail: None,
        },
        None => Check {
            id: "app_root",
            label: "本地服务目录",
            value: "找不到".into(),
            level: Level::Warn,
            detail: Some(crate::sidecar::app_root_missing_hint()),
        },
    });

    // ── 语音引擎 ──────────────────────────────────────────────
    out.push(if input.tts_reachable {
        Check {
            id: "tts",
            label: "语音引擎",
            // 零样本克隆的引擎音色数可能是 0 但完全可用，别把它说成有问题
            value: if input.tts_voices > 0 {
                format!("{} · 可达 · {} 音色", input.tts_engine, input.tts_voices)
            } else {
                format!("{} · 可达", input.tts_engine)
            },
            level: Level::Ok,
            detail: None,
        }
    } else {
        Check {
            id: "tts",
            label: "语音引擎",
            value: format!("{} · 连不上", input.tts_engine),
            level: Level::Warn,
            detail: Some(match &input.tts_error {
                Some(err) => format!("她不会出声，只有文字。{err}"),
                None => "她不会出声，只有文字。".into(),
            }),
        }
    });

    // ── 断句 VAD ──────────────────────────────────────────────
    out.push(if input.silero_present {
        Check {
            id: "vad",
            label: "断句 VAD",
            value: "Silero v5".into(),
            level: Level::Ok,
            detail: None,
        }
    } else {
        Check {
            id: "vad",
            label: "断句 VAD",
            value: "缺模型 · 已退回能量 VAD".into(),
            level: Level::Warn,
            // 症状写在这儿，因为用户是**先遇到症状**才来看这一页的
            detail: Some(
                "能量 VAD 会把尾音和轻辅音判成静音 —— 症状是「话没说完就被切断」。".into(),
            ),
        }
    });

    // ── 识别模型 ──────────────────────────────────────────────
    out.push(if input.stt_model.is_empty() {
        Check {
            id: "stt",
            label: "识别模型",
            value: "未配置".into(),
            level: Level::Warn,
            detail: Some("说话不会变成文字，整条语音链路到这里就断了。".into()),
        }
    } else {
        let name = Path::new(&input.stt_model)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| input.stt_model.clone());
        Check {
            id: "stt",
            label: "识别模型",
            value: name,
            level: Level::Ok,
            detail: None,
        }
    });

    // ── 嵌入后端 ──────────────────────────────────────────────
    out.push(embedding_check(input));

    // ── 动作库 ────────────────────────────────────────────────
    out.push(if input.animation_count > 0 {
        Check {
            id: "animations",
            label: "动作库",
            value: format!("{} 个 .vrma", input.animation_count),
            level: Level::Ok,
            detail: None,
        }
    } else {
        Check {
            id: "animations",
            label: "动作库",
            value: "空 · 已退回内置手势".into(),
            level: Level::Warn,
            detail: Some("内置手势是代码生成的，只有挥手、比心这几个，动作会比较单薄。".into()),
        }
    });

    // ── 图像后端精度 ──────────────────────────────────────────
    if let Some(argv) = &input.comfy_argv {
        out.push(comfy_precision_check(argv));
    }

    out
}

/// ComfyUI 的扩散精度。**这是本页里最安静的一条坏法**。
///
/// ComfyUI 在 MPS 上默认给扩散模型选 bf16，而 FLUX 在 MPS + bf16 下前向出 NaN，
/// 采样照跑完、日志零报错，出来是一张**纯黑图**。fp16 也不对：FLUX 激活值超过
/// fp16 上限，出来是彩色噪声 —— 像素范围 0~255，看统计值还以为成功了。
///
/// 实测代价为零（SDXL 1024/20 步在 fp16 与 fp32 下同为 30.1s），所以这里不做
/// 「按模型选精度」的判断，直接要求全局 fp32。
fn comfy_precision_check(argv: &[String]) -> Check {
    let has = |flag: &str| argv.iter().any(|a| a == flag);

    if has("--fp32-unet") {
        return Check {
            id: "comfy_precision",
            label: "图像后端精度",
            value: "ComfyUI · fp32 UNet".into(),
            level: Level::Ok,
            detail: None,
        };
    }

    let now = if has("--fp16-unet") {
        "fp16"
    } else if has("--bf16-unet") {
        "bf16"
    } else {
        "默认 bf16"
    };

    Check {
        id: "comfy_precision",
        label: "图像后端精度",
        value: format!("ComfyUI · {now} UNet"),
        level: Level::Warn,
        detail: Some(
            "FLUX 在 MPS 上只有 fp32 是对的：bf16 出纯黑图，fp16 出彩色噪声，             两种都跑完全部采样步、不报任何错。SDXL 不受影响。             重启 ComfyUI 时加 --fp32-unet（实测不影响速度）。"
                .into(),
        ),
    }
}

/// 嵌入后端单独抽出来，因为它有两种**都不报错**的坏法。
fn embedding_check(input: &Input) -> Check {
    if !input.memory_ready {
        return Check {
            id: "embedding",
            label: "嵌入后端",
            value: "记忆层未启动".into(),
            level: Level::Warn,
            detail: Some("她记不住跨会话的事 —— 每次对话都从零开始。".into()),
        };
    }

    let value = format!(
        "{} · {} 维",
        input.embedding_model, input.embedding_dimensions
    );

    // 坏法一：英文向模型。它照样返回向量、照样算得出相似度，只是排序毫无意义
    if !is_multilingual(&input.embedding_model) {
        return Check {
            id: "embedding",
            label: "嵌入后端",
            value,
            level: Level::Warn,
            detail: Some(
                "这不是多语言模型，对中文给不出有意义的排序 —— 症状是不管问什么，\
                 召回的永远是最短的那条事实。"
                    .into(),
            ),
        };
    }

    // 坏法二：换了模型没重测阈值。分数整体偏低的模型配 0.65 会把**所有**记忆滤掉
    if input
        .embedding_model
        .to_ascii_lowercase()
        .starts_with("bge-m3")
    {
        return Check {
            id: "embedding",
            label: "嵌入后端",
            value,
            level: Level::Ok,
            detail: None,
        };
    }
    if input.recall_threshold > 0.5 {
        return Check {
            id: "embedding",
            label: "嵌入后端",
            value,
            level: Level::Warn,
            detail: Some(format!(
                "召回阈值 {:.2} 是按 bge-m3 测出来的。换了模型不重测，\
                 分数整体偏低时会把所有记忆都滤掉，表现为「记忆好像没生效」。",
                input.recall_threshold
            )),
        };
    }
    Check {
        id: "embedding",
        label: "嵌入后端",
        value,
        level: Level::Ok,
        detail: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn healthy() -> Input {
        Input {
            silero_present: true,
            embedding_model: "bge-m3".into(),
            embedding_dimensions: 1024,
            recall_threshold: 0.65,
            memory_ready: true,
            animation_count: 8,
            tts_engine: "Edge".into(),
            tts_reachable: true,
            tts_voices: 41,
            tts_error: None,
            stt_model: "/models/ggml-large-v3.bin".into(),
            comfy_argv: None,
            app_root: Some("/repo".into()),
        }
    }

    fn find<'a>(checks: &'a [Check], id: &str) -> &'a Check {
        checks
            .iter()
            .find(|c| c.id == id)
            .expect("这一项应该总是出现")
    }

    #[test]
    fn 全部正常时没有告警() {
        let checks = checks(&healthy());
        assert!(checks.iter().all(|c| c.level == Level::Ok), "{checks:#?}");
        // 每一项都要出现，缺项等于把问题藏起来
        for id in ["tts", "vad", "stt", "embedding", "animations"] {
            find(&checks, id);
        }
    }

    #[test]
    fn 缺silero要说清楚退回到了什么() {
        let input = Input {
            silero_present: false,
            ..healthy()
        };
        let vad = find(&checks(&input), "vad").clone();
        assert_eq!(vad.level, Level::Warn);
        assert!(
            vad.value.contains("能量 VAD"),
            "只说缺模型不够：{}",
            vad.value
        );
        // 症状必须写出来 —— 用户是带着症状来看这一页的
        assert!(vad.detail.unwrap().contains("话没说完"));
    }

    #[test]
    fn 英文向嵌入模型要告警() {
        for model in [
            "mxbai-embed-large",
            "nomic-embed-text",
            "nomic-embed-text:latest",
        ] {
            let input = Input {
                embedding_model: model.into(),
                ..healthy()
            };
            let check = find(&checks(&input), "embedding").clone();
            assert_eq!(check.level, Level::Warn, "{model} 是英文向模型，应该告警");
            assert!(check.detail.unwrap().contains("最短"));
        }
    }

    #[test]
    fn 多语言模型带tag也认() {
        for model in ["bge-m3:latest", "nomic-embed-text-v2-moe", "BGE-M3"] {
            let input = Input {
                embedding_model: model.into(),
                // 非 bge-m3 的多语言模型走阈值那条分支，压低避免误报
                recall_threshold: 0.3,
                ..healthy()
            };
            let check = find(&checks(&input), "embedding").clone();
            assert_eq!(
                check.level,
                Level::Ok,
                "{model} 应该被认成多语言：{check:?}"
            );
        }
    }

    #[test]
    fn 换了模型不重测阈值要告警() {
        let input = Input {
            embedding_model: "nomic-embed-text-v2-moe".into(),
            recall_threshold: 0.65,
            ..healthy()
        };
        let check = find(&checks(&input), "embedding").clone();
        assert_eq!(check.level, Level::Warn);
        assert!(check.detail.unwrap().contains("记忆好像没生效"));
    }

    #[test]
    fn 零样本引擎没有音色数也算正常() {
        let input = Input {
            tts_engine: "GPT-SoVITS".into(),
            tts_voices: 0,
            ..healthy()
        };
        let tts = find(&checks(&input), "tts").clone();
        assert_eq!(tts.level, Level::Ok, "零样本克隆本来就没有音色列表");
        assert!(
            !tts.value.contains("0 音色"),
            "别把 0 显出来：{}",
            tts.value
        );
    }

    #[test]
    fn 引擎连不上时原样带出原因() {
        let input = Input {
            tts_reachable: false,
            tts_error: Some("connection refused".into()),
            ..healthy()
        };
        let tts = find(&checks(&input), "tts").clone();
        assert_eq!(tts.level, Level::Warn);
        assert!(tts.detail.unwrap().contains("connection refused"));
    }

    #[test]
    fn 记忆层没起来时不冒充维度() {
        let input = Input {
            memory_ready: false,
            embedding_dimensions: 0,
            ..healthy()
        };
        let check = find(&checks(&input), "embedding").clone();
        assert_eq!(check.level, Level::Warn);
        assert!(
            !check.value.contains('0'),
            "别显示 0 维，那看着像配错了：{}",
            check.value
        );
    }

    fn argv(extra: &[&str]) -> Option<Vec<String>> {
        let mut v: Vec<String> = ["main.py", "--listen", "127.0.0.1", "--port", "8188"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        v.extend(extra.iter().map(|s| s.to_string()));
        Some(v)
    }

    #[test]
    fn comfyui_没在跑时不体检精度() {
        let out = checks(&Input {
            comfy_argv: None,
            app_root: Some("/repo".into()),
            ..healthy()
        });
        assert!(
            out.iter().all(|c| c.id != "comfy_precision"),
            "没开图像功能不是故障"
        );
    }

    #[test]
    fn 带了_fp32_unet_才算正常() {
        let ok = find(
            &checks(&Input {
                comfy_argv: argv(&["--fp32-unet", "--fp32-vae"]),
                ..healthy()
            }),
            "comfy_precision",
        )
        .clone();
        assert_eq!(ok.level, Level::Ok);
        assert!(ok.value.contains("fp32"));
    }

    /// 这是最容易漏的一种：**什么开关都没加**。
    /// ComfyUI 会自己在 MPS 上选 bf16，FLUX 于是静默出黑图。
    #[test]
    fn 不加任何精度开关要告警() {
        let c = find(
            &checks(&Input {
                comfy_argv: argv(&["--fp32-vae"]),
                ..healthy()
            }),
            "comfy_precision",
        )
        .clone();
        assert_eq!(
            c.level,
            Level::Warn,
            "默认就是坏的，不能因为「没显式选 bf16」就放过"
        );
        assert!(c.value.contains("bf16"));
    }

    /// fp16 是个陷阱：它不出黑图，出的是**像素范围看着正常**的彩色噪声。
    #[test]
    fn fp16_也要告警并说清它的症状() {
        let c = find(
            &checks(&Input {
                comfy_argv: argv(&["--fp16-unet"]),
                ..healthy()
            }),
            "comfy_precision",
        )
        .clone();
        assert_eq!(c.level, Level::Warn);
        assert!(c.value.contains("fp16"));
        let d = c.detail.unwrap();
        assert!(d.contains("噪声"), "得说明 fp16 的症状和 bf16 不一样：{d}");
        assert!(d.contains("--fp32-unet"), "得给出具体开关：{d}");
    }
}
