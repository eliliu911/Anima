import { readFileSync } from "node:fs";
import { describe, it, expect, beforeEach } from "vitest";
import { useCanvasStore } from "../store/useCanvasStore";
import { inferVideoMode } from "./videoMode";
import { audioLengthNote, emptyUpstreamAudios, isAudioFile, videoInputAudios } from "./audioClips";
import { compileAudioMentions, compilePromptMentions } from "./referenceMentions";
import { aggregateNodeInputs } from "../engine/dagExecutor";
import { ADD_NODE_ITEMS } from "../ui/CanvasAddNodePanel";

/** 用户 2026-10-08：画布里加音频参考（像参考图一样）；「上传资源」自动认音频；额外加「音频」节点 */
beforeEach(() => useCanvasStore.getState().clearCanvas());

const graph = () => {
  const s = useCanvasStore.getState();
  return { nodes: s.nodes, edges: s.edges };
};
/** 一个放好了音频的音频素材节点，连到 target 上 */
function audio(target: string, path: string, label: string, secs = 15) {
  const s = useCanvasStore.getState();
  const id = s.addNode("audioClip", { x: 0, y: 0 }, { label, audioPath: path, secs });
  s.setEdges([...useCanvasStore.getState().edges, { id: `e-${id}`, source: id, target, sourceHandle: "audio-out", targetHandle: "prompt-in" }]);
  return id;
}

describe("画布参考音频", () => {
  it("认音频文件：看 MIME，空 MIME 看扩展名", () => {
    expect(isAudioFile({ type: "audio/mpeg", name: "a.mp3" })).toBe(true);
    expect(isAudioFile({ type: "", name: "青花瓷.flac" })).toBe(true);
    expect(isAudioFile({ type: "image/png", name: "a.png" })).toBe(false);
    expect(isAudioFile({ type: "", name: "a.png" })).toBe(false);
  });

  it("视频节点按连线顺序收音频、去重；只认直接连着的", () => {
    const s = useCanvasStore.getState();
    const v = s.addNode("videoGen");
    audio(v, "/song.wav", "青花瓷");
    audio(v, "/voice.wav", "人声");
    audio(v, "/song.wav", "重复的");
    const { nodes, edges } = graph();
    expect(videoInputAudios(v, nodes, edges).map((a) => [a.path, a.name])).toEqual([["/song.wav", "青花瓷"], ["/voice.wav", "人声"]]);
  });

  it("连了音频 = 参考生视频，一张图也不当首帧；没图要说清楚", () => {
    const s = useCanvasStore.getState();
    const v = s.addNode("videoGen");
    audio(v, "/song.wav", "青花瓷");
    let got = inferVideoMode(v, graph().nodes, graph().edges);
    expect(got.mode).toBe("r2v");
    expect(got.reason).toContain("还要再连至少一张图");
    s.bindReference(s.activeProjectId, v, { path: "/singer.png", name: "歌手" });
    got = inferVideoMode(v, graph().nodes, graph().edges);
    expect(got.mode).toBe("r2v");
    expect(got.images).toEqual(["/singer.png"]);
    expect(got.reason).toContain("<Audio 1>「青花瓷」");
  });

  it("还没放音频的音频节点也算连着（点生成时报，不悄悄当没连）", () => {
    const s = useCanvasStore.getState();
    const v = s.addNode("videoGen");
    const a = s.addNode("audioClip");
    s.setEdges([{ id: "e", source: a, target: v, sourceHandle: "audio-out", targetHandle: "prompt-in" }]);
    expect(emptyUpstreamAudios(v, graph().nodes, graph().edges)).toBe(1);
    expect(inferVideoMode(v, graph().nodes, graph().edges).mode).toBe("r2v");
  });

  it("@音频N / @音频名 → <Audio N>，要在图片那步之前编译（不然名字被摘掉）", () => {
    const auds = [{ name: "青花瓷" }, { name: "人声" }];
    expect(compileAudioMentions("@青花瓷 整段照用，@音频2 只参考音色", auds, "h3")).toBe("<Audio 1> 整段照用，<Audio 2> 只参考音色");
    expect(compileAudioMentions("@青花瓷", auds, "ltx")).toBe("@青花瓷");
    const both = compilePromptMentions(compileAudioMentions("@歌手 跟着 @青花瓷 唱", auds, "h3"), [{ name: "歌手" }], "h3");
    expect(both).toBe("<Picture 1> 跟着 <Audio 1> 唱");
  });

  it("片长和音频对不上要说：长了截、短了后面没声音", () => {
    expect(audioLengthNote([{ path: "a", name: "歌", secs: 240, nodeId: "n" }], 15)).toContain("只用前 15 秒");
    expect(audioLengthNote([{ path: "a", name: "歌", secs: 6, nodeId: "n" }], 15)).toContain("后面没声音");
    expect(audioLengthNote([{ path: "a", name: "歌", secs: 15, nodeId: "n" }], 15)).toBeNull();
  });

  it("加节点菜单有「音频」；上传资源图片和音频都收；执行器把音频发出去", () => {
    expect(ADD_NODE_ITEMS.find((i) => i.label === "音频")?.type).toBe("audio");
    // 2026-10-13 合并：只剩一个「音频」，没有「试音色」了（照 LibTV）
    expect(ADD_NODE_ITEMS.find((i) => i.label === "试音色")).toBeUndefined();
    const img = readFileSync(new URL("../nodes/ImageGenNode.tsx", import.meta.url), "utf8");
    expect(img).toContain("<CanvasMediaUploadButton nodeId={id} />");
    const btn = readFileSync(new URL("../ui/CanvasMediaUploadButton.tsx", import.meta.url), "utf8");
    expect(btn).toContain("isAudioFile(file) ? uploadAudioInput(nodeId, file) : uploadImageInput(nodeId, file, \"source\")");
    const exec = readFileSync(new URL("../engine/dagExecutor.ts", import.meta.url), "utf8");
    expect(exec).toContain("...(audios.length ? { audios } : {}),");
    expect(exec.indexOf("compileAudioMentions(promptToUse")).toBeLessThan(exec.indexOf("compilePromptMentions(promptToUse, resolvedRefs"));
    const view = readFileSync(new URL("../CanvasView.tsx", import.meta.url), "utf8");
    expect(view).toContain("audioClip: memo(AudioClipNodeComponent),");
  });
});

