import { useTranslation } from "react-i18next";
/**
 * TTS 引擎切换。
 *
 * 三种引擎的"选音色"是**两种不同的模型**，界面必须区分开，
 * 否则用户会在 GPT-SoVITS 下找音色下拉框而永远找不到：
 *   - Edge / OpenAI 兼容：从目录里挑一个音色名
 *   - GPT-SoVITS：零样本克隆，音色由**参考音频**决定，没有音色名这回事
 */
import { useEffect, useState } from "react";
import { PersonaLockBanner, usePersona } from "./PersonaLock";
import {
  playMp3Bytes,
  previewTtsVoice,
  listTtsVoices,
  probeTtsEngine,
  ttsEngineApi,
  type EngineProbe,
  type TtsEngineConfig,
  type TtsEngineView,
  type TtsVoiceInfo,
} from "../api/voice";
import {
  setImageEngine,
  setSdxlCheckpoint,
  listSdxlCheckpoints,
  setSheetLoras,
  setZimageLoras,
  setH3NativeRef2va,
} from "../api";
import { useChat } from "../store/chat";
import { loraNote, sortLorasForSheet, checkpointNote, loraSectionsFor } from "./loraNotes";
import { isMissing, rowOptions } from "./studio/zimageLoraRow";
import { studioLoras, type LoraOption } from "../api/studio";

// 每条说明都带上**这台机器上的实测数字**。
//
// 理由很实际：网上的评测表都不指明硬件，而在 Apple Silicon 上决定能不能用的是
// 架构而不是评分 —— 自回归模型在 MPS 上会慢到不可用。IndexTTS2 在各种榜单里
// 速度都是满星，实测每句 13.7 秒。
const KIND_NOTE: Record<string, string> = {
  edge:
    "微软 Edge 朗读。云端、免费、中文自然，且是唯一会返回真实词边界时间戳的引擎" +
    "（对口型用得上）。代价是音色固定：中文一共 14 个，没有二次元/御姐类音色。",
  openai_compatible:
    "OpenAI 兼容的 /v1/audio/speech —— 我们这里跑的是 Kokoro。**最快的一个**：" +
    "实测 0.11 秒出 3.8 秒音频（35× 实时），因为它是非自回归架构。" +
    "代价是音色固定 54 个，且没有情绪控制。",
  f5_tts:
    "F5-TTS 零样本克隆。**非自回归**（flow matching + DiT），这是它在 Mac 上" +
    "能用的根本原因 —— 实测 2.5 秒出 6.8 秒音频（2.8× 实时）。" +
    "给 5~10 秒清晰人声就能克隆任意音色，二次元/御姐都行。" +
    "音色放在 tools/f5-tts/voices/：<名字>.wav 配一个同名 <名字>.txt 写那段话的文字。" +
    "注意权重是 CC-BY-NC，禁止商用。",
  gpt_sovits:
    "GPT-SoVITS 零样本克隆。二次元音色包基本都在这个生态里 —— " +
    "音色由参考音频决定，给 5~20 秒清晰人声就能克隆。" +
    "实测 2.4 秒出 4.8 秒音频（2.0× 实时），32kHz —— 采样率是这几个里最高的。" +
    "参考音频路径由**服务端**读取，所以要填绝对路径。" +
    "它的 GPT 段是自回归的，所以每次输出长度会略有不同。",
  cosy_voice2:
    "CosyVoice2 零样本克隆。参考音频由 Anima **上传**给服务端，所以填本机路径。" +
    "返回裸 PCM，采样率要配对。**没在这台机器上验证过**，而且它的论文明确写了" +
    "自回归架构，Mac 上的速度存疑。",
  index_tts2:
    "IndexTTS2，情感控制最强（8 个维度各 0~1）。**但在这台机器上不可用**：" +
    "实测每句 13.7 秒（RTF 4.0），fp16 更差（77 秒）。它的加速手段 deepspeed 与" +
    "flash-attn 全是 CUDA 专用，而 MPS 上的自回归解码本身就有延迟病态。" +
    "留着这一项是为了将来有 N 卡时能用。",
};

/** 音色靠参考音频克隆的引擎 —— 它们没有音色列表 */
const CLONING: string[] = ["gpt_sovits", "cosy_voice2", "index_tts2"];

/** 返回裸 PCM、需要配采样率的引擎 */
const RAW_PCM: string[] = ["cosy_voice2", "index_tts2"];

