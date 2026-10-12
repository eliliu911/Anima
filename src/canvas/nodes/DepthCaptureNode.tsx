import { useMemo } from "react";
import { MediaCornerDownload } from "../ui/MediaCornerDownload";
import { Handle, Position, type NodeProps } from "@xyflow/react";
import { BaseNodeWrapper } from "./BaseNodeWrapper";
import { useCanvasStore } from "../store/useCanvasStore";
import { selectContentNodes } from "../store/selectContentNodes";
import { NodeHeader } from "../ui/NodeHeader";
import { Layers, Loader2, Play } from "../ui/icons";
import { MediaThumb, pathFromMediaUrl } from "../../components/studio/MediaThumb";
import { executeSingleNode } from "../engine/dagExecutor";
import { depthSourceVideo } from "../utils/depthChain";
import type { DepthCaptureNode } from "../types";

/**
 * 深度动作捕捉节点（照 libtv）。
 *
 * 左边接一个视频节点，点「开始捕捉」抽出黑白深度视频（越近越白）；
 * 右边接一个视频节点、再给那个视频节点连一张人物图，就能把这段动作换到那个人身上。
 */
export function DepthCaptureNodeComponent({ id, data, selected }: NodeProps<DepthCaptureNode>) {
  const removeNode = useCanvasStore((s) => s.removeNode);
  const nodes = useCanvasStore(selectContentNodes);
  const edges = useCanvasStore((s) => s.edges);
  const source = useMemo(() => depthSourceVideo(id, nodes, edges), [id, nodes, edges]);
  const stale = Boolean(data.depthVideoUrl && source.path && data.sourceVideoPath !== source.path);
  const depth = data.depthVideoUrl ? pathFromMediaUrl(data.depthVideoUrl) ?? data.depthVideoUrl : null;

  return (
    <div className="flex flex-col gap-2 select-none" style={{ width: 360 }}>
      <NodeHeader
        icon={<Layers className="h-3.5 w-3.5 text-white/60" />}
        title="深度动作捕捉"
        onDelete={() => removeNode(id)}
      />
      <BaseNodeWrapper id={id} selected={selected} isGenerating={data.isGenerating} error={data.error} notes={data.notes} sourceHandleId="depth-out">
        <Handle
          type="target"
          position={Position.Left}
          id="prompt-in"
          className="!w-2.5 !h-5 !rounded-full !bg-zinc-300 hover:!bg-blue-400 hover:!border-white/50 !border !border-zinc-800 shadow-md transition-transform hover:scale-125 cursor-crosshair z-10"
        />
        <div className="flex w-full flex-col gap-2.5 p-3">
          <div className="relative aspect-video w-full overflow-hidden rounded-lg bg-black/40">
            {depth ? (
              <>
                <MediaThumb path={depth} kind="video" className="h-full w-full object-contain" />
                {!data.isGenerating && <MediaCornerDownload path={depth} />}
              </>
            ) : (
              <div className="flex h-full items-center justify-center px-4 text-center text-[11px] leading-relaxed text-white/40">
                {data.isGenerating ? "正在抽深度…几秒到一分钟" : source.path ? "点下面「开始捕捉」，抽出这段视频里的动作" : source.reason}
              </div>
            )}
          </div>

          {stale && (
            <p className="rounded-md bg-amber-400/10 px-2 py-1.5 text-[11px] leading-relaxed text-amber-200/80">
              上游换了视频，这段深度还是旧的 —— 重新捕捉一次
            </p>
          )}

          <button
            type="button"
            disabled={data.isGenerating || !source.path}
            onClick={() => void executeSingleNode(id)}
            className="nodrag nopan flex items-center justify-center gap-1.5 rounded-md bg-white/10 px-3 py-1.5 text-[12px] text-white/85 hover:bg-white/20 disabled:opacity-40"
          >
            {data.isGenerating ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Play className="h-3.5 w-3.5" />}
            {data.isGenerating ? "捕捉中…" : depth ? "重新捕捉" : "开始捕捉"}
          </button>

          <p className="text-[11px] leading-relaxed text-white/40">
            下一步：右边接一个视频节点，再给它连一张人物图 —— 就把这段动作换到那个人身上。
            人物图用单人全身像最好；三视图也行，会自动挑正面全身。
          </p>
        </div>
      </BaseNodeWrapper>
    </div>
  );
}
