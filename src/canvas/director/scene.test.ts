import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import * as THREE from "three";
import { GRID_FADE_FAR } from "./world";
import { KNOWN_CLIPS, POSES, POSE_CLIPS, resolvePoseClip } from "./poses";
import {
  splatPlacement,
  readNamedCameras,
  directorViewFrom,
  interpolateCamera,
  applyCamera,
  actorProblems,
  bindIdentity,
  blockingFor,
  nextCameraName,
  crowdGrid,
  CROWD_COLOR,
  isPerson,
  buildFrameMeta,
  cameraPreset,
  lookAtAngles,
  defaultCamera,
  facingCameraYaw,
  facingOf,
  frameRect,
  migrateMarkers,
  nextAnonymousLabel,
  offsetFromCapturePoint,
  projectActor,
  readActors,
  readCamera,
  type DirectorActor,
} from "./scene";

const actor = (id: string, x: number, z: number, extra: Partial<DirectorActor> = {}): DirectorActor => ({
  id,
  label: id,
  color: "#FF00FF",
  position: [x, 0, z],
  yawDeg: 0,
  scale: 1,
  pose: "standing",
  ...extra,
});

describe("姿势表", () => {
  /** 映射表写错一个字，那个姿势就会静默退回 T-pose */
  it("15 个姿势都至少有一个真实存在的片段", () => {
    expect(POSES).toHaveLength(15);
    const known = new Set(KNOWN_CLIPS);
    for (const p of POSES) {
      expect(resolvePoseClip(p.id, known), p.id).not.toBeNull();
    }
  });

  it("候选按顺序取，前面的没有才用后面的", () => {
    expect(resolvePoseClip("standing", new Set(["Idle_No_Loop", "Idle_Loop"]))?.name).toBe("Idle_Loop");
    expect(resolvePoseClip("standing", new Set(["Idle_No_Loop"]))?.name).toBe("Idle_No_Loop");
    expect(resolvePoseClip("standing", new Set())).toBeNull();
    expect(POSE_CLIPS.crouching.sampleTime).toBe(0.25);
  });
});

describe("读数据", () => {
  it("坏人偶丢掉，缺字段补默认", () => {
    const got = readActors([
      { id: "a", position: [1, 0, 2], pose: "nope", color: "red" },
      { id: "b" },
      null,
      { position: [0, 0, 0] },
    ]);
    expect(got).toHaveLength(1);
    expect(got[0]).toMatchObject({ id: "a", pose: "standing", color: "#FFFFFF", scale: 1, yawDeg: 0 });
  });

  it("相机参数越界时夹住", () => {
    const c = readCamera({ position: [1, 2, 3], pitchDeg: 200, fovDeg: 1 });
    expect(c.position).toEqual([1, 2, 3]);
    expect(c.pitchDeg).toBe(89);
    expect(c.fovDeg).toBe(10);
    expect(readCamera(null, 1.2).position).toEqual([0, 1.2, 0]);
  });

  it("匿名人物编号取最大值加一", () => {
    expect(nextAnonymousLabel([])).toBe("匿名人物 1");
    expect(nextAnonymousLabel([actor("匿名人物 1", 0, 0), actor("匿名人物 4", 0, 0), actor("沈星晴", 0, 0)])).toBe("匿名人物 5");
  });
});

describe("旧标记迁移", () => {
  it("方位角 + 距离换成 xyz，面朝相机，道具丢掉", () => {
    const got = migrateMarkers([
      { id: "m1", kind: "actor", label: "甲", yawDeg: 0, distance: 3, color: "#00FFFF" },
      { id: "m2", kind: "actor", label: "乙", yawDeg: 90, distance: 2, color: "#CCFF00" },
      { id: "m3", kind: "prop", label: "桌子", yawDeg: 0, distance: 1, color: "#FFD34D" },
    ]);
    expect(got.map((a) => a.id)).toEqual(["m1", "m2"]);
    expect(got[0].position).toEqual([0, 0, -3]);
    expect(got[1].position).toEqual([2, 0, -0]);
    for (const a of got) {
      expect(facingOf(a, { ...defaultCamera(), position: [0, 1.6, 0] })).toBe("正面");
    }
  });
});

