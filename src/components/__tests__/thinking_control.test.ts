import { describe, it, expect } from "vitest";
import { controlShape } from "../thinkingShape";

/**
 * 控件形态的判定规则。
 *
 * 规则本身很短，但取值来自实测且反直觉（gpt-oss 有梯度却没有「关」，
 * 其余族正好相反），所以值得钉住 —— 否则以后有人「顺手统一一下」就全错了。
 */
describe("思考控件按模型能力自适应", () => {
  it("不支持任何干预的模型不渲染控件", () => {
    // coder 系压根不产生思考内容，给个点了没反应的控件比没有更糟
    expect(controlShape([])).toBe("none");
  });

  it("只有开关两态的渲染成胶囊", () => {
    // qwen35moe / gemma4：实测只有 none 生效，low/medium/high 全被忽略
    expect(controlShape(["off", "auto"])).toBe("toggle");
  });

  it("有真实梯度的渲染成滑杆", () => {
    // gpt-oss：实测 low 9 / medium 92 / high 184，干净单调；
    // 既没有「关」，也不含 auto —— auto 是"不干预"不是一个强度档，
    // 而且实测和最低档等价（不发 8 tok vs low 9 tok）
    expect(controlShape(["low", "medium", "high"])).toBe("slider");
  });

  it("既有关又有档位时用滑杆，不能退化成胶囊", () => {
    // 目前没有这种模型，真出现了必须显示全部档位而不是只给开关
    expect(controlShape(["off", "low", "high", "auto"])).toBe("slider");
  });
});
