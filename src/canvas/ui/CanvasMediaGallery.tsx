import React, { useState } from "react";
import { MediaThumb } from "../../components/studio/MediaThumb";
import { ChevronDown, Download, Loader2, ArrowUpRight } from "./icons";
import { downloadMedia } from "../utils/downloadMedia";
import { CanvasImageLightbox } from "./CanvasImageLightbox";

export interface CanvasMediaGalleryProps {
  nodeId?: string;
  kind: "image" | "video";
  items: string[];
  primaryItem?: string;
  targetCount?: number;
  isGenerating?: boolean;
  width?: number;
  height?: number;
  isExpanded: boolean;
  onToggleExpand: () => void;
  onSelectPrimary: (path: string) => void;
  onApplyToCanvas?: (path: string) => void;
  placeholder?: React.ReactNode;
  /** 这个节点正在跑的画布任务（`data.jobId`）：进度按它显示，不读全局日志 */
  jobId?: string;
}

import { useComfyProgress } from "./useComfyProgress";
import { useCanvasJobProgress } from "./useCanvasJobProgress";

export function CanvasMediaGallery({
  kind,
  items,
  primaryItem,
  targetCount = 1,
  isGenerating = false,
  isExpanded,
  onToggleExpand,
  onSelectPrimary,
  onApplyToCanvas,
  placeholder,
  jobId,
}: CanvasMediaGalleryProps) {
  const [hoveredIdx, setHoveredIdx] = useState<number | null>(null);
  const [lightboxMedia, setLightboxMedia] = useState<string | null>(null);

  const displayPrimary = primaryItem || items[0];

  // 渲染序号对齐：当进行到第 N 张时展示 N/total，如 1/2, 2/2，绝不显示 0/2
  // ComfyUI 跑到哪一步了。**只在生成时轮询**，见 `useComfyProgress`
  const globalComfy = useComfyProgress(isGenerating);
  // **优先用这个节点自己任务的进度**（排队中 / 第几步 / 预计几点完成）。没有时才退回全局的 ComfyUI 日志 ——
  // 全局日志只认正在跑的那一个，排队的节点读到的也是它的步数（2026-10-11 用户截图）
  const own = useCanvasJobProgress(isGenerating ? jobId : undefined);
  const comfy = own ? { text: own, step: own.includes("排队") ? null : 0, total: null, eta_secs: null } : globalComfy;
  const currentRenderingStep = Math.min(targetCount, items.length + 1);
  const totalDisplayCount = Math.max(items.length, targetCount);

  // 画册展示槽位（含已出图与渲染中槽位）
  const effectiveCount = Math.max(items.length, isGenerating ? targetCount : 0);
  const slots = Array.from({ length: effectiveCount }, (_, i) => items[i] || null);

  // 1. 空状态且未生成
  if (items.length === 0 && !isGenerating) {
    return (
      <div className="relative w-full h-[248px] rounded-2xl bg-[#1e1e20] overflow-hidden flex flex-col items-center justify-center transition-all duration-200">
        {placeholder}
      </div>
    );
  }

  // 2. 堆叠模式 (isExpanded === false)
  if (!isExpanded) {
    const hasMultiple = items.length > 1 || (isGenerating && targetCount > 1);

    return (
      <>
        <div className="relative w-full">
          {/* 堆叠底层卡片立体阴影外沿 */}
          {hasMultiple && (
            <>
              <div
                className="absolute top-1.5 -right-2 bottom-1.5 w-6 rounded-r-2xl bg-[#1c1c1e] border-r border-y border-white/10 -z-10 shadow-lg pointer-events-none transition-all"
              />
              {totalDisplayCount > 2 && (
                <div
                  className="absolute top-3 -right-3.5 bottom-3 w-4 rounded-r-2xl bg-[#141415] border-r border-y border-white/5 -z-20 shadow-md pointer-events-none transition-all"
                />
              )}
            </>
          )}

          {/* 主视口卡片：卡片默认尺寸固定（h-[248px]），图片通过 object-contain 自适应显示而不撑大卡片 */}
          <div
            onDoubleClick={(e) => {
              e.stopPropagation();
              if (displayPrimary) {
                setLightboxMedia(displayPrimary);
              }
            }}
            className="relative w-full h-[248px] rounded-2xl bg-[#1e1e20] overflow-hidden flex flex-col items-center justify-center transition-all duration-200 group select-none cursor-pointer"
            title={
              displayPrimary
                ? kind === "video"
                  ? "点击播放视频，双击全屏查看"
                  : "双击全屏查看，点击右上角展开画册"
                : undefined
            }
          >
            {displayPrimary ? (
              <MediaThumb
                path={displayPrimary}
                alt={kind === "image" ? "生成的图像" : "生成的视频"}
                kind={kind}
                className={
                  kind === "video"
                    ? "w-full h-full object-contain pointer-events-auto nodrag"
                    : "w-full h-full object-contain select-none pointer-events-none"
                }
              />
            ) : (
              <div className="w-full h-full flex flex-col items-center justify-center bg-black/40">
                <Loader2 className="h-6 w-6 text-white animate-spin" />
                <span className="text-xs text-white/80 mt-2 font-medium tracking-wide">
                  {kind === "image" ? "ComfyUI 渲染中" : "ComfyUI 视频渲染中"} ({currentRenderingStep}/{targetCount})...
                </span>
                {/* **进度是另起一行，不替换上面那句。** 上面那句说的是「第几张」，
                    这一句说的是「这一张跑到哪了」—— 两件事。
                    拿不到进度时这一行不出现，界面退回原样。 */}
                {comfy && (
                  <span className="text-[11px] text-white/55 mt-1 tabular-nums">{comfy.text}</span>
                )}
              </div>
            )}

            {/* 左上角轻量渲染中徽章：完成一张后图片可见可点，不影响后续渲染 */}
            {isGenerating && displayPrimary && (
              <div className="absolute top-3 left-3 z-20 flex items-center gap-1.5 rounded-full bg-black/80 backdrop-blur-md px-2.5 py-1 text-xs font-medium text-white/90 border border-white/15 shadow-xl pointer-events-none">
                <Loader2 className="h-3.5 w-3.5 text-blue-400 animate-spin" />
                <span>渲染中 ({currentRenderingStep}/{targetCount})</span>
                {comfy?.step != null && <span className="text-white/50">· {comfy.text}</span>}
              </div>
            )}

            {/* 右上角堆叠展开胶囊 (显示图片数量与展开箭头) */}
            {hasMultiple && (
              <button
                type="button"
                onClick={(e) => {
                  e.stopPropagation();
                  onToggleExpand();
                }}
                className="absolute top-3 right-3 z-20 flex items-center gap-1 rounded-full bg-black/65 hover:bg-black/85 backdrop-blur-md px-2.5 py-1 text-xs font-medium text-white/90 border border-white/15 shadow-xl transition-all nodrag cursor-pointer hover:scale-105 active:scale-95"
                title="点击展开画册"
              >
                <span>{totalDisplayCount}</span>
                <ChevronDown className="h-3.5 w-3.5 opacity-80" />
              </button>
            )}

            {/* 悬浮快速下载图标 (右上角) */}
            {displayPrimary && (
              <button
                type="button"
                onClick={(e) => {
                  e.stopPropagation();
                  downloadMedia(displayPrimary);
                }}
                className={`absolute z-20 flex items-center justify-center rounded-full bg-black/65 hover:bg-black/90 backdrop-blur-md p-1.5 text-white/80 hover:text-white border border-white/15 shadow-lg transition-all nodrag cursor-pointer hover:scale-110 active:scale-95 opacity-0 group-hover:opacity-100 ${
                  hasMultiple ? "top-3 right-16" : "top-3 right-3"
                }`}
                title="下载到本地"
              >
                <Download className="h-3.5 w-3.5" />
              </button>
            )}
          </div>
        </div>

        {/* 双击全屏大图预览 */}
        <CanvasImageLightbox
          src={lightboxMedia}
          kind={kind}
          onClose={() => setLightboxMedia(null)}
        />
      </>
    );
  }

  // 3. 展开画册模式 (isExpanded === true)
  return (
    <>
      <div className="relative w-full rounded-2xl bg-[#18181b] border border-white/15 p-2 shadow-2xl backdrop-blur-2xl flex flex-col gap-2">
        {/* 平铺卡片行：固定卡片高度，图片使用 object-contain 自适应显示 */}
        <div className="grid grid-flow-col auto-cols-fr gap-2.5 w-full items-stretch">
          {slots.map((itemPath, idx) => {
            const isSlotPrimary = Boolean(itemPath && itemPath === displayPrimary);
            const isGeneratingSlot = !itemPath;

            return (
              <div
                key={itemPath || `slot-${idx}`}
                onMouseEnter={() => setHoveredIdx(idx)}
                onMouseLeave={() => setHoveredIdx(null)}
                onClick={(e) => {
                  e.stopPropagation();
                  if (itemPath && !isSlotPrimary) {
                    onSelectPrimary(itemPath);
                  }
                }}
                onDoubleClick={(e) => {
                  e.stopPropagation();
                  if (itemPath) {
                    setLightboxMedia(itemPath);
                  }
                }}
                className={`group/card relative h-[210px] rounded-xl overflow-hidden bg-[#242426] border flex flex-col items-center justify-center transition-all select-none ${
                  isSlotPrimary
                    ? "border-blue-500/80 ring-2 ring-blue-500/30 shadow-lg cursor-default"
                    : isGeneratingSlot
                    ? "border-white/10 opacity-80 cursor-wait"
                    : "border-white/15 hover:border-white/40 hover:shadow-xl cursor-pointer"
                }`}
                title={itemPath ? "双击全屏查看，单击设为主图" : undefined}
              >
                {/* 卡片媒体内容 */}
                {itemPath ? (
                  <MediaThumb
                    path={itemPath}
                    alt={`${kind === "image" ? "画册图片" : "画册视频"} ${idx + 1}`}
                    kind={kind}
                    className={
                      kind === "video"
                        ? "w-full h-full object-contain pointer-events-auto nodrag"
                        : "w-full h-full object-contain pointer-events-none select-none"
                    }
                  />
                ) : (
                  <div className="flex flex-col items-center justify-center gap-2 w-full h-full bg-black/40">
                    <Loader2 className="h-5 w-5 text-white/70 animate-spin" />
                    <span className="text-xs text-white/60 font-medium">
                      渲染中 ({currentRenderingStep}/{targetCount})...
                    </span>
                  </div>
                )}

                {/* 主图标签 (左下角) */}
                {isSlotPrimary && (
                  <div className="absolute bottom-2 left-2 z-10 flex items-center gap-1 rounded bg-black/70 backdrop-blur-md px-2 py-0.5 text-[10px] font-medium text-white/90 border border-white/15 shadow-sm">
                    <span>主图</span>
                  </div>
                )}

                {/* 悬浮操作层 */}
                {itemPath && hoveredIdx === idx && (
                  <div className="absolute inset-0 z-20 flex flex-col justify-between p-2 bg-gradient-to-b from-black/50 via-transparent to-black/60 pointer-events-auto animate-in fade-in-0 duration-100">
                    {/* 顶部工具栏: 左侧应用到画布，右侧下载 */}
                    <div className="flex items-center justify-between w-full">
                      {onApplyToCanvas ? (
                        <button
                          type="button"
                          onClick={(e) => {
                            e.stopPropagation();
                            onApplyToCanvas(itemPath);
                          }}
                          className="flex items-center gap-1 rounded-full bg-black/65 hover:bg-black/90 backdrop-blur-md px-2.5 py-1 text-[11px] font-medium text-white border border-white/20 shadow-md transition-all nodrag cursor-pointer hover:scale-105 active:scale-95"
                          title="复制为画布上的独立节点"
                        >
                          <ArrowUpRight className="h-3 w-3" />
                          <span>应用到画布</span>
                        </button>
                      ) : <div />}

                      <button
                        type="button"
                        onClick={(e) => {
                          e.stopPropagation();
                          downloadMedia(itemPath);
                        }}
                        className="flex items-center justify-center rounded-full bg-black/65 hover:bg-black/90 backdrop-blur-md p-1.5 text-white border border-white/20 shadow-md transition-all nodrag cursor-pointer hover:scale-110 active:scale-95"
                        title="下载到本地"
                      >
                        <Download className="h-3.5 w-3.5" />
                      </button>
                    </div>

                    {/* 非主图时悬浮提示「点击设为主图」 */}
                    {!isSlotPrimary && (
                      <div className="self-center mb-1 flex items-center justify-center rounded-full bg-blue-600/90 backdrop-blur-md px-3 py-1 text-[11px] font-medium text-white shadow-lg border border-white/20 animate-in zoom-in-95 pointer-events-none">
                        点击设为主图
                      </div>
                    )}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      </div>

      {/* 双击全屏大图预览 */}
      <CanvasImageLightbox
        src={lightboxMedia}
        kind={kind}
        onClose={() => setLightboxMedia(null)}
      />
    </>
  );
}
