import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const DIALOG = readFileSync(fileURLToPath(new URL("./ModelSettingsDialog.tsx", import.meta.url)), "utf8");

/**
 * 线路开关：A / B 两条都能选，默认 B，选了真的写进设置。
 * 任何一条断了都不报错 —— 界面照样显示，只是出片永远走默认那条。
 */
describe("生成设置里的提示词线路", () => {
  it("两条线路都列出来了", () => {
    expect(DIALOG).toContain('{ id: "a", label: "线路 A');
    expect(DIALOG).toContain('{ id: "b", label: "线路 B');
  });

  it("选中会写进设置，而不是只改界面", () => {
    const at = DIALOG.indexOf("ROUTES.map(");
    expect(at, "没有渲染线路").toBeGreaterThan(0);
    expect(DIALOG.slice(at, at + 400)).toContain('set("prompt_route", r.id)');
  });

  it("没设置过的项目默认 B，和后端一致", () => {
    expect(DIALOG).toContain('prompt_route: "b"');
    expect(DIALOG).toContain('(draft.prompt_route ?? "b")');
  });
});
