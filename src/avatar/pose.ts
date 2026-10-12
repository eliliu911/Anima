import * as THREE from "three";
import type { EmotionState } from "../store/voice";

export interface Bones {
  head: THREE.Object3D | null;
  neck: THREE.Object3D | null;
  chest: THREE.Object3D | null;
  spine: THREE.Object3D | null;
  hips: THREE.Object3D | null;
  upperArmL: THREE.Object3D | null;
  upperArmR: THREE.Object3D | null;
  lowerArmL: THREE.Object3D | null;
  lowerArmR: THREE.Object3D | null;
}

const PATTERNS: Record<keyof Bones, { any: string[]; side?: "l" | "r" }> = {
  head: { any: ["head", "首", "头"] },
  neck: { any: ["neck", "颈"] },
  chest: { any: ["upperchest", "upper_chest", "chest", "胸"] },
  spine: { any: ["spine", "脊椎"] },
  hips: { any: ["hips", "pelvis", "腰"] },
  upperArmL: { any: ["upperarm", "upper_arm", "leftarm", "arm_l", "arm.l", "bip_l_arm", "左腕", "左肩", "arm"], side: "l" },
  upperArmR: { any: ["upperarm", "upper_arm", "rightarm", "arm_r", "arm.r", "bip_r_arm", "右腕", "右肩", "arm"], side: "r" },
  lowerArmL: { any: ["lowerarm", "lower_arm", "forearm", "leftforearm", "forearm_l", "forearm.l", "bip_l_forearm", "elbow", "左ひじ", "左肘", "左前腕", "forearm"], side: "l" },
  lowerArmR: { any: ["lowerarm", "lower_arm", "forearm", "rightforearm", "forearm_r", "forearm.r", "bip_r_forearm", "elbow", "右ひじ", "右肘", "右前腕", "forearm"], side: "r" },
};

/**
 * 名字里表示左右的写法。
 *
 * 最后那条（结尾的大写 `L`/`R`，前面没有分隔符）**不是为了兼容某种怪命名**，
 * 而是因为 **GLTFLoader 会清洗节点名**：`PropertyBinding.sanitizeNodeName`
 * 把 `.` 之类的保留字符去掉，于是文件里写着 `upper_arm.L` 的骨头，
 * 到了 three 里叫 `upper_armL`。
 *
 * 实测 momo.vrm 就是这样，胳膊一直平举着。而我第一次排查时是拿 glTF 文件里的
 * **原始**名字去验的，看到 `upper_arm.L` 带分隔符就判断"这条没问题" ——
 * 验错了对象。要对着产品实际拿到的数据验。
 *
 * 结尾那条只认**大写**：小写会把 `Tail`、`Ctrl` 这类正常名字误判成左侧。
 */
function sideOf(name: string): "l" | "r" | null {
  const n = name.toLowerCase();
  if (/(^|[._:\-\s])l([._:\-\s]|$)/.test(n) || n.includes("left") || n.includes("左") || n.endsWith("_l") || n.endsWith(".l") || n.startsWith("l_") || n.startsWith("l.")) return "l";
  if (/(^|[._:\-\s])r([._:\-\s]|$)/.test(n) || n.includes("right") || n.includes("右") || n.endsWith("_r") || n.endsWith(".r") || n.startsWith("r_") || n.startsWith("r.")) return "r";
  // 分隔符被 GLTFLoader 清掉之后的样子：upper_armL / shoulderR
  const tail = /[a-z0-9]([LR])$/.exec(name);
  if (tail) return tail[1] === "L" ? "l" : "r";
  return null;
}

/**
 * 找骨骼。
 *
 * `hint` 是**权威映射**（VRM 的 humanoid 扩展，见 humanoid.ts）：有就直接用，
 * 名字匹配只用来补它没给的那些。名字匹配漏了不会报错，只会让那根骨头不动 ——
 * 星穹铁道那批模型的小臂叫 `joint_LeftElbow`，匹配表里没有 elbow，
 * 结果就是"胳膊全程笔直"。
 */
