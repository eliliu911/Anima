import { describe, expect, it } from "vitest";
import * as THREE from "three";
import { applyCamera, type DirectorActor, type DirectorCamera } from "./scene";
import { CAMERA_PRESETS, framingCamera, framingFromPoint, motionKeyframes, motionKeyframesAtPoint, pickShotTarget, SIZE_FRAMING } from "./framing";

const actor = (id: string, label: string, x: number, z: number, yawDeg = 0, assetId?: string): DirectorActor => ({
  id, label, color: "#FF00FF", assetId, position: [x, 0, z], yawDeg, scale: 1, pose: "standing",
});

/** 世界点在这个机位画面里的 NDC 坐标 */
function project(cam: DirectorCamera, p: [number, number, number]) {
  const c = new THREE.PerspectiveCamera(cam.fovDeg, 16 / 9, 0.05, 500);
  applyCamera(c, cam, 16 / 9);
  const view = new THREE.Vector3(...p).applyMatrix4(c.matrixWorldInverse);
  const ndc = new THREE.Vector3(...p).project(c);
  return { x: ndc.x, y: ndc.y, inFront: view.z < 0 };
}

describe("按景别摆机", () => {
  const jc = actor("a", "陆川", 0, -3, 0);

  it("中景：画面竖直正好装下 1.15 米（腰以上），人在画面正中、正脸对镜头", () => {
    const cam = framingCamera({ target: jc, size: "medium", angle: "eye_level", fovDeg: 50 });
    const { frameH, aimH } = SIZE_FRAMING.medium;
    const top = project(cam, [0, aimH + frameH / 2, -3]);
    const bottom = project(cam, [0, aimH - frameH / 2, -3]);
    expect(top.y).toBeCloseTo(1, 1);
    expect(bottom.y).toBeCloseTo(-1, 1);
    expect(project(cam, [0, aimH, -3]).x).toBeCloseTo(0, 5);
    // 人偶 yaw 0 脸朝 +Z：机位要在他的 +Z 一侧
    expect(cam.position[2]).toBeGreaterThan(-3);
  });

  it("景别越紧离得越近；特写对准脸", () => {
    const d = (size: string) => {
      const c = framingCamera({ target: jc, size, angle: "eye_level", fovDeg: 50 });
      return Math.hypot(c.position[0], c.position[2] + 3);
    };
    expect(d("wide")).toBeGreaterThan(d("full_body"));
    expect(d("full_body")).toBeGreaterThan(d("medium"));
    expect(d("medium")).toBeGreaterThan(d("close_up"));
    const cu = framingCamera({ target: jc, size: "close_up", angle: "eye_level", fovDeg: 50 });
    expect(project(cu, [0, 1.66, -3]).y).toBeCloseTo(0, 1);
  });

  it("仰拍在下面往上看、俯拍在上面往下看、鸟瞰几乎垂直、荷兰角横滚", () => {
    const low = framingCamera({ target: jc, size: "medium", angle: "low_angle", fovDeg: 50 });
    const high = framingCamera({ target: jc, size: "medium", angle: "high_angle", fovDeg: 50 });
    const bird = framingCamera({ target: jc, size: "wide", angle: "birds_eye", fovDeg: 50 });
    const dutch = framingCamera({ target: jc, size: "medium", angle: "dutch", fovDeg: 50 });
    expect(low.pitchDeg).toBeGreaterThan(10);
    expect(low.position[1]).toBeGreaterThanOrEqual(0.15);
    expect(high.pitchDeg).toBeLessThan(-10);
    expect(bird.pitchDeg).toBeLessThan(-70);
    expect(dutch.rollDeg).toBe(15);
  });

  it("侧面在人的侧边、背面在身后", () => {
    const side = framingCamera({ target: jc, size: "medium", angle: "eye_level", side: "side", fovDeg: 50 });
    const back = framingCamera({ target: jc, size: "medium", angle: "eye_level", side: "back", fovDeg: 50 });
    expect(Math.abs(side.position[0])).toBeGreaterThan(1);
    expect(back.position[2]).toBeLessThan(-3);
  });

  it("过肩：站在另一个人身后偏一侧，对面的人在画面里；没有另一个人就退回正面", () => {
    const by = actor("b", "星澜", 0, -1, 180);
    const os = framingCamera({ target: jc, other: by, size: "medium_close", angle: "over_shoulder", shoulder: "left", fovDeg: 50 });
    expect(os.position[2]).toBeGreaterThan(-1);
    expect(Math.abs(os.position[0])).toBeGreaterThan(0.2);
    const p = project(os, [0, 1.52, -3]);
    expect(p.inFront && Math.abs(p.x) < 0.2).toBe(true);
    const alone = framingCamera({ target: jc, size: "medium_close", angle: "over_shoulder", fovDeg: 50 });
    expect(alone.position[2]).toBeGreaterThan(-3);
  });

  it("预设都能摆出机位（认不出的词回落中景平视）", () => {
    const by = actor("b", "星澜", 0, -1, 180);
    for (const p of CAMERA_PRESETS) {
      const c = framingCamera({ target: jc, other: by, size: p.size, angle: p.angle, side: p.side, shoulder: p.shoulder, fovDeg: p.fovDeg ?? 50 });
      expect(c.position.every(Number.isFinite), p.label).toBe(true);
    }
    expect(framingCamera({ target: jc, size: "奇怪", angle: "奇怪", fovDeg: 50 })).toEqual(framingCamera({ target: jc, size: "medium", angle: "eye_level", fovDeg: 50 }));
  });
});

