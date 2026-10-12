/**
 * 项目页的调试台：`npm run dev` 之后打开 /studio-lab.html。
 *
 * 和 canvas-lab 一个道理：完整应用离开 Tauri 起不来，这里用官方 mock 替换后端，
 * 只挂 `ProjectPage`，数据是一个 6 集的假系列（有待归属、有只留本集、有出了分镜的集）。
 * `?standalone=1` 看单集项目（还是原来的五步）。生产构建不包含这个入口。
 */
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { createRoot } from "react-dom/client";
import { ProjectPage } from "./components/studio/project/ProjectPage";
import { Toasts } from "./components/studio/Toasts";
import { useToasts } from "./components/studio/toastStore";
import "./locales";
import "./theme.css";
import "./studio-theme.css";

mockWindows("main");
const standalone = new URLSearchParams(location.search).has("standalone");

const LAB_PROP = `data:image/svg+xml,${encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="300" height="400"><rect width="300" height="400" fill="#2b3440"/><rect x="125" y="60" width="50" height="270" rx="18" fill="#c9a227"/><circle cx="150" cy="70" r="42" fill="#8aa0b8"/><rect x="60" y="330" width="180" height="50" fill="#5a5f66"/></svg>')}`;
// 360 全景测试图（2:1 等距柱状）：上半是墙（每 90° 一个色块 + 门），下半是地板（纬线 = 地上一圈圈，经线 = 放射线）。
// 导演世界的球形、地面投影对不对，拿它一眼看得出来
const LAB_PANO = `data:image/svg+xml,${encodeURIComponent(
  '<svg xmlns="http://www.w3.org/2000/svg" width="2048" height="1024">' +
    '<rect width="2048" height="512" fill="#39424f"/>' +
    ["#8a5a44", "#4f7a5a", "#4f5f8a", "#8a7a44"].map((c, i) => `<rect x="${i * 512 + 60}" y="200" width="392" height="300" fill="${c}"/><rect x="${i * 512 + 220}" y="330" width="80" height="170" fill="#1b1f26"/><text x="${i * 512 + 256}" y="180" font-size="64" fill="#e8ecf2" text-anchor="middle" font-family="sans-serif">${["北", "东", "南", "西"][i]}</text>`).join("") +
    '<rect y="512" width="2048" height="512" fill="#5b5048"/>' +
    [548, 580, 620, 680, 760, 880, 1000].map((y) => `<line x1="0" y1="${y}" x2="2048" y2="${y}" stroke="#d8c8a8" stroke-width="4"/>`).join("") +
    Array.from({ length: 16 }, (_, i) => `<line x1="${i * 128}" y1="512" x2="${i * 128}" y2="1024" stroke="#d8c8a8" stroke-width="4"/>`).join("") +
    '<line x1="0" y1="512" x2="2048" y2="512" stroke="#ffcc33" stroke-width="6"/></svg>',
)}`;
const LAB_FRAME = `data:image/svg+xml,${encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="320" height="180"><rect width="320" height="180" fill="#2b3040"/><text x="160" y="96" font-size="20" text-anchor="middle" fill="#c8cde0">首帧示例</text></svg>')}`;
const labShot = (id: string, n: number, extra: Record<string, unknown>) => ({
  id, shot_num: n, scene_no: 1, scene_name: "拆机间", duration: 5, status: "pending", scale: "close_up", camera_movement: "push_in",
  camera_angle: "eye_level", dialogue: "", prompt: "", character_ids: [], prop_ids: [], ref_slots: [], dialogue_lines: [],
  segments: [{ start: 0, end: 5, title: "睁眼", visual: "星澜睁眼" }], takes: [],
  prompts: { b: { frame: "A close-up of a silver-haired android lying on a dismantling table.", video: "v", synthesized_at: 1 } },
  ...extra,
});
const LAB_SHOTS = [
  labShot("s1", 1, {
    keyframes: { first: true, last: false }, image_url: LAB_FRAME, image_rev: 1, scene_id: "s1", prop_ids: ["p3"],
    // 这一场的固定布景（镜头表每场写一次）
    stage: {
      facing: "从拆机间正门往里看",
      anchors: ["拆机台：左侧靠墙", "传送链：从右后方斜到中间"],
      lights: ["冷蓝顶灯：厂房顶部整片往下", "暖黄工作灯：拆机台上方，斜照台面"],
      axis: "陆川 ↔ 竹竿：机位在拆机台前方这一侧",
      people: ["陆川：坐在拆机台前，面朝传送链"],
      time: "零点，暴雨",
    },
    // 导演世界那一栏：有一张输出的人偶稿，一个绑了陆川、一个匿名
    director_scene: {
      // 摆过的站位：第 2 镜同一个场景、还没摆过，打开导演世界会沿用这里（站位跨镜头沿用）
      scene: { actors: [
        { id: "lab-jc", label: "陆川", color: "#FF00FF", assetId: "c1", position: [0.6, 0, -3], yawDeg: 200, scale: 1, pose: "standing" },
        { id: "lab-arm", label: "机械臂", color: "#FFB000", kind: "prop", assetId: "p1", imagePath: LAB_PROP, position: [-0.8, 0, -3.4], yawDeg: 0, scale: 1, pose: "standing" },
      ] },
      outputs: { combined: LAB_FRAME, env_only: LAB_FRAME, at: 1, actors: [
      { asset_id: "c1", label: "陆川", color_name: "FLUORESCENT MAGENTA", marker_color: "#FF00FF", in_frame: true },
      { asset_id: null, label: "匿名人物 1", color_name: "FLUORESCENT CYAN", marker_color: "#00FFFF", in_frame: true },
    ] } },
  }),
  // 同一个场景；分镜里出场的是星澜（陆川沿用过来会被标「这一镜没有」）、点名要工具箱
  labShot("s2", 2, { prompts: {}, scene_id: "s1", character_ids: ["c2"], prop_ids: ["p2"] }),
];

