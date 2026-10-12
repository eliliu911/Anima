import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { SHORTCUTS, keyCode } from "./DirectorWorldEditor";

const src = readFileSync(fileURLToPath(new URL("./DirectorWorldEditor.tsx", import.meta.url)), "utf8");
const engineSrc = readFileSync(fileURLToPath(new URL("./engine.ts", import.meta.url)), "utf8");

/** 面板上写的单个按键 → 应该在 switch 里出现的 code */
const TOKEN_CODES: Record<string, string> = {
  "1": "Digit1", C: "KeyC", F: "KeyF", G: "KeyG", I: "KeyI", J: "KeyJ", K: "KeyK", L: "KeyL",
  U: "KeyU", O: "KeyO", R: "KeyR", T: "KeyT", B: "KeyB", N: "KeyN", V: "KeyV",
  "-": "Minus", "=": "Equal", "[": "BracketLeft", "]": "BracketRight",
  "↑": "ArrowUp", "↓": "ArrowDown", "←": "ArrowLeft", "→": "ArrowRight",
  Tab: "Tab", Esc: "Escape", Delete: "Delete", "⌘Z": "KeyZ", "⇧⌘Z": "KeyZ",
};

describe("导演世界快捷键", () => {
  /** 面板列了、按了没反应 —— 画布快捷键面板出过一次，这里同一个规矩 */
  it("面板上每个按键都在按键处理里接好了", () => {
    const missing: string[] = [];
    for (const [keys] of SHORTCUTS) {
      for (const token of keys.split(/[\s/·]+/).filter(Boolean)) {
        const code = TOKEN_CODES[token];
        if (!code) continue; // 鼠标手势、WASD 等由引擎处理，另外核对
        const wired = code === "KeyZ" ? src.includes('c === "KeyZ"') : src.includes(`case "${code}":`);
        if (!wired) missing.push(`${token} → ${code}`);
      }
    }
    expect(missing).toEqual([]);
  });

  it("飞行键由引擎接", () => {
    for (const c of ["KeyW", "KeyA", "KeyS", "KeyD", "KeyQ", "KeyE"]) expect(engineSrc).toContain(`"${c}"`);
  });

  /** 远程桌面、部分输入法、自动化工具里 event.code 是空的 —— 不兜底的话所有快捷键都没反应 */
  it("event.code 为空时按 key 推回物理键位", () => {
    expect(keyCode({ code: "KeyT", key: "Ï" })).toBe("KeyT");
    expect(keyCode({ code: "", key: "t" })).toBe("KeyT");
    expect(keyCode({ code: "", key: "T" })).toBe("KeyT");
    expect(keyCode({ code: "", key: "]" })).toBe("BracketRight");
    expect(keyCode({ code: "", key: "=" })).toBe("Equal");
    expect(keyCode({ code: "", key: "1" })).toBe("Digit1");
    expect(keyCode({ code: "", key: "ArrowLeft" })).toBe("ArrowLeft");
    expect(keyCode({ code: "", key: "Escape" })).toBe("Escape");
  });
});

describe("全景贴图", () => {
  /** 节点里的全景是本机路径，直接喂给 TextureLoader 必然失败（`[object Event]` 那次） */
  it("先过 mediaDataUrl 再给引擎", () => {
    expect(src).toContain("mediaDataUrl(panoSource)");
  });
});

/**
 * 界面升级 U1（左图标栏 + 左面板 + 右属性栏）只搬家：原来一条长栏里的每一个功能都要还在。
 * 方案：docs/plans/2026-09-27-导演台界面升级.md
 */
