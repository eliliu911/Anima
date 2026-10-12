import { useEffect, useMemo, useState } from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";
import { BaseNodeWrapper } from "./BaseNodeWrapper";
import { useCanvasStore } from "../store/useCanvasStore";
import { selectContentNodes } from "../store/selectContentNodes";
import { NodeHeader } from "../ui/NodeHeader";
import { Volume2, Loader2 } from "../ui/icons";
import { canvasSpeak } from "../../api/canvasAudio";
import { listTtsVoices, type TtsVoiceInfo } from "../../api/voice";
import { upstreamSpeechText } from "../utils/audioChain";
import { mediaSrc } from "../../components/studio/MediaThumb";
import { ComposingTextarea } from "../../components/ComposingInput";
import type { AudioNode } from "../types";

/**
 * 试音色：同一段词，几把嗓子并排听。
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
  const [listErr, setListErr] = useState("");

  useEffect(() => {
    let live = true;
    listTtsVoices()
      .then((v) => live && setVoices(v))
      .catch((e) => live && setListErr(String(e)));
    return () => { live = false; };
  }, []);

  /** 上游接了提示词/分镜就念那一段 —— 三个节点挂同一个上游才比得公平。 */
  const upstream = useMemo(() => upstreamSpeechText(id, nodes, edges), [id, nodes, edges]);
  const text = (upstream || data.text || "").trim();

  const speak = async () => {
    if (!text || data.isGenerating) return;
    updateNodeData(id, { isGenerating: true, error: undefined });
    try {
      const out = await canvasSpeak(id, { text, voice: data.voice, engineId: data.engineId });
      updateNodeData(id, {
        outputAudioUrl: out.audio_path,
        usedVoice: out.voice, usedEngineId: out.engine_id, secs: out.secs,
        isGenerating: false,
      });
    } catch (e) {
      updateNodeData(id, { error: String(e), isGenerating: false });
    }
  };

  return (
    <div className="flex flex-col gap-2 select-none" style={{ width: 300 }}>
      <NodeHeader icon={<Volume2 className="h-3.5 w-3.5 text-white/60" />} title="试音色" onDelete={() => removeNode(id)} />
      <BaseNodeWrapper id={id} selected={selected} isGenerating={data.isGenerating} error={data.error} sourceHandleId="audio-out">
        <Handle type="target" position={Position.Left} id="text-in"
          className="!w-2.5 !h-5 !rounded-full !bg-zinc-300 hover:!bg-blue-400 hover:!border-white/50 !border !border-zinc-800 shadow-md transition-transform hover:scale-125 cursor-crosshair z-10" />
        <div className="flex w-full flex-col gap-2 p-3 text-[12px]">
          {/* **必须用 `ComposingTextarea`，不能用裸 textarea。** 裸的每敲一个
              字母就回灌一次节点数据，把输入法的组合缓冲冲掉 —— 打拼音出来是
              `qqiqinqinaqin'ai` 那种串（2026-09-20 用户撞到，而这个组件的
              模块注释里早就记着同一个症状）。
              `nodrag nopan nowheel` + 挡住 keydown：不挡的话空格会平移画布。 */}
          {upstream ? (
            <p className="text-white/55 line-clamp-2" title={upstream}>念上游的词：{upstream}</p>
          ) : (
            <ComposingTextarea
              rows={2}
              value={data.text ?? ""}
              onCommit={(v) => updateNodeData(id, { text: v })}
              onKeyDown={(e) => e.stopPropagation()}
              placeholder="要念的词。左边接个提示词节点，几个音色节点就能念同一段"
              className="nodrag nopan nowheel w-full rounded-md px-2 py-1 bg-white/5 text-white outline-none resize-none"
            />
          )}

          {listErr ? (
            <p className="text-red-300">音色列表读不出来：{listErr}</p>
          ) : (
            <select
              className="nodrag nopan w-full rounded-md px-2 py-1 bg-white/5 text-white outline-none"
              value={data.voice ?? ""}
              onChange={(e) => updateNodeData(id, { voice: e.target.value })}
            >
              <option value="" className="bg-zinc-900">当前引擎的默认音色</option>
              {/* **值是 `short_name`** —— `name` 是给人看的长名，
                  填进 config.tts.voice 的必须是 short_name，填错会静默退回默认 */}
              {(voices ?? []).map((v) => (
                <option key={v.short_name} value={v.short_name} className="bg-zinc-900">
                  {v.name || v.short_name}{v.gender ? ` · ${v.gender}` : ""}
                </option>
              ))}
            </select>
          )}

          <button type="button" disabled={!text || data.isGenerating} onClick={() => void speak()}
            className="nodrag nopan h-7 rounded-md bg-white/10 hover:bg-white/20 text-white disabled:opacity-40 flex items-center justify-center gap-1.5">
            {data.isGenerating ? <><Loader2 className="h-3 w-3 animate-spin" />合成中…</> : "念一遍"}
          </button>

          {data.outputAudioUrl && !data.isGenerating && (
            <>
              {/* **必须走 `anima-media://`。** `file://` 被 CSP 拦掉，
                  Tauri 自带的 `asset://` 在这个应用里实测显示不出来
                  （见 `MediaThumb` 的模块注释）。两种都是静默失败 ——
                  播放器出现了但一点声音没有。 */}
              <audio controls src={mediaSrc(data.outputAudioUrl, data.secs)} className="w-full h-7" />
              {/* **回显实际用的那把嗓子。** 传了个不存在的音色时引擎会悄悄
                  退回默认 —— 这一行是唯一看得出来的地方。 */}
              <p className="text-white/40">
                {data.usedVoice}（{data.usedEngineId}）· {data.secs?.toFixed(1)}s
                {data.voice && data.usedVoice !== data.voice && (
                  <span className="text-amber-300"> · 引擎没有你选的那个音色，退回了默认</span>
                )}
              </p>
            </>
          )}

          {/* 取舍写在节点上，不藏在文档里 */}
          <p className="text-white/35 leading-relaxed">
            配音会顶掉 H3 的原声，而<b className="text-white/55">口型是跟着原声做的</b> ——
            换成配音口型就对不上。这是取舍，不是两全。
          </p>
        </div>
        <Handle type="source" position={Position.Right} id="audio-out"
          className="!w-2.5 !h-5 !rounded-full !bg-zinc-300 hover:!bg-blue-400 hover:!border-white/50 !border !border-zinc-800 shadow-md transition-transform hover:scale-125 cursor-crosshair z-10" />
      </BaseNodeWrapper>
    </div>
  );
}