const TITLES = ["第三次", "本机无武器", "前女友的婚车", "回收站的主管", "七十二小时", "根权限"];
const episodes = TITLES.map((t, i) => ({
  id: `ep_0${i + 1}`,
  episode_num: i + 1,
  title: `E0${i + 1}-《${t}》`,
  synopsis: "",
  script_text: i < 5 ? "陆川：请问您找谁？\n星澜挡在他身前。\n夏予安：枪放下。" : "",
  script_doc: null,
  // 第 1 集带两镜完整字段，分镜页的详情栏（含关键帧那一栏）才画得出来：第 1 镜挂了首帧，第 2 镜什么都没挂
  storyboard: i === 0 ? LAB_SHOTS : [],
  storyboard_version: 3,
  final_video_url: null,
  style_lock: "",
}));
const project = {
  id: "drama_lab",
  title: "回收师",
  kind: standalone ? "standalone" : "series",
  content_mode: "scripted",
  aspect_ratio: "9:16",
  model_settings: {},
  default_shot_mode: "first_frame",
};
const asset = (id: string, name: string, kind: string, scope: object, o: object = {}) => ({
  id, name, kind, scope, scope_label: "", description: "", starred: false, locked: false, created_at: 0, updated_at: 0,
  unit: { image_variants: [], video_variants: [] }, character: kind === "character" ? { disembodied: false } : null,
  cover_url: null, image_urls: [], image_paths: [], video_urls: [], reference_count: 0, looks: [], ...o,
});
const P = { kind: "project", project_id: "drama_lab" };
const series = [
  asset("c1", "陆川", "character", P),
  asset("c2", "星澜", "character", P),
  asset("c3", "夏予安", "character", P),
  asset("s1", "铁皮屋", "scene", P, { cover_url: LAB_PANO }),
  // 导演世界的道具（U3.5）：有图的是立牌，没图的是方块
  asset("p1", "机械臂", "prop", P, { cover_url: LAB_PROP }),
  asset("p2", "工具箱", "prop", P),
  // 道具的形象：量产款金属牌是主形象，星澜那块是「星澜专属」款（分镜详情里能换）
  asset("p3", "协议铭牌", "prop", P, {
    cover_url: LAB_PROP,
    look_views: [
      { id: "main", name: "主形象", description: "银色椭圆金属牌", nsfw: false, is_main: true, cover_url: LAB_PROP, prompt: "银色椭圆金属牌", unit: { image_variants: [], video_variants: [] }, image_urls: [], image_paths: [] },
      { id: "look_baiye", name: "星澜专属", description: "青色晶石", nsfw: false, is_main: false, cover_url: null, prompt: "星澜专属：\n青色晶石", unit: { image_variants: [], video_variants: [] }, image_urls: [], image_paths: [] },
    ],
  }),
];
const E3 = { kind: "episode", project_id: "drama_lab", episode_id: "ep_03" };
const inbox = {
  pending: [
    { episode_id: "ep_03", episode_num: 3, episode_title: "E03-《前女友的婚车》", match_id: null, asset: asset("x1", "苏晴", "character", E3) },
    { episode_id: "ep_03", episode_num: 3, episode_title: "E03-《前女友的婚车》", match_id: "c2", asset: asset("x2", "星澜", "character", E3, { reference_count: 3 }) },
  ],
  kept: [{ episode_id: "ep_02", episode_num: 2, episode_title: "E02-《本机无武器》", match_id: null, asset: asset("x3", "铁头", "character", E3, { episode_only: true }) }],
};

