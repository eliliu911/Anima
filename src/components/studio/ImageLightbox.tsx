import { useCallback, useEffect } from "react";
import { MediaThumb } from "./MediaThumb";

/**
 * 看大图。整屏，← → 翻页，Esc 关。
 *
 * # 为什么必须有
 *
 * 设定卡是**一张图里塞了四五个视图**（大头特写 + 三个全身）。
 * 缩到 140px 的方格里，脸只剩十几个像素 —— 而选哪一张当封面，
 * 判据恰恰是「这张脸对不对」。看不全就等于在瞎选。
 *
 * # 这里用整屏遮罩是对的
 *
 * `useDismissable` 的注释说别用整屏遮罩，那条针对的是**菜单**：
 * 菜单浮在触发它的元素旁边，插一层遮罩会把它的 `:hover` 吃掉、
 * 卡片反复闪。看大图本来就该盖住整屏、本来就该拦住底下的一切。
 */
export function ImageLightbox({
  paths,
  index,
  onIndex,
  onClose,
  caption,
  actions,
}: {
  /** 本机路径或 `anima-media://` URL */
  paths: string[];
  index: number;
  onIndex: (i: number) => void;
  onClose: () => void;
  caption?: string;
  /** 底部动作区（设为封面 / 收藏之类）。 */
  actions?: React.ReactNode;
}) {
  const step = useCallback(
    (d: number) => onIndex(Math.min(paths.length - 1, Math.max(0, index + d))),
    [index, paths.length, onIndex],
  );

  const onKey = useCallback(
    (e: KeyboardEvent) => {
      if (e.key === "Escape") return onClose();
      if (e.key === "ArrowLeft") return step(-1);
      if (e.key === "ArrowRight") return step(1);
    },
    [onClose, step],
  );

  useEffect(() => {
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onKey]);

  const src = paths[index];
  if (!src) return null;

  return (
    <div
      role="dialog"
      aria-modal="true"
      // **比工作台更靠上。** 它是从工作台里打开的，压在下面就白开了。
      className="fixed inset-0 z-[70] flex flex-col backdrop-blur-md"
      style={{ background: "color-mix(in srgb, var(--studio-bg-base) 86%, black)" }}
      onClick={onClose}
    >
      <div className="flex shrink-0 items-center justify-between px-5 py-3">
        <span
          className="font-mono text-[11px] tabular-nums"
          style={{ color: "var(--studio-text-muted)" }}
        >
          {index + 1} / {paths.length}
        </span>
        <button
          type="button"
          onClick={onClose}
          className="rounded-lg px-3 py-1.5 text-[12px]"
          style={{ background: "var(--studio-bg-inset)", color: "var(--studio-text-2)" }}
        >
          关闭 · Esc
        </button>
      </div>

      <div className="flex min-h-0 flex-1 items-center gap-3 px-3">
        <Chevron dir="prev" disabled={index === 0} onClick={() => step(-1)} />
        {/* 点图本身不关 —— 想凑近看细节的时候误关最烦人。点周围空白才关。 */}
        <div
          className="flex min-h-0 min-w-0 flex-1 items-center justify-center"
          onClick={(e) => e.stopPropagation()}
        >
          <MediaThumb
            key={src}
            path={src}
            alt={caption ?? ""}
            className="max-h-full max-w-full object-contain"
          />
        </div>
        <Chevron dir="next" disabled={index === paths.length - 1} onClick={() => step(1)} />
      </div>

      <div
        className="flex shrink-0 items-center justify-center gap-3 px-6 pb-5 pt-3"
        onClick={(e) => e.stopPropagation()}
      >
        {caption && (
          <span className="text-[11.5px]" style={{ color: "var(--studio-text-2)" }}>
            {caption}
          </span>
        )}
        {actions}
      </div>
    </div>
  );
}

function Chevron({
  dir,
  disabled,
  onClick,
}: {
  dir: "prev" | "next";
  disabled: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      aria-label={dir === "prev" ? "上一张" : "下一张"}
      onClick={(e) => {
        e.stopPropagation();
        onClick();
      }}
      className="shrink-0 rounded-full px-3 py-6 text-[18px] leading-none"
      style={{
        background: "var(--studio-bg-elevated)",
        color: "var(--studio-text-2)",
        opacity: disabled ? 0.22 : 1,
        cursor: disabled ? "default" : "pointer",
      }}
    >
      {dir === "prev" ? "‹" : "›"}
    </button>
  );
}
