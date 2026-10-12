/**
 * 粒子核心的调试台。
 *
 * 存在的理由：这个效果只能靠眼睛调，而在 Tauri 窗口里改一版看一版极其笨重
 * （要重启、还没法截图对比）。这里把 AvatarCanvas 单独挂在浏览器里，
 * 手动切状态、拉音量，就能快速看出「聚合/离散」和「图谱浮现」到底像不像。
 *
 * 它不依赖任何 Tauri API，所以直接 `npm run dev` 打开 /lab.html 就能用。
 * 生产构建不包含这个入口。
 */
import { useState } from "react";
import { createRoot } from "react-dom/client";

import { AvatarCanvas, type AvatarState } from "./components/AvatarCanvas";
import "./styles.css";

const STATES: AvatarState[] = ["idle", "listening", "thinking", "speaking"];

function Lab() {
  const [state, setState] = useState<AvatarState>("thinking");
  const [level, setLevel] = useState(0);
  const [auto, setAuto] = useState(false);

  // 自动循环各状态，用来看过渡是否顺滑
  useState(() => {
    let timer: number | undefined;
    const loop = () => {
      timer = window.setInterval(() => {
        setState((s) => STATES[(STATES.indexOf(s) + 1) % STATES.length]);
      }, 4000);
    };
    if (auto) loop();
    return () => window.clearInterval(timer);
  });

  return (
    <div className="lab">
      <AvatarCanvas state={state} level={level} className="lab__canvas" />

      <div className="lab__panel">
        <div className="lab__row">
          {STATES.map((s) => (
            <button
              key={s}
              type="button"
              className={s === state ? "pill pill--on" : "pill"}
              onClick={() => setState(s)}
            >
              {s}
            </button>
          ))}
        </div>

        <label className="lab__slider">
          音量 {level.toFixed(2)}
          <input
            type="range"
            min={0}
            max={1}
            step={0.01}
            value={level}
            onChange={(e) => setLevel(Number(e.target.value))}
          />
        </label>

        <label className="lab__row">
          <input
            type="checkbox"
            checked={auto}
            onChange={(e) => setAuto(e.target.checked)}
          />
          每 4 秒自动切换状态
        </label>

        <p className="lab__hint">当前：{state}</p>
      </div>
    </div>
  );
}

const root = document.getElementById("root");
if (!root) throw new Error("找不到 #root");
createRoot(root).render(<Lab />);
