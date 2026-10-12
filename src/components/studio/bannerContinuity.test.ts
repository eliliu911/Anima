import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

/**
 * **页头不许自己刷一层不透明底色。**
 *
 * 环境渐变是 `position: fixed; inset: 0` —— 它本来就铺满整个窗口。
 * 页头刷上 `studio-chrome`（78% 不透明 + 模糊）之后，渐变被盖住，
 * 看起来像是「从 banner 下面才开始」，页头和内容之间凭空多出一道色差。
 *
 * 侧边栏是另一回事：它是一根竖条，本来就该和主区分开。
 */
const STUDIO = fileURLToPath(new URL(".", import.meta.url));
const CSS = readFileSync(
  fileURLToPath(new URL("../../studio-theme.css", import.meta.url)),
  "utf8",
);

function tsx(dir = STUDIO): string[] {
  return readdirSync(dir).flatMap((n) => {
    const p = join(dir, n);
    if (statSync(p).isDirectory()) return tsx(p);
    return n.endsWith(".tsx") ? [p] : [];
  });
}

/** 页头 = 那一行同款布局：`shrink-0 items-start justify-between px-7 py-5`。 */
const HEADER_SHAPE = "shrink-0 items-start justify-between px-7 py-5";

describe("页头和内容的底色是连着的", () => {
  const headers = tsx()
    .map((f) => [f.slice(STUDIO.length), readFileSync(f, "utf8")] as const)
    .filter(([, src]) => src.includes(HEADER_SHAPE));

  it("确实扫到了页头 —— 不是扫了个空", () => {
    expect(headers.length).toBeGreaterThanOrEqual(4);
  });

  it("每个页头都用 studio-banner，没有一个用 studio-chrome", () => {
    const bad = headers
      .filter(([, src]) => src.includes(`studio-chrome flex ${HEADER_SHAPE}`))
      .map(([n]) => n);
    expect(bad, `这些页头把渐变盖住了：${bad.join(", ")}`).toEqual([]);
    for (const [name, src] of headers) {
      expect(src, `${name} 没用 studio-banner`).toContain(
        `studio-banner flex ${HEADER_SHAPE}`,
      );
    }
  });

  it("studio-banner 是透明的 —— 不透明就等于没改", () => {
    const at = CSS.indexOf(".anima-studio .studio-banner");
    expect(at).toBeGreaterThan(0);
    const block = CSS.slice(at, CSS.indexOf("}", at));
    expect(block).toContain("background: transparent");
  });

  /** 侧边栏是竖条，本来就该和主区分开 —— 它保留 chrome 是对的。 */
  it("侧边栏仍然保留 studio-chrome", () => {
    const rails = tsx().filter((f) =>
      readFileSync(f, "utf8").includes("studio-chrome studio-titlebar-safe"),
    );
    expect(rails.length).toBeGreaterThanOrEqual(2);
  });
});
