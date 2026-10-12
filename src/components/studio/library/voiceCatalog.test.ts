import { describe, expect, it } from "vitest";
import { buildCatalog, matchesVoice, unvoicedCount, voiceOf } from "./voiceCatalog";
import type { AssetView } from "../../../api/assets";

const ch = (name: string, voice: string | null): AssetView =>
  ({ id: name, kind: "character", name, character: voice === null ? null : { voice_id: voice } } as never as AssetView);
const prop = (name: string): AssetView => ({ id: name, kind: "prop", name, character: null } as never as AssetView);

describe("音色目录", () => {
  it("空白音色当没配 —— 存了个空串和没存效果一样，读出来得一样", () => {
    expect(voiceOf(ch("a", "  "))).toBe("");
    expect(voiceOf(ch("a", null))).toBe("");
    expect(voiceOf(ch("a", " v "))).toBe("v");
  });

  /**
   * **配了但引擎里没有的必须列出来。** 那种情况配音会悄悄回落到全局音色
   * —— 不报错，听上去就是「这个角色音色没生效」，从现象上查不回来。
   */
  it("角色配了、引擎里没有的也要进目录，并标成不在引擎里", () => {
    const cat = buildCatalog(["edge-a"], [ch("阎王", "野生音色"), ch("小鬼", "edge-a")]);
    const 野 = cat.find((e) => e.name === "野生音色")!;
    expect(野).toBeDefined();
    expect(野.inEngine).toBe(false);
    expect(cat.find((e) => e.name === "edge-a")!.inEngine).toBe(true);
  });

  it("**在用的排前面** —— Edge 全量目录几百个，按字母排会把自己配的那几个淹掉", () => {
    const cat = buildCatalog(["zz", "aa", "mm"], [ch("阎王", "zz")]);
    expect(cat[0].name).toBe("zz");
    expect(cat[0].usedBy.map((a) => a.name)).toEqual(["阎王"]);
  });

  it("谁在用按名字排，重复的角色不会丢", () => {
    const cat = buildCatalog([], [ch("小鬼", "v"), ch("阎王", "v")]);
    expect(cat[0].usedBy.map((a) => a.name)).toEqual(["小鬼", "阎王"]);
  });

  it("引擎里有但没人用的也列出来 —— 那是可选项，不是错误", () => {
    const cat = buildCatalog(["空闲音色"], []);
    expect(cat).toEqual([{ name: "空闲音色", inEngine: true, usedBy: [] }]);
  });

  /**
   * **筛「未配音色」时不能混进道具。** 道具没有音色这回事，
   * 把它们算成「未配」会让这个筛选变成一堆无关的东西。
   */
  it("筛音色时只看角色，道具一律不显示", () => {
    expect(matchesVoice(prop("桌子"), "none")).toBe(false);
    expect(matchesVoice(prop("桌子"), { voice: "v" })).toBe(false);
    expect(matchesVoice(prop("桌子"), "all")).toBe(true);
  });

  it("未配音色 / 指定音色都筛得对", () => {
    expect(matchesVoice(ch("a", null), "none")).toBe(true);
    expect(matchesVoice(ch("a", "v"), "none")).toBe(false);
    expect(matchesVoice(ch("a", "v"), { voice: "v" })).toBe(true);
    expect(matchesVoice(ch("a", "v"), { voice: "w" })).toBe(false);
  });

  it("未配数只数角色", () => {
    expect(unvoicedCount([ch("a", null), ch("b", "v"), prop("桌子")])).toBe(1);
  });
});
