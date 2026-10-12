import { useEffect, useLayoutEffect, useState } from "react";
import { useToasts, type Toast } from "./toastStore";

/**
 * 右下角要让开的东西（画布的缩放工具条、导演世界的控制条……）挂这个属性。
 * 状态卡会量它们的位置，**有就抬到它们上面，没有就贴底** ——
 * 以前是写死 `bottom: 76px`，为画布工具条留的，结果在底下什么都没有的页面上
 * 也悬在半空（用户 2026-09-26：「不能在每个页面都在这儿，下面空着呢」）。
 */
export const TOAST_AVOID_ATTR = "data-toast-avoid";

/** 贴底时离窗口底边多远；让开别的东西时和它之间留多少。 */
export const TOAST_EDGE_PX = 16;
export const TOAST_GAP_PX = 12;
/** 通知栏的宽度和离右边的距离 —— 判「挡没挡着」要用 */
export const TOAST_WIDTH_PX = 340;
export const TOAST_RIGHT_PX = 16;

type Box = { top: number; left: number; right: number; bottom: number };

/**
 * 通知栏该离底边多高。纯函数：只看「右下角那一列里有哪些东西、各自顶边在哪」。
 *
 * 只算**水平方向和通知栏那一列重叠**、并且**在窗口下半部分**的 ——
 * 左下角的东西、页面顶上的东西都不该把通知顶起来。
 */
export function toastBottom(boxes: Box[], viewport: { width: number; height: number }): number {
  const colLeft = viewport.width - TOAST_RIGHT_PX - TOAST_WIDTH_PX;
  const colRight = viewport.width - TOAST_RIGHT_PX;
  let bottom = TOAST_EDGE_PX;
  for (const b of boxes) {
    if (b.right <= colLeft || b.left >= colRight) continue;
    if (b.bottom <= b.top) continue; // 隐藏的（0 高）
    if (b.top < viewport.height / 2) continue;
    bottom = Math.max(bottom, viewport.height - b.top + TOAST_GAP_PX);
  }
  return Math.round(bottom);
}

/** 实时量右下角要让开的东西。页面切换、工具条出现消失、窗口缩放都会重算。 */
function useToastBottom(active: boolean): number {
  const [bottom, setBottom] = useState(TOAST_EDGE_PX);
  useLayoutEffect(() => {
    if (!active) return;
    let raf = 0;
    const measure = () => {
      cancelAnimationFrame(raf);
      raf = requestAnimationFrame(() => {
        const boxes = [...document.querySelectorAll(`[${TOAST_AVOID_ATTR}]`)].map((el) =>
          el.getBoundingClientRect(),
        );
        setBottom(toastBottom(boxes, { width: window.innerWidth, height: window.innerHeight }));
      });
    };
    measure();
    // 工具条是跟着页面挂载 / 卸载的 —— 盯 DOM 的增删；窗口缩放也会让它们挪位置
    const mo = new MutationObserver(measure);
    mo.observe(document.body, { childList: true, subtree: true });
    window.addEventListener("resize", measure);
    return () => {
      cancelAnimationFrame(raf);
      mo.disconnect();
      window.removeEventListener("resize", measure);
    };
  }, [active]);
  return bottom;
}

type Slot = { t: Toast; out: boolean };

/**
 * 新的一轮 store 内容合进正在显示的槽位。
 *
 * - 还在的：原位，内容换成最新的（进度条就地变成成功时要跟着变）
 * - 不在了的：**原位**标成退场 —— 挪到末尾再收拢的话，看起来是它先跳下去再消失
 * - 新来的：追加在最后（最新的在最下面）
 */
export function mergeSlots(cur: Slot[], items: Toast[]): Slot[] {
  const next: Slot[] = cur.map((x) => {
    const live = items.find((t) => t.id === x.t.id);
    return live ? { t: live, out: false } : { t: x.t, out: true };
  });
  for (const t of items) if (!cur.some((x) => x.t.id === t.id)) next.push({ t, out: false });
  return next;
}

/** 退场动画的时长。和 CSS 里 `.studio-toast-slot` 的过渡一致。 */
const LEAVE_MS = 220;

/**
 * 通知条的渲染。挂在 Studio 的最外层，一次只有一处。
 *
 * 右下角、竖排、最新的在下面 —— 和「生成」按钮同侧，
 * 因为它报的就是那个按钮引发的事。
 *
 * **这是个堆叠区，不是几张死卡**：新的一张从底下进来，把上面的往上挤；
 * 某一张完成消失时，它的高度收拢，上面的自然落下来。
 * 做法是每张卡外面包一层 `grid-template-rows: 0fr ↔ 1fr` 的槽 ——
 * 高度能平滑过渡，而不用量每张卡多高。
 */