describe("站位", () => {
  const cam = { position: [0, 1.6, 0] as [number, number, number], yawDeg: 0, pitchDeg: 0, fovDeg: 70 };

  it("正前方的人在画面中央，偏左偏右各归一边", () => {
    const rows = blockingFor(
      [actor("中", 0, -4), actor("左", -3, -4), actor("右", 3, -4)],
      cam,
      16 / 9,
    );
    expect(rows.map((r) => [r.actor, r.zone])).toEqual([
      ["左", "画面左侧"],
      ["中", "画面中央"],
      ["右", "画面右侧"],
    ]);
  });

  /** 背后的人投影后 NDC 也可能落在 [-1,1] 里 —— 必须按「在不在相机前面」排除 */
  it("相机背后的人不算进画面", () => {
    expect(projectActor(actor("背后", 0, 4), cam, 16 / 9).inFrame).toBe(false);
    expect(blockingFor([actor("背后", 0, 4)], cam, 16 / 9)).toEqual([]);
  });

  it("相机往右转之后，原来正前方的人跑到画面左边", () => {
    const turned = { ...cam, yawDeg: 25 };
    expect(blockingFor([actor("甲", 0, -4)], turned, 16 / 9)[0].zone).toBe("画面左侧");
  });

  it("前中后景按水平距离分", () => {
    const rows = blockingFor([actor("近", 0, -2), actor("中", 0.1, -5), actor("远", -0.1, -9)], cam, 16 / 9);
    const byName = Object.fromEntries(rows.map((r) => [r.actor, r.depth]));
    expect(byName).toEqual({ 近: "前景", 中: "中景", 远: "后景" });
  });

  it("朝向：面朝相机正面，背对背面，横着侧面", () => {
    const at: [number, number, number] = [0, 0, -4];
    expect(facingOf(actor("a", 0, -4, { yawDeg: facingCameraYaw(at, [0, 0, 0]) }), cam)).toBe("正面");
    expect(facingOf(actor("a", 0, -4, { yawDeg: 180 + facingCameraYaw(at, [0, 0, 0]) }), cam)).toBe("背面");
    expect(facingOf(actor("a", 0, -4, { yawDeg: 90 }), cam)).toBe("侧面");
  });

  /** 画幅变窄（竖屏），偏一点的人就出框了 —— 站位按画幅框算，不按视口 */
  it("竖屏画幅下偏左的人出框", () => {
    const a = actor("偏左", -1.6, -4);
    expect(projectActor(a, cam, 16 / 9).inFrame).toBe(true);
    expect(projectActor(a, cam, 9 / 16).inFrame).toBe(false);
  });
});

describe("画幅框", () => {
  it("宽视口放竖画幅：高度撑满、居中", () => {
    const r = frameRect(1600, 900, "9:16");
    expect(r.height).toBe(900);
    expect(r.width).toBe(506);
    expect(r.x).toBe(Math.round((1600 - 506.25) / 2));
    expect(r.y).toBe(0);
  });

  it("窄视口放横画幅：宽度撑满，留边距", () => {
    const r = frameRect(800, 900, "16:9", 20);
    expect(r.width).toBe(760);
    expect(r.height).toBe(428);
    expect(r.x).toBe(20);
  });
});

