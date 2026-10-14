import { useState, useRef, useEffect, useMemo, useCallback } from "react";
import {
  ArrowUp,
  Film,
  Bot,
  Loader2,
  Maximize2,
} from "../../ui/icons";
import { OperationPanelShell } from "../../ui/OperationPanelShell";
import { inferVideoMode } from "../../utils/videoMode";
import {
  WEETODD_RATIO_VIDEO_SIZES,
  inferVideoRatio,
} from "../../ui/CanvasVideoResolutionSelect";
import { CanvasVideoSettingsPopover } from "../../ui/CanvasVideoSettingsPopover";
import { CanvasReferenceControls } from "../../ui/CanvasReferenceControls";
import { CanvasEngineSelect } from "../../ui/CanvasEngineSelect";
import { VideoModeMenu } from "../../ui/VideoModeMenu";
import { effectiveQuality, type CanvasH3Quality } from "../../utils/h3Quality";
import type { ModeInference } from "../../utils/videoMode";
import { upstreamCards } from "../../ui/UpstreamCards";
import { upstreamVideoNodes } from "../../utils/depthChain";
import { useCanvasPolish } from "../../utils/useCanvasPolish";
import { useCanvasStore } from "../../store/useCanvasStore";
import { selectContentNodes } from "../../store/selectContentNodes";
import type { CanvasReference, VideoGenNodeData } from "../../types";
import { ComposingTextarea } from "../../../components/ComposingInput";
import {
  PromptMentionEditor,
  type PromptMentionEditorHandle,
  type MentionCandidate,
} from "../../ui/PromptMentionEditor";
import { useReferenceMentionSync } from "../../utils/referenceMentions";
import type { PolishKind } from "../../../api/polish";
import { polishKindForMode } from "../../utils/polishKind";
import { isLongTake, openLongTake, planLongTake, windowLines } from "../../utils/longTakeWindows";
import { auditVideoNode, ignoredInputs } from "../../utils/runAudit";
import { studioLoras } from "../../../api/studio";

interface VideoOperationPanelProps {
  nodeId: string;
  mode: "auto" | "t2v" | "i2v" | "fl2v" | "r2v";
  /** `omni` = 全能参考（原片复刻：图 + 一段参考视频） */
  onModeChange: (mode: "auto" | "t2v" | "i2v" | "fl2v" | "r2v" | "omni") => void;
  prompt: string;
  onPromptChange: (prompt: string) => void;
  upstreamPrompt?: string;
  hasUpstreamContent?: boolean;
  hasUpstreamImage?: boolean;
  upstreamImageGenerating?: boolean;
  engine: string;
  onEngineChange: (engine: string) => void;
  width?: number;
  height?: number;
  ratio?: string;
  resolution?: string;
  onDimensionChange?: (width: number, height: number, ratio: string, resolution: string) => void;
  durationSecs: number;
  onDurationChange: (duration: number) => void;
  videoCount?: number;
  onVideoCountChange?: (count: number) => void;
  generateAudio?: boolean;
  onGenerateAudioChange?: (generateAudio: boolean) => void;
  h3OfficialFormat?: boolean;
  onH3OfficialFormatChange?: (on: boolean) => void;
  /** 节点存的加速档（空 = 按链路默认）。见 `utils/h3Quality` */
  h3Quality?: string;
  onH3QualityChange?: (q: CanvasH3Quality) => void;
  h3Sampling?: string;
  onH3SamplingChange?: (s: string) => void;
  isGenerating?: boolean;
  onGenerate: () => void;
}


import { CAMERA_MOVES, cameraMovesIn, cameraTag, type MotionPresetItem } from "../../utils/cameraMoves";
import { CameraMoveDialog } from "../../ui/CameraMoveDialog";

/** 运镜在 @ 候选里的样子：`@运镜·跟拍`，没有编号（index 0，下拉里不显示 #） */
export function cameraCandidate(m: MotionPresetItem): MentionCandidate {
  return { key: `camera:${m.id}`, name: cameraTag(m), index: 0, displayName: m.desc };
}
export { MOTION_CATEGORIES, type MotionCategory, type MotionPresetItem } from "../../utils/cameraMoves";

