/**
 * 右侧滑出面板。任务队列、资产选择器共用这一个壳。
 *
 * # 为什么 portal 到 `.anima-studio` 而不是 body
 *
 * 面板要贴在**窗口**右边、占满整高。而 `position: fixed` 的参照物
 * 不一定是窗口 —— 祖先链上任何一个带 `transform` / `filter` /
 * `backdrop-filter` / `contain` 的元素都会把它变成新的参照物。
 * Studio 里这三样到处都是（卡片的悬浮位移、页头的毛玻璃、流光卡片的
 * `translate3d`）。
 *
 * 真出过事：资产选择器写成 `fixed inset-y-0 right-0` 放在镜头卡里，
 * 结果它贴在那张卡的右边、高度只有卡那么高 —— **CSS 没错，参照物错了**，
 * 而且不报任何错。
 *
 * 但**不能 portal 到 `document.body`**：所有 `--studio-*` 变量都定义在
 * `.anima-studio` 这个类上，不在 `:root`。挂到 body 之后
 * `var(--studio-bg-elevated)` 解析为空，`background` 整条声明作废 ——
 * 面板变成全透明的，底下的内容直接透上来，遮罩也一起没了。
 * 这也是"不报错"的一种：CSS 里一个无效的 `var()` 只是让那条声明消失。
 *
 * 所以挂到 `.anima-studio` 本身：它没有 transform / filter，
 * 不会成为 fixed 的参照物，而变量都在它身上。
 *
 * # 为什么两个面板要共用
 *
 * 队列和选择器各写一份的时候，宽度、动画、遮罩、关闭方式全都对不齐 ——
 * 用户会以为它们是两种不同的东西。
 */
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { createPortal } from "react-dom";

/**
 * 让面板里的内容也能触发**带动画的**关闭。
 *
 * 不给这条通路的话，「选中一个资产就关掉」会走父组件的 `setOpen(false)` ——
 * 那是直接卸载，没有出场动画。同一个面板两种关法长得不一样，
 * 用户会觉得「有时候会滑、有时候不会」。
 */
const CloseCtx = createContext<() => void>(() => {});
export const useSidePanelClose = () => useContext(CloseCtx);

/** 出场动画时长。**和 CSS 里那个数必须一致** —— 早了会截断，晚了会卡住。 */
const EXIT_MS = 220;

export function SidePanel({
  title,
  subtitle,
  onClose,
  trailing,
  children,
  label,
}: {
  title: string;
  subtitle?: string;
  onClose: () => void;
  /** 标题右边、关闭按钮左边的额外动作（比如「清空」）。 */
  trailing?: ReactNode;
  children: ReactNode;
  /** 给读屏用的区域名。不传就用标题。 */
  label?: string;
}) {
  // **先播出场动画，再真的卸载。**
  //
  // 只有进场动画的话，关闭是"啪"地消失 —— 和滑进来完全不对称，
  // 看着像被强制关掉的。所有关闭路径（遮罩、✕、Esc、选中一项）
  // 都走这一个入口，否则会出现"有时候会滑、有时候不会"。
  const [closing, setClosing] = useState(false);
  const timer = useRef<number | undefined>(undefined);
  const requestClose = useCallback(() => {
    if (timer.current !== undefined) return;
    setClosing(true);
    timer.current = window.setTimeout(onClose, EXIT_MS);
  }, [onClose]);
  useEffect(() => () => window.clearTimeout(timer.current), []);

  // Esc 关掉。**面板在右边，而鼠标多半停在左边的内容上** ——
  // 键盘是这里最顺手的出口。
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && requestClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [requestClose]);

  // **挂到 `.anima-studio`，不是 body。** 变量都定义在那个类上。
  // 找不到就退回 body —— 那时候面板会没底色，但至少不会整个不显示，
  // 而且这条只在 Studio 之外被误用时才会走到。
  const host =
    (typeof document !== "undefined" && document.querySelector(".anima-studio")) ||
    (typeof document !== "undefined" ? document.body : null);
  if (!host) return null;

  return createPortal(
    <>
      <div
        aria-hidden="true"
        onClick={requestClose}
        className="fixed inset-0 z-[60]"
        style={{
          background: "var(--studio-overlay)",
          animation: `${closing ? "studio-scrim-out" : "studio-scrim-in"} ${EXIT_MS}ms ease-out both`,
        }}
      />
      <aside
        role="region"
        aria-label={label ?? title}
        className="fixed inset-y-0 right-0 z-[61] flex w-[340px] max-w-[90vw] flex-col"
        style={{
          background: "var(--studio-bg-elevated)",
          borderLeft: "1px solid var(--studio-border-subtle)",
          animation: `${closing ? "studio-queue-out" : "studio-queue-in"} ${EXIT_MS}ms cubic-bezier(0.22,1,0.36,1) both`,
        }}
      >
        <header
          className="flex shrink-0 items-start justify-between gap-2 px-4 py-3"
          style={{ borderBottom: "1px solid var(--studio-border-subtle)" }}
        >
          <div className="flex min-w-0 flex-col gap-0.5">
            <span className="truncate text-[13px] font-medium">{title}</span>
            {subtitle && (
              <span className="truncate text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
                {subtitle}
              </span>
            )}
          </div>
          <div className="flex shrink-0 items-center gap-1">
            {trailing}
            <button
              type="button"
              onClick={requestClose}
              aria-label="关闭"
              className="grid h-6 w-6 place-items-center rounded-md"
              style={{ color: "var(--studio-text-muted)" }}
            >
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" aria-hidden="true">
                <path d="M6 6l12 12M18 6L6 18" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
              </svg>
            </button>
          </div>
        </header>
        <CloseCtx.Provider value={requestClose}>{children}</CloseCtx.Provider>
      </aside>
    </>,
    host,
  );
}
