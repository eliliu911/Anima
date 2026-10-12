import { describe, expect, it } from "vitest";
import { byGroup, levelOf, summary } from "./serviceView";
import type { ServiceStatus } from "../api/services";

const mk = (p: Partial<ServiceStatus>): ServiceStatus => ({
  id: "x",
  name: "X",
  group: "voice",
  group_label: "本地语音合成",
  port: 1,
  online: false,
  what: "w",
  degraded: "d",
  how: "",
  optional: false,
  ...p,
});

describe("服务页", () => {
  /**
   * **「离线」不等于「有问题」。** CosyVoice2 没开是完全正常的，
   * 标成红的会让人去修一个不存在的问题。
   */
  it("可选服务离线是灰的不是红的", () => {
    expect(levelOf(mk({ online: false, optional: true }))).toBe("idle");
    expect(levelOf(mk({ online: false, optional: false }))).toBe("warn");
    expect(levelOf(mk({ online: true, optional: true }))).toBe("ok");
    expect(levelOf(mk({ online: true, optional: false }))).toBe("ok");
  });

  /** 总结里**只数该开没开的** —— 把可选的算进去会天天报警 */
  it("总结不把可选服务算成异常", () => {
    const list = [
      mk({ id: "a", online: true, optional: false }),
      mk({ id: "b", online: false, optional: true }),
      mk({ id: "c", online: false, optional: true }),
    ];
    expect(summary(list).bad).toBe(0);
    expect(summary(list).text).toContain("要紧的都在");

    const withBad = [...list, mk({ id: "d", online: false, optional: false })];
    expect(summary(withBad).bad).toBe(1);
    expect(summary(withBad).text).toContain("1 个该开的没开");
  });

  it("没数据时不说「都正常」", () => {
    expect(summary([]).text).toBe("还没探测");
  });

  /** 分组要**保持后端给的顺序** —— 那个顺序是按「坏了影响多大」排的 */
  it("分组保持后端顺序", () => {
    const g = byGroup([
      mk({ id: "1", group: "llm", group_label: "大模型后端" }),
      mk({ id: "2", group: "voice", group_label: "本地语音合成" }),
      mk({ id: "3", group: "llm", group_label: "大模型后端" }),
    ]);
    expect(g.map((x) => x.key)).toEqual(["llm", "voice"]);
    expect(g[0].items.map((x) => x.id)).toEqual(["1", "3"]);
  });
});
