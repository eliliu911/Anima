import { type SizeOption } from "../../api/studio";

export interface VideoRatioPreset {
  id: string;
  label: string;
  shortLabel: string;
  desc: string;
}

export const VIDEO_RATIO_PRESETS: VideoRatioPreset[] = [
  { id: "16:9", label: "横屏 16:9", shortLabel: "16:9", desc: "横屏 · 影视宽屏" },
  { id: "9:16", label: "竖屏 9:16", shortLabel: "9:16", desc: "竖屏 · 短剧/抖音" },
  { id: "1:1", label: "正方 1:1", shortLabel: "1:1", desc: "方形 · 社媒/头像" },
  { id: "4:3", label: "标清 4:3", shortLabel: "4:3", desc: "标清 · 经典胶片" },
  { id: "3:4", label: "竖屏 3:4", shortLabel: "3:4", desc: "竖屏 · 肖像写真" },
  { id: "21:9", label: "宽画幅 21:9", shortLabel: "21:9", desc: "超宽 · 院线巨幕" },
];

/**
 * WeeTodd/MLX 真实支持的多画幅视频分辨率预设库
 * 所有分辨率严格满足 VAE 空间下采样约束：长宽必须为 32 的整数倍 (width % 32 === 0 && height % 32 === 0)
 * 且单边最大不超过 1920 像素。
 */
export const WEETODD_RATIO_VIDEO_SIZES: Record<string, Record<string, SizeOption[]>> = {
  ltx: {
    "16:9": [
      { label: "768×512", width: 768, height: 512, verified: true, secs: 121, peak_gb: 22.88 },
      { label: "1344×768", width: 1344, height: 768, verified: true, secs: 473, peak_gb: 123.45 },
      { label: "1920×1088 1080p", width: 1920, height: 1088, verified: true, secs: 839, peak_gb: 103.78 },
    ],
    "9:16": [
      { label: "512×768", width: 512, height: 768, verified: true, secs: 121, peak_gb: 22.88 },
      { label: "768×1344", width: 768, height: 1344, verified: true, secs: 473, peak_gb: 123.45 },
      { label: "1088×1920 1080p", width: 1088, height: 1920, verified: true, secs: 839, peak_gb: 103.78 },
    ],
    "1:1": [
      { label: "512×512 预览", width: 512, height: 512, verified: true, secs: 98, peak_gb: 18.2 },
      { label: "768×768 原生", width: 768, height: 768, verified: true, secs: 320, peak_gb: 55.4 },
      { label: "1024×1024 高清", width: 1024, height: 1024, verified: true, secs: 620, peak_gb: 98.0 },
    ],
    "4:3": [
      { label: "672×512 标清", width: 672, height: 512, verified: true, secs: 110, peak_gb: 21.0 },
      { label: "1024×768 原生", width: 1024, height: 768, verified: true, secs: 380, peak_gb: 85.2 },
      { label: "1440×1088 巨幕", width: 1440, height: 1088, verified: true, secs: 680, peak_gb: 102.5 },
    ],
    "3:4": [
      { label: "512×672 标清", width: 512, height: 672, verified: true, secs: 110, peak_gb: 21.0 },
      { label: "768×1024 原生", width: 768, height: 1024, verified: true, secs: 380, peak_gb: 85.2 },
      { label: "1088×1440 巨幕", width: 1088, height: 1440, verified: true, secs: 680, peak_gb: 102.5 },
    ],
    "21:9": [
      { label: "1216×512 预览", width: 1216, height: 512, verified: true, secs: 210, peak_gb: 25.6 },
      { label: "1792×768 院线", width: 1792, height: 768, verified: true, secs: 540, peak_gb: 128.0 },
    ],
  },
  h3: {
    "16:9": [
      { label: "640×384 预览", width: 640, height: 384, verified: true, secs: 222, peak_gb: 15.81 },
      { label: "768×512 标准", width: 768, height: 512, verified: false, secs: 450, peak_gb: 28.0 },
      { label: "1344×768 原生", width: 1344, height: 768, verified: false, secs: 780, peak_gb: 31.0 },
    ],
    "9:16": [
      { label: "384×640 预览", width: 384, height: 640, verified: true, secs: 222, peak_gb: 15.81 },
      { label: "512×768 标准", width: 512, height: 768, verified: false, secs: 450, peak_gb: 28.0 },
      { label: "768×1344 原生", width: 768, height: 1344, verified: false, secs: 780, peak_gb: 31.0 },
    ],
    "1:1": [
      { label: "384×384 预览", width: 384, height: 384, verified: true, secs: 150, peak_gb: 12.5 },
      { label: "512×512 标准", width: 512, height: 512, verified: false, secs: 310, peak_gb: 18.0 },
      { label: "768×768 原生", width: 768, height: 768, verified: false, secs: 550, peak_gb: 30.0 },
    ],
    "4:3": [
      { label: "512×384 预览", width: 512, height: 384, verified: true, secs: 170, peak_gb: 13.8 },
      { label: "672×512 标准", width: 672, height: 512, verified: false, secs: 360, peak_gb: 22.0 },
      { label: "1024×768 原生", width: 1024, height: 768, verified: false, secs: 620, peak_gb: 29.5 },
    ],
    "3:4": [
      { label: "384×512 预览", width: 384, height: 512, verified: true, secs: 170, peak_gb: 13.8 },
      { label: "512×672 标准", width: 512, height: 672, verified: false, secs: 360, peak_gb: 22.0 },
      { label: "768×1024 原生", width: 768, height: 1024, verified: false, secs: 620, peak_gb: 29.5 },
    ],
    "21:9": [
      { label: "896×384 预览", width: 896, height: 384, verified: true, secs: 260, peak_gb: 18.0 },
      // H3 长边上限 1344（`weetodd::fit_dimensions`）。以前这里写 1792×768，
      // 后端会静默缩成 1344×576，界面上显示的尺寸是假的。1344×576 正好 21:9。
      { label: "1344×576 原生", width: 1344, height: 576, verified: false, secs: null, peak_gb: null },
    ],
  },
};

// 保持基础 16:9 预设兼容性
export const WEETODD_VIDEO_SIZES: Record<string, SizeOption[]> = {
  ltx: [
    { label: "768×512", width: 768, height: 512, verified: true, secs: 121, peak_gb: 22.88 },
    { label: "1344×768", width: 1344, height: 768, verified: true, secs: 473, peak_gb: 123.45 },
    { label: "1920×1088 1080p", width: 1920, height: 1088, verified: true, secs: 839, peak_gb: 103.78 },
  ],
  h3: [
    { label: "640×384 预览", width: 640, height: 384, verified: true, secs: 222, peak_gb: 15.81 },
    { label: "768×512", width: 768, height: 512, verified: false, secs: null, peak_gb: null },
  ],
};

export function inferVideoRatio(width?: number, height?: number, explicitRatio?: string): string {
  if (explicitRatio && ["16:9", "9:16", "1:1", "4:3", "3:4", "21:9"].includes(explicitRatio)) {
    return explicitRatio;
  }
  if (!width || !height) return "16:9";
  if (width === height) return "1:1";
  const r = width / height;
  if (Math.abs(r - 1) < 0.05) return "1:1";
  if (Math.abs(r - 4 / 3) < 0.08) return "4:3";
  if (Math.abs(r - 3 / 4) < 0.08) return "3:4";
  if (Math.abs(r - 21 / 9) < 0.15) return "21:9";
  if (width < height) return "9:16";
  return "16:9";
}

