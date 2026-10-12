/**
 * 画布的运镜预设（21 个，5 类）+ 提示词里的运镜标签。
 *
 * # 标签
 *
 * 运镜窗口 / @ 候选里选中一个运镜，提示词里插一个 `@运镜·跟拍` 这样的标签（用户 2026-09-25：
 * 「可以叠几个，用 @ 的形式在提示词里标记顺序」）。**标签在提示词里的前后就是运镜顺序**。
 * 发出去之前 `compileCameraMoves` 把它换成那个运镜的描述（中文提示词用中文描述，否则英文）。
 *
 * 示例短片在 `public/camera-previews/<id>.mp4`：H3 图生视频，同一张首帧同一个 seed，只换运镜那句
 * （生成脚本 docs/reports/camera-previews-2026-09-25/gen_h3.py）。**不用 LTX**：它出「摇 / 绕」时是人自己转身、
 * 背景不动 —— 用户要的是「固定场景，固定站立的人物，动的是镜头」（2026-09-25 实测对比过）。
 */
export interface MotionPresetItem {
  id: string;
  label: string;
  zh: string;
  en: string;
  desc: string;
}

export interface MotionCategory {
  id: string;
  name: string;
  items: MotionPresetItem[];
}

export const MOTION_CATEGORIES: MotionCategory[] = [
  {
    id: "dolly",
    name: "推拉变焦",
    items: [
      {
        id: "push_in",
        label: "推镜头 (Push In)",
        zh: "镜头缓慢推近，逐步收紧到主体",
        en: "slow push in, dolly in",
        desc: "推进景别收紧主体",
      },
      {
        id: "pull_out",
        label: "拉镜头 (Pull Out)",
        zh: "镜头缓慢拉远，逐步交代环境",
        en: "pull out, dolly out",
        desc: "后撤拉远交代全景",
      },
      {
        id: "zoom_in",
        label: "光学变焦推 (Zoom In)",
        zh: "光学平滑变焦推近",
        en: "smooth optical zoom in",
        desc: "焦段变化画面放大",
      },
      {
        id: "zoom_out",
        label: "光学变焦拉 (Zoom Out)",
        zh: "光学平滑变焦拉远",
        en: "smooth optical zoom out",
        desc: "焦段变化画面缩小",
      },
      {
        id: "dolly_zoom",
        label: "希区柯克变焦 (Dolly Zoom)",
        zh: "希区柯克眩晕变焦，主体大小不变背景透视剧烈畸变",
        en: "vertigo dolly zoom effect",
        desc: "眩晕透视视觉冲击",
      },
    ],
  },
  {
    id: "pan_tilt",
    name: "横摇俯仰",
    items: [
      {
        id: "pan_left",
        label: "左摇 (Pan Left)",
        zh: "镜头原地向左横摇，带出画面左侧的信息",
        en: "pan left",
        desc: "原地向左水平旋转",
      },
      {
        id: "pan_right",
        label: "右摇 (Pan Right)",
        zh: "镜头原地向右横摇，带出画面右侧的信息",
        en: "pan right",
        desc: "原地向右水平旋转",
      },
      {
        id: "tilt_up",
        label: "上摇 (Tilt Up)",
        zh: "镜头原地向上摇，逐步揭示主体上方的空间",
        en: "tilt up",
        desc: "原地仰头视角抬升",
      },
      {
        id: "tilt_down",
        label: "下摇 (Tilt Down)",
        zh: "镜头原地向下摇，逐步落到主体动作或地面",
        en: "tilt down",
        desc: "原地俯首视线压低",
      },
    ],
  },
  {
    id: "truck_pedestal",
    name: "移机升降",
    items: [
      {
        id: "truck_left",
        label: "左移 (Truck Left)",
        zh: "机身整体向左平移，保持朝向不变",
        en: "truck left, lateral dolly left",
        desc: "机位整体向左横移",
      },
      {
        id: "truck_right",
        label: "右移 (Truck Right)",
        zh: "机身整体向右平移，保持朝向不变",
        en: "truck right, lateral dolly right",
        desc: "机位整体向右横移",
      },
      {
        id: "pedestal_up",
        label: "升镜头 (Pedestal Up)",
        zh: "机身整体垂直上升，视角逐步抬高",
        en: "pedestal up, crane up",
        desc: "机位升降垂直升起",
      },
      {
        id: "pedestal_down",
        label: "降镜头 (Pedestal Down)",
        zh: "机身整体垂直下降，视角逐步压低",
        en: "pedestal down, crane down",
        desc: "机位升降垂直下降",
      },
    ],
  },
  {
    id: "orbit_track",
    name: "环绕跟随",
    items: [
      {
        id: "orbit",
        label: "环绕 (Orbit)",
        zh: "镜头围绕主体转动，逐步展开空间关系",
        en: "orbit around subject",
        desc: "360度水平绕主体旋转",
      },
      {
        id: "orbit_up",
        label: "环绕抬升 (Orbit Up)",
        zh: "镜头围绕主体转动，同时缓慢抬升",
        en: "orbit around subject while rising",
        desc: "螺旋上升环绕",
      },
      {
        id: "orbit_down",
        label: "环绕下降 (Orbit Down)",
        zh: "镜头围绕主体转动，同时缓慢下降",
        en: "orbit around subject while descending",
        desc: "螺旋下降环绕",
      },
      {
        id: "follow",
        label: "跟拍 (Tracking)",
        zh: "镜头持续跟随主体移动，主体始终在视觉中心",
        en: "camera tracking alongside subject",
        desc: "运动跟随锁定主体",
      },
    ],
  },
  {
    id: "special",
    name: "视点纪实",
    items: [
      {
        id: "handheld",
        label: "手持呼吸感 (Handheld)",
        zh: "手持轻微晃动，模拟呼吸感",
        en: "handheld, subtle shake",
        desc: "纪实微晃呼吸感",
      },
      {
        id: "pov",
        label: "主观视角 (POV)",
        zh: "第一人称主观视角，代入角色的眼睛",
        en: "first person point of view (POV)",
        desc: "第一人称沉浸视角",
      },
      {
        id: "fpv",
        label: "穿越机穿梭 (FPV Drone)",
        zh: "穿越机高速穿梭飞行动态",
        en: "FPV drone dynamic flythrough",
        desc: "极速穿梭大动态",
      },
      {
        id: "static",
        label: "固定镜头 (Static)",
        zh: "镜头固定不动，由表演和环境完成叙事",
        en: "static camera, locked off",
        desc: "机位锁定零抖动",
      },
    ],
  },
];

