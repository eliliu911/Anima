import type { StoryboardShot } from "../../../api/cinema";

/**
 * 手动加一镜。
 *
 * # 编号必须重排，不能只给新的那条编号
 *
 * `shot_num` 是显示用的序号（卡片左上角那个大字），而 id 是身份。
 * 在中间插一镜之后只给新的编号的话，会出现两个「03」——
 * 而用户是**按编号沟通**的（「第三镜再慢一点」）。
 *
 * # 为什么默认 `first_frame`
 *
 * 参考模式要有 `@` 引用的资产才有意义，空镜头一个引用都没有，
 * 直接出片等于让引擎随便画一个人。
 */
export function blankShot(existing: StoryboardShot[]): StoryboardShot {
  const id = `shot_${Date.now().toString(36)}`;
  return {
    id,
    shot_num: existing.length + 1,
    // **词表 id，不是显示串** —— 见 shared/shot-language.json
    scale: "medium",
    camera_movement: "static",
    camera_angle: "",
    camera_note: "",
    dialogue: "",
    character_id: "",
    character_ids: [],
    scene_id: null,
    prop_ids: [],
    mode: "first_frame",
    prompt: "",
    negative_prompt: undefined,
    duration: 4,
    image_url: null,
    video_url: null,
    audio_url: null,
    continues_previous: false,
    status: "pending",
  };
}

/** 删掉一镜之后重排编号。**加和删都要走它** —— 只在一边排会漏。 */
export function renumber(shots: StoryboardShot[]): StoryboardShot[] {
  return shots.map((s, i) => ({ ...s, shot_num: i + 1 }));
}

/**
 * 上移 / 下移一镜。**越界时原样返回**，不要环绕到另一头 ——
 * 第一镜点"上移"跳到末尾，那是没人预期的。
 */
export function moveShot(shots: StoryboardShot[], id: string, delta: -1 | 1): StoryboardShot[] {
  const i = shots.findIndex((s) => s.id === id);
  const j = i + delta;
  if (i < 0 || j < 0 || j >= shots.length) return shots;
  const next = [...shots];
  [next[i], next[j]] = [next[j], next[i]];
  return renumber(next);
}

/**
 * 复制一镜，插在它后面。
 *
 * **不复制产物**（首帧 / 视频 / 音频）。复制的是"设定"，
 * 不是"结果" —— 带着上一镜的图会让人以为新镜头已经出过了，
 * 而那张图对应的是旧提示词。
 */
export function duplicateShot(shots: StoryboardShot[], id: string): StoryboardShot[] {
  const i = shots.findIndex((s) => s.id === id);
  if (i < 0) return shots;
  const copy: StoryboardShot = {
    ...shots[i],
    id: `shot_${Date.now().toString(36)}_${Math.random().toString(36).slice(2, 6)}`,
    image_url: null,
    video_url: null,
    audio_url: null,
    status: "pending",
    // 接上一镜是相对关系，复制过来多半不成立 —— 让人自己开
    continues_previous: false,
  };
  return renumber([...shots.slice(0, i + 1), copy, ...shots.slice(i + 1)]);
}
