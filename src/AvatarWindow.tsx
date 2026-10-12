import { useEffect, useRef, useState } from "react";
import { cursorPosition, getCurrentWindow } from "@tauri-apps/api/window";

import { Avatar3D } from "./components/Avatar3D";
import { AvatarCanvas } from "./components/AvatarCanvas";
import { AvatarLive2D } from "./components/AvatarLive2D";
import { AvatarNeuralStream } from "./components/AvatarNeuralStream";
import { useFace } from "./store/face";
import { useVoice } from "./store/voice";
import { useAvatarState } from "./store/avatarState";
import { alphaAt, Passthrough } from "./avatar/passthrough";
import { feedDesktopCursor, stopFeedingDesktopCursor } from "./avatar/desktopCursor";
import { openPanel, setAvatarPassthrough } from "./api/voice";
import { LevelBars, MicFill } from "./components/MicLevel";
import { ErrorBoundary } from "./components/ErrorBoundary";

/**
 * 角色层：一个盖在桌面上的无边框透明窗口，里面**只有她**。
 *
 * ## 和主窗口的关系
 *
 * 没有关系 —— 这是重点。它是独立窗口，主窗口关掉她也还在桌面上。
 * 「桌宠」和「一个可以缩小的应用」最根本的区别就在这儿。
 *
 * ## 唯一的常驻 UI 是状态胶囊
 *
 * 没有工具栏、没有按钮。需要什么从托盘开窗口。状态用一颗点表达：
 * 琥珀=正在听、青瓷=空闲、陶=出错 —— 而不是一圈会发光的边框。
 */
