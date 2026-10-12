import { beforeEach, expect, it, vi } from "vitest";
import { studioLoras, studioCapabilities } from "../../api/studio";
import { canvasLoraConfig } from "./loraConfig";
vi.mock("../../api/studio", async (original) => ({ ...await original<typeof import("../../api/studio")>(), studioLoras: vi.fn(), studioCapabilities: vi.fn() }));
beforeEach(() => {
  vi.mocked(studioCapabilities).mockResolvedValue({ engines: [{ id: "sdxl", loras: true }] } as never);
  vi.mocked(studioLoras).mockResolvedValue([
    { name: "a", engines: ["sdxl"], confirmed_triggers: ["hero"], triggers: ["guess"] },
    { name: "b", engines: ["sdxl"], confirmed_triggers: [], triggers: ["hero", "ink"] },
  ] as never);
});
it("merges confirmed triggers first, deduplicates, and preserves weights", async () => {
  const loras = [{name: "a", strength: .8}, {name: "b", strength: .5}];
  expect(await canvasLoraConfig({loras}, "sdxl", "hero portrait")).toEqual({loras, prompt: "ink, hero portrait", added: ["ink"]});
});
it("allows disabling injection without disabling LoRA", async () => {
  const result = await canvasLoraConfig({loras: [{name: "a", strength: .8}], injectTriggers: false}, "sdxl", "portrait");
  expect(result.prompt).toBe("portrait"); expect(result.loras).toHaveLength(1);
});
it("rejects incompatible selections instead of silently dropping them", async () => {
  await expect(canvasLoraConfig({loras: [{name: "missing", strength: .8}]}, "sdxl", "")).rejects.toThrow("不兼容");
});
