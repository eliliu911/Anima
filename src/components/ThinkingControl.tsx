import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { useChat } from "../store/chat";
import type { ThinkingEffort } from "../api";
import { controlShape } from "./thinkingShape";
import { STAR_LAYERS, starsToBackground } from "./starfield";

/**
 * 思考档位控件：对标 Claude 3.7 / Sonnet 4.0 官方 Effort 交互与视觉规范。
 *
 * 视觉升级特性：
 * 1. 标题与状态：Effort / 思考 + 动态高亮状态（Ultracode / High 带有 Claude 标志性紫青辉光）。
 * 2. 进度轨道：
 *    - 常规档位（Low / Medium / High）：从灰色至渐进激活的平滑指示条与细腻刻度圆点；
 *    - 最高级 Turbo / Ultracode 档位：全息像素微粒子流光（Animated Pixel Matrix Gradient）与边缘呼吸辉光特效。
 * 3. 滑块（Thumb）：纯白微悬浮胶囊，带有精致边框与柔和投影（Shadow-md），拖动时伴随弹簧过冲（Spring Easing）。
 * 4. 底栏触发器（Trigger）：呈现当前档位状态并附带呼吸动效微光标。
 */

const LABEL: Record<ThinkingEffort, string> = {
  off: "Off",
  low: "Low",
  medium: "Medium",
  high: "High",
  auto: "Ultracode",
};

const HINT: Record<ThinkingEffort, string> = {
  off: "不思考，最快响应速度",
  low: "轻度思考（Low Effort），平衡速度与轻量逻辑",
  medium: "深度思考（Medium Effort），处理复杂分析与代码重构",
  high: "极高思考（High Effort），极致逻辑链推演",
  auto: "Turbo / Ultracode 极限模式，释放模型满血推理潜能",
};

