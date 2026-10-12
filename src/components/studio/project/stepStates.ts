/**
 * 六步各自的状态标记。
 *
 * **只是标记，不参与能不能点。** 判断一律保守：剧本有字才算做过，
 * 分镜有镜头才算。报一个我们验证不了的「完成」，比不报更糟 ——
 * 用户会以为那一步过了，直到最后合成时才发现是空的。
 */
import type { EpisodeData } from "../../../api/cinema";
import type { StepId, StepState } from "./StepRail";

export function computeStepStates(
  episode: EpisodeData | null,
  opts: { hasStyle?: boolean; assetCount?: number; assetsWithImage?: number } = {},
): Record<StepId, StepState> {
  const shots = episode?.storyboard ?? [];
  const rendered = shots.filter((s) => s.video_url).length;
  const framed = shots.filter((s) => s.image_url).length;
  const script = episode?.script_text?.trim() ?? "";
  const hasStyle = !!opts.hasStyle;
  const assetCount = opts.assetCount ?? 0;
  const withImage = opts.assetsWithImage ?? 0;

  return {
    script: {
      done: script.length > 0,
      note: script.length > 0 ? `${script.length} 字` : undefined,
    },
    style: { done: hasStyle, note: hasStyle ? undefined : "未定调" },
    /**
     * 资产这一步。
     *
     * # 「有资产」不等于「这一步做完了」
     *
     * 判据是**有图的资产**，不是资产条数。没有图的资产只会贡献一段
     * 文字描述 —— 而这一步存在的全部理由就是给后面锁人物身份，
     * 只有描述锁不住。提取完六个空壳就打对号，等于告诉用户
     * 「这步过了」，直到出片时人脸每一镜都不一样才发现。
     *
     * 数的是**项目级 + 分集级两处**。提取出来的实体默认落在分集级
     * （「本集待归属」），只数项目级的话刚提取完永远是 0。
     */
    assets: {
      done: withImage > 0,
      note:
        assetCount === 0
          ? undefined
          : withImage === assetCount
            ? `${assetCount} 个`
            : `${withImage}/${assetCount} 有图`,
    },
    storyboard: {
      done: shots.length > 0,
      // **出片进度并进这一行** —— 「动效」那步删掉之后，
      // 「出了几镜」没别的地方显示了，而那是分镜页最要紧的一个数。
      note: shots.length
        ? `${shots.length} 镜 · ${framed} 首帧 · ${rendered} 出片`
        : undefined,
    },
    assembly: {
      done: !!episode?.final_video_url,
      note: episode?.final_video_url ? "已合成" : undefined,
      // **软锁只是一把锁的图标和一句原因，照样能点。**
      // 见 StepRail.tsx 的文件头 —— 这是和旧面板最根本的区别。
      softLock: rendered === 0 ? "还没有出片的镜头，先去第 4 步" : undefined,
    },
  };
}
