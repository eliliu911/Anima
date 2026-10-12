import { describe, expect, it } from "vitest";
import * as THREE from "three";

import { generateAnimation } from "./animations";
import { relaxArms, type Bones, type IdleState } from "./pose";

/**
 * 一副**真实的 T-pose 骨架**：有父子关系、有骨骼长度。
 *
 * 原先这里是一堆孤立的 `Object3D`，谁也不是谁的孩子。那种骨架能让测试跑
 * 起来，但**测不出手最后到了哪儿** —— 而"手到了哪儿"正是用户看到的东西
 * （他的原话是"这是要起飞么"）。手势现在按世界方向定向，没有子骨头就
 * 算不出朝向，孤立骨架下整套动作会退化成"不动"。
 *
 * 尺寸按成年人比例，单位米。T-pose：手臂沿 ±X 平举。
 */
function skeleton(present: (keyof Bones)[]): {
  bones: Bones;
  rest: IdleState;
  root: THREE.Object3D;
  handL: THREE.Object3D;
  handR: THREE.Object3D;
} {
  const bone = (name: string, x: number, y: number, z = 0) => {
    const b = new THREE.Bone();
    b.name = name;
    b.position.set(x, y, z);
    return b;
  };
  const root = bone("root", 0, 0, 0);
  const hips = bone("hips", 0, 0.95, 0);
  const spine = bone("spine", 0, 0.12, 0);
  const chest = bone("chest", 0, 0.16, 0);
  const neck = bone("neck", 0, 0.18, 0);
  const head = bone("head", 0, 0.08, 0);
  root.add(hips); hips.add(spine); spine.add(chest); chest.add(neck); neck.add(head);
  head.add(bone("headTop", 0, 0.2, 0));

  const arm = (side: "L" | "R") => {
    const s = side === "L" ? 1 : -1;
    const upper = bone(`upperArm${side}`, s * 0.16, 0.1, 0);
    const lower = bone(`lowerArm${side}`, s * 0.27, 0, 0);
    const hand = bone(`hand${side}`, s * 0.25, 0, 0);
    upper.add(lower); lower.add(hand); hand.add(bone(`fingers${side}`, s * 0.09, 0, 0));
    chest.add(upper);
    return { upper, lower, hand };
  };
  const L = arm("L");
  const R = arm("R");
  root.updateMatrixWorld(true);

  const all: Record<keyof Bones, THREE.Object3D> = {
    head, neck, chest, spine, hips,
    upperArmL: L.upper, upperArmR: R.upper,
    lowerArmL: L.lower, lowerArmR: R.lower,
  };
  const bones: Bones = {
    head: null, neck: null, chest: null, spine: null, hips: null,
    upperArmL: null, upperArmR: null, lowerArmL: null, lowerArmR: null,
  };
  for (const key of present) bones[key] = all[key];

  // 产品里静止姿势是 relaxArms 压过的（手臂垂下 65°），不是 T-pose。
  // **测试必须用同一份静止姿势** —— 用 T-pose 当静止的话，
  // 正好绕过了这次要修的那个 bug
  relaxArms(bones);
  root.updateMatrixWorld(true);

  const rest: IdleState = { rest: new Map() };
  for (const key of present) {
    const b = all[key];
    rest.rest.set(b, b.quaternion.clone());
  }
  return { bones, rest, root, handL: L.hand, handR: R.hand };
}

/** 播到某个时刻时，这根骨头在世界里的位置 */
function poseAt(
  clip: THREE.AnimationClip,
  root: THREE.Object3D,
  time: number,
  probe: THREE.Object3D,
): THREE.Vector3 {
  const mixer = new THREE.AnimationMixer(root);
  const action = mixer.clipAction(clip);
  action.play();
  mixer.setTime(time);
  root.updateMatrixWorld(true);
  const p = new THREE.Vector3().setFromMatrixPosition(probe.matrixWorld);
  action.stop();
  mixer.uncacheRoot(root);
  return p;
}

const ALL: (keyof Bones)[] = ["spine", "upperArmL", "upperArmR", "lowerArmL", "lowerArmR"];

function trackOf(clip: THREE.AnimationClip, bone: THREE.Object3D) {
  return clip.tracks.find((t) => t.name.startsWith(bone.uuid));
}

