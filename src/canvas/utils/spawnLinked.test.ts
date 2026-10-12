import { describe, expect, it } from "vitest";
import { INPUT_HANDLE, linkFor, OUTPUT_HANDLE, spawnChoices, spawnPosition } from "./spawnLinked";

const labels = (c: { label: string }[]) => c.map((x) => x.label);

describe("从节点长出连好线的新节点（2026-10-10：左侧「+」+ 拖线松手菜单）", () => {
  it("往后接：和以前右侧「+」那份一样", () => {
    expect(labels(spawnChoices("down", { type: "prompt" }))).toEqual(["文本", "图片", "视频"]);
    expect(labels(spawnChoices("down", { type: "audioClip" }))).toEqual(["视频"]);
    expect(labels(spawnChoices("down", { type: "videoGen", isVideo: true, hasVideo: true }))).toEqual(["文本", "图片", "视频", "深度动作捕捉", "超分 ×2", "补帧 ×2"]);
  });
  it("往前接：只给真能连进去的", () => {
    expect(labels(spawnChoices("up", { type: "videoGen" }))).toEqual(["文本", "图片", "视频", "音频"]);
    expect(labels(spawnChoices("up", { type: "imageGen" }))).toEqual(["文本", "图片"]);
    expect(labels(spawnChoices("up", { type: "prompt" }))).toEqual(["文本"]);
    // 没有输入口的（音频素材）往前接不了
    expect(spawnChoices("up", { type: "audioClip" })).toEqual([]);
  });
  it("往后接：它的输出口 → 新节点的输入口；右侧「+」的 -plus 后缀去掉", () => {
    expect(linkFor("down", { id: "a", type: "imageGen", handle: "image-out-plus" }, { id: "b", type: "videoGen" }))
      .toEqual({ source: "a", target: "b", sourceHandle: "image-out", targetHandle: "prompt-in" });
  });
  it("往前接：新节点的输出口 → 它的输入口（左侧「+」是 prompt-in-plus）", () => {
    expect(linkFor("up", { id: "v", type: "videoGen", handle: "prompt-in-plus" }, { id: "i", type: "imageGen" }))
      .toEqual({ source: "i", target: "v", sourceHandle: "image-out", targetHandle: "prompt-in" });
    expect(linkFor("up", { id: "v", type: "videoGen" }, { id: "s", type: "audioClip" })).toEqual({ source: "s", target: "v", sourceHandle: "audio-out", targetHandle: "prompt-in" });
  });
  it("每个能往前接的节点、每个能长出来的节点都有对应的口", () => {
    for (const t of Object.keys(INPUT_HANDLE)) {
      for (const c of spawnChoices("up", { type: t })) expect(OUTPUT_HANDLE[c.type], `${t} ← ${c.type}`).toBeTruthy();
    }
    for (const t of Object.keys(OUTPUT_HANDLE)) {
      for (const c of spawnChoices("down", { type: t })) expect(INPUT_HANDLE[c.type], `${t} → ${c.type}`).toBeTruthy();
    }
  });
  it("位置：往后放右边、往前放左边；松手的放在松手处", () => {
    const from = { position: { x: 100, y: 50 }, width: 400 };
    expect(spawnPosition("down", from, 320)).toEqual({ x: 560, y: 50 });
    expect(spawnPosition("up", from, 320)).toEqual({ x: -280, y: 50 });
    expect(spawnPosition("down", from, 320, { x: 900, y: 300 })).toEqual({ x: 900, y: 240 });
    expect(spawnPosition("up", from, 320, { x: 900, y: 300 })).toEqual({ x: 580, y: 240 });
  });
});

import { avoidOverlap } from "./spawnLinked";

describe("新节点不压别的节点", () => {
  it("挡住了就挪到它下面；没挡就原地", () => {
    const others = [{ x: 0, y: 0, w: 400, h: 300 }, { x: 0, y: 330, w: 400, h: 200 }];
    expect(avoidOverlap({ x: 500, y: 0 }, { w: 300, h: 200 }, others)).toEqual({ x: 500, y: 0 });
    expect(avoidOverlap({ x: 100, y: 50 }, { w: 300, h: 200 }, others)).toEqual({ x: 100, y: 554 });
  });
});
