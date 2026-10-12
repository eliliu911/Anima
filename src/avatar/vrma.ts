import * as THREE from "three";
import type { Bones } from "./pose";

/**
 * 加载 `.vrma`（VRM Animation）并把它重定向到当前模型的骨骼。
 *
 * ## 为什么需要重定向
 *
 * VRMA 文件里的动画轨道绑的是**它自己文件里的节点**（`Normalized_J_Bip_C_Hips`
 * 之类），跟你的模型没有任何关系。直接 `mixer.clipAction(clip)` 的结果是
 * **一动不动而且不报错** —— three 找不到同名节点就静默跳过那条轨道。
 *
 * 但 VRMA 里有一张 `VRMC_vrm_animation.humanoid.humanBones` 映射表，
 * 把「标准骨骼名」（`leftLowerArm`…）指到它自己的节点。而我们这边有
 * [`readHumanoidBones`](./humanoid.ts) 能从模型里读出同一套标准名对应的骨骼。
 * 两张表一对，就能把轨道从「VRMA 的节点」改写成「你模型的骨骼」。
 *
 * ## 只重定向旋转
 *
 * 位移轨道（hips 的 translation）**故意丢掉**：不同模型身高、比例都不一样，
 * 照搬别人的位移会让人整个飘起来或者陷进地里。只用旋转的话，动作的形状保留，
 * 而落地位置永远是模型自己的。
 *
 * ## 静止姿势的差异
 *
 * VRMA 的旋转是相对 VRM 规范姿势（T-pose）的。而我们加载模型之后做过
 * `relaxArms`（把 T-pose 的手臂放下来）——那是**运行时改的**，骨骼的
 * 绑定姿势没变，所以把 VRMA 的旋转直接当局部旋转用是对的：
 * 动作播放期间手臂由动画完全接管，播完再淡回 idle。
 */

/**
 * 从 `Bones` 那 10 根兜底出来的标准名映射。
 *
 * **只在模型不给全量 humanoid 表的时候用。** 优先走
 * [`readAllHumanoidNodes`](./humanoid.ts) 的全量表 —— 只认这 10 根的话，
 * 每个 VRMA 会丢掉 42/52 根骨骼，其中包括双肩（角度全错）、
 * 整个下半身（上身动腿不动）、以及 30 根手指。
 */
const TO_FIELD: Record<string, keyof Bones> = {
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
  animations: THREE.AnimationClip[];
  parser?: { json?: unknown };
}

/** VRMA 里「节点名 → 标准骨骼名」 */
export function humanBoneByNodeName(gltf: GltfLike): Map<string, string> {
  const json = gltf.parser?.json as
    | {
        nodes?: Array<{ name?: string }>;
        extensions?: {
          VRMC_vrm_animation?: {
            humanoid?: { humanBones?: Record<string, { node?: number }> };
          };
        };
      }
    | undefined;
  const out = new Map<string, string>();
  const bones = json?.extensions?.VRMC_vrm_animation?.humanoid?.humanBones;
  if (!json?.nodes || !bones) return out;
  for (const [standard, ref] of Object.entries(bones)) {
    if (typeof ref?.node !== "number") continue;
    const raw = json.nodes[ref.node]?.name;
    if (!raw) continue;
    // three 会清洗节点名，而轨道名里存的就是清洗后的
    out.set(THREE.PropertyBinding.sanitizeNodeName(raw), standard);
  }
  return out;
}

export interface RetargetResult {
  clip: THREE.AnimationClip | null;
  /** 成功接上的骨骼数 */
  bound: number;
  /** 没能接上的标准骨骼名。模型缺这些骨头，或者我们不驱动它们 */
  skipped: string[];
}

/**
 * 把 VRMA 的动画重定向到给定的骨骼上。
 *
 * 失败时 `clip` 为 null 而不是抛异常 —— 调用方要能退回代码生成的手势。
 */
export function retargetVrma(
  gltf: GltfLike,
  bones: Bones,
  /**
   * 模型的**全量**标准骨骼表（`readAllHumanoidNodes`）。给了就用它 ——
   * 这是肩膀、手指、腿能不能动起来的唯一区别。没给才退回那 10 根
   */
  all?: Map<string, THREE.Object3D>,
): RetargetResult {
  const source = gltf.animations?.[0];
  if (!source) return { clip: null, bound: 0, skipped: [] };

  const byNode = humanBoneByNodeName(gltf);
  const tracks: THREE.KeyframeTrack[] = [];
  const skipped: string[] = [];
  const boundBones = new Set<THREE.Object3D>();

  for (const track of source.tracks) {
    // 轨道名形如 `节点名.quaternion`
    const dot = track.name.lastIndexOf(".");
    if (dot < 0) continue;
    const nodeName = track.name.slice(0, dot);
    const property = track.name.slice(dot + 1);
    // 只要旋转 —— 位移会让不同身高的模型飘起来或陷进地里
    if (property !== "quaternion") continue;

    const standard = byNode.get(nodeName);
    if (!standard) continue;
    // 全量表优先。**这一行就是"胳膊角度对不对、腿动不动"的分界** ——
    // 退回 TO_FIELD 的话每个动画丢掉 42/52 根骨骼
    const field = TO_FIELD[standard];
    const target = all?.get(standard) ?? (field ? bones[field] : null);
    if (!target) {
      skipped.push(standard);
      continue;
    }
    // 按 uuid 绑定：名字会被清洗，而且不同模型重名很常见
    const clone = track.clone();
    clone.name = `${target.uuid}.quaternion`;
    tracks.push(clone);
    boundBones.add(target);
  }

  if (tracks.length === 0) return { clip: null, bound: 0, skipped };
  return {
    clip: new THREE.AnimationClip(source.name || "vrma", source.duration, tracks),
    bound: boundBones.size,
    skipped,
  };
}
