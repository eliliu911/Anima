import { useState, useRef, useEffect, useMemo, useCallback } from "react";
import {
  ArrowUp,
  Camera,
  ChevronDown,
  Bot,
  Sparkles,
  Loader2,
  Box,
  Maximize2,
} from "../../ui/icons";
import { OperationPanelShell } from "../../ui/OperationPanelShell";
import { DimensionSelector } from "./DimensionSelector";
import { CanvasReferenceControls } from "../../ui/CanvasReferenceControls";
import { CanvasEngineSelect } from "../../ui/CanvasEngineSelect";
import { useCanvasPolish } from "../../utils/useCanvasPolish";
import { useCanvasStore } from "../../store/useCanvasStore";
import type { CanvasReference } from "../../types";
import {
  PromptMentionEditor,
  type PromptMentionEditorHandle,
  type MentionCandidate,
} from "../../ui/PromptMentionEditor";
import { useReferenceMentionSync } from "../../utils/referenceMentions";


interface ImageOperationPanelProps {
  nodeId: string;
  prompt: string;
  onPromptChange: (prompt: string) => void;
  upstreamPrompt?: string;
  hasUpstreamContent?: boolean;
  hasInputImage: boolean;
  /** 推导出来的模式（见 `inferImageMode`）。决定模型下拉里列哪些 */
  imageMode?: import("../../utils/imageMode").ImageMode;
  /** 连了几张图 */
  referenceCount?: number;
  engine: string;
  onEngineChange: (engine: string) => void;
  width: number;
  height: number;
  ratio?: string;
  resolution?: string;
  onDimensionChange: (width: number, height: number, ratio: string, resolution: string) => void;
  imageCount?: number;
  onImageCountChange?: (count: number) => void;
  isGenerating?: boolean;
  onGenerate: () => void;
}



const PALETTE_PRESETS = [
  { label: "电影冷调", tag: "cinematic cool blue color grade, moody teal and cyan tones" },
  { label: "暖阳金光", tag: "golden hour sunlight, warm amber glow, soft rim light" },
  { label: "赛博霓虹", tag: "neon magenta and cyan dual-tone lighting, high contrast" },
  { label: "黑白胶片", tag: "monochrome black and white photography, rich contrast, film noir" },
  { label: "自然日光", tag: "soft daylight, natural diffusion, clean clean white balance" },
];

const CAMERA_PRESETS = [
  { label: "特写镜头", tag: "close-up portrait shot, shallow depth of field, bokeh" },
  { label: "中景叙事", tag: "medium shot, cinematic composition, environmental context" },
  { label: "全景远景", tag: "wide-angle panoramic view, expansive background, epic scale" },
  { label: "仰拍视角", tag: "low-angle dramatic perspective, heroic and towering look" },
  { label: "俯拍视角", tag: "high-angle birds eye view, overhead architectural shot" },
  { label: "50mm 标准", tag: "50mm prime lens photography, natural human eye perspective" },
  { label: "24mm 广角", tag: "24mm ultra-wide lens, dynamic perspective, expansive field" },
];

