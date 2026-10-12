// 本机文件直接复制到「下载」再在访达里选中；不再先走 base64 + <a download>（桌面 WebView 里不一定生效，还不报错）。
import { describe, it, expect, vi, beforeEach } from "vitest";

const invoke = vi.fn();
const reveal = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => invoke(...a), convertFileSrc: (p: string) => `asset://${p}` }));
vi.mock("@tauri-apps/plugin-opener", () => ({ revealItemInDir: (...a: unknown[]) => reveal(...a) }));

import { downloadMedia } from "./downloadMedia";

describe("downloadMedia：本机文件", () => {
  beforeEach(() => {
    invoke.mockReset();
    reveal.mockReset();
  });

  it("复制到下载文件夹，再在访达里选中复制出来的那个文件", async () => {
    invoke.mockResolvedValue("/Users/me/Downloads/canvas_upscale_1.mp4");
    await downloadMedia("/Users/me/Library/Application Support/x/canvas_upscale_1.mp4");
    expect(invoke).toHaveBeenCalledWith("media_save_to_downloads", { path: "/Users/me/Library/Application Support/x/canvas_upscale_1.mp4" });
    expect(reveal).toHaveBeenCalledWith("/Users/me/Downloads/canvas_upscale_1.mp4");
    expect(invoke).not.toHaveBeenCalledWith("assets_read_media", expect.anything());
  });

  it("复制失败时退回老路（读成 base64 再下载）", async () => {
    invoke.mockImplementation((cmd: string) => (cmd === "media_save_to_downloads" ? Promise.reject("不是媒体文件") : Promise.resolve("data:video/mp4;base64,AAAA")));
    const click = vi.fn();
    const doc = globalThis.document;
    globalThis.document = {
      createElement: vi.fn().mockReturnValue({ href: "", download: "", click }),
      body: { appendChild: vi.fn(), removeChild: vi.fn() },
    } as unknown as Document;
    globalThis.URL.createObjectURL = vi.fn(() => "blob:x");
    globalThis.URL.revokeObjectURL = vi.fn();
    try {
      await downloadMedia("/Users/me/a.mp4");
    } finally {
      globalThis.document = doc;
    }
    expect(invoke).toHaveBeenCalledWith("assets_read_media", { path: "/Users/me/a.mp4" });
    expect(click).toHaveBeenCalled();
  });
});
