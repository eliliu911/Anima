import type { CanvasEdge, CanvasNode, CanvasNodeType } from "../types";

/**
 * 内置工作流模板：**开箱就有三套能跑的接法**。
 *
 * # 为什么要内置
 *
 * 工作流目录解决了「存得下来」，但一个新项目里它是空的 ——
 * 用户还是得先知道「该怎么连」才连得出第一套。这三套是把已经验证过的
 * 路径固化下来。
 *
 * # 只用真实存在的节点类型
 *
 * 写一个不存在的类型进去，载入后画布上会出现一个渲染不出来的空节点，
 * **而且不报错** —— 又是这个项目的主病。`PRESET_TYPES` 有测试对着
 * `CanvasNodeType` 核。
 */

export interface Preset {
  id: string;
  name: string;
  /** 一句话说清它解决什么 */
  hint: string;
  build: () => { nodes: CanvasNode[]; edges: CanvasEdge[] };
}

let seq = 0;
const mk = (type: CanvasNodeType, x: number, y: number, data: Record<string, unknown>): CanvasNode =>
  ({ id: `${type}-preset-${Date.now()}-${seq++}`, type, position: { x, y }, data } as CanvasNode);
const link = (s: CanvasNode, t: CanvasNode): CanvasEdge =>
  ({ id: `${s.id}>${t.id}`, source: s.id, target: t.id, animated: true,
     style: { stroke: "rgba(176, 176, 183, 0.45)", strokeWidth: 2 } } as CanvasEdge);

export const PRESETS: Preset[] = [
  {
    id: "scene_to_frame",
    name: "场景 → 角色 → 首帧",
    hint: "先把地理和站位锁住，再让角色进场出首帧。出来的图和成片同源。",
    build: () => {
      const ctx = mk("shotContext", 40, 40, { sceneName: "", viewMap: "", lock: "", blocking: "", state: "" });
      const role = mk("character", 40, 260, { characterId: "custom_char", name: "新角色", promptTrigger: "", appearancePrompt: "" });
      const img = mk("imageGen", 380, 140, { engine: "zimage", width: 1024, height: 1024, seed: 0, prompt: "" });
      return { nodes: [ctx, role, img], edges: [link(ctx, img), link(role, img)] };
    },
  },
  {
    id: "frame_to_video",
    name: "分镜 → 首帧 → 视频",
    hint: "从分集里取一镜，出首帧再出片。视频节点上能直接收回那一镜。",
    build: () => {
      const shot = mk("shot", 40, 60, { label: "", frame: "", unresolved: [] });
      const img = mk("imageGen", 360, 60, { engine: "zimage", width: 1024, height: 1024, seed: 0, prompt: "" });
      const vid = mk("videoGen", 700, 60, { engine: "h3", durationSecs: 5, prompt: "", generateAudio: true });
      // 分镜也直连视频：视频节点靠它知道「收回哪一镜」（见 `upstreamShot`）
      return { nodes: [shot, img, vid], edges: [link(shot, img), link(img, vid), link(shot, vid)] };
    },
  },
  {
    id: "motion_transfer",
    name: "动作参考 → 深度 → 视频",
    hint: "拿一段视频抽骨架，把这段动作换到另一个人身上。",
    build: () => {
      const src = mk("videoGen", 40, 40, { engine: "h3", durationSecs: 5, prompt: "" });
      const depth = mk("depthCapture", 360, 40, {});
      const who = mk("imageGen", 360, 260, { engine: "zimage", width: 1024, height: 1024, seed: 0, prompt: "" });
      const out = mk("videoGen", 700, 140, { engine: "h3", durationSecs: 5, prompt: "", motionEngine: "ltx" });
      return { nodes: [src, depth, who, out], edges: [link(src, depth), link(depth, out), link(who, out)] };
    },
  },
  {
    id: "source_replica",
    name: "原视频 → 视频（原片复刻）",
    hint: "原视频直接连出片节点：背景、光、运镜、切镜都照原片，只把人换成人物图（三视图整张给）。不用深度。",
    build: () => {
      // 2026-09-24 Kotte 跑通的那一套：原视频原样当参考视频 1，人物图按连线顺序是图片 1、图片 2
      const src = mk("videoGen", 40, 40, { engine: "h3", durationSecs: 5, prompt: "" });
      const who = mk("imageGen", 40, 640, { engine: "zimage", width: 1024, height: 1024, seed: 0, prompt: "" });
      const out = mk("videoGen", 620, 40, { engine: "h3", durationSecs: 5, prompt: "", motionEngine: "h3_ref", width: 960, height: 544 });
      return { nodes: [src, who, out], edges: [link(src, out), link(who, out)] };
    },
  },
  {
    id: "long_take_3",
    name: "长镜头 · 三个窗口",
    hint: "一个节点出一整段：选中它，在「长镜头」那一栏逐窗口写提示词。三个窗口都要写。",
    build: () => {
      // **一个节点。** 以前是三个视频节点连成链 —— 底层一直是一次提交、
      // 一个 mp4，但画面上是三个各带播放器的出片节点，而后两个永远不会有视频。
      const n = mk("videoGen", 40, 40, {
        engine: "h3", durationSecs: 5, prompt: "", generateAudio: true,
        longTakeWindows: ["", "", ""],
      });
      return { nodes: [n], edges: [] };
    },
  },
];
