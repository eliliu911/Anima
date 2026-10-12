import { useEffect, useRef } from "react";
import { useVoice } from "../store/voice";

/**
 * 麦克风电平指示：像系统的麦克风组件那样，**说话时从下往上填充**。
 *
 * ## 为什么抽成共用组件
 *
 * 角色层和对话窗各是一个独立 webview，各订阅一份 `anima://voice`。
 * 两边各写一份画法的话，迟早出现"对话窗的条子在跳、她身上那个不跳"——
 * 而那种不一致看起来像 bug，实际只是两处代码不同步。
 *
 * ## 为什么不用 React state 驱动每一帧
 *
 * 电平每 30ms 来一次。用 `useState` 的话每帧都会重渲染宿主组件
 * （输入框、按钮、整条 CommandBar 全跟着重算）。这里直接写 DOM 的
 * `height`，React 那边一帧都不用醒。
 */

/** 电平到填充高度的放大倍数。人声的 rms 通常在 0.02~0.15 之间 */
const GAIN = 16;
/** 低于这个电平算没在说话，走待机呼吸 */
const FLOOR = 0.04;

export interface MicFillProps {
  /** `input` = 我说话的电平，`output` = 她说话的电平 */
  source?: "input" | "output";
  /** 关着的时候不填充，只显示图标 */
  active: boolean;
  /** 填充色。默认青瓷 */
  color?: string;
  className?: string;
}

/**
 * 麦克风图标 + 从下往上的电平填充。
 *
 * 关着时显示带斜杠的图标 —— 电平在关闭状态下是死的，
 * 留着一个不动的填充只会让人以为它坏了。
 */
export function MicFill({ source = "input", active, color, className }: MicFillProps) {
  const fillRef = useRef<HTMLSpanElement>(null);
  const iconRef = useRef<SVGSVGElement>(null);
  const levelRef = useRef(0);
  levelRef.current = useVoice((s) => (source === "input" ? s.level : s.outputLevel));

  useEffect(() => {
    if (!active) {
      if (fillRef.current) fillRef.current.style.height = "0%";
      return;
    }
    let raf = 0;
    let shown = 0;
    const start = performance.now();
    const tick = () => {
      const lv = Math.min(1, Math.max(0, levelRef.current * GAIN));
      // 待机时给一点呼吸，让"开着但没人说话"和"关着"看起来不一样
      const t = (performance.now() - start) / 1000;
      const target = lv > FLOOR ? 16 + lv * 84 : 12 + 5 * Math.sin(t * 3);
      // **要平滑**。直接用原始 rms 的话填充在抖，看起来像故障而不是在说话
      shown += (target - shown) * 0.3;

      if (fillRef.current) fillRef.current.style.height = `${shown}%`;
      // 填过图标一半时把图标转成反白，否则填充盖上去就看不清了
      if (iconRef.current) {
        iconRef.current.style.color = shown > 45 ? "#fff" : (color ?? "var(--celadon)");
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [active, color]);

  return (
    <span className={`relative overflow-hidden inline-flex items-center justify-center ${className ?? ""}`}>
      {active && (
        <span
          ref={fillRef}
          className="absolute bottom-0 left-0 right-0 pointer-events-none opacity-85"
          style={{ height: "0%", background: color ?? "var(--celadon)" }}
        />
      )}
      <svg
        ref={iconRef}
        width="15"
        height="15"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.8"
        strokeLinecap="round"
        className="relative z-10"
        aria-hidden="true"
      >
        <rect x="9" y="3" width="6" height="11" rx="3" />
        <path d="M5 11a7 7 0 0 0 14 0M12 18v3" />
        {!active && <path d="M4 4l16 16" />}
      </svg>
    </span>
  );
}

/** 几根条。奇数根时中间最高，形状才像声波 */
const BARS = 5;
const FALLOFF = [0.55, 0.85, 1, 0.85, 0.55];

/**
 * 横向的电平条。用在状态胶囊里当"她正在说"的背景 ——
 * **一颗静止的点说不出"正在说"，而波形本身就是那个信息**。
 */
export function LevelBars({
  source = "output",
  height = 13,
  className,
}: {
  source?: "input" | "output";
  height?: number;
  className?: string;
}) {
  const barsRef = useRef<HTMLSpanElement[]>([]);
  const levelRef = useRef(0);
  levelRef.current = useVoice((s) => (source === "input" ? s.level : s.outputLevel));

  useEffect(() => {
    let raf = 0;
    let shown = 0;
    const tick = () => {
      shown += (levelRef.current - shown) * 0.35;
      for (let i = 0; i < barsRef.current.length; i++) {
        const el = barsRef.current[i];
        if (!el) continue;
        // 给一个可见下限：静默时是"几乎平"而不是"没有"，
        // 后者看起来像组件没渲染出来
        el.style.transform = `scaleY(${Math.min(1, 0.14 + shown * GAIN * 0.4 * FALLOFF[i])})`;
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, []);

  return (
    <span
      className={`inline-flex items-center gap-[2px] ${className ?? ""}`}
      style={{ height }}
      aria-hidden="true"
    >
      {Array.from({ length: BARS }, (_, i) => (
        <span
          key={i}
          ref={(el) => {
            if (el) barsRef.current[i] = el;
          }}
          className="w-[2px] rounded-full bg-current origin-center"
          style={{ height, transform: "scaleY(0.14)" }}
        />
      ))}
    </span>
  );
}