export function ThinkingControl() {
  const { t } = useTranslation(["chat", "common"]);
  const config = useChat((s) => s.config);
  const tier = useChat((s) => s.tier);
  const setThinkingEffort = useChat((s) => s.setThinkingEffort);

  const [open, setOpen] = useState(false);
  const rootRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!rootRef.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const providerId =
    tier === "realtime" ? config?.realtime_provider : config?.background_provider;
  const provider = config?.providers.find((p) => p.id === providerId);
  const levels = provider?.thinking_levels ?? [];
  const current: ThinkingEffort =
    (tier === "realtime" ? config?.thinking?.realtime : config?.thinking?.background) ??
    "auto";

  const shape = controlShape(levels);
  if (!config || shape === "none") return null;

  const tierName = tier === "realtime" ? "实时" : "后台";
  const isTurbo = current === "high" || (current === "auto" && levels.includes("high"));

  // 二态开关（仅有开/关）
  if (shape === "toggle") {
    const on = current !== "off";
    return (
      <button
        type="button"
        className={`h-7 px-2.5 rounded-lg flex items-center gap-1.5 text-[11.5px] font-medium transition-all cursor-pointer border ${
          on
            ? "bg-[var(--surface-2)]/80 text-[var(--ink)] border-[var(--line)]/70 shadow-2xs"
            : "text-[var(--ink-3)] bg-transparent border-transparent hover:bg-[var(--surface-2)] hover:text-[var(--ink-2)]"
        }`}
        title={`${tierName}链路 · ${provider?.model ?? ""}\n${
          on ? "思考已开启：质量优先" : "思考已关闭：出声更快"
        }`}
        onClick={() => void setThinkingEffort(on ? "off" : "auto")}
      >
        <span
          className={`w-1.5 h-1.5 rounded-full transition-colors ${
            on ? "bg-[#8b5cf6]" : "bg-[var(--ink-3)]/40"
          }`}
        />
        <span>{t("thinking", { defaultValue: "思考" })}</span>
      </button>
    );
  }

  // 判定当前显示的 Label
  const displayLabel = levels.includes(current) ? LABEL[current] : LABEL.auto;

  return (
    <div ref={rootRef} className="relative shrink-0">
      {/* 底部触发器：对标 Claude 设计，固定最小宽度或左对齐保证稳定性 */}
      <button
        type="button"
        aria-haspopup="dialog"
        aria-expanded={open}
        className={`h-7 px-2.5 rounded-lg flex items-center gap-1.5 text-[11.5px] font-medium transition-all cursor-pointer border select-none ${
          open
            ? "bg-[var(--surface-2)]/80 text-[var(--ink)] border-[var(--line)]/70 shadow-xs"
            : "text-[var(--ink-2)] bg-transparent border-transparent hover:bg-[var(--surface-2)] hover:text-[var(--ink)]"
        }`}
        title={`${tierName}链路 · ${provider?.model ?? ""}`}
        onClick={() => setOpen((v) => !v)}
      >
        <span className="text-[var(--ink-3)] font-normal">Effort</span>
        <span
          key={current}
          className={`font-semibold inline-block transition-colors ${
            isTurbo
              ? "text-[#7c3aed] dark:text-[#a78bfa] drop-shadow-[0_0_8px_rgba(124,58,237,0.3)]"
              : "text-[var(--ink)]"
          }`}
        >
          {displayLabel}
        </span>
        {isTurbo && (
          <span className="w-1.5 h-1.5 rounded-full bg-[#7c3aed] animate-pulse shrink-0" />
        )}
      </button>

      {/* Claude 风格浮层卡片：使用 left-0 左对齐锚定，避免底部文字变长时向左挤压抖动 */}
      {open && (
        <div
          role="dialog"
          aria-label="思考档位"
          className="absolute bottom-full left-0 mb-2.5 w-[268px] rounded-2xl border border-[var(--line)]/80 bg-[var(--paper)] shadow-[0_12px_36px_rgba(0,0,0,0.12),0_4px_12px_rgba(0,0,0,0.06)] dark:shadow-[0_16px_40px_rgba(0,0,0,0.45)] z-50 px-4 py-3.5 select-none animate-in fade-in zoom-in-95 duration-150 backdrop-blur-md"
        >
          {/* 顶栏：Effort + 当前级别状态 + 提示问号 */}
          <div className="flex items-center justify-between mb-3">
            <div className="flex items-center gap-1.5">
              <span className="text-[12px] text-[var(--ink-3)] font-medium">Effort</span>
              <span
                key={current}
                className={`text-[13px] font-bold tracking-tight inline-block animate-in fade-in slide-in-from-bottom-1 duration-200 ${
                  isTurbo
                    ? "bg-gradient-to-r from-[#6366f1] via-[#8b5cf6] to-[#ec4899] bg-clip-text text-transparent drop-shadow-[0_0_12px_rgba(139,92,246,0.35)]"
                    : "text-[var(--ink)]"
                }`}
              >
                {displayLabel}
              </span>
            </div>

            {/* 提示问号徽标 */}
            <span
              className="w-4 h-4 rounded-full border border-[var(--line)] text-[10px] text-[var(--ink-3)] hover:text-[var(--ink)] hover:border-[var(--ink-3)] transition-colors flex items-center justify-center cursor-help select-none shrink-0"
              title={`${provider?.model ?? ""}\n\n${HINT[current]}\n\n当前档位自动同步至 ${tierName} 链路。`}
            >
              ?
            </span>
          </div>

          {/* 辅文字：Faster —— Smarter */}
          <div className="flex items-center justify-between mb-2 text-[10.5px] font-medium text-[var(--ink-3)]">
            <span>Faster</span>
            <span>Smarter</span>
          </div>

          {/* 核心拟物/流光滑杆组件 */}
          <EffortSlider
            levels={levels}
            current={current}
            onPick={(l) => void setThinkingEffort(l)}
          />
        </div>
      )}
    </div>
  );
}



/** 三层星空各自的闪烁周期，互不通约（4.1 / 5.3 / 6.7 秒，质数 ÷ 10）。 */
const TWINKLE = ["twinkle-d", "twinkle-e", "twinkle-f"] as const;