describe("控制帧元数据", () => {
  it("带上颜色名、身份和画面位置；画面外的留着但没有位置", () => {
    const meta = buildFrameMeta({
      sourceUrl: "/p.png",
      context: { project_id: "p", episode_id: "e", shot_id: "s3" },
      aspect: "16:9",
      camera: { position: [0, 1.6, 0], yawDeg: 0, pitchDeg: 0, fovDeg: 70 },
      actors: [
        actor("沈星晴", 0, -4, { assetId: "char_1", pose: "sitting" }),
        actor("匿名人物 1", 0, 5, { color: "#00FFFF" }),
      ],
      colorNames: { "#FF00FF": "FLUORESCENT MAGENTA" },
    });
    expect(meta.schema_version).toBe("anima_director_frame_v2");
    expect(meta.camera.frame_aspect).toBe("16:9");
    const [a, b] = meta.actors;
    expect(a).toMatchObject({ asset_id: "char_1", color_name: "FLUORESCENT MAGENTA", pose_label: "坐下", zone: "画面中央", in_frame: true });
    expect(a.screen!.x).toBeCloseTo(0.5, 2);
    expect(b).toMatchObject({ asset_id: null, color_name: null, in_frame: false, zone: null, screen: null });
    expect(meta.blocking.map((r) => r.actor)).toEqual(["沈星晴"]);
  });

  it("3DGS 世界优先写入 splat 来源和格式", () => {
    const meta = buildFrameMeta({
      sourceUrl: "/fallback-pano.png",
      splat: { path: "/project/worlds/workshop.ksplat", format: "ksplat" },
      context: null,
      aspect: "16:9",
      camera: defaultCamera(),
      actors: [],
    });
    expect(meta.source).toEqual({
      type: "splat",
      path: "/project/worlds/workshop.ksplat",
      format: "ksplat",
    });
  });
});

describe("相机预设", () => {
  const cam = { position: [0, 1.6, 0] as [number, number, number], yawDeg: 0, pitchDeg: 0, fovDeg: 70 };

  it("看向正前方的点 yaw 0，右边 yaw 90，往下看 pitch 为负", () => {
    expect(lookAtAngles([0, 0, 0], [0, 0, -5]).yawDeg).toBe(0);
    expect(lookAtAngles([0, 0, 0], [5, 0, 0]).yawDeg).toBe(90);
    expect(lookAtAngles([0, 2, 0], [0, 0, -2]).pitchDeg).toBe(-45);
  });

  /** 切到正面后，这个人在画面中央、而且是正脸；切到背后是背影 */
  it("切到正面是正脸居中，切到背后是背影", () => {
    const a = actor("甲", 3, -6, { yawDeg: 40 });
    const face = cameraPreset(a, "face", cam);
    expect(facingOf(a, face)).toBe("正面");
    expect(Math.abs(projectActor(a, face, 16 / 9).ndcX)).toBeLessThan(0.05);
    const behind = cameraPreset(a, "behind", cam);
    expect(facingOf(a, behind)).toBe("背面");
    expect(projectActor(a, behind, 16 / 9).inFrame).toBe(true);
  });

  it("原地看向：位置不动，人在画面中央", () => {
    const a = actor("甲", -4, -2);
    const look = cameraPreset(a, "look", cam);
    expect(look.position).toEqual(cam.position);
    expect(Math.abs(projectActor(a, look, 16 / 9).ndcX)).toBeLessThan(0.05);
  });
});

describe("输出前检查", () => {
  it("两人同色要报出来", () => {
    const got = actorProblems([actor("甲", 0, 0), actor("乙", 1, 0), actor("丙", 2, 0, { color: "#00FFFF" })], false);
    expect(got).toHaveLength(1);
    expect(got[0]).toContain("「甲」和「乙」");
  });

  /** 切到主线模式之前摆的匿名人物，颜色表里查不到他是谁 */
  it("主线模式下没绑角色的人要报出来，自由模式不报", () => {
    const list = [actor("匿名人物 1", 0, 0, { color: "#39FF14" }), actor("沈星晴", 1, 0, { assetId: "char_a" })];
    expect(actorProblems(list, false)).toEqual([]);
    const got = actorProblems(list, true);
    expect(got).toHaveLength(1);
    expect(got[0]).toContain("匿名人物 1");
  });

  it("绑定角色：名字颜色 id 一起换；角色已在场就拒绝", () => {
    const list = [actor("匿名人物 1", 0, 0), actor("沈星晴", 1, 0, { assetId: "char_a", color: "#00FFFF" })];
    const who = { asset_id: "char_b", name: "陆临川", color: "#CCFF00" };
    const ok = bindIdentity(list, "匿名人物 1", who);
    expect("actors" in ok && ok.actors[0]).toMatchObject({ label: "陆临川", color: "#CCFF00", assetId: "char_b" });
    const clash = bindIdentity(list, "匿名人物 1", { asset_id: "char_a", name: "沈星晴", color: "#00FFFF" });
    expect(clash).toEqual({ error: "「沈星晴」已经在场了" });
  });
});

