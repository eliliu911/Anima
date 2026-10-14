import { useMemo, useState } from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";
import { NodePanelDock } from "../ui/NodePanelDock";
import { MIN_VIDEO_SECS } from "../utils/videoLimits";
import { BaseNodeWrapper } from "./BaseNodeWrapper";
import { useCanvasStore } from "../store/useCanvasStore";
import { selectContentNodes } from "../store/selectContentNodes";
import { executeNodePipeline, aggregateNodeInputs } from "../engine/dagExecutor";
import { MediaThumb, pathFromMediaUrl } from "../../components/studio/MediaThumb";
import { VideoOperationPanel } from "./panels/VideoOperationPanel";
import { NodeHeader } from "../ui/NodeHeader";
import { Video, Film, Loader2, Play } from "../ui/icons";
import { CanvasVideoUploadButton } from "../ui/CanvasVideoUploadButton";
import { spawnVideoTool } from "../utils/imageTools";
import { CanvasMediaGallery } from "../ui/CanvasMediaGallery";
import type { VideoGenNode } from "../types";
import { inferVideoMode, liveNotes } from "../utils/videoMode";
import { effectiveMotionEngine, isRefMotion, motionSpec, upstreamVideoNodes } from "../utils/depthChain";
import { inferCurrentResolution } from "../ui/CanvasVideoSettingsPopover";

import { planVideoCommit } from "../utils/videoCommit";
import { isLongTake, planLongTake } from "../utils/longTakeWindows";
import { cinemaAdoptVideo } from "../../api/cinema";

