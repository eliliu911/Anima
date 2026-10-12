import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const DIR = fileURLToPath(new URL(".", import.meta.url));
function tsx(dir = DIR): string[] {
  return readdirSync(dir).flatMap((n) => {
    const p = join(dir, n);
    if (statSync(p).isDirectory()) return tsx(p);
    return n.endsWith(".tsx") ? [p] : [];
  });
}

describe("下拉菜单", () => {
  /**
   * **不许用 `fixed inset-0` 的透明层接「点外面关闭」。**
   *
   * 那个遮罩盖在触发它的卡片上，卡片立刻失去 `:hover`，抬升动画退回，
   * 元素移动，鼠标又落回卡片 —— hover 反复横跳，表现是卡片疯狂闪烁；
   * 而且 hover 被吃掉之后鼠标移出也不触发 leave，菜单关不掉。
   *
   * 正确做法是 `useDismissable`：在 document 上听 pointerdown，不插元素。
   */
  it("没有人再用透明遮罩接关闭", () => {
    const bad: string[] = [];
    for (const f of tsx()) {
      const src = readFileSync(f, "utf8");
      if (/className="fixed inset-0 z-\d+"\s*onClick=/.test(src)) {
        bad.push(f.replace(DIR, ""));
      }
    }
    expect(bad, `还在用透明遮罩：\n${bad.join("\n")}`).toEqual([]);
  });

  it("菜单用 studio-menu，不用 studio-modal 的圆角", () => {
    // 弹窗 20px 圆角是为了压住一整块内容；菜单套上去就变成一颗药丸。
    const src = readFileSync(join(DIR, "workspace/WorkspacePage.tsx"), "utf8");
    expect(src).toContain("studio-menu absolute");
    expect(src).not.toMatch(/studio-modal absolute/);
  });

  it("菜单项默认带图标", () => {
    const src = readFileSync(join(DIR, "workspace/WorkspacePage.tsx"), "utf8");
    const items = [...src.matchAll(/<MenuItem\b[\s\S]*?\/>/g)].map((m) => m[0]);
    expect(items.length).toBeGreaterThan(0);
    const noIcon = items.filter((it) => !it.includes("icon="));
    expect(noIcon, `没图标的菜单项：\n${noIcon.join("\n---\n")}`).toEqual([]);
  });

  it("菜单开着时卡片冻住，不再有抬升动画", () => {
    const css = readFileSync(fileURLToPath(new URL("../../studio-theme.css", import.meta.url)), "utf8");
    expect(css).toContain('studio-tile[data-menu-open="true"]');
    expect(css).toMatch(/data-menu-open="true"\][\s\S]{0,200}transform:\s*none/);
  });
});
