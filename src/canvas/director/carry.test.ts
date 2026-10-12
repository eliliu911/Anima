import { describe, expect, it } from "vitest";
import type { StoryboardShot } from "../../api/cinema";
import { blockingSources, castCheck, hasSavedBlocking, inheritFrom } from "./carry";
import type { DirectorActor } from "./scene";

const person = (label: string, assetId?: string, x = 0): DirectorActor => ({
  id: `a-${label}`, label, color: "#FF00FF", assetId, position: [x, 0, -3], yawDeg: 0, scale: 1, pose: "standing",
});
const shot = (num: number, scene: string | null, actors?: DirectorActor[]) =>
  ({
    id: `s${num}`,
    shot_num: num,
    scene_id: scene,
    director_scene: actors ? { scene: { actors } } : null,
  }) as unknown as StoryboardShot;

describe("还没摆过的一镜沿用哪一镜的站位", () => {
  const jc = person("陆川", "char_jc", 0.5);
  const by = person("星澜", "char_by", -0.5);
  const shots = [shot(1, "拆机间", [jc]), shot(2, "走廊", [by]), shot(3, "拆机间", [jc, by]), shot(4, "拆机间"), shot(5, "拆机间"), shot(6, "走廊")];

  it("同一个场景、前面最近的那一镜；坐标原样", () => {
    const from = inheritFrom(shots, shots[3])!;
    expect(from.shotNum).toBe(3);
    expect(from.actors.map((a) => [a.label, a.position])).toEqual([["陆川", [0.5, 0, -3]], ["星澜", [-0.5, 0, -3]]]);
    // 第 5 镜：第 4 镜还没摆过，跳过它接着往前找
    expect(inheritFrom(shots, shots[4])?.shotNum).toBe(3);
  });

  it("前面没有同场景的就用后面最近的；别的场景不沿用", () => {
    const list = [shot(1, "拆机间"), shot(2, "拆机间", [jc]), shot(3, "走廊", [by])];
    expect(inheritFrom(list, list[0])?.shotNum).toBe(2);
    expect(inheritFrom(shots, shots[5])?.shotNum).toBe(2);
  });

  it("自己摆过（包括清空后存的空站位）、没有场景资产：都不沿用", () => {
    const cleared = shot(4, "拆机间", []);
    expect(hasSavedBlocking(cleared)).toBe(true);
    expect(inheritFrom([...shots, cleared], cleared)).toBeNull();
    expect(inheritFrom(shots, shot(9, null))).toBeNull();
    expect(hasSavedBlocking(shot(4, "拆机间"))).toBe(false);
  });

  it("复制来源只有同一个场景的（别的场景坐标系不同，搬过去人会在墙里）；按远近；空站位和自己不算", () => {
    const list = blockingSources([...shots, shot(7, "拆机间", [])], shots[3]);
    expect(list.map((s) => s.shotNum)).toEqual([3, 1]);
    // 走廊那一镜摆过人，但对拆机间不是来源
    expect(blockingSources(shots, shots[5]).map((s) => s.shotNum)).toEqual([2]);
    expect(blockingSources(shots, shot(9, null))).toEqual([]);
  });
});

describe("按这一镜的分镜核对场上", () => {
  const colors = [{ asset_id: "char_jc", name: "陆川" }, { asset_id: "char_by", name: "星澜" }, { asset_id: "char_lzx", name: "夏予安" }];
  const props = [{ id: "prop_knife", name: "激光刀" }, { id: "prop_table", name: "拆机台" }];

  it("多出来的人、缺的人、缺的道具；群演 / 匿名 / 多出来的道具不提示", () => {
    const actors: DirectorActor[] = [
      person("陆川", "char_jc"),
      person("星澜", "char_by"),
      person("匿名人物 1"),
      { ...person("群演 1"), kind: "extra" },
      { ...person("拆机台", "prop_table"), kind: "prop" },
    ];
    const r = castCheck(actors, { character_ids: ["char_jc", "char_lzx"], prop_ids: ["prop_knife"] }, colors, props);
    expect(r.absent.map((a) => a.label)).toEqual(["星澜"]);
    expect(r.missing.map((c) => c.name)).toEqual(["夏予安"]);
    expect(r.missingProps.map((p) => p.name)).toEqual(["激光刀"]);
  });

  it("道具绑了角色 id 也不算人在场；没有分镜就不核对", () => {
    const fake = { ...person("陆川立牌", "char_jc"), kind: "prop" as const };
    expect(castCheck([fake], { character_ids: ["char_jc"] }, colors).missing.map((c) => c.name)).toEqual(["陆川"]);
    expect(castCheck([person("陆川", "char_jc")], null, colors)).toEqual({ absent: [], missing: [], missingProps: [] });
  });
});
