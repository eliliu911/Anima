import { useEffect, useMemo, useState } from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";
import { BaseNodeWrapper } from "./BaseNodeWrapper";
import { useCanvasStore } from "../store/useCanvasStore";
import { selectContentNodes } from "../store/selectContentNodes";
import { NodeHeader } from "../ui/NodeHeader";
import { Music, Volume2, Loader2 } from "../ui/icons";
import { NodePanelDock } from "../ui/NodePanelDock";
import { AudioOperationPanel } from "./panels/AudioOperationPanel";
import { uploadAudioInput } from "../utils/audioClips";
import { canvasSpeak } from "../../api/canvasAudio";
import { listTtsVoices, ttsEngineApi, type TtsEngineConfig, type TtsVoiceInfo } from "../../api/voice";
import { emptyReferenceAudio, referenceAudio, upstreamSpeechText } from "../utils/audioChain";
import { mediaSrc } from "../../components/studio/MediaThumb";
import type { AudioNode } from "../types";

/**
 * **音频节点**（2026-10-13 起一个节点三种用法，照 LibTV）：上传一段 = 素材；打字生成 = 文字转语音；
 * 左边连一个音频 = 声音复制。**节点里只放内容（播放器），设置全在选中后下方的设置卡里**（`AudioOperationPanel`）——
 * 用户：「所有的设置都在设置卡里，不要在节点里放那么多按钮。包括模型和音频设置。」
 *
 * 最早它叫「试音色」：同一段词，几把嗓子并排听。
 *
 * 配音能力早就有（`cinema_dub`），但绑死在「某一集的某一镜」上 ——
 * 换个音色要走「改角色资产 → 整镜重新配音 → 听」，比三把嗓子得来回三趟，
 * 每趟还会覆盖掉上一趟的结果。
 *
 * **口型这件事写在节点上，不藏在文档里。** H3 原声的口型是跟着它自己生成
 * 的那段语音做的，换成 TTS 配音口型就对不上 —— 这是个真取舍。
 * 这个节点不改变取舍，只是让取舍看得见。
 */
