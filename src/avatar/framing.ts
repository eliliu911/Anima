/**
 * 相机取景：给定「脸在哪儿」和「整个模型多大」，算出相机该放哪儿。
 *
 * ## 为什么单独成文件
 *
 * 取景全是几何、纯函数，本来就该单独测。而它已经错过四次，每次都只在画面上
 * 表现成"看着不对"，反推不到原因：
 *
 * 1. 按整体包围盒 +「高 > 宽 × 2 就当全身」猜头在哪儿 —— T-pose 的双臂展开
 *    让宽度和身高差不多，判断失效，整个人被框进画面、脸只剩几十个像素。
 * 2. 拿「脸盒」当头高 —— 有的模型整个身体是一个网格、口型也挂在上面，
 *    脸盒等于全身盒，相机被推到 5.79 米。
 * 3. 按脸盒推头顶 —— 头发和头饰是**单独的网格、没有口型通道**，脸盒顶端只到
 *    发际线，头顶被切掉一截。
 * 4. 把头顶抬到模型顶端、头高却还用脸盒的 —— 画框整体上移，胸口被切掉，
 *    只剩一个头。
 *
 * ## 所以改成同时钉住上下两条边
 *
 * 前面四次都是"先定画框多高、再去摆它"，于是总有一头顾不上。现在反过来，
 * 先定两条边，画框高度由它们决定：
 *
 * - **上沿**：真实的最高点（模型包围盒顶端，也就是头发/头饰），加一点留白。
 * - **下沿**：颅底往下 `BODY_HEADS` 个头高，大约到胸口。
 *
 * 头饰特别高的模型会把画框**撑大**（人显得小一点），而不是把身体挤出画面。
 */
import * as THREE from "three";

/**
 * 头发让头看起来比「脸」高多少倍。
 *
 * 「脸盒」是带口型通道的那些网格算出来的，不含头发 —— 只到发际线附近。
 * 1.35 按常见的二次元模型比例取：头发大致给整个头加了三分之一高度。
 */
const HAIR = 1.35;

/**
 * 下沿落在颅底往下几个头高 —— **这是"远近"那个旋钮**。
 *
 * 小 = 画框短 = 相机更近、人更大。1.25 大约到锁骨下方。
 */
const BODY_HEADS = 1.25;

/** 颅底到发顶大约占多少个头高。没有可信最高点时用它估头顶 */
const CROWN = 0.75;

/**
 * 上沿再往上留多少（按头高的比例）—— **这是"上下"那个旋钮**。
 *
 * 大 = 头顶上方留白更多 = 人在画面里坐得更低，等效于把相机抬高。
 */
const TOP_MARGIN = 0.3;

/** 画面留白：算出来的取景高度再乘这个系数 */
const MARGIN = 1.04;

export interface Framing {
  /** 相机看向哪儿 */
  center: THREE.Vector3;
  /** 相机离 center 多远 */
  distance: number;
}

/**
 * 算出半身像的取景。
 *
 * @param headBox  脸的包围盒（带口型通道的那些 mesh 算出来的）
 * @param modelBox 整个模型的包围盒
 * @param fovDeg   相机的垂直视角
 * @param aspect   画面宽高比。窄画面要按宽度退，否则左右会被切掉
 * @param headBonePos 头骨骼的世界坐标。**蒙皮网格的包围盒位置不可靠**
 *        （几何体在绑定空间里，网格节点常是单位变换），有骨骼就以它定位；
 *        尺寸仍用包围盒 —— 绑定姿势的几何**大小**是对的，只是位置可能不对。
 */
export function portraitFraming(
  headBox: THREE.Box3,
  modelBox: THREE.Box3,
  fovDeg: number,
  aspect = 1,
  headBonePos?: THREE.Vector3,
  /**
   * 往下取几个头高。默认 [`BODY_HEADS`]（到锁骨，半身像）。
   *
   * **桌宠窗口要传大得多的值**：那个窗口又窄又高（360×520），
   * 半身像会让她缩在中间一小块，上下大片留白 —— 看起来"离待机状态很远"。
   * 传 4~5 大约是到腰/膝，正好把窗口填满。
   */
  bodyHeads = BODY_HEADS,
): Framing {
  const headSize = headBox.getSize(new THREE.Vector3());
  const modelHeight = Math.max(modelBox.getSize(new THREE.Vector3()).y, 1e-4);

  // 「脸盒」不一定真的只框住脸：有的模型整个身体是一个网格、口型也挂在上面。
  // 头不可能占全身三分之一以上 —— 超了就说明这个盒子不可信，
  // 退回人体比例（一个头约占身高七分之一）
  const faceHeight =
    headSize.y > modelHeight * 0.35
      ? modelHeight * 0.14
      : Math.max(headSize.y, 1e-4);
  const headHeight = faceHeight * HAIR;

  // ── 上沿 ──
  const anatomicalTop = headBonePos
    ? headBonePos.y + headHeight * CROWN
    : headBox.max.y;
  // 防一手：手举过头顶、或者有悬空挂件时，包围盒顶端就不是头了
  const topIsHead = modelBox.max.y < anatomicalTop + headHeight * 1.5;
  const top =
    (topIsHead ? Math.max(modelBox.max.y, anatomicalTop) : anatomicalTop) +
    headHeight * TOP_MARGIN;

  // ── 下沿 ──
  const chinRef = headBonePos ? headBonePos.y : headBox.min.y + faceHeight * 0.5;
  // 只有头的模型不能硬往下退 —— 那样画面一大半是空的，头反而更小
  const bottom = Math.max(chinRef - headHeight * bodyHeads, modelBox.min.y);

  const shot = Math.max(top - bottom, headHeight * 0.5);
  const centerY = top - shot * 0.5;

  const fov = (fovDeg * Math.PI) / 180;
  let distance = (shot * MARGIN) / (2 * Math.tan(fov / 2));
  // 横向也要装得下：窄窗口下按高度算出来的距离会把肩膀切掉
  const width = Math.max(headSize.x * 1.8, headHeight);
  const horizontal = width / (2 * Math.tan(fov / 2) * Math.max(aspect, 0.01));
  distance = Math.max(distance, horizontal);

  const center = headBonePos
    ? new THREE.Vector3(headBonePos.x, centerY, headBonePos.z)
    : headBox.getCenter(new THREE.Vector3()).setY(centerY);

  return { center, distance };
}