describe("镜头离开拍摄点", () => {
  const cam = (x: number, y: number, z: number) => ({ position: [x, y, z] as [number, number, number], yawDeg: 0, pitchDeg: 0, fovDeg: 70 });

  it("有全景时离开拍摄点超过 0.3 米不许输出；没全景（黑色空世界）随便飞", () => {
    expect(actorProblems([], false, { cam: cam(0, 1.6, 0), eyeHeight: 1.6, hasPano: true })).toEqual([]);
    expect(actorProblems([], false, { cam: cam(0.2, 1.6, -0.1), eyeHeight: 1.6, hasPano: true })).toEqual([]);
    const far = actorProblems([], false, { cam: cam(0, 1.6, -2), eyeHeight: 1.6, hasPano: true });
    expect(far).toHaveLength(1);
    expect(far[0]).toContain("2.0 米");
    expect(actorProblems([], false, { cam: cam(0, 1.6, -2), eyeHeight: 1.6, hasPano: false })).toEqual([]);
  });

  /** 眼高改了、镜头还在原来的高度，也算离开 —— 不然地面和人偶的比例是错的 */
  it("眼高变了高度差也算", () => {
    expect(offsetFromCapturePoint(cam(0, 1.6, 0), 1.2)).toBeCloseTo(0.4, 6);
  });
});

describe("3DGS 片场放进导演世界", () => {
  /**
   * SHARP 输出原相机坐标（+Z 前、+Y 下）。2026-09-27 实跑拆机间：点全在 z > 0，脚下拆机台 y ≈ +0.9。
   * 不转就在相机背后、上下颠倒。
   */
  it("SHARP 的原相机坐标：绕 X 翻 180°、原点抬到眼高；来源不明的原样放", () => {
    const p = splatPlacement("opencv_camera", 1.6);
    expect(p).toEqual({ rotation: [1, 0, 0, 0], position: [0, 1.6, 0] });
    // 四元数 [1,0,0,0] 把 SHARP 里「前方 z=+5、脚下 y=+0.9」的点送到导演世界的「前方 −Z、眼高以下」
    const q = new THREE.Quaternion().fromArray(p.rotation!);
    const v = new THREE.Vector3(0, 0.9, 5).applyQuaternion(q).add(new THREE.Vector3(...p.position!));
    expect(v.z).toBeCloseTo(-5);
    expect(v.y).toBeCloseTo(0.7);
    expect(splatPlacement(undefined, 1.6)).toEqual({});
  });

  /** 运镜首尾帧是「片场 + 彩色人偶」的合成稿，挂成首帧 H3 会出一段人偶戏 */
  it("导演世界不自动把人偶稿挂成分镜首帧 / 尾帧", () => {
    const src = readFileSync(fileURLToPath(new URL("../nodes/DirectorWorldNode.tsx", import.meta.url)), "utf8");
    expect(src).not.toContain("cinemaSetShotKeyframe");
  });
});

