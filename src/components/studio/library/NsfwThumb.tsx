import { MediaThumb } from "../MediaThumb";
import { shouldBlur, useNsfw } from "./nsfw";

/**
 * 可能是 NSFW 的缩略图：默认模糊，右上角眼睛按钮切换显示。
 *
 * `revealKey` 用形象 id 之类的稳定值 —— 同一张图在卡片和详情里是同一个开关状态。
 * 眼睛按钮 `stopPropagation`：卡片本身是按钮，点眼睛不能顺带把详情打开。
 */
export function NsfwThumb({
  path,
  nsfw,
  revealKey,
  className,
  compact = false,
}: {
  path: string | null | undefined;
  nsfw: boolean;
  revealKey: string;
  className?: string;
  /** 小缩略图：只模糊、不放眼睛按钮（太小点不准），显示一个 NSFW 小角标 */
  compact?: boolean;
}) {
  const state = useNsfw();
  const blur = shouldBlur(nsfw, revealKey, state);
  return (
    <span className="relative block h-full w-full overflow-hidden">
      <span className="block h-full w-full" style={blur ? { filter: "blur(14px) saturate(0.7)", transform: "scale(1.1)" } : undefined}>
        <MediaThumb path={path} className={className} />
      </span>
      {nsfw && compact && (
        <span className="absolute left-0.5 top-0.5 rounded px-1 font-mono text-[8px] leading-tight" style={{ background: "var(--studio-danger)", color: "var(--studio-on-primary)" }}>
          NSFW
        </span>
      )}
      {nsfw && !compact && (
        <span
          role="button"
          tabIndex={0}
          aria-label={blur ? "显示这张图" : "模糊这张图"}
          title={blur ? "显示" : "模糊"}
          onClick={(e) => {
            e.stopPropagation();
            e.preventDefault();
            state.toggle(revealKey);
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" || e.key === " ") {
              e.stopPropagation();
              e.preventDefault();
              state.toggle(revealKey);
            }
          }}
          className="absolute left-1.5 top-1.5 flex items-center gap-1 rounded-full px-1.5 py-0.5 text-[10px]"
          style={{ background: "var(--studio-danger)", color: "var(--studio-on-primary)" }}
        >
          <EyeIcon off={blur} />
          NSFW
        </span>
      )}
    </span>
  );
}

function EyeIcon({ off }: { off: boolean }) {
  return (
    <svg width="11" height="11" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M1.5 8S4 3.5 8 3.5 14.5 8 14.5 8 12 12.5 8 12.5 1.5 8 1.5 8z" />
      <circle cx="8" cy="8" r="2" />
      {off && <path d="M2.5 13.5 13.5 2.5" />}
    </svg>
  );
}

export { EyeIcon };