export function Toasts() {
  const items = useToasts((s) => s.items);
  // 渲染中的槽位：store 里删掉的卡**留在原位**标成退场，收拢完再真正拿掉
  const [shown, setShown] = useState<Slot[]>(() => items.map((t) => ({ t, out: false })));
  const [seen, setSeen] = useState(items);
  if (seen !== items) {
    setSeen(items);
    setShown((cur) => mergeSlots(cur, items));
  }
  const anyOut = shown.some((x) => x.out);
  useEffect(() => {
    if (!anyOut) return;
    const timer = setTimeout(() => setShown((cur) => cur.filter((x) => !x.out)), LEAVE_MS);
    return () => clearTimeout(timer);
  }, [anyOut, shown]);

  const bottom = useToastBottom(shown.length > 0);
  if (shown.length === 0) return null;
  return (
    <div
      className="pointer-events-none fixed z-[60] flex flex-col"
      style={{
        right: TOAST_RIGHT_PX,
        width: TOAST_WIDTH_PX,
        bottom,
        // 工具条出现 / 消失时整栏平滑挪过去，不是一跳
        transition: "bottom 220ms cubic-bezier(0.22, 1, 0.36, 1)",
      }}
    >
      {shown.map(({ t, out }) => (
        <div key={t.id} className="studio-toast-slot" data-out={out ? "" : undefined}>
          <div className="min-h-0 overflow-hidden">
            {/* 卡之间的间距放在槽里：收拢时连间距一起收，不会剩一道缝 */}
            <div className="pt-2">
              <Row t={t} />
            </div>
          </div>
        </div>
      ))}
    </div>
  );
}

const TONE: Record<Toast["kind"], { fg: string; bg: string; bd: string }> = {
  progress: {
    fg: "var(--studio-primary)",
    bg: "var(--studio-bg-elevated)",
    bd: "var(--studio-border)",
  },
  success: {
    fg: "var(--studio-done-fg)",
    bg: "var(--studio-done-bg)",
    bd: "var(--studio-done-bd)",
  },
  error: {
    fg: "var(--studio-failed-fg)",
    bg: "var(--studio-failed-bg)",
    bd: "var(--studio-failed-bd)",
  },
};

function Row({ t }: { t: Toast }) {
  const dismiss = useToasts((s) => s.dismiss);

  useEffect(() => {
    if (t.ttl === null) return;
    const timer = setTimeout(() => dismiss(t.id), t.ttl);
    return () => clearTimeout(timer);
    // `t.kind` 也在依赖里：进度条被就地换成成功之后，
    // ttl 从 null 变成有值，计时器要在那一刻才起来。
  }, [t.id, t.ttl, t.kind, dismiss]);

  const tone = TONE[t.kind];
  return (
    <div
      className="studio-toast pointer-events-auto flex items-start gap-2.5 rounded-xl px-3.5 py-3"
      style={{ background: tone.bg, borderColor: tone.bd }}
    >
      <span className="mt-0.5 shrink-0" style={{ color: tone.fg }}>
        <Icon kind={t.kind} />
      </span>
      <div className="min-w-0 flex-1">
        <p className="text-[12.5px] font-medium leading-snug">{t.title}</p>
        {t.body && (
          <p
            className="mt-0.5 whitespace-pre-line text-[11.5px] leading-relaxed"
            style={{ color: "var(--studio-text-2)" }}
          >
            {t.body}
          </p>
        )}
      </div>
      <button
        type="button"
        onClick={() => dismiss(t.id)}
        aria-label="关闭"
        className="shrink-0 px-1 text-[13px] leading-none"
        style={{ color: "var(--studio-text-muted)" }}
      >
        ✕
      </button>
    </div>
  );
}

function Icon({ kind }: { kind: Toast["kind"] }) {
  const common = {
    width: 15,
    height: 15,
    viewBox: "0 0 24 24",
    fill: "none",
    stroke: "currentColor",
    strokeWidth: 1.8,
    strokeLinecap: "round" as const,
    strokeLinejoin: "round" as const,
  };
  if (kind === "progress") {
    // 缺口圆环 —— 转起来才看得出在动，实心圆转了跟没转一样
    return (
      <svg {...common} style={{ animation: "studio-spin 900ms linear infinite" }}>
        <path d="M21 12a9 9 0 1 1-6.2-8.6" />
      </svg>
    );
  }
  if (kind === "success") {
    return (
      <svg {...common}>
        <path d="M20 6.5 9.5 17.5 4 12" />
      </svg>
    );
  }
  return (
    <svg {...common}>
      <path d="M12 3.5 22 20H2zM12 10v4M12 17h.01" />
    </svg>
  );
}
