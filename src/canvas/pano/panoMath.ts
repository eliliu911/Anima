/**
 * 全景取景的数学。**这一块单独拆出来是因为它是唯一能自动验的部分** ——
 * three.js 渲染出来对不对只能看，而「反打是不是真的差 180°」可以算。
 *
 * # 为什么全景查看器比「再出一张图」重要得多
 *
 * 跨镜场景一致性一直卡在同一个地方：**让模型转到对面去，它不转。**
 * 实测两轮，「柜台反打」出来的都还是从店堂朝柜台看 —— 四个方位实际
 * 只落在两个机位上（见 `docs/ANIMA_PRODUCTION_ANALYSIS.md`）。
 *
 * 全景这条路不问模型转不转，**它转的是相机**：同一张球面贴图，
 * 正打 yaw=0°、反打 yaw=180°，两张图的地理关系是**几何保证**的，
 * 不是提示词求来的。这是我们第一次真正拿到能力表上「保证」那一层
 * 的东西（那张表在分析文档第七部分）。
 *
 * 抄 DramaClaw 的 `viewer-kit/pano`：它的查看器真正的产出不是「浏览」，
 * 是 `PanoCaptureResult` —— 在某个 yaw/pitch/fov 下按画幅裁一块出来。
 */

/** 视场角上下限。和 DramaClaw 一致。 */
export const FOV_MIN = 5;
export const FOV_MAX = 170;

/** 默认视场角。70° 约等于 26mm，接近人眼舒适视角，也是它的默认。 */
export const FOV_DEFAULT = 70;

/**
 * 等效焦距（35mm 全画幅）。
 *
 * **给导演看的是毫米数，不是角度。** 「26mm」是一句行话，
 * 「70 度」不是 —— 分镜表上写的也是焦段。
 *
 * 公式和 DramaClaw 一致：`18 / tan(fov/2)`。
 */
export function fovToFocal(fovDeg: number): number {
  const f = clampFov(fovDeg);
  return Math.round(18 / Math.tan(((f / 2) * Math.PI) / 180));
}

export function clampFov(fovDeg: number): number {
  return Math.max(FOV_MIN, Math.min(FOV_MAX, fovDeg));
}

/**
 * 归一化到 **[-180, 180)**。和 DramaClaw 同一个公式。
 *
 * **角度必须归一化**，否则「差 180°」这种判断会漏（450° 和 90° 是同一个
 * 方向，不归一化就看不出来）。
 *
 * 注意值域是左闭右开：**180 会落成 -180**。算夹角没问题（同一个方向），
 * 但**给人看的地方别用它** —— `yaw-180` 既难看又容易被读成「反着的」。
 * 显示和文件名走 [`displayDeg`]。
 */
export function normalizeDeg(value: number): number {
  return ((Number(value) + 540) % 360) - 180;
}

/**
 * 给人看的角度：**0 ~ 359**。
 *
 * 方位读数用这个 —— 「正后 = 180°」是所有人都认的写法，
 * 「正后 = -180°」不是。
 */
export function displayDeg(value: number): number {
  return ((Math.round(Number(value)) % 360) + 360) % 360;
}

/** 两个方位相差多少度（0~180）。 */
export function angleBetween(a: number, b: number): number {
  return Math.abs(normalizeDeg(a - b));
}

export type CaptureAspect = "16:9" | "4:3" | "1:1" | "9:16" | "2:3";

export const CAPTURE_ASPECTS: { id: CaptureAspect; label: string; ratio: number }[] = [
  { id: "16:9", label: "16:9 横", ratio: 16 / 9 },
  { id: "4:3", label: "4:3", ratio: 4 / 3 },
  { id: "1:1", label: "1:1 方", ratio: 1 },
  { id: "9:16", label: "9:16 竖", ratio: 9 / 16 },
  { id: "2:3", label: "2:3", ratio: 2 / 3 },
];

export function aspectRatio(aspect: CaptureAspect): number {
  return CAPTURE_ASPECTS.find((a) => a.id === aspect)?.ratio ?? 16 / 9;
}

export interface CropRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/**
 * 在一块画布上按画幅**居中**裁一块最大的。
 *
 * 取景框必须是裁出来的而不是改渲染尺寸：改尺寸会改变 FOV 的含义
 * （同样 70° 在 16:9 和 9:16 下框住的东西完全不同），
 * 而导演调的是「这一镜多广」，不该被画幅偷偷改掉。
 */
export function cropRectForAspect(
  canvas: { width: number; height: number },
  aspect: CaptureAspect,
): CropRect {
  const target = aspectRatio(aspect);
  const source = canvas.width / Math.max(1, canvas.height);
  let width = canvas.width;
  let height = canvas.height;
  if (source > target) width = Math.round(canvas.height * target);
  else height = Math.round(canvas.width / target);
  return {
    x: Math.max(0, Math.round((canvas.width - width) / 2)),
    y: Math.max(0, Math.round((canvas.height - height) / 2)),
    width: Math.max(1, width),
    height: Math.max(1, height),
  };
}

/** 一次取景：站在球心，朝哪、多广、出什么画幅。 */
export interface Shot {
  /** 方位名，进文件名 */
  name: string;
  /** 水平角。**0° = 正前方**（校正之后的那个正前方） */
  yawDeg: number;
  /** 俯仰角。正 = 抬头 */
  pitchDeg: number;
  fovDeg: number;
}