// 2026-10-13 用户：「试音色」念出来的连到视频节点当参考音频（以前连上去被静默忽略）
describe("试音色念出来的当参考音频", () => {
  function tts(target: string, extra: Record<string, unknown>) {
    const s = useCanvasStore.getState();
    const id = s.addNode("audio", { x: 0, y: 0 }, extra);
    s.setEdges([...useCanvasStore.getState().edges, { id: `e-${id}`, source: id, target, sourceHandle: "audio-out", targetHandle: "prompt-in" }]);
    return id;
  }

  it("念好了的进参考音频，和音频素材按连线顺序排；名字带实际用的音色", () => {
    const s = useCanvasStore.getState();
    const v = s.addNode("videoGen");
    tts(v, { outputAudioUrl: "/tts/line1.wav", usedVoice: "星澜", voice: "星澜", secs: 3.2 });
    audio(v, "/song.wav", "青花瓷");
    const { nodes, edges } = graph();
    expect(videoInputAudios(v, nodes, edges).map((a) => [a.path, a.name, a.secs])).toEqual([["/tts/line1.wav", "音频 · 星澜", 3.2], ["/song.wav", "青花瓷", 15]]);
    expect(inferVideoMode(v, nodes, edges).mode).toBe("r2v");
  });

  it("还没念的算「连着但空」，点生成时报", () => {
    const s = useCanvasStore.getState();
    const v = s.addNode("videoGen");
    tts(v, { text: "零号，你逃不掉的。" });
    const { nodes, edges } = graph();
    expect(videoInputAudios(v, nodes, edges)).toEqual([]);
    expect(emptyUpstreamAudios(v, nodes, edges)).toBe(1);
  });

  it("试音色上游那段台词不会混进视频提示词（音频节点是提示词边界）", () => {
    const s = useCanvasStore.getState();
    const v = s.addNode("videoGen");
    const a = tts(v, { outputAudioUrl: "/tts/line1.wav" });
    const p = s.addNode("prompt", { x: 0, y: 0 }, { prompt: "零号，你逃不掉的。" });
    const film = s.addNode("prompt", { x: 0, y: 0 }, { prompt: "rooftop at dawn" });
    s.setEdges([...useCanvasStore.getState().edges,
      { id: "e-p", source: p, target: a, sourceHandle: "text-out", targetHandle: "text-in" },
      { id: "e-f", source: film, target: v, sourceHandle: "text-out", targetHandle: "prompt-in" }]);
    const { nodes, edges } = graph();
    const got = aggregateNodeInputs(v, nodes, edges).combinedPrompt;
    expect(got).toContain("rooftop at dawn");
    expect(got).not.toContain("逃不掉");
  });
});