export function findBones(root: THREE.Object3D, hint?: Partial<Bones>): Bones {
  const found: Bones = {
    head: null,
    neck: null,
    chest: null,
    spine: null,
    hips: null,
    upperArmL: null,
    upperArmR: null,
    lowerArmL: null,
    lowerArmR: null,
  };

  const skinBones: THREE.Object3D[] = [];
  root.traverse((obj) => {
    const skinned = obj as THREE.SkinnedMesh;
    if (skinned.isSkinnedMesh && skinned.skeleton) {
      skinBones.push(...skinned.skeleton.bones);
    }
  });

  const nodes: THREE.Object3D[] = [];
  const seen = new Set<THREE.Object3D>();
  const push = (obj: THREE.Object3D) => {
    if (obj.name && !seen.has(obj)) {
      seen.add(obj);
      nodes.push(obj);
    }
  };
  if (skinBones.length) skinBones.forEach(push);
  else root.traverse(push);

  for (const [key, spec] of Object.entries(PATTERNS) as Array<
    [keyof Bones, (typeof PATTERNS)[keyof Bones]]
  >) {
    // 权威映射优先。名字匹配只补它没给的
    const authoritative = hint?.[key];
    if (authoritative) {
      found[key] = authoritative;
      continue;
    }
    for (const kw of spec.any) {
      const hit = nodes.find((obj) => {
        if (spec.side && sideOf(obj.name) !== spec.side) return false;
        return obj.name.toLowerCase().includes(kw);
      });
      if (hit) {
        found[key] = hit;
        break;
      }
    }
  }

  return found;
}

const _axis = new THREE.Vector3();
const _q = new THREE.Quaternion();
const _parentQ = new THREE.Quaternion();
const _boneQ = new THREE.Quaternion();
const DOWN = new THREE.Vector3(0, -1, 0);

function findChildBone(bone: THREE.Object3D): THREE.Object3D | null {
  for (const child of bone.children) {
    if ((child as THREE.Bone).isBone) return child;
    const sub = findChildBone(child);
    if (sub) return sub;
  }
  return bone.children[0] ?? null;
}

/** 这根骨头现在在世界里指向哪（骨头 → 它第一个子骨头的方向） */
export function boneDirection(bone: THREE.Object3D): THREE.Vector3 | null {
  const child = findChildBone(bone);
  if (!child) return null;
  bone.updateWorldMatrix(true, false);
  child.updateWorldMatrix(true, false);
  const a = new THREE.Vector3().setFromMatrixPosition(bone.matrixWorld);
  const b = new THREE.Vector3().setFromMatrixPosition(child.matrixWorld);
  const dir = b.sub(a);
  if (dir.lengthSq() < 1e-8) return null;
  return dir.normalize();
}

/**
 * 算出「把这根骨头**从它现在的朝向**往 `toward` 转 `angle` 弧度」之后的局部四元数。
 * 不写回骨头 —— 返回值给动画当关键帧用。
 *
 * `angle` 传 `null` 表示**一路转到底**，让骨头正好指向 `toward`。
 *
 * ## 为什么动作要用这个，而不是写死的欧拉角
 *
 * 写死的欧拉角（`makeQuat(0, 0, 0.5)`）是**相对绑定姿势**的，而 VRM 的绑定
 * 姿势是 T-pose（手臂平举）。但屏幕上的静止姿势是 [`relaxArms`](#relaxArms)
 * 把手臂往下压 65° 之后的样子。两者差了 65°。
 *
 * 后果是每个手势都先把手臂**拽回 T-pose 附近**再动 —— 挥个手像在做扩胸操，
 * 比心的小臂折到头顶去。用户的原话是"这是要起飞么"。
 *
 * 按方向定向就没有这个问题：不管模型的静止姿势是什么，"把手举到侧上方"
 * 永远是同一件事。
 */
export function aimedQuat(
  bone: THREE.Object3D,
  toward: THREE.Vector3,
  angle: number | null,
): THREE.Quaternion | null {
  const dir = boneDirection(bone);
  if (!dir) return null;

  const target = toward.clone().normalize();
  _axis.copy(dir).cross(target);
  // 已经指着那个方向了（或者正好反向），没有唯一的旋转轴
  if (_axis.lengthSq() < 1e-8) return bone.quaternion.clone();
  _axis.normalize();

  const full = Math.acos(THREE.MathUtils.clamp(dir.dot(target), -1, 1));
  _q.setFromAxisAngle(_axis, angle === null ? full : Math.min(angle, full));

  bone.parent?.getWorldQuaternion(_parentQ);
  bone.getWorldQuaternion(_boneQ);
  return new THREE.Quaternion()
    .copy(_parentQ.invert())
    .multiply(_q)
    .multiply(_boneQ);
}

