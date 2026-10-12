import { describe, expect, it } from "vitest";
import type { EpisodeData } from "../../../api/cinema";
import type { AssetView } from "../../../api/assets";
import { castOf } from "./castOf";

const a = (id: string, name: string) => ({ id, name, kind: "character" }) as AssetView;
const ep = (o: Partial<EpisodeData>) => ({ id: "e", script_text: "", storyboard: [], ...o }) as EpisodeData;

describe("本集选角", () => {
  const series = [a("c1", "陆川"), a("c2", "星澜"), a("c3", "夏予安"), a("s1", "巷口"), a("x", "七")];

  it("剧本里出现名字就算出场", () => {
    const got = castOf(series, ep({ script_text: "陆川：请问您找谁？\n星澜挡在他身前。" }));
    expect(got.map((x) => x.id)).toEqual(["c1", "c2"]);
  });

  it("分镜按 id 引用的也算，名字没出现也算", () => {
    const got = castOf(
      series,
      ep({ storyboard: [{ character_ids: ["c3"], scene_id: "s1", ref_slots: [{ asset_id: "c2" }] }] as never }),
    );
    expect(got.map((x) => x.id).sort()).toEqual(["c2", "c3", "s1"]);
  });

  it("单字名不按名字认 —— 否则到处都是", () => {
    expect(castOf(series, ep({ script_text: "七十二小时" }))).toEqual([]);
  });

  it("没有分集就是空的", () => {
    expect(castOf(series, null)).toEqual([]);
  });
});