/**
 * 输入框的占位文字 = 这个模式下该写什么（照 LibTV：说明留在输入框里，不另占一行、不用黄字）。
 */
export function promptPlaceholder(inference: ModeInference | undefined, hasUpstreamPrompt: boolean): string {
  if (hasUpstreamPrompt) return "追加运镜或动作描述，或直接点 ↑ 生成，@引用素材";
  switch (inference?.mode) {
    case "motion":
      return inference.via === "video"
        ? "描述人物和场景（可不写），照参考视频的动作和镜头出片，@引用素材"
        : "描述人物和场景（可不写），动作照深度视频，@引用素材";
    case "i2v": return "描述首帧之后画面怎么动，@引用素材";
    case "fl2v": return "描述从首帧到尾帧怎么过渡，@引用素材";
    case "r2v": return "描述你想要生成的画面内容，@引用素材";
    default: return "描述你想要生成的画面内容";
  }
}

export function VideoOperationPanel({
  nodeId,
  onModeChange,
  prompt,
  onPromptChange,
  upstreamPrompt,
  hasUpstreamContent,
  hasUpstreamImage,
  upstreamImageGenerating,
  engine,
  onEngineChange,
  width = 1280,
  height = 720,
  ratio,
  resolution,
  onDimensionChange,
  durationSecs,
  onDurationChange,
  videoCount = 1,
  onVideoCountChange,
  generateAudio = false,
  onGenerateAudioChange,
  h3OfficialFormat = false,
  onH3OfficialFormatChange,
  h3Quality,
  onH3QualityChange,
  h3Sampling,
  onH3SamplingChange,
  isGenerating,
  onGenerate,
}: VideoOperationPanelProps) {
  const [cameraOpen, setCameraOpen] = useState(false);

  const [localPrompt, setLocalPrompt] = useState(prompt);
  const promptEditorRef = useRef<PromptMentionEditorHandle>(null);

  const node = useCanvasStore((s) => s.nodes.find((n) => n.id === nodeId));
  // 连线一变模式就跟着变 —— 所以要订阅 nodes/edges，不能只读一次
  const graphNodes = useCanvasStore(selectContentNodes);
  const graphEdges = useCanvasStore((s) => s.edges);
  const inference = useMemo(
    () => inferVideoMode(nodeId, graphNodes, graphEdges),
    [nodeId, graphNodes, graphEdges],
  );

  // **@ 的候选 = 上面那排卡片里的图**（照 LibTV：连进来的图片节点在提示词里直接 @ 得到）。
  // 以前只取节点上直接绑的 references —— 连线接进来的图片节点（模板、上传的设定卡）@ 不到，
  // 只能手写 <Picture N>。卡片编号和发给模型的 <Picture N> 是同一个顺序（见 UpstreamCards）。
  // 上游直连了视频（不含补帧 / 超分这种工具节点自己）：全能参考要它
  const videoUpstream = useMemo(
    () => !(node?.data as VideoGenNodeData | undefined)?.videoTool && upstreamVideoNodes(nodeId, graphNodes, graphEdges).length > 0,
    [node?.data, nodeId, graphNodes, graphEdges],
  );
  const imageCards = useMemo(
    () => upstreamCards(nodeId, graphNodes, graphEdges).filter((c) => c.kind === "image"),
    [nodeId, graphNodes, graphEdges],
  );
  const mentionCandidates = useMemo<MentionCandidate[]>(() => {
    return imageCards.map((c) => {
      const cleanName = c.reference?.name?.trim();
      const hasCustomName = Boolean(cleanName && cleanName !== "画布图片" && cleanName !== "参考图");
      return {
        key: c.sourceNodeId || c.reference?.path || c.key,
        name: hasCustomName ? cleanName! : `图片${c.index}`,
        imageUrl: c.path,
        index: c.index,
        displayName: hasCustomName ? `图片${c.index}` : c.label,
      };
    });
  }, [imageCards]);

  // @ 的候选里也列运镜（在图片后面）：敲 @ 能直接挑，插进去的是同一种标签
  const editorCandidates = useMemo(() => [...mentionCandidates, ...CAMERA_MOVES.map(cameraCandidate)], [mentionCandidates]);

  const referenceIds = useMemo(() => mentionCandidates.map((c) => c.key), [mentionCandidates]);

  const referenceNames = useMemo(() => {
    return imageCards.map((c) => c.reference?.name?.trim() || "");
  }, [imageCards]);

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



  // **按实际生成模式选润色槽位，而且用的必须是推导结果。**
  //
  // `video` 那份提示词的前提是「首帧已经定了画面，不要重复静态外观」——
  // 而 t2v 根本没有首帧，r2v 是用参考图定身份、不是用首帧定构图。
  //
  // 判据只能是 `inference.mode`：它和真正发出去的那一次生成读的是同一个
  // 推导函数。第一版在这里自己按「有没有上游图」又推了一次 ——
  // 于是连了三张身份参考的 `auto` 节点被判成 i2v，套上「首帧已定」那份
  // 提示词（2026-09-19 Codex Astra 的第三份审查抓到）。理由见 `polishKindForMode`。
  // **长镜头：一个节点，里面一张窗口表。** 见 `utils/longTakeWindows` 的模块注释 ——
  // 上一版是「视频节点连视频节点」，行为对但表达是四个假出片节点。
  const nodeData = node?.data as VideoGenNodeData | undefined;
  const windows: string[] = useMemo(() => nodeData?.longTakeWindows ?? [], [nodeData?.longTakeWindows]);
  const chain = useMemo(() => planLongTake(nodeData), [nodeData]);
  const longTake = isLongTake(nodeData);
  const setWindows = (next: string[]) => useCanvasStore.getState().updateNodeData(nodeId, { longTakeWindows: next });
  /**
   * 每个窗口的台词落点。
   *
   * **长镜头一次提交，不能只重来某一窗口**（H3 的潜变量存不了盘）。
   * 「第三个窗口的台词说不完」跑完才发现的代价是整条重来 ——
   * 所以这些必须在提交之前摊开。
   */
  const chainDetail = useMemo(() => {
    if (!chain.ok) return [];
    return windows.map((prompt, i) => {
      const lines = windowLines(prompt, chain.windowSecs);
      return { i, hasPrompt: Boolean(prompt.trim()), lines, overflow: lines.filter((l) => !l.fits) };
    });
  }, [chain, windows]);
  /**
   * 出片前的预检：**这次实际会发出什么，哪些连线白连了。**
   *
   * 这个项目最贵的一类错误是「连线看起来成功、模型根本不吃这个输入」——
   * 跑完四十分钟才发现参考图没生效，而任务状态是成功。
   *
   * LoRA 的架构**问后端**（`studio_loras` 由张量键名判定），
   * 不按文件名猜 —— `models/loras/` 现在按引擎分了子目录，
   * 只看名字的话扔进 `zimage/` 的任何 SDXL LoRA 都会被判成 Z-Image。
   */
  const [loraEngines, setLoraEngines] = useState<Record<string, string[] | undefined>>({});
  useEffect(() => {
    let alive = true;
    studioLoras()
      .then((l) => { if (alive) setLoraEngines(Object.fromEntries(l.map((x) => [x.name, x.engines]))); })
      .catch(() => {});  // 取不到就不判架构。**宁可不报，也不要报错的**
    return () => { alive = false; };
  }, []);
  const audit = useMemo(
    () => auditVideoNode(nodeId, graphNodes, graphEdges, loraEngines),
    [nodeId, graphNodes, graphEdges, loraEngines],
  );
  const ignored = ignoredInputs(audit);


  const polishKind: PolishKind = polishKindForMode(inference.mode);
  // 参考图名单要跟着一起发 —— 参考生视频那份提示词的活就是把草稿里提到的
  // 资产补成 `[角色:名字]`，名字从这里来。**和 @ 提及用同一份**，
  // 否则用户看到的候选名和模型拿到的名单会不一样。
  const polishRefs = useMemo(() => mentionCandidates.map((c) => c.name), [mentionCandidates]);
  const { isPolishing, polishMessage, handlePolish } = useCanvasPolish(
    nodeId,
    localPrompt,
    engine,
    polishKind,
    (next) => {
      setLocalPrompt(next);
      onPromptChange(next);
    },
    polishRefs,
  );


  return (
    <OperationPanelShell widthClass="w-[660px]">
      {({ toggleExpand }) => (
        <div className="relative flex flex-col gap-2.5">
          <CanvasReferenceControls
            nodeId={nodeId}
            onMentionReference={handleMentionReference}
            videoMode={inference.mode}
            frames={inference.images}
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
            {/* 运镜：打开运镜窗口（照 LibTV）。选中的在提示词里是 @运镜·X 标签，可以叠几个，前后就是顺序 */}
            <button
              type="button"
              onClick={() => setCameraOpen(true)}
              className={`h-7 flex items-center gap-1 rounded-md px-2 transition-colors ${cameraMovesIn(localPrompt).length ? "bg-white/15 text-white" : "text-white/70 hover:bg-white/10 hover:text-white"}`}
            >
              <Film className="h-3.5 w-3.5 text-white/60" />
              <span>运镜{cameraMovesIn(localPrompt).length ? ` · ${cameraMovesIn(localPrompt).length}` : ""}</span>
            </button>
            {cameraOpen && (
              <CameraMoveDialog
                prompt={localPrompt}
                disabled={isGenerating}
                onPick={(m) => {
                  const c = cameraCandidate(m);
                  if (promptEditorRef.current) promptEditorRef.current.insertMentionAtCursor(c);
                  else {
                    const next = `${localPrompt.trim()} @${c.name} `.trimStart();
                    setLocalPrompt(next);
                    onPromptChange(next);
                  }
                }}
                onRemove={(m) => {
                  const next = localPrompt.split(`@${cameraTag(m)} `).join("").split(`@${cameraTag(m)}`).join("");
                  setLocalPrompt(next);
                  onPromptChange(next);
                }}
                onClose={() => setCameraOpen(false)}
              />
            )}
          </CanvasReferenceControls>
          {/* 模式说明**不再单独占一行**（用户 2026-09-25：「直接按这种，在输入框留出说明就可以，不用黄色提示词」，照 LibTV）。
              说明写在输入框的占位文字里；完整理由在模式按钮的悬停提示里；回到按连线 = 菜单里标「按连线」的那一项 */}
          {polishMessage && <p role="status" className="text-xs text-amber-300 break-words">{polishMessage}</p>}

          {/* 提示词输入区域: 与卡片底色完全一致、零色差、无内框、无嵌套边框 */}
          <div className="relative py-1 flex flex-col gap-1.5">
            {/* 上游继承内容在文本区内的合理提示 */}
            {upstreamPrompt?.trim() && (
              <div className="flex items-center gap-1.5 text-xs text-white/50 select-text">
                <span className="text-white/70 font-medium">↳ 继承上游描述：</span>
                <span className="text-white/90 truncate max-w-[480px]">
                  "{upstreamPrompt.trim()}"
                  {hasUpstreamImage ? " · 包含首帧图像" : upstreamImageGenerating ? " · 等待首帧渲染" : ""}
                </span>
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
              candidates={editorCandidates}
              placeholder={promptPlaceholder(inference, Boolean(upstreamPrompt?.trim()))}
              rows={upstreamPrompt?.trim() ? 3 : 4}
              className="w-full bg-transparent border-0 outline-none text-white text-sm focus:outline-none p-0 leading-relaxed"
            />
          </div>




          {/* 出片前预检：只说被忽略了什么和确定会出错的。生效的不用说。 */}
          {(ignored.length > 0 || audit.conflicts.length > 0) && (
            <div className="mt-2 rounded-md border border-amber-400/20 bg-amber-400/[0.06] px-2.5 py-2 space-y-1">
              {audit.conflicts.map((c, i) => (
                <div key={`c${i}`} className="text-[11px] text-rose-300/90">⚠ {c}</div>
              ))}
              {ignored.map((x, i) => (
                <div key={`i${i}`} className="text-[11px] text-amber-200/80">
                  这条{x.from}连了但没用上：{x.role}
                </div>
              ))}
            </div>
          )}

          {/* 长镜头：一个节点一张窗口表。**开关在底栏的设置里**（用户 2026-09-24），开了之后这里才出窗口编辑 + 提交前摊开 */}
          {longTake && (
          <div className="mt-2 rounded-md border border-white/10 bg-white/[0.03] px-2.5 py-2 space-y-1.5">
            <div className="flex items-center justify-between gap-2">
              <span className="text-[11px] text-white/70">
                {longTake ? "长镜头（这一个节点出一整段）" : "长镜头"}
              </span>
              <button
                type="button"
                onClick={() => setWindows(longTake ? [] : openLongTake(localPrompt || prompt))}
                className="h-6 px-2 rounded-md bg-white/10 hover:bg-white/20 text-[11px] text-white/80"
              >
                {longTake ? "取消长镜头" : "改成长镜头"}
              </button>
            </div>


            {longTake && (
              <>
                {windows.map((w, i) => (
                  <div key={i} className="space-y-0.5">
                    <div className="flex items-center justify-between">
                      <span className="text-[10px] text-white/45">
                        窗口 {i + 1} / {windows.length}
                        {i === 0 ? " · 挂上游的参考图" : " · 挂同一组参考图，从上一窗的重叠帧接着走"}
                      </span>
                      {windows.length > 2 && (
                        <button
                          type="button"
                          onClick={() => setWindows(windows.filter((_, k) => k !== i))}
                          className="text-[10px] text-white/40 hover:text-rose-300"
                        >
                          删掉
                        </button>
                      )}
                    </div>
                    <ComposingTextarea
                      value={w}
                      onCommit={(v) => setWindows(windows.map((x, k) => (k === i ? v : x)))}
                      rows={2}
                      placeholder={`第 ${i + 1} 个窗口这 ${chain.ok ? chain.windowSecs : durationSecs} 秒里发生什么`}
                      className="w-full resize-y rounded-md border border-white/10 bg-black/30 px-2 py-1 text-[11px] text-white/85 outline-none focus:border-white/25"
                    />
                  </div>
                ))}
                <button
                  type="button"
                  onClick={() => setWindows([...windows, ""])}
                  className="h-6 px-2 rounded-md bg-white/5 hover:bg-white/15 text-[11px] text-white/70"
                >
                  加一个窗口
                </button>

                {chain.ok ? (
                  <>
                    <div className="text-[11px] text-white/70">
                      {chain.windows} 个窗口 × {chain.windowSecs}s = 约 {chain.netSecs.toFixed(1)}s 成片
                      <span className="text-white/40">（预计 {chain.etaHours < 1 ? `${Math.round(chain.etaHours * 60)} 分钟` : `${chain.etaHours.toFixed(1)} 小时`}）</span>
                    </div>
                    {/* **上下文重叠常显，不藏在告警里。** 它是每个窗口被吃掉的那一段，
                        发布时会裁掉 —— 不显示的话用户按窗口数 × 时长算成片时长必然算错。 */}
                    <div className="text-[11px] text-white/45">
                      窗口间重叠 {(chain.windowSecs * chain.overlapShare).toFixed(1)}s
                      （占每窗 {Math.round(chain.overlapShare * 100)}%，发布时裁掉）
                    </div>
                    {/* 每个窗口的台词落点。跑完才发现说不完，代价是整条重来。 */}
                    {chainDetail.some((w) => w.lines.length || !w.hasPrompt) && (
                      <div className="text-[11px] text-white/55 space-y-0.5">
                        {chainDetail.map((w) => (
                          <div key={w.i} className={w.overflow.length || !w.hasPrompt ? "text-amber-300/85" : undefined}>
                            窗口 {w.i + 1}：
                            {!w.hasPrompt
                              ? "还没写提示词"
                              : w.lines.length === 0
                              ? "没有带时间码的台词"
                              : w.lines.map((l) => `${l.at.toFixed(1)}s 起说 ${l.needs.toFixed(1)}s${l.fits ? "" : "（说不完）"}`).join("，")}
                          </div>
                        ))}
                      </div>
                    )}
                    {chain.warning && <div className="text-[11px] text-amber-300/80">{chain.warning}</div>}
                    <div className="text-[10px] text-white/35 leading-relaxed">
                      点下面的「生成」一次出完一整条。中途不能只重来某一窗口 —— H3 的潜变量存不了盘。
                      成片会落进这个节点的播放器，超分和收进分镜照常用。
                    </div>
                  </>
                ) : (
                  <div className="text-[11px] text-amber-300/80">还出不了：{chain.why}</div>
                )}
              </>
            )}
          </div>
          )}

          {/* 底栏参数控制行 (统一 h-7 高度，无硬底色边框，不折行) */}
          <div className="flex items-center justify-between gap-1.5 pt-1 text-xs text-white/70">
            <div className="flex items-center gap-2 flex-nowrap min-w-0">
              {/* 模型选择 */}
              <div className="relative flex items-center">
                <CanvasEngineSelect
                  mode={inference && inference.mode !== "motion" ? inference.mode : hasUpstreamImage ? "i2v" : "t2v"}
                  value={engine}
                  onChange={(newEngine) => {
                    onEngineChange(newEngine);
                    const currentR = inferVideoRatio(width, height, ratio);
                    const engineRatios = WEETODD_RATIO_VIDEO_SIZES[newEngine] || WEETODD_RATIO_VIDEO_SIZES.ltx;
                    const sizes = engineRatios[currentR] || engineRatios["16:9"];
                    const match = sizes.find((s) => s.width === width && s.height === height);
                    if (!match) {
                      const def = sizes[0];
                      onDimensionChange?.(def.width, def.height, currentR, def.label);
                    }
                  }}
                  disabled={isGenerating}
                />
              </div>

              {/* 视频生成模式：照 libtv 放在底栏、模型旁边（以前是顶上那一排的下拉框） */}
              {inference && (
                <VideoModeMenu inference={inference} videoUpstream={videoUpstream} disabled={isGenerating} onChange={(m) => onModeChange(m)} />
              )}

              {/* 仿照 libtv 的集成设置按钮卡片 (画幅、清晰度、时长、音频开关、生成数量) */}
              <CanvasVideoSettingsPopover
                engine={engine}
                width={width}
                height={height}
                ratio={ratio}
                resolution={resolution}
                durationSecs={durationSecs}
                videoCount={videoCount}
                generateAudio={generateAudio}
                disabled={isGenerating}
                onDimensionChange={(w, h, r, res) => onDimensionChange?.(w, h, r, res)}
                onDurationChange={onDurationChange}
                onVideoCountChange={onVideoCountChange}
                onGenerateAudioChange={onGenerateAudioChange}
                h3OfficialFormat={h3OfficialFormat}
                onH3OfficialFormatChange={onH3OfficialFormatChange}
                mode={inference?.mode}
                h3Quality={inference ? effectiveQuality(h3Quality, inference.mode) : undefined}
                onH3QualityChange={onH3QualityChange}
                h3Sampling={h3Sampling}
                onH3SamplingChange={onH3SamplingChange}
                longTake={longTake}
                onLongTakeChange={(on) => setWindows(on ? (longTake ? windows : openLongTake(localPrompt || prompt)) : [])}
              />


              {/* 润色按钮 文/A (接入 Anima AI 运镜润色) */}
              <button
                type="button"
                disabled={isPolishing}
                onClick={handlePolish}
                className="h-7 w-7 flex items-center justify-center rounded-md text-white/60 hover:text-white hover:bg-white/10 transition-colors shrink-0 disabled:opacity-50"
                title={isPolishing ? "AI 运镜润色中..." : "AI 智能运镜润色"}
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
                    ? `执行视频生成 (上游: ${upstreamPrompt.trim().slice(0, 30)}...)`
                    : "执行视频生成 (DAG)"
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
