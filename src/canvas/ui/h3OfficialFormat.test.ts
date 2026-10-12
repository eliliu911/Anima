import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const read = (rel: string) => readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");

/**
 * 画布的 H3「官方格式」开关（2026-10-06，用户：「只做格式部分就行」）：
 * 节点上存 `h3OfficialFormat`，出片时作为 `h3_official_format` 交给后端整理（Rust `h3_spec::canvas_official`）。
 * 默认关；只在 H3 时显示。
 */
describe("画布 H3 官方格式开关", () => {
  it("只在 H3 时显示，两档：整理 / 原样发送", () => {
    const pop = read("./CanvasVideoSettingsPopover.tsx");
    expect(pop).toContain('{engine === "h3" && onH3OfficialFormatChange && (');
    expect(pop).toContain('{on ? "整理" : "原样发送"}');
  });

  it("节点 → 面板 → 弹层接上，默认关", () => {
    expect(read("../nodes/VideoGenNode.tsx")).toContain("h3OfficialFormat={data.h3OfficialFormat ?? false}");
    const panel = read("../nodes/panels/VideoOperationPanel.tsx");
    expect(panel).toContain("h3OfficialFormat = false,");
    expect(panel).toContain("onH3OfficialFormatChange={onH3OfficialFormatChange}");
  });

  it("出片时带给后端", () => {
    expect(read("../engine/dagExecutor.ts")).toContain("h3_official_format: Boolean(data.h3OfficialFormat),");
  });
});
