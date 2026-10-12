import { describe, it, expect } from "vitest";
import { formName, lockedBy } from "./PersonaLock";
import type { PersonaView } from "../api/voice";

function view(isDefault: boolean, over: Partial<PersonaView> = {}): PersonaView {
  const origin = isDefault ? ("default" as const) : ("character" as const);
  return {
    resolved: {
      engine: { value: isDefault ? "edge" : "gpt_sovits", origin },
      voice: { value: isDefault ? "zh-CN-XiaoxiaoNeural" : "yaemiko/d", origin },
      avatar: { value: isDefault ? null : "Yae miko.glb", origin },
      skills: { value: [], origin: "default" },
      is_default_form: isDefault,
    },
    active: isDefault ? null : "yae",
    characters: [{ id: "yae", name: "八重神子", marker: "🦊", description: "狐狸" } as never],
    engines: [],
    avatars: [],
    skills: [],
    ...over,
  } as PersonaView;
}

describe("形态锁定", () => {
  /// 默认形态是兜底形态，面容和语音声学在这一档下必须能改
  it("默认形态下什么都不锁", () => {
    const v = view(true);
    for (const f of ["engine", "voice", "avatar", "skills"] as const) {
      expect(lockedBy(v, f), f).toBe(false);
    }
  });

  /// **这是这次重构的核心约束。** 选了角色之后语音和面容归角色管，
  /// 允许在全局面板上改就会有两个写入口，谁后写谁赢 ——
  /// 之前那一串查不明白的"没声音"全是这么来的
  it("选中角色后，角色指定的那几项锁死", () => {
    const v = view(false);
    expect(lockedBy(v, "engine")).toBe(true);
    expect(lockedBy(v, "voice")).toBe(true);
    expect(lockedBy(v, "avatar")).toBe(true);
  });

  /// 角色没绑的那一项落回默认形态，那一项**仍然可改** ——
  /// 一刀切全锁的话，用户连一个角色没管的设置都动不了
  it("角色没绑的那项不锁", () => {
    const v = view(false);
    v.resolved.skills = { value: ["general"], origin: "default" };
    expect(lockedBy(v, "skills")).toBe(false);
    expect(lockedBy(v, "engine")).toBe(true);
  });

  /// 还没读到形态时不要抢先锁 —— 界面一闪一变很难受，
  /// 而且读失败时用户会被永久挡在外面
  it("还没读到形态时不锁", () => {
    for (const f of ["engine", "voice", "avatar", "skills"] as const) {
      expect(lockedBy(null, f)).toBe(false);
    }
  });

  it("形态名字：默认形态 / 角色显示名", () => {
    expect(formName(view(true))).toBe("默认形态");
    expect(formName(view(false))).toBe("八重神子");
    expect(formName(null)).toBe("…");
  });

  /// 角色被删了但配置里还留着 id 时，退回显示 id 而不是空白 ——
  /// 空白会让人以为界面坏了
  it("找不到角色时退回显示 id", () => {
    const v = view(false, { characters: [] });
    expect(formName(v)).toBe("yae");
  });
});
