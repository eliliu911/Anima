import { useState, useRef, useEffect } from "react";
import { longVideoPlan, MAX_LONG_VIDEO_SECS, H3_SINGLE_MAX_SECS } from "../utils/longVideo";
import { Volume2, VolumeX, HelpCircle, ChevronDown } from "lucide-react";
import { WEETODD_RATIO_VIDEO_SIZES } from "./CanvasVideoResolutionSelect";
import { MIN_VIDEO_SECS } from "../utils/videoLimits";
import { renderSize } from "../../components/studio/storyboard/renderOptions";
import { qualityChoices, qualityInfo, type CanvasH3Quality } from "../utils/h3Quality";
import type { VideoMode } from "../utils/videoMode";

export interface CanvasVideoSettingsPopoverProps {
  engine: string;
  width?: number;
  height?: number;
  ratio?: string;
  resolution?: string;
  durationSecs: number;
  videoCount: number;
  generateAudio: boolean;
  disabled?: boolean;
  onDimensionChange?: (width: number, height: number, ratio: string, resolution: string) => void;
  onDurationChange: (duration: number) => void;
  onVideoCountChange?: (count: number) => void;
  onGenerateAudioChange?: (generateAudio: boolean) => void;
  /** H3 官方格式（只 H3 显示）。不传回调就不显示这一项 */
  h3OfficialFormat?: boolean;
  onH3OfficialFormatChange?: (on: boolean) => void;
  /** 现在走哪条链路（决定「加速」给哪几档）。不传就不显示「加速」 */
  mode?: VideoMode;
  /** 实际用的加速档（`effectiveQuality` 算好的） */
  h3Quality?: CanvasH3Quality;
  onH3QualityChange?: (q: CanvasH3Quality) => void;
  /** H3 采样方法（空 = euler）。不传回调就不显示 */
  h3Sampling?: string;
  onH3SamplingChange?: (s: string) => void;
  /** 长镜头开没开（一个节点出一整段）。不传就不显示这一项 */
  longTake?: boolean;
  onLongTakeChange?: (enable: boolean) => void;
}

export const H3_SAMPLINGS = [
  { id: "euler", label: "Euler", hint: "默认" },
  { id: "res_multistep", label: "RES 多步", hint: "打斗 LoRA 推荐" },
] as const;

export const RATIO_OPTIONS = [
  { id: "Auto", label: "Auto" },
  { id: "16:9", label: "16:9" },
  { id: "4:3", label: "4:3" },
  { id: "1:1", label: "1:1" },
  { id: "3:4", label: "3:4" },
  { id: "9:16", label: "9:16" },
  { id: "21:9", label: "21:9" },
];

export const RESOLUTION_OPTIONS = ["480P", "720P", "1080P"] as const;

export function inferCurrentRatio(ratio?: string, width?: number, height?: number): string {
  if (ratio && ["Auto", "16:9", "4:3", "1:1", "3:4", "9:16", "21:9"].includes(ratio)) {
    return ratio;
  }
  if (!width || !height) return "16:9";
  const r = width / height;
  if (Math.abs(r - 1) < 0.05) return "1:1";
  if (Math.abs(r - 4 / 3) < 0.08) return "4:3";
  if (Math.abs(r - 3 / 4) < 0.08) return "3:4";
  if (Math.abs(r - 21 / 9) < 0.15) return "21:9";
  if (width < height) return "9:16";
  return "16:9";
}

/**
 * **H3 原生最高 1344×768**（官方总像素上限 1,032,192 —— 1344×768 正好卡在上限；1920×1080 是它的两倍）。
 * 所以 H3 没有 1080P：要 1080P 出完接「超分 ×2」。以前画布 H3 的「1080P」实际发的是 1344×768、「720P」是 768×512，
 * 标签和工作台对不上（用户 2026-10-08 问「H3 可以出 1080P 吗」）。现在 H3 用工作台同一套尺寸（`renderSize`）。
 */
export const H3_NO_1080P = "H3 原生最高约 720P（官方总像素上限 1344×768）。要 1080P：出完从右边 + 接「超分 ×2」";

const isH3 = (engine?: string) => (engine ?? "").toLowerCase().includes("h3");

