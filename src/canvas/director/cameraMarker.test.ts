import { describe, expect, it } from "vitest";
import * as THREE from "three";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { CAMERA_COLORS, cameraColor, cameraIconSvg, createCameraMarker, DEFAULT_CAMERA_COLOR } from "./cameraMarker";
import { readNamedCameras } from "./scene";

const cam = (yawDeg = 0) => ({ position: [1, 1.6, -2] as [number, number, number], yawDeg, pitchDeg: -5, fovDeg: 50, rollDeg: 0 });

describe("机位图标（SVG 摄像机 + 取景框）", () => {
  it("图标用机位的颜色；选中白描边；当前机位有红色录制灯；颜色不合法回落默认", () => {
    const plain = cameraIconSvg("#5ab4ff", false, false);
    expect(plain).toContain('fill="#5AB4FF"');
    expect(plain).not.toContain("#FF3B30");
    expect(cameraIconSvg("#5AB4FF", true, true)).toContain('stroke="#FFFFFF"');
    expect(cameraIconSvg("#5AB4FF", false, true)).toContain("#FF3B30");
    expect(cameraColor("红色")).toBe(DEFAULT_CAMERA_COLOR);
    expect(cameraColor(undefined)).toBe(CAMERA_COLORS[0]);
  });

  it("pivot 在机位点、按机位转；取景框跟颜色，选中变白", () => {
    const m = createCameraMarker({ id: "c1", camera: cam(30), aspect: 16 / 9, active: false, selected: false, color: "#4ADE80" });
    expect(m.pivot.position.toArray()).toEqual([1, 1.6, -2]);
    expect(m.pivot.rotation.y).toBeCloseTo((-30 * Math.PI) / 180);
    const lines = m.pivot.children.find((c) => c instanceof THREE.LineSegments) as THREE.LineSegments;
    expect(`#${(lines.material as THREE.LineBasicMaterial).color.getHexString().toUpperCase()}`).toBe("#4ADE80");
    m.update({ id: "c1", camera: cam(30), aspect: 16 / 9, active: false, selected: true, color: "#4ADE80" });
    expect((lines.material as THREE.LineBasicMaterial).color.getHexString()).toBe("ffffff");
    expect(m.sprite.userData.cameraId).toBe("c1");
    m.dispose();
  });

  it("正在拖坐标轴时 React 回写不把位置和朝向打回去", () => {
    const m = createCameraMarker({ id: "c1", camera: cam(), aspect: 1, active: false, selected: true });
    m.pivot.position.set(5, 2, 0);
    m.pivot.rotation.set(0.3, 0.2, 0.1, "YXZ");
    m.update({ id: "c1", camera: cam(), aspect: 1, active: false, selected: true }, { keepPose: true });
    expect(m.pivot.position.toArray()).toEqual([5, 2, 0]);
    expect(m.pivot.rotation.x).toBeCloseTo(0.3);
    m.update({ id: "c1", camera: cam(), aspect: 1, active: false, selected: true });
    expect(m.pivot.position.toArray()).toEqual([1, 1.6, -2]);
  });

  it("取景框在屏幕上朝左时图标镜像（图标画的是镜头朝右的侧面）", () => {
    const view = new THREE.PerspectiveCamera(50, 1, 0.1, 100);
    view.position.set(1, 1.6, 5);
    view.lookAt(1, 1.6, -2);
    view.updateMatrixWorld();
    // yaw 90 = 朝 +X（屏幕右）；yaw -90 = 朝 -X（屏幕左）
    const right = createCameraMarker({ id: "r", camera: { ...cam(90), pitchDeg: 0 }, aspect: 1, active: false, selected: false });
    const left = createCameraMarker({ id: "l", camera: { ...cam(-90), pitchDeg: 0 }, aspect: 1, active: false, selected: false });
    for (const m of [right, left]) {
      m.pivot.updateMatrixWorld(true);
      m.faceScreen(view);
    }
    expect(right.sprite.scale.x).toBeGreaterThan(0);
    expect(left.sprite.scale.x).toBeLessThan(0);
  });

  it("机位颜色跟着存盘读回；不是 #RRGGBB 的丢掉", () => {
    const got = readNamedCameras([{ id: "a", name: "机位 1", camera: {}, color: "#5ab4ff" }, { id: "b", name: "机位 2", camera: {}, color: "blue" }]);
    expect(got[0].color).toBe("#5AB4FF");
    expect(got[1].color).toBeUndefined();
  });
});

describe("接线", () => {
  const editor = readFileSync(fileURLToPath(new URL("./DirectorWorldEditor.tsx", import.meta.url)), "utf8");
  const engine = readFileSync(fileURLToPath(new URL("./engine.ts", import.meta.url)), "utf8");
  it("右栏：机位视角显示摄像机，导演视角一律 3D 场景（用户 2026-09-28）", () => {
    expect(editor).toContain('const selectedCam = viewMode === "camera" ? namedCameras.find((c) => c.id === activeCameraId) ?? null : null;');
  });
  it("XYZ 各有一个回弹操作杆，速度按偏移平方；Esc 放下选中的机位", () => {
    expect((editor.match(/<JogLever/g) ?? []).length).toBe(1);
    expect(editor).toContain("jog.current(Math.sign(o) * o * o * JOG_MAX_SPEED * dt)");
    expect(editor).toContain("else if (selectedCameraIdRef.current) setSelectedCameraId(null);");
  });
  it("引擎：机位按 id 留着同一个对象（坐标轴挂在它上面）；点图标选中；拖完报位置", () => {
    expect(engine).toContain("function reconcileMarkers(list: CameraMarker[])");
    expect(engine).not.toContain("clearMarkers()");
    expect(engine).toContain("const target = entry?.m.root ?? camMarker?.pivot ?? null;");
    expect(engine).toContain("opts.onCameraPose?.(camMarker.id, {");
    expect(engine).toContain("opts.onSelectCamera?.(camId);");
  });
});

