import { create } from "zustand";

/**
 * # 用这个 store 的一条硬规则
 *
 * **每个字段单独选，不要返回对象。**
 *
 * ```ts
 * const size  = useStudioStore((s) => s.size);   // ✅
 * const batch = useStudioStore((s) => s.batch);
 *
 * const { size, batch } = useStudioStore((s) => ({ size: s.size, batch: s.batch })); // ❌
 * ```
 *
 * zustand v5 去掉了默认的浅比较。返回新对象的 selector 每次渲染都produces
 * 一个新引用，被判定为「状态变了」→ 重渲染 → 又是新对象 → 无限循环，
 * 报 `Maximum update depth exceeded`，**整个窗口白屏**。
 * 2026-09-04 在 ParamsPanel 上踩过一次。
 *
 * 真要一次取多个，用 `useShallow`。
 */

export type JobStatus = "pending" | "running" | "done" | "failed";

export interface StudioResult {
  id: string;
  mode: string;
  engineId: string;
  prompt: string;
  status: JobStatus;
  /** 出的文件（图或视频）的本地路径 */
  path?: string;
  seed?: number;
  error?: string;
  /**
   * **跑成功了、但有话要说。**
   *
   * 最典型的是引擎回落：选了 Z-Image + 图生图，实际出的是 SDXL 的图。
   * 工具那边一直在说这件事，而这里以前是
   * `error: out.ok ? undefined : out.message` —— **成功时整段丢掉**，
   * 于是回落、LoRA 名字写错、提示词被摊薄这几类告警一条都到不了界面。
   */
  notes?: string[];
  /** 提交时自动加进提示词的激活词。**显示出来**，别让人不知道提示词被动过 */
  triggersAdded?: string[];
  startedAt: number;
  endedAt?: number;
}

interface StudioState {
  mode: string;
  engineId: string | null;
  prompt: string;
  negativePrompt: string;
  /** ComfyUI `input/` 之外的本机绝对路径，按模式需要 1~2 张 */
  images: string[];
  seed: number | null;
  /**
   * 图生图的重绘幅度。`null` = 用后端默认（0.65）。
   *
   * 界面上以前**没有这个出口** —— 后端一直用默认值，等于图生图只有一档，
   * 而它是这条链路唯一真正的旋钮。
   */
  denoise: number | null;
  /** 出图/出片尺寸。切引擎时若不在新引擎的可选项里会被换掉 */
  size: { width: number; height: number } | null;
  /** 一次排几个 */
  batch: number;
  /** 视频时长（秒） */
  duration: number | null;
  /** H3 的采样档位 id */
  /** 选中的 LoRA 及强度 */
  loras: { name: string; strength: number }[];
  /** 提交时把猜到的激活词并进提示词 */
  injectTriggers: boolean;
  results: StudioResult[];
  /** 同时最多跑几个。**ComfyUI 是串行的**，排队交给它反而看不到进度 */
  concurrency: number;

  setMode: (m: string) => void;
  setEngine: (e: string | null) => void;
  setPrompt: (p: string) => void;
  setNegativePrompt: (p: string) => void;
  setImage: (i: number, path: string | null) => void;
  setSeed: (s: number | null) => void;
  setDenoise: (v: number | null) => void;
  setSize: (s: { width: number; height: number } | null) => void;
  setBatch: (n: number) => void;
  setDuration: (d: number | null) => void;
  /** `strength === null` = 取消选中 */
  setLora: (name: string, strength: number | null) => void;
  setInjectTriggers: (v: boolean) => void;
  addResult: (r: StudioResult) => void;
  patchResult: (id: string, patch: Partial<StudioResult>) => void;
  /** 从画廊里去掉一条。文件的删除由调用方先做 —— 这里只管列表。 */
  removeResult: (id: string) => void;
  /** 整份换掉结果列表。启动时从磁盘装回历史用。 */
  setResults: (r: StudioResult[]) => void;
  setConcurrency: (n: number) => void;
}

