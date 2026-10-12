import * as THREE from "three";
import type { Bones } from "./pose";

/**
 * 从 VRM 的 humanoid 映射里读骨骼，而不是靠名字猜。
 *
 * ## 为什么需要它
 *
 * `pose.ts` 是按关键词匹配骨骼名的（`lowerarm` / `forearm` / …）。这套办法
 * 对每一种新的命名习惯都要补一次表，而**漏了不会报错** —— 骨骼是 null，
 * 那根骨头就不动，画面上表现为"胳膊不会弯"。
 *
 * 实测撞到的就是这个：星穹铁道那批 VRM 把小臂叫 `55.joint_LeftElbow`，
 * 而匹配表里只有 lowerarm / forearm，没有 **elbow** —— 于是所有动作里
 * 只有大臂在转，胳膊全程笔直。
 *
 * 而 VRM 文件里本来就写着标准答案：`humanoid.humanBones` 把
 * `leftLowerArm` 之类的**标准名**直接映到节点。规格里定死的，不用猜。
 *
 * VRM 0.x 和 1.0 的结构不同（数组 vs 对象），两种都认。
 *
 * ## 名字要清洗
 *
 * 拿到节点下标之后要按名字去场景里找对象，而 **GLTFLoader 会清洗节点名**：
 * `PropertyBinding.sanitizeNodeName` 会去掉 `.` 等保留字符，于是文件里的
 * `55.joint_LeftElbow` 到 three 里叫 `55joint_LeftElbow`。
 * 拿原始名字去 `getObjectByName` 一个都找不到 —— 这个坑这个项目踩过两次。
 */

/** VRM 标准骨骼名 → 我们的字段名 */
const HUMAN_BONE_MAP: Record<string, keyof Bones> = {
  head: "head",
  neck: "neck",
  chest: "chest",
  upperChest: "chest",
  spine: "spine",
  hips: "hips",
  leftUpperArm: "upperArmL",
  rightUpperArm: "upperArmR",
  leftLowerArm: "lowerArmL",
  rightLowerArm: "lowerArmR",
};

interface GltfLike {
  scene: THREE.Object3D;
  parser?: { json?: unknown };
}

/**
 * 读出模型里**全部**标准骨骼，键是 VRM 的标准名（`leftShoulder` / `leftHand` /
 * `leftIndexProximal` / `rightUpperLeg` …）。
 *
 * ## 为什么要有这个，而不是只用上面那 10 根
 *
 * [`Bones`](./pose.ts) 只有 10 根，因为 idle（呼吸、摇晃）只需要那么多。
 * 但**动作重定向不该受这个限制** —— 它需要的是骨骼对象，不是一个有类型的字段。
 *
 * 实测代价：`animations/` 里那批 VRMA 每个动画 52 根骨骼，只用上 10 根，
 * **丢掉 42 根（81%）**，而且丢的正好是决定"像不像人"的那些：
 *
 * - **双肩**。VRMA 里大臂的旋转是**叠在肩膀之上**写的。肩膀不转、只转大臂，
 *   胳膊就落到完全不同的位置 —— 这就是"角度很刁钻"。
 * - **整个下半身**（大腿/小腿/脚/脚趾）。上半身在动、腿一动不动 ——
 *   这就是"跟僵尸一样"。`Squat.vrma`（蹲下）丢光腿骨，人根本蹲不下去。
 * - **30 根手指 + 双手**。手全程是块木头。
 *
 * 所以重定向走这张全量表，idle 继续用那 10 根。
 */
export function readAllHumanoidNodes(gltf: GltfLike): Map<string, THREE.Object3D> {
  const out = new Map<string, THREE.Object3D>();
  for (const [index, standard] of humanBoneIndices(gltf)) {
    const node = nodeAt(gltf, index);
    if (node) out.set(standard, node);
  }
  return out;
}

/** 标准名 → 节点下标。VRM 0.x（数组）和 1.0（对象）两种结构都认 */
function humanBoneIndices(gltf: GltfLike): Map<number, string> {
  const json = gltf.parser?.json as
    | {
        extensions?: {
          VRM?: { humanoid?: { humanBones?: Array<{ bone?: string; node?: number }> } };
          VRMC_vrm?: { humanoid?: { humanBones?: Record<string, { node?: number }> } };
        };
      }
    | undefined;
  const byIndex = new Map<number, string>();
  const v0 = json?.extensions?.VRM?.humanoid?.humanBones;
  if (Array.isArray(v0)) {
    for (const entry of v0) {
      if (entry?.bone && typeof entry.node === "number") byIndex.set(entry.node, entry.bone);
    }
  }
  const v1 = json?.extensions?.VRMC_vrm?.humanoid?.humanBones;
  if (v1 && !Array.isArray(v1)) {
    for (const [bone, ref] of Object.entries(v1)) {
      if (typeof ref?.node === "number") byIndex.set(ref.node, bone);
    }
  }
  return byIndex;
}

/** 按下标取场景里的节点。**名字要清洗过** —— 见文件顶部注释 */
function nodeAt(gltf: GltfLike, index: number): THREE.Object3D | null {
  const json = gltf.parser?.json as { nodes?: Array<{ name?: string }> } | undefined;
  const raw = json?.nodes?.[index]?.name;
  if (!raw) return null;
  const clean = THREE.PropertyBinding.sanitizeNodeName(raw);
  return gltf.scene.getObjectByName(clean) ?? gltf.scene.getObjectByName(raw) ?? null;
}

/**
 * 读出 VRM 的骨骼映射。不是 VRM、或者没有 humanoid 扩展时返回空对象 ——
 * 调用方退回按名字匹配。
 */
export function readHumanoidBones(gltf: GltfLike): Partial<Bones> {
  const json = gltf.parser?.json as
    | {
        nodes?: Array<{ name?: string }>;
        extensions?: {
          VRM?: { humanoid?: { humanBones?: Array<{ bone?: string; node?: number }> } };
          VRMC_vrm?: { humanoid?: { humanBones?: Record<string, { node?: number }> } };
        };
      }
    | undefined;
  if (!json?.nodes) return {};

  // 下标 → 标准骨骼名
  const byIndex = new Map<number, string>();
  const v0 = json.extensions?.VRM?.humanoid?.humanBones;
  if (Array.isArray(v0)) {
    for (const entry of v0) {
      if (entry?.bone && typeof entry.node === "number") byIndex.set(entry.node, entry.bone);
    }
  }
  const v1 = json.extensions?.VRMC_vrm?.humanoid?.humanBones;
  if (v1 && !Array.isArray(v1)) {
    for (const [bone, ref] of Object.entries(v1)) {
      if (typeof ref?.node === "number") byIndex.set(ref.node, bone);
    }
  }
  if (byIndex.size === 0) return {};

  const out: Partial<Bones> = {};
  for (const [index, standard] of byIndex) {
    const field = HUMAN_BONE_MAP[standard];
    if (!field) continue;
    const raw = json.nodes[index]?.name;
    if (!raw) continue;
    // 必须用清洗后的名字去找 —— 见文件顶部注释
    const clean = THREE.PropertyBinding.sanitizeNodeName(raw);
    const node = gltf.scene.getObjectByName(clean) ?? gltf.scene.getObjectByName(raw);
    if (!node) continue;
    // upperChest 会和 chest 抢同一个字段：优先 chest，别让 upperChest 覆盖它
    if (field === "chest" && standard === "upperChest" && out.chest) continue;
    out[field] = node;
  }
  return out;
}
