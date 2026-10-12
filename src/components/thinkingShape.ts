import type { ThinkingEffort } from "../api";

/**
 * 根据模型支持的档位，决定思考控件长什么样。
 *
 * 抽成纯函数是为了能单测 —— 判定规则来自实测，而且反直觉：
 * `gpt-oss` 有 low/medium/high 梯度**但没有「关」**，其余族正好相反
 * （只有「关」生效，档位被忽略）。
 */
export function controlShape(levels: ThinkingEffort[]): "none" | "toggle" | "slider" {
  if (levels.length === 0) return "none";
  if (levels.length === 2 && levels.includes("off") && levels.includes("auto")) return "toggle";
  return "slider";
}
