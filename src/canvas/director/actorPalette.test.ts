import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { ACTOR_PALETTE, nextFreeColor } from "./actorPalette";

const rust = readFileSync(fileURLToPath(new URL("../../../src-tauri/src/actor_colors.rs", import.meta.url)), "utf8");

describe("人物调色盘", () => {
  /** 两份各写各的话，改了一边另一边不知道 —— 前端选的颜色后端不认 */
  it("和 Rust 那份逐项一致", () => {
    const block = rust.slice(rust.indexOf("pub const PALETTE"), rust.indexOf("];", rust.indexOf("pub const PALETTE")));
    const pairs = [...block.matchAll(/\("(#[0-9A-F]{6})",\s*"([A-Z ]+)"\)/g)].map((m) => ({ hex: m[1], name: m[2] }));
    expect(pairs.length).toBe(12);
    expect(ACTOR_PALETTE).toEqual(pairs);
  });

  it("下一个颜色跳过已占用的，大小写不敏感", () => {
    expect(nextFreeColor([])).toBe("#FF00FF");
    expect(nextFreeColor(["#ff00ff"])).toBe("#00FFFF");
  });
});
