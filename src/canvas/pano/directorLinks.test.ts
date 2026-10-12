import { describe, it, expect } from "vitest";
import { upstreamSceneAsset, withBlocking, overwriteNote, shotOptions, withDirectorCamera, withoutLegacyDirectorMarkers, directorMotionNote } from "./directorLinks";
import type { CanvasEdge, CanvasNode } from "../types";
import type { EpisodeData } from "../../api/cinema";

const node = (id: string, type: string, data: Record<string, unknown> = {}): CanvasNode =>
  ({ id, type, position: { x: 0, y: 0 }, data }) as CanvasNode;
const edge = (from: string, to: string): CanvasEdge => ({ id: `${from}-${to}`, source: from, target: to });

const shot = (id: string, n: number, extra: Record<string, unknown> = {}) =>
  ({
    id,
    shot_num: n,
    prompt: `第${n}镜的画面描述很长很长很长很长很长`,
    dialogue: "",
    duration: 5,
    status: "pending",
    ...extra,
  }) as never;

const ep = (shots: unknown[]): EpisodeData =>
  ({ id: "ep1", title: "第 1 集", storyboard: shots }) as unknown as EpisodeData;

describe("导演世界往外接的两条线", () => {
  describe("导演机位绑定", () => {
    const camera = { id: "c1", name: "低机位", camera: { position: [0, 1, 2] as [number, number, number], yawDeg: 0, pitchDeg: -5, fovDeg: 60 } };

    it("结构化保存，旧 JSON 标记全部清除", () => {
      const before = ep([shot("s1", 1, { camera_note: "保留这句\n[导演机位:旧1] {\"camera\":{}}\n[导演机位:旧2] {\"camera\":{}}" })]);
      const after = withDirectorCamera(before, "s1", camera)!;
      expect(after.storyboard[0].camera_note).toBe("保留这句");
      expect(after.storyboard[0].director_camera?.name).toBe("低机位");
      expect(after.storyboard[0].director_camera?.motion_note).toContain("固定机位");
    });

    /** 推镜和横移不能写成同一句「平移 1 米」—— 按起始镜头的朝向拆 */
    it("位移按镜头自己的方向拆成推拉 / 横移 / 升降", () => {
      const cam = (x: number, y: number, z: number, yawDeg = 0) => ({ position: [x, y, z] as [number, number, number], yawDeg, pitchDeg: 0, fovDeg: 60 });
      const path = (a: ReturnType<typeof cam>, b: ReturnType<typeof cam>) => ({ id: "p", name: "主运镜", durationSec: 4, cameras: [a, b] });
      // yaw 0 朝 −Z：往 −Z 走是推
      expect(directorMotionNote(camera, path(cam(0, 1.6, 0), cam(0, 1.6, -1.2)))).toContain("机位向前推约 1.20 米");
      expect(directorMotionNote(camera, path(cam(0, 1.6, 0), cam(0, 1.6, 1)))).toContain("机位向后拉约 1.00 米");
      expect(directorMotionNote(camera, path(cam(0, 1.6, 0), cam(0.5, 1.6, 0)))).toContain("向右横移约 0.50 米");
      // 转过 90°（朝 +X）之后，往 +X 走才是推
      const turned = directorMotionNote(camera, path(cam(0, 1.6, 0, 90), cam(1, 1.9, 0, 90)));
      expect(turned).toContain("机位向前推约 1.00 米");
      expect(turned).toContain("升高约 0.30 米");
      expect(turned).not.toContain("横移");
    });

    it("清标记不裁掉用户结尾空格和换行", () => {
      expect(withoutLegacyDirectorMarkers("slow push  \n\n")).toBe("slow push  \n\n");
    });
  });
  describe("这张全景是哪个场景的", () => {
    /** 典型接法是 `全景出图 → 图片 → 导演世界`，中间隔着图片节点。 */
    it("隔着中间节点也找得到", () => {
      const nodes = [
        node("pano", "panorama", { sceneAssetId: "scene_1", sceneName: "安和面馆" }),
        node("img", "imageGen"),
        node("dw", "directorWorld"),
      ];
      const edges = [edge("pano", "img"), edge("img", "dw")];
      expect(upstreamSceneAsset("dw", nodes, edges)).toEqual({
        assetId: "scene_1",
        sceneName: "安和面馆",
      });
    });

    it("上游没挑场景就当没有", () => {
      const nodes = [node("pano", "panorama", {}), node("dw", "directorWorld")];
      expect(upstreamSceneAsset("dw", nodes, [edge("pano", "dw")])).toBeNull();
    });

    it("上游根本没有全景节点也当没有", () => {
      const nodes = [node("img", "imageGen"), node("dw", "directorWorld")];
      expect(upstreamSceneAsset("dw", nodes, [edge("img", "dw")])).toBeNull();
    });

    /** **画布允许用户连出环来**，转死了整个界面就卡住，而且看不出原因。 */
    it("上游有环也不会转死", () => {
      const nodes = [node("a", "imageGen"), node("b", "imageGen"), node("dw", "directorWorld")];
      const edges = [edge("a", "b"), edge("b", "a"), edge("b", "dw")];
      expect(upstreamSceneAsset("dw", nodes, edges)).toBeNull();
    });

    /**
     * **环后面的全景仍然要找得到。**
     *
     * 上一条只断言「不转死」，而「转死」在测试里表现为**挂住而不是失败** ——
     * 挂住的测试抓不到回归，只会让整个套件停在那儿。
     * 这一条把去掉查重的后果变成一个**快速失败**：不查重的话 BFS 会在
     * a↔b 之间空转到耗尽预算，永远走不到后面那个全景节点。
     */
    it("环后面还挂着全景时照样找得到", () => {
      const nodes = [
        node("pano", "panorama", { sceneAssetId: "scene_1", sceneName: "安和面馆" }),
        node("a", "imageGen"),
        node("b", "imageGen"),
        node("dw", "directorWorld"),
      ];
      const edges = [edge("pano", "a"), edge("a", "b"), edge("b", "a"), edge("b", "dw")];
      expect(upstreamSceneAsset("dw", nodes, edges)?.assetId).toBe("scene_1");
    });

    it("往下游找不算 —— 只顺着入边往上", () => {
      const nodes = [
        node("dw", "directorWorld"),
        node("pano", "panorama", { sceneAssetId: "scene_1" }),
      ];
      // 边的方向是 dw → pano，全景在**下游**
      expect(upstreamSceneAsset("dw", nodes, [edge("dw", "pano")])).toBeNull();
    });
  });

  describe("站位写进某一镜", () => {
    const 站位 = [{ actor: "沈星晴", zone: "画面左侧", depth: "前景" }];

    it("只动那一镜，别的原样", () => {
      const before = ep([shot("s1", 1), shot("s2", 2)]);
      const after = withBlocking(before, "s2", 站位)!;
      expect(after.storyboard[1].blocking).toEqual(站位);
      expect(after.storyboard[0].blocking).toBeUndefined();
      // **不改原来那份**
      expect(before.storyboard[1].blocking).toBeUndefined();
    });

    /**
     * 找不到那一镜要**明确失败**。
     *
     * 「点了保存、提示成功、打开一看没变」是这个项目里反复出现的那种失败 ——
     * 悄悄什么都不做比报错糟得多。
     */
    it("找不到那一镜就返回 null，不是悄悄什么都不做", () => {
      expect(withBlocking(ep([shot("s1", 1)]), "根本没有", 站位)).toBeNull();
    });

    it("空分集也不炸", () => {
      expect(withBlocking(ep([]), "s1", 站位)).toBeNull();
    });
  });

  describe("写之前说清会覆盖什么", () => {
    /** 原来手写过站位的话，**这次是整条换掉**，不是往上加。 */
    it("原来有站位就说会覆盖几条", () => {
      const e = ep([shot("s1", 1, { blocking: [{ actor: "旧的" }, { actor: "也旧" }] })]);
      const note = overwriteNote(e, "s1", [{ actor: "沈星晴", zone: "画面左侧", depth: "前景" }]);
      expect(note).toContain("覆盖");
      expect(note).toContain("原有的 2 条");
      expect(note).toContain("1 条");
    });

    it("原来没有就只说写入几条", () => {
      const note = overwriteNote(ep([shot("s1", 1)]), "s1", [
        { actor: "沈星晴", zone: "画面左侧", depth: "前景" },
      ]);
      expect(note).toBe("写入 1 条站位");
    });

    /** **画面里没人时写进去等于清空**，这件事必须说。 */
    it("画面里没人时说清是在清空", () => {
      expect(overwriteNote(ep([shot("s1", 1)]), "s1", [])).toContain("清空");
    });
  });

  describe("镜头选择器", () => {
    /**
     * 标签带**提示词开头**而不是只有编号 —— 用户记得的是画面内容，
     * 不是「第 7 镜」。这一条在分镜卡折叠态上已经验过一次。
     */
    it("带上画面摘要", () => {
      const opts = shotOptions(ep([shot("s1", 1)]));
      expect(opts[0].label).toContain("第 1 镜");
      expect(opts[0].label).toContain("第1镜的画面");
      // 摘要要截断，不能把整段塞进下拉框
      expect(opts[0].label.length).toBeLessThan(30);
    });

    it("没有画面描述时退到台词", () => {
      const opts = shotOptions(ep([shot("s1", 1, { prompt: "", dialogue: "我不走" })]));
      expect(opts[0].label).toContain("我不走");
    });

    it("两样都空就只有编号", () => {
      const opts = shotOptions(ep([shot("s1", 3, { prompt: "", dialogue: "" })]));
      expect(opts[0].label).toBe("第 3 镜");
    });

    it("没分集时给空数组", () => {
      expect(shotOptions(null)).toEqual([]);
    });
  });
});