describe("按运镜出关键帧", () => {
  const jc = actor("a", "陆川", 0, -3, 0);
  const shot = framingCamera({ target: jc, size: "medium", angle: "eye_level", fovDeg: 50 });
  const aim: [number, number, number] = [0, 1.35, -3];
  const dist = (c: DirectorCamera) => Math.hypot(c.position[0] - aim[0], c.position[1] - aim[1], c.position[2] - aim[2]);

  it("推镜：从远处推到这一镜的景别；拉镜反过来", () => {
    const push = motionKeyframes(shot, aim, "push_in")!;
    expect(push).toHaveLength(2);
    expect(dist(push[0])).toBeGreaterThan(dist(push[1]));
    expect(push[1]).toEqual(shot);
    const pull = motionKeyframes(shot, aim, "pull_out")!;
    expect(dist(pull[1])).toBeGreaterThan(dist(pull[0]));
  });

  it("移、摇、升降、变焦、环绕各动各的；固定出不了关键帧", () => {
    expect(motionKeyframes(shot, aim, "truck_right")![1].position[0]).toBeGreaterThan(shot.position[0]);
    expect(motionKeyframes(shot, aim, "pan_left")![1].yawDeg).toBe(shot.yawDeg - 25);
    expect(motionKeyframes(shot, aim, "pedestal_up")![1].position[1]).toBeCloseTo(shot.position[1] + 0.8);
    expect(motionKeyframes(shot, aim, "zoom_in")![1].fovDeg).toBeLessThan(shot.fovDeg);
    const orbit = motionKeyframes(shot, aim, "orbit")!;
    expect(orbit).toHaveLength(3);
    expect(dist(orbit[0])).toBeCloseTo(dist(shot), 1);
    expect(motionKeyframes(shot, aim, "static")).toBeNull();
  });
});

describe("按分镜摆机对准谁", () => {
  const jc = actor("a", "陆川", 0, -3, 0, "char_jc");
  const by = actor("b", "星澜", 1, -3, 0, "char_by");
  it("第一个开口的在场的人；画外音不算；没台词按出场人物", () => {
    expect(pickShotTarget([jc, by], { dialogue_lines: [{ speaker: "旁白", off_screen: true, line: "x" }, { speaker: "星澜", line: "你好" }] }).target?.id).toBe("b");
    expect(pickShotTarget([jc, by], { dialogue_lines: [], character_ids: ["char_by"] }).target?.id).toBe("b");
    const r = pickShotTarget([jc, by], {});
    expect(r.target?.id).toBe("a");
    expect(r.other?.id).toBe("b");
    expect(pickShotTarget([], {}).target).toBeNull();
  });
  it("群演、道具不当主体：只有它们时对不准任何人", () => {
    const extra = { ...actor("x", "群演 1", 0, -2, 0), kind: "extra" as const };
    const prop = { ...actor("p", "机械臂", 0, -2, 0, "prop_arm"), kind: "prop" as const };
    expect(pickShotTarget([extra, prop, by], { character_ids: ["prop_arm"] }).target?.id).toBe("b");
    expect(pickShotTarget([extra, prop], {}).target).toBeNull();
    expect(pickShotTarget([jc, extra, prop], {}).other).toBeNull();
  });
});

describe("全景锁在拍摄点：不挪相机，转向 + 视场角摆景别", () => {
  const jc = actor("a", "陆川", 0, -4, 0);
  const eye: [number, number, number] = [0, 1.6, 0];
  it("相机不动；视场角正好装下这个景别；对准那个人", () => {
    const cam = framingFromPoint(eye, { target: jc, size: "medium", angle: "low_angle" });
    expect(cam.position).toEqual(eye);
    const { frameH, aimH } = SIZE_FRAMING.medium;
    expect(project(cam, [0, aimH + frameH / 2, -4]).y).toBeCloseTo(1, 1);
    expect(project(cam, [0, aimH, -4]).x).toBeCloseTo(0, 5);
    // 特写视场角比中景窄
    expect(framingFromPoint(eye, { target: jc, size: "close_up", angle: "eye_level" }).fovDeg).toBeLessThan(cam.fovDeg);
  });
  it("推 / 拉 → 变焦；移 → 摇；相机位置始终不变", () => {
    const shot = framingFromPoint(eye, { target: jc, size: "medium", angle: "eye_level" });
    const push = motionKeyframesAtPoint(shot, "push_in")!;
    expect(push[0].fovDeg).toBeGreaterThan(push[1].fovDeg);
    const truck = motionKeyframesAtPoint(shot, "truck_right")!;
    expect(truck[1].yawDeg).toBeGreaterThan(shot.yawDeg);
    for (const k of [...push, ...truck]) expect(k.position).toEqual(eye);
    expect(motionKeyframesAtPoint(shot, "static")).toBeNull();
  });
});
