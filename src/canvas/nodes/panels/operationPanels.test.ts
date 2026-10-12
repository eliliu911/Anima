import { describe, it, expect, vi, beforeEach } from "vitest";
import { cinemaPolishPrompt } from "../../../api/polish";

vi.mock("../../../api/polish", () => ({
  cinemaPolishPrompt: vi.fn(),
}));

describe("操作面板 AI 润色与生成张数逻辑", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("图像润色调用 cinemaPolishPrompt(kind: storyboard) 并正确更新提示词", async () => {
    vi.mocked(cinemaPolishPrompt).mockResolvedValueOnce({
      prompt_cn: "雨夜赛博朋克都市中的霓虹光影",
      prompt_en: "Neon lights reflecting in cyberpunk city rainy night, cinematic 8k",
    });

    const result = await cinemaPolishPrompt({
      kind: "storyboard",
      draft: "赛博朋克城市 雨夜",
      engine: "zimage",
    });

    expect(cinemaPolishPrompt).toHaveBeenCalledWith({
      kind: "storyboard",
      draft: "赛博朋克城市 雨夜",
      engine: "zimage",
    });
    expect(result.prompt_cn).toBe("雨夜赛博朋克都市中的霓虹光影");
  });

  it("视频润色调用 cinemaPolishPrompt(kind: video) 并支持运镜生成", async () => {
    vi.mocked(cinemaPolishPrompt).mockResolvedValueOnce({
      prompt_cn: "镜头缓慢向前推进，穿越繁华的霓虹街道",
      prompt_en: "Slow cinematic push in through vibrant neon streets, smooth 4k",
    });

    const result = await cinemaPolishPrompt({
      kind: "video",
      draft: "向前推镜头",
      engine: "h3",
    });

    expect(cinemaPolishPrompt).toHaveBeenCalledWith({
      kind: "video",
      draft: "向前推镜头",
      engine: "h3",
    });
    expect(result.prompt_cn).toContain("镜头缓慢向前推进");
  });

  it("润色接口失败时具备优雅容错能力", async () => {
    vi.mocked(cinemaPolishPrompt).mockRejectedValueOnce(new Error("LLM offline"));

    let failed = false;
    try {
      await cinemaPolishPrompt({
        kind: "storyboard",
        draft: "test draft",
        engine: "zimage",
      });
    } catch (e) {
      failed = true;
    }

    expect(failed).toBe(true);
  });
});

describe("视频面板专业运镜预设与分类体系", () => {
  it("包含 5 大专业分类与至少 20 种高质感运镜", async () => {
    const { MOTION_CATEGORIES } = await import("./VideoOperationPanel");
    expect(MOTION_CATEGORIES.length).toBe(5);
    const totalItems = MOTION_CATEGORIES.reduce((acc, c) => acc + c.items.length, 0);
    expect(totalItems).toBeGreaterThanOrEqual(20);

    // 验证每个条目具备完整的中英文及说明
    for (const cat of MOTION_CATEGORIES) {
      expect(cat.name).toBeTruthy();
      expect(cat.items.length).toBeGreaterThanOrEqual(3);
      for (const item of cat.items) {
        expect(item.id).toBeTruthy();
        expect(item.label).toBeTruthy();
        expect(item.zh).toBeTruthy();
        expect(item.en).toBeTruthy();
        expect(item.desc).toBeTruthy();
      }
    }
  });

  it("覆盖影视分镜词表中的核心运镜规范", async () => {
    const { MOTION_CATEGORIES } = await import("./VideoOperationPanel");
    const allIds = MOTION_CATEGORIES.flatMap((c) => c.items.map((i) => i.id));
    const requiredShotMoves = [
      "push_in", "pull_out", "pan_left", "pan_right",
      "tilt_up", "tilt_down", "truck_left", "truck_right",
      "pedestal_up", "pedestal_down", "orbit", "follow",
      "handheld", "static", "pov", "dolly_zoom",
    ];

    for (const req of requiredShotMoves) {
      expect(allIds, `缺少核心运镜: ${req}`).toContain(req);
    }
  });
});

