import * as THREE from "three";
import { aimedQuat } from "./pose";
import type { Bones, IdleState } from "./pose";

/**
 * 代码生成的手势动作片段。
 *
 * ## 为什么动作会显得僵硬（这一版修的就是它）
 *
 * 原先每个动作只有 3~4 个关键帧：静止 → 目标 → 保持 → 静止。
 * three 的 `QuaternionKeyframeTrack` 在关键帧之间做的是**匀速 slerp**，
 * 于是每一段都是"以恒定角速度出发、到点硬停"—— 那正是机器人的运动特征。
 * 真人的肢体动作有三件事它一件都没有：
 *
 *   1. **缓动**：起步慢、中间快、到位前减速。
 *   2. **预备动作（anticipation）**：抬手之前先有一个极小的反向下沉。
 *   3. **过冲与回稳（overshoot / settle）**：到位时略微越过再荡回来。
 *
 * 这三件事都没法用"多加两个关键帧"糊过去，因为关键帧之间仍然是匀速的。
 * 所以这里改成**按曲线密集采样**：把每一段按 `SAMPLES_PER_SEGMENT` 采样，
 * 采样点之间的匀速插值就足够密、看不出折线了。
 *
 * 代价是轨道数据变大（一个动作几十个关键帧而不是四个），
 * 但这些片段是运行时生成、只在内存里活几秒，无所谓。
 */

/** 每段（两个姿势之间）采样多少个中间帧。12 帧足够让缓动看不出台阶 */
const SAMPLES_PER_SEGMENT = 12;

function makeQuat(x: number, y: number, z: number): THREE.Quaternion {
  return new THREE.Quaternion().setFromEuler(new THREE.Euler(x, y, z));
}

/** 缓入缓出。起步和到位都减速，这是"像人"和"像机器"最主要的分界 */
function easeInOut(t: number): number {
  return t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2;
}

/**
 * 缓入缓出 + 两端各带一点"回勾"。用于"抬起到位"那一段。
 *
 * 起点处轻微反向（天然的预备动作），终点处轻微越过再回来（过冲与回稳）。
 * **不要用 easeOutBack** —— 那是缓出曲线，第一帧就走掉三成行程，
 * 等于从静止瞬间加速到最大，那本身就是僵硬的来源（这条被测试抓到过）。
 */
function easeInOutBack(t: number): number {
  const c2 = 1.0 * 1.525;
  return t < 0.5
    ? (Math.pow(2 * t, 2) * ((c2 + 1) * 2 * t - c2)) / 2
    : (Math.pow(2 * t - 2, 2) * ((c2 + 1) * (t * 2 - 2) + c2) + 2) / 2;
}

type Ease = (t: number) => number;

/** 世界坐标的下方向。预备动作要往这边沉一点 */
const DOWN = new THREE.Vector3(0, -1, 0);
/** 摆手绕的轴 */
const AXIS_Y = new THREE.Vector3(0, 1, 0);

/** 一个姿势关键点：到达时刻、目标四元数、用什么曲线从上一个点过来 */
interface Key {
  at: number;
  quat: THREE.Quaternion;
  ease?: Ease;
}

/**
 * 把姿势关键点展开成密集采样的四元数轨道。
 *
 * 停留段（前后两个关键点姿势相同）不需要采样，直接两个端点即可 ——
 * 匀速插值两个相同的值仍然是那个值。
 */
function sampledTrack(bone: THREE.Object3D, keys: Key[]): THREE.QuaternionKeyframeTrack {
  const times: number[] = [];
  const values: number[] = [];
  const push = (t: number, q: THREE.Quaternion) => {
    times.push(t);
    values.push(q.x, q.y, q.z, q.w);
  };

  push(keys[0].at, keys[0].quat);
  for (let i = 1; i < keys.length; i++) {
    const from = keys[i - 1];
    const to = keys[i];
    const ease = to.ease ?? easeInOut;
    const still = from.quat.angleTo(to.quat) < 1e-4;
    const steps = still ? 1 : SAMPLES_PER_SEGMENT;
    for (let s = 1; s <= steps; s++) {
      const u = s / steps;
      const q = from.quat.clone().slerp(to.quat, ease(u));
      push(from.at + (to.at - from.at) * u, q);
    }
  }

  // 绑定用 uuid：GLTFLoader 会清洗节点名（`upper_arm.L` → `upper_armL`），
  // 按名字绑定在不同模型上会时灵时不灵
  return new THREE.QuaternionKeyframeTrack(`${bone.uuid}.quaternion`, times, values);
}

