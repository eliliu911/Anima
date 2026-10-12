import { useMemo } from "react";
import { MediaCornerDownload } from "../ui/MediaCornerDownload";
import { Handle, Position, type NodeProps } from "@xyflow/react";
import { BaseNodeWrapper } from "./BaseNodeWrapper";
import { useCanvasStore } from "../store/useCanvasStore";
import { selectContentNodes } from "../store/selectContentNodes";
import { NodeHeader } from "../ui/NodeHeader";
import { Maximize2, Loader2, Play } from "../ui/icons";
import { MediaThumb } from "../../components/studio/MediaThumb";
import { executeSingleNode } from "../engine/dagExecutor";
import { mediaKind, upscaleSource, upscaleStale } from "../utils/upscaleChain";
import type { UpscaleNode } from "../types";

/**
 * 超分节点：左边接图或视频，放大 ×2（RealESRGAN），右边照常往下连。
 *
 * 2026-09-14 同题对照（第 10 镜）：RealESRGAN 125 秒，和 LTX 生成式精修（51 分钟）肉眼几乎一样，
 * 镜头运动时还不出重影 —— 用户逐个看过后定的，所以这里不给选模型。
 */
export function UpscaleNodeComponent({ id, data, selected }: NodeProps<UpscaleNode>) {
  const removeNode = useCanvasStore((s) => s.removeNode);
  const nodes = useCanvasStore(selectContentNodes);
  const edges = useCanvasStore((s) => s.edges);
  const source = useMemo(() => upscaleSource(id, nodes, edges), [id, nodes, edges]);
  const self = nodes.find((n) => n.id === id);
  const stale = self ? upscaleStale(self, nodes, edges) : false;
  const out = data.outputUrl || null;
  const kind = out ? mediaKind(out) : source.kind;

  return (
    <div className="flex flex-col gap-2 select-none" style={{ width: 360 }}>
      <NodeHeader icon={<Maximize2 className="h-3.5 w-3.5 text-white/60" />} title="超分 ×2" onDelete={() => removeNode(id)} />
      <BaseNodeWrapper id={id} selected={selected} isGenerating={data.isGenerating} error={data.error} notes={data.notes} sourceHandleId="upscale-out">
        <Handle
          type="target"
          position={Position.Left}
          id="prompt-in"
          className="!w-2.5 !h-5 !rounded-full !bg-zinc-300 hover:!bg-blue-400 hover:!border-white/50 !border !border-zinc-800 shadow-md transition-transform hover:scale-125 cursor-crosshair z-10"
        />
        <div className="flex w-full flex-col gap-2.5 p-3">
          <div className="relative aspect-video w-full overflow-hidden rounded-lg bg-black/40">
            {out ? (
              <>
                <MediaThumb path={out} kind={mediaKind(out)} className="h-full w-full object-contain" />
                {/* 和视频节点画廊右上角那个是同一个样子。超分节点不走画廊，以前这里一个下载入口都没有 */}
                {!data.isGenerating && <MediaCornerDownload path={out} />}
              </>
            ) : (
              <div className="flex h-full items-center justify-center px-4 text-center text-[11px] leading-relaxed text-white/40">
                {data.isGenerating
                  ? kind === "video" ? "正在放大…每 1 秒片子约 25 秒" : "正在放大…几秒"
                  : source.path ? "点下面「开始超分」" : source.reason}
              </div>
            )}
          </div>

          {stale && (
            <p className="rounded-md bg-amber-400/10 px-2 py-1.5 text-[11px] leading-relaxed text-amber-200/80">
              上游换了{source.kind === "video" ? "视频" : "图"}，这个结果还是旧的 —— 重新超分一次
            </p>
          )}

          <button
            type="button"
            disabled={data.isGenerating || !source.path}
            onClick={() => void executeSingleNode(id)}
            className="nodrag nopan flex items-center justify-center gap-1.5 rounded-md bg-white/10 px-3 py-1.5 text-[12px] text-white/85 hover:bg-white/20 disabled:opacity-40"
          >
            {data.isGenerating ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Play className="h-3.5 w-3.5" />}
            {data.isGenerating ? "超分中…" : out ? "重新超分" : "开始超分"}
          </button>

          <p className="text-[11px] leading-relaxed text-white/40">
            宽高各放大一倍，细节更锐；视频的时长和声音不变。图片几秒，视频每 1 秒片子约 25 秒。
            已经是 1080P 以上的视频不再放大。结果可以接着往右连。
          </p>
        </div>
      </BaseNodeWrapper>
    </div>
  );
}
