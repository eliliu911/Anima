import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { mediaSrc, pathFromMediaUrl } from "./MediaThumb";

const SRC = readFileSync(
  fileURLToPath(new URL("./MediaThumb.tsx", import.meta.url)),
  "utf8",
);

describe("媒体地址", () => {
  it("路径 → 协议 URL 能来回转", () => {
    const p = "/Users/x/Library/Application Support/com.anima.desktop/a b.png";
    expect(pathFromMediaUrl(mediaSrc(p))).toBe(p);
  });

  /**
   * **老分集里的 `image_url` 是 base64 data URL。**
   *
   * 不单独认出来的话，它会被当成路径包进
   * `anima-media://?p=data:image/png;base64,...`，协议按路径去读，必然失败 ——
   * 表现是老镜头一律显示不出来，而且看起来像「文件没了」。
   */
  it("data: 不会被当成路径", () => {
    const d = "data:image/png;base64,iVBORw0KGgo=";
    expect(pathFromMediaUrl(d)).toBe(d);
    expect(SRC).toContain("function isInlineSrc");
    const at = SRC.indexOf("function isInlineSrc");
    const body = SRC.slice(at, SRC.indexOf("}", at));
    expect(body).toContain('"data:"');
    expect(body).toContain('"blob:"');
  });

  /** 认出来之后要**直接用**，而不是照旧走协议。 */
  it("inline 的地址直接喂给 img，不走协议", () => {
    expect(SRC).toContain("if (inline) {");
    const at = SRC.indexOf("if (inline) {");
    expect(SRC.slice(at, at + 80)).toContain("setSrc(path)");
  });

  /** 协议加载失败的兜底不该对 inline 地址生效 —— 它压根没走协议。 */
  it("inline 地址不触发协议兜底", () => {
    expect(SRC).toContain("if (!path || inline) return;");
  });
});

describe("缓存击穿", () => {
  /**
   * **光有 ETag 不够。**
   *
   * 分镜首帧固定写 `shots/<镜头 id>.png`。重出一次内容换了、路径没变 ——
   * `<img src>` 那个字符串一模一样，React 不会重设 src，浏览器也就不会
   * 发请求，HTTP 那层的 ETag 根本轮不到。表现是要切一次页面才看到新图。
   */
  it("带版本号时 URL 会变", () => {
    const p = "/x/shot_1.png";
    expect(mediaSrc(p, 1)).not.toBe(mediaSrc(p, 2));
    expect(mediaSrc(p)).not.toBe(mediaSrc(p, 1));
  });

  it("版本号不影响路径解析 —— 协议那边只读 p", () => {
    const p = "/x/shot 1.png";
    expect(pathFromMediaUrl(mediaSrc(p, 12345))).toBe(p);
  });

  /** 0 / undefined 当作"没有版本"，不要拼一个 `v=0` 上去。 */
  it("没有版本号时不加参数", () => {
    expect(mediaSrc("/a.png")).not.toContain("v=");
    expect(mediaSrc("/a.png", 0)).not.toContain("v=");
  });

  /** 版本变了要真的重新解析 —— 不进依赖的话 src 不会更新。 */
  it("version 在 effect 依赖里", () => {
    expect(SRC).toContain("[path, inline, version]");
  });

  /** 路径会被覆写的地方必须传版本，否则界面一直是旧图。 */
  it("队列传了版本", () => {
    const queue = readFileSync(
      fileURLToPath(new URL("./jobs/QueuePanel.tsx", import.meta.url)),
      "utf8",
    );
    expect(queue).toContain("version={job.finished_at}");
  });
});
