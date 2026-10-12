import { describe, expect, it } from "vitest";
import { anonymousCount, directorDrafts } from "./directorFrames";

describe("分镜的人偶稿", () => {
  const scene = { actors: [{ label: "陆川", color: "#FF00FF", assetId: "char_jc" }, { label: "匿名人物 1", color: "#00ffff" }] };

  it("运镜首尾帧用场面里的人偶：颜色换成调色板名字、没有位置", () => {
    const d = directorDrafts({ scene, trajectory: { rgb_video: "v.webm", first: "f.png", last: "l.png", at: 1 } });
    expect(d.map((x) => [x.label, x.slot])).toEqual([["运镜首帧（人偶稿）", "first"], ["运镜尾帧（人偶稿）", "last"]]);
    expect(d[0].actors[0]).toMatchObject({ asset_id: "char_jc", color_name: "FLUORESCENT MAGENTA", marker_color: "#FF00FF" });
    expect(d[0].actors[1]).toMatchObject({ asset_id: null, color_name: "FLUORESCENT CYAN" });
    expect(anonymousCount(d[0].actors)).toBe(1);
  });

  it("「输出」那张带元数据原样用；比运镜晚的排最前", () => {
    const actors = [{ asset_id: "char_jc", label: "陆川", color_name: "FLUORESCENT MAGENTA", marker_color: "#FF00FF", screen: { x: 0.2, y: 0.5 } }];
    const d = directorDrafts({ scene, trajectory: { rgb_video: "v", first: "f.png", last: "l.png", at: 1 }, outputs: { combined: "c.png", env_only: "e.png", actors, at: 2 } });
    expect(d[0]).toMatchObject({ label: "导演合成图（人偶稿）", path: "c.png", slot: "first" });
    expect(d[0].actors).toEqual(actors);
    expect(directorDrafts(null)).toEqual([]);
  });

  it("群演、道具带着类别和立牌图过去；「没绑角色」只数人", () => {
    const withExtras = {
      actors: [
        ...scene.actors,
        { label: "群演 1", color: "#9E9E9E", kind: "extra" },
        { label: "机械臂", color: "#FFB000", kind: "prop", assetId: "prop_arm", imagePath: "/a/arm.png" },
        { label: "怪东西", color: "#FFB000", kind: "奇怪" },
      ],
    };
    const d = directorDrafts({ scene: withExtras, trajectory: { rgb_video: "v", first: "f.png", last: "l.png", at: 1 } });
    expect(d[0].actors.map((a) => a.role)).toEqual(["person", "person", "extra", "prop", "person"]);
    expect(d[0].actors[3]).toMatchObject({ asset_id: "prop_arm", image_path: "/a/arm.png" });
    // 匿名人物 1 + 认不出类别的当人 = 2；群演和道具不算
    expect(anonymousCount(d[0].actors)).toBe(2);
  });
});
