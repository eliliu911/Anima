import { beforeEach, expect, it, vi } from "vitest";
import { cinemaPolishPrompt } from "../../api/polish";
import { polishCanvasPrompt } from "./polishPrompt";
vi.mock("../../api/polish", () => ({ cinemaPolishPrompt: vi.fn() }));
beforeEach(() => vi.resetAllMocks());
const args = { kind: "storyboard" as const, draft: "original", engine: "zimage" };
it("applies a real result and exposes backend warnings", async () => {
  vi.mocked(cinemaPolishPrompt).mockResolvedValue({ prompt_cn: "new", prompt_en: "new", warning: "unchanged" });
  const apply = vi.fn();
  expect(await polishCanvasPrompt(args, () => true, apply)).toBe("unchanged");
  expect(apply).toHaveBeenCalledWith("new");
});
it("preserves the prompt on failure without invented enhancement", async () => {
  vi.mocked(cinemaPolishPrompt).mockRejectedValue(new Error("offline"));
  const apply = vi.fn();
  await expect(polishCanvasPrompt(args, () => true, apply)).rejects.toThrow("offline");
  expect(apply).not.toHaveBeenCalled();
});
it("does not overwrite changed inputs with a late response", async () => {
  vi.mocked(cinemaPolishPrompt).mockResolvedValue({ prompt_cn: "old result", prompt_en: "old result" });
  const apply = vi.fn();
  expect(await polishCanvasPrompt(args, () => false, apply)).toContain("未应用");
  expect(apply).not.toHaveBeenCalled();
});
it("rejects empty results without erasing the original", async () => {
  vi.mocked(cinemaPolishPrompt).mockResolvedValue({ prompt_cn: "", prompt_en: "" });
  const apply = vi.fn();
  await expect(polishCanvasPrompt(args, () => true, apply)).rejects.toThrow("有效提示词");
  expect(apply).not.toHaveBeenCalled();
});