export function TtsEngineSettings() {
  const { t, i18n } = useTranslation(["settings", "common"]);
  const isZh = (i18n.language || "").startsWith("zh");
  // 语音声学**只在默认形态下可编辑** —— 选了角色就归角色管
  const { view: persona } = usePersona();
  const locked = persona ? !persona.resolved.is_default_form : false;
  const [view, setView] = useState<TtsEngineView | null>(null);
  const [busy, setBusy] = useState(false);
  // 每个引擎能不能用。undefined = 还在探
  const [probes, setProbes] = useState<Record<string, EngineProbe>>({});
  const [message, setMessage] = useState<string | null>(null);

  // 出图底模。放在应用设定里 —— 它和 TTS 引擎是同一类东西：
  // 「这台机器上用哪个后端服务」，不是界面偏好。
  // 读不到配置时的兜底要和 Rust 的 `DEFAULT_ENGINE` 一致（2026-09-26 起 qwen21）
  const imageEngine = useChat((st) => st.config?.image_engine) || "qwen21";
  const refreshConfig = useChat((st) => st.refreshConfig);
  const [engineBusy, setEngineBusy] = useState(false);

  // SDXL 那条路的 checkpoint 单独可选。
  //
  // **这条路绕不开**：挂 LoRA、控姿势、出角色设定卡都强制走 SDXL，而内置
  // 优先级把欧美写实的 RealVisXL 排第一 —— 做国风设定卡时 AWPortrait_XL
  // 明显更合适（交领、纱质层次、玉簪更清楚，多视图之间的脸更一致）。
  // 谁更合适取决于用户在做什么，不该由代码替他定。
  const sdxlCkpt = useChat((st) => st.config?.sdxl_checkpoint) || "";
  const [ckpts, setCkpts] = useState<string[]>([]);
  useEffect(() => {
    // ComfyUI 没起来时返回空数组，这一项自己不渲染 —— 不报错、不占地方
    void listSdxlCheckpoints()
      .then(setCkpts)
      .catch(() => setCkpts([]));
  }, []);
  // 设定卡专用 LoRA。
  //
  // **只在设定卡那条路生效，刻意不做全局常挂。** LoRA 只对 SDXL 架构有效，
  // 而默认引擎是 Z-Image —— 全局设置会在那条路上一声不吭地什么都不做，
  // 用户以为风格生效了、实际没有。设定卡那条路本来就强制回落 SDXL。
  const sheetLoras = useChat((st) => st.config?.sheet_loras) || [];
  // **按架构过滤必须用 Rust 算出来的 `arch`，不能按文件名猜。**
  //
  // 2026-09-17 用户在 SDXL 的设定卡下拉里看到了 `krea/DynamicCharacterSheet_krea2_v1`。
  // 根因：这里原来的过滤条件是 `!isZImageLora(n)`（「不是 Z-Image 的」）——
  // 那是只有两种架构的年代写的，现在有五种，于是 **Krea 2 / LTX / H3 / Qwen
  // 的 LoRA 全从这个口子漏进了 SDXL 的下拉**。挂上去一声不吭什么都不做。
  //
  // `studioLoras()` 的 `arch` 是 Rust 侧**读 safetensors 张量键名**算的
  // （`image_engine::arch_from_keys`），和出图时的判定同一个口径。
  const [loraOpts, setLoraOpts] = useState<LoraOption[]>([]);
  useEffect(() => {
    void studioLoras()
      .then(setLoraOpts)
      .catch(() => setLoraOpts([]));
  }, []);
  const loraList = loraOpts.map((o) => o.name);
  const byArch = (a: string) => loraOpts.filter((o) => o.arch === a).map((o) => o.name);
  // Z-Image 那条路常挂的 LoRA。**和上面那批不是同一批权重** ——
  // Z-Image 是 DiT、SDXL 是 UNet，互挂都是静默失效，所以两个下拉各自只列自己架构的。
  // 三块全列出来太吵。跟当前底模有关的展开，另一块收进折叠 ——
  // **不能直接藏掉 SDXL 那两块**：设定卡/控姿势永远回落 SDXL，和选了什么底模无关。
  const sections = loraSectionsFor(imageEngine);
    const zimageLoras = useChat((st) => st.config?.zimage_loras) || [];
  const saveZimageLoras = async (next: { name: string; strength: number }[]) => {
    if (engineBusy) return;
    setEngineBusy(true);
    try {
      await setZimageLoras(next);
      await refreshConfig();
    } finally {
      setEngineBusy(false);
    }
  };
  // SDXL 那两块（底模 + 设定卡 LoRA）。**和当前选的底模无关** ——
  // 设定卡、控姿势、SDXL 架构的 LoRA 都强制回落 SDXL，所以任何底模下都得可达。
  // 选中 SDXL 时展开，否则收进折叠：既不占地方，也不会配不到。
  const sdxlBlocks = (
    <>
        {ckpts.length > 0 && (
          <label className="flex flex-col gap-1.5">
            <span className="text-[11.5px] font-bold text-[var(--ink-2)]">
              {t("image.sdxlCkpt", { defaultValue: "SDXL 底模（挂 LoRA / 控姿势 / 出设定卡时用）" })}
            </span>
            <select
              className="bg-[var(--surface)] hover:bg-[var(--surface-2)] border border-[var(--line)] focus:border-[var(--celadon)] focus:ring-2 focus:ring-[var(--celadon)]/15 rounded-xl px-3 py-1.5 outline-none text-[12.5px] text-[var(--ink)] w-full transition-all cursor-pointer shadow-2xs"
              value={sdxlCkpt}
              onChange={(e) => void pickCkpt(e.target.value)}
            >
              <option value="">
                {t("image.sdxlAuto", { defaultValue: "自动挑选（当前会选 RealVisXL）" })}
              </option>
              {ckpts.map((c) => {
                const note = checkpointNote(c, isZh);
                return (
                  <option key={c} value={c}>
                    {note ? `${c} — ${note}` : c}
                  </option>
                );
              })}
            </select>
            <span className="text-[11px] text-[var(--ink-3)] leading-relaxed">
              {t("image.sdxlHint", {
                defaultValue:
                  "做国风角色设定卡时 AWPortrait_XL 比默认的 RealVisXL 更合适 —— 交领、纱质层次、玉簪都更清楚，多视图之间的脸也更一致。RealVisXL 的皮肤质感更真实。",
              })}
            </span>
          </label>
        )}

        {loraList.length > 0 && (
          <div className="flex flex-col gap-1.5">
            <span className="text-[11.5px] font-bold text-[var(--ink-2)]">
              {t("image.sheetLora", { defaultValue: "设定卡专用 LoRA（只在出角色设定卡时生效）" })}
            </span>
            {sheetLoras.map((l, i) => (
              <div key={`${l.name}-${i}`} className="flex items-center gap-2">
                <select
                  className="min-w-0 flex-1 truncate bg-[var(--surface)] hover:bg-[var(--surface-2)] border border-[var(--line)] rounded-xl px-3 py-1.5 outline-none text-[12px] text-[var(--ink)] cursor-pointer"
                  value={l.name}
                  title={loraNote(l.name, isZh)?.text ?? l.name}
                  onChange={(e) => {
                    const next = [...sheetLoras];
                    next[i] = { ...next[i], name: e.target.value };
                    void saveLoras(next);
                  }}
                >
                  {rowOptions(sortLorasForSheet(byArch("sdxl"), isZh), l.name).map((o) => (
                    <option key={o.value} value={o.value}>
                      {o.label}
                    </option>
                  ))}
                </select>
                <input
                  type="number"
                  step="0.05"
                  min="0"
                  max="2"
                  value={l.strength}
                  onChange={(e) => {
                    const next = [...sheetLoras];
                    next[i] = { ...next[i], strength: Number(e.target.value) };
                    void saveLoras(next);
                  }}
                  className="w-20 bg-[var(--surface)] border border-[var(--line)] rounded-xl px-2 py-1.5 text-[12px] text-[var(--ink)] outline-none"
                />
                <button
                  onClick={() => void saveLoras(sheetLoras.filter((_, j) => j !== i))}
                  className="px-2 py-1.5 text-[12px] text-[var(--ink-3)] hover:text-[var(--clay)] transition-colors"
                >
                  {t("common.remove", { defaultValue: "移除" })}
                </button>
              </div>
            ))}
            {sheetLoras.map((l, i) => {
              const note = loraNote(l.name, isZh);
              if (!note) return null;
              return (
                <span
                  key={`note-${l.name}-${i}`}
                  className={`text-[11px] leading-relaxed ${
                    note.incompatible ? "text-[var(--clay)]" : "text-[var(--ink-3)]"
                  }`}
                >
                  {l.name.replace(/\.safetensors$/, "")} · {note.text}
                </span>
              );
            })}
            <button
              onClick={() => void saveLoras([...sheetLoras, { name: byArch("sdxl")[0], strength: 0.6 }])}
              className="w-fit px-3 py-1.5 text-[12px] rounded-xl border border-[var(--line)] hover:border-[var(--celadon)] text-[var(--ink-2)] transition-colors"
            >
              + 添加 LoRA
            </button>
            <span className="text-[11px] text-[var(--ink-3)] leading-relaxed">
              {t("image.sheetLoraHint", {
                defaultValue:
                  "版式 LoRA（CharTurnXL @0.55）会自动挂在最前，这里配的叠在它后面。权重通常要低一些，否则会把版式压掉。出图后返回里会写明实际挂了哪几个。",
              })}
            </span>
          </div>
        )}
    </>
  );


  const saveLoras = async (next: { name: string; strength: number }[]) => {
    if (engineBusy) return;
    setEngineBusy(true);
    try {
      await setSheetLoras(next);
      await refreshConfig();
    } finally {
      setEngineBusy(false);
    }
  };

  const pickCkpt = async (name: string) => {
    if (engineBusy) return;
    setEngineBusy(true);
    try {
      await setSdxlCheckpoint(name);
      await refreshConfig();
    } finally {
      setEngineBusy(false);
    }
  };
  // H3 参考生视频的权重。2026-10-06 之前是「装了原生 Ref2VA 就自动用」，EP03 第 2 镜对照里原生那几条
  // 都把第二女角色换成了星澜 —— 用户定：做成开关、默认兼容模式
  const h3Native = useChat((st) => st.config?.h3_native_ref2va) ?? false;
  const h3NativeInstalled = useChat((st) => st.config?.h3_ref2va_installed) ?? false;
  const pickH3Native = async (on: boolean) => {
    if (on === h3Native || engineBusy) return;
    setEngineBusy(true);
    try {
      await setH3NativeRef2va(on);
      await refreshConfig();
    } finally {
      setEngineBusy(false);
    }
  };
  const pickImageEngine = async (e: "zimage" | "sdxl" | "krea2" | "qwen21") => {
    if (e === imageEngine || engineBusy) return;
    setEngineBusy(true);
    try {
      await setImageEngine(e);
      await refreshConfig();
    } finally {
      setEngineBusy(false);
    }
  };

  // 音色列表在这里自己管，不用全局 store 里那份。
  //
  // 那份只在 store 初始化时拉一次 —— 而本地引擎（Docker 里的 Kokoro）
  // 那会儿还在加载模型，请求失败就永远是空列表，也不会重试。
  // 用户看到的就是一个点不开的空下拉框。
  const [voices, setVoices] = useState<TtsVoiceInfo[]>([]);
  const [voiceMsg, setVoiceMsg] = useState<string | null>(null);
  const [previewing, setPreviewing] = useState(false);
  /**
   * 当前引擎自己记着的音色。
   *
   * **不能用全局的 `voiceConfig.tts.voice`** —— 音色名每家一套
   * （Edge 是 `zh-CN-XiaoxiaoNeural`，Kokoro 是 `zf_xiaobei`，
   * SoVITS 是音色库路径）。共用一份的结果是换引擎之后把上一家的名字
   * 丢给新引擎：Kokoro 会去 HuggingFace 找 `.pt` 然后 404，
   * SoVITS 会说"需要参考音频"，而用户看到的都是"没声音"。
   */
  const engineRow = view?.engines.find((e) => e.id === view?.selected);
  const currentVoice = engineRow?.voice ?? "";

  useEffect(() => {
    ttsEngineApi
      .list()
      .then(setView)
      .catch((err) => setMessage(`读取引擎列表失败：${err}`));
  }, []);

  // 引擎变了就重新拉音色 —— 每个引擎的音色是完全不同的一套
  const selectedEngine = view?.selected;
  useEffect(() => {
    if (!selectedEngine) return;
    let cancelled = false;
    setVoiceMsg("读取中…");
    listTtsVoices()
      .then((v) => {
        if (cancelled) return;
        setVoices(v);
        setVoiceMsg(
          v.length === 0
            ? "这个引擎没有音色列表（零样本克隆型引擎的音色由参考音频决定）"
            : null,
        );
      })
      .catch((err) => {
        if (!cancelled) {
          setVoices([]);
          // 失败原因要显示出来 —— 空列表最常见的原因是服务还没起来
          setVoiceMsg(`${err}`);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [selectedEngine]);

  // 逐个探活。
  //
  // **串行而不是并发**：探活会去连本地服务，而那些服务多半是同一台机器上的
  // torch 进程 —— 同时打它们会明显变慢。这个面板不是热路径，串行几秒无所谓。
  //
  // 只在引擎集合变化时重探，不跟着选中项变 —— 否则每次切引擎都要重探六次。
  useEffect(() => {
    if (!view) return;
    const ids = view.engines.map((e) => e.id);
    let cancelled = false;
    (async () => {
      for (const id of ids) {
        if (cancelled) return;
        try {
          const p = await probeTtsEngine(id);
          if (!cancelled) setProbes((prev) => ({ ...prev, [id]: p }));
        } catch (err) {
          if (!cancelled) {
            setProbes((prev) => ({
              ...prev,
              [id]: { id, reachable: false, voices: 0, error: String(err) },
            }));
          }
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [view?.engines.map((e) => e.id).join(",")]);

  if (!view) {
    return <div className="text-[13px] text-[var(--ink-3)] p-4">加载中...</div>;
  }

  const current = view.engines.find((e) => e.id === view.selected);

  const select = async (id: string) => {
    if (locked) return;
    setBusy(true);
    try {
      await ttsEngineApi.select(id);
      const next = await ttsEngineApi.list();
      setView(next);
      setMessage(null);
    } catch (err) {
      setMessage(`切换引擎失败：${err}`);
    } finally {
      setBusy(false);
    }
  };

  const saveCurrent = async (patch: Partial<TtsEngineConfig>) => {
    if (!current || locked) return;
    try {
      await ttsEngineApi.update({ ...current, ...patch });
      setMessage(null);
    } catch (err) {
      setMessage(`保存配置失败：${err}`);
    }
  };

  return (
    <div className="flex flex-col gap-4 max-w-4xl pb-8">
      {/* 出图底模。三个引擎实测耗时差 8.5 倍，是最影响体感的一项设置 */}
      <div className="flex flex-col">
        <span className="text-[14px] font-bold text-[var(--ink)] tracking-tight">
          {t("image.engineTitle", { defaultValue: "出图底模" })}
        </span>
        <span className="text-[12px] text-[var(--ink-2)] mt-0.5 leading-relaxed">
          {t("image.engineDesc", {
            defaultValue:
              "耗时为本机实测。需要 LoRA、姿势控制或角色设定卡时会自动改用 SDXL —— 那些权重只有 SDXL 架构的。",
          })}
        </span>
      </div>

      <div className="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-4 gap-3">
        {(
          [
            {
              id: "zimage" as const,
              name: "Z-Image Turbo",
              cost: "约 45 秒",
              badge: "最快",
              desc: "要快、要反复试稿时用。质量远高于 SDXL，代价只多 15%。改图（图生图）默认走它。它的负向提示词不起作用（官方配置把负向通路清零了），但照样能出对手。",
            },
            {
              id: "sdxl" as const,
              name: "SDXL",
              cost: "约 39 秒",
              badge: null,
              desc: "唯一能挂 LoRA、ControlNet 和角色设定卡版式的引擎。长提示词会被摊薄，细节比前两个丢得多。",
            },
            {
              id: "krea2" as const,
              name: "Krea 2 Turbo",
              cost: "约 1 分 50 秒",
              badge: null,
              desc: "提示词服从度和写实质感最强：同一段词它「三个青苹果」数量都对，而 Z-Image 出成了四个、还把「雨夜」出成白天。Qwen3-VL 编码器中文原生，图上的中文标注也画得出来。慢 3 倍，适合定稿而不是反复迭代。负向提示词不起作用（cfg=1）。",
            },
            {
              id: "qwen21" as const,
              name: "Qwen-Image 2.1",
              cost: "约 3 分 31 秒",
              badge: "默认推荐",
              desc: "画质、文字和一致性最好的一档：同一段设定卡提示词，只有它把中文名字写对、资料栏英文全对、十几个格子是同一张脸。Qwen3-VL-8B 编码器，中文原生、长提示词不摊薄；用的是去审查版（UC Q8_0），没有内容过滤。代价是慢（Z-Image 的 4~5 倍）。**只做文生图**：它的 VAE 是四通道 RGBA，改图那条路在它身上出坏图，所以改图会自动换成 Z-Image。负向提示词不起作用（cfg=1）。",
            },
          ]
        ).map((e) => (
          <div
            key={e.id}
            onClick={() => pickImageEngine(e.id)}
            className={`p-4 rounded-2xl border transition-all cursor-pointer flex flex-col gap-2 select-none shadow-2xs ${
              imageEngine === e.id
                ? "bg-[var(--surface-2)] border-[var(--celadon)] ring-2 ring-[var(--celadon)]/20"
                : "bg-[var(--paper)] border-[var(--line)] hover:border-[var(--line-2)] hover:bg-[var(--surface-2)]"
            }`}
          >
            <div className="flex items-center justify-between gap-2">
              <span className="text-[13px] font-bold text-[var(--ink)]">{e.name}</span>
              {e.badge && (
                <span className="px-2 py-0.5 text-[10px] font-semibold tracking-wide uppercase rounded-full bg-[var(--celadon)]/10 text-[var(--celadon)] border border-[var(--celadon)]/20">
                  {e.badge}
                </span>
              )}
            </div>
            <span className="w-fit rounded-full border border-[var(--line)] bg-[var(--surface-3)]/70 px-2 py-[1px] text-[10.5px] font-medium text-[var(--ink-2)]">
              {e.cost} / 张
            </span>
            <p className="text-[11.5px] text-[var(--ink-3)] leading-relaxed">{e.desc}</p>
          </div>
        ))}
      </div>


      {sections.sdxlPrimary ? (
        sdxlBlocks
      ) : (
        <details className="rounded-xl border border-[var(--line)] px-3 py-2">
          <summary className="text-[11.5px] font-bold text-[var(--ink-2)] cursor-pointer select-none">
            {t("image.sdxlFallbackSection", {
              defaultValue: "SDXL 回落设置（设定卡 / 控姿势 / SDXL LoRA 时用）",
            })}
          </summary>
          <div className="mt-2 flex flex-col gap-3">{sdxlBlocks}</div>
        </details>
      )}

      {sections.zimage && byArch("zimage").length > 0 && (
        <div className="flex flex-col gap-1.5">
          <span className="text-[11.5px] font-bold text-[var(--ink-2)]">
            {t("image.zimageLora", {
              defaultValue: "Z-Image 常挂 LoRA（默认底模，纯文生图时生效）",
            })}
          </span>
          {zimageLoras.map((l, i) => {
            const avail = byArch("zimage");
            const gone = isMissing(avail, l.name);
            return (
            <div key={`z-${l.name}-${i}`} className="flex items-center gap-2">
              {/* **`min-w-0` 不能少。** flex 子项默认 min-width:auto，选项一长
                  select 就按内容撑宽，把右边的强度框和「移除」按钮挤出可视区 ——
                  2026-09-17 用户报的「删不了」就是这么来的，按钮一直在、只是看不见。
                  说明也不再拼进选项，改挂 title。 */}
              <select
                className="min-w-0 flex-1 truncate bg-[var(--surface)] hover:bg-[var(--surface-2)] border rounded-xl px-3 py-1.5 outline-none text-[12px] cursor-pointer"
                style={{
                  borderColor: gone ? "var(--clay)" : "var(--line)",
                  color: gone ? "var(--clay)" : "var(--ink)",
                }}
                value={l.name}
                title={loraNote(l.name, isZh)?.text ?? l.name}
                onChange={(e) => {
                  const next = [...zimageLoras];
                  next[i] = { ...next[i], name: e.target.value };
                  void saveZimageLoras(next);
                }}
              >
                {rowOptions(avail, l.name).map((o) => (
                  <option key={o.value} value={o.value}>
                    {o.label}
                  </option>
                ))}
              </select>
              <input
                type="number"
                step="0.05"
                min="0"
                max="2"
                value={l.strength}
                onChange={(e) => {
                  const next = [...zimageLoras];
                  next[i] = { ...next[i], strength: Number(e.target.value) };
                  void saveZimageLoras(next);
                }}
                className="w-20 bg-[var(--surface)] border border-[var(--line)] rounded-xl px-2 py-1.5 text-[12px] text-[var(--ink)] outline-none"
              />
              <button
                onClick={() => void saveZimageLoras(zimageLoras.filter((_, j) => j !== i))}
                className="px-2 py-1.5 text-[12px] text-[var(--ink-3)] hover:text-[var(--clay)] transition-colors"
              >
                {t("common.remove", { defaultValue: "移除" })}
              </button>
            </div>
            );
          })}
          <button
            onClick={() =>
              void saveZimageLoras([
                ...zimageLoras,
                { name: byArch("zimage")[0], strength: 0.8 },
              ])
            }
            className="w-fit px-3 py-1.5 text-[12px] rounded-xl border border-[var(--line)] hover:border-[var(--celadon)] text-[var(--ink-2)] transition-colors"
          >
            + 添加 LoRA
          </button>
          <span className="text-[11px] text-[var(--ink-3)] leading-relaxed">
            {t("image.zimageLoraHint", {
              defaultValue:
                "只在 Z-Image 纯文生图时生效。你在对话里自己点名 LoRA、或走图生图/设定卡时都不挂 —— 叠加会让权重失控。角色类 LoRA 通常还要在提示词里带触发词才生效。",
            })}
          </span>
        </div>
      )}

      <div className="h-px bg-[var(--line)] my-1" />

      {/* H3 参考生视频用哪份权重。默认兼容模式 —— 原生那份对照里串人，原因见 docs/reports/2026-10-06-原生Ref2VA串人排查.md */}
      <div className="flex flex-col">
        <span className="text-[14px] font-bold text-[var(--ink)] tracking-tight">
          {t("h3ref.title", { defaultValue: "H3 参考生视频权重" })}
        </span>
        <span className="text-[12px] text-[var(--ink-2)] mt-0.5 leading-relaxed">
          {t("h3ref.desc", {
            defaultValue:
              "参考生视频、接上一镜、长镜头用哪份权重。首帧 / 首尾帧 / 纯文生不受影响。下一次提交出片生效，正在跑的不变。",
          })}
        </span>
      </div>
      <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
        {(
          [
            {
              on: false,
              name: "兼容模式",
              badge: "默认推荐",
              desc: "用首尾帧（FL2VA）那份权重跑参考版式。一直在用的那条，多人镜头每个角色都能对上。",
              disabled: false,
            },
            {
              on: true,
              name: "原生 Ref2VA",
              badge: "实验",
              desc: h3NativeInstalled
                ? "官方参考生视频专用权重（55 GB），挂 Ref2VA 专用的 4 步加速 LoRA（通用那个挂在它上面会把苏晴换成星澜）。速度和兼容模式差不多。10-07 EP03 第 2 镜实测：人对了，但苏晴下半身串了星澜的纱裙、群演复制了一个 —— 还是实验，先别批量用。没装专用 LoRA 时会退回稠密 19 次（约 4~5 倍时长）。"
                : "本机没装原生 Ref2VA 权重，选了也只能走兼容模式。",
              disabled: !h3NativeInstalled,
            },
          ]
        ).map((e) => (
          <div
            key={String(e.on)}
            onClick={() => !e.disabled && pickH3Native(e.on)}
            className={`p-4 rounded-2xl border transition-all flex flex-col gap-2 select-none shadow-2xs ${
              e.disabled ? "opacity-50 cursor-not-allowed" : "cursor-pointer"
            } ${
              (h3Native && h3NativeInstalled) === e.on
                ? "bg-[var(--surface-2)] border-[var(--celadon)] ring-2 ring-[var(--celadon)]/20"
                : "bg-[var(--paper)] border-[var(--line)] hover:border-[var(--line-2)] hover:bg-[var(--surface-2)]"
            }`}
          >
            <div className="flex items-center justify-between gap-2">
              <span className="text-[13px] font-bold text-[var(--ink)]">{e.name}</span>
              <span className="px-2 py-0.5 text-[10px] font-semibold tracking-wide rounded-full bg-[var(--celadon)]/10 text-[var(--celadon)] border border-[var(--celadon)]/20">
                {e.badge}
              </span>
            </div>
            <p className="text-[11.5px] text-[var(--ink-3)] leading-relaxed">{e.desc}</p>
          </div>
        ))}
      </div>

      <div className="h-px bg-[var(--line)] my-1" />

      <div className="flex flex-col">
        <span className="text-[14px] font-bold text-[var(--ink)] tracking-tight">{t("tts.title", { defaultValue: "语音合成引擎" })}</span>
        <span className="text-[12px] text-[var(--ink-2)] mt-0.5 leading-relaxed">
          {t("tts.desc", { defaultValue: "管理实时语音与角色朗读的声学合成后端。" })}
        </span>
      </div>

      {/* **这条提示归属于语音这一块，不是整页。**
          原来挂在页面最顶上，于是「出图底模」也被它罩着 —— 而它说的是
          语音声学被角色接管了，和出图一点关系都没有。放在这里，
          罩住的正好是它真正只读的那些控件。 */}
      <PersonaLockBanner view={persona} what="语音声学" />

      <div className="flex flex-col gap-3.5 p-4 bg-[var(--paper)] border border-[var(--line)] rounded-2xl shadow-2xs">
        <label className="flex flex-col gap-1.5">
          <span className="text-[11.5px] font-bold text-[var(--ink-2)]">{t("tts.currentEngine", { defaultValue: "当前引擎" })}</span>
          <select
            className="bg-[var(--surface)] hover:bg-[var(--surface-2)] border border-[var(--line)] focus:border-[var(--celadon)] focus:ring-2 focus:ring-[var(--celadon)]/15 rounded-xl px-3 py-1.5 outline-none text-[12.5px] text-[var(--ink)] w-full transition-all cursor-pointer shadow-2xs"
            value={view.selected}
            onChange={(e) => void select(e.target.value)}
            disabled={locked || busy}
          >
            {view.engines.map((e) => {
              const p = probes[e.id];
              const mark =
                p === undefined ? "…" : p.reachable ? "●" : "○";
              return (
                <option key={e.id} value={e.id}>
                  {mark} {e.label}
                  {p?.reachable && p.voices > 0 ? `（${p.voices} 个音色）` : ""}
                  {p?.reachable && p.voices === 0 ? "（可用·克隆型）" : ""}
                  {p && !p.reachable ? "（服务未运行）" : ""}
                </option>
              );
            })}
          </select>
        </label>

        {/* 音色配置 */}
        <label className="flex flex-col gap-1.5 pt-2.5 border-t border-[var(--line)]/50">
          <span className="text-[11.5px] font-bold text-[var(--ink-2)]">
            {t("tts.synthVoice", { defaultValue: "合成音色" })}
            {voices.length > 0 && ` · ${voices.length} 个可用`}
          </span>
          <div className="flex items-center gap-2 w-full">
            <select
              className="flex-1 bg-[var(--surface)] hover:bg-[var(--surface-2)] border border-[var(--line)] focus:border-[var(--celadon)] focus:ring-2 focus:ring-[var(--celadon)]/15 rounded-xl px-3 py-1.5 outline-none text-[12.5px] text-[var(--ink)] transition-all cursor-pointer shadow-2xs"
              value={currentVoice}
              onChange={(e) => {
                if (!engineRow) return;
                const next = { ...engineRow, voice: e.target.value };
                void ttsEngineApi
                  .update(next)
                  .then(() => ttsEngineApi.list().then(setView))
                  .catch((err) => setMessage(`保存音色失败：${err}`));
              }}
              disabled={locked || !engineRow || voices.length === 0}
            >
              {currentVoice && !voices.some((v) => v.short_name === currentVoice) && (
                <option value={currentVoice}>{currentVoice}（不在当前引擎的列表里）</option>
              )}
              {voices.map((v) => (
                <option key={v.short_name} value={v.short_name}>
                  {v.short_name}
                  {v.locale ? ` · ${v.locale}` : ""}
                  {v.gender ? ` · ${v.gender}` : ""}
                </option>
              ))}
            </select>
            <button
              type="button"
              className="bg-[var(--surface-2)] hover:bg-[var(--surface-3)] text-[var(--ink)] border border-[var(--line)] rounded-xl px-4 py-1.5 text-[12px] font-medium transition-colors disabled:opacity-40 cursor-pointer shadow-2xs shrink-0"
              disabled={locked || !currentVoice || previewing}
              onClick={() => {
                setPreviewing(true);
                setVoiceMsg(null);
                void previewTtsVoice(currentVoice)
                  .then(playMp3Bytes)
                  .catch((err) => setVoiceMsg(`{t("tts.preview", { defaultValue: "试听" })}失败：${err}`))
                  .finally(() => setPreviewing(false));
              }}
            >
              {previewing ? "合成中…" : "试听"}
            </button>
          </div>
        </label>
        {voiceMsg && <p className="text-[12px] text-[var(--ink-2)] leading-relaxed bg-[var(--surface-2)] p-2.5 rounded-xl m-0">{voiceMsg}</p>}
      </div>

      {/* 当前引擎连不上时必须显式说出来。之前 Kokoro 没起来时每句话都报
          "TTS 请求失败"，而设置面板里一点迹象也没有 */}
      {current && probes[current.id] && !probes[current.id].reachable && (
        <p className="text-[12.5px] text-[var(--clay)] leading-relaxed bg-[color-mix(in_srgb,var(--clay)_10%,transparent)] border border-[color-mix(in_srgb,var(--clay)_32%,transparent)] p-3 rounded-lg m-0">
          这个引擎现在连不上：{probes[current.id].error ?? "未知原因"}
          <br />
          先把对应服务跑起来（<code className="bg-[color-mix(in_srgb,var(--clay)_10%,transparent)] px-1 rounded border border-[color-mix(in_srgb,var(--clay)_20%,transparent)]">./scripts/anima.sh start</code>），或换一个引擎。
        </p>
      )}

      {current && <p className="text-[12.5px] text-[var(--ink-3)] leading-relaxed bg-[var(--surface-2)] p-3 rounded-lg m-0">{KIND_NOTE[current.kind]}</p>}

      {current && current.kind !== "edge" && (
        <>
          <label className="flex flex-col gap-1.5 mt-2">
            <span className="text-[11.5px] font-semibold text-[var(--ink-2)]">服务地址</span>
            <input
              className="bg-[var(--surface)] border border-[var(--line)] rounded px-2 py-1.5 outline-none text-[12.5px] text-[var(--ink)] w-full font-mono"
              value={current.base_url}
              onChange={(e) =>
                setView({
                  ...view,
                  engines: view.engines.map((x) =>
                    x.id === current.id ? { ...x, base_url: e.target.value } : x,
                  ),
                })
              }
              onBlur={(e) => void saveCurrent({ base_url: e.target.value })}
              placeholder="http://127.0.0.1:9880"
            />
          </label>

          {/* 词边界是估算的，这点必须说出来 —— 阶段三对口型会受影响 */}
          <p className="text-[12.5px] text-[var(--amber)] leading-relaxed bg-[color-mix(in_srgb,var(--amber)_10%,transparent)] border border-[var(--amber)] p-3 rounded-lg m-0">
            本地引擎不返回词边界时间戳，嘴型时间轴由字符权重估算得出。
            够用，但不是测量值。
          </p>
        </>
      )}

      {current && CLONING.includes(current.kind) && (
        <>
          <label className="flex flex-col gap-1.5 mt-2">
            <span className="text-[11.5px] font-semibold text-[var(--ink-2)]">
              参考音频路径（wav，5~20 秒清晰人声）
              {current.kind === "cosy_voice2"
                ? " —— 本机路径，由 Anima 上传"
                : " —— 服务端能读到的路径"}
            </span>
            <input
              className="bg-[var(--surface)] border border-[var(--line)] rounded px-2 py-1.5 outline-none text-[12.5px] text-[var(--ink)] w-full font-mono"
              value={current.ref_audio_path ?? ""}
              onChange={(e) =>
                setView({
                  ...view,
                  engines: view.engines.map((x) =>
                    x.id === current.id
                      ? { ...x, ref_audio_path: e.target.value }
                      : x,
                  ),
                })
              }
              onBlur={(e) => void saveCurrent({ ref_audio_path: e.target.value })}
              placeholder="/path/to/ref.wav"
            />
          </label>

          <label className="flex flex-col gap-1.5">
            <span className="text-[11.5px] font-semibold text-[var(--ink-2)]">参考音频对应的文本（留空能跑，但音色相似度会明显下降）</span>
            <input
              className="bg-[var(--surface)] border border-[var(--line)] rounded px-2 py-1.5 outline-none text-[12.5px] text-[var(--ink)] w-full"
              value={current.ref_text ?? ""}
              onChange={(e) =>
                setView({
                  ...view,
                  engines: view.engines.map((x) =>
                    x.id === current.id ? { ...x, ref_text: e.target.value } : x,
                  ),
                })
              }
              onBlur={(e) => void saveCurrent({ ref_text: e.target.value })}
              placeholder="参考音频里说的那句话"
            />
          </label>

          <p className="text-[12.5px] text-[var(--ink-3)] leading-relaxed bg-[var(--surface-2)] p-3 rounded-lg m-0">
            这个引擎没有音色列表 —— 音色就是参考音频本身。换音色 = 换这个文件。
          </p>
        </>
      )}

      {current && RAW_PCM.includes(current.kind) && (
        <label className="flex flex-col gap-1.5 mt-2">
          <span className="text-[11.5px] font-semibold text-[var(--ink-2)]">
            裸 PCM 采样率（配错不会报错，只会整体音调偏移；CosyVoice2 是 24000）
          </span>
          <input
            className="bg-[var(--surface)] border border-[var(--line)] rounded px-2 py-1.5 outline-none text-[12.5px] text-[var(--ink)] w-full font-mono"
            value={current.pcm_sample_rate ?? ""}
            onChange={(e) =>
              setView({
                ...view,
                engines: view.engines.map((x) =>
                  x.id === current.id
                    ? { ...x, pcm_sample_rate: Number(e.target.value) || null }
                    : x,
                ),
              })
            }
            onBlur={(e) =>
              void saveCurrent({ pcm_sample_rate: Number(e.target.value) || null })
            }
            placeholder="24000"
          />
        </label>
      )}

      {current?.kind === "index_tts2" && (
        <label className="flex flex-col gap-1.5 mt-2">
          <span className="text-[11.5px] font-semibold text-[var(--ink-2)]">端点路径（各 fork 不统一，见过 /tts、/api/tts、/v1/tts）</span>
          <input
            className="bg-[var(--surface)] border border-[var(--line)] rounded px-2 py-1.5 outline-none text-[12.5px] text-[var(--ink)] w-full font-mono"
            value={current.endpoint_path ?? ""}
            onChange={(e) =>
              setView({
                ...view,
                engines: view.engines.map((x) =>
                  x.id === current.id ? { ...x, endpoint_path: e.target.value } : x,
                ),
              })
            }
            onBlur={(e) => void saveCurrent({ endpoint_path: e.target.value })}
            placeholder="/tts"
          />
        </label>
      )}

      {message && <div className="text-[12px] text-[var(--amber)] bg-[color-mix(in_srgb,var(--amber)_10%,transparent)] px-3 py-2 rounded-md mt-2">{message}</div>}
    </div>
  );
}
