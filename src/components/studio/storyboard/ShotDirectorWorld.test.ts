import { describe, expect, it } from "vitest";
import type { StoryboardShot } from "../../../api/cinema";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { initialDirectorScene, openingDirectorScene } from "./ShotDirectorWorld";

const shot = (extra: Partial<StoryboardShot> = {}) => ({ id: "s3", shot_num: 3, duration: 12, status: "pending", ...extra }) as StoryboardShot;

describe("工作区打开导演世界：这一镜的初始场面", () => {
  it("没存过：空场面、默认机位在眼高、画幅跟项目、上下文是这一镜", () => {
    const s = initialDirectorScene(shot(), "p", "e", "16:9", 1.7);
    expect(s.actors).toEqual([]);
    expect(s.camera.position).toEqual([0, 1.7, 0]);
    expect(s.aspect).toBe("16:9");
    expect(s.context).toEqual({ projectId: "p", episodeId: "e", shotId: "s3" });
  });

  it("存过：人偶、机位、运镜路径原样恢复；存下来的上下文不算数", () => {
    const saved = {
      actors: [{ id: "a1", label: "陆川", color: "#FF00FF", assetId: "char_jc", position: [0.5, 0, -2], yawDeg: 180, scale: 1, pose: "sit" }],
      camera: { position: [0, 1.6, 0.4], yawDeg: 10, pitchDeg: -5, fovDeg: 50 },
      aspect: "21:9",
      context: { projectId: "别的", episodeId: "别的", shotId: "别的" },
      cameraPaths: [{ id: "p1", name: "主运镜", durationSec: 12, cameras: [] }],
    };
    const s = initialDirectorScene(shot({ director_scene: { scene: saved } }), "p", "e", "16:9", 1.6);
    expect(s.actors[0]).toMatchObject({ label: "陆川", assetId: "char_jc", position: [0.5, 0, -2] });
    expect(s.camera).toMatchObject({ position: [0, 1.6, 0.4], yawDeg: 10, fovDeg: 50 });
    expect(s.aspect).toBe("21:9");
    expect(s.cameraPaths).toHaveLength(1);
    expect(s.context?.shotId).toBe("s3");
  });
});

describe("还没摆过的一镜沿用同场景前一镜的站位", () => {
  const jc = { id: "a1", label: "陆川", color: "#FF00FF", assetId: "char_jc", position: [0.5, 0, -2], yawDeg: 180, scale: 1, pose: "sitting" };
  const s3 = shot({ id: "s3", shot_num: 3, scene_id: "拆机间", director_scene: { scene: { actors: [jc], camera: { position: [2, 1, 2], yawDeg: 40, pitchDeg: 0, fovDeg: 30 } } } });
  const s4 = shot({ id: "s4", shot_num: 4, scene_id: "拆机间" });

  it("人偶原样带过来，机位不带（还是眼高的默认机位）", () => {
    const { scene, carriedFrom } = openingDirectorScene(s4, [s3, s4], "p", "e", "16:9", 1.6);
    expect(carriedFrom).toBe(3);
    expect(scene.actors[0]).toMatchObject({ label: "陆川", position: [0.5, 0, -2], pose: "sitting" });
    expect(scene.camera.position).toEqual([0, 1.6, 0]);
    expect(scene.namedCameras).toEqual([]);
    expect(scene.context?.shotId).toBe("s4");
  });

  it("自己存过（哪怕是空的）就不沿用", () => {
    const own = shot({ id: "s4", shot_num: 4, scene_id: "拆机间", director_scene: { scene: { actors: [] } } });
    expect(openingDirectorScene(own, [s3, own], "p", "e", "16:9", 1.6)).toMatchObject({ carriedFrom: null, scene: { actors: [] } });
  });

  it("关窗前等场面队列写完、校准 flush 完；场面走串行队列（外面一关就重读分镜，下一镜沿用读的就是它）", () => {
    const src = readFileSync(fileURLToPath(new URL("./ShotDirectorWorld.tsx", import.meta.url)), "utf8");
    expect(src).toContain("onClose={() => void close()}");
    expect(src).toContain("await writer.idle();");
    expect(src).toContain("await calibSaver.flush();");
    expect(src).toContain("writer.run(() => cinemaSetDirectorScene(");
    // 不能再有绕过队列的直接写
    expect(src.match(/cinemaSetDirectorScene\(/g)).toHaveLength(1);
    const step = readFileSync(fileURLToPath(new URL("../steps/StoryboardStep.tsx", import.meta.url)), "utf8");
    expect(step).toMatch(/<ShotDirectorWorld[\s\S]*?shots=\{shots\}/);
  });
});
