import { describe, expect, it } from "vitest";
import * as THREE from "three";
import {
  equirectUv,
  lockViewCorrection,
  planetUv,
  seamView,
  stereoRay,
  textureDirectionAt,
} from "./panoOrient";
import { makeSphereGeometry } from "./panoStage";

/** 球上离这个方向最近的顶点的贴图 u。跳过两极 —— 那里一圈顶点方向相同、u 各不相同。 */
function uAt(dir: THREE.Vector3): number {
  const g = makeSphereGeometry();
  const pos = g.getAttribute("position");
  const uv = g.getAttribute("uv");
  const d = dir.clone().normalize();
  let best = -2;
  let u = 0;
  const v = new THREE.Vector3();
  for (let i = 0; i < pos.count; i++) {
    v.fromBufferAttribute(pos, i).normalize();
    if (Math.abs(v.y) > 0.95) continue;
    const dot = v.dot(d);
    if (dot > best) {
      best = dot;
      u = uv.getX(i);
    }
  }
  return u;
}

/** u 在 0/1 处首尾相接，比较时按圆周距离算 */
const uDist = (a: number, b: number) => Math.min(Math.abs(a - b), 1 - Math.abs(a - b));

describe("环境球的贴图朝向", () => {
  const none = { roll: 0, pitch: 0, yaw: 0 };

  it("yaw 0 看到的是贴图正中", () => {
    expect(uDist(uAt(textureDirectionAt(none, 0, 0)), 0.5)).toBeLessThan(0.01);
  });

  /**
   * **不镜像**：往右转，u 变大。原来 `SphereGeometry + BackSide` 在这里是反的，
   * 整张全景左右颠倒而没人发现。
   */
  it("往右转 90° 看到 u=0.75，往左转看到 u=0.25", () => {
    expect(uDist(uAt(textureDirectionAt(none, 90, 0)), 0.75)).toBeLessThan(0.01);
    expect(uDist(uAt(textureDirectionAt(none, -90, 0)), 0.25)).toBeLessThan(0.01);
  });

  it("往右转一点点，u 就大一点点", () => {
    const a = uAt(textureDirectionAt(none, 10, 0));
    const b = uAt(textureDirectionAt(none, 20, 0));
    expect(b).toBeGreaterThan(a);
  });
});

const close = (a: THREE.Vector3, b: THREE.Vector3) => expect(a.distanceTo(b)).toBeLessThan(1e-3);

describe("锁定当前视角", () => {
  const cases = [
    { c: { roll: 0, pitch: 0, yaw: 0 }, yaw: 60, pitch: 0 },
    { c: { roll: 0, pitch: 0, yaw: 0 }, yaw: 0, pitch: 40 },
    // 有俯仰 + 已有校正：直接相加会在这里偏掉
    { c: { roll: 3, pitch: -5, yaw: 20 }, yaw: 135, pitch: 40 },
    { c: { roll: -8, pitch: 12, yaw: -170 }, yaw: -100, pitch: -35 },
  ];

  it("锁定后回正，看到的还是锁定前那个点", () => {
    for (const k of cases) {
      const before = textureDirectionAt(k.c, k.yaw, k.pitch);
      const locked = lockViewCorrection(k.c, k.yaw, k.pitch);
      close(textureDirectionAt(locked, 0, 0), before);
    }
  });

  /** 锁定之后画面不该转：屏幕「上方」对应的贴图方向也不变，否则地平线会歪。 */
  it("锁定后画面不旋转", () => {
    for (const k of cases) {
      const upBefore = textureDirectionAt(k.c, k.yaw, k.pitch + 1).sub(textureDirectionAt(k.c, k.yaw, k.pitch));
      const locked = lockViewCorrection(k.c, k.yaw, k.pitch);
      const upAfter = textureDirectionAt(locked, 0, 1).sub(textureDirectionAt(locked, 0, 0));
      close(upAfter.normalize(), upBefore.normalize());
    }
  });

  /** 这条是反例：把角度直接加到校正上（DC 的做法），有俯仰时回正就不对了。 */
  it("直接相加的做法在有俯仰时会偏", () => {
    const k = cases[2];
    const naive = { roll: k.c.roll, pitch: k.c.pitch + k.pitch, yaw: k.c.yaw + k.yaw };
    const before = textureDirectionAt(k.c, k.yaw, k.pitch);
    expect(textureDirectionAt(naive, 0, 0).distanceTo(before)).toBeGreaterThan(0.1);
  });
});