export function VideoGenNodeComponent({ id, data, selected }: NodeProps<VideoGenNode>) {
  const updateNodeData = useCanvasStore((s) => s.updateNodeData);
  const removeNode = useCanvasStore((s) => s.removeNode);
  const addNode = useCanvasStore((s) => s.addNode);
  const nodes = useCanvasStore(selectContentNodes);
  const edges = useCanvasStore((s) => s.edges);

  const upstream = useMemo(() => {
    return aggregateNodeInputs(id, nodes, edges);
  }, [id, nodes, edges]);
  const inputPreview = data.references?.[0]?.path || upstream.inputImageUrl;

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

  const outputList = data.outputVideos?.length
    ? data.outputVideos
    : (data.outputVideoUrl ? [data.outputVideoUrl] : []);
  const count = outputList.length || data.videoCount || 1;
  const isExpanded = Boolean(data.isExpanded && count > 1);

  // 动态卡片宽度：展开画册时水平平铺拓展
  const nodeWidth = isExpanded ? Math.min(960, Math.max(440, count * 310 + 24)) : 440;

  const handleApplyToCanvas = (path: string) => {
    const thisNode = useCanvasStore.getState().nodes.find((n) => n.id === id);
    const pos = thisNode
      ? { x: thisNode.position.x + (isExpanded ? nodeWidth + 40 : 480), y: thisNode.position.y }
      : { x: 300, y: 300 };

    addNode("videoGen", pos, {
      outputVideoUrl: path,
      outputVideos: [path],
      prompt: data.prompt || "",
      engine: data.engine || "ltx",
      width: data.width || (data.engine === "h3" ? 640 : 768),
      height: data.height || (data.engine === "h3" ? 384 : 512),
      ratio: data.ratio,
      resolution: data.resolution,
      durationSecs: data.durationSecs || 5,
    });
  };

  const currentW = data.width || (data.engine === "h3" ? 640 : 768);
  const currentH = data.height || (data.engine === "h3" ? 384 : 512);
  const hasUpstreamNodes = edges.some((e) => e.target === id);
  const inference = useMemo(() => inferVideoMode(id, nodes, edges), [id, nodes, edges]);
  const motion = inference.mode === "motion" ? inference : null;
  // 连了几张人物图：多于一张就是多人换人（后端强制走 Qwen 首帧重绘）
  const multiPeople = (motion?.images.length ?? 0) > 1;
  // 和执行时同一套推法：清晰度、时长跟下方设置走，按这条路的尺寸表和上限折算
  // 实际走哪一档：原视频直接连进来时只有原片复刻两档（另外两档要深度）
  const eng = effectiveMotionEngine(data.motionEngine, motion?.via !== "video");
  const spec = motion
    ? motionSpec(eng, inferCurrentResolution(data.resolution, currentW, currentH), data.durationSecs || 5)
    : null;

  /**
   * 这个节点是一条长镜头时的标题。不是就 `null`。
   *
   * **显示在标题上，不用选中就能看见** —— 一条长镜头和一次普通出片
   * 耗时差一个数量级（四窗口约 1.2 小时），点下去之前就得知道。
   *
   * 这里写的是**净成片时长**，不是「窗口数 × 窗口秒数」：
   * 窗口之间有重叠帧，发布时会裁掉，按乘法算必然算多。
   */
  const longTakeLabel = (() => {
    const p = planLongTake(data);
    if (!p.ok) return isLongTake(data) ? "长镜头 · 接不上" : null;
    return `长镜头 · ${p.windows} 个窗口 × ${p.windowSecs}s = 约 ${p.netSecs.toFixed(1)}s`;
  })();

  return (

    <div className="flex flex-col gap-2 select-none transition-all duration-200" style={{ width: nodeWidth }}>
      {/* 顶部悬浮标题栏 */}
      <NodeHeader
        icon={<Video className="h-3.5 w-3.5 text-white/60" />}
        title={longTakeLabel ?? (isExpanded ? `画册 · ${count} 个` : data.videoTool ? String(data.label || "视频工具") : "视频")}
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
                  <Video className="h-3 w-3 opacity-60" />
                  <span>{currentW}×{currentH}</span>
                </span>
                <CanvasVideoUploadButton nodeId={id} />
                <CommitToShotButton nodeId={id} />
                {data.outputVideoUrl && !data.isGenerating && (
                  <>
                    <button
                      type="button"
                      title="超分 ×2：在右边新建一个节点，宽高各放大一倍、画面更清楚，时长和声音不变 · 12 秒的 480p 片子约 2 分钟"
                      onClick={() => spawnVideoTool(id, "upscale")}
                      className="nodrag nopan rounded-md border border-white/10 bg-white/5 px-2 py-1 text-xs text-white/70 hover:bg-white/10"
                    >
                      超分 ×2
                    </button>
                    <button
                      type="button"
                      title="补帧 ×2：在右边新建一个节点，帧率翻倍、动作更顺，时长和声音不变 · 约几秒"
                      onClick={() => spawnVideoTool(id, "interpolate")}
                      className="nodrag nopan rounded-md border border-white/10 bg-white/5 px-2 py-1 text-xs text-white/70 hover:bg-white/10"
                    >
                      补帧 ×2
                    </button>
                  </>
                )}
              </>
            )}
          </div>
        }
      />

      {/* 主卡片 */}
      <BaseNodeWrapper
        id={id}
        selected={selected}
        isGenerating={data.isGenerating}
        error={data.error}
        notes={liveNotes(data.notes, inference)}
        sourceHandleId="video-out"
      >
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
          kind="video"
          items={outputList}
          primaryItem={data.outputVideoUrl}
          targetCount={data.videoCount || 1}
          isGenerating={data.isGenerating}
          width={currentW}
          height={currentH}
          isExpanded={isExpanded}
          onToggleExpand={() => updateNodeData(id, { isExpanded: !isExpanded })}
          onSelectPrimary={(path) => updateNodeData(id, { outputVideoUrl: path })}
          onApplyToCanvas={handleApplyToCanvas}
          placeholder={
            data.mode === "r2v" ? (
              <div className="relative w-full h-full flex items-center justify-center">
                <Play className="h-10 w-10 text-white/20 pointer-events-none" />
              </div>
            ) : inputPreview ? (
              <div className="relative w-full h-full flex items-center justify-center bg-black/40 group">
                <MediaThumb
                  path={inputPreview}
                  alt="首帧图像"
                  className="w-full h-full object-contain"
                />
                <div className="absolute top-2.5 left-2.5 flex items-center gap-1.5 rounded-full bg-black/70 backdrop-blur-md px-2.5 py-0.5 text-[10px] text-white/80 border border-white/10 shadow-lg">
                  <Film className="h-3 w-3 text-emerald-400" />
                  <span>{motion ? "这是连进来的人物图 · 生成后这里显示视频"
                    : inference.mode === "r2v" ? "这是外观参考图（不当首帧）· 未生成"
                    : inference.mode === "fl2v" ? "已接入首帧和尾帧 · 未生成"
                    : "已接入首帧 · 未生成"}</span>
                </div>
                <div className="absolute inset-0 flex items-center justify-center bg-black/20 opacity-0 group-hover:opacity-100 transition-opacity pointer-events-none">
                  <div className="h-10 w-10 rounded-full bg-white/25 backdrop-blur-md flex items-center justify-center text-white">
                    <Play className="h-4 w-4 ml-0.5" />
                  </div>
                </div>
              </div>
            ) : upstream.upstreamImageNode?.isGenerating ? (
              <div className="relative w-full h-full flex flex-col items-center justify-center gap-2 p-6 bg-black/20">
                <Loader2 className="h-5 w-5 text-white/60 animate-spin" />
                <span className="text-xs text-white/60">等待上游图片生成首帧...</span>
              </div>
            ) : hasUpstreamNodes ? (
              <div className="relative w-full h-full flex items-center justify-center">
                <Play className="h-16 w-16 text-white/[0.04] pointer-events-none" />
              </div>
            ) : (
              <div className="relative w-full h-full flex flex-col items-start justify-center p-6">
                {/* 首帧从外部素材节点连进来，不在视频节点自己身上传（用户 2026-09-14：素材节点 → 生成节点） */}
                <span className="text-xs leading-relaxed text-white/45">
                  直接写提示词出片；<br />
                  要用首帧或人物图：建一个「上传资源」素材节点，从它右边的 + 连到这里；<br />
                  要复刻动作：点上面「上传视频」，再接「深度动作捕捉」；要原样换人（原片复刻），把这段视频直接连到下一个视频节点。
                </span>
                <div className="absolute inset-0 flex items-center justify-center pointer-events-none">
                  <Play className="h-16 w-16 text-white/[0.04]" />
                </div>
              </div>
            )
          }
        />
        {/* 上游直连了一段视频、但没选原片复刻：这条线现在什么都不传 —— 给一个明确的开关，不靠连线偷偷切模式 */}
        {!motion && !data.videoTool && upstreamVideoNodes(id, nodes, edges).length > 0 && (
          <div className="nodrag nopan flex items-center gap-2 border-t border-white/10 px-3 py-2 text-[11px] text-white/55">
            <span className="flex-1 leading-relaxed">上游连了一段视频 —— 现在这根线不传任何东西。要照这段视频的动作和镜头出片、人换成连进来的人物图，设成全能参考（模式菜单里也能选）</span>
            <button type="button" disabled={data.isGenerating}
              onClick={() => updateNodeData(id, { motionEngine: "h3_ref", engine: "h3", mode: "auto" })}
              className="shrink-0 rounded-md border border-emerald-400/40 bg-emerald-400/10 px-2 py-1 text-[11px] text-white/85 hover:bg-emerald-400/20">
              设成全能参考
            </button>
          </div>
        )}
        {/* 动作迁移：清晰度、时长跟下方设置走；这条路有自己的尺寸表和时长上限，实际出多大多长要写出来 */}
        {motion && (
          <div className="nodrag nopan flex flex-col gap-2 border-t border-white/10 px-3 py-2.5 text-[11px] text-white/60">
            <p className="leading-relaxed text-emerald-200/80">{motion.via === "video" ? "全能参考" : "动作迁移"} · {motion.reason}</p>
            <div className="grid grid-cols-2 gap-1.5" role="radiogroup" aria-label="动作迁移方式">
              {([
                // 动作精准只靠首帧记住人：人物图不完整（被裁到边上）时后端会拦下，这里先说清
                ["ltx", "动作精准", "动作逐帧跟原视频走；人物图要完整全身（头到脚都在图里）。约 2~3 分钟"],
                ["h3", "人物清晰", "半身、全身都行，脸和衣服全程清楚；关键姿势大多对得上但不逐帧。约 17 分钟"],
                // 同一个做法走 WeeTodd：09-24 Kotte 8 段跑通的那一套
                ["h3_ref", "原片复刻 · 快", "背景、光、运镜、切镜都照原片，人换成人物图（三视图整张给）。走 WeeTodd，8.5 秒约 65 分钟。原视频直接连进来，不用深度"],
              ] as const).filter(([value]) => motion.via !== "video" || isRefMotion(value)).map(([value, label, hint]) => {
                const on = eng === value;
                return (
                  <button
                    key={value} type="button" role="radio" aria-checked={on} title={hint}
                    disabled={data.isGenerating}
                    onClick={() => updateNodeData(id, { motionEngine: value })}
                    className={`flex-1 rounded-md border px-2 py-1.5 text-left ${on ? "border-emerald-400/50 bg-emerald-400/10 text-white/90" : "border-white/10 bg-white/5 text-white/55 hover:bg-white/10"}`}
                  >
                    <span className="block text-[12px]">{label}</span>
                    <span className="block text-[10px] leading-snug text-white/40">{hint}</span>
                  </button>
                );
              })}
            </div>
            {/* 一类一行、左边固定标题。以前四样挤一行，440 宽的节点里被压成竖排 */}
            <div className="grid grid-cols-[44px_1fr] items-center gap-x-2 gap-y-2">
              <span className="text-white/40">片段</span>
              <label className="flex items-center gap-1.5 whitespace-nowrap">
                从第
                <input
                  type="number" min={0} step={0.5}
                  value={data.motionStartSecs ?? 0}
                  onChange={(e) => updateNodeData(id, { motionStartSecs: Math.max(0, Number(e.target.value) || 0) })}
                  // 小一号：和旁边 11px 的字一样高，去掉上下箭头和系统的粗蓝色焦点框
                  className="h-5 w-10 rounded border border-white/10 bg-white/5 px-1 text-center text-[11px] leading-none text-white/85 outline-none focus:border-emerald-400/50 [appearance:textfield] [&::-webkit-inner-spin-button]:appearance-none [&::-webkit-outer-spin-button]:appearance-none"
                />
                秒开始，取 {spec!.secs} 秒
              </label>

              {eng === "ltx" && (
                <>
                  <span className="text-white/40">控制</span>
                  <div className="flex gap-1.5" role="radiogroup" aria-label="动作控制信号">
                    {([
                      // 2026-09-22 同题实测：轮廓线的帧间运动相关 0.160，深度 0.069（MEDIA_PIPELINES 5.8）
                      ["canny", "轮廓线", "从原视频抽边缘线，动作跟得最紧（实测是深度的 2 倍多）。默认"],
                      ["depth", "深度", "只看远近。大脸特写、画面很暗时轮廓线也弱，那几段用它"],
                    ] as const).map(([value, label, hint]) => {
                      const on = (data.motionControl ?? "canny") === value;
                      return (
                        <button
                          key={value} type="button" role="radio" aria-checked={on} title={hint}
                          disabled={data.isGenerating}
                          onClick={() => updateNodeData(id, { motionControl: value })}
                          className={`rounded-md border px-2 py-0.5 text-[11px] ${on ? "border-emerald-400/50 bg-emerald-400/10 text-white/90" : "border-white/10 bg-white/5 text-white/55 hover:bg-white/10"}`}
                        >
                          {label}
                        </button>
                      );
                    })}
                  </div>
                  <span className="text-white/40">选项</span>
                  <div className="flex flex-wrap items-center gap-x-4 gap-y-1.5">
                    {/* 多人时必开：贴图对位一次只能贴一个人，只能靠 Qwen 一次换掉所有人 */}
                    <label className="flex items-center gap-1.5 whitespace-nowrap" title={multiPeople ? "连了多张人物图：首帧必须用 Qwen 一次把所有人换掉，所以这一项自动打开" : "先用 Qwen 编辑把原视频这一帧里的人换成人物图，再按动作出片：墙面、地面、光线都保留。多约 5~6 分钟"}>
                      <input type="checkbox" checked={multiPeople || Boolean(data.motionKeepBackground)} disabled={data.isGenerating || multiPeople}
                        onChange={(e) => updateNodeData(id, { motionKeepBackground: e.target.checked })} />
                      保留原视频背景（多约 5 分钟）{multiPeople && <span className="text-white/35">· 多人时必开</span>}
                    </label>
                  </div>
                </>
              )}
              {eng === "h3_ref" && (
                <>
                  <span className="text-white/40">说明</span>
                  <span className="text-white/45">原视频原样当参考视频 1（不缩放、不改帧率；直接把原视频连到这个节点就行，不用深度节点），人物图按连线顺序是图片 1、图片 2……提示词照「逐毫秒母版」写：谁换成谁、各自长什么样、「图片1、图片2仅为形象参考，不要出现在视频中」、再逐毫秒写动作。多人时按角色写，不要按左右。一次最长 10 秒</span>
                </>
              )}
              {eng === "h3" && multiPeople && (
                <>
                  <span className="text-white/40">多人</span>
                  <span className="text-white/45">每个人全程看自己那张参考图（不靠首帧），剪辑切换也跟着原片走，不用按镜头切段。默认按首帧从左到右对应；镜头里左右会变时，在提示词里按角色写清谁是谁（比如「开枪的是 &lt;Picture 1&gt;」）</span>
                </>
              )}

              <span className="text-white/40">规格</span>
              <span className="text-white/35">
                {spec!.tier} {spec!.size}（竖屏视频宽高对调）· {spec!.secs} 秒 · 跟下方的清晰度、时长设置走
                {spec!.downgraded && <span className="block text-amber-200/70">人物清晰最高 720P，按 720P 出</span>}
                {spec!.clipped && <span className="block text-amber-200/70">这条路一次最长 {spec!.secs} 秒，更长的从后面的起始秒再出一条</span>}
              </span>
            </div>
            {data.motionFirstFrame && eng === "ltx" && (
              <div className="flex items-center gap-2">
                <MediaThumb path={data.motionFirstFrame} alt="自动对位的首帧" className="h-16 w-24 rounded object-contain bg-black/40" />
                <span className="leading-relaxed text-white/40">{multiPeople || data.motionKeepBackground ? "上一次模型收到的首帧：Qwen 在原视频这一帧里把人换好的那张" : "上一次模型收到的首帧：人物已自动缩放、摆到视频第一帧里人的位置"}</span>
              </div>
            )}
          </div>
        )}
      </BaseNodeWrapper>


      {/* 底部附着生成设定控制台：不跟画布缩放（NodePanelDock） */}
      <NodePanelDock visible={Boolean(selected)}>
        <div className="animate-in fade-in-0 duration-150">
          <VideoOperationPanel
            nodeId={id}
            mode={data.mode ?? "auto"}
            onModeChange={(mode) => mode === "omni"
              // 全能参考 = 原片复刻（以前是下面那个「设成原片复刻」按钮）
              ? updateNodeData(id, { motionEngine: "h3_ref", engine: "h3", mode: "auto" })
              // 从全能参考切走：关掉原片复刻；深度那条的 motionEngine（ltx / h3）不动
              : updateNodeData(id, { mode, ...(mode === "r2v" ? { engine: "h3" } : {}), ...(isRefMotion(data.motionEngine) ? { motionEngine: undefined } : {}) })}
            prompt={data.prompt || ""}
            onPromptChange={(p) => updateNodeData(id, { prompt: p })}
            upstreamPrompt={upstream.combinedPrompt}
            hasUpstreamContent={hasUpstreamContent}
            hasUpstreamImage={Boolean(inputPreview)}
            upstreamImageGenerating={Boolean(upstream.upstreamImageNode?.isGenerating)}
            engine={data.engine || "ltx"}
            onEngineChange={(e) => updateNodeData(id, { engine: e })}
            width={currentW}
            height={currentH}
            ratio={data.ratio}
            resolution={data.resolution}
            onDimensionChange={(w, h, r, res) =>
              updateNodeData(id, { width: w, height: h, ratio: r, resolution: res })
            }
            durationSecs={Math.max(MIN_VIDEO_SECS, data.durationSecs || MIN_VIDEO_SECS)}
            onDurationChange={(d) => updateNodeData(id, { durationSecs: d })}
            videoCount={data.videoCount || 1}
            onVideoCountChange={(count) => updateNodeData(id, { videoCount: count })}
            generateAudio={data.generateAudio ?? false}
            onGenerateAudioChange={(enabled) => updateNodeData(id, { generateAudio: enabled })}
            h3OfficialFormat={data.h3OfficialFormat ?? false}
            onH3OfficialFormatChange={(on) => updateNodeData(id, { h3OfficialFormat: on })}
            h3Quality={data.h3Quality}
            onH3QualityChange={(q) => updateNodeData(id, { h3Quality: q })}
            h3Sampling={data.h3Sampling}
            onH3SamplingChange={(s) => updateNodeData(id, { h3Sampling: s })}
            isGenerating={data.isGenerating || !!data.referenceLoading}
            onGenerate={handleGenerate}
          />
        </div>
      </NodePanelDock>
    </div>
  );
}


