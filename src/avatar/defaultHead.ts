/**
 * 内置的默认头像 —— **用代码生成，不是一个模型文件**。
 *
 * ## 为什么必须有它
 *
 * 原先这个功能依赖用户自己去 Ready Player Me 下一个带 morph target 的 .glb。
 * 而 RPM 在 2026 年 1 月 31 日随 Netflix 收购关停了 Avatar Creator 和公开 API，
 * 没有公开继任者。也就是说那条路**已经不存在了**，整个 3D 面容功能会卡在
 * "先去弄个模型"这一步上，永远走不到第一次能看见的地方。
 *
 * ## 为什么生成而不是塞一个 .glb 进仓库
 *
 * 三个理由，按重要性排：
 *
 * 1. **它走的是和真实模型完全相同的代码路径。** 生成出来的 mesh 带着规规矩矩的
 *    `morphTargetDictionary`，名字就是 `viseme_aa` 这一套。Avatar3D 不需要为它
 *    开任何特例 —— 于是它同时是一份**活的自检**：默认头能动，就证明从
 *    时间线到 morph target 这一段是好的。
 * 2. 不进二进制、没有授权问题、不用下载。仓库里一个几十 MB 的模型对
 *    "个人原型"这个定位来说也不合适。
 * 3. 每个口型长什么样是**在代码里写明的**（见下面那张表）。想知道 `U` 和 `O`
 *    到底该有什么区别，读这张表比去模型里翻 blend shape 快得多。
 *
 * ## 它长什么样
 *
 * 刻意做成风格化的半透明头，不是拟真人脸。放一个做得半吊子的人脸只会掉进
 * 恐怖谷 —— 而这本来就是个占位形象。它的职责是让口型**看得清**，
 * 所以嘴做得比例偏大、开合明确。
 */
import * as THREE from "three";
import type { Viseme } from "../api/voice";
import { VISEMES } from "./mouth";

/** 这个 id 不对应任何文件。设置面板和加载逻辑都靠它识别"用内置的" */
export const DEFAULT_HEAD_ID = "__builtin__";

/**
 * 每个口型的嘴形。
 *
 * - `w`：嘴角到嘴角的半宽
 * - `h`：上下唇的半开度
 * - `z`：嘴唇前突（撮口音靠它，光靠宽高做不出 U 和 O 的区别）
 *
 * 这张表就是"这 15 个通道分别是什么意思"的可执行定义。数值按一个半径为 1 的
 * 头来取。
 */
const SHAPES: Record<Viseme, { w: number; h: number; z: number }> = {
  // 闭嘴。这是基准形，所以全 0 位移
  sil: { w: 0.24, h: 0.012, z: 0 },
  // 双唇音：抿住并略微收窄，比静默更"用力"
  PP: { w: 0.21, h: 0.008, z: 0.005 },
  // 唇齿音：下唇内收，只留一条缝
  FF: { w: 0.26, h: 0.035, z: -0.005 },
  TH: { w: 0.22, h: 0.09, z: 0.01 },
  DD: { w: 0.23, h: 0.11, z: 0 },
  kk: { w: 0.25, h: 0.15, z: -0.01 },
  // j/q/x 与 zh/ch/sh：扁嘴前送
  CH: { w: 0.20, h: 0.10, z: 0.03 },
  // z/c/s：咧开但几乎不张
  SS: { w: 0.29, h: 0.045, z: -0.01 },
  nn: { w: 0.22, h: 0.075, z: 0 },
  RR: { w: 0.21, h: 0.12, z: 0.025 },
  // 元音。这五个的区分度决定了"看起来像在说话"还是"嘴在开合"
  aa: { w: 0.25, h: 0.30, z: 0 },
  E: { w: 0.31, h: 0.17, z: -0.01 },
  I: { w: 0.33, h: 0.085, z: -0.015 },
  O: { w: 0.19, h: 0.25, z: 0.04 },
  U: { w: 0.125, h: 0.125, z: 0.075 },
};

/** 嘴的轮廓采样点数。24 个足够圆滑，再多是浪费 */
const RING = 24;

