import { useState } from "react";
import { mediaSrc } from "../../components/studio/MediaThumb";

/**
 * 改前 / 改后并排比。
 *
 * # 为什么非有不可
 *
 * 指令式局部编辑最典型的失败是**「它把整张图重画了一遍」** ——
 * 脸变了、构图挪了、颜色偏了，而任务状态是成功、**不报任何错**。
 * 只看结果那一张根本看不出来：那张图本身可能很好看。
 *
 * 所以「不该变的动没动」必须靠并排看。拉杆是最省地方的看法 ——
 * 同一个位置左右推，像素级的位移一眼就看出来。
 */
export function BeforeAfter({ before, after }: { before: string; after: string }) {
  const [x, setX] = useState(50);
  return (
    <div className="nodrag nopan border-t border-white/10 px-3 py-2">
      <div className="relative w-full overflow-hidden rounded-md" style={{ aspectRatio: "1 / 1" }}>
        <img src={mediaSrc(after)} alt="改后" className="absolute inset-0 h-full w-full object-contain" />
        {/* 改前那张盖在上面，按拉杆裁掉右边 */}
        <img
          src={mediaSrc(before)}
          alt="改前"
          className="absolute inset-0 h-full w-full object-contain"
          style={{ clipPath: `inset(0 ${100 - x}% 0 0)` }}
        />
        <div className="pointer-events-none absolute inset-y-0 w-px bg-white/70" style={{ left: `${x}%` }} />
        <span className="pointer-events-none absolute left-1 top-1 rounded bg-black/60 px-1 text-[10px] text-white/80">改前</span>
        <span className="pointer-events-none absolute right-1 top-1 rounded bg-black/60 px-1 text-[10px] text-white/80">改后</span>
      </div>
      <input
        type="range" min={0} max={100} value={x}
        onChange={(e) => setX(Number(e.target.value))}
        aria-label="改前改后对比"
        className="mt-1 w-full"
      />
      <p className="text-[10px] text-white/35 leading-relaxed">
        左右推着看**不该变的地方动没动**。指令式编辑最常见的失败是整张重画 ——
        脸、构图、颜色都变了，而任务显示成功、不报错。
      </p>
    </div>
  );
}