/** 全部运镜，按分类顺序铺平 */
export const CAMERA_MOVES: MotionPresetItem[] = MOTION_CATEGORIES.flatMap((c) => c.items);

export const CAMERA_TAG_PREFIX = "运镜·";

/** 标签里的短名：「跟拍 (Tracking)」→「跟拍」 */
export function cameraShortName(item: MotionPresetItem): string {
  return item.label.split(" (")[0].trim();
}

/** 提示词里的标签文字（不带 @） */
export function cameraTag(item: MotionPresetItem): string {
  return `${CAMERA_TAG_PREFIX}${cameraShortName(item)}`;
}

/** 示例短片地址：按页面源拼成完整 URL（开发时 http://localhost:1420，打包后 tauri://localhost） */
export function cameraPreviewUrl(item: MotionPresetItem): string {
  const rel = `/camera-previews/${item.id}.mp4`;
  return typeof window === "undefined" ? rel : new URL(rel, window.location.href).href;
}

/** 提示词里按出现顺序的运镜（同一个写两遍只算一次） */
export function cameraMovesIn(prompt: string): MotionPresetItem[] {
  const hits = CAMERA_MOVES
    .map((m) => ({ m, at: prompt.indexOf(`@${cameraTag(m)}`) }))
    .filter((h) => h.at >= 0)
    .sort((a, b) => a.at - b.at);
  return hits.map((h) => h.m);
}

/**
 * 发出去之前把 `@运镜·X` 换成运镜描述。**原位替换**，所以顺序就是用户在提示词里排的顺序。
 * 必须在 `compilePromptMentions` 之前做 —— 它会把认不出的 `@名字` 摘成纯文字「运镜·X」，那样模型只看到一个词。
 */
export function compileCameraMoves(prompt: string, chinese: boolean): string {
  let out = prompt;
  for (const m of [...CAMERA_MOVES].sort((a, b) => cameraTag(b).length - cameraTag(a).length)) {
    out = out.split(`@${cameraTag(m)}`).join(chinese ? m.zh : m.en);
  }
  return out;
}