describe("导演台布局：功能一个不少", () => {
  it("左侧五个菜单，原来每个功能都找得到", () => {
    for (const t of ['id: "stage"', 'id: "people"', 'id: "cameras"', 'id: "motion"', 'id: "output"']) expect(src).toContain(t);
    for (const feature of [
      "分集（选了就是主线模式）", "世界来源", "画幅",
      "把人物放到准星", "场上 · 人物",
      // U3.5 加的
      "把道具放到准星", "在准星铺一片群演（3×3）",
      // 站位跨镜头沿用
      "从同一场景的别的镜头复制站位", "这一镜分镜里没有", "这一镜还要", "清空重摆", "确认替换场上",
      "新建机位", "重置镜头", "对准选中人物", "镜头锁在拍摄点",
      "加入运镜关键帧", "输出运镜 RGB、深度和首尾帧", "将当前机位与主运镜绑定到当前分镜",
      "输出人偶稿", "写进分镜",
      "重置世界校准", "眼高", "删除选中", "人物姿势",
    ]) expect(src, feature).toContain(feature);
  });
});

/** 界面升级 U2：导演视角 / 机位视角 */
describe("导演视角 / 机位视角", () => {
  it("输出、写进分镜按当前机位，导演视角下临时切过去截", () => {
    expect(src).toContain("withShotView((e) => ({ combined: e.capture(\"combined\", frame), envOnly: e.capture(\"env_only\", frame) }))");
    expect(src).toContain("const rows = blockingFor(actors, camera, aspectRatio(aspect));");
    expect(src).toContain("const next = withDirectorCamera(episode, context.shotId, active, cameraPaths[0]);");
  });

  it("两个页签共用外部观察相机；当前机位只在截图时临时使用", () => {
    expect(src).toContain('engine.setNavMode("orbit")');
    expect(src).toContain('setCameraLocked(false)');
    expect(src).toContain('return engine.withCamera(camera, () => fn(engine));');
    expect(engineSrc).toContain('if (navMode === "orbit") {\n      // 整体缩放');
  });

  it("老存档没有机位时收成「机位 1」", () => {
    expect(src).toContain('[{ id: "camera-1", name: "机位 1", camera: initial.camera }]');
  });

  it("右上角方向器可点六个轴向，并且只重置导演视角", () => {
    expect(src).toContain('aria-label="三维方向器"');
    expect(src).toContain("snapDirectorView");
    expect(src).toContain("resetDirectorView");
    expect(engineSrc).toContain('if (navMode !== "orbit") return;');
    expect(engineSrc).toContain("orbit.dist = Math.max(8, sphereRadius * 1.7)");
  });
});

/** 用户 2026-09-28：「新建机位 bug：1. 按钮样式，换行。2. 没有在当前视角创建，多次创建的机位全重叠在一起」 */
describe("新建机位", () => {
  it("按钮不折行（固定高度，折成两行会顶出边框）", () => {
    const btn = src.slice(src.indexOf("function OutlineButton"));
    expect(btn.slice(0, 900)).toContain("whitespace-nowrap");
    expect(src).toContain(">新建机位</OutlineButton>");
    expect(src).not.toContain("新建机位（当前视角）</OutlineButton>");
  });
  it("导演视角 + 全景锁在拍摄点：放在拍摄点、朝导演相机的方向（不是绕转中心）；一模一样的不再叠一个；3D 片场建完往后退一步", () => {
    const fn = src.slice(src.indexOf("const saveNamedCamera = () => {"), src.indexOf("const addPathKeyframe"));
    expect(fn).toContain("const atCapture = inDirector && fixedPoint;");
    expect(fn).toMatch(/position: \[0, eyeHeight, 0\], yawDeg: seen\.yawDeg/);
    expect(fn).toContain("if (inDirector && same) {");
    expect(fn).toContain("engine?.pullBack(2.5)");
    expect(fn).toContain("nextCameraName(namedCameras)");
  });
});

/** 导演世界写进分镜 / 绑定机位只存这一镜：整集保存会拿打开时那份把后台细化 / 合成写回的盖掉（2026-09-28） */
describe("导演世界写回分镜", () => {
  it("写进分镜、绑定机位都走 cinema_save_shot，不再整集保存", () => {
    expect(src).not.toContain("cinemaSaveEpisode(");
    expect(src).toContain("await cinemaSaveShot(context.projectId, context.episodeId, shot, base);");
    expect((src.match(/await saveShotOf\(next, context\.shotId\)/g) ?? []).length).toBe(2);
  });
});
