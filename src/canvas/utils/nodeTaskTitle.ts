import { nodeGuide } from "./nodeGuide";

const TYPE_NAME: Record<string, string> = {
  videoGen: "视频", imageGen: "图片", upscale: "超分", depthCapture: "深度动作", audio: "试音色",
  directorWorld: "导演世界", panorama: "全景出图", panoViewer: "全景查看", prompt: "文本",
};

/**
 * 任务记录、右下角提示框里显示的节点名（2026-10-11 用户截图：两条「画布 · 生成中」一模一样，看不出是哪个节点）。
 *
 * 有步骤说明的取第一条里「【第 N 步 / 共 M 步】」后面那一句：`第 5 步 · 第一段 · 对峙与追逐`；
 * 没有就用节点类型 + 节点 id 末尾几位（同类节点也分得开）。
 */
export function nodeTaskTitle(node: { id: string; type?: string; data?: unknown }): string {
  const first = nodeGuide(node.data)[0] ?? "";
  const m = first.match(/【第\s*(\d+)\s*步[^】]*】\s*([^。！？\n]+)/);
  if (m) {
    // 括号里的补充（时长、说明）不要：「第一段 · 对峙与追逐（0~15 秒）」→「第一段 · 对峙与追逐」
    const what = m[2].replace(/[（(][^）)]*[）)]/g, "").trim();
    return `第 ${m[1]} 步 · ${what.length > 18 ? `${what.slice(0, 18)}…` : what}`;
  }
  const name = TYPE_NAME[node.type ?? ""] ?? "节点";
  return `${name} ${node.id.slice(-4)}`;
}