function rotateToward(bone: THREE.Object3D, toward: THREE.Vector3, angle: number) {
  const q = aimedQuat(bone, toward, angle);
  if (q) bone.quaternion.copy(q);
}

const ARM_DROP = (65 * Math.PI) / 180;
const ELBOW_BEND = (10 * Math.PI) / 180;

export function relaxArms(bones: Bones) {
  for (const arm of [bones.upperArmL, bones.upperArmR]) {
    if (arm) rotateToward(arm, DOWN, ARM_DROP);
  }
  for (const arm of [bones.lowerArmL, bones.lowerArmR]) {
    if (arm) rotateToward(arm, DOWN, ELBOW_BEND);
  }
}

export interface IdleState {
  rest: Map<THREE.Object3D, THREE.Quaternion>;
}

export function captureRest(bones: Bones): IdleState {
  const rest = new Map<THREE.Object3D, THREE.Quaternion>();
  for (const bone of [
    bones.chest, bones.spine, bones.head, bones.neck,
    bones.upperArmL, bones.upperArmR, bones.lowerArmL, bones.lowerArmR
  ]) {
    if (bone) rest.set(bone, bone.quaternion.clone());
  }
  return { rest };
}

const _euler = new THREE.Euler();
const _tmp = new THREE.Quaternion();

export function idle(bones: Bones, state: IdleState, t: number, emotion?: EmotionState) {
  const energy = emotion?.energy ?? 0; // -100 to 100
  const mood = emotion?.mood ?? 0;
  const irritation = emotion?.irritation ?? 0;

  // 能量越高，呼吸越快，晃动越大
  const speedScale = 1.0 + (energy / 100) * 0.5;
  const breath = Math.sin(t * 1.15 * speedScale);
  const sway = Math.sin(t * 0.37 * speedScale);
  const drift = Math.sin(t * 0.23 * speedScale + 1.7);

  // 基础幅度放大系数
  const ampScale = 1.0 + (energy / 100) * 0.3;

  // 心情差时，略微含胸低头
  const moodBiasChestX = mood < 0 ? (-mood / 100) * 0.05 : 0;
  const moodBiasNeckX = mood < 0 ? (-mood / 100) * 0.08 : 0;
  // 愤怒时，稍微挺胸
  const angryBiasChestX = irritation > 0 ? -(irritation / 100) * 0.04 : 0;

  apply(state, bones.chest, breath * 0.035 * ampScale + moodBiasChestX + angryBiasChestX, 0, 0);
  apply(state, bones.spine, breath * 0.02 * ampScale, sway * 0.02 * ampScale, sway * 0.012 * ampScale);
  apply(state, bones.neck, drift * 0.03 * ampScale + moodBiasNeckX, sway * 0.05 * ampScale, drift * 0.02 * ampScale);
  apply(state, bones.head, drift * 0.04 * ampScale, sway * 0.07 * ampScale, -sway * 0.025 * ampScale);

  // **手臂也要每帧重新施加。**
  //
  // 不加这段的话，一个动作播完之后手臂会永远停在片段的最后一帧 ——
  // 因为 three 的 mixer **不会把它动过的骨骼还原**，而这里原本只管
  // 胸/脊柱/脖子/头。用户实测：放完一个 VRMA 之后人物双手举着不放下来。
  //
  // 幅度取得很小（是躯干的三分之一），只为让手臂有一点重量感，
  // 顺带把上一个动作的残留每帧擦干净。
  apply(state, bones.upperArmL, breath * 0.012 * ampScale, 0, sway * 0.01 * ampScale);
  apply(state, bones.upperArmR, breath * 0.012 * ampScale, 0, -sway * 0.01 * ampScale);
  apply(state, bones.lowerArmL, drift * 0.01 * ampScale, 0, 0);
  apply(state, bones.lowerArmR, drift * 0.01 * ampScale, 0, 0);
}

function apply(
  state: IdleState,
  bone: THREE.Object3D | null,
  x: number,
  y: number,
  z: number,
) {
  if (!bone) return;
  const rest = state.rest.get(bone);
  if (!rest) return;
  _euler.set(x, y, z);
  _tmp.setFromEuler(_euler);
  bone.quaternion.copy(rest).multiply(_tmp);
}
