import { describe, expect, it } from "vitest";
import * as THREE from "three";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { createPanoDome, DEFAULT_SPHERE_RADIUS, DOME_RADIUS, groundedDomeGeometry, SPHERE_RADIUS_RANGE, sphereDomeGeometry, worldToTexture } from "./world";
import { textureDirectionAt } from "../pano/panoOrient";

describe("地面投影穹顶", () => {
  const R = 60;
  const h = 1.6;

  /**
   * **压平只能改距离，不能改方向。** 取色靠「片元到拍摄点的方向」，
   * 方向变了整张全景就错位了。
   */
  it("每个顶点从拍摄点看过去的方向不变", () => {
    const flat = groundedDomeGeometry(R, h, 32).getAttribute("position");
    const orig = new THREE.SphereGeometry(R, 64, 32).getAttribute("position");
    const a = new THREE.Vector3();
    const b = new THREE.Vector3();
    for (let i = 0; i < flat.count; i++) {
      a.fromBufferAttribute(flat, i);
      b.fromBufferAttribute(orig, i);
      if (b.length() < 1e-6) continue;
      expect(a.clone().normalize().distanceTo(b.normalize())).toBeLessThan(1e-6);
    }
  });

  /** 地面上的点正好在拍摄点下方 h 米，也就是世界里的 y = 0 */
  it("往下看得够陡的点都压到了地面上", () => {
    const flat = groundedDomeGeometry(R, h, 32).getAttribute("position");
    const v = new THREE.Vector3();
    let onFloor = 0;
    for (let i = 0; i < flat.count; i++) {
      v.fromBufferAttribute(flat, i);
      const dir = v.clone().normalize();
      // 比「-1.5h 那条过渡线」更陡的方向
      if (dir.y < (-1.5 * h) / R) {
        expect(v.y).toBeCloseTo(-h, 6);
        onFloor++;
      }
    }
    expect(onFloor).toBeGreaterThan(100);
  });

  it("地面上一点的距离 = h / tan(俯角)", () => {
    const flat = groundedDomeGeometry(R, h, 64).getAttribute("position");
    const v = new THREE.Vector3();
    for (let i = 0; i < flat.count; i++) {
      v.fromBufferAttribute(flat, i);
      if (Math.abs(v.y + h) > 1e-6) continue;
      const dist = Math.hypot(v.x, v.z);
      const depression = Math.atan2(h, dist);
      expect(h / Math.tan(depression)).toBeCloseTo(dist, 6);
    }
  });
});

describe("世界方向 → 贴图方向", () => {
  /** 和全景查看器同一个约定：导演世界里朝某方向看到的，和查看器里同样的 yaw/pitch 看到的是贴图同一处 */
  it("和全景查看器的 textureDirectionAt 一致", () => {
    const cases = [
      { c: { roll: 0, pitch: 0, yaw: 0 }, front: 0, yaw: 30, pitch: -20 },
      { c: { roll: 3, pitch: -5, yaw: 20 }, front: 40, yaw: -70, pitch: 10 },
      { c: { roll: -8, pitch: 12, yaw: -170 }, front: -120, yaw: 150, pitch: -45 },
    ];
    for (const k of cases) {
      // 相机：yaw 往右为正、看向 -Z（和 scene.applyCamera 一致），导演世界里 yaw 不含正前方偏移
      const cam = new THREE.Quaternion().setFromEuler(new THREE.Euler((k.pitch * Math.PI) / 180, (-k.yaw * Math.PI) / 180, 0, "YXZ"));
      const world = new THREE.Vector3(0, 0, -1).applyQuaternion(cam);
      const got = world.applyMatrix3(worldToTexture(k.c, k.front)).normalize();
      // 查看器里 yawTotal = yaw + front
      const want = textureDirectionAt(k.c, k.yaw + k.front, k.pitch);
      expect(got.distanceTo(want)).toBeLessThan(1e-6);
    }
  });
});

/**
 * 用户 2026-09-28 对比 libtv：「你这个导演世界不是个球型啊，怎么是个半球」「半球底部图形畸变，完全看不出地面」。
 * 两种工作视角和截图都保持完整球；网格单独表示真实地面。
 */
describe("导演视角和机位视角都保持完整球", () => {
  it("球：每个顶点都在半径上（底部没被压平）", () => {
    const pos = sphereDomeGeometry(10, 16).getAttribute("position");
    const v = new THREE.Vector3();
    let lowest = 0;
    for (let i = 0; i < pos.count; i++) {
      v.fromBufferAttribute(pos, i);
      expect(v.length()).toBeCloseTo(10, 4);
      lowest = Math.min(lowest, v.y);
    }
    expect(lowest).toBeCloseTo(-10, 4);
  });

  it("换形状、换半径只换几何体；地面投影那份不跟着变", () => {
    const dome = createPanoDome(1.6);
    const grounded = dome.mesh.geometry;
    expect(dome.shape()).toBe("grounded");
    dome.setShape("sphere");
    expect(dome.mesh.geometry).not.toBe(grounded);
    expect(dome.mesh.geometry.boundingSphere?.radius ?? new THREE.Box3().setFromBufferAttribute(dome.mesh.geometry.getAttribute("position") as THREE.BufferAttribute).max.x).toBeCloseTo(DEFAULT_SPHERE_RADIUS, 1);
    dome.setSphereRadius(1000);
    const r = new THREE.Box3().setFromBufferAttribute(dome.mesh.geometry.getAttribute("position") as THREE.BufferAttribute).max.x;
    expect(r).toBeCloseTo(SPHERE_RADIUS_RANGE[1], 1);
    dome.setShape("grounded");
    expect(dome.mesh.geometry).toBe(grounded);
    // 眼高变了，切回球的时候球还在，地面投影那份换成新眼高
    dome.setHeight(2);
    expect(dome.mesh.position.y).toBe(2);
    expect(new THREE.Box3().setFromBufferAttribute(dome.mesh.geometry.getAttribute("position") as THREE.BufferAttribute).max.x).toBeCloseTo(DOME_RADIUS, 0);
    dome.dispose();
  });

  it("切换页签和截图都使用完整球，截完换回原来的形状", () => {
    const src = readFileSync(fileURLToPath(new URL("./engine.ts", import.meta.url)), "utf8");
    expect(src).toContain('dome.setShape("sphere");');
    const capture = src.slice(src.indexOf("capture(mode, rectCss"));
    expect(capture).toContain('dome.setShape("sphere");');
    expect(capture.slice(capture.indexOf("} finally {"))).toContain("dome.setShape(before.shape);");
  });
});
