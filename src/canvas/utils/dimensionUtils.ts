export interface AspectRatioPreset {
  id: string;
  label: string;
  wRatio: number;
  hRatio: number;
  desc: string;
}

export const ASPECT_RATIOS: AspectRatioPreset[] = [
  { id: "16:9", label: "16:9", wRatio: 16, hRatio: 9, desc: "横屏 · 电影/影视" },
  { id: "9:16", label: "9:16", wRatio: 9, hRatio: 16, desc: "竖屏 · 抖音/微短剧" },
  { id: "1:1", label: "1:1", wRatio: 1, hRatio: 1, desc: "正方 · 头像/社媒" },
  { id: "4:3", label: "4:3", wRatio: 4, hRatio: 3, desc: "标准 · 经典构图" },
  { id: "3:4", label: "3:4", wRatio: 3, hRatio: 4, desc: "人像 · 艺术写真" },
  { id: "21:9", label: "21:9", wRatio: 21, hRatio: 9, desc: "宽画幅 · 院线巨幕" },
  // **3:2 / 2:3 是后端实测过的横竖版尺寸（1216×832 / 832×1216）。** 以前没有这两档，
  // `inferRatio(1216, 832)` 落不进任何一档的容差，于是返回默认值「16:9」——
  // 一张 3:2 的图在画布上标着 16:9。
  { id: "3:2", label: "3:2", wRatio: 3, hRatio: 2, desc: "横版 · 设定图" },
  { id: "2:3", label: "2:3", wRatio: 2, hRatio: 3, desc: "竖版 · 人物全身" },
];

export interface ResolutionPreset {
  id: string;
  label: string;
  desc: string;
}

export const VIDEO_RESOLUTIONS: ResolutionPreset[] = [
  { id: "720p", label: "720P", desc: "标清 · 快速流畅" },
  { id: "1080p", label: "1080P", desc: "高清 · 锐利画质" },
];

const VIDEO_DIMENSION_MAP: Record<string, Record<string, [number, number]>> = {
  "16:9": { "720p": [1280, 720], "1080p": [1920, 1080] },
  "9:16": { "720p": [720, 1280], "1080p": [1080, 1920] },
  "1:1":  { "720p": [720, 720], "1080p": [1080, 1080] },
  "4:3":  { "720p": [960, 720], "1080p": [1440, 1080] },
  "3:4":  { "720p": [720, 960], "1080p": [1080, 1440] },
  "21:9": { "720p": [1280, 548], "1080p": [1920, 820] },
};

/** 视频表里有尺寸的画幅。视频模式的画幅按钮只显示这些 —— 3:2 / 2:3 视频表里没有，
 *  点了会静默退回 1280×720。 */
export function videoRatioIds(): string[] {
  return Object.keys(VIDEO_DIMENSION_MAP);
}

export function calculateVideoDimensions(ratio: string, resolution = "720p"): { width: number; height: number } {
  const [w, h] = VIDEO_DIMENSION_MAP[ratio]?.[resolution] || [1280, 720];
  return { width: w, height: h };
}

export function inferRatio(width?: number, height?: number): string {
  if (!width || !height) return "16:9";
  // **取最近的一档，不是第一个落进容差的。** 旧写法按顺序找「差 < 0.08」的第一档：
  // 832×1216（0.684）先碰到 3:4（0.75，差 0.066）就返回了，而它真正是 2:3（0.667）。
  const r = width / height;
  let best = ASPECT_RATIOS[0];
  for (const a of ASPECT_RATIOS) {
    if (Math.abs(r - a.wRatio / a.hRatio) < Math.abs(r - best.wRatio / best.hRatio)) best = a;
  }
  return best.id;
}

export function inferVideoResolution(width?: number, height?: number): string {
  if (!width || !height) return "720p";
  const minDim = Math.min(width, height);
  if (minDim >= 1000) return "1080p";
  return "720p";
}

/**
 * 后端尺寸表里的一项（`studio::image_sizes()`，经 `studioCapabilities` 拿到）。
 *
 * # 为什么图片尺寸不再写死在这里
 *
 * 这里曾经有一份自己的表，和后端对不上，而且**提供 4K** —— 而后端明写着 4K
 * 在这台机器上跑不了（实测峰值 210.65 GB，MPS 上限 107.52 GiB，857 秒后失败）。
 * 画布在提供一个已知会失败的选项。现在图片尺寸只有后端一个来源。
 */
export interface CatalogSize {
  label: string;
  width: number;
  height: number;
  verified: boolean;
  secs: number | null;
}

/** 画布出图节点的默认尺寸。用户点名的 16:9 视频参考图尺寸，32 的倍数。 */
export const DEFAULT_IMAGE_SIZE = { width: 1024, height: 576 } as const;

/** 尺寸自己的 id —— 选中态靠它比对，不靠「1k / 2k」这种档位名。 */
export function sizeId(width: number, height: number): string {
  return `${width}x${height}`;
}

/** 按画幅分组，组内按像素从小到大。只返回有尺寸的画幅，顺序跟 `ASPECT_RATIOS` 一致。 */
export function imageSizesByRatio(sizes: CatalogSize[]): { ratio: AspectRatioPreset; sizes: CatalogSize[] }[] {
  const groups = new Map<string, CatalogSize[]>();
  for (const s of sizes) {
    const id = inferRatio(s.width, s.height);
    groups.set(id, [...(groups.get(id) ?? []), s]);
  }
  return ASPECT_RATIOS.filter((a) => groups.has(a.id)).map((a) => ({
    ratio: a,
    sizes: [...groups.get(a.id)!].sort((x, y) => x.width * x.height - y.width * y.height),
  }));
}

/**
 * 切到某个画幅时选哪个尺寸：**优先实测过的**，其次最接近当前像素量的。
 * 这个画幅一个尺寸都没有时返回 null —— 不瞎给一个。
 */
export function pickSizeForRatio(
  sizes: CatalogSize[],
  ratioId: string,
  current?: { width: number; height: number },
): CatalogSize | null {
  const group = imageSizesByRatio(sizes).find((g) => g.ratio.id === ratioId)?.sizes ?? [];
  if (!group.length) return null;
  const px = current ? current.width * current.height : 0;
  return [...group].sort((a, b) => {
    if (a.verified !== b.verified) return a.verified ? -1 : 1;
    return Math.abs(a.width * a.height - px) - Math.abs(b.width * b.height - px);
  })[0];
}
