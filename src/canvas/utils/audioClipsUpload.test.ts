import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("../../api/canvasDepth", () => ({
  canvasSaveAudio: vi.fn(async () => ({ path: "/uploads/song_ab12.mp3", secs: 15.2 })),
}));

import { useCanvasStore } from "../store/useCanvasStore";
import { uploadAudioInput } from "./audioClips";

// 测试环境（node）没有 FileReader：桩一个只会回 data URL 的
class FakeReader { result = "data:audio/mpeg;base64,eA=="; onload: (() => void) | null = null; onerror = null; readAsDataURL() { queueMicrotask(() => this.onload?.()); } }
vi.stubGlobal("FileReader", FakeReader);

beforeEach(() => useCanvasStore.getState().clearCanvas());

describe("「上传资源」选了音频：就地变成音频素材节点", () => {
  it("类型换掉、连线保留、出线桩换成音频的", async () => {
    const s = useCanvasStore.getState();
    const up = s.addNode("imageGen", { x: 0, y: 0 }, { referenceOnly: true, label: "外部素材" });
    const v = s.addNode("videoGen", { x: 500, y: 0 });
    s.setEdges([{ id: "e1", source: up, target: v, sourceHandle: "image-out", targetHandle: "prompt-in" }]);
    await uploadAudioInput(up, new File(["x"], "青花瓷.mp3", { type: "audio/mpeg" }));
    const st = useCanvasStore.getState();
    const n = st.nodes.find((x) => x.id === up)!;
    expect(n.type).toBe("audioClip");
    expect(n.data).toMatchObject({ label: "青花瓷.mp3", audioPath: "/uploads/song_ab12.mp3", secs: 15.2 });
    expect(st.edges).toEqual([expect.objectContaining({ id: "e1", source: up, target: v, sourceHandle: "audio-out" })]);
  });

  it("不是音频的文件拒收", async () => {
    const s = useCanvasStore.getState();
    const a = s.addNode("audioClip");
    await expect(uploadAudioInput(a, new File(["x"], "a.png", { type: "image/png" }))).rejects.toThrow("请选择音频文件");
  });
});