describe("接缝", () => {
  it("相机转到接缝方向时，看的正是贴图 u=0 那条经线", () => {
    const seam = new THREE.Vector3(0, 0, 1);
    for (const c of [
      { roll: 0, pitch: 0, yaw: 0 },
      { roll: 0, pitch: 0, yaw: 45 },
      { roll: 4, pitch: -6, yaw: -120 },
    ]) {
      for (const front of [0, 30, -150]) {
        const v = seamView(c, front);
        close(textureDirectionAt(c, v.yawDeg + front, v.pitchDeg), seam);
        expect(uDist(uAt(textureDirectionAt(c, v.yawDeg + front, v.pitchDeg)), 0)).toBeLessThan(0.01);
      }
    }
  });

  it("没有校正、没有偏移时接缝在正后方", () => {
    expect(Math.abs(seamView({ roll: 0, pitch: 0, yaw: 0 }, 0).yawDeg)).toBe(180);
  });
});

describe("小行星（球极投影）", () => {
  const none = { roll: 0, pitch: 0, yaw: 0 };
  const angle = (a: THREE.Vector3, b: THREE.Vector3) => (a.angleTo(b) * 180) / Math.PI;

  /** 着色器和 JS 都靠这个公式取贴图，和球的 uv 对不上就是整张错位 */
  it("方向 → 贴图坐标和环境球的顶点 uv 一致", () => {
    const g = makeSphereGeometry();
    const pos = g.getAttribute("position");
    const uv = g.getAttribute("uv");
    const v = new THREE.Vector3();
    for (let i = 0; i < pos.count; i += 7) {
      v.fromBufferAttribute(pos, i);
      if (Math.abs(v.clone().normalize().y) > 0.99) continue; // 两极 u 无定义
      const got = equirectUv(v);
      expect(uDist(got.u, uv.getX(i))).toBeLessThan(1e-4);
      expect(Math.abs(got.v - uv.getY(i))).toBeLessThan(1e-4);
    }
  });

  it("屏幕中心看的是相机正前方，上边缘正好是视场角的一半", () => {
    expect(angle(stereoRay(0, 0, 270), new THREE.Vector3(0, 0, -1))).toBeLessThan(1e-6);
    for (const fov of [120, 270, 330]) {
      expect(angle(stereoRay(0, 1, fov), new THREE.Vector3(0, 0, -1))).toBeCloseTo(fov / 2, 6);
    }
  });

  /** 不镜像：屏幕右边 = 相机右边（+X），屏幕上边 = 相机上边（+Y） */
  it("屏幕右边是右、上边是上", () => {
    expect(stereoRay(0.3, 0, 200).x).toBeGreaterThan(0);
    expect(stereoRay(0, 0.3, 200).y).toBeGreaterThan(0);
  });

  it("朝正下方看，屏幕中心是地面正下方、画面边缘能看到天空", () => {
    const view = { correction: none, yawTotalDeg: 0, pitchDeg: -90, fovDeg: 270 };
    expect(planetUv(0, 0, view).v).toBeLessThan(0.01); // 贴图最底行
    expect(planetUv(0, 1, view).v).toBeGreaterThan(0.6); // 上边缘已经过了地平线
  });

  /** 局部不变形：同一个小圆，放在画面中心和偏一点的地方，横竖张开的角度比例一致 */
  it("球极投影保角：小圆不会被拉成长条", () => {
    const fov = 270;
    for (const [cx, cy] of [[0, 0], [0.6, 0.4], [1.2, -0.7]]) {
      const c = stereoRay(cx, cy, fov);
      const dx = angle(stereoRay(cx + 1e-5, cy, fov), c);
      const dy = angle(stereoRay(cx, cy + 1e-5, fov), c);
      expect(dx / dy).toBeCloseTo(1, 3);
    }
  });
});
