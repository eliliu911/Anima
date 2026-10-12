import type { PolishKind } from "../../api/polish";
import type { VideoMode } from "./videoMode";

/**
 * 视频节点该用哪一份润色提示词。
 *
 * # 入参必须是**推导出来的**模式，不是面板上选的那个
 *
 * 面板上的 `mode` 可以是 `auto`，而 `auto` 的意思是「按连线推导」——
 * 推导规则在 [`inferVideoMode`](./videoMode.ts) 里，它会数**实际会发出去
 * 的图有几张、叫什么名字**：0 张是 t2v，1 张是 i2v，多张带名字是 r2v，
 * 还有标了尾帧的 fl2v 和接了深度节点的 motion。
 *
 * 2026-09-19 的第一版在这里自己又推了一次，判据只有「有没有上游图」——
 * 于是连了三张身份参考的 `auto` 节点被判成「有图 → i2v」，
 * 套上「首帧已经定了画面，不要重复描述静态外观」那份提示词，
 * 而它根本没有首帧（Codex Astra 的第三份审查抓到）。
 * 两份推导必然会分叉，所以这里只接受推导结果。
 *
 * # 为什么 t2v 要单独一份
 *
 * 文生视频没有首帧，`video` 那份的前提整个不成立。曾经把它接去
 * 「出图提示词润色」—— 绕开了首帧假设，却换成一份**静态画面**的模板，
 * 时间、动作、声音这三样一样没有。见 `PromptSlot::T2vPolish`。
 */
export function polishKindForMode(mode: VideoMode): PolishKind {
  switch (mode) {
    case "t2v":
      return "t2v";
    case "r2v":
      return "r2v";
    // i2v / fl2v / motion 都是**从一张已有的图开始动** —— 首帧假设成立
    default:
      return "video";
  }
}
