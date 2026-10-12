import type { ResolvedRef } from "../../api/assets";
import type { StoryboardShot } from "../../api/cinema";

/**
 * 这一镜引用的资产里，**有几张参考图真的会被用上**。
 *
 * # 三条链路用图的方式完全不同
 *
 * | 链路 | 实际用到的参考图 |
 * |---|---|
 * | 多参考 R2V | 全部（去重后上限 4 张，见 `assets::reference_images`） |
 * | 首帧 + SDXL | **只有一张** —— `shot.character_id` 那个角色的 avatar，用来锁脸 |
 * | 首帧 + Z-Image / Chroma | **一张都不用** —— 这条路是文生图，只吃文字描述 |
 *
 * # 为什么必须说出来
 *
 * 三种情况下界面长得**一模一样**：头像行照样排着一串带图的圆片。
 * 用户看到「小花」那张三视图挂在那儿，理所当然认为这一镜锁住了它的
 * 长相 —— 而在默认的 Z-Image 首帧上，那张图一个像素都没进去，
 * 出来的猫和上一镜不是同一只。全程不报错。
 *
 * 传 `image_path` 能不能解决？不能：它会命中 `SdxlOnly.img2img`，
 * 整条请求被强制回落 SDXL，等于放弃了选 Z-Image 的全部理由。
 * 所以这不是一个「顺手补上」的功能，而是一个**要让用户知道并自己选**
 * 的取舍 —— 需要锁身份就切「多参考 R2V」，那条路本来就是为它准备的。
 */
export interface CastUsage {
  /** 引用里有图的有几个 */
  withImage: number;
  /** 这条链路实际会用上几张 */
  used: number;
  /** 用不上的那些的名字，按引用顺序 */
  unused: string[];
  /** 首帧走参考生图：用不上是因为槽位满了，不是这条路不吃图。 */
  slotsFull?: boolean;
}

/** 参考模式的上限。和 Rust 侧 `assets::reference_images` 对齐。 */
export const R2V_IMAGE_LIMIT = 4;

export function castUsage(
  refs: ResolvedRef[],
  shot: Pick<StoryboardShot, "mode" | "character_id">,
  frameEngine: string,
  /** 首帧引擎是参考生图时，后端挑好的结果（`cinemaFrameReferencePick`）。 */
  framePick?: { names: string[]; unused: string[] } | null,
): CastUsage {
  const imaged = refs.filter((r) => r.asset_id && r.image_path);
  const withImage = imaged.length;

  if (shot.mode === "reference") {
    const used = Math.min(withImage, R2V_IMAGE_LIMIT);
    return { withImage, used, unused: imaged.slice(used).map((r) => r.name) };
  }

  // 首帧 + 参考生图：场景和人物的图按槽位进去，挤不下的由后端点名。
  // 道具不进槽，但它本来就不算「以为挂上了」—— 算进去的话每一镜都在报警。
  if (framePick) {
    return { withImage, used: framePick.names.length, unused: framePick.unused, slotsFull: true };
  }

  // 首帧模式。SDXL 那条路会拿 `character_id` 对应角色的 avatar 锁脸，
  // **只有那一个**；场景和道具的图从来不参与。
  if (frameEngine === "sdxl") {
    const locked = imaged.find((r) => r.kind === "character" && r.asset_id === shot.character_id);
    return {
      withImage,
      used: locked ? 1 : 0,
      unused: imaged.filter((r) => r !== locked).map((r) => r.name),
    };
  }

  return { withImage, used: 0, unused: imaged.map((r) => r.name) };
}

/**
 * 说给人听的那句话。`null` = 没什么可说的。
 *
 * **要带上出路，不只是报告现象。** 「参考图用不上」本身不可操作，
 * 而「切到多参考 R2V」是。
 */
export function castUsageNote(u: CastUsage, mode: StoryboardShot["mode"]): string | null {
  if (u.unused.length === 0) return null;
  const names = u.unused.join("、");
  if (mode === "reference") {
    return `参考图最多挂 ${R2V_IMAGE_LIMIT} 张，${names} 这次用不上 —— 删掉几个引用，或者拆成两镜`;
  }
  if (u.slotsFull) {
    // 首帧最多挂 4 张（场景和角色优先，Rust `FIRST_FRAME_MAX_REFS`）—— 换槽位更多的引擎也不会多挂；出片时这些图照样挂
    return `首帧最多挂 4 张参考图（场景和角色优先），${names} 的图没挂、按文字描述 —— 出片时照样挂`;
  }
  if (u.used > 0) {
    return `首帧只锁了主角的脸，${names} 的参考图用不上（只有描述会进提示词）`;
  }
  return `首帧是文生图，${names} 的参考图一张都用不上 —— 只有描述会进提示词。要锁住长相，就在生成设置里把首帧换成参考生图，或切「多参考 R2V」`;
}
