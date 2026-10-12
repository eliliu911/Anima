import { describe, expect, it } from "vitest";
import * as THREE from "three";

import { humanBoneByNodeName, retargetVrma } from "./vrma";
import type { Bones } from "./pose";

/** 造一份 VRMA：节点名带点（三会清洗），轨道绑在它自己的节点上 */
function vrma(entries: Array<[standard: string, node: string]>, withPosition = false) {
  const nodes = entries.map(([, node]) => ({ name: node }));
  const humanBones: Record<string, { node: number }> = {};
  entries.forEach(([standard], i) => (humanBones[standard] = { node: i }));

  const tracks: THREE.KeyframeTrack[] = entries.map(([, node]) => {
    const clean = THREE.PropertyBinding.sanitizeNodeName(node);
    return new THREE.QuaternionKeyframeTrack(
      `${clean}.quaternion`,
      [0, 1],
      [0, 0, 0, 1, 0, 0.3, 0, 0.95],
    );
  });
  if (withPosition) {
    const clean = THREE.PropertyBinding.sanitizeNodeName(entries[0][1]);
    tracks.push(new THREE.VectorKeyframeTrack(`${clean}.position`, [0, 1], [0, 0, 0, 0, 5, 0]));
  }

  return {
    scene: new THREE.Object3D(),
    animations: [new THREE.AnimationClip("挥手", 1, tracks)],
    parser: {
      json: { nodes, extensions: { VRMC_vrm_animation: { humanoid: { humanBones } } } },
    },
  };
}

function rig(): Bones {
  const make = (name: string) => {
    const o = new THREE.Object3D();
    o.name = name;
    return o;
  };
  return {
    head: make("16joint_Head"),
    neck: null,
    chest: null,
    spine: make("12joint_Torso"),
    hips: null,
    upperArmL: make("50joint_LeftArm"),
    upperArmR: make("20joint_RightArm"),
    lowerArmL: make("55joint_LeftElbow"),
    lowerArmR: make("25joint_RightElbow"),
  };
}

const ENTRIES: Array<[string, string]> = [
  ["hips", "Normalized_J_Bip_C_Hips"],
  ["spine", "Normalized_J_Bip_C_Spine"],
  ["rightUpperArm", "Normalized_J_Bip_R_UpperArm"],
  ["rightLowerArm", "Normalized_J_Bip_R_LowerArm"],
];

describe("VRMA 重定向", () => {
  it("读得出「节点名 → 标准骨骼名」，而且用的是清洗后的名字", () => {
    const map = humanBoneByNodeName(vrma([["leftLowerArm", "J.Bip.L.LowerArm"]]));
    // 文件里带点，three 里没有 —— 轨道名匹配的是后者
    expect(map.get("JBipLLowerArm")).toBe("leftLowerArm");
  });

  /**
   * **这条是这个功能的核心。**
   *
   * VRMA 的轨道绑在它自己文件的节点上，直接播的结果是一动不动**而且不报错** ——
   * three 找不到同名节点就静默跳过。必须改写成本模型骨骼的 uuid。
   */
  it("轨道被改写到本模型的骨骼上", () => {
    const bones = rig();
    const { clip, bound } = retargetVrma(vrma(ENTRIES), bones);
    expect(clip).not.toBeNull();
    const names = clip!.tracks.map((t) => t.name);
    expect(names).toContain(`${bones.lowerArmR!.uuid}.quaternion`);
    expect(names).toContain(`${bones.upperArmR!.uuid}.quaternion`);
    expect(names).toContain(`${bones.spine!.uuid}.quaternion`);
    expect(bound).toBe(3); // hips 我们不驱动
  });

  it("位移轨道被丢掉 —— 照搬别人的位移会让模型飘起来或陷进地里", () => {
    const { clip } = retargetVrma(vrma(ENTRIES, true), rig());
    expect(clip!.tracks.every((t) => t.name.endsWith(".quaternion"))).toBe(true);
  });

  it("模型缺的骨头记进 skipped，而不是静默丢掉", () => {
    const bones = rig();
    bones.lowerArmR = null;
    const { skipped } = retargetVrma(vrma(ENTRIES), bones);
    expect(skipped).toContain("rightLowerArm");
  });

  it("不是 VRMA（没有那个扩展）时返回 null，让调用方退回生成的手势", () => {
    const fake = {
      scene: new THREE.Object3D(),
      animations: [
        new THREE.AnimationClip("x", 1, [
          new THREE.QuaternionKeyframeTrack("SomeBone.quaternion", [0, 1], [0, 0, 0, 1, 0, 0, 0, 1]),
        ]),
      ],
      parser: { json: { nodes: [{ name: "SomeBone" }], extensions: {} } },
    };
    expect(retargetVrma(fake, rig()).clip).toBeNull();
  });

  it("文件里没有动画时不抛异常", () => {
    const empty = { scene: new THREE.Object3D(), animations: [], parser: { json: {} } };
    expect(() => retargetVrma(empty, rig())).not.toThrow();
    expect(retargetVrma(empty, rig()).clip).toBeNull();
  });

  it("时长保留 —— 播一半就切回 idle 会看起来像卡住", () => {
    const { clip } = retargetVrma(vrma(ENTRIES), rig());
    expect(clip!.duration).toBe(1);
  });
});

