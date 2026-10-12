import { useEffect, useRef, useState } from "react";
import { comfyProgress, type ComfyProgress } from "../../api/comfyProgress";

/**
 * 正在生成时，每几秒问一次 ComfyUI 跑到哪了。
 *
 * # 为什么不做进度条
 *
 * 条形需要一个可信的百分比，而我们**有时候拿不到** ——
 * 日志读不到、或者 WeeTodd 换了那行的格式（见 Rust `comfy_progress`）。
 * 那时候只能说「已跑 N 分钟」，一根停在某个位置的条比一行字更容易被误读。
 *
 * DramaClaw 的 `NodeGenerationOverlay`（Elastic-2.0，参考其做法）在这一层
 * 显示一条按写死的预估时长模拟出来的百分比。那是另一种取舍：
 * 换来「永远有东西在动」，代价是卡住和正常慢看起来一样。
 *
 * # 轮询间隔
 *
 * 5 秒。读的是本地文件末尾 64 KB，几乎没有成本；而步与步之间隔十几分钟，
 * 更快也没有更多信息。**不生成时不轮询** —— 别为了一个看不见的数字占着 CPU。
 */
export function useComfyProgress(isGenerating: boolean): ComfyProgress | null {
  const [p, setP] = useState<ComfyProgress | null>(null);
  /** 这一轮从什么时候开始的。用来兜底显示「已跑 N 分钟」 */
  const startedAt = useRef<number | null>(null);

  useEffect(() => {
    if (!isGenerating) {
      startedAt.current = null;
      setP(null);
      return;
    }
    startedAt.current ??= Math.floor(Date.now() / 1000);
    let alive = true;
    const tick = () => {
      comfyProgress(startedAt.current ?? undefined)
        .then((r) => { if (alive) setP(r); })
        // 读不到就不显示 —— 保持原来那句「渲染中」，别弹错误
        .catch(() => { if (alive) setP(null); });
    };
    tick();
    const t = window.setInterval(tick, 5000);
    return () => { alive = false; window.clearInterval(t); };
  }, [isGenerating]);

  return p;
}
