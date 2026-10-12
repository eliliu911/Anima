import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

/**
 * **工作室的弹窗点空白不关。**
 *
 * 这些弹窗里装的是勾选、填了一半的表单、或者跑了几分钟的结果 ——
 * 「从设定文档建资产」提取完弹出实体列表，手一滑点到遮罩上，
 * 窗口关掉、结果全丢，只能重新提取一遍（几分钟一次）。
 * 用户原话：「这类窗体不允许空白区域关闭」。
 *
 * 关只走弹窗自己的「取消 / 关闭」按钮。
 */
const STUDIO = fileURLToPath(new URL("./", import.meta.url));

function walk(dir: string): string[] {
  return readdirSync(dir).flatMap((n) => {
    const p = join(dir, n);
    if (statSync(p).isDirectory()) return walk(p);
    return n.endsWith(".tsx") && !n.includes(".test.") ? [p] : [];
  });
}

/** 取遮罩那个元素的开标签（到第一个 `>`），看它身上有没有 onClick。 */
function scrimTags(src: string): string[] {
  const tags: string[] = [];
  let i = src.indexOf('className="studio-scrim ');
  while (i >= 0) {
    const start = src.lastIndexOf("<", i);
    const end = src.indexOf(">", i);
    tags.push(src.slice(start, end + 1));
    i = src.indexOf('className="studio-scrim ', end);
  }
  return tags;
}

describe("弹窗遮罩", () => {
  const files = walk(STUDIO).map((f) => ({ f: f.slice(STUDIO.length), src: readFileSync(f, "utf8") }));
  const withScrim = files.filter(({ src }) => scrimTags(src).length > 0);

  it("确实扫到了弹窗 —— 扫不到的话下面那条永远是绿的", () => {
    expect(withScrim.length).toBeGreaterThanOrEqual(10);
  });

  it("没有哪个遮罩点一下就关窗", () => {
    const offenders = withScrim
      .filter(({ src }) => scrimTags(src).some((t) => /onClick|onMouseDown|onPointerDown/.test(t)))
      .map(({ f }) => f);
    expect(offenders, `这些弹窗点空白会关：${offenders.join(", ")}`).toEqual([]);
  });
});

/**
 * **弹窗必须限高。** 提取出几十个实体时，没限高的弹窗往下长，
 * 底部被主窗体截掉 ——「应用」按钮看不见也点不到。
 * 限高后要么中间一段自己滚（header/footer 固定），要么整个弹窗滚。
 */
describe("弹窗高度", () => {
  const files = walk(STUDIO).map((f) => ({ f: f.slice(STUDIO.length), src: readFileSync(f, "utf8") }));
  const modals = files.flatMap(({ f, src }) =>
    [...src.matchAll(/className="(studio-modal [^"]*)"/g)].map((m) => ({ f, cls: m[1] })),
  );

  it("确实扫到了弹窗", () => {
    expect(modals.length).toBeGreaterThanOrEqual(10);
  });

  it("每个弹窗都有高度上限（max-h-[…] 或 h-[…]）", () => {
    const offenders = modals.filter(({ cls }) => !/(^|\s)(max-)?h-\[/.test(cls)).map(({ f }) => f);
    expect(offenders, `这些弹窗会无限拉长：${offenders.join(", ")}`).toEqual([]);
  });

  it("限了高就得能滚：要么自己 overflow-y-auto，要么是 flex-col 让中间段滚", () => {
    const offenders = modals
      .filter(({ cls }) => /(^|\s)max-h-\[/.test(cls))
      .filter(({ cls }) => !/overflow-y-auto/.test(cls) && !/flex-col/.test(cls))
      .map(({ f }) => f);
    expect(offenders).toEqual([]);
  });
});
