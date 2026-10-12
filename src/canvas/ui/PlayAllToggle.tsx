import { useState } from "react";
import { Pause, Play } from "lucide-react";

const KEY = "anima.canvas.playAllPreviews";

/**
 * 「全部播放」：运镜 / 风格窗口顶栏的开关（用户 2026-09-25）。关 = 鼠标停在卡片上才播，开 = 所有示例一起循环。
 * 记在本机（localStorage），读写失败就按关处理。
 */
export function usePlayAll(): [boolean, (v: boolean) => void] {
  const [on, setOn] = useState(() => {
    try { return window.localStorage.getItem(KEY) === "1"; } catch { return false; }
  });
  const set = (v: boolean) => {
    setOn(v);
    try { window.localStorage.setItem(KEY, v ? "1" : "0"); } catch { /* 存不进去就算了 */ }
  };
  return [on, set];
}

export function PlayAllToggle({ on, onChange }: { on: boolean; onChange: (v: boolean) => void }) {
  return (
    <button type="button" aria-pressed={on} onClick={() => onChange(!on)}
      title={on ? "全部播放：开 —— 点一下改回鼠标停留才播" : "全部播放：关 —— 鼠标停在卡片上才播，点一下让全部一起播"}
      className={`flex h-8 items-center gap-1.5 rounded-lg px-2.5 text-xs ${on ? "bg-white/15 text-white" : "text-white/55 hover:bg-white/10 hover:text-white"}`}>
      {on ? <Pause className="h-3.5 w-3.5" /> : <Play className="h-3.5 w-3.5" />}全部播放
    </button>
  );
}
