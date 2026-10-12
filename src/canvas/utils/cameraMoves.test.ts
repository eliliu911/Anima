import { describe, expect, it } from "vitest";
import { CAMERA_MOVES, cameraMovesIn, cameraShortName, cameraTag, compileCameraMoves } from "./cameraMoves";
import { existsSync } from "node:fs";

const by = (id: string) => CAMERA_MOVES.find((m) => m.id === id)!;

describe("运镜标签", () => {
  it("21 个，标签是「运镜·短名」，短名不带英文", () => {
    expect(CAMERA_MOVES).toHaveLength(21);
    expect(cameraTag(by("follow"))).toBe("运镜·跟拍");
    for (const m of CAMERA_MOVES) expect(cameraShortName(m), m.id).not.toMatch(/\(/);
    expect(new Set(CAMERA_MOVES.map(cameraTag)).size).toBe(21);
  });

  // 用户 2026-09-25：「可以叠几个，用 @ 的形式在提示词里标记顺序」
  it("顺序 = 标签在提示词里的前后", () => {
    const p = "走廊里，@运镜·推镜头 接着 @运镜·跟拍";
    expect(cameraMovesIn(p).map((m) => m.id)).toEqual(["push_in", "follow"]);
  });

  it("发出去前原位换成描述：中文提示词用中文，否则英文", () => {
    expect(compileCameraMoves("女孩，@运镜·跟拍 。", true)).toBe(`女孩，${by("follow").zh} 。`);
    expect(compileCameraMoves("a girl, @运镜·推镜头", false)).toBe(`a girl, ${by("push_in").en}`);
  });

  it("长名先换：「环绕抬升」不会被「环绕」吃掉一半", () => {
    const out = compileCameraMoves(`@${cameraTag(by("orbit_up"))}`, true);
    expect(out).toBe(by("orbit_up").zh);
  });

  it("每个运镜都有示例短片（public/camera-previews/<id>.mp4）", () => {
    const missing = CAMERA_MOVES.filter((m) => !existsSync(new URL(`../../../public/camera-previews/${m.id}.mp4`, import.meta.url)));
    expect(missing.map((m) => m.id)).toEqual([]);
  });
});