// 2026-10-13 声音复制（照 LibTV：上游连进来的音频节点 = 参考）
describe("声音复制的参考音频", () => {
  it("直接连着的音频素材 / 念好的试音色当参考；没音频的算空；pcm 不算", async () => {
    const { referenceAudio, emptyReferenceAudio, upstreamSpeechText, canClone } = await import("./audioChain");
    const s = useCanvasStore.getState();
    const target = s.addNode("audio", { x: 0, y: 0 }, { text: "新台词" });
    const clip = s.addNode("audioClip", { x: 0, y: 0 }, { label: "星澜原声", audioPath: "/baiye.wav" });
    s.setEdges([{ id: "e1", source: clip, target, sourceHandle: "audio-out", targetHandle: "text-in" }]);
    let g = graph();
    expect(referenceAudio(target, g.nodes, g.edges)).toEqual({ path: "/baiye.wav", name: "星澜原声", nodeId: clip });
    // 参考那条上游的词不会被拿来念
    const p = s.addNode("prompt", { x: 0, y: 0 }, { prompt: "参考念的那句" });
    s.setEdges([...useCanvasStore.getState().edges, { id: "e2", source: p, target: clip, sourceHandle: "text-out", targetHandle: "text-in" }]);
    g = graph();
    expect(upstreamSpeechText(target, g.nodes, g.edges)).toBe("");
    // 换成还没念的试音色当参考：空 → 要提醒
    const tts = s.addNode("audio", { x: 0, y: 0 }, {});
    s.setEdges([{ id: "e3", source: tts, target, sourceHandle: "audio-out", targetHandle: "text-in" }]);
    g = graph();
    expect(referenceAudio(target, g.nodes, g.edges)).toBeNull();
    expect(emptyReferenceAudio(target, g.nodes, g.edges)).toBe(true);
    useCanvasStore.getState().updateNodeData(tts, { outputAudioUrl: "/x.pcm" });
    g = graph();
    expect(referenceAudio(target, g.nodes, g.edges)).toBeNull();
    useCanvasStore.getState().updateNodeData(tts, { outputAudioUrl: "/x.wav", usedVoice: "星澜" });
    g = graph();
    expect(referenceAudio(target, g.nodes, g.edges)?.path).toBe("/x.wav");
    expect(canClone("gpt_sovits") && canClone("cosy_voice2") && canClone("index_tts2")).toBe(true);
    expect(canClone("edge") || canClone("openai_compatible") || canClone("f5_tts")).toBe(false);
  });

  it("pcm 不当视频的参考音频", () => {
    const s = useCanvasStore.getState();
    const v = s.addNode("videoGen");
    const a = s.addNode("audio", { x: 0, y: 0 }, { outputAudioUrl: "/x.pcm" });
    s.setEdges([{ id: "e", source: a, target: v, sourceHandle: "audio-out", targetHandle: "prompt-in" }]);
    const g = graph();
    expect(videoInputAudios(v, g.nodes, g.edges)).toEqual([]);
  });
});