/** 用户 2026-09-28：「点击某一个机位，要自动切换到这个机位的真实视角，可移动，但是点击重置视角要恢复到这个摄像机本身的观测视角」
 *  「摄像机缺少了旋转功能」「导演视角的重置视角是有效的，要调整的是机位视角的重置」 */
describe("机位视角透过机位看、重置按视角分；坐标轴能旋转", () => {
  const editor = readFileSync(fileURLToPath(new URL("./DirectorWorldEditor.tsx", import.meta.url)), "utf8");
  const engine = readFileSync(fileURLToPath(new URL("./engine.ts", import.meta.url)), "utf8");
  it("重置视角：机位视角回到当前机位，导演视角回到标准视角", () => {
    expect(editor).toContain('onClick={() => (viewMode === "camera" ? lookThrough(activeCameraId) : engineRef.current?.resetDirectorView())}');
  });
  it("切到机位视角、点机位（左栏、名字标签、图标、右栏切换）都透过它看；自己转开了就不再跟着机位走", () => {
    expect(editor).toContain("engineRef.current?.viewFrom(target.camera);");
    expect(editor).toContain("lookThroughRef.current(id);");
    expect(editor).toContain("onSelectCamera={(id) => lookThrough(id)}");
    expect(editor).toContain("else setThroughLens(false);");
    expect(editor).toContain('if (viewMode === "camera" && throughLens) engineRef.current?.viewFrom(camera);');
  });
  it("回导演视角回到离开时那个位置", () => {
    expect(editor).toContain("engine.setCamera(directorCamRef.current ?? directorViewFrom(camera));");
  });
  it("机位坐标轴能切旋转：按机位自己的轴转，松手报位置和角度，转过了不再自动注视", () => {
    expect(engine).toContain('gizmo.setMode(rotate ? "rotate" : "translate");');
    expect(engine).toContain('gizmo.setSpace(rotate ? "local" : "world");');
    expect(engine).toContain("yawDeg: deg(-r.y),");
    expect(editor).toContain("...(turned ? { lookAt: null } : {})");
  });
  it("viewFrom 不改机位本身，绕转中心就在镜头前（不继承导演视角几十米的绕转半径）", () => {
    const fn = engine.slice(engine.indexOf("    viewFrom(c, pivot = 2) {"));
    expect(fn.slice(0, 200)).toContain("orbitFrom(c, pivot)");
    expect(fn.slice(0, 200)).not.toContain("onCameraChange");
  });
});

/** 用户 2026-09-28：「为什么人偶一拖动就会被水平线吸回来。libtv 里是可以上天入地的」 */
describe("人偶上天入地", () => {
  const engine = readFileSync(fileURLToPath(new URL("./engine.ts", import.meta.url)), "utf8");
  const dragEnd = engine.slice(engine.indexOf('gizmo.addEventListener("dragging-changed"'), engine.indexOf('gizmo.addEventListener("objectChange"'));
  it("高斯地面吸附关着：松手不动高度（能上天入地）；开着：只能在地面上 —— 松手贴地、绿箭头隐藏", () => {
    expect(dragEnd).toContain("if (gaussianGroundSnap) p.y = splatGroundAt(p.x, p.z)?.[1] ?? groundHeight;");
    expect(dragEnd).not.toMatch(/\n\s*p\.y = surface\?\.\[1\] \?\? groundHeight;/);
    expect(dragEnd).toContain("if (gizmoDragging) dragAxis = gizmo.axis;");
    expect(engine).toContain("gizmo.showY = !(entry && gaussianGroundSnap);");
    expect(engine).toMatch(/setGaussianGroundSnap\(on\) \{\s*gaussianGroundSnap = on;\s*syncGizmo\(\);/);
  });
  it("离地超过 5 厘米脚下画一个同色的圈，拖的时候跟着走", () => {
    expect(engine).toContain("ring.visible = Math.abs(p.y - groundHeight) > RING_MIN_LIFT;");
    const change = engine.slice(engine.indexOf('gizmo.addEventListener("objectChange"'));
    expect(change.slice(0, 1400)).toMatch(/updateSelectionBox\(\);\s*syncGroundRings\(\);/);
  });
});

/** 用户 2026-09-28：「高斯地面吸附和网格吸附都有开关，但是没有关联」 */
describe("两个吸附分工叠加", () => {
  const engine = readFileSync(fileURLToPath(new URL("./engine.ts", import.meta.url)), "utf8");
  it("网格吸附（照 libtv）：水平一拖 x、z 一起吸到交叉点，拖着就吸；只拖 Y 不动水平；松手不再取整", () => {
    const change = engine.slice(engine.indexOf('gizmo.addEventListener("objectChange"'));
    expect(change.slice(0, 1400)).toContain('if (gridSnap && entry && gizmo.object === entry.m.root && dragAxis && dragAxis !== "Y") {');
    const dragEnd = engine.slice(engine.indexOf('gizmo.addEventListener("dragging-changed"'), engine.indexOf('gizmo.addEventListener("objectChange"'));
    expect(dragEnd).not.toContain("p.x = Math.round(p.x)");
  });
  it("放置：先网格定水平位置，再在那儿找 3D 片场地表定高度", () => {
    const gp = engine.slice(engine.indexOf("function groundPoint("), engine.indexOf("function snapPoint("));
    expect(gp).toContain("const [x, y, z] = snapPoint(hit);");
    expect(gp).toContain("splatGroundAt(x, z)");
  });
});