/**
 * 造一张嘴。
 *
 * 结构是最简单的扇形：中心一个点，外圈 RING 个点，围成一个可变形的椭圆。
 * 材质用不受光的纯暗色 —— 它代表的是口腔内部，跟着光照变亮会很怪。
 */
function buildMouth(): THREE.Mesh {
  const base = ringPositions(SHAPES.sil);
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.Float32BufferAttribute(base, 3));

  // 扇形三角面：中心点 + 相邻两个外圈点
  const index: number[] = [];
  for (let i = 0; i < RING; i++) {
    index.push(0, 1 + i, 1 + ((i + 1) % RING));
  }
  geometry.setIndex(index);

  // morph 用相对位移。绝对坐标也能work，但相对位移在读代码时更直观：
  // 一眼能看出"这个口型相对闭嘴动了多少"
  geometry.morphAttributes.position = VISEMES.map((v) => {
    const target = ringPositions(SHAPES[v]);
    const delta = target.map((value, i) => value - base[i]);
    return new THREE.Float32BufferAttribute(delta, 3);
  });
  geometry.computeVertexNormals();

  const mesh = new THREE.Mesh(
    geometry,
    new THREE.MeshBasicMaterial({
      color: 0x1a0f1e,
      side: THREE.DoubleSide,
    }),
  );
  geometry.morphTargetsRelative = true;
  // **名字必须是 `viseme_xx`。** 这样它走的就是真实模型那条匹配路径，
  // 而不是一条只有内置头才走的特例分支
  mesh.morphTargetDictionary = Object.fromEntries(
    VISEMES.map((v, i) => [`viseme_${v}`, i]),
  );
  mesh.morphTargetInfluences = new Array(VISEMES.length).fill(0);
  mesh.name = "BuiltinMouth";
  return mesh;
}

/** 按一个嘴形算出所有顶点坐标 */
function ringPositions(shape: { w: number; h: number; z: number }): number[] {
  const pos: number[] = [0, 0, shape.z * 0.5];
  for (let i = 0; i < RING; i++) {
    const a = (i / RING) * Math.PI * 2;
    const x = Math.cos(a) * shape.w;
    const y = Math.sin(a) * shape.h;
    // 前突在中间最强、嘴角最弱 —— 撮口时嘴角是被拉进去的
    const z = shape.z * (1 - Math.abs(Math.cos(a)) * 0.6);
    pos.push(x, y, z);
  }
  return pos;
}

/**
 * 把一个球揉成头的形状。
 *
 * 第一版直接用了球 —— 结果是一颗贴了两个点的**鸡蛋**，谁看都是鸡蛋。
 * 问题不在细节不够，在**轮廓**：脑袋和下巴一样宽的东西，人眼一律读成蛋。
 *
 * 所以只做轮廓上的三件事，都是廓形层面的，不加任何细碎装饰：
 *
 * 1. **收下巴。** 越往下越窄，到底部收到六成。这一条贡献最大 ——
 *    上宽下窄是"头"和"蛋"最主要的区别。
 * 2. **压后脑、留额头。** 脸这一侧（+z）稍微压平，后脑稍微拉长，
 *    球的正圆截面才不会到处一样。
 * 3. **下巴尖往前。** 底部靠前的一圈往前推一点，出一个下颌的转折。
 */
function shapeSkull(geometry: THREE.BufferGeometry) {
  const pos = geometry.attributes.position as THREE.BufferAttribute;
  for (let i = 0; i < pos.count; i++) {
    const x = pos.getX(i);
    const y = pos.getY(i);
    const z = pos.getZ(i);

    // y 从 -1（下巴）到 +1（头顶）
    const down = Math.max(0, -y); // 只影响下半部分
    const taper = 1 - down * down * 0.42; // 平方：收窄集中在最下面，不是整个下半张脸

    // 脸这一侧压平一点，后脑留出来
    const front = Math.max(0, z);
    const flat = 1 - front * 0.14;

    // 下颌：底部靠前的一圈往前推
    const jut = down * down * front * 0.16;

    pos.setXYZ(i, x * taper, y, z * taper * flat + jut);
  }
  pos.needsUpdate = true;
  geometry.computeVertexNormals();
}