/**
 * 一套覆盖一圈的默认机位。
 *
 * **反打就是 yaw + 180**，不是一句提示词。这是这条路的全部价值 ——
 * 它不可能不转过去。
 */
export function ringShots(count: number, fovDeg = FOV_DEFAULT): Shot[] {
  const n = Math.max(1, Math.floor(count));
  const names = ["正前", "右前", "右", "右后", "正后", "左后", "左", "左前"];
  return Array.from({ length: n }, (_, i) => {
    const yaw = normalizeDeg((360 / n) * i);
    // 八向以内用方位名，超过就用角度 —— 编号（视角1、视角2）是
    // drama-skills 明令的反面教材：无语义命名会让下游读不出它是哪
    const name = n <= names.length ? names[Math.round((i * names.length) / n)] : `${Math.round(yaw)}°`;
    return { name, yawDeg: yaw, pitchDeg: 0, fovDeg: clampFov(fovDeg) };
  });
}

/**
 * 多视角截图组的机位表。
 *
 * 改写自 DramaClaw（ClaymoreLab, Elastic License 2.0）
 * frontend/src/features/canvas/nodes/Pano360ViewerNode.tsx 的
 * GRID_2X2_FRAMES / GRID_4X3_FRAMES：4 视角 = 四个水平方向平视、90°；
 * 12 视角 = 四个方向 × 仰 40° / 平视 / 俯 40°、75°，**每行一个俯仰**。
 * 出处规则见 DECISIONS.md 2026-09-16。
 *
 * yaw 都相对「正前方」—— 舞台的 `setView` 自己会加上 `frontYaw` 偏移。
 */
export interface ViewGrid {
  id: "four" | "twelve";
  label: string;
  /** 截图组排几列。12 视角排 4 列，一行正好是同一个俯仰 */
  cols: number;
  shots: Shot[];
}

const GRID_DIRECTIONS = [
  { name: "前方", yawDeg: 0 },
  { name: "右侧", yawDeg: 90 },
  { name: "后方", yawDeg: 180 },
  { name: "左侧", yawDeg: -90 },
];
const GRID_PITCHES = [
  { name: "仰视", pitchDeg: 40 },
  { name: "平视", pitchDeg: 0 },
  { name: "俯视", pitchDeg: -40 },
];

export function viewGrid(id: ViewGrid["id"]): ViewGrid {
  if (id === "four") {
    return {
      id,
      label: "4 视角",
      cols: 2,
      shots: GRID_DIRECTIONS.map((d) => ({ name: d.name, yawDeg: d.yawDeg, pitchDeg: 0, fovDeg: 90 })),
    };
  }
  return {
    id,
    label: "12 视角",
    cols: 4,
    shots: GRID_PITCHES.flatMap((p) =>
      GRID_DIRECTIONS.map((d) => ({ name: `${d.name}·${p.name}`, yawDeg: d.yawDeg, pitchDeg: p.pitchDeg, fovDeg: 75 })),
    ),
  };
}

/** 正反打一对。**差整 180°，几何保证。** */
export function shotReverseOf(shot: Shot): Shot {
  return {
    ...shot,
    name: `${shot.name}反打`,
    yawDeg: normalizeDeg(shot.yawDeg + 180),
    // 反打的俯仰也要翻过来：正打俯视对方，反打就是仰视
    pitchDeg: -shot.pitchDeg,
  };
}

/**
 * 「正前方」偏移。
 *
 * 全景图的 x=0 未必是场景的正前方（我们出图时把第一个方位放在 x=0%，
 * 但用户可能想把别的方向当正前）。定了之后所有机位的 yaw 都相对它算 ——
 * **不改图，只改读数的原点**。
 */
export function applyFrontOffset(yawDeg: number, frontYawDeg: number): number {
  return normalizeDeg(yawDeg + frontYawDeg);
}

/**
 * 「把现在看的方向设为正前方」：**画面不动，读数归零**。
 *
 * 舞台实际朝向 = `applyFrontOffset(yawDeg, frontYawDeg)`。要让它不变而 yaw 读数变 0，
 * 新的偏移就得是「旧偏移 + 当前 yaw」。
 *
 * 原来节点里写成 `frontYawDeg: -yaw`（yawDeg 不动），实际朝向变成 yaw − yaw = 0 ——
 * 一点按钮**画面跳回全景图正中**，读数却还是原来的角度，和按钮说明正好相反。
 * 而且原来的写法无视了已有的偏移，连按两次结果还不一样。
 */
export function frontToCurrent(yawDeg: number, frontYawDeg: number): { frontYawDeg: number; yawDeg: number } {
  return { frontYawDeg: normalizeDeg(frontYawDeg + yawDeg), yawDeg: 0 };
}

/**
 * 文件名。**带上角度** —— 只写「反打」的话，两个场景各出一张就撞名了，
 * 而且事后看不出它是哪个方向。
 */
export function shotFileName(sceneName: string, shot: Shot): string {
  const safe = (sceneName || "场景").replace(/[/\\:*?"<>|]/g, "_").slice(0, 24);
  const yaw = displayDeg(shot.yawDeg);
  // 俯仰不为 0 才写 —— 12 视角里同一个方向有三张，不带俯仰的话文件名只靠方位名区分
  const pitch = Math.round(shot.pitchDeg) ? `_pitch${Math.round(shot.pitchDeg)}` : "";
  return `${safe}_${shot.name}_yaw${yaw}${pitch}_fov${Math.round(shot.fovDeg)}.png`;
}
