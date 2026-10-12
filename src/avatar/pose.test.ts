/**
 * 站姿与呼吸的测试。
 *
 * 这里守的是一件在画面上很难判断对错的事：**旋转轴选错了会让胳膊往后翻或者
 * 拧成麻花**，而"胳膊看起来怪"这个观察没法反推到原因。所以断言直接量
 * 世界空间里的朝向：放下来之后，胳膊必须比原来更朝下。
 */
import { describe, expect, it } from "vitest";
import * as THREE from "three";
import { captureRest, findBones, idle, relaxArms, type Bones, type IdleState } from "./pose";

/**
 * 造一个 T-pose 骨架。
 *
 * 用 VRoid 的命名（`J_Bip_*`）—— 那是实际手上这个模型用的，
 * 也是 RPM 关停之后最主要的来源。
 */
function tPose(): THREE.Object3D {
  const root = new THREE.Object3D();
  root.name = "Root";

  const hips = new THREE.Bone();
  hips.name = "J_Bip_C_Hips";
  hips.position.set(0, 0.9, 0);
  root.add(hips);

  const spine = new THREE.Bone();
  spine.name = "J_Bip_C_Spine";
  spine.position.set(0, 0.15, 0);
  hips.add(spine);

  const chest = new THREE.Bone();
  chest.name = "J_Bip_C_UpperChest";
  chest.position.set(0, 0.15, 0);
  spine.add(chest);

  const neck = new THREE.Bone();
  neck.name = "J_Bip_C_Neck";
  neck.position.set(0, 0.15, 0);
  chest.add(neck);

  const head = new THREE.Bone();
  head.name = "J_Bip_C_Head";
  head.position.set(0, 0.1, 0);
  neck.add(head);

  const headTop = new THREE.Bone();
  headTop.name = "J_Bip_C_Head_end";
  headTop.position.set(0, 0.2, 0);
  head.add(headTop);

  // 双臂沿 ±X 平举 —— 这就是 T-pose
  for (const [side, sign] of [
    ["L", 1],
    ["R", -1],
  ] as const) {
    const upper = new THREE.Bone();
    upper.name = `J_Bip_${side}_UpperArm`;
    upper.position.set(sign * 0.15, 0.1, 0);
    chest.add(upper);

    const lower = new THREE.Bone();
    lower.name = `J_Bip_${side}_LowerArm`;
    lower.position.set(sign * 0.25, 0, 0);
    upper.add(lower);

    const hand = new THREE.Bone();
    hand.name = `J_Bip_${side}_Hand`;
    hand.position.set(sign * 0.25, 0, 0);
    lower.add(hand);
  }

  root.updateMatrixWorld(true);
  return root;
}

/** 一根骨头在世界空间里指向哪儿（从自己指向第一个子骨头） */
function direction(bone: THREE.Object3D): THREE.Vector3 {
  bone.updateWorldMatrix(true, true);
  const child = bone.children[0];
  const a = new THREE.Vector3().setFromMatrixPosition(bone.matrixWorld);
  const b = new THREE.Vector3().setFromMatrixPosition(child.matrixWorld);
  return b.sub(a).normalize();
}


/**
 * 动作播完之后手臂必须回到静止姿势。
 *
 * three 的 mixer **不会还原它动过的骨骼** —— 一个 VRMA 片段把手举起来、
 * 播完之后如果 idle 不管手臂，人物就永远举着手（用户实测撞到过）。
 */