export function inferCurrentResolution(resolution?: string, width?: number, height?: number, engine?: string): string {
  // H3：按实际尺寸说（老节点存着「1080P」其实是 1344×768，就是 720P 那一档）
  if (isH3(engine)) {
    // **先看实际尺寸**：老节点存着「720P」实际是 768×512（以前画布 H3 的 720P 档），照标签显示就是高亮错的那一格（用户 10-08 截图）
    if (width && height) return Math.max(width, height) <= 960 ? "480P" : "720P";
    return resolution === "720P" ? "720P" : "480P";
  }
  if (resolution && (RESOLUTION_OPTIONS as readonly string[]).includes(resolution)) {
    return resolution;
  }
  if (resolution === "4K") {
    return "1080P";
  }
  if (!width || !height) return "720P";
  const maxDim = Math.max(width, height);
  const minDim = Math.min(width, height);
  if (maxDim <= 640 || minDim <= 400) return "480P";
  if (maxDim <= 1024 && minDim <= 600) return "720P";
  return "1080P";
}

export function resolveVideoDimensions(
  engine: string,
  ratio: string,
  resolution: string,
): { width: number; height: number } {
  if (isH3(engine)) {
    // 和工作台出片设置同一套（480P = 短边 512、720P = 短边 736，按比例、32 对齐）；1080P 没有，按 720P
    const [width, height] = renderSize(
      { quality: "staged", keyframe_quality: "lightx2v", resolution: resolution === "480P" ? "480p" : "720p", aspect: ratio === "Auto" ? "16:9" : ratio, takes: 1, generate_audio: true, burn_subtitles: false },
      "16:9",
    );
    return { width, height };
  }
  const normEngine = engine.toLowerCase().includes("h3") ? "h3" : "ltx";
  const normRatio = ratio === "Auto" ? "16:9" : ratio;
  const engineMap = WEETODD_RATIO_VIDEO_SIZES[normEngine] || WEETODD_RATIO_VIDEO_SIZES.ltx;
  const sizes = engineMap[normRatio] || engineMap["16:9"] || [];

  if (resolution === "480P") {
    return { width: sizes[0]?.width || 640, height: sizes[0]?.height || 384 };
  } else if (resolution === "720P") {
    const idx = sizes.length > 1 ? 1 : 0;
    return { width: sizes[idx]?.width || 768, height: sizes[idx]?.height || 512 };
  } else {
    // 1080P 或兜底
    const top = sizes[sizes.length - 1];
    return { width: top?.width || 1344, height: top?.height || 768 };
  }
}

