import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { MediaThumb } from "./MediaThumb";
import { useStudioStore, type StudioResult } from "./useStudioStore";
import { kindOf } from "./resultFilter";
import { formatTime } from "./ResultDetail";

/**
 * 一张结果卡。
 *
 * 照搬 LumenX 的 `playground/ResultCard`（MIT，Copyright 2026 Alibaba）：
 * 图上悬停浮出一排动作、图下一行是模型 + 模式 + 时间、点图进详情。
 *
 * # 「下载」换成「在访达中显示」
 *
 * LumenX 是网页，图在服务器上，所以要下载。我们是本机应用，
 * **文件本来就在磁盘上**（`.anima/generated/`），再"下载"一份到下载文件夹
 * 只会多出一个副本。真正的需求是「我要拿到这个文件」——
 * 在访达里选中它才是那件事。
 */
export function ResultCard({
  r,
  onOpen,
  onEnroll,
  onError,
}: {
  r: StudioResult;
  onOpen: () => void;
  onEnroll: () => void;
  onError: (msg: string) => void;
}) {
  const setMode = useStudioStore((s) => s.setMode);
  const setImage = useStudioStore((s) => s.setImage);
  const isVideo = kindOf(r) === "video";
  const ready = r.status === "done" && !!r.path;

  /** 拿它当参考图 —— 留在当前模式，只填输入槽。 */
  const useAsRef = () => {
    setImage(0, r.path!);
  };

  /** 拿它当首帧生成视频 —— 要连模式一起切，否则填了也不会用上。 */
  const toVideo = () => {
    setMode("i2v");
    setImage(0, r.path!);
  };

  return (
    <article className="studio-card group overflow-hidden">
      <div
        className="relative flex aspect-video items-center justify-center"
        style={{ background: "var(--studio-bg-inset)" }}
      >
        {ready ? (
          <MediaThumb
            path={r.path}
            alt={r.prompt.slice(0, 40)}
            kind={isVideo ? "video" : "image"}
            className="h-full w-full object-contain"
          />
        ) : (
          <span
            className="font-mono text-[0.625rem] uppercase tracking-[0.16em]"
            style={{ color: `var(--studio-${r.status}-fg)` }}
          >
            {r.status}
          </span>
        )}

        {/* **点图进详情。** 铺一层透明按钮而不是把 MediaThumb 包进 button ——
            视频要留出播放控件，包起来会把控件一起吞掉。 */}
        {ready && !isVideo && (
          <button
            type="button"
            onClick={onOpen}
            title="查看详情"
            aria-label="查看详情"
            className="absolute inset-0 cursor-zoom-in"
          />
        )}

        {/* 悬停浮出的动作条。**盖在图的下缘**，不遮主体。 */}
        {ready && (
          <div className="studio-media-bar pointer-events-none absolute inset-x-0 bottom-0 flex h-12 items-end justify-end gap-1.5 px-3 pb-2.5 opacity-0 transition-opacity group-hover:opacity-100">
            <IconButton
              label="在访达中显示"
              onClick={() =>
                revealItemInDir(r.path!).catch((e) => onError(`打开访达失败：${e}`))
              }
            >
              {/* 下载/取文件 */}
              <path d="M8 2v7m0 0L5.2 6.2M8 9l2.8-2.8M2.6 11.4v1.2c0 .6.5 1 1 1h8.8c.6 0 1-.4 1-1v-1.2" />
            </IconButton>
            {!isVideo && (
              <IconButton label="用作参考图" onClick={useAsRef}>
                <path d="M2.6 3.4h10.8v9.2H2.6zM2.6 10l3-3 2.4 2.4 2-2 3.4 3.4" />
              </IconButton>
            )}
            {!isVideo && (
              <IconButton label="用它生成视频" onClick={toVideo}>
                <path d="M2.4 4.6h7.2v6.8H2.4zM9.6 7.6l4-2v4.8l-4-2z" />
              </IconButton>
            )}
            {!isVideo && (
              <IconButton label="录入资产库" onClick={onEnroll}>
                <path d="M4.2 2.4h7.6v11.2L8 11l-3.8 2.6z" />
              </IconButton>
            )}
          </div>
        )}
      </div>

      <div className="flex flex-col gap-1.5 px-3 py-2.5">
        <p
          className="line-clamp-2 text-[11px] leading-relaxed"
          style={{ color: "var(--studio-text-2)" }}
        >
          {r.prompt}
        </p>

        {/* 失败原因原样带出来 —— 这条链路的失败几乎都是可操作的 */}
        {r.error && (
          <p className="text-[11px] leading-relaxed" style={{ color: "var(--studio-failed-fg)" }}>
            {r.error}
          </p>
        )}

        {/* **跑成功了但有话要说。** 最典型的是引擎回落：
            选了 Z-Image + 图生图，实际出的是 SDXL 的图 ——
            不说等于骗人。用告警色，不用失败色：图是好的。 */}
        {r.notes?.map((n) => (
          <p
            key={n}
            className="text-[11px] leading-relaxed"
            style={{ color: "var(--studio-running-fg)" }}
          >
            {n}
          </p>
        ))}

        {/* 自动加了激活词就说出来 —— 提示词被动过而人不知道，
            就是我们自己造的静默失败 */}
        {r.triggersAdded && r.triggersAdded.length > 0 && (
          <p className="font-mono text-[0.5625rem]" style={{ color: "var(--studio-primary)" }}>
            + {r.triggersAdded.join(" ")}
          </p>
        )}

        {/* 一行说清「哪个模型、什么模式、什么时候」——
            一屏几十张图之后，这三样是唯一能把它们分开的东西 */}
        <div className="flex flex-wrap items-center gap-1.5">
          <span
            className="rounded px-1.5 py-0.5 font-mono text-[0.5625rem]"
            style={{ background: "var(--studio-bg-inset)", color: "var(--studio-text-muted)" }}
          >
            {r.engineId}
          </span>
          <span
            className="rounded px-1.5 py-0.5 font-mono text-[0.5625rem] uppercase"
            style={{
              background: "color-mix(in srgb, var(--studio-primary) 12%, transparent)",
              color: "var(--studio-primary)",
            }}
          >
            {r.mode}
          </span>
          {r.seed !== undefined && (
            <span
              className="font-mono text-[0.5625rem]"
              style={{ color: "var(--studio-text-muted)" }}
            >
              seed {r.seed}
            </span>
          )}
          <span
            className="ml-auto font-mono text-[0.5625rem]"
            style={{ color: "var(--studio-text-muted)" }}
          >
            {formatTime(r.startedAt).slice(11, 16)}
          </span>
        </div>
      </div>
    </article>
  );
}

/**
 * 动作条上的一个图标按钮。
 *
 * `pointer-events-auto` 是必须的 —— 外面那层动作条设了
 * `pointer-events-none`，好让不在按钮上的部分把点击**穿透**给底下
 * 「进详情」的那层。不加的话整条动作条会挡住点图进详情。
 */
function IconButton({
  label,
  onClick,
  children,
}: {
  label: string;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      onClick={(e) => {
        e.stopPropagation();
        onClick();
      }}
      className="studio-media-btn pointer-events-auto grid h-7 w-7 place-items-center rounded-full transition-colors"
    >
      <svg
        width="16"
        height="16"
        viewBox="0 0 16 16"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.3"
        strokeLinecap="round"
        strokeLinejoin="round"
      >
        {children}
      </svg>
    </button>
  );
}