/// 轨道两端为滑块留出的距离 = 滑块半宽 + 1px 缝。
///
/// 滑块 20px 宽 → 10 + 1 = 11。**那 1px 不是留白，是几何要求**：
/// 槽 22px 高、圆角 9，滑块 20px、圆角 8 —— 两个圆角矩形只有在
/// 「四边各内缩 1px、圆角小 1px」时才是同心的，处处等距。
///
/// 早先滑块 19px / 圆角 7 / 内缩 10：在滑块上边缘那个高度上，槽的右边界
/// 比滑块的右边界还往外约 2.6px，于是端点处露出一小片深色槽底 ——
/// 看起来就像"滑块右侧有块阴影"，其实是圆角没对上。
const TRACK_INSET = 11;

/**
 * Claude 官方级精细滑杆（含 Turbo 像素矩阵流光与双色刻度）
 */
function EffortSlider({
  levels,
  current,
  onPick,
}: {
  levels: ThinkingEffort[];
  current: ThinkingEffort;
  onPick: (l: ThinkingEffort) => void;
}) {
  const stops: ThinkingEffort[] = levels.filter((l) => l !== "auto");

  const trackRef = useRef<HTMLDivElement>(null);
  const [pressed, setPressed] = useState(false);
  const index = Math.max(0, stops.indexOf(current));

  const pickAt = useCallback(
    (clientX: number) => {
      const el = trackRef.current;
      if (!el || stops.length === 0) return;
      const r = el.getBoundingClientRect();
      const usable = Math.max(r.width - TRACK_INSET * 2, 1);
      const ratio = (clientX - r.left - TRACK_INSET) / usable;
      const i = Math.round(ratio * (stops.length - 1));
      const clamped = Math.min(stops.length - 1, Math.max(0, i));
      if (stops[clamped] !== current) onPick(stops[clamped]);
    },
    [stops, current, onPick],
  );

  useEffect(() => {
    if (!pressed) return;
    const onMove = (e: MouseEvent) => pickAt(e.clientX);
    const onUp = () => setPressed(false);
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
  }, [pressed, pickAt]);

  const pct = stops.length > 1 ? (index / (stops.length - 1)) * 100 : 0;
  const isMaxLevel = index === stops.length - 1 && stops.length > 1;

  return (
    <div
      ref={trackRef}
      role="slider"
      tabIndex={0}
      aria-valuemin={0}
      aria-valuemax={stops.length - 1}
      aria-valuenow={index}
      aria-valuetext={LABEL[current]}
      className="relative h-[28px] cursor-pointer rounded-[11px] outline-none select-none focus-visible:ring-1 focus-visible:ring-[#8b5cf6]"
      onMouseDown={(e) => {
        e.preventDefault();
        setPressed(true);
        pickAt(e.clientX);
      }}
      onKeyDown={(e) => {
        const d = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
        if (!d) return;
        e.preventDefault();
        const next = Math.min(stops.length - 1, Math.max(0, index + d));
        if (stops[next] !== current) onPick(stops[next]);
      }}
    >
      {/* 1. 轨道底槽背景 */}
      <div className="absolute inset-x-0 top-1/2 -translate-y-1/2 h-[22px] rounded-[9px] bg-[#e5e7eb]/80 dark:bg-[#20242a] border border-[#d1d5db]/50 dark:border-[#2f3540] overflow-hidden">
        {/* 2. 激活进度填充：宽度必须与刻度起点 TRACK_INSET 精确对齐，
            止步于当前滑块中心点，绝不向右多冒出任何灰色残影。 */}
        <div
          className="absolute inset-y-0 left-0 transition-[width] duration-200 ease-spring rounded-l-[9px]"
          style={{ width: `calc(${TRACK_INSET}px + (100% - ${TRACK_INSET * 2}px) * ${pct / 100})` }}
        >
          {/* 非最大档位：柔和中性进度色 */}
          <div className="absolute inset-0 bg-[#d1d5db]/60 dark:bg-[#374151]/60" />

          {/* 3. 最高档专属：点阵微光。
              CSS 的 background-image 点阵**没法让单个点各自动画** ——
              一层一个 opacity，那层所有点必然齐闪，看起来是"一整片在呼吸"
              而不是星芒。所以靠两件事伪造随机（详见 theme.css 的注释）：

                互质格距   7 / 9 / 11 / 13 / 17，两两互质 ——
                           在可见宽度内不会重合成规律。
                           早先是 6 / 6 / 12，12 是 6 的倍数，
                           三层叠起来是完全规则的点阵。
                互不通约周期 1.7 / 2.3 / 3.1 / 4.1 / 5.3 秒（质数 ÷ 10），
                           公倍数长到几分钟，抓不到整体节拍。

              再加一层很宽很淡的横扫高光：点仍然原地闪（位移会有摩擦感），
              但整体有了流动方向。 */}
          {isMaxLevel && (
            <div className="absolute inset-0 overflow-hidden bg-gradient-to-r from-transparent via-[#a78bfa]/40 to-[#818cf8]/70 dark:from-transparent dark:via-[#6366f1]/50 dark:to-[#8b5cf6]/80">
              {STAR_LAYERS.map((layer, i) => (
                <div
                  key={i}
                  className={`absolute inset-0 ${TWINKLE[i]}`}
                  style={{ backgroundImage: starsToBackground(layer) }}
                />
              ))}
              {/* 基准必须是 left-0：keyframes 里的 222% 是按「元素宽 45%、
                  基准 left:0」算出来的（222 ≈ 100/45）。原来写 -left-1/2，
                  等于把整条轨迹左移了半个容器 —— 高光在容器 22% 处出现、
                  往左跑出去，看起来就是"中间往左扫"而不是"右边扫进来"。 */}
              <div className="absolute inset-y-0 left-0 w-[45%] shimmer-sweep bg-gradient-to-r from-transparent via-white/25 to-transparent" />
            </div>
          )}
        </div>
      </div>

      {/* 4. 刻度圆点与滑块交互层 */}
      <div className="absolute inset-y-0" style={{ left: TRACK_INSET, right: TRACK_INSET }}>
        {/* 各档位刻度点 */}
        {stops.map((l, i) => {
          const isStopActive = i <= index;
          const isTurboStop = i === stops.length - 1;
          return (
            <span
              key={l}
              className={`absolute top-1/2 w-[3.5px] h-[3.5px] rounded-full -translate-x-1/2 -translate-y-1/2 transition-colors duration-200 ${
                isTurboStop
                  ? "bg-[#6366f1] dark:bg-[#818cf8] shadow-[0_0_4px_rgba(99,102,241,0.8)]"
                  : isStopActive
                  ? "bg-[#6b7280] dark:bg-[#9ca3af]"
                  : "bg-[#9ca3af]/40 dark:bg-[#4b5563]/60"
              }`}
              style={{ left: `${(i / Math.max(stops.length - 1, 1)) * 100}%` }}
            />
          );
        })}

        {/* 5. 纯白高精悬浮滑块（Thumb） */}
        <span
          className="absolute top-1/2 -translate-x-1/2 -translate-y-1/2 pointer-events-none transition-[left] duration-200 ease-spring"
          style={{ left: `${pct}%` }}
        >
          {/* 高度 20 对 22 的槽：**只留 1px 缝**。
              早先是 16 对 22，上下各露 3px 轨道，再叠上滑块自己的 border
              和两层阴影 —— 那圈软边和露出的轨道混在一起，看起来就是
              "边缘脏、过渡色不干净"。border 也去掉：它正好落在槽壁上，
              等于把缝画粗一倍；靠阴影分层就够了。

              阴影用**负扩散**（`-1px`）：正常的 `0 1px 3px` 会向四周晕开，
              在滑块左右两侧糊出一圈灰边 —— 轨道本来就浅，这圈灰边看起来
              就是"滑块边上有脏东西"。负扩散把阴影往里收，只在下方留一线。

              按下的缩放从 1.1 降到 1.05：20px 放大 10% 是 22px，
              正好等于槽高，会顶破槽壁。 */}
          <span
            className={`block w-[20px] h-[20px] rounded-[8px] bg-white dark:bg-[#f3f4f6] shadow-[0_1px_2px_-1px_rgba(0,0,0,0.35)] transition-all duration-150 ease-out ${
              pressed ? "scale-105 shadow-[0_2px_4px_-1px_rgba(0,0,0,0.4)]" : "scale-100"
            } ${
              isMaxLevel
                ? "shadow-[0_0_8px_rgba(139,92,246,0.5),0_1px_4px_rgba(0,0,0,0.2)]"
                : ""
            }`}
          />
        </span>
      </div>
    </div>
  );
}

