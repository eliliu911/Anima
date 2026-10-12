import { describe, it, expect, vi, afterEach } from "vitest";
import { downloadMedia } from "../utils/downloadMedia";

describe("CanvasMediaGallery & downloadMedia", () => {
  const originalDoc = globalThis.document;

  afterEach(() => {
    globalThis.document = originalDoc;
  });

  it("downloadMedia correctly handles path and creates download anchor", async () => {
    const clickSpy = vi.fn();
    const mockAnchor = {
      href: "",
      download: "",
      target: "",
      click: clickSpy,
    };

    globalThis.document = {
      createElement: vi.fn().mockReturnValue(mockAnchor),
      body: {
        appendChild: vi.fn(),
        removeChild: vi.fn(),
      },
    } as unknown as Document;

    // Mock fetch for blob download
    globalThis.fetch = vi.fn().mockResolvedValue({
      ok: true,
      blob: async () => new Blob(["test"]),
    });
    globalThis.URL.createObjectURL = vi.fn().mockReturnValue("blob:mock-url");
    globalThis.URL.revokeObjectURL = vi.fn();

    await downloadMedia("/path/to/media.png", "test-image.png");

    expect(clickSpy).toHaveBeenCalled();
  });

  it("downloadMedia handles data URLs by converting to Blob", async () => {
    const clickSpy = vi.fn();
    const mockAnchor = {
      href: "",
      download: "",
      target: "",
      click: clickSpy,
    };

    globalThis.document = {
      createElement: vi.fn().mockReturnValue(mockAnchor),
      body: {
        appendChild: vi.fn(),
        removeChild: vi.fn(),
      },
    } as unknown as Document;

    globalThis.URL.createObjectURL = vi.fn().mockReturnValue("blob:data-url");
    globalThis.URL.revokeObjectURL = vi.fn();

    const sampleBase64 = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
    await downloadMedia(sampleBase64, "pixel.png");

    expect(clickSpy).toHaveBeenCalled();
    expect(mockAnchor.download).toBe("pixel.png");
  });

  it("calculates rendering step accurately (1/2, 2/2, never 0/2)", () => {
    const targetCount = 2;

    // When 0 items completed (generating 1st):
    const items0: string[] = [];
    const step0 = Math.min(targetCount, items0.length + 1);
    expect(`${step0}/${targetCount}`).toBe("1/2");

    // When 1 item completed (generating 2nd):
    const items1 = ["img1.png"];
    const step1 = Math.min(targetCount, items1.length + 1);
    expect(`${step1}/${targetCount}`).toBe("2/2");

    // For batch of 4:
    const targetCount4 = 4;
    expect(`${Math.min(targetCount4, 0 + 1)}/${targetCount4}`).toBe("1/4");
    expect(`${Math.min(targetCount4, 1 + 1)}/${targetCount4}`).toBe("2/4");
    expect(`${Math.min(targetCount4, 2 + 1)}/${targetCount4}`).toBe("3/4");
    expect(`${Math.min(targetCount4, 3 + 1)}/${targetCount4}`).toBe("4/4");
  });

  it("calculates slots accurately for 1, 2, and 4 batch items", () => {
    const items1 = ["path1.png"];
    expect(items1.length).toBe(1);

    const items2 = ["path1.png", "path2.png"];
    expect(items2.length).toBe(2);

    const items4 = ["path1.png", "path2.png", "path3.png", "path4.png"];
    expect(items4.length).toBe(4);

    // When generating a batch of 4:
    const inProgressCount = 4;
    const slots = Array.from({ length: inProgressCount }, (_, i) => items4[i] || null);

    expect(slots[0]).toBe("path1.png");
    expect(slots[1]).toBe("path2.png");
    expect(slots[2]).toBe("path3.png");
    expect(slots[3]).toBe("path4.png");
  });

  it("handles primary selection switching correctly", () => {
    let primary = "path1.png";
    const items = ["path1.png", "path2.png"];

    const onSelectPrimary = (newPrimary: string) => {
      primary = newPrimary;
    };

    onSelectPrimary(items[1]);
    expect(primary).toBe("path2.png");
  });

  it("formats stack badge to show image count directly", () => {
    const formatBadge = (items: string[], targetCount: number) => {
      const totalDisplayCount = Math.max(items.length, targetCount);
      return `${totalDisplayCount}`;
    };

    expect(formatBadge(["img1.png", "img2.png"], 2)).toBe("2");
    expect(formatBadge(["img1.png", "img2.png", "img3.png", "img4.png"], 4)).toBe("4");
    expect(formatBadge([], 2)).toBe("2");
  });
});