export function AvatarWindow() {
  const mode = useFace((s) => s.mode);
  const modelId = useFace((s) => s.modelId);
  const hologramStyle = useFace((s) => s.hologramStyle);
  const hologramColor = useFace((s) => s.hologramColor);
  // **口型通道的识别结果只有这里知道** —— 模型是在这个窗口里解析的。
  // 不往 store 里送，诊断窗（另一个 webview）就永远看不到「认出几个通道」，
  // 而那正是排查「嘴不动」时唯一有用的数字。
  const setReport = useFace((s) => s.setReport);
  const state = useAvatarState();
  const level = useVoice((s) => s.level);
  const emotion = useVoice((s) => s.emotion);
  const action = useVoice((s) => s.action);
  const actionSeq = useVoice((s) => s.actionSeq);
  const running = useVoice((s) => s.running);
  const toggleVoice = useVoice((s) => s.toggle);

  const hostRef = useRef<HTMLDivElement>(null);
  const [interactive, setInteractive] = useState(false);

  usePassthrough(hostRef, setInteractive);

  // 监听后端形态与面容广播事件 (anima://avatar-model)
  useEffect(() => {
    const applyModel = (model: string | null | undefined) => {
      if (!model) {
        useFace.getState().setMode("abstract");
        useFace.getState().setModelId(null);
        return;
      }
      const lower = model.toLowerCase();
      const isNeural =
        lower.startsWith("neural/") ||
        lower.endsWith(".mp4") ||
        lower.endsWith(".mov") ||
        lower.endsWith(".webm") ||
        lower.endsWith(".jpg") ||
        lower.endsWith(".png") ||
        lower.endsWith(".jpeg");
      const isLive2d =
        lower.includes("live2d") ||
        lower.endsWith(".model3.json") ||
        lower.endsWith(".model.json");

      if (isNeural) {
        useFace.getState().setMode("neural_stream");
      } else if (isLive2d) {
        useFace.getState().setMode("live2d");
      } else {
        useFace.getState().setMode("concrete");
      }
      useFace.getState().setModelId(model);
    };

    // 1. 启动时同步当前生效形态
    import("@tauri-apps/api/core").then(({ invoke }) => {
      invoke<any>("persona_view")
        .then((view) => {
          if (view?.resolved?.avatar?.value) {
            applyModel(view.resolved.avatar.value);
          }
        })
        .catch(() => {});
    });

    let unlistenModel: (() => void) | undefined;
    let unlistenClear: (() => void) | undefined;
    let unlistenPersona: (() => void) | undefined;

    // 2. 监听事件广播
    import("@tauri-apps/api/event").then(({ listen }) => {
      listen<string>("anima://avatar-model", (event) => {
        applyModel(event.payload);
      }).then((u) => {
        unlistenModel = u;
      });

      listen("anima://persona-changed", () => {
        import("@tauri-apps/api/core").then(({ invoke }) => {
          invoke<any>("persona_view")
            .then((view) => {
              if (view?.resolved?.avatar?.value) {
                applyModel(view.resolved.avatar.value);
              }
            })
            .catch(() => {});
        });
      }).then((u) => {
        unlistenPersona = u;
      });

      listen("anima://avatar-model-clear", () => {
        applyModel(null);
      }).then((u) => {
        unlistenClear = u;
      });
    });

    return () => {
      unlistenModel?.();
      unlistenClear?.();
      unlistenPersona?.();
    };
  }, []);

  return (
    <div
      ref={hostRef}
      className="w-screen h-screen overflow-hidden relative select-none"
    >
      {/* 模型渲染全景画布：完全满铺，不截断人物腿部与阴影 */}
      <div className="w-full h-full absolute inset-0 flex items-center justify-center overflow-hidden">
        <ErrorBoundary
          fallbackTitle="桌面面容渲染异常"
          onReset={() => {
            useFace.getState().setMode("abstract");
            useFace.getState().setModelId(null);
          }}
        >
          {mode === "abstract" ? (
            <AvatarCanvas state={state} level={level} emotion={emotion} />
          ) : mode === "live2d" ? (
            <AvatarLive2D
              modelId={modelId}
              emotion={emotion ?? undefined}
              action={action ?? undefined}
              actionSeq={actionSeq}
              className="w-full h-full"
            />
          ) : mode === "neural_stream" ? (
            <AvatarNeuralStream
              modelId={modelId ?? undefined}
              emotion={emotion ?? undefined}
              className="w-full h-full"
            />
          ) : (
            <Avatar3D
              // **桌宠窗口要取到腰**。默认的半身像会让她缩在中间一小块、
              // 上下大片留白 —— 在 360×520 这种又窄又高的窗口里尤其明显
              bodyHeads={4.5}
              modelId={modelId ?? ""}
              hologramMode={mode === "hologram"}
              hologramStyle={hologramStyle}
              hologramColor={hologramColor}
              emotion={emotion ?? undefined}
              action={action ?? undefined}
              actionSeq={actionSeq}
              onReport={setReport}
              className="w-full h-full"
            />
          )}
        </ErrorBoundary>
      </div>

      {/* 悬浮控制与状态岛：统一位于右上角空白区，完全移出人物模型主体 */}
      <div className={`avatarwin-hud absolute top-4 right-4 z-50 flex flex-col items-center gap-2 p-2 bg-[var(--paper)]/95 backdrop-blur-md rounded-2xl border border-[var(--line)] shadow-lg transition-all duration-300 ${interactive || state === "speaking" || state === "listening" ? "opacity-100 translate-x-0 pointer-events-auto" : "opacity-0 translate-x-4 pointer-events-none"}`}>
        {/* 拖动手柄 */}
        <div
          className="w-8 h-8 flex items-center justify-center text-[var(--ink-3)] hover:text-[var(--ink)] hover:bg-[var(--surface-2)] rounded-xl transition-colors cursor-grab active:cursor-grabbing select-none"
          data-tauri-drag-region
          onMouseDown={(e) => {
            if (e.button === 0) {
              e.preventDefault();
              void getCurrentWindow().startDragging();
            }
          }}
          onPointerDown={(e) => {
            if (e.button === 0) {
              e.preventDefault();
              void getCurrentWindow().startDragging();
            }
          }}
          title="按住拖动"
          aria-label="按住拖动"
        >
          <svg width="14" height="14" viewBox="0 0 24 24" aria-hidden="true" className="pointer-events-none">
            <circle cx="9" cy="6" r="1.6" fill="currentColor" />
            <circle cx="15" cy="6" r="1.6" fill="currentColor" />
            <circle cx="9" cy="12" r="1.6" fill="currentColor" />
            <circle cx="15" cy="12" r="1.6" fill="currentColor" />
            <circle cx="9" cy="18" r="1.6" fill="currentColor" />
            <circle cx="15" cy="18" r="1.6" fill="currentColor" />
          </svg>
        </div>

        {/* 麦克风开关 */}
        <button
          type="button"
          className={`w-8 h-8 flex items-center justify-center rounded-xl transition-colors ${running ? "text-[var(--red)] bg-[color-mix(in_srgb,var(--red)_10%,transparent)] hover:bg-[color-mix(in_srgb,var(--red)_15%,transparent)]" : "text-[var(--ink-3)] hover:text-[var(--ink)] hover:bg-[var(--surface-2)]"}`}
          onClick={(e) => {
            e.stopPropagation();
            void toggleVoice();
          }}
          title={running ? "关掉麦克风" : "开麦克风，开始听"}
          aria-label={running ? "关掉麦克风" : "开麦克风"}
        >
          <MicFill source="input" active={running} className="w-full h-full rounded-xl" />
        </button>

        {/* 打开对话 */}
        <button
          type="button"
          className="w-8 h-8 flex items-center justify-center text-[var(--ink-3)] hover:text-[var(--ink)] hover:bg-[var(--surface-2)] rounded-xl transition-colors cursor-pointer"
          onClick={(e) => {
            e.stopPropagation();
            void openPanel("chat");
          }}
          title="打开对话"
          aria-label="打开对话"
        >
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none"
               stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true">
            <path d="M21 12a8 8 0 0 1-11.6 7.1L4 20l1-4.6A8 8 0 1 1 21 12z" />
          </svg>
        </button>

        {/* 打开设置 */}
        <button
          type="button"
          className="w-8 h-8 flex items-center justify-center text-[var(--ink-3)] hover:text-[var(--ink)] hover:bg-[var(--surface-2)] rounded-xl transition-colors cursor-pointer"
          onClick={(e) => {
            e.stopPropagation();
            void openPanel("settings");
          }}
          title="打开设置"
          aria-label="打开设置"
        >
          <svg
            width="15"
            height="15"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden="true"
          >
            <path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" />
            <circle cx="12" cy="12" r="3" />
          </svg>
        </button>

        {/* 分割线 */}
        <div className="w-4 h-[1px] bg-[var(--line)] my-0.5" />

        {/* 状态指示（集成在控制岛中，完全不遮挡人物） */}
        <div
          className="avatarwin-status flex items-center justify-center w-8 h-6 text-[10.5px] select-none"
          title={`当前状态：${label(state)}`}
        >
          {state === "speaking" ? (
            <LevelBars source="output" height={12} className="text-[var(--celadon)]" />
          ) : state === "listening" ? (
            <LevelBars source="input" height={12} className="text-[var(--amber)]" />
          ) : (
            <span
              className={`w-2.5 h-2.5 rounded-full ${
                dotTone(state) === "busy"
                  ? "bg-[var(--amber)] animate-pulse"
                  : dotTone(state) === "bad"
                    ? "bg-[var(--red)]"
                    : "bg-emerald-500"
              }`}
            />
          )}
        </div>
      </div>
    </div>
  );
}