describe("3DGS 渲染库的两个黑屏坑", () => {
  /** 2026-09-27 实测：0.4.7 连库自带查看器都黑；GPU 排序开着也黑。两个都不报错 */
  it("gaussian-splats-3d 锁在 0.4.6，引擎不开 GPU 排序", () => {
    const pkg = JSON.parse(readFileSync(fileURLToPath(new URL("../../../package.json", import.meta.url)), "utf8"));
    expect(pkg.dependencies["@mkkellogg/gaussian-splats-3d"]).toBe("0.4.6");
    const engine = readFileSync(fileURLToPath(new URL("./engine.ts", import.meta.url)), "utf8");
    expect(engine).toContain("gpuAcceleratedSort: false");
    expect(engine).not.toContain("gpuAcceleratedSort: true");
  });

  /** 放置规则只写在 splatPlacement 里；引擎要真的用它，SHARP 的结果要带上坐标系 */
  it("引擎载入时套用 splatPlacement；SHARP 生成的片场标成原相机坐标", () => {
    const engine = readFileSync(fileURLToPath(new URL("./engine.ts", import.meta.url)), "utf8");
    expect(engine).toContain("...splatPlacement(source.convention, eyeHeight, splatScale)");
    const node = readFileSync(fileURLToPath(new URL("../nodes/DirectorWorldNode.tsx", import.meta.url)), "utf8");
    expect(node).toContain('convention: "opencv_camera"');
    const editor = readFileSync(fileURLToPath(new URL("./DirectorWorldEditor.tsx", import.meta.url)), "utf8");
    expect(editor).toContain("readSplatBlobUrl(splatSource.path)");
    expect(editor).not.toContain("mediaDataUrl(splatSource.path)");
  });
});

describe("导演世界的网格和眼高", () => {
  /** 网格一直铺到地平线会穿墙、浮在画面中间；只铺在摆人的范围里 */
  it("网格 20 米内淡没", () => {
    expect(GRID_FADE_FAR).toBeLessThanOrEqual(20);
  });

  /** 眼高滑块要把 SHARP 片场一起挪；主视口始终是独立观察相机，不再要求它回到出片机位 */
  it("眼高改了 SHARP 片场跟着挪；外部观察不再弹「回到拍摄点」", () => {
    const engine = readFileSync(fileURLToPath(new URL("./engine.ts", import.meta.url)), "utf8");
    expect(engine).toContain("dynamicScene: true");
    const eye = engine.slice(engine.indexOf("setEyeHeight(h) {"));
    expect(eye.slice(0, eye.indexOf("\n    },"))).toContain("getSplatScene(0).position.set(");
    const editor = readFileSync(fileURLToPath(new URL("./DirectorWorldEditor.tsx", import.meta.url)), "utf8");
    expect(editor).not.toContain("cameraOffset > CAPTURE_POINT_TOLERANCE");
    expect(editor).toContain('engine.setNavMode("orbit")');
    expect(editor).toContain("return engine.withCamera(camera, () => fn(engine));");
  });
});

describe("机位是对象（界面升级 U2）", () => {
  it("横滚正 = 画面顺时针歪：世界里竖直的杆子，顶端往画面右边倒", () => {
    const c = new THREE.PerspectiveCamera(60, 16 / 9, 0.1, 100);
    applyCamera(c, { position: [0, 1.6, 0], yawDeg: 0, pitchDeg: 0, fovDeg: 60, rollDeg: 20 }, 16 / 9);
    const base = new THREE.Vector3(0, 1.6, -5).project(c);
    const top = new THREE.Vector3(0, 2.6, -5).project(c);
    expect(top.x).toBeGreaterThan(base.x);
  });

  it("存档里的机位：坏的丢掉、缺的补默认、老数据没有横滚当 0", () => {
    const list = readNamedCameras([{ id: "a", name: "低机位", camera: { position: [0, 0.4, 1], yawDeg: 10, pitchDeg: 15, fovDeg: 50 }, lookAt: "actor-1" }, { name: "没 id" }, null], 1.6);
    expect(list).toHaveLength(1);
    expect(list[0]).toMatchObject({ id: "a", name: "低机位", lookAt: "actor-1" });
    expect(list[0].camera.rollDeg).toBe(0);
    expect(readNamedCameras("坏数据")).toEqual([]);
  });

  it("导演视角从机位身后、上方起步，看得到机位", () => {
    const d = directorViewFrom({ position: [0, 1.6, 0], yawDeg: 0, pitchDeg: 0, fovDeg: 50 });
    expect(d.position).toEqual([0, 2.8, 3]);
    expect(d.pitchDeg).toBeLessThan(0);
  });

  it("运镜插值带上横滚", () => {
    const a = { position: [0, 1, 0] as [number, number, number], yawDeg: 0, pitchDeg: 0, fovDeg: 50, rollDeg: 0 };
    expect(interpolateCamera(a, { ...a, rollDeg: 20 }, 0.5).rollDeg).toBe(10);
  });
});

