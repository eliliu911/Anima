import { describe, expect, it } from "vitest";

import { activeLayer, neuralStreamWanted, type LayerInput } from "./neuralLayer";

const xiaoya: LayerInput = { isMultiClip: true, isVideo: true, hasWsFrames: false, hasLipsync: false };
const single: LayerInput = { isMultiClip: false, isVideo: true, hasWsFrames: false, hasLipsync: false };
const photo: LayerInput = { isMultiClip: false, isVideo: false, hasWsFrames: false, hasLipsync: false };

describe("谁在画面上", () => {
  it("三段素材时状态机赢，哪怕 sidecar 正在推帧", () => {
    // 这一条就是那个 bug：原先只要在推帧画布就盖上去，
    // 而 sidecar 只有待机那一段 —— speaking / listening 永远不出现
    expect(activeLayer({ ...xiaoya, hasWsFrames: true })).toBe("video");
    expect(activeLayer(xiaoya)).toBe("video");
  });

  it("单段视频：推帧了用画布，没推就播原片", () => {
    expect(activeLayer({ ...single, hasWsFrames: true })).toBe("canvas");
    expect(activeLayer(single)).toBe("video");
  });

  it("静图只能靠画布 —— 没有可播的视频", () => {
    expect(activeLayer(photo)).toBe("canvas");
    expect(activeLayer({ ...photo, hasWsFrames: true })).toBe("canvas");
  });
});

describe("神经流要不要起", () => {
  it("三段素材时不起 —— 帧推给谁都看不见", () => {
    expect(neuralStreamWanted(xiaoya)).toBe(false);
  });

  it("单段视频和静图要起", () => {
    expect(neuralStreamWanted(single)).toBe(true);
    expect(neuralStreamWanted(photo)).toBe(true);
  });

  it("还没推帧时也要起，否则永远连不上", () => {
    // 拿 hasWsFrames=false 去问 activeLayer 会得到 "video"，
    // 照着它决定要不要连，就成了「不连就没有帧，没有帧就不连」
    expect(neuralStreamWanted({ ...single, hasWsFrames: false })).toBe(true);
  });
});

describe("sidecar 能做真口型时", () => {
  const lip: LayerInput = { isMultiClip: true, isVideo: true, hasWsFrames: true, hasLipsync: true };

  it("画布赢 —— 它同时管口型和体态切换", () => {
    // 和上面那条正好相反：那时 sidecar 只有一段素材、只会做形变，
    // 所以让位；现在它三段都有、还能重绘嘴，前端的 <video> 反而多余
    expect(activeLayer(lip)).toBe("canvas");
  });

  it("但还没推出帧时先播原片，不留白屏", () => {
    expect(activeLayer({ ...lip, hasWsFrames: false })).toBe("video");
  });

  it("连接一定要保持", () => {
    expect(neuralStreamWanted(lip)).toBe(true);
    expect(neuralStreamWanted({ ...lip, hasWsFrames: false })).toBe(true);
  });
});