function dotTone(state: string): string {
  if (state === "listening" || state === "speaking" || state === "thinking") return "busy";
  if (state === "error") return "bad";
  return "idle";
}

function label(state: string): string {
  switch (state) {
    case "listening":
      return "正在听";
    case "thinking":
      return "在想";
    case "speaking":
      return "说话中";
    case "error":
      return "出错了";
    default:
      return "待机";
  }
}

/**
 * 每帧判定光标压没压在她身上，按滞后切换窗口的鼠标穿透。
 *
 * 判定与滞后的逻辑在 `avatar/passthrough.ts`（纯逻辑，有测试）。
 * 这里只负责三件脏活：读像素、收集输入、在状态真的变了时调一次系统 API。
 */
function usePassthrough(
  hostRef: React.RefObject<HTMLDivElement | null>,
  onInteractive: (v: boolean) => void,
) {
  useEffect(() => {
    const machine = new Passthrough();
    const win = getCurrentWindow();
    let raf = 0;
    let last = performance.now();
    /** 光标相对本窗口内容区的位置（CSS 像素）。拿不到就是 null */
    let cursor: { x: number; y: number } | null = null;
    let pressed = false;
    let focused = false;
    let disposed = false;

    /**
     * **必须轮询系统的全局光标，不能用 pointermove。**
     *
     * 这是这套东西最容易做错的地方，而且错了会**彻底卡死**：
     * 窗口一旦穿透就收不到自己的鼠标事件 —— 于是判不出"光标压在她身上"，
     * 于是永远退不出穿透，于是永远收不到事件。一个死循环，
     * 表现是"鼠标怎么都定位不到她"。
     *
     * 所以位置只能从系统层面问。轮询频率 30Hz 就够（滞后本来是 250ms），
     * 每次是一次很小的 IPC。
     */
    const poll = async () => {
      while (!disposed) {
        try {
          const [cur, pos, size, factor] = await Promise.all([
            cursorPosition(),
            win.outerPosition(),
            win.innerSize(),
            win.scaleFactor(),
          ]);
          // 全是物理像素，换算成 CSS 像素再和 canvas 对齐
          const x = (cur.x - pos.x) / factor;
          const y = (cur.y - pos.y) / factor;
          const w = size.width / factor;
          const h = size.height / factor;
          cursor = x >= 0 && y >= 0 && x < w && y < h ? { x, y } : null;
          // 视线跟随也读这一份 —— 它同样收不到 pointermove
          feedDesktopCursor(cursor);
        } catch {
          // 窗口正在关闭之类。下一轮再说
          cursor = null;
          feedDesktopCursor(null);
        }
        await new Promise((r) => setTimeout(r, 33));
      }
    };
    void poll();

    const onDown = () => {
      pressed = true;
    };
    const onUp = () => {
      pressed = false;
    };
    const onFocus = () => {
      focused = true;
    };
    const onBlur = () => {
      focused = false;
      pressed = false;
    };
    window.addEventListener("pointerdown", onDown);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("focus", onFocus);
    window.addEventListener("blur", onBlur);

    const tick = () => {
      const now = performance.now();
      const dt = (now - last) / 1000;
      last = now;

      const changed = machine.update(
        { onOpaquePixel: onHer(hostRef.current, cursor), pressed, focused },
        dt,
      );
      if (changed) {
        onInteractive(!machine.ignoreCursor);
        // 失败不重试：窗口可能刚被隐藏，下一帧还会再算一次
        void setAvatarPassthrough(machine.ignoreCursor).catch(() => {});
      }
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);

    return () => {
      disposed = true;
      stopFeedingDesktopCursor();
      cancelAnimationFrame(raf);
      window.removeEventListener("pointerdown", onDown);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("focus", onFocus);
      window.removeEventListener("blur", onBlur);
      // 卸载时务必恢复接收事件，否则窗口会永远穿透 ——
      // 那时候连拖都拖不动，只能杀进程
      void setAvatarPassthrough(false).catch(() => {});
    };
  }, [hostRef, onInteractive]);
}