describe("idle 要负责还原手臂", () => {
  function rig() {
    const bones: Bones = {
      head: null, neck: null, chest: null, spine: null, hips: null,
      upperArmL: null, upperArmR: null, lowerArmL: null, lowerArmR: null,
    };
    const state: IdleState = { rest: new Map() };
    for (const k of ["upperArmL", "upperArmR", "lowerArmL", "lowerArmR", "chest"] as const) {
      const b = new THREE.Object3D();
      b.name = k;
      b.quaternion.setFromEuler(new THREE.Euler(0, 0, k.endsWith("L") ? 1.2 : -1.2));
      bones[k] = b;
      state.rest.set(b, b.quaternion.clone());
    }
    return { bones, state };
  }

  it("被动作摆到别处的手臂会被拉回静止附近", () => {
    const { bones, state } = rig();
    const arm = bones.upperArmR!;
    const rest = state.rest.get(arm)!.clone();

    // 模拟动作片段把手举起来
    arm.quaternion.setFromEuler(new THREE.Euler(0, 0, 1.8));
    expect(arm.quaternion.angleTo(rest)).toBeGreaterThan(0.5);

    idle(bones, state, 12.34);
    // idle 自带极小幅度的呼吸，所以不要求完全相等，只要求回到静止附近
    expect(arm.quaternion.angleTo(rest)).toBeLessThan(0.06);
  });

  it("四根手臂骨骼都管，不能只管一半", () => {
    const { bones, state } = rig();
    for (const k of ["upperArmL", "upperArmR", "lowerArmL", "lowerArmR"] as const) {
      const b = bones[k]!;
      const rest = state.rest.get(b)!.clone();
      b.quaternion.setFromEuler(new THREE.Euler(1.5, 0, 0));
      idle(bones, state, 5);
      expect(b.quaternion.angleTo(rest), k).toBeLessThan(0.06);
    }
  });

  it("幅度要小 —— 手臂不该像躯干那样明显晃", () => {
    const { bones, state } = rig();
    const arm = bones.upperArmL!;
    const rest = state.rest.get(arm)!.clone();
    let max = 0;
    for (let t = 0; t < 20; t += 0.1) {
      idle(bones, state, t);
      max = Math.max(max, arm.quaternion.angleTo(rest));
    }
    expect(max).toBeLessThan(0.05);
    expect(max).toBeGreaterThan(0); // 也不能完全僵死
  });
});

describe("找骨头", () => {
  it("认得 VRoid 的 J_Bip 命名", () => {
    const bones = findBones(tPose());
    expect(bones.head?.name).toBe("J_Bip_C_Head");
    expect(bones.neck?.name).toBe("J_Bip_C_Neck");
    expect(bones.hips?.name).toBe("J_Bip_C_Hips");
    expect(bones.upperArmL?.name).toBe("J_Bip_L_UpperArm");
    expect(bones.upperArmR?.name).toBe("J_Bip_R_UpperArm");
  });

  it("UpperChest 优先于 Chest —— 呼吸主要发生在上胸", () => {
    const root = new THREE.Object3D();
    const chest = new THREE.Bone();
    chest.name = "J_Bip_C_Chest";
    const upper = new THREE.Bone();
    upper.name = "J_Bip_C_UpperChest";
    chest.add(upper);
    root.add(chest);
    expect(findBones(root).chest?.name).toBe("J_Bip_C_UpperChest");
  });

  it("左右不能认反", () => {
    // 认反了胳膊会交叉过去，一眼就看得出，但排查时很容易怀疑到旋转轴上
    const bones = findBones(tPose());
    expect(bones.upperArmL?.name).toContain("_L_");
    expect(bones.upperArmR?.name).toContain("_R_");
  });

  it("找不到的返回 null，不炸", () => {
    const bones = findBones(new THREE.Object3D());
    expect(bones.head).toBeNull();
    expect(bones.upperArmL).toBeNull();
  });

  it("认得被 GLTFLoader 清洗过的名字（upper_armL）", () => {
    // **这条是真出现过的 bug，而且我第一次排查时判断反了。**
    // GLTFLoader 会清洗节点名（去掉 `.` 之类的保留字符），文件里写的
    // `upper_arm.L` 到了 three 里叫 `upper_armL`。我拿 glTF 原始文件去验证，
    // 看到带分隔符就认定没问题 —— 而真实运行时匹配不上，胳膊一直平举着。
    const root = new THREE.Object3D();
    for (const name of ["head", "upper_armL", "lower_armL", "upper_armR", "lower_armR"]) {
      const b = new THREE.Bone();
      b.name = name;
      root.add(b);
    }
    const bones = findBones(root);
    expect(bones.upperArmL?.name).toBe("upper_armL");
    expect(bones.upperArmR?.name).toBe("upper_armR");
    expect(bones.lowerArmL?.name).toBe("lower_armL");
  });

  it("结尾小写的 l 不算左侧", () => {
    // `Tail`、`Ctrl` 这种正常名字不能被误判成左手
    const root = new THREE.Object3D();
    for (const name of ["Tail", "Ctrl", "upper_armL"]) {
      const b = new THREE.Bone();
      b.name = name;
      root.add(b);
    }
    expect(findBones(root).upperArmL?.name).toBe("upper_armL");
  });

  it("Mixamo 命名也认", () => {
    const root = new THREE.Object3D();
    for (const name of ["mixamorig:Head", "mixamorig:LeftArm", "mixamorig:RightArm"]) {
      const b = new THREE.Bone();
      b.name = name;
      root.add(b);
    }
    const bones = findBones(root);
    expect(bones.head?.name).toBe("mixamorig:Head");
    expect(bones.upperArmL?.name).toBe("mixamorig:LeftArm");
    expect(bones.upperArmR?.name).toBe("mixamorig:RightArm");
  });
});

