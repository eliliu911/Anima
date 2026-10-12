import type { SceneContract } from "../../api/assets";
import { lockLine } from "../nodes/ShotContextNode";

/**
 * 全景的长宽比。**等距圆柱投影就是 2:1**，不是审美选择 ——
 * 别的比例出来的图当全景用会在上下两极撕开。
 */
export const PANO_ASPECT = 2;

/** 全景的默认长边。再大本机出图会明显变慢，而全景本来就要靠后续精修。 */
export const PANO_LONG_SIDE = 2048;

export interface PanoramaPlan {
  prompt: string;
  width: number;
  height: number;
  /** 给人看的提醒。**不阻断生成** —— 这条路信息本来就不全。 */
  warnings: string[];
}

/**
 * 把空间契约编译成一张 360° 全景的提示词。
 *
 * # 位置写成百分比，不写坐标
 *
 * 抄 DramaClaw `scene_spatial_contract` v8 的做法：模型守不住像素坐标，
 * 守得住「全景四分之一处」。所以每个视角落在 `x=N%`，
 * 顺序就是契约里 `views` 的顺序 —— **这不是猜的，是作者排的**。
 *
 * 别的做法是去猜哪个方向是「正面」、哪个是「反打」，
 * 猜错了不报错，只是全景里的门和柜台对调了位置。
 *
 * # 视角少于两个时全景基本是编的
 *
 * 一个方向撑不起 360°，剩下的四分之三只能由模型自己想。这种时候
 * 全景仍然出得来、仍然好看，**但它不是这个场景** —— 所以要告警。
 */
export function panoramaPlan(
  contract: SceneContract | null,
  sceneName: string,
  longSide = PANO_LONG_SIDE,
): PanoramaPlan {
  // **名字只用来说人话，不进提示词。** 见下面那段注释 ——
  // 参数留着是因为告警和界面还要用它称呼这个场景。
  void sceneName;
  const width = Math.round(longSide);
  const height = Math.round(longSide / PANO_ASPECT);
  const warnings: string[] = [];

  if (!contract) {
    return {
      prompt: "",
      width,
      height,
      warnings: ["这个场景还没有空间契约 —— 全景会是纯编的，先去生成契约"],
    };
  }

  const lines: string[] = [
    // 版式用英文的摄影/三维术语起头 —— 训练语料里出现过，比中文描述更容易
    // 命中「等距圆柱」这个具体版式。内容仍然用中文。
    "a seamless 360 degree equirectangular panorama, full spherical environment map, " +
      "horizontally tiling left and right edges, no people, no text.",
  ];

  // **场景名不发给模型。**
  //
  // 宫格那条路上实测过：提示词里同时写了三道禁令（「不要把它写进画面」、
  // 「不要出现任何文字：没有招牌…」、契约负向清单里的「任何招牌文字」），
  // 四格里仍有两格把名字当招牌画在了门楣上。删掉名字之后重跑，招牌没了。
  //
  // 全景这条路当初漏了，2026-09-11 实跑前发现。名字对画面零贡献 ——
  // 场景长什么样全在 `shape` 和锚点里。见 `scene_sheet::grid_prompt`。
  const shape = contract.shape.trim();
  if (shape) lines.push(shape);

  const views = contract.views.filter((v) => v.name.trim());
  if (views.length < 2) {
    warnings.push(
      views.length === 0
        ? "契约里一个视角都没有 —— 整张全景都是编的"
        : "契约里只有一个视角 —— 全景里其余四分之三是编的，别拿它当地理依据",
    );
  }
  if (views.length > 0) {
    // 均分一圈。**百分比不是坐标**，是拓扑位置。
    const spots = views.map((v, i) => {
      const at = Math.round((i / views.length) * 100);
      const bits = [v.toward.trim(), v.far.trim() && `尽头是${v.far.trim()}`]
        .filter(Boolean)
        .join("，");
      return `x=${at}% 是${v.name.trim()}${bits ? `（${bits}）` : ""}`;
    });
    lines.push(`绕一圈依次是：${spots.join("；")}。`);
  }

  const anchors = contract.anchors
    .filter((a) => a.name.trim())
    .map((a) => (a.zone.trim() ? `${a.name.trim()}（${a.zone.trim()}）` : a.name.trim()));
  if (anchors.length) {
    lines.push(`不可移动的锚点，每个**只出现一次**：${anchors.join("、")}。`);
  }

  const rels = contract.relations.map((r) => r.trim()).filter(Boolean);
  if (rels.length) lines.push(`相对位置：${rels.join("；")}。`);

  const mats = contract.materials.map((m) => m.trim()).filter(Boolean);
  if (mats.length) lines.push(`材质：${mats.join("、")}。`);

  const lock = lockLine(contract);
  if (lock) lines.push(lock);

  // **重复是全景最典型的失败样子。**
  //
  // 同一扇门被画成左右各一扇 —— 单看每一侧都正常，展开成一圈才露馅。
  // 正向描述压不住它，必须有一句明说。DramaClaw 把这一类叫
  // `do_not_duplicate`，是它的一等公民。
  lines.push(
    "同一件东西不要在画面里出现两次：门、窗、柜台、楼梯各自只有一处，" +
      "不要因为要填满一圈就复制它们。填不满的地方画连续的墙面、地面和天花。",
  );

  const extra = contract.no_extra.map((x) => x.trim()).filter(Boolean);
  if (extra.length) lines.push(`**不要出现**：${extra.join("、")}。`);

  if (contract.inferred.length) {
    // 编的部分照写出来，但**标明它是编的** —— 下游拿这张图当地理依据时
    // 要知道哪一块不能信。
    warnings.push(`契约里这些是推断补的，全景里别当准：${contract.inferred.join("、")}`);
  }

  return { prompt: lines.join("\n"), width, height, warnings };
}
