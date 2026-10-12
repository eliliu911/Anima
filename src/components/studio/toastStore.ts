import { create } from "zustand";

/**
 * Studio 的通知条。
 *
 * # 为什么需要它
 *
 * 提取实体要 20~60 秒。这段时间里界面上什么都没有的话，看起来就是卡死了 ——
 * 而用户的反应是再点一次，于是两个请求并发跑，回来两份结果互相覆盖。
 * （实测 LumenX 那边就是靠一条常驻的进度条把这段时间填上的。）
 *
 * # 一条规则：进度条要能被自己的结果替换掉
 *
 * 「分析中…」和「分析完成」是**同一件事的两个状态**，不是两条消息。
 * 各推一条的话，屏幕上会同时挂着「分析中」和「完成」——
 * 用户截图里 LumenX 就有这个毛病（一条转圈的 + 两条完成的叠在一起）。
 * 所以 `push` 返回 id，`resolve(id, ...)` 就地替换。
 */
export type ToastKind = "progress" | "success" | "error";

export interface Toast {
  id: string;
  kind: ToastKind;
  title: string;
  /** 第二行。说清「在干什么、要多久、能不能走开」。 */
  body?: string;
  /** 到点自动消失（毫秒）。`null` = 要手动关。 */
  ttl: number | null;
}

interface ToastState {
  items: Toast[];
  push: (t: Omit<Toast, "id" | "ttl"> & { ttl?: number | null }) => string;
  /** 就地把某条换成结果 —— 不是再推一条。 */
  resolve: (id: string, t: Omit<Toast, "id" | "ttl"> & { ttl?: number | null }) => void;
  dismiss: (id: string) => void;
  /**
   * 按固定 id 放一条：有就就地换，没有就新加。给后台任务用 —— id 取任务 id，
   * 同一个任务从「在跑」到「完成」始终是同一条，界面刷新后重放也不会叠出第二条。
   *
   * **用户手动关掉的进度条不再弹回来**（进度每场都会更新一次，不挡住就会一直冒）；
   * 但结果（成功 / 失败）照样显示 —— 那是用户走开之后要知道的事。
   */
  upsert: (id: string, t: Omit<Toast, "id" | "ttl"> & { ttl?: number | null }) => void;
  /** 被用户手动关掉的 id */
  closed: string[];
}

/** 默认停留时间。成功的看一眼就够，报错的要留着让人读完。 */
export const DEFAULT_TTL: Record<ToastKind, number | null> = {
  progress: null,
  success: 4500,
  error: null,
};

let seq = 0;

export const useToasts = create<ToastState>((set) => ({
  items: [],
  closed: [],
  push: (t) => {
    const id = `toast-${++seq}`;
    const item: Toast = { ...t, id, ttl: t.ttl ?? DEFAULT_TTL[t.kind] };
    set((s) => ({ items: [...s.items, item] }));
    return id;
  },
  resolve: (id, t) =>
    set((s) => ({
      items: s.items.map((x) =>
        x.id === id ? { ...t, id, ttl: t.ttl ?? DEFAULT_TTL[t.kind] } : x,
      ),
    })),
  dismiss: (id) =>
    set((s) => ({
      items: s.items.filter((x) => x.id !== id),
      closed: s.closed.includes(id) ? s.closed : [...s.closed.slice(-199), id],
    })),
  upsert: (id, t) =>
    set((s) => {
      if (t.kind === "progress" && s.closed.includes(id)) return s;
      const item: Toast = { ...t, id, ttl: t.ttl ?? DEFAULT_TTL[t.kind] };
      return s.items.some((x) => x.id === id)
        ? { items: s.items.map((x) => (x.id === id ? item : x)) }
        : { items: [...s.items, item] };
    }),
}));

/**
 * 「发现 3 个角色、1 个场景、1 个道具。」
 *
 * **零的那类不写出来。** 「发现 3 个角色、0 个场景、0 个道具」读起来
 * 像是出了问题，而实际上那段剧本里本来就没有场景。
 */
export function describeCounts(counts: { label: string; n: number }[]): string {
  const parts = counts.filter((c) => c.n > 0).map((c) => `${c.n} 个${c.label}`);
  return parts.length === 0 ? "什么都没找到" : `发现 ${parts.join("、")}。`;
}

export interface TaskSpec<T> {
  /** 在跑时的标题，例如「正在合成成片…」 */
  title: string;
  /** 说清要多久、能不能走开 */
  body?: string;
  /** 成功那条怎么写。拿得到结果，可以写「发现 3 个角色」这种 */
  success: (value: T) => { title: string; body?: string };
  /** 失败那条的标题；正文是错误原文 */
  failure: string;
}

/**
 * 前端直接等结果的短任务（几十秒的 LM 调用、合成成片这类）走这里：
 * 推一条进度 → 就地换成结果。**报错照样抛出去**，页面原来的错误处理不用改。
 *
 * 通知条在全局 store 里，**切走页面也会报结果** —— 页面卸载了，结果照样有人看到。
 * 用 `upsert` 而不是 `resolve`：用户关掉了进度条，结果也还要显示。
 *
 * 真正跑几分钟以上、要扛住界面刷新的，做成后端任务（见 `jobs/useJobToasts`）。
 */
export async function runTask<T>(spec: TaskSpec<T>, work: () => Promise<T>): Promise<T> {
  const toasts = useToasts.getState();
  const id = `task-${++seq}`;
  toasts.upsert(id, { kind: "progress", title: spec.title, body: spec.body });
  try {
    const value = await work();
    useToasts.getState().upsert(id, { kind: "success", ...spec.success(value) });
    return value;
  } catch (e) {
    useToasts.getState().upsert(id, { kind: "error", title: spec.failure, body: String(e) });
    throw e;
  }
}
