import { useTranslation } from "react-i18next";
import type { EngineSpec } from "../../api/studio";
import { useStudioStore } from "./useStudioStore";
import { estimateSecs } from "./estimateSecs";
import {
  clampDenoise,
  DENOISE_DEFAULT,
  DENOISE_MAX,
  DENOISE_MIN,
  DENOISE_STEP,
  denoiseHint,
  fmtDenoise,
  usesDenoise,
} from "./denoise";

/**
 * 参数区：尺寸 / 时长 / 档位 / 批量 / 种子。
 *
 * **每一项都只在这个引擎真的有这个维度时才出现。** 出图没有时长，
 * 只有 H3 有采样档位，视频不暴露步数。控件在但选项是空的，
 * 比没有控件更让人困惑。
 */
export function ParamsPanel({ engine }: { engine: EngineSpec }) {
  const { t } = useTranslation(["studio", "common"]);
  // **每个字段单独选。** zustand v5 去掉了默认浅比较：返回新对象的 selector
  // 会让每次渲染都判定为「变了」，直接无限循环
  // （"Maximum update depth exceeded"，2026-09-04 踩到）。
  const size = useStudioStore((s) => s.size);
  const batch = useStudioStore((s) => s.batch);
  const duration = useStudioStore((s) => s.duration);
  const seed = useStudioStore((s) => s.seed);
  const denoise = useStudioStore((s) => s.denoise);
  const mode = useStudioStore((s) => s.mode);
  const setSize = useStudioStore((s) => s.setSize);
  const setBatch = useStudioStore((s) => s.setBatch);
  const setDuration = useStudioStore((s) => s.setDuration);
  const setSeed = useStudioStore((s) => s.setSeed);
  const setDenoise = useStudioStore((s) => s.setDenoise);

  const label = (k: string, d: string) => t(`studio:${k}`, { defaultValue: d });

  return (
    <div className="flex flex-col gap-3">
      {/* 尺寸 */}
      <Row title={engine.kind === "video" ? label("resolution", "分辨率") : label("size", "图像尺寸")}>
        <select
          className="studio-input w-full px-2 py-1.5 text-[12px]"
          value={size ? `${size.width}x${size.height}` : ""}
          onChange={(e) => {
            const [w, h] = e.target.value.split("x").map(Number);
            setSize({ width: w, height: h });
          }}
        >
          {engine.sizes.map((s) => (
            <option key={`${s.width}x${s.height}`} value={`${s.width}x${s.height}`}>
              {/* **代价要在点下去之前看得见。** 1080p 出片 5 秒要 14 分钟、
                  吃 104 GB，让人点完等着才发现是最糟的形式。
                  没跑过的也标出来 —— 「验过」和「没验过」是两种东西。 */}
              {s.label}
              {s.secs !== null ? ` · ~${s.secs}s` : ""}
              {s.peak_gb !== null ? ` · ${s.peak_gb.toFixed(0)}GB` : ""}
              {s.verified ? "" : " · 未验"}
            </option>
          ))}
        </select>
      </Row>

      {/* 时长：只有视频有。**数字步进而不是固定几档** ——
          两个引擎的合法范围不一样（LTX 1~20 步进 0.5，H3 2.5~15），
          固定 5/10/15/20 在 H3 身上没有意义。 */}
      {engine.duration && (
        <Row title={label("duration", "时长")}>
          <DurationStepper
            spec={engine.duration}
            value={duration ?? engine.duration.default}
            onChange={setDuration}
          />
        </Row>
      )}

      {/* 批量：走客户端队列串行发，不是图里的 batch_size */}
      <Row title={label("batch", "生成数量")}>
        <div className="studio-seg">
          {Array.from({ length: engine.max_batch }, (_, i) => i + 1).map((n) => (
            <button key={n} type="button" data-active={batch === n} onClick={() => setBatch(n)}>
              ×{n}
            </button>
          ))}
        </div>
      </Row>

      {/* **重绘幅度只在图生图出现。** 它是那条链路唯一真正的旋钮，
          而以前界面上没有出口 —— 后端一直用默认的 0.65，等于只有一档。 */}
      {usesDenoise(mode) && (
        <Row title={label("denoise", "重绘幅度")}>
          <div className="flex flex-col gap-1">
            <div className="flex items-center gap-2">
              <input
                type="range"
                className="flex-1"
                min={DENOISE_MIN}
                max={DENOISE_MAX}
                step={DENOISE_STEP}
                value={denoise ?? DENOISE_DEFAULT}
                onChange={(e) => setDenoise(clampDenoise(Number(e.target.value)))}
              />
              <span
                className="w-[38px] shrink-0 text-right font-mono text-[11px] tabular-nums"
                style={{ color: "var(--studio-text-2)" }}
              >
                {fmtDenoise(denoise ?? DENOISE_DEFAULT)}
              </span>
            </div>
            {/* **说会发生什么，不说「强 / 弱」。** 后者没有信息量。 */}
            <p className="text-[10px] leading-relaxed" style={{ color: "var(--studio-text-muted)" }}>
              {denoiseHint(denoise ?? DENOISE_DEFAULT)}
            </p>
          </div>
        </Row>
      )}

      <Row title="SEED">
        <input
          className="studio-input w-full px-2 py-1.5 text-[12px]"
          placeholder={label("seedRandom", "留空 = 随机")}
          value={seed ?? ""}
          inputMode="numeric"
          onChange={(e) => {
            const v = e.target.value.replace(/\D/g, "");
            setSeed(v === "" ? null : Number(v));
          }}
        />
      </Row>

      {/* 会分段的引擎要说清楚 —— 分段后不单独写提示词，场面会一段一个样 */}
      {engine.duration?.note && (
        <p className="text-[10px] leading-relaxed" style={{ color: "var(--studio-text-muted)" }}>
          {engine.duration.note}
        </p>
      )}

      <div
        className="flex items-center justify-between font-mono text-[0.625rem]"
        style={{ color: "var(--studio-text-muted)" }}
      >
        <span>
          {label("estimate", "预计")} ~{estimateSecs(engine, size, duration, batch)}s
        </span>
        {/* 步数只在出图那四个上出现；视频由预设内部决定 */}
        {engine.steps !== null && <span>{engine.steps} steps</span>}
      </div>
    </div>
  );
}