describe("道具和群演（界面升级 U3.5）", () => {
  it("群演 3×3：九个灰人偶，列沿画面左右、行往纵深，大致面向相机；编号接着已有的往下数", () => {
    const cam: [number, number, number] = [0, 1.6, 0];
    const center: [number, number, number] = [0, 0, -6];
    const crowd = crowdGrid(center, cam, [actor("旧群演", 0, 0, { kind: "extra" })]);
    expect(crowd).toHaveLength(9);
    expect(crowd.every((a) => a.kind === "extra" && a.color === CROWD_COLOR && !a.assetId)).toBe(true);
    expect(crowd[0].label).toBe("群演 2");
    expect(new Set(crowd.map((a) => a.id)).size).toBe(9);
    // 相机看向 -Z：第一行（近处）比最后一行离相机近，列在 X 上铺开
    const z = crowd.map((a) => a.position[2]);
    expect(Math.max(...z.slice(0, 3))).toBeGreaterThan(Math.min(...z.slice(6)));
    const xs = crowd.slice(0, 3).map((a) => a.position[0]);
    expect(xs[0]).toBeLessThan(xs[2]);
    // 正对相机的朝向 ±20° 以内
    for (const a of crowd) expect(Math.abs(a.yawDeg - facingCameraYaw(a.position, cam))).toBeLessThanOrEqual(20);
    // 相机转到侧面：还是列沿画面、行往纵深（不是世界坐标轴）
    const side = crowdGrid([6, 0, 0], cam, []);
    const sx = side.map((a) => a.position[0]);
    expect(Math.min(...sx.slice(0, 3))).toBeLessThan(Math.max(...sx.slice(6)));
  });

  it("群演、道具不算人：不查同色、不要求绑角色、不写站位", () => {
    const cam = { position: [0, 1.6, 0] as [number, number, number], yawDeg: 0, pitchDeg: -10, fovDeg: 60 };
    const list = [
      actor("陆川", 0, -4, { assetId: "char_jc", color: "#FF00FF" }),
      actor("群演 1", -1, -5, { kind: "extra", color: CROWD_COLOR }),
      actor("群演 2", 1, -5, { kind: "extra", color: CROWD_COLOR }),
      actor("机械臂", 0.5, -3, { kind: "prop", color: "#FFB000" }),
    ];
    expect(actorProblems(list, true)).toEqual([]);
    expect(blockingFor(list, cam, 16 / 9).map((b) => b.actor)).toEqual(["陆川"]);
    expect(list.map(isPerson)).toEqual([true, false, false, false]);
    const meta = buildFrameMeta({ sourceUrl: null, context: null, aspect: "16:9", camera: cam, actors: [{ ...list[3], imagePath: "/a/arm.png" }, list[1]] });
    expect(meta.actors.map((a) => [a.role, a.image_path])).toEqual([["prop", "/a/arm.png"], ["extra", null]]);
  });

  it("存盘读回：类别和立牌图留着，认不出的类别当人", () => {
    const got = readActors([
      { ...actor("a", 0, 0), kind: "prop", imagePath: "/a/arm.png" },
      { ...actor("b", 0, 0), kind: "extra" },
      { ...actor("c", 0, 0), kind: "robot", imagePath: 3 },
    ]);
    expect(got.map((a) => [a.kind, a.imagePath])).toEqual([["prop", "/a/arm.png"], ["extra", undefined], [undefined, undefined]]);
  });
});

describe("新机位叫什么", () => {
  it("第一个没被占的「机位 N」：删过一个再新建不撞名", () => {
    expect(nextCameraName([])).toBe("机位 1");
    expect(nextCameraName([{ name: "机位 1" }, { name: "机位 3" }])).toBe("机位 2");
    expect(nextCameraName([{ name: "机位 1" }, { name: "第 3 镜" }, { name: "机位 2" }])).toBe("机位 3");
  });
});