export const useStudioStore = create<StudioState>((set) => ({
  mode: "t2i",
  engineId: null,
  prompt: "",
  negativePrompt: "",
  images: [],
  seed: null,
  denoise: null,
  size: null,
  batch: 1,
  duration: null,
  loras: [],
  injectTriggers: true,
  results: [],
  concurrency: 1,

  setMode: (mode) => set({ mode }),
  setEngine: (engineId) => set({ engineId }),
  setPrompt: (prompt) => set({ prompt }),
  setNegativePrompt: (negativePrompt) => set({ negativePrompt }),
  setImage: (i, path) =>
    set((s) => {
      const images = [...s.images];
      if (path === null) images.splice(i, 1);
      else images[i] = path;
      return { images };
    }),
  setSeed: (seed) => set({ seed }),
  setDenoise: (denoise) => set({ denoise }),
  setSize: (size) => set({ size }),
  setBatch: (batch) => set({ batch }),
  setDuration: (duration) => set({ duration }),
  setLora: (name, strength) =>
    set((s) => ({
      loras:
        strength === null
          ? s.loras.filter((l) => l.name !== name)
          : s.loras.some((l) => l.name === name)
            ? s.loras.map((l) => (l.name === name ? { ...l, strength } : l))
            : [...s.loras, { name, strength }],
    })),
  setInjectTriggers: (injectTriggers) => set({ injectTriggers }),
  addResult: (r) => set((s) => ({ results: [r, ...s.results] })),
  removeResult: (id) => set((s) => ({ results: s.results.filter((r) => r.id !== id) })),
  patchResult: (id, patch) =>
    set((s) => ({
      results: s.results.map((r) => (r.id === id ? { ...r, ...patch } : r)),
    })),
  setResults: (results) => set({ results }),
  setConcurrency: (concurrency) => set({ concurrency }),
}));

/**
 * 排队里该不该再发一个。
 *
 * **ComfyUI 是串行执行的** —— 一次并发发三个，它自己排队，而我们这边三个
 * 计时器同时在跑，第三个必然显示成"超时"。踩过一次（一次三张固定第三张失败）。
 * 所以闸门放在客户端，默认 1。
 */
/**
 * 换引擎后这些参数该留还是该换。
 *
 * **越界的值必须换掉，不越界的必须留住。** 无脑重置会让人每次切引擎都要
 * 重设一遍；无脑保留则会把 1344×768 带到一个只支持 640×384 的引擎上，
 * 而那个错误要等到提交才会出现。
 */
export function reconcileParams(
  engine: {
    sizes: { width: number; height: number }[];
    duration: { min: number; max: number; step: number; default: number } | null;
    max_batch: number;
  },
  cur: {
    size: { width: number; height: number } | null;
    duration: number | null;
    batch: number;
  },
): { size: { width: number; height: number } | null; duration: number | null; batch: number } {
  const sizeOk =
    cur.size && engine.sizes.some((s) => s.width === cur.size!.width && s.height === cur.size!.height);
  const size = sizeOk
    ? cur.size
    : engine.sizes[0]
      ? { width: engine.sizes[0].width, height: engine.sizes[0].height }
      : null;

  // 时长：**夹到新引擎的范围里，别重置**。
  // LTX 的 20 秒带到 H3（上限 15）要变成 15，而不是弹回默认 5 ——
  // 无脑重置会让人每次切引擎都重设一遍。
  const d = engine.duration;
  const duration =
    d === null
      ? null
      : cur.duration === null
        ? d.default
        : Math.min(Math.max(cur.duration, d.min), d.max);

  const batch = Math.min(Math.max(1, cur.batch), Math.max(1, engine.max_batch));

  return { size, duration, batch };
}

export function canDispatch(results: StudioResult[], concurrency: number): boolean {
  const inFlight = results.filter(
    (r) => r.status === "pending" || r.status === "running",
  ).length;
  return inFlight < Math.max(1, concurrency);
}