/** 眉毛。两条细横杠 —— 有没有它，"这是张脸"的辨识度差很多 */
function buildBrow(x: number): THREE.Mesh {
  const mesh = new THREE.Mesh(
    new THREE.BoxGeometry(0.17, 0.028, 0.05),
    new THREE.MeshStandardMaterial({
      color: 0x14202e,
      roughness: 0.5,
    }),
  );
  mesh.position.set(x, 0.33, 0.74);
  // 眉头低眉尾高，眉毛不是平的
  mesh.rotation.z = x < 0 ? -0.1 : 0.1;
  mesh.name = x < 0 ? "BuiltinBrowLeft" : "BuiltinBrowRight";
  return mesh;
}

/** 鼻子。一个很小的锥 —— 侧面轮廓全靠它 */
function buildNose(): THREE.Mesh {
  const mesh = new THREE.Mesh(
    new THREE.ConeGeometry(0.075, 0.16, 4),
    new THREE.MeshStandardMaterial({
      color: 0x6ea3c6,
      roughness: 0.4,
      metalness: 0.1,
      transparent: true,
      opacity: 0.95,
    }),
  );
  mesh.position.set(0, -0.09, 0.80);
  mesh.rotation.set(Math.PI / 2, Math.PI / 4, 0);
  mesh.name = "BuiltinNose";
  return mesh;
}

/** 造一只眼睛。带 `eyesClosed` morph，压扁成一条线 */
function buildEye(x: number): THREE.Mesh {
  const geometry = new THREE.SphereGeometry(0.075, 20, 14);
  const base = geometry.attributes.position.array as Float32Array;

  const closed = new Float32Array(base.length);
  for (let i = 0; i < base.length; i += 3) {
    // 只压 y，且保留一点厚度 —— 压到 0 会看起来像眼睛消失了
    closed[i] = 0;
    closed[i + 1] = base[i + 1] * -0.88;
    closed[i + 2] = 0;
  }
  geometry.morphAttributes.position = [
    new THREE.Float32BufferAttribute(closed, 3),
  ];

  const mesh = new THREE.Mesh(
    geometry,
    new THREE.MeshStandardMaterial({
      color: 0x0d1420,
      roughness: 0.25,
      metalness: 0.1,
    }),
  );
  geometry.morphTargetsRelative = true;
  mesh.morphTargetDictionary = { eyesClosed: 0 };
  mesh.morphTargetInfluences = [0];
  mesh.position.set(x, 0.17, 0.78);
  mesh.scale.set(1, 1, 0.5);
  mesh.name = x < 0 ? "BuiltinEyeLeft" : "BuiltinEyeRight";
  return mesh;
}

/**
 * 造一颗完整的内置头。
 *
 * 返回的东西**和 GLTFLoader 解析出来的 scene 是同一类**，所以调用方不用分支。
 */
export function buildDefaultHead(): THREE.Group {
  const group = new THREE.Group();
  group.name = "AnimaBuiltinHead";

  // 头。半透明加自发光，和界面里那套电子云的调子统一 ——
  // 做成拟真皮肤反而会掉进恐怖谷，而这本来就是个占位形象
  const skullGeometry = new THREE.SphereGeometry(1, 48, 32);
  // **这一步不能省。** 不揉的话就是一颗贴了两个点的鸡蛋（第一版就是）
  shapeSkull(skullGeometry);
  const skull = new THREE.Mesh(
    skullGeometry,
    new THREE.MeshStandardMaterial({
      color: 0x7fb3d5,
      emissive: 0x1a3a52,
      emissiveIntensity: 0.6,
      roughness: 0.35,
      metalness: 0.15,
      transparent: true,
      opacity: 0.92,
    }),
  );
  skull.scale.set(0.9, 1.08, 0.92);
  skull.name = "BuiltinSkull";
  group.add(skull);

  group.add(buildBrow(-0.26));
  group.add(buildBrow(0.26));
  group.add(buildEye(-0.26));
  group.add(buildEye(0.26));
  group.add(buildNose());

  const mouth = buildMouth();
  // 放在鼻子下方。**五官的 z 都要按揉过之后的脸面算**，不能按正球 ——
  // 收下巴让脸面整体后退了近 0.15，照原值摆会让五官浮在脸前面
  mouth.position.set(0, -0.42, 0.73);
  group.add(mouth);

  return group;
}
