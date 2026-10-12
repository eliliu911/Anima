import { useEffect } from "react";
import { MediaThumb } from "../../components/studio/MediaThumb";
import { X, Download } from "./icons";
import { downloadMedia } from "../utils/downloadMedia";

interface CanvasImageLightboxProps {
  src: string | null;
  kind?: "image" | "video";
  alt?: string;
  onClose: () => void;
}

export function CanvasImageLightbox({
  src,
  kind = "image",
  alt = "全屏预览",
  onClose,
}: CanvasImageLightboxProps) {
  useEffect(() => {
    if (!src) return;

    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };

    window.addEventListener("keydown", handleKeyDown, true);
    return () => {
      window.removeEventListener("keydown", handleKeyDown, true);
    };
  }, [src, onClose]);

  if (!src) return null;

  return (
    <div
      role="dialog"
      aria-modal="true"
      className="fixed inset-0 z-[99999] flex flex-col items-center justify-center bg-black/90 backdrop-blur-xl p-6 select-none animate-in fade-in-0 duration-150"
      onClick={onClose}
    >
      {/* 顶部悬浮工具栏 */}
      <div
        className="absolute top-5 inset-x-6 flex items-center justify-between pointer-events-auto z-10"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-2 rounded-full bg-white/10 px-3 py-1.5 backdrop-blur-md border border-white/10 text-xs text-white/80">
          <span>{kind === "image" ? "全屏大图预览" : "全屏视频预览"}</span>
          <span className="text-white/40">·</span>
          <span className="text-white/50 font-mono text-[11px]">按 ESC 或点击空白退出</span>
        </div>

        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => downloadMedia(src)}
            className="flex items-center gap-1.5 rounded-full bg-white/10 hover:bg-white/20 active:scale-95 px-3 py-1.5 text-xs font-medium text-white border border-white/15 backdrop-blur-md shadow-lg transition-all cursor-pointer"
            title="下载此文件"
          >
            <Download className="h-3.5 w-3.5" />
            <span>下载</span>
          </button>

          <button
            type="button"
            onClick={onClose}
            className="flex h-8 w-8 items-center justify-center rounded-full bg-white/10 hover:bg-white/20 active:scale-95 text-white/80 hover:text-white border border-white/15 backdrop-blur-md transition-all cursor-pointer"
            title="关闭全屏 (Esc)"
          >
            <X className="h-4 w-4" />
          </button>
        </div>
      </div>

      {/* 主展示区 */}
      <div
        className="relative max-w-[92vw] max-h-[85vh] flex items-center justify-center overflow-hidden rounded-2xl shadow-2xl"
        onClick={(e) => e.stopPropagation()}
        onDoubleClick={onClose}
      >
        <MediaThumb
          path={src}
          alt={alt}
          kind={kind}
          className="max-h-[85vh] max-w-[92vw] object-contain rounded-2xl"
        />
      </div>

      {/* 底部 ESC 提示 */}
      <div className="mt-4 text-xs text-white/40 flex items-center gap-1.5 pointer-events-none">
        <kbd className="px-1.5 py-0.5 rounded bg-white/10 text-[10px] font-mono border border-white/15 text-white/70">
          Esc
        </kbd>
        <span>退出全屏显示</span>
      </div>
    </div>
  );
}