describe("手势片段", () => {
  it("认识的动作都能生成片段，不认识的返回 null", () => {
    const { bones, rest } = skeleton(ALL);
    for (const name of ["stretch", "伸懒腰", "wave", "挥手", "cover_mouth", "heart", "比心"]) {
      expect(generateAnimation(name, bones, rest), name).not.toBeNull();
    }
    expect(generateAnimation("moonwalk", bones, rest)).toBeNull();
  });

  /**
   * 这条是"僵硬"那个 bug 的回归测试。
   *
   * 原先每个动作只有 3~4 个关键帧，而 three 在关键帧之间做的是**匀速** slerp ——
   * 恒速出发、到点硬停，正是机器人的运动特征。修法是按缓动曲线密集采样，
   * 所以关键帧数必须远多于姿势数。
   */
  it("按曲线密集采样，而不是几个关键帧硬连", () => {
    const { bones, rest } = skeleton(ALL);
    const clip = generateAnimation("heart", bones, rest)!;
    const track = trackOf(clip, bones.upperArmR!)!;
    expect(track.times.length).toBeGreaterThan(20);

    // 时间必须严格递增，否则 three 的插值会乱跳
    for (let i = 1; i < track.times.length; i++) {
      expect(track.times[i]).toBeGreaterThan(track.times[i - 1]);
    }
  });

  it("速度是变化的 —— 匀速就是僵硬本身", () => {
    const { bones, rest } = skeleton(ALL);
    const clip = generateAnimation("heart", bones, rest)!;
    const track = trackOf(clip, bones.upperArmR!) as THREE.QuaternionKeyframeTrack;

    // 逐帧角位移。缓动的话中段明显大于起步段
    const step: number[] = [];
    const q = (i: number) =>
      new THREE.Quaternion(
        track.values[i * 4], track.values[i * 4 + 1],
        track.values[i * 4 + 2], track.values[i * 4 + 3],
      );
    for (let i = 1; i < Math.min(12, track.times.length); i++) {
      step.push(q(i - 1).angleTo(q(i)));
    }
    const first = step[0];
    const middle = Math.max(...step);
    expect(middle).toBeGreaterThan(first * 2);
  });

  it("起止都停在静止姿势上，否则动作放完人是歪的", () => {
    const { bones, rest } = skeleton(ALL);
    const clip = generateAnimation("wave", bones, rest)!;
    const arm = bones.upperArmR!;
    const track = trackOf(clip, arm) as THREE.QuaternionKeyframeTrack;
    const restQ = rest.rest.get(arm)!;
    const at = (i: number) =>
      new THREE.Quaternion(
        track.values[i * 4], track.values[i * 4 + 1],
        track.values[i * 4 + 2], track.values[i * 4 + 3],
      );
    const last = track.times.length - 1;
    expect(at(0).angleTo(restQ)).toBeLessThan(1e-3);
    expect(at(last).angleTo(restQ)).toBeLessThan(1e-3);
  });

  /**
   * 模型缺骨骼时必须生成**零轨道**的片段，而不是一个"看起来正常"的空片段。
   * 调用方靠 `clip.tracks.length` 判断要不要报警 —— 缺骨骼时画面上什么都不会
   * 发生，而且不报错，那正是"我让它挥手它没反应"最难查的一种。
   */
  it("缺骨骼时轨道数为 0，让调用方能发现", () => {
    const { bones, rest } = skeleton([]);
    const clip = generateAnimation("wave", bones, rest)!;
    expect(clip.tracks.length).toBe(0);
  });

  it("只有部分骨骼时，只给存在的那些排轨道", () => {
    const { bones, rest } = skeleton(["upperArmR"]);
    const clip = generateAnimation("wave", bones, rest)!;
    expect(clip.tracks.length).toBe(1);
    expect(clip.tracks[0].name.startsWith(bones.upperArmR!.uuid)).toBe(true);
  });

  it("轨道按 uuid 绑定 —— GLTFLoader 会清洗节点名，按名字绑不稳", () => {
    const { bones, rest } = skeleton(ALL);
    const clip = generateAnimation("stretch", bones, rest)!;
    for (const t of clip.tracks) {
      expect(t.name).toMatch(/^[0-9a-f-]{36}\.quaternion$/i);
    }
  });
});

