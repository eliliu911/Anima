import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import type { CanvasEdge, CanvasNode } from "../types";
import { upstreamSpeechText } from "./audioChain";

const n = (id: string, type: string, data: Record<string, unknown>): CanvasNode =>
  ({ id, type, position: { x: 0, y: 0 }, data } as CanvasNode);
const e = (s: string, t: string): CanvasEdge => ({ id: `${s}>${t}`, source: s, target: t } as CanvasEdge);

describe("音频节点念哪段词", () => {
  it("接提示词节点就念它 —— 几把嗓子才比得公平", () => {
    const nodes = [n("a", "audio", {}), n("p", "prompt", { prompt: "你到底想干什么" })];
    expect(upstreamSpeechText("a", nodes, [e("p", "a")])).toBe("你到底想干什么");
  });

  /**
   * **分镜节点取台词，不取 `frame`。** frame 是后端算好的画面描述，
   * 念出来毫无意义 —— 而且取错了完全不报错，只是音频里念了一大段镜头语言。
   */
  it("接分镜节点取的是台词，不是画面描述", () => {
    const nodes = [n("a", "audio", {}), n("s", "shot", { frame: "中景，暖光，她站在窗边", dialogue: "我没想干什么" })];
    expect(upstreamSpeechText("a", nodes, [e("s", "a")])).toBe("我没想干什么");
  });

  it("隔着几层也找得到 —— 中间常常接着技能或上下文节点", () => {
    const nodes = [n("a", "audio", {}), n("k", "skill", {}), n("p", "prompt", { prompt: "远来的词" })];
    expect(upstreamSpeechText("a", nodes, [e("k", "a"), e("p", "k")])).toBe("远来的词");
  });

  it("上游没有词就回空 —— 节点自己那个输入框接着用", () => {
    expect(upstreamSpeechText("a", [n("a", "audio", {}), n("k", "skill", {})], [e("k", "a")])).toBe("");
  });

  it("连成环也不卡死", () => {
    const nodes = [n("a", "audio", {}), n("x", "skill", {}), n("y", "skill", {})];
    expect(upstreamSpeechText("a", nodes, [e("x", "a"), e("y", "x"), e("x", "y")])).toBe("");
  });
});

describe("接线", () => {
  const node = readFileSync(new URL("../nodes/AudioNode.tsx", import.meta.url), "utf8");

  it("节点真的走上游取词，并调 canvasSpeak", () => {
    expect(node).toContain("upstreamSpeechText(");
    expect(node).toContain("canvasSpeak(");
  });

  /**
   * 传了个不存在的音色时引擎会**悄悄退回默认** ——
   * 不回显的话，用户以为在比三把嗓子，其实听的是同一把。
   */
  it("回显实际用的那把嗓子，并在退回默认时说出来", () => {
    expect(node).toContain("data.usedVoice !== data.voice");
    expect(node).toContain("退回了默认");
  });

  /** 口型的取舍写在节点上，不藏在文档里。 */
  it("节点上写着配音会让口型对不上", () => {
    expect(node).toContain("口型");
  });

  /**
   * **面板渲染了但到不了 = 没做**（记忆：rendered-but-unreachable-ui）。
   */
  it("画布上真的加得出这个节点", () => {
    const view = readFileSync(new URL("../CanvasView.tsx", import.meta.url), "utf8");
    const menu = readFileSync(new URL("../ui/CanvasAddNodePanel.tsx", import.meta.url), "utf8");
    // 节点组件可能包了 memo（bb940b9 的性能优化），两种写法都算注册上了
    expect(view).toMatch(/audio: (memo\()?AudioNodeComponent/);
    expect(menu).toContain('type: "audio"');
  });

  /**
   * 音色下拉的值必须是 `short_name` —— `name` 是给人看的长名，
   * 填错引擎会**静默退回默认音色**，三个节点听起来一模一样。
   */
  /**
   * `file://` 被 CSP 拦掉、`asset://` 在这个应用里实测显示不出来 ——
   * 两种都是静默失败：播放器出现了但一点声音没有。
   */
  it("播放走 anima-media:// 那条协议", () => {
    expect(node).toContain("mediaSrc(data.outputAudioUrl");
    // 注释里出现 file:// 是在解释为什么不能用它，只查代码里没有
    expect(node).not.toMatch(/src=\{`file:\/\//);
  });

  it("音色下拉发的是 short_name", () => {
    expect(node).toContain("value={v.short_name}");
  });
});
