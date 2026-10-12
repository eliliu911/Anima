import { useMemo } from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";
import { NodePanelDock } from "../ui/NodePanelDock";
import { BaseNodeWrapper } from "./BaseNodeWrapper";
import { useCanvasStore } from "../store/useCanvasStore";
import { selectContentNodes } from "../store/selectContentNodes";
import { DEFAULT_IMAGE_SIZE } from "../utils/dimensionUtils";
import { executeNodePipeline, aggregateNodeInputs, videoInputReferences } from "../engine/dagExecutor";
import { inferImageMode } from "../utils/imageMode";
import { MediaThumb } from "../../components/studio/MediaThumb";
import { ImageOperationPanel } from "./panels/ImageOperationPanel";
import { NodeHeader } from "../ui/NodeHeader";
import { ImageIcon, Upload } from "../ui/icons";
import { CanvasImageUploadButton } from "../ui/CanvasImageUploadButton";
import { CanvasMediaUploadButton } from "../ui/CanvasMediaUploadButton";
import { CanvasMediaGallery } from "../ui/CanvasMediaGallery";
import { BeforeAfter } from "../ui/BeforeAfter";
import type { ImageGenNode } from "../types";
import { CanvasImageAssetToolbar } from "../ui/CanvasImageAssetToolbar";