/**
 * `animations/Greeting.vrma` 里真实的骨骼清单（用 node 直接读出来的）。
 *
 * **不能用我自己编的名字。** 编出来的清单只能证明代码做了我写的事，
 * 证明不了下载来的动作文件里到底有什么 —— 这个项目已经在这上面栽过两次。
 */
const REAL_VRMA_BONES = [
  "hips", "spine", "chest", "upperChest", "neck", "head",
  "leftShoulder", "leftUpperArm", "leftLowerArm", "leftHand",
  "rightShoulder", "rightUpperArm", "rightLowerArm", "rightHand",
  "leftUpperLeg", "leftLowerLeg", "leftFoot", "leftToes",
  "rightUpperLeg", "rightLowerLeg", "rightFoot", "rightToes",
  "leftThumbProximal", "leftIndexProximal", "leftMiddleProximal",
  "rightThumbProximal", "rightIndexProximal", "rightMiddleProximal",
];

describe("全量骨骼重定向", () => {
  const real = () => vrma(REAL_VRMA_BONES.map((b) => [b, `Normalized_${b}`]));

  /** 模型侧的全量 humanoid 表 */
  function fullRig(): Map<string, THREE.Object3D> {
    const m = new Map<string, THREE.Object3D>();
    for (const b of REAL_VRMA_BONES) {
      const o = new THREE.Object3D();
      o.name = b;
      m.set(b, o);
    }
    return m;
  }

  /// 这是"角度刁钻 + 僵尸"的根因测试。只认那 10 根的话，
  /// 每个动画丢掉的正好是决定自然感的骨头
  it("给了全量表就全都接上，而不是只接 10 根", () => {
    const withAll = retargetVrma(real(), rig(), fullRig());
    const without = retargetVrma(real(), rig());
    expect(withAll.bound).toBe(REAL_VRMA_BONES.length);
    expect(without.bound).toBeLessThanOrEqual(10);
    expect(withAll.bound).toBeGreaterThan(without.bound * 2.5);
  });

  /// **肩膀是角度错得最离谱的那一根。** VRMA 里大臂的旋转是叠在肩膀之上
  /// 写的，肩膀不转、只转大臂，胳膊就落到完全不同的位置
  it("肩膀接上了", () => {
    // **同一份 all**。造两份的话 uuid 是两批，断言恒真、测试白绿
    const all = fullRig();
    const { clip } = retargetVrma(real(), rig(), all);
    const bound = new Set(clip!.tracks.map((t) => t.name.split(".")[0]));
    expect(bound.has(all.get("leftShoulder")!.uuid)).toBe(true);
    expect(bound.has(all.get("rightShoulder")!.uuid)).toBe(true);
  });

  /// 腿不动就是僵尸。Squat（蹲下）丢光腿骨的话人根本蹲不下去
  it("下半身和手指也接上了", () => {
    const all = fullRig();
    const { clip } = retargetVrma(real(), rig(), all);
    const names = new Set(clip!.tracks.map((t) => t.name.split(".")[0]));
    for (const b of ["leftUpperLeg", "rightFoot", "leftHand", "rightIndexProximal"]) {
      expect(names.has(all.get(b)!.uuid), `${b} 没接上`).toBe(true);
    }
  });

  /// upperChest 和 chest 是**两根不同的骨头**。都塞进同一个字段的话，
  /// 两条轨道写同一根骨骼，躯干旋转会打架
  it("chest 和 upperChest 不再挤同一根骨头", () => {
    const all = fullRig();
    const { clip } = retargetVrma(real(), rig(), all);
    const bound = clip!.tracks.map((t) => t.name.split(".")[0]);
    expect(bound).toContain(all.get("chest")!.uuid);
    expect(bound).toContain(all.get("upperChest")!.uuid);
    expect(new Set(bound).size).toBe(bound.length);
  });

  /// 模型缺某根骨头时不能整个失败 —— 退回它有的那些
  it("模型缺骨头时只跳过那几根", () => {
    const partial = fullRig();
    partial.delete("leftToes");
    partial.delete("rightToes");
    const { clip, bound, skipped } = retargetVrma(real(), rig(), partial);
    expect(clip).not.toBeNull();
    expect(bound).toBe(REAL_VRMA_BONES.length - 2);
    expect(skipped).toEqual(expect.arrayContaining(["leftToes", "rightToes"]));
  });
});