/**
 * 手最后到了哪儿 —— 这是用户唯一能看见的东西。
 *
 * 之前手势的目标是写死的欧拉角，而欧拉角是**相对绑定姿势（T-pose）**的；
 * 屏幕上的静止姿势却是 `relaxArms` 压下 65° 之后的样子。两者差 65°，
 * 于是每个动作都先把手臂拽回 T-pose 附近再动：挥手像扩胸操，
 * 比心的小臂折到头顶。用户的原话是"这是要起飞么"。
 *
 * 所以这一组测的不是"有没有关键帧"，而是**手的世界坐标**。
 */
describe("手到底去了哪儿", () => {
  const ALL_ARMS: (keyof Bones)[] = ["spine", "upperArmL", "upperArmR", "lowerArmL", "lowerArmR"];
  /** 肩膀高度（chest 在 y≈1.23，大臂根部再高 0.1） */
  const SHOULDER_Y = 0.95 + 0.12 + 0.16 + 0.1;
  const HEAD_TOP_Y = 0.95 + 0.12 + 0.16 + 0.18 + 0.08 + 0.2;

  it("静止时手是垂着的，不是平举", () => {
    const { handL, handR, root } = skeleton(ALL_ARMS);
    root.updateMatrixWorld(true);
    for (const h of [handL, handR]) {
      const y = new THREE.Vector3().setFromMatrixPosition(h.matrixWorld).y;
      expect(y, "静止姿势手应该在肩膀以下").toBeLessThan(SHOULDER_Y);
    }
  });

  it("挥手：手抬到肩膀以上，而且不过头顶", () => {
    const s = skeleton(ALL_ARMS);
    const clip = generateAnimation("wave", s.bones, s.rest)!;
    // 抬到位之后、放下之前
    const y = poseAt(clip, s.root, 1.0, s.handR).y;
    expect(y, "招手时手要在肩膀以上").toBeGreaterThan(SHOULDER_Y);
    expect(y, "招手不该把手举过头顶").toBeLessThan(HEAD_TOP_Y);
  });

  /// **这条就是"要起飞"的回归测试。** 比心的手要在胸前，不能举到头上去
  it("比心：手在胸前，不在头顶", () => {
    const s = skeleton(ALL_ARMS);
    const clip = generateAnimation("heart", s.bones, s.rest)!;
    const l = poseAt(clip, s.root, 1.2, s.handL);
    const r = poseAt(clip, s.root, 1.2, s.handR);
    for (const [name, p] of [["左手", l], ["右手", r]] as const) {
      expect(p.y, `${name}不该举过头顶`).toBeLessThan(SHOULDER_Y + 0.12);
      expect(p.z, `${name}要送到身体前面`).toBeGreaterThan(0.05);
    }
    // 两只手要靠拢 —— 比心是合在一起的动作
    expect(l.distanceTo(r), "两手应该在胸前靠拢").toBeLessThan(0.42);
  });

  it("伸懒腰：这个才是该举过头顶的", () => {
    const s = skeleton(ALL_ARMS);
    const clip = generateAnimation("stretch", s.bones, s.rest)!;
    for (const h of [s.handL, s.handR]) {
      const y = poseAt(clip, s.root, 1.6, h).y;
      expect(y, "伸懒腰手要举过头").toBeGreaterThan(HEAD_TOP_Y - 0.15);
    }
  });

  it("捂嘴：手要够到脸，不是停在半空", () => {
    const s = skeleton(ALL_ARMS);
    const clip = generateAnimation("cover_mouth", s.bones, s.rest)!;
    const p = poseAt(clip, s.root, 0.9, s.handR);
    const mouth = new THREE.Vector3(0, HEAD_TOP_Y - 0.16, 0.06);
    expect(p.distanceTo(mouth), `手离嘴 ${p.distanceTo(mouth).toFixed(2)}m，太远`).toBeLessThan(0.3);
  });

  /// 动作播完必须回到静止姿势，否则手就举在那儿放不下来了
  it("播完回到静止姿势", () => {
    const s = skeleton(ALL_ARMS);
    for (const name of ["wave", "heart", "stretch", "cover_mouth"]) {
      const clip = generateAnimation(name, s.bones, s.rest)!;
      const start = poseAt(clip, s.root, 0, s.handR);
      const end = poseAt(clip, s.root, clip.duration, s.handR);
      expect(start.distanceTo(end), `${name} 播完没回到原位`).toBeLessThan(0.05);
    }
  });
});