function onHer(host: HTMLDivElement | null, cursor: { x: number; y: number } | null): boolean {
  if (!host || !cursor) return false;

  // 1. 如果光标在悬浮控制条（HUD）或状态栏范围内，必须直接判定为命中！
  const hudEls = host.querySelectorAll(".avatarwin-hud, .avatarwin-status");
  for (let i = 0; i < hudEls.length; i++) {
    const el = hudEls[i] as HTMLElement;
    const rect = el.getBoundingClientRect();
    if (
      cursor.x >= rect.left &&
      cursor.x <= rect.right &&
      cursor.y >= rect.top &&
      cursor.y <= rect.bottom
    ) {
      return true;
    }
  }

  // 2. 检查 3D 模型 Canvas 像素的 Alpha 通道
  const canvas = host.querySelector("canvas");
  if (!canvas) return false;
  const rect = canvas.getBoundingClientRect();

  // 3. 人物主体包围盒（宽度中间 70%，高度 8%~92%）作为双重保障
  const inModelBodyBox =
    cursor.x >= rect.left + rect.width * 0.15 &&
    cursor.x <= rect.left + rect.width * 0.85 &&
    cursor.y >= rect.top + rect.height * 0.08 &&
    cursor.y <= rect.top + rect.height * 0.92;

  const gl = canvas.getContext("webgl2") ?? canvas.getContext("webgl");
  if (gl) {
    const buf = new Uint8Array(4);
    const alpha = alphaAt(
      {
        canvas: {
          width: canvas.width,
          height: canvas.height,
          clientWidth: canvas.clientWidth,
          clientHeight: canvas.clientHeight,
        },
        readPixel: (px, py) => {
          gl.readPixels(px, Math.max(0, canvas.height - 1 - py), 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, buf);
          return buf[3] / 255;
        },
      },
      cursor.x - rect.left,
      cursor.y - rect.top,
    );
    if (alpha >= 0.05) return true;
  }

  return inModelBodyBox;
}