export function ImageGenNodeComponent({ id, data, selected }: NodeProps<ImageGenNode>) {
  const updateNodeData = useCanvasStore((s) => s.updateNodeData);
  const removeNode = useCanvasStore((s) => s.removeNode);
  const addNode = useCanvasStore((s) => s.addNode);
  const nodes = useCanvasStore(selectContentNodes);
  const edges = useCanvasStore((s) => s.edges);

  const upstream = useMemo(() => {
    return aggregateNodeInputs(id, nodes, edges);
  }, [id, nodes, edges]);

  const imageRefs = useMemo(() => {
    try {
      return videoInputReferences(id, nodes, edges);
    } catch {
      return [];
    }
  }, [id, nodes, edges]);
  const imageMode = inferImageMode(imageRefs.length, data.engine || "zimage", null, {
    pose: data.poseFromReference, sceneSheet: data.sceneSheet,
  });

  const hasUpstreamContent = Boolean(
    data.references?.length || upstream.combinedPrompt?.trim() ||
    upstream.inputImageUrl ||
    upstream.character ||
    edges.some((e) => e.target === id)
  );

  const handleGenerate = async () => {
    if (data.isGenerating) return;
    await executeNodePipeline(id);
  };

  const outputList = data.outputImages?.length
    ? data.outputImages
    : (data.outputImageUrl ? [data.outputImageUrl] : []);
  const count = outputList.length || data.imageCount || 1;
  const isExpanded = Boolean(data.isExpanded && count > 1);

  // 动态卡片宽度：展开画册时水平平铺拓展
  const nodeWidth = isExpanded ? Math.min(960, Math.max(440, count * 310 + 24)) : 440;

  const handleApplyToCanvas = (path: string) => {
    const thisNode = useCanvasStore.getState().nodes.find((n) => n.id === id);
    const pos = thisNode
      ? { x: thisNode.position.x + (isExpanded ? nodeWidth + 40 : 480), y: thisNode.position.y }
      : { x: 300, y: 300 };

    addNode("imageGen", pos, {
      outputImageUrl: path,
      outputImages: [path],
      prompt: data.prompt || "",
      engine: data.engine || "zimage",
      width: data.width || DEFAULT_IMAGE_SIZE.width,
      height: data.height || DEFAULT_IMAGE_SIZE.height,
      ratio: data.ratio,
      resolution: data.resolution,
    });
  };

  const hasUpstreamNodes = edges.some((e) => e.target === id);

  if (data.referenceOnly) return (
    <div style={{ width: 400 }} className="flex flex-col gap-2">
      <NodeHeader icon={<Upload className="h-3.5 w-3.5 text-white/60" />}
        title={String(data.label || "外部素材")} onDelete={() => removeNode(id)} />
      <BaseNodeWrapper id={id} selected={selected} sourceHandleId="image-out">
        <CanvasImageAssetToolbar nodeId={id} selected={selected} path={data.outputImageUrl} name={String(data.label || "")} description={data.prompt} />
        {/* 输入桩：只用来表达「这张图是从哪来的」（如全景查看器截出来的图）。
            素材节点不执行，连进来的东西不会改它的图 */}
        <Handle
          type="target"
          position={Position.Left}
          id="prompt-in"
          className="!w-2.5 !h-5 !rounded-full !bg-zinc-300 hover:!bg-blue-400 hover:!border-white/50 !border !border-zinc-800 shadow-md transition-transform hover:scale-125 cursor-crosshair z-10"
        />
        <div style={{ height: 260, borderRadius: "inherit", overflow: "hidden" }}>
          {data.outputImageUrl ? (
            <MediaThumb path={data.outputImageUrl} alt={String(data.label || "参考素材")}
              className="h-full w-full object-contain" />
          ) : (
            <div className="flex h-full flex-col items-center justify-center gap-2 text-[11px] text-white/40">
              <CanvasMediaUploadButton nodeId={id} />
              <span>图片或音频都行，自动认。放好之后，从右边的 + 连一个图片或视频节点出去</span>
            </div>
          )}
        </div>
      </BaseNodeWrapper>
    </div>
  );

  return (
    <div className="flex flex-col gap-2 select-none transition-all duration-200" style={{ width: nodeWidth }}>
      {/* 顶部悬浮标题栏 (展开时显示 画册 · N 张，未展开显示 图片节点 与分辨率) */}
      <NodeHeader
        icon={<ImageIcon className="h-3.5 w-3.5 text-white/60" />}
        title={isExpanded ? `画册 · ${count} 张` : data.imageTool ? String(data.label || "图片工具") : "图片节点"}
        onDelete={() => removeNode(id)}
        action={
          <div className="flex items-center gap-1.5">
            {isExpanded ? (
              <button
                type="button"
                onClick={() => updateNodeData(id, { isExpanded: false })}
                className="px-2 py-0.5 rounded text-[11px] font-medium bg-white/10 hover:bg-white/20 text-white/80 transition-colors"
                title="收起画册为堆叠模式"
              >
                收起
              </button>
            ) : (
              <>
                <span className="flex items-center gap-1 rounded bg-black/40 border border-white/10 px-2 py-0.5 text-[11px] font-mono text-white/60">
                  <ImageIcon className="h-3 w-3 opacity-60" />
                  <span>{data.width || DEFAULT_IMAGE_SIZE.width}×{data.height || DEFAULT_IMAGE_SIZE.height}</span>
                </span>
                <CanvasImageUploadButton nodeId={id} label="上传图片" />
              </>
            )}
          </div>
        }
      />

      {/* 主卡片: 单层纯净灰黑统一背景，直接挂载胶囊连接桩 */}
      <BaseNodeWrapper
        id={id}
        selected={selected}
        isGenerating={data.isGenerating}
        error={data.error}
        notes={data.notes}
        sourceHandleId="image-out"
      >
        <CanvasImageAssetToolbar nodeId={id} selected={selected} path={data.outputImageUrl || data.references?.[0]?.path}
          name={typeof data.label === "string" ? data.label : undefined} description={data.prompt} />
        {/* 左侧目标胶囊输入桩 */}
        <Handle
          type="target"
          position={Position.Left}
          id="prompt-in"
          className="!w-2.5 !h-5 !rounded-full !bg-zinc-300 hover:!bg-blue-400 hover:!border-white/50 !border !border-zinc-800 shadow-md transition-transform hover:scale-125 cursor-crosshair z-10"
        />

        {/* 画册 / 堆叠多卡渲染主体 */}
        <CanvasMediaGallery
          jobId={(data as { jobId?: string }).jobId}
          nodeId={id}
          kind="image"
          items={outputList}
          primaryItem={data.outputImageUrl}
          targetCount={data.imageCount || 1}
          isGenerating={data.isGenerating}
          width={data.width || DEFAULT_IMAGE_SIZE.width}
          height={data.height || DEFAULT_IMAGE_SIZE.height}
          isExpanded={isExpanded}
          onToggleExpand={() => updateNodeData(id, { isExpanded: !isExpanded })}
          onSelectPrimary={(path) => updateNodeData(id, { outputImageUrl: path })}
          onApplyToCanvas={handleApplyToCanvas}
          placeholder={
            data.references?.[0] ? (
              <>
                {/* 不再贴「源图 · 未生成」：上传的图多半就是拿来给下游当素材的，那句话让人以为还没弄好（用户 2026-09-14） */}
                <MediaThumb path={data.references[0].path} alt="上传的图" className="w-full h-full object-contain" />
              </>
            ) : hasUpstreamNodes ? (
              <div className="relative w-full h-full flex items-center justify-center">
                <ImageIcon className="h-16 w-16 text-white/[0.04] pointer-events-none" />
              </div>
            ) : (
              <div className="relative w-full h-full flex flex-col items-start justify-center p-6">
                <span className="text-xs text-white/40 mb-2">试试：</span>
                <CanvasImageUploadButton nodeId={id} label="上传图片" />
                <div className="absolute inset-0 flex items-center justify-center pointer-events-none">
                  <ImageIcon className="h-16 w-16 text-white/[0.04]" />
                </div>
              </div>
            )
          }
        />
        {/* **改前 / 改后并排比。** 只在局部修改的节点上出现 ——
            指令式编辑最典型的失败是整张重画，而那不报错、只看结果看不出来。 */}
        {data.editSourcePath && data.outputImageUrl && !data.isGenerating && (
          <BeforeAfter before={data.editSourcePath} after={data.outputImageUrl} />
        )}
        {/* 模式说明和开关只在选中时出现：只拿来放素材的节点平时不该挂一堆「图生图 / 照姿势」 */}
        {!data.imageTool && selected && (
          <div className="nodrag nopan flex flex-col gap-1.5 border-t border-white/10 px-3 py-2 text-[11px]">
            {(imageRefs.length > 0 || data.poseFromReference || data.sceneSheet) && (
              <p className={`leading-relaxed ${imageMode.mode === "t2i" || imageMode.mode === "i2i" ? "text-white/45" : "text-emerald-200/80"}`}>
                {imageMode.reason}
                {imageMode.mode === "ref2i" && imageRefs.length > 1 && `：${imageRefs.map((r, i) => `${i + 1}. ${r.name || "图片"}`).join("　")}`}
              </p>
            )}
            {imageRefs.length > 0 ? (
              <label className="flex items-center gap-1.5 text-white/60" title="连着的第一张图只取人物姿势（自动抽骨架），画面内容看提示词。只能用 SDXL">
                <input type="checkbox" checked={Boolean(data.poseFromReference)} disabled={data.isGenerating}
                  onChange={(e) => updateNodeData(id, { poseFromReference: e.target.checked })} />
                照第一张图的姿势出
              </label>
            ) : (
              <label className="flex items-center gap-1.5 text-white/60" title="同一个场景一次出正面、左前、右前、深处四个机位，再切成四张。画幅固定 1536×1024">
                <input type="checkbox" checked={Boolean(data.sceneSheet)} disabled={data.isGenerating}
                  onChange={(e) => updateNodeData(id, { sceneSheet: e.target.checked })} />
                场景四宫格（同一场景四个机位）
              </label>
            )}
          </div>
        )}
      </BaseNodeWrapper>


      {/* 底部附着生成设定控制台：不跟画布缩放（NodePanelDock） */}
      <NodePanelDock visible={Boolean(selected)}>
        <div className="animate-in fade-in-0 duration-150">
          <ImageOperationPanel
            nodeId={id}
            prompt={data.prompt || ""}
            onPromptChange={(p) => updateNodeData(id, { prompt: p })}
            upstreamPrompt={upstream.combinedPrompt}
            hasUpstreamContent={hasUpstreamContent}
            hasInputImage={Boolean(data.references?.length || upstream.inputImageUrl)}
            imageMode={imageMode.mode}
            referenceCount={imageRefs.length}
            engine={data.engine || "zimage"}
            onEngineChange={(e) => updateNodeData(id, { engine: e })}
            width={data.width || DEFAULT_IMAGE_SIZE.width}
            height={data.height || DEFAULT_IMAGE_SIZE.height}
            ratio={data.ratio}
            resolution={data.resolution}
            onDimensionChange={(w, h, r, res) =>
              updateNodeData(id, { width: w, height: h, ratio: r, resolution: res })
            }
            imageCount={data.imageCount || 1}
            onImageCountChange={(count) => updateNodeData(id, { imageCount: count })}
            isGenerating={data.isGenerating || !!data.referenceLoading}
            onGenerate={handleGenerate}
          />
        </div>
      </NodePanelDock>
    </div>
  );
}
