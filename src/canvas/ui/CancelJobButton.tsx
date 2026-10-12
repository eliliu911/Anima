import { useState } from "react";
import { X, Loader2 } from "lucide-react";
import { canvasJobCancel } from "../../api/canvasJobs";
import { useCanvasStore } from "../store/useCanvasStore";

/**
 * 取消一个正在跑的节点。
 *
 * # 为什么这个按钮到今天才有
 *
 * `jobs.rs` 的模块注释里立过一条规矩：**在能真取消之前，一个取消入口都不给** ——
 * 「点了状态变成已取消、ComfyUI 照跑」是这个项目最典型的病。
 * 后端现在按任务 id 记着它发出去的 `prompt_id`，能真撤了，所以补上。
 *
 * # 为什么按钮只在有 `jobId` 时出现
 *
 * 没有 `jobId` 说明这一次还没登记成后台任务（刚点下去的那一瞬间），
 * 这时候点取消，后端找不到任何 prompt 可撤 —— 给一个撤不掉的按钮，
 * 比没有按钮更糟。
 *
 * # 后端那句话要原样显示
 *
 * 它会在「没找到对应的 ComfyUI 任务」时明说，让人知道该去 ComfyUI 界面上撤。
 * 把它吞掉换成一句「已取消」，就又回到上面那个病了。
 */
export function CancelJobButton({ nodeId }: { nodeId: string }) {
  const jobId = useCanvasStore((s) => (s.nodes.find((n) => n.id === nodeId)?.data as { jobId?: string } | undefined)?.jobId);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState("");
  if (!jobId) return null;

  const go = async () => {
    setBusy(true);
    try {
      setMsg(await canvasJobCancel(jobId));
    } catch (e) {
      setMsg(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="absolute top-3 right-3 z-30 flex flex-col items-end gap-1">
      <button
        type="button"
        disabled={busy}
        onClick={(e) => { e.stopPropagation(); void go(); }}
        title="取消这次生成 —— 会真的让 ComfyUI 撤掉排队中或正在跑的任务"
        className="nodrag nopan flex items-center gap-1 rounded-full border border-white/15 bg-black/80 px-2.5 py-1 text-xs text-white/85 backdrop-blur-md hover:bg-black/95 hover:text-white disabled:opacity-50"
      >
        {busy ? <Loader2 className="h-3 w-3 animate-spin" /> : <X className="h-3 w-3" />}
        取消
      </button>
      {msg && (
        <span role="status" className="nodrag max-w-[240px] rounded-md bg-black/85 px-2 py-1 text-right text-[10px] leading-snug text-white/70 backdrop-blur-md">
          {msg}
        </span>
      )}
    </div>
  );
}