export function CanvasVideoSettingsPopover({
  engine,
  width,
  height,
  ratio: propRatio,
  resolution: propResolution,
  durationSecs,
  videoCount,
  generateAudio,
  disabled = false,
  onDimensionChange,
  onDurationChange,
  onVideoCountChange,
  onGenerateAudioChange,
  h3OfficialFormat = false,
  onH3OfficialFormatChange,
  longTake,
  onLongTakeChange,
  mode,
  h3Quality,
  onH3QualityChange,
  h3Sampling,
  onH3SamplingChange,
}: CanvasVideoSettingsPopoverProps) {
  const choices = isH3(engine) && mode && onH3QualityChange ? qualityChoices(mode) : [];
  const [isOpen, setIsOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  const currentRatio = inferCurrentRatio(propRatio, width, height);
  const currentResolution = inferCurrentResolution(propResolution, width, height, engine);

  useEffect(() => {
    if (!isOpen) return;
    const handleClickOutside = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setIsOpen(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, [isOpen]);

  const handleRatioClick = (newRatio: string) => {
    const { width: newW, height: newH } = resolveVideoDimensions(engine, newRatio, currentResolution);
    onDimensionChange?.(newW, newH, newRatio, currentResolution);
  };

  const handleResolutionClick = (newRes: string) => {
    const { width: newW, height: newH } = resolveVideoDimensions(engine, currentRatio, newRes);
    onDimensionChange?.(newW, newH, currentRatio, newRes);
  };

  // 比例图标渲染
  const renderRatioIcon = (id: string) => {
    switch (id) {
      case "Auto":
        return <div className="w-3.5 h-3.5 border border-dashed border-current rounded-xs mb-1 opacity-75" />;
      case "16:9":
        return <div className="w-4 h-2.5 border border-current rounded-xs mb-1" />;
      case "4:3":
        return <div className="w-3.5 h-2.5 border border-current rounded-xs mb-1" />;
      case "1:1":
        return <div className="w-3 h-3 border border-current rounded-xs mb-1" />;
      case "3:4":
        return <div className="w-2.5 h-3.5 border border-current rounded-xs mb-1" />;
      case "9:16":
        return <div className="w-2.5 h-4 border border-current rounded-xs mb-1" />;
      case "21:9":
        return <div className="w-5 h-2 border border-current rounded-xs mb-1" />;
      default:
        return <div className="w-3.5 h-2.5 border border-current rounded-xs mb-1" />;
    }
  };

  return (
    <div className="relative flex items-center" ref={containerRef}>
      {/* 仿照 libtv 的单胶囊集成按钮 (无边框无底色) */}
      <button
        type="button"
        disabled={disabled}
        onClick={() => setIsOpen((prev) => !prev)}
        className={`nodrag nopan h-7 flex items-center gap-1.5 px-1 text-xs font-normal transition-all cursor-pointer ${
          isOpen
            ? "text-white font-medium"
            : "text-white/80 hover:text-white"
        } disabled:opacity-50 disabled:cursor-not-allowed`}
        title="打开视频尺寸、时长、音频与抽卡参数设置"
      >
        {/* 当前比例小图标 */}
        <span className="shrink-0 flex items-center text-white/70">
          {currentRatio === "9:16" ? (
            <div className="w-2 h-3.5 border border-current rounded-xs" />
          ) : currentRatio === "1:1" ? (
            <div className="w-3 h-3 border border-current rounded-xs" />
          ) : currentRatio === "3:4" ? (
            <div className="w-2.5 h-3 border border-current rounded-xs" />
          ) : currentRatio === "21:9" ? (
            <div className="w-4 h-1.5 border border-current rounded-xs" />
          ) : (
            <div className="w-3.5 h-2 border border-current rounded-xs" />
          )}
        </span>

        {/* 摘要文字: 16:9 · 720P · 5s · 1个 */}
        <span className="tracking-tight">
          {currentRatio} · {currentResolution} · {durationSecs}s · {videoCount}个{choices.length > 0 && h3Quality ? ` · ${qualityInfo(h3Quality).label}` : ""}
        </span>

        {/* 音频状态图标 */}
        <span className="shrink-0 ml-0.5" title={generateAudio ? "已开启模型音效生成" : "已关闭模型音效 (纯净无声)"}>
          {generateAudio ? (
            <Volume2 className="h-3.5 w-3.5 text-white/90" />
          ) : (
            <VolumeX className="h-3.5 w-3.5 text-white/40" />
          )}
        </span>

        {/* 下拉箭头 */}
        <ChevronDown className={`h-3 w-3 text-white/40 transition-transform ${isOpen ? "rotate-180 text-white/70" : ""}`} />
      </button>

      {/* 弹出的参数设置卡片 */}
      {isOpen && (
        <div
          className="nodrag nopan absolute bottom-full left-0 mb-2 z-50 w-[620px] max-w-[calc(100vw-48px)] rounded-2xl border border-white/15 bg-[#18181b]/98 p-3.5 shadow-2xl backdrop-blur-2xl grid grid-cols-2 items-start gap-x-5 select-none text-zinc-300 text-xs animate-in fade-in-0 zoom-in-95"
          onClick={(e) => e.stopPropagation()}
        >
          {/* 横向两栏（用户 2026-10-08：「这个菜单现在太高了，能不能做成横向分布的」）：左边画面（比例 / 清晰度 / 时长 / 数量），右边出片（加速 / 声音 / 格式 / 长镜头） */}
          <div className="flex min-w-0 flex-col gap-3.5">
          {/* 1. 比例设置 */}
          <div>
            <div className="text-[11px] font-medium text-zinc-400 mb-2">比例</div>
            <div className="grid grid-cols-5 gap-1.5 mb-1.5">
              {RATIO_OPTIONS.slice(0, 5).map((opt) => {
                const isActive = currentRatio === opt.id;
                return (
                  <button
                    key={opt.id}
                    type="button"
                    onClick={() => handleRatioClick(opt.id)}
                    className={`flex flex-col items-center justify-center py-2 px-1 rounded-xl text-[11px] transition-all cursor-pointer ${
                      isActive
                        ? "border border-white/90 bg-white/10 text-white font-medium shadow-sm"
                        : "border border-white/5 bg-zinc-800/40 text-zinc-400 hover:border-white/20 hover:text-zinc-200 hover:bg-zinc-800/80"
                    }`}
                  >
                    {renderRatioIcon(opt.id)}
                    <span className="leading-none">{opt.label}</span>
                  </button>
                );
              })}
            </div>
            <div className="grid grid-cols-5 gap-1.5">
              {RATIO_OPTIONS.slice(5).map((opt) => {
                const isActive = currentRatio === opt.id;
                return (
                  <button
                    key={opt.id}
                    type="button"
                    onClick={() => handleRatioClick(opt.id)}
                    className={`flex flex-col items-center justify-center py-2 px-1 rounded-xl text-[11px] transition-all cursor-pointer ${
                      isActive
                        ? "border border-white/90 bg-white/10 text-white font-medium shadow-sm"
                        : "border border-white/5 bg-zinc-800/40 text-zinc-400 hover:border-white/20 hover:text-zinc-200 hover:bg-zinc-800/80"
                    }`}
                  >
                    {renderRatioIcon(opt.id)}
                    <span className="leading-none">{opt.label}</span>
                  </button>
                );
              })}
            </div>
          </div>

          {/* 2. 清晰度设置 */}
          <div>
            <div className="text-[11px] font-medium text-zinc-400 mb-2">清晰度</div>
            <div className="grid grid-cols-3 gap-2">
              {RESOLUTION_OPTIONS.map((res) => {
                const isActive = currentResolution === res;
                // H3 没有 1080P（官方像素上限），置灰并说清楚怎么拿到 1080P
                const blocked = isH3(engine) && res === "1080P";
                return (
                  <button
                    key={res}
                    type="button"
                    disabled={blocked}
                    title={blocked ? H3_NO_1080P : undefined}
                    onClick={() => handleResolutionClick(res)}
                    className={`h-8 flex items-center justify-center rounded-xl text-xs font-medium transition-all cursor-pointer disabled:cursor-not-allowed disabled:opacity-35 ${
                      isActive
                        ? "border border-white/90 bg-white/10 text-white shadow-sm"
                        : "border border-white/5 bg-zinc-800/40 text-zinc-400 hover:border-white/20 hover:text-zinc-200 hover:bg-zinc-800/80"
                    }`}
                  >
                    {res}
                  </button>
                );
              })}
            </div>
            {isH3(engine) && (
              <p className="mt-1.5 text-[10.5px] leading-snug text-zinc-500">
                {width && height ? `实际 ${width}×${height}。` : ""}H3 最高约 720P，要 1080P 出完接「超分 ×2」
              </p>
            )}
          </div>

          {/* 3. 视频时长 */}
          <div>
            <div className="flex items-center justify-between mb-2">
              <span className="text-[11px] font-medium text-zinc-400">视频时长</span>
              <span className="px-2.5 py-0.5 rounded-lg bg-zinc-800/80 border border-white/10 text-xs font-mono text-zinc-200">
                {durationSecs} s
              </span>
            </div>
            <div className="flex items-center gap-2 px-0.5">
              <input
                type="range"
                min={MIN_VIDEO_SECS}
                max={isH3(engine) ? MAX_LONG_VIDEO_SECS : H3_SINGLE_MAX_SECS}
                step={1}
                value={durationSecs}
                onChange={(e) => onDurationChange(Number(e.target.value))}
                className="w-full h-1.5 bg-zinc-700/80 rounded-lg appearance-none cursor-pointer accent-blue-500"
              />
            </div>
            {/* 超长视频：还是这个「生成」，后台分段串成一整条（和工作台超长镜同一套）。说清楚会怎么分、有什么要求 */}
            {isH3(engine) && (() => {
              const lv = longVideoPlan(durationSecs);
              return lv ? (
                <p className="mt-1.5 text-[10.5px] leading-snug text-zinc-500">
                  超长视频：分 {lv.windows} 段 × {lv.windowSecs}s，后台一次串成一整条。提示词里按时间写了镜头就按时间拆，没写就交给后台模型拆；
                  要参考出片（连上人物 / 场景图），每段最后 2 秒别安排台词。
                </p>
              ) : null;
            })()}
          </div>

          {/* 5. 生成数量 (抽卡) */}
          <div>
            <div className="text-[11px] font-medium text-zinc-400 mb-2">生成数量</div>
            <div className="grid grid-cols-3 gap-2">
              {[1, 2, 4].map((count) => {
                const isActive = videoCount === count;
                return (
                  <button
                    key={count}
                    type="button"
                    onClick={() => onVideoCountChange?.(count)}
                    className={`h-8 flex items-center justify-center rounded-xl text-xs font-medium transition-all cursor-pointer ${
                      isActive
                        ? "border border-white/90 bg-white/10 text-white shadow-sm"
                        : "border border-white/5 bg-zinc-800/40 text-zinc-400 hover:border-white/20 hover:text-zinc-200 hover:bg-zinc-800/80"
                    }`}
                  >
                    {count}个
                  </button>
                );
              })}
            </div>
          </div>
          </div>
          <div className="flex min-w-0 flex-col gap-3.5">
          {/* 加速（H3）：和工作台出片设置同一套档位，按链路给（首帧 / 参考 / 文生各自的默认不同） */}
          {choices.length > 0 && (
            <div>
              <div className="text-[11px] font-medium text-zinc-400 mb-2">加速</div>
              <div className="grid grid-cols-2 gap-1.5">
                {choices.map(({ id, tag }) => {
                  const q = qualityInfo(id);
                  const on = h3Quality === id;
                  return (
                    <button
                      key={id}
                      type="button"
                      title={q.lora}
                      onClick={() => onH3QualityChange?.(id)}
                      className={`flex flex-col items-start rounded-xl px-2.5 py-1.5 text-left transition-all ${
                        on
                          ? "border border-white/90 bg-white/10 text-white"
                          : "border border-white/5 bg-zinc-800/40 text-zinc-400 hover:border-white/20 hover:text-zinc-200"
                      }`}
                    >
                      <span className="text-[11.5px] font-medium">
                        {q.label}
                        {tag && <span className="ml-1 text-[10px] text-emerald-300/80">{tag}</span>}
                      </span>
                      <span className="text-[10px] leading-snug text-zinc-500">{q.hint}</span>
                    </button>
                  );
                })}
              </div>
              {h3Quality && <p className="mt-1.5 text-[10.5px] leading-snug text-zinc-500">{qualityInfo(h3Quality).lora}</p>}
            </div>
          )}

          {/* 采样方法（H3）：默认 euler；Combat 类 LoRA 作者推荐 res_multistep（打斗 A/B 的 E 组） */}
          {choices.length > 0 && onH3SamplingChange && (
            <div>
              <div className="text-[11px] font-medium text-zinc-400 mb-2">采样方法</div>
              <div className="grid grid-cols-2 gap-1.5">
                {H3_SAMPLINGS.map(({ id, label, hint }) => {
                  const on = (h3Sampling || "euler") === id;
                  return (
                    <button
                      key={id}
                      type="button"
                      onClick={() => onH3SamplingChange(id === "euler" ? "" : id)}
                      className={`flex flex-col items-start rounded-xl px-2.5 py-1.5 text-left transition-all ${
                        on
                          ? "border border-white/90 bg-white/10 text-white"
                          : "border border-white/5 bg-zinc-800/40 text-zinc-400 hover:border-white/20 hover:text-zinc-200"
                      }`}
                    >
                      <span className="text-[11.5px] font-medium">{label}</span>
                      <span className="text-[10px] leading-snug text-zinc-500">{hint}</span>
                    </button>
                  );
                })}
              </div>
            </div>
          )}

          {/* 4. 生成音频开关 */}
          <div className="flex items-center justify-between gap-3">
            <div className="flex items-center gap-1.5">
              <span className="text-[11px] font-medium text-zinc-400">生成音频</span>
              <span title="开启后模型自动合成环境拟音与音效；关闭则生成纯净无声视频">
                <HelpCircle className="h-3 w-3 text-zinc-500 hover:text-zinc-300 cursor-help" />
              </span>
            </div>
            <div className="flex shrink-0 gap-1">
              <button
                type="button"
                onClick={() => onGenerateAudioChange?.(true)}
                className={`h-7 px-3 flex items-center justify-center rounded-lg text-xs font-medium transition-all cursor-pointer ${
                  generateAudio
                    ? "border border-white/90 bg-white/10 text-white shadow-sm"
                    : "border border-white/5 bg-zinc-800/40 text-zinc-400 hover:border-white/20 hover:text-zinc-200 hover:bg-zinc-800/80"
                }`}
              >
                开启
              </button>
              <button
                type="button"
                onClick={() => onGenerateAudioChange?.(false)}
                className={`h-7 px-3 flex items-center justify-center rounded-lg text-xs font-medium transition-all cursor-pointer ${
                  !generateAudio
                    ? "border border-white/90 bg-white/10 text-white shadow-sm"
                    : "border border-white/5 bg-zinc-800/40 text-zinc-400 hover:border-white/20 hover:text-zinc-200 hover:bg-zinc-800/80"
                }`}
              >
                关闭
              </button>
            </div>
          </div>

          {/* H3 官方格式（2026-10-06）：出片前按出片方式套官方结构（三字段 / 对齐指令）、规整时间写法和画面文字引号。
              **只动格式，正文一个字不改**；默认关 —— 会改变已验证过的模板实际发给 H3 的内容，先对照再开 */}
          {engine === "h3" && onH3OfficialFormatChange && (
            <div className="flex items-center justify-between gap-3">
              <div className="flex items-center gap-1.5">
                <span className="text-[11px] font-medium text-zinc-400">H3 官方格式</span>
                <span title="出片前按 MiniMax H3 官方规范整理格式：文生三字段；首帧 / 首尾帧加官方对齐说明；镜头内的 At 00:SS 换成 Around the N-second mark；画面文字「」换成英文双引号。只动格式，正文不改。参考生视频（六段）代码编不出来，只整理时间和引号">
                  <HelpCircle className="h-3 w-3 text-zinc-500 hover:text-zinc-300 cursor-help" />
                </span>
              </div>
              <div className="flex shrink-0 gap-1">
                {[true, false].map((on) => (
                  <button
                    key={String(on)}
                    type="button"
                    onClick={() => onH3OfficialFormatChange(on)}
                    className={`h-7 px-3 flex items-center justify-center rounded-lg text-xs font-medium transition-all cursor-pointer ${
                      h3OfficialFormat === on
                        ? "border border-white/90 bg-white/10 text-white shadow-sm"
                        : "border border-white/5 bg-zinc-800/40 text-zinc-400 hover:border-white/20 hover:text-zinc-200 hover:bg-zinc-800/80"
                    }`}
                  >
                    {on ? "整理" : "原样发送"}
                  </button>
                ))}
              </div>
            </div>
          )}

          {/* 长镜头：以前是面板里常驻的一张卡（用户 2026-09-24：「长镜头这个卡片放到下面设置里」）。
              开了之后窗口编辑还在面板里 —— 每个窗口要写提示词，只能放那儿 */}
          {onLongTakeChange && (
            <div className="flex items-center justify-between gap-3">
              <div className="flex items-center gap-1.5">
                <span className="text-[11px] font-medium text-zinc-400">长镜头</span>
                <span title="一段超过 15 秒才需要：分成几个等长窗口，一次提交、出一个 MP4。15 秒以内直接出片就行，拆窗口只会白烧重叠帧">
                  <HelpCircle className="h-3 w-3 text-zinc-500 hover:text-zinc-300 cursor-help" />
                </span>
              </div>
              <div className="flex shrink-0 gap-1">
                {([true, false] as const).map((on) => (
                  <button key={String(on)} type="button" onClick={() => onLongTakeChange(on)}
                    className={`h-7 px-3 flex items-center justify-center rounded-lg text-xs font-medium transition-all cursor-pointer ${
                      Boolean(longTake) === on
                        ? "border border-white/90 bg-white/10 text-white shadow-sm"
                        : "border border-white/5 bg-zinc-800/40 text-zinc-400 hover:border-white/20 hover:text-zinc-200 hover:bg-zinc-800/80"
                    }`}>
                    {on ? "开启" : "关闭"}
                  </button>
                ))}
              </div>
            </div>
          )}

          </div>
        </div>
      )}
    </div>
  );
}
