import { describe, expect, it } from "vitest";
import * as THREE from "three";

import { readHumanoidBones } from "./humanoid";
import { findBones } from "./pose";

/** 造一棵带这些节点名的场景，并附上对应的 glTF JSON */
function model(names: string[], extensions: Record<string, unknown>) {
  const scene = new THREE.Object3D();
  for (const raw of names) {
    const node = new THREE.Object3D();
    // GLTFLoader 会清洗节点名 —— 场景里存的是清洗后的
    node.name = THREE.PropertyBinding.sanitizeNodeName(raw);
    scene.add(node);
  }
  return {
    scene,
    parser: { json: { nodes: names.map((name) => ({ name })), extensions } },
  };
}

/** 星穹铁道那批 VRM 的真实命名：带数字前缀和点，小臂叫 Elbow */
const STAR_RAIL = [
  "12.joint_Torso",
  "16.joint_Head",
  "20.joint_RightArm",
  "25.joint_RightElbow",
  "50.joint_LeftArm",
  "55.joint_LeftElbow",
];

const VRM0 = {
  VRM: {
    humanoid: {
      humanBones: [
        { bone: "spine", node: 0 },
        { bone: "head", node: 1 },
        { bone: "rightUpperArm", node: 2 },
        { bone: "rightLowerArm", node: 3 },
        { bone: "leftUpperArm", node: 4 },
        { bone: "leftLowerArm", node: 5 },
      ],
    },
  },
};

describe("VRM humanoid 映射", () => {
  it("读得出 VRM 0.x 的骨骼", () => {
    const gltf = model(STAR_RAIL, VRM0);
    const bones = readHumanoidBones(gltf);
    expect(bones.lowerArmL?.name).toBe("55joint_LeftElbow");
    expect(bones.lowerArmR?.name).toBe("25joint_RightElbow");
    expect(bones.upperArmL?.name).toBe("50joint_LeftArm");
    expect(bones.head?.name).toBe("16joint_Head");
  });

  it("读得出 VRM 1.0 的骨骼（结构是对象不是数组）", () => {
    const gltf = model(["Spine", "L_Elbow"], {
      VRMC_vrm: { humanoid: { humanBones: { spine: { node: 0 }, leftLowerArm: { node: 1 } } } },
    });
    const bones = readHumanoidBones(gltf);
    expect(bones.spine?.name).toBe("Spine");
    expect(bones.lowerArmL?.name).toBe("L_Elbow");
  });

  it("不是 VRM 就返回空，让调用方退回按名字匹配", () => {
    expect(readHumanoidBones(model(["Armature", "Bone"], {}))).toEqual({});
  });

  /**
   * **这条是这次 bug 的回归测试。**
   *
   * 小臂叫 `joint_LeftElbow`，而名字匹配表里当时只有 lowerarm / forearm ——
   * 于是小臂骨骼是 null，所有动作里只有大臂在转，胳膊全程笔直。
   * 用户的原话是"感觉胳膊不会弯曲"。
   */
  it("权威映射能救回名字匹配漏掉的小臂", () => {
    const gltf = model(STAR_RAIL, VRM0);
    const guessed = findBones(gltf.scene);
    const authoritative = findBones(gltf.scene, readHumanoidBones(gltf));
    expect(authoritative.lowerArmL).not.toBeNull();
    expect(authoritative.lowerArmR).not.toBeNull();
    // 名字匹配现在也补了 elbow，所以两条路都能找到 —— 但权威映射不依赖补表
    expect(guessed.lowerArmL).not.toBeNull();
  });

  it("权威映射优先于名字匹配", () => {
    const gltf = model(STAR_RAIL, VRM0);
    const decoy = new THREE.Object3D();
    decoy.name = "假的_LeftLowerArm";
    gltf.scene.add(decoy);
    const bones = findBones(gltf.scene, readHumanoidBones(gltf));
    expect(bones.lowerArmL?.name).toBe("55joint_LeftElbow");
  });

  it("映射指向不存在的节点时跳过，不抛异常", () => {
    const gltf = model(["A"], { VRM: { humanoid: { humanBones: [{ bone: "head", node: 9 }] } } });
    expect(() => readHumanoidBones(gltf)).not.toThrow();
    expect(readHumanoidBones(gltf).head).toBeUndefined();
  });
});
