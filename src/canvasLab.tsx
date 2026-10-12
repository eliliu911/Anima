/**
 * 画布的调试台：`npm run dev` 之后打开 /canvas-lab.html。
 *
 * 存在的理由：画布的样子只能靠眼睛打磨（用户 2026-09-24：「我自己对比着来……我们一点点打磨」），
 * 而完整应用离开 Tauri 起不来（读不到窗口元数据）。这里用 Tauri 官方的 mock 把后端调用全替换掉，
 * 只挂画布，放几个常用节点 —— 改完样式直接在浏览器里看、截图对比。
 *
 * 后端调用一律回空（出图出片点了也不会真跑）；生产构建不包含这个入口（vite 只打 index.html）。
 */
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { createRoot } from "react-dom/client";

import { CanvasView } from "./canvas/CanvasView";
import { useCanvasStore } from "./canvas/store/useCanvasStore";
import { canvasCatalogFixture } from "./canvas/utils/modelCatalog.fixture";
import "./locales";
import "./theme.css";
import "./studio-theme.css";

mockWindows("main");

// `?reducedMotion=1`：模拟系统开了「减少动态效果」（浏览器面板里没法真去改系统设置）
if (new URLSearchParams(location.search).has("reducedMotion")) {
  const real = window.matchMedia.bind(window);
  window.matchMedia = (q: string) =>
    q.includes("prefers-reduced-motion") ? ({ ...real(q), matches: true, media: q, addEventListener() {}, removeEventListener() {} } as MediaQueryList) : real(q);
}
mockIPC((cmd) => {
  // 调试台里各引擎都当能挂 LoRA（风格窗口要看 LoRA 那几页）
  if (cmd === "studio_capabilities") return { ...canvasCatalogFixture, engines: canvasCatalogFixture.engines.map((e) => ({ ...e, loras: true })) };
  if (cmd === "studio_loras") return LAB_LORAS;
  if (cmd === "style_catalog") return { categories: [], thumbs_ready: 0, presets: [
    { id: "bjd", category: "3d", name: "BJD", name_zh: "BJD 3D 国漫", subtitle_zh: "半写实人偶质感", description: "", best_for: [], avoid_for: [], positive_prompt: "BJD 3D 国漫质感，柔和体积光", negative_prompt: "", sample_prompt: "", thumb_url: null },
    { id: "ink", category: "2d", name: "Ink", name_zh: "水墨奇幻", subtitle_zh: "东方水墨", description: "", best_for: [], avoid_for: [], positive_prompt: "中国水墨奇幻画风", negative_prompt: "", sample_prompt: "", thumb_url: null },
  ] };
  if (cmd.startsWith("plugin:event|")) return 0;
  // 三个库各一条：顶部资产库要全部列出来（09-25 修的：以前只列当前那一级）
  if (cmd === "assets_list") return [
    { id: "a1", name: "苏菲", kind: "character", scope: { kind: "global" }, scope_label: "全局", image_urls: [] },
    { id: "a2", name: "王小姐", kind: "character", scope: { kind: "project", project_id: "p9" }, scope_label: "《每日小离谱》", image_urls: [] },
    { id: "a3", name: "咖啡馆", kind: "scene", scope: { kind: "episode", project_id: "p9", episode_id: "e1" }, scope_label: "《每日小离谱》· 第 1 集", image_urls: [] },
  ];
  if (cmd.endsWith("_list") || cmd.includes("list_")) return [];
  return null;
});

const lora = (name: string, category: string, o: Record<string, unknown> = {}) => ({
  name, label: name.split("/").pop()!.replace(".safetensors", ""), arch: "zimage", engines: ["zimage"], triggers: [], confirmed_triggers: [],
  concepts: [], note: `${category}示例`, nsfw: false, category, source_url: "https://civitai.com/models/1", thumb: null, ...o,
});
const LAB_LORAS = [
  lora("zimage/ZImage-Cyber-Mecha.safetensors", "画风"), lora("zimage/anime_zimgt_v252.safetensors", "画风"),
  lora("zimage/凡人_韩立.safetensors", "角色", { confirmed_triggers: ["韩立"] }), lora("zimage/Z-Image_Cyber_Bob.safetensors", "角色", { confirmed_triggers: ["Cyber Bob", "cowboy hat"] }),
  lora("zimage/NexBlend_Asian_Face.safetensors", "人像"), lora("zimage/360hdri_skybox_zit.safetensors", "场景"),
  lora("zimage/gp_zimage.safetensors", "人体", { nsfw: true }), lora("krea/samucyber.safetensors", "画风", { engines: ["krea2"] }),
];

/** 一套典型的连法：原视频 + 两张人物图 → 出片（原片复刻），再加一个出图节点 */
function seed() {
  const s = useCanvasStore.getState();
  if (s.nodes.length) return;
  const src = s.addNode("videoGen", { x: 40, y: 40 }, { uploadedVideo: true, prompt: "" });
  const a = s.addNode("imageGen", { x: 40, y: 520 }, { engine: "zimage", prompt: "" });
  const b = s.addNode("imageGen", { x: 40, y: 960 }, { engine: "zimage", prompt: "" });
  const out = s.addNode("videoGen", { x: 620, y: 40 }, { engine: "h3", motionEngine: "h3_ref", durationSecs: 8.542, prompt: "把参考视频1中的角色完全替换成对应图片中的角色" });
  s.addNode("imageGen", { x: 620, y: 700 }, { engine: "zimage", prompt: "一只橘猫坐在窗台上" });
  // 用户 09-24 报的那种：连着一个还没出图的图片节点 → 应该是图生视频
  const empty = s.addNode("imageGen", { x: 1200, y: 40 }, { engine: "zimage", prompt: "" });
  const i2v = s.addNode("videoGen", { x: 1700, y: 40 }, { engine: "ltx", prompt: "" });
  s.setEdges([
    { id: "e1", source: src, target: out },
    { id: "e2", source: a, target: out },
    { id: "e3", source: b, target: out },
    { id: "e4", source: empty, target: i2v },
  ]);
}

// 调试台专用：控制台里能直接改状态（比如把节点标成「在跑」看连线效果）
(window as unknown as { canvasStore: typeof useCanvasStore }).canvasStore = useCanvasStore;

createRoot(document.getElementById("root")!).render(<CanvasView />);
setTimeout(seed, 300);