export function AudioNodeComponent({ id, data, selected }: NodeProps<AudioNode>) {
  const removeNode = useCanvasStore((s) => s.removeNode);
  const updateNodeData = useCanvasStore((s) => s.updateNodeData);
  const nodes = useCanvasStore(selectContentNodes);
  const edges = useCanvasStore((s) => s.edges);
  const [voices, setVoices] = useState<TtsVoiceInfo[] | null>(null);
  const [engines, setEngines] = useState<{ list: TtsEngineConfig[]; selected: string } | null>(null);
  const upload = (file: File) => {
    void uploadAudioInput(id, file).catch((e) => updateNodeData(id, { error: String(e) }));
  };

  useEffect(() => {
    let live = true;
    listTtsVoices()
      .then((v) => live && setVoices(v))
      .catch(() => live && setVoices([]));
    ttsEngineApi.list().then((v) => live && setEngines({ list: v.engines, selected: v.selected })).catch(() => {});
    return () => { live = false; };
  }, []);

  // **声音复制**（照 LibTV）：上游直接连着的音频节点 = 参考音频
  const ref = useMemo(() => referenceAudio(id, nodes, edges), [id, nodes, edges]);
  const refEmpty = useMemo(() => emptyReferenceAudio(id, nodes, edges), [id, nodes, edges]);
  // 换了参考音频，上次的参考文字就不作数了
  const cloneForThisRef = data.cloneUsed && ref && data.cloneUsed.ref_path === ref.path ? data.cloneUsed : undefined;

  /** 上游接了提示词/分镜就念那一段 —— 三个节点挂同一个上游才比得公平。 */
  const upstream = useMemo(() => upstreamSpeechText(id, nodes, edges), [id, nodes, edges]);
  const text = (upstream || data.text || "").trim();

  const speak = async () => {
    if (!text || data.isGenerating) return;
    if (refEmpty) { updateNodeData(id, { error: "连着的音频节点还没有音频 —— 先上传一段（或先让上游那个音频节点生成一遍）" }); return; }
    updateNodeData(id, { isGenerating: true, error: undefined });
    try {
      const out = await canvasSpeak(id, {
        text, voice: data.voice, engineId: data.engineId,
        refAudio: ref?.path, refText: cloneForThisRef ? data.refText : undefined,
        speed: data.speed, pitch: data.pitch, volume: data.volume, sampleRate: data.sampleRate, format: data.format,
      });
      updateNodeData(id, {
        outputAudioUrl: out.audio_path,
        usedVoice: out.clone ? `复制：${ref?.name ?? "参考音频"}` : out.voice, usedEngineId: out.engine_id, secs: out.secs,
        cloneUsed: out.clone && ref ? { ...out.clone, ref_path: ref.path } : undefined,
        // 生成的覆盖掉上传的那段
        uploaded: false, label: undefined,
        refText: out.clone ? out.clone.ref_text : data.refText,
        isGenerating: false,
      });
    } catch (e) {
      updateNodeData(id, { error: String(e), isGenerating: false });
    }
  };

  return (
    <div className="flex flex-col gap-2 select-none" style={{ width: 320 }}>
      <NodeHeader icon={<Volume2 className="h-3.5 w-3.5 text-white/60" />} title={ref ? "声音复制" : "音频"} onDelete={() => removeNode(id)} />
      <BaseNodeWrapper id={id} selected={selected} isGenerating={data.isGenerating} error={data.error} sourceHandleId="audio-out">
        <Handle type="target" position={Position.Left} id="text-in"
          className="!w-2.5 !h-5 !rounded-full !bg-zinc-300 hover:!bg-blue-400 hover:!border-white/50 !border !border-zinc-800 shadow-md transition-transform hover:scale-125 cursor-crosshair z-10" />
        {/* 节点里只放内容：有音频放播放器，没有放空状态（照 LibTV） */}
        <div className="flex min-h-[150px] w-full flex-col justify-center gap-2 p-4 text-[12px]">
          {data.isGenerating ? (
            <div className="flex flex-col items-center gap-2 text-white/50"><Loader2 className="h-5 w-5 animate-spin" />生成中…</div>
          ) : data.outputAudioUrl ? (
            <>
              {/* **必须走 `anima-media://`。** `file://` 被 CSP 拦掉，Tauri 自带的 `asset://` 在这个应用里实测显示不出来
                  （见 `MediaThumb` 的模块注释）。两种都是静默失败 —— 播放器出现了但一点声音没有。pcm 是裸数据，放不了 */}
              {data.outputAudioUrl.endsWith(".pcm")
                ? <p className="text-white/50">pcm 裸数据，放不了（在文件夹里找：{data.outputAudioUrl.split("/").pop()}）</p>
                : <audio controls src={mediaSrc(data.outputAudioUrl, data.secs)} className="nodrag nopan h-8 w-full" />}
              {/* 回显实际用的那把嗓子：传了不存在的音色时引擎会悄悄退回默认，这一行是唯一看得出来的地方 */}
              <p className="truncate text-white/45" title={data.outputAudioUrl}>
                {data.uploaded ? data.label : <>{data.usedVoice}<span className="text-white/30">（{data.usedEngineId}）</span></>} · {data.secs?.toFixed(1)} 秒
                {!data.cloneUsed && !data.uploaded && data.voice && data.usedVoice !== data.voice && <span className="text-amber-300"> · 引擎没有你选的音色，退回了默认</span>}
              </p>
            </>
          ) : (
            <div className="flex flex-col items-center gap-3 text-white/40">
              <Music className="h-7 w-7 text-white/25" />
              <span>{ref ? "选中节点，在下面写要念的话" : "选中节点：上传一段，或写字生成"}</span>
            </div>
          )}
        </div>
        <Handle type="source" position={Position.Right} id="audio-out"
          className="!w-2.5 !h-5 !rounded-full !bg-zinc-300 hover:!bg-blue-400 hover:!border-white/50 !border !border-zinc-800 shadow-md transition-transform hover:scale-125 cursor-crosshair z-10" />
      </BaseNodeWrapper>

      {/* 设置卡：选中时挂在下方，不跟画布缩放（NodePanelDock，和视频 / 图片节点同一个外壳） */}
      <NodePanelDock visible={Boolean(selected)}>
        <div className="animate-in fade-in-0 duration-150">
          <AudioOperationPanel
            data={data}
            upstreamText={upstream}
            reference={ref}
            referenceEmpty={refEmpty}
            engines={engines}
            voices={voices}
            isGenerating={!!data.isGenerating}
            onChange={(patch) => updateNodeData(id, patch)}
            onGenerate={() => void speak()}
            onUpload={upload}
          />
        </div>
      </NodePanelDock>
    </div>
  );
}