/**
 * 把这段片子收回它上游那一镜。
 *
 * **上游没接分镜就不显示。** 和图片那边一个规矩 —— 没有落点还给按钮，
 * 点下去只能回一句「不知道收到哪儿」，比不显示更糟。
 *
 * 收进去是**追加一「次」**，不覆盖：画布出的片不一定比原来那条好，
 * 用户在分镜页能换回去。
 */
function CommitToShotButton({ nodeId }: { nodeId: string }) {
  const nodes = useCanvasStore(selectContentNodes);
  const edges = useCanvasStore((s) => s.edges);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState("");
  const [err, setErr] = useState("");
  const plan = useMemo(() => planVideoCommit(nodeId, nodes, edges), [nodeId, nodes, edges]);
  if (!plan.ok) return null;

  const go = async () => {
    setBusy(true); setErr("");
    try {
      const out = await cinemaAdoptVideo({
        projectId: plan.shot.projectId,
        episodeId: plan.shot.episodeId,
        shotId: plan.shot.shotId,
        sourcePath: pathFromMediaUrl(plan.path) ?? plan.path,
        prompt: plan.prompt,
        engine: plan.engine,
        picturePaths: plan.pictures,
      });
      setDone(`已收为第 ${out.take_count} 次`);
    } catch (e) { setErr(String(e)); } finally { setBusy(false); }
  };

  return (
    <>
      <button
        type="button"
        disabled={busy}
        title={`收进「${plan.shot.label}」，这一集的预览和组装直接用它${plan.note ? `\n注意：${plan.note}` : ""}`}
        onClick={() => void go()}
        className="nodrag nopan rounded-md border border-white/10 bg-white/5 px-2 py-1 text-xs text-white/70 hover:bg-white/10 disabled:opacity-50"
      >
        {busy ? "收片中…" : `收进${plan.shot.label}`}
      </button>
      {done && <span className="text-[11px] text-emerald-300/80" role="status">{done}</span>}
      {err && <span className="text-[11px] text-rose-300/80" role="alert">{err}</span>}
    </>
  );
}