describe("从 T-pose 放下手臂", () => {
  it("放完之后胳膊明显朝下", () => {
    // **这是本文件最要紧的一条。** 旋转轴选错会让胳膊往后翻或者拧麻花，
    // 而那在画面上只表现成"胳膊看着怪"，反推不到原因
    const root = tPose();
    const bones = findBones(root);

    for (const arm of [bones.upperArmL!, bones.upperArmR!]) {
      expect(direction(arm).y).toBeCloseTo(0, 5); // T-pose：水平
    }

    relaxArms(bones);
    root.updateMatrixWorld(true);

    for (const arm of [bones.upperArmL!, bones.upperArmR!]) {
      const dir = direction(arm);
      expect(dir.y, `胳膊没朝下：${JSON.stringify(dir)}`).toBeLessThan(-0.7);
    }
  });

  it("胳膊仍然分在两边，没有交叉到身体另一侧", () => {
    // 轴取反的典型症状：两条胳膊穿过身体交叉过去
    const root = tPose();
    const bones = findBones(root);
    relaxArms(bones);
    root.updateMatrixWorld(true);

    const l = new THREE.Vector3().setFromMatrixPosition(
      bones.lowerArmL!.matrixWorld,
    );
    const r = new THREE.Vector3().setFromMatrixPosition(
      bones.lowerArmR!.matrixWorld,
    );
    expect(l.x, "左臂应当仍在左边").toBeGreaterThan(0);
    expect(r.x, "右臂应当仍在右边").toBeLessThan(0);
  });

  it("胳膊没有往前后翻出去", () => {
    // 绕错轴的另一种症状：胳膊翻到身体前面或后面
    const root = tPose();
    const bones = findBones(root);
    relaxArms(bones);
    root.updateMatrixWorld(true);

    for (const arm of [bones.upperArmL!, bones.upperArmR!]) {
      expect(Math.abs(direction(arm).z)).toBeLessThan(0.2);
    }
  });

  it("没有骨头也不炸", () => {
    expect(() => relaxArms(findBones(new THREE.Object3D()))).not.toThrow();
  });
});

describe("呼吸与晃动", () => {
  it("会动，但幅度很小", () => {
    // 幅度大了立刻从"在呼吸"变成"在跳舞"
    const root = tPose();
    const bones = findBones(root);
    const state = captureRest(bones);

    idle(bones, state, 0);
    const a = bones.head!.quaternion.clone();
    idle(bones, state, 1.4);
    const b = bones.head!.quaternion.clone();

    expect(a.angleTo(b)).toBeGreaterThan(0.001);
    expect(a.angleTo(b), "晃动幅度过大").toBeLessThan(0.25);
  });

  it("长时间不漂移", () => {
    // **必须从静止朝向重新叠加，不能在当前值上累加。**
    // 累加的话几分钟之后身体会慢慢拧到一边去 —— 而且现象出现得很慢，
    // 等发现的时候根本想不到是这里
    const root = tPose();
    const bones = findBones(root);
    const state = captureRest(bones);

    // 注意断言的对象：不是"t=0 等于静止姿势"（drift 有相位偏移，t=0 本来就
    // 不在原点），而是"同一个 t 跑两次结果一样"。那才是没有累加的意思
    idle(bones, state, 0);
    const before = bones.head!.quaternion.clone();

    // 跑十分钟（60fps）
    for (let i = 0; i < 36_000; i++) idle(bones, state, i / 60);

    idle(bones, state, 0);
    expect(bones.head!.quaternion.angleTo(before)).toBeLessThan(1e-6);
  });

  it("骨头不全也不炸", () => {
    const root = new THREE.Object3D();
    const head = new THREE.Bone();
    head.name = "Head";
    root.add(head);
    const bones = findBones(root);
    const state = captureRest(bones);
    expect(() => idle(bones, state, 3)).not.toThrow();
  });
});