mockIPC((cmd, args) => {
  const a = args as Record<string, unknown> | undefined;
  if (cmd === "cinema_get_project_detail") return { project, episodes };
  if (cmd === "style_get") return { preset_id: "", positive_prompt: "", negative_prompt: "" };
  if (cmd === "cinema_validate_storyboard") return [];
  // 导演世界（分镜里打开）按分镜上下文读颜色表和剧本
  if (cmd === "cinema_assign_actor_colors") return { colors: [{ asset_id: "c1", name: "陆川", color: "#FF00FF", color_name: "FLUORESCENT MAGENTA" }, { asset_id: "c2", name: "星澜", color: "#00FFFF", color_name: "FLUORESCENT CYAN" }], unassigned: [] };
  if (cmd === "cinema_scripts") return [{ project_id: "drama_lab", episode_id: "ep_01", project_title: "回收师", episode_title: "E01-《第三次》" }];
  if (cmd === "series_assets_inbox") return inbox;
  if (cmd === "studio_script_projects") return [{ id: "sp1", title: "回收师", note: "" }];
  if (cmd === "studio_script_list")
    return ["00-系列总纲", "02-人物小传", "04-分集大纲", "05-场景设计", "06-道具设计", "第01章-第三次"].map((t, i) => ({
      id: `d${i}`, title: t, text: "字".repeat(1000 + i * 700), doc: null, revision: 1, project_id: "sp1", kind: "source",
    }));
  if (cmd === "series_extract_from_texts")
    return [
      { kind: "character", name: "陆川", description: "", persona: "", exists: true, disembodied: false, anchors: null, spec: null },
      { kind: "character", name: "圆婶", description: "", persona: "", exists: false, disembodied: false, anchors: null, spec: null },
      { kind: "scene", name: "郑老的修理铺", description: "", persona: "", exists: false, disembodied: false, anchors: null, spec: null },
      { kind: "prop", name: "协议铭牌", description: "", persona: "", exists: false, disembodied: false, anchors: null, spec: null },
    ];
  if (cmd === "assets_list") {
    const sc = a?.scope as { kind?: string } | null;
    return sc?.kind === "project" ? series : sc?.kind === "episode" ? [] : series;
  }
  if (cmd.startsWith("plugin:event|")) return 0;
  if (cmd.endsWith("_list") || cmd.includes("list_") || cmd.startsWith("jobs")) return [];
  return null;
});

// 控制台里推通知条看堆叠效果：`__toasts.getState().push({...})`
(window as unknown as { __toasts: typeof useToasts }).__toasts = useToasts;

createRoot(document.getElementById("root")!).render(
  <div className="anima-studio flex h-full overflow-hidden">
    <ProjectPage projectId="drama_lab" onBack={() => {}} onOpenScripts={(t) => alert(`跳到剧本编辑器：${t}`)} />
    <Toasts />
  </div>,
);