/**
 * 时长步进器。照搬 LumenX 的 `−  5s  +`，两侧标出范围。
 *
 * **分段的引擎要显示会切成几段。** LTX 超过 5 秒就是多段接续，
 * 而多段的连贯性要靠每段单独写提示词 —— 人得先知道自己踩进了多段模式。
 */
function DurationStepper({
  spec,
  value,
  onChange,
}: {
  spec: NonNullable<EngineSpec["duration"]>;
  value: number;
  onChange: (v: number) => void;
}) {
  const clamp = (v: number) =>
    Math.round(Math.min(Math.max(v, spec.min), spec.max) / spec.step) * spec.step;
  const windows = spec.window_secs ? Math.ceil(value / spec.window_secs) : 1;
  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center gap-2">
        <button
          type="button"
          className="studio-input h-8 w-8 shrink-0 text-[14px]"
          disabled={value <= spec.min}
          onClick={() => onChange(clamp(value - spec.step))}
        >
          −
        </button>
        <input
          className="studio-input min-w-0 flex-1 px-2 py-1.5 text-center text-[13px]"
          value={value}
          inputMode="decimal"
          onChange={(e) => {
            const v = Number(e.target.value.replace(/[^\d.]/g, ""));
            if (!Number.isNaN(v)) onChange(clamp(v));
          }}
        />
        <span className="font-mono text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
          s
        </span>
        <button
          type="button"
          className="studio-input h-8 w-8 shrink-0 text-[14px]"
          disabled={value >= spec.max}
          onClick={() => onChange(clamp(value + spec.step))}
        >
          +
        </button>
      </div>
      <div
        className="flex justify-between font-mono text-[0.5625rem]"
        style={{ color: "var(--studio-text-muted)" }}
      >
        <span>{spec.min}s</span>
        {windows > 1 && (
          <span style={{ color: "var(--studio-running-fg)" }}>
            {windows} 段接续
          </span>
        )}
        <span>{spec.max}s</span>
      </div>
    </div>
  );
}

function Row({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-col gap-1.5">
      <span
        className="font-mono text-[0.5625rem] uppercase tracking-[0.14em]"
        style={{ color: "var(--studio-text-muted)" }}
      >
        {title}
      </span>
      {children}
    </div>
  );
}