export function ImageOperationPanel({
  nodeId,
  prompt,
  onPromptChange,
  upstreamPrompt,
  hasUpstreamContent,
  engine,
  hasInputImage,
  imageMode,
  referenceCount = 0,
  onEngineChange,
  width,
  height,
  ratio,
  resolution,
  onDimensionChange,
  imageCount = 1,
  onImageCountChange,
  isGenerating,
  onGenerate,
}: ImageOperationPanelProps) {
  const [activeMenu, setActiveMenu] = useState<"style" | "palette" | "camera" | "count" | null>(null);
  const containerRef = useRef<HTMLDivElement | null>(null);

  const [localPrompt, setLocalPrompt] = useState(prompt);
  const promptEditorRef = useRef<PromptMentionEditorHandle>(null);

  const node = useCanvasStore((s) => s.nodes.find((n) => n.id === nodeId));
  const references = useMemo(() => {
    return (node?.data?.references as CanvasReference[] | undefined) || [];
  }, [node?.data?.references]);

  const mentionCandidates = useMemo<MentionCandidate[]>(() => {
    return references.map((ref, idx) => {
      const cleanName = ref.name?.trim();
      const hasCustomName = cleanName && cleanName !== "画布图片" && cleanName !== "参考图";
      const name = hasCustomName ? cleanName : `图片${idx + 1}`;
      return {
        key: ref.path || ref.sourceNodeId || String(idx),
        name,
        imageUrl: ref.path,
        index: idx + 1,
        displayName: hasCustomName ? `图片${idx + 1}` : undefined,
      };
    });
  }, [references]);

  const referenceIds = useMemo(() => {
    return references.map((r, i) => r.sourceNodeId || r.path || String(i));
  }, [references]);

  const referenceNames = useMemo(() => {
    return references.map((r) => r.name?.trim() || "");
  }, [references]);

  useReferenceMentionSync(
    localPrompt,
    useMemo(
      () => [{ prefix: "图片", ids: referenceIds, names: referenceNames }],
      [referenceIds, referenceNames],
    ),
    useCallback(
      (next: string) => {
        setLocalPrompt(next);
        onPromptChange(next);
      },
      [onPromptChange],
    ),
  );

  const handleMentionReference = useCallback(
    (idx: number, ref: CanvasReference) => {
      const cleanName = ref.name?.trim();
      const hasCustomName = cleanName && cleanName !== "画布图片" && cleanName !== "参考图";
      const name = hasCustomName ? cleanName : `图片${idx}`;
      const candidate = mentionCandidates.find((c) => c.index === idx) || {
        key: ref.path,
        name,
        imageUrl: ref.path,
        index: idx,
        displayName: hasCustomName ? `图片${idx}` : undefined,
      };
      promptEditorRef.current?.insertMentionAtCursor(candidate);
    },
    [mentionCandidates],
  );

  useEffect(() => {
    setLocalPrompt(prompt);
  }, [prompt]);

  const canGenerate = !isGenerating && Boolean(localPrompt.trim() || prompt.trim() || hasUpstreamContent);

  const handleBlur = () => {
    onPromptChange(localPrompt);
  };

  // 点击外部关闭弹层
  useEffect(() => {
    if (!activeMenu) return;
    const handlePointerDown = (e: PointerEvent) => {
      if (containerRef.current?.contains(e.target as Node)) return;
      setActiveMenu(null);
    };
    window.addEventListener("pointerdown", handlePointerDown);
    return () => window.removeEventListener("pointerdown", handlePointerDown);
  }, [activeMenu]);

  const applyTag = (tag: string) => {
    if (localPrompt.includes(tag)) {
      setActiveMenu(null);
      return;
    }
    const newPrompt = localPrompt.trim() ? `${localPrompt}, ${tag}` : tag;
    setLocalPrompt(newPrompt);
    onPromptChange(newPrompt);
    setActiveMenu(null);
  };

  // 参考图名单跟着一起发（`{ASSETS}`）—— **和 @ 提及用同一份**，
  // 否则用户看到的候选名和模型拿到的名单会不一样。
  const polishRefs = useMemo(() => mentionCandidates.map((c) => c.name), [mentionCandidates]);
  const { isPolishing, polishMessage, handlePolish } = useCanvasPolish(
    nodeId,
    localPrompt,
    engine,
    "storyboard",
    (next) => {
      setLocalPrompt(next);
      onPromptChange(next);
    },
    polishRefs,
  );



  return (
    <OperationPanelShell>
      {({ toggleExpand }) => (
        <div ref={containerRef} className="relative flex flex-col gap-2.5">
          <CanvasReferenceControls
            nodeId={nodeId}
            onMentionReference={handleMentionReference}
            rightAction={
              <button
                type="button"
                onClick={toggleExpand}
                className="h-7 w-7 flex items-center justify-center rounded-md text-white/40 hover:text-white hover:bg-white/10 transition-colors shrink-0"
                title="最大化输入面板"
              >
                <Maximize2 className="h-3.5 w-3.5" />
              </button>
            }
          >
            {/* 「风格词」按钮去掉了（用户 2026-09-25）：提示词风格并进了「风格」窗口的「提示词风格」一类，那边有带示例图的 20 个画风预设 */}
            {/* 上下文调色盘 (胶囊浅底色) */}
            <div className="relative">
              <button
                type="button"
                onClick={() => setActiveMenu(activeMenu === "palette" ? null : "palette")}
                className="h-7 flex items-center gap-1.5 rounded-lg bg-white/10 px-2.5 text-white/90 hover:bg-white/15 transition-colors shrink-0 whitespace-nowrap"
              >
                <Sparkles className="h-3.5 w-3.5 text-white/70" />
                <span>上下文调色盘</span>
                <ChevronDown className="h-3 w-3 opacity-60" />
              </button>

              {activeMenu === "palette" && (
                <div className="absolute top-full left-0 mt-1.5 z-50 w-48 rounded-xl border border-white/10 bg-[#1c1c1e] p-1.5 shadow-2xl backdrop-blur-2xl flex flex-col gap-0.5 animate-in fade-in-0 zoom-in-95">
                  <div className="px-2 py-1 text-[10px] font-semibold text-white/40">光影色彩</div>
                  {PALETTE_PRESETS.map((p) => (
                    <button
                      key={p.label}
                      type="button"
                      onClick={() => applyTag(p.tag)}
                      className="flex w-full items-center justify-between rounded-lg px-2.5 py-1.5 text-left text-xs text-white/80 hover:bg-white/10 hover:text-white transition-colors"
                    >
                      <span>{p.label}</span>
                    </button>
                  ))}
                </div>
              )}
            </div>
          </CanvasReferenceControls>
          {polishMessage && <p role="status" className="text-xs text-amber-300 break-words">{polishMessage}</p>}

          {/* 提示词输入区域: 与卡片底色完全一致、零色差、无内框、无嵌套边框 */}
          <div className="relative py-1 flex flex-col gap-1.5">
            {/* 上游继承内容在文本区内的合理提示 */}
            {upstreamPrompt?.trim() && (
              <div className="flex items-center gap-1.5 text-xs text-white/50 select-text">
                <span className="text-white/70 font-medium">↳ 继承上游描述：</span>
                <span className="text-white/90 truncate max-w-[480px]">"{upstreamPrompt.trim()}"</span>
              </div>
            )}

            <PromptMentionEditor
              ref={promptEditorRef}
              value={localPrompt}
              onChange={(next) => {
                setLocalPrompt(next);
                onPromptChange(next);
              }}
              onBlur={handleBlur}
              candidates={mentionCandidates}
              placeholder={
                upstreamPrompt?.trim()
                  ? "可在此输入追加画面描述，或直接点击 ↑ 开始生成..."
                  : "描述你想要生成的画面内容，支持键入 @ 引用素材与角色..."
              }
              rows={upstreamPrompt?.trim() ? 3 : 4}
              className="w-full bg-transparent border-0 outline-none text-white text-sm focus:outline-none p-0 leading-relaxed"
            />
          </div>



          {/* 底栏参数控制行: 单行不折行，组件无硬边框底色，严格统一 h-7 高度 */}
          <div className="flex items-center justify-between gap-1.5 pt-1 text-xs text-white/70">
            <div className="flex items-center gap-2 flex-nowrap min-w-0">
              {/* 模型选择 (无底色无边框，配立方体图标) */}
              <div className="relative flex items-center">
                <Box className="pointer-events-none h-3.5 w-3.5 text-white/50 mr-1" />
                <CanvasEngineSelect
                  mode={imageMode ?? (hasInputImage ? "i2i" : "t2i")}
                  alsoMode={referenceCount === 1 ? (imageMode === "ref2i" ? "i2i" : "ref2i") : undefined}
                  value={engine} onChange={onEngineChange} disabled={isGenerating} />
              </div>

              {/* 画幅尺寸与分辨率选择 (支持 16:9/9:16/1:1/4:3/3:4/21:9 及 1K/2K/4K) */}
              <DimensionSelector
                engine={engine}
                mode="image"
                width={width}
                height={height}
                ratio={ratio}
                resolution={resolution}
                onChange={onDimensionChange}
              />

              {/* 摄像机参数菜单 (无底色无边框) */}
              <div className="relative">
                <button
                  type="button"
                  onClick={() => setActiveMenu(activeMenu === "camera" ? null : "camera")}
                  className={`h-7 flex items-center gap-1 rounded-md px-1.5 transition-colors ${
                    activeMenu === "camera"
                      ? "bg-white/15 text-white"
                      : "text-white/80 hover:bg-white/10 hover:text-white"
                  }`}
                  title="摄像机视点与焦段"
                >
                  <Camera className="h-3.5 w-3.5 text-white/60" />
                  <span>摄像机</span>
                </button>

                {activeMenu === "camera" && (
                  <div className="absolute bottom-full left-0 mb-1.5 z-50 w-52 rounded-xl border border-white/10 bg-[#1c1c1e] p-1.5 shadow-2xl backdrop-blur-2xl flex flex-col gap-0.5 animate-in fade-in-0 zoom-in-95">
                    <div className="px-2 py-1 text-[10px] font-semibold text-white/40">镜头与景别</div>
                    {CAMERA_PRESETS.map((c) => (
                      <button
                        key={c.label}
                        type="button"
                        onClick={() => applyTag(c.tag)}
                        className="flex w-full items-center justify-between rounded-lg px-2.5 py-1.5 text-left text-xs text-white/80 hover:bg-white/10 hover:text-white transition-colors"
                      >
                        <span>{c.label}</span>
                      </button>
                    ))}
                  </div>
                )}
              </div>

              {/* 生成张数 (无底色无边框) */}
              <div className="relative">
                <button
                  type="button"
                  onClick={() => setActiveMenu(activeMenu === "count" ? null : "count")}
                  className="h-7 flex items-center gap-1 rounded-md px-1.5 text-white/80 hover:bg-white/10 hover:text-white transition-colors"
                  title="生成张数"
                >
                  <span>{imageCount}张</span>
                  <ChevronDown className="h-3 w-3 opacity-50" />
                </button>

                {activeMenu === "count" && (
                  <div className="absolute bottom-full left-0 mb-1.5 z-50 w-24 rounded-xl border border-white/10 bg-[#1c1c1e] p-1 shadow-2xl backdrop-blur-2xl flex flex-col gap-0.5">
                    {[1, 2, 4].map((count) => (
                      <button
                        key={count}
                        type="button"
                        onClick={() => {
                          onImageCountChange?.(count);
                          setActiveMenu(null);
                        }}
                        className={`rounded-lg px-2.5 py-1 text-center text-xs transition-colors ${
                          imageCount === count
                            ? "bg-white/20 text-white font-medium shadow-sm"
                            : "text-white/80 hover:bg-white/10 hover:text-white"
                        }`}
                      >
                        {count} 张
                      </button>
                    ))}
                  </div>
                )}
              </div>

              {/* 润色按钮 文/A (接入 Anima AI 润色接口) */}
              <button
                type="button"
                disabled={isPolishing}
                onClick={handlePolish}
                className="h-7 w-7 flex items-center justify-center rounded-md text-white/60 hover:text-white hover:bg-white/10 transition-colors shrink-0 disabled:opacity-50"
                title={isPolishing ? "AI 润色中..." : "AI 智能提示词润色"}
              >
                {isPolishing ? (
                  <Loader2 className="h-3.5 w-3.5 animate-spin text-white" />
                ) : (
                  <Bot className="h-3.5 w-3.5" />
                )}
              </button>
            </div>

            {/* 提交生成执行按钮 (本节点有词或上游有内容时高亮可用) */}
            <button
              type="button"
              disabled={!canGenerate}
              onClick={onGenerate}
              className={`h-7 w-7 flex items-center justify-center rounded-full transition-all shrink-0 ${
                canGenerate
                  ? "bg-white text-black shadow-lg shadow-white/10 hover:scale-105 hover:bg-zinc-200 cursor-pointer"
                  : "bg-white/10 text-white/30 cursor-not-allowed"
              }`}
              title={
                isGenerating
                  ? "生成中..."
                  : canGenerate
                  ? upstreamPrompt?.trim()
                    ? `执行生成 (上游: ${upstreamPrompt.trim().slice(0, 30)}...)`
                    : "执行生成 (DAG)"
                  : "请输入提示词或连接上游内容"
              }
            >
              {isGenerating ? (
                <Loader2 className="h-3.5 w-3.5 animate-spin text-black" />
              ) : (
                <ArrowUp className="h-3.5 w-3.5 stroke-[2.5]" />
              )}
            </button>
          </div>
        </div>
      )}
    </OperationPanelShell>
  );
}
