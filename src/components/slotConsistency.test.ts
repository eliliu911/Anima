import { describe, expect, it } from "vitest";

import { getTurnImageCalls } from "./ChatHistory";
import { progressSlot } from "../store/imageProgress";

/**
 * 卡片的去重键和进度的归档键**必须逐字一致**。
 *
 * 两边现在是各写各的（`getTurnImageCalls` 里拼一次、`progressSlot` 里拼一次）。
 * 一旦漂了，进度会存进一个没人读的格子 —— 卡片照样永远转骨架屏，
 * **而且不报任何错**。这正是这个项目里最典型的失败形状，所以单独盯一条。
 */
describe("卡片与进度的归档键", () => {
  it("同一张图在两边算出同一个键", () => {
    const cards = getTurnImageCalls(
      [
        {
          id: "c1",
          name: "generate_image",
          arguments: JSON.stringify({ prompt: "方案二", width: 1024, height: 1024 }),
        },
      ],
      [],
    );
    expect(cards).toHaveLength(1);

    const card = cards[0];
    // 卡片那边 ImageArtifactCard 用的是 (alt=prompt, width, height)
    expect(progressSlot(card.prompt, card.width, card.height)).toBe(
      progressSlot("方案二", 1024, 1024),
    );
  });

  /**
   * **不要把 seed 加进这个键。试过，是错的，已回退。**
   *
   * 动机看起来合理：模型一次发三张（同提示词、不同 seed），三张被折叠成一张卡，
   * 三条进度挤进同一个格子互相覆盖，底模徽章在 Z-Image 和 Chroma 之间跳。
   *
   * 但拆开之后是**满屏骨架屏** —— 一次三张就是三个大方块占满整屏，
   * 用户的原话是「体验感很差」。而且这个键在代码里有**三处**在拼
   * （`progressSlot`、`getTurnImageCalls`、`MarkdownRenderer` 的查表），
   * 只改了前两处，渲染那处仍按老键查 —— 进度存进没人读的格子，
   * 徽章、进度、队列信息全部消失，骨架屏永远转。
   *
   * ComfyUI 单实例串行，同一时刻只有一个任务在跑，所以「一张卡 + 当前在跑
   * 那个的引擎和队列」本来就是准确的描述。
   */
  it("同提示词的多次调用仍然折叠成一张卡", () => {
    const calls = [1234, 5678, 9012].map((seed, i) => ({
      id: `c${i}`,
      name: "generate_image",
      arguments: JSON.stringify({ prompt: "同一段", width: 1024, height: 768, seed }),
    }));
    expect(getTurnImageCalls(calls, [])).toHaveLength(1);
  });

  /**
   * **`progressSlot` 只能接三个参数。**
   *
   * 多加一个可选参数看起来无害，实际是陷阱：三处调用点里只要有一处没跟着改，
   * 就是「进度存进没人读的格子」—— 而且不报任何错，只是骨架屏永远转。
   * 这条测试是那次回退留下的护栏。
   */
  it("归档键的参数个数不变", () => {
    expect(progressSlot.length).toBe(3);
  });
});