export function generateAnimation(
  action: string,
  bones: Bones,
  restPose: IdleState,
): THREE.AnimationClip | null {
  const tracks: THREE.KeyframeTrack[] = [];
  const rest = (bone: THREE.Object3D | null): THREE.Quaternion =>
    (bone && restPose.rest.get(bone)?.clone()) || new THREE.Quaternion();

  /**
   * 给一根骨头排一段"抬起 → 保持 → 放下"。
   *
   * `anticipation` 是抬起前那个极小的反向预备动作 —— 幅度只有主动作的
   * 十几分之一、时长 0.15 秒，单看几乎看不见，但去掉之后动作立刻"从静止
   * 瞬间启动"，就是僵硬感的来源之一。
   */
  const gesture = (
    bone: THREE.Object3D | null,
    target: THREE.Quaternion,
    opts: { rise: number; hold: number; fall: number; anticipation?: THREE.Quaternion },
  ) => {
    if (!bone) return;
    const r = rest(bone);
    const keys: Key[] = [{ at: 0, quat: r }];
    let t = 0;
    if (opts.anticipation) {
      t += 0.15;
      keys.push({ at: t, quat: opts.anticipation });
    }
    t += opts.rise;
    // 到位这一下带一点过冲，然后靠下一段回稳
    keys.push({ at: t, quat: target, ease: easeInOutBack });
    t += opts.hold;
    keys.push({ at: t, quat: target });
    t += opts.fall;
    keys.push({ at: t, quat: r });
    tracks.push(sampledTrack(bone, keys));
  };

  /** 摆动：在两个姿势之间来回若干次（挥手用） */
  const swing = (
    bone: THREE.Object3D | null,
    a: THREE.Quaternion,
    b: THREE.Quaternion,
    opts: { start: number; period: number; cycles: number; end: number },
  ) => {
    if (!bone) return;
    const r = rest(bone);
    const keys: Key[] = [{ at: 0, quat: r }];
    let t = opts.start;
    keys.push({ at: t, quat: a });
    for (let i = 0; i < opts.cycles; i++) {
      t += opts.period / 2;
      keys.push({ at: t, quat: b });
      t += opts.period / 2;
      keys.push({ at: t, quat: a });
    }
    t += opts.end;
    keys.push({ at: t, quat: r });
    tracks.push(sampledTrack(bone, keys));
  };

  const act = action.trim().toLowerCase();

  /**
   * 摆一组姿势，返回每根骨头该到达的局部四元数。
   *
   * **必须按父到子的顺序依次摆、边摆边算**：小臂朝哪儿取决于大臂已经转到
   * 哪儿了。一次性各算各的，小臂会按"大臂还在原位"去定向，肘部于是拐向
   * 一个谁也没想要的方向。算完把骨头恢复原状，这里只负责产出目标值。
   */
  const aimPose = (
    entries: Array<[THREE.Object3D | null, THREE.Vector3]>,
  ): Map<THREE.Object3D, THREE.Quaternion> => {
    const out = new Map<THREE.Object3D, THREE.Quaternion>();
    const saved = new Map<THREE.Object3D, THREE.Quaternion>();
    for (const [bone] of entries) {
      if (bone && !saved.has(bone)) saved.set(bone, bone.quaternion.clone());
    }
    for (const [bone, dir] of entries) {
      if (!bone) continue;
      const q = aimedQuat(bone, dir, null);
      if (!q) continue;
      bone.quaternion.copy(q);
      bone.updateWorldMatrix(true, true);
      out.set(bone, q.clone());
    }
    for (const [bone, q] of saved) {
      bone.quaternion.copy(q);
      bone.updateWorldMatrix(true, true);
    }
    return out;
  };

  /** 摆好的目标，取不到就退回静止姿势（那根骨头不动，而不是乱转） */
  const at = (
    pose: Map<THREE.Object3D, THREE.Quaternion>,
    bone: THREE.Object3D | null,
  ): THREE.Quaternion => (bone && pose.get(bone)?.clone()) || rest(bone);

  if (act === "stretch" || act === "伸懒腰") {
    // 伸懒腰就是往头顶伸 —— 手臂指向斜上方，稍微朝外
    const pose = aimPose([
      [bones.upperArmL, new THREE.Vector3(-0.3, 1, 0)],
      [bones.upperArmR, new THREE.Vector3(0.3, 1, 0)],
      // 小臂顺着继续往上，别在肘部折出一个角
      [bones.lowerArmL, new THREE.Vector3(-0.15, 1, 0)],
      [bones.lowerArmR, new THREE.Vector3(0.15, 1, 0)],
    ]);
    // 伸懒腰是慢动作：抬得慢、停得久、放得更慢
    const slow = { rise: 1.1, hold: 0.9, fall: 1.0 };
    // 预备：先往下沉一点点再往上伸。幅度只有主动作的十几分之一
    const dipL = aimedQuat(bones.upperArmL!, DOWN, 0.12) ?? undefined;
    const dipR = aimedQuat(bones.upperArmR!, DOWN, 0.12) ?? undefined;
    gesture(bones.upperArmL, at(pose, bones.upperArmL), { ...slow, anticipation: dipL });
    gesture(bones.upperArmR, at(pose, bones.upperArmR), { ...slow, anticipation: dipR });
    gesture(bones.lowerArmL, at(pose, bones.lowerArmL), slow);
    gesture(bones.lowerArmR, at(pose, bones.lowerArmR), slow);
    gesture(bones.spine, makeQuat(0.12, 0, 0), slow);
    return new THREE.AnimationClip("stretch", 3.15, tracks);
  }

  if (act === "cover_mouth" || act === "捂嘴" || act === "捂嘴笑" || act === "惊讶") {
    // 手要能够到嘴：大臂略微抬起并前送，小臂折起来朝身体中线和上方
    const pose = aimPose([
      [bones.upperArmR, new THREE.Vector3(0.55, -0.55, 0.6)],
      [bones.lowerArmR, new THREE.Vector3(-0.5, 0.75, 0.4)],
    ]);
    // 捂嘴是快动作：手"啪"地抬上来，但落下要慢
    const quick = { rise: 0.35, hold: 1.1, fall: 0.7 };
    gesture(bones.upperArmR, at(pose, bones.upperArmR), quick);
    gesture(bones.lowerArmR, at(pose, bones.lowerArmR), quick);
    return new THREE.AnimationClip("cover_mouth", 2.2, tracks);
  }

  if (act === "wave" || act === "挥手" || act === "打招呼") {
    // 招手：大臂抬到侧上方，小臂立起来，手在头旁边
    const pose = aimPose([
      [bones.upperArmR, new THREE.Vector3(0.75, 0.35, 0.15)],
      [bones.lowerArmR, new THREE.Vector3(0.3, 0.94, 0.15)],
    ]);
    gesture(bones.upperArmR, at(pose, bones.upperArmR), { rise: 0.4, hold: 1.5, fall: 0.5 });
    // 摆手是**从立起来的位置**左右摆，不是从静止姿势摆
    const upright = at(pose, bones.lowerArmR);
    const tilt = (rad: number) =>
      upright.clone().multiply(new THREE.Quaternion().setFromAxisAngle(AXIS_Y, rad));
    swing(bones.lowerArmR, tilt(0.32), tilt(-0.32), {
      start: 0.4,
      period: 0.5,
      cycles: 3,
      end: 0.5,
    });
    return new THREE.AnimationClip("wave", 2.4, tracks);
  }

  if (act === "heart" || act === "比心" || act === "爱心") {
    // 比心：两手在**胸前**合拢。以前小臂折 114° 直接把手送到头顶去了
    const pose = aimPose([
      [bones.upperArmL, new THREE.Vector3(-0.5, -0.6, 0.62)],
      [bones.upperArmR, new THREE.Vector3(0.5, -0.6, 0.62)],
      [bones.lowerArmL, new THREE.Vector3(0.62, 0.45, 0.65)],
      [bones.lowerArmR, new THREE.Vector3(-0.62, 0.45, 0.65)],
    ]);
    const opts = { rise: 0.55, hold: 1.4, fall: 0.6 };
    for (const b of [bones.upperArmL, bones.upperArmR, bones.lowerArmL, bones.lowerArmR]) {
      gesture(b, at(pose, b), opts);
    }
    return new THREE.AnimationClip("heart", 2.55, tracks);
  }

  console.warn("[Animation] 不认识的动作：", action);
  return null;
}
