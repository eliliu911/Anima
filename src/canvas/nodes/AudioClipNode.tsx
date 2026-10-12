import { useRef, useState } from "react";
import type { NodeProps } from "@xyflow/react";
import { BaseNodeWrapper } from "./BaseNodeWrapper";
import { useCanvasStore } from "../store/useCanvasStore";
import { NodeHeader } from "../ui/NodeHeader";
import { Music, Upload, Loader2 } from "../ui/icons";
import { mediaSrc } from "../../components/studio/MediaThumb";
import { uploadAudioInput } from "../utils/audioClips";
import type { AudioClipNode } from "../types";

/**
 * 音频素材：放一段本机音频，从右边连到视频节点上当 H3 的参考音频 `<Audio N>`（2026-10-08）。
 *
 * 不生成东西 —— 和图片素材节点一样只是「放文件」。要念词去用「试音色」。
 */
export function AudioClipNodeComponent({ id, data, selected }: NodeProps<AudioClipNode>) {
  const removeNode = useCanvasStore((s) => s.removeNode);
  const input = useRef<HTMLInputElement>(null);
  const [err, setErr] = useState("");
  const pick = (file?: File) => {
    if (!file) return;
    setErr("");
    void uploadAudioInput(id, file).catch((e) => setErr(String(e)));
  };
  return (
    <div className="flex flex-col gap-2 select-none" style={{ width: 320 }}>
      <NodeHeader icon={<Music className="h-3.5 w-3.5 text-white/60" />} title={String(data.label || "音频素材")} onDelete={() => removeNode(id)} />
      <BaseNodeWrapper id={id} selected={selected} error={data.error} sourceHandleId="audio-out">
        <div className="flex w-full flex-col gap-2 p-3 text-[12px]">
          {data.audioPath ? (
            <>
              {/* 走 `anima-media://`（`file://` 被 CSP 拦，见试音色节点） */}
              <audio controls src={mediaSrc(data.audioPath)} className="nodrag nopan h-8 w-full" />
              <p className="text-white/45">
                {data.secs ? `${data.secs.toFixed(1)} 秒` : "时长读不出来"}
                {data.secs && data.secs > 15 ? " · H3 一条最长 15 秒，出片时只用前面那段（截到视频节点的时长）" : ""}
              </p>
            </>
          ) : (
            <p className="text-white/40">还没放音频</p>
          )}
          <button type="button" disabled={Boolean(data.referenceLoading)} onClick={() => input.current?.click()}
            className="nodrag nopan flex h-7 items-center justify-center gap-1.5 rounded-md bg-white/10 text-white/80 hover:bg-white/20 disabled:opacity-40">
            {data.referenceLoading ? <><Loader2 className="h-3 w-3 animate-spin" />保存中…</> : <><Upload className="h-3 w-3" />{data.audioPath ? "换一段" : "上传音频"}</>}
          </button>
          <input ref={input} type="file" accept="audio/*,.mp3,.wav,.m4a,.flac,.ogg" className="nodrag nopan hidden"
            onChange={(e) => { const f = e.target.files?.[0]; e.target.value = ""; pick(f); }} />
          {err && <p role="alert" className="break-words text-red-400">{err}</p>}
          <p className="leading-relaxed text-white/35">
            从右边连到<b className="text-white/55">视频节点</b>：H3 参考生视频，提示词里是 <code>&lt;Audio 1&gt;</code>（或写 @音频1）。
            还要再连至少一张图（人物 / 场景）。整段照用就在提示词里写 <code>fully_copy</code>，口型会跟着它
          </p>
        </div>
      </BaseNodeWrapper>
    </div>
  );
}
