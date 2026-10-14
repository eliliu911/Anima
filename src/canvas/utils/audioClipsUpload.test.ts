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

describe("「上传资源」选了音频：就地变成音频节点", () => {
  it("类型换掉、连线保留、出线桩换成音频的", async () => {
    const s = useCanvasStore.getState();
    const up = s.addNode("imageGen", { x: 0, y: 0 }, { referenceOnly: true, label: "外部素材" });
    const v = s.addNode("videoGen", { x: 500, y: 0 });
    s.setEdges([{ id: "e1", source: up, target: v, sourceHandle: "image-out", targetHandle: "prompt-in" }]);
    await uploadAudioInput(up, new File(["x"], "青花瓷.mp3", { type: "audio/mpeg" }));
    const st = useCanvasStore.getState();
    const n = st.nodes.find((x) => x.id === up)!;
    // 2026-10-13 起变成统一的音频节点（上传 / 生成 / 声音复制一个节点）
    expect(n.type).toBe("audio");
    expect(n.data).toMatchObject({ label: "青花瓷.mp3", uploaded: true, outputAudioUrl: "/uploads/song_ab12.mp3", secs: 15.2 });
    expect(st.edges).toEqual([expect.objectContaining({ id: "e1", source: up, target: v, sourceHandle: "audio-out" })]);
  });

  it("不是音频的文件拒收", async () => {
    const s = useCanvasStore.getState();
    const a = s.addNode("audioClip");
    await expect(uploadAudioInput(a, new File(["x"], "a.png", { type: "image/png" }))).rejects.toThrow("请选择音频文件");
  });
});

describe("统一的音频节点", () => {
  it("在音频节点上上传：放进去、清掉上次生成的回显；老的音频素材节点上传还是它自己", async () => {
    const s = useCanvasStore.getState();
    const a = s.addNode("audio", { x: 0, y: 0 }, { text: "台词", usedVoice: "复制：星澜", cloneUsed: { ref_text: "x", ref_text_edited: false, from_secs: 0, to_secs: 1, source_secs: 1, padded: false, ref_path: "/r.wav" } });
    await uploadAudioInput(a, new File(["x"], "旁白.mp3", { type: "audio/mpeg" }));
    const n = useCanvasStore.getState().nodes.find((x) => x.id === a)!;
    expect(n.type).toBe("audio");
    expect(n.data).toMatchObject({ text: "台词", label: "旁白.mp3", uploaded: true, outputAudioUrl: "/uploads/song_ab12.mp3" });
    expect(n.data.usedVoice).toBeUndefined();
    expect(n.data.cloneUsed).toBeUndefined();
    const old = s.addNode("audioClip", { x: 0, y: 0 }, { label: "青花瓷" });
    await uploadAudioInput(old, new File(["x"], "新的.mp3", { type: "audio/mpeg" }));
    const o = useCanvasStore.getState().nodes.find((x) => x.id === old)!;
    expect(o.type).toBe("audioClip");
    expect(o.data).toMatchObject({ label: "青花瓷", audioPath: "/uploads/song_ab12.mp3" });
  });
});
