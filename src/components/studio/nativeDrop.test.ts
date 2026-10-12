import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { isAcceptedImagePath, NATIVE_DROP_ATTR, toLogical } from "./nativeDrop";

const read = (rel: string) => readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");

/**
 * 从访达拖图进来走 Tauri 的原生拖放（HTML5 `drop` 收不到文件）。
 * 用户 2026-09-26：角色工作台「上传图片（点选或拖进来）」拖了没反应。
 */
describe("原生拖放", () => {
  it("落点从物理像素换成逻辑像素 —— Retina 上差一倍", () => {
    expect(toLogical({ x: 800, y: 400 }, 2)).toEqual({ x: 400, y: 200 });
    expect(toLogical({ x: 10, y: 20 }, 0)).toEqual({ x: 10, y: 20 });
  });

  it("只收出片链路认的三种图", () => {
    expect(isAcceptedImagePath("/a/陆川.PNG")).toBe(true);
    expect(isAcceptedImagePath("/a/b.jpeg")).toBe(true);
    expect(isAcceptedImagePath("/a/b.webp")).toBe(true);
    expect(isAcceptedImagePath("/a/b.heic")).toBe(false);
    expect(isAcceptedImagePath("/a/b.svg")).toBe(false);
  });

  /**
   * **接线要在。** 上传框和首帧槽位都得挂上原生拖放、标上拖放区；
   * 聊天栏得给拖放区让路 —— 不让的话一张图会同时进资产和聊天附件。
   */
  it("上传框、首帧槽位接了原生拖放，聊天栏会让路", () => {
    for (const f of ["./assets/UploadImageButton.tsx", "./MediaInput.tsx"]) {
      const src = read(f);
      expect(src, f).toContain("useNativeFileDrop(");
      expect(src, f).toContain("[NATIVE_DROP_ATTR]");
    }
    const bar = read("../CommandBar.tsx");
    expect(bar).toContain("if (onZone(event.payload.position)) return;");
    expect(NATIVE_DROP_ATTR).toBe("data-native-drop");
  });
});
