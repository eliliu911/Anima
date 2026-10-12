import type { StoryboardShot } from "../../../api/cinema";

/**
 * 这一镜怎么接上一镜。**和 Rust 的 `cinema::Continuity` 一一对应。**
 *
 * 用户 2026-09-20 提的是「要不要以上一镜作为首帧参考」，但在 H3 这里那是
 * 三件不同的事，混成一个勾选框必然误导 —— 界面上说「首帧参考」而实际发的是
 * 视频参考，就是又一次「界面说的和实际做的不是一回事」。
 */
export type Continuity = "none" | "ref_video" | "first_frame";

export const CONTINUITY_LABEL: Record<Continuity, string> = {
  none: "不接",
  ref_video: "运镜与光线",
  first_frame: "接住最后一帧",
};

/** 每一档的一句话代价。**要显示出来** —— 三个名字本身说不清区别。 */
export const CONTINUITY_HINT: Record<Continuity, string> = {
  none: "各拍各的",
  ref_video: "延续机位、光线、布景；不锁画面内容",
  first_frame: "上一镜末帧当第一帧；姿势接不上会跳",
};

/**
 * 这一镜实际的衔接方式。**老数据的 `continues_previous` 在这里折进来** ——
 * 和 Rust 的 `StoryboardShot::effective_continuity` 是同一份判断。
 *
 * 两边各读各的就是两份真相：界面显示「不接」而出片去拿了上一镜末帧。
 */
export function effectiveContinuity(shot: StoryboardShot): Continuity {
  const c = (shot as { continuity?: Continuity }).continuity;
  if ((!c || c === "none") && shot.continues_previous) return "first_frame";
  return c ?? "none";
}

export interface ContinuityAdvice {
  /** 推荐哪一档 */
  recommended: Continuity;
  /** 为什么。**必须显示** —— 不说理由的推荐等于让人盲点 */
  why: string;
  /** 非空 = 这一镜根本不能接（第一镜），整组禁用并说明 */
  blocked?: string;
}

/**
 * 给这一镜一个建议。**只建议，不自动改。**
 *
 * 判据就是「上一镜是不是同一场、镜号是不是紧挨着」：
 *
 * - 第一镜：没有上一镜，整组禁用；
 * - 跨场：接了会把上一场的光线布景带进来，推荐「不接」；
 * - 同场紧邻：推荐「运镜与光线」（2026-09-20 实测这一档跨镜比同镜头内部还稳）。
 *
 * **不推荐 `first_frame`** —— 它要求姿势严丝合缝，接不上时开头会跳，
 * 是个需要人判断「这两镜真的是同一瞬间的延续」才该选的档。
 */
/**
 * @param longTake 这一镜所在的场勾了「整段出」没有。
 *
 * **勾了就不能接上一镜。** 长镜头的衔接靠窗口之间的重叠帧，而
 * `run_long_take` 不绑任何视频输入 —— 合出来的正文里那句
 * `<Video 1> is the end of the immediately preceding shot` 会指向一个
 * 不存在的输入，**而且不报错**（2026-09-20 用户问出来的）。
 *
 * 界面置灰只是第一道；`prompt_route_b::synthesize` 还有一道硬拦，
 * 挡住老数据里已经存着 continuity 的那些镜头。
 */
export function continuityAdvice(shot: StoryboardShot, prev?: StoryboardShot | null, longTake = false): ContinuityAdvice {
  if (longTake) {
    return {
      recommended: "none",
      why: "",
      blocked: "这一场整段出（长镜头）—— 窗口之间靠重叠帧衔接，不用接上一镜",
    };
  }
  if (!prev) {
    return { recommended: "none", why: "", blocked: "这是第一镜，没有上一镜可接" };
  }
  const sameScene = !!shot.scene_no && shot.scene_no === prev.scene_no;
  if (!sameScene) {
    return {
      recommended: "none",
      why: `上一镜是另一场（第 ${prev.scene_no ?? "?"} 场），接了会把那一场的光线和布景带过来`,
    };
  }
  return {
    recommended: "ref_video",
    why: `和上一镜同在第 ${shot.scene_no} 场，接上机位和光线不会跳`,
  };
}
