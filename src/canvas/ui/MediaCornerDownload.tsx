import { Download } from "./icons";
import { downloadMedia } from "../utils/downloadMedia";

/**
 * 预览画面右上角的下载图标。**和视频节点画廊右上角那个是同一个样子** ——
 * 超分、深度动作捕捉这些不走画廊的节点用它，不在标题栏另放「下载」按钮（2026-09-23 用户定的）。
 * 父元素要是 `relative`。存到「下载」文件夹并在访达里选中（见 `downloadMedia`）。
 */
export function MediaCornerDownload({ path }: { path: string }) {
  return (
    <button
      type="button"
      onClick={(e) => {
        e.stopPropagation();
        void downloadMedia(path);
      }}
      title="下载到本地（存到「下载」文件夹，并在访达里选中）"
      className="nodrag nopan absolute top-2 right-2 z-20 flex items-center justify-center rounded-full bg-black/65 hover:bg-black/90 backdrop-blur-md p-1.5 text-white/80 hover:text-white border border-white/15 shadow-lg transition-all cursor-pointer hover:scale-110 active:scale-95"
    >
      <Download className="h-3.5 w-3.5" />
    </button>
  );
}
