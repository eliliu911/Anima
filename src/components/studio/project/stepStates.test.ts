import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import type { EpisodeData, StoryboardShot } from "../../../api/cinema";
import { computeStepStates } from "./stepStates";
import { STEPS } from "./StepRail";

function shot(over: Partial<StoryboardShot> = {}): StoryboardShot {
  return {
    id: "s1", shot_num: 1, scale: "中景", camera_movement: "固定", camera_angle: "", camera_note: "",
    dialogue: "", character_id: "", character_ids: [], scene_id: null,
    prop_ids: [], mode: "first_frame", prompt: "", duration: 3,
    image_url: null, video_url: null, audio_url: null, status: "pending",
    ...over,
  };
}

function ep(over: Partial<EpisodeData> = {}): EpisodeData {
  return {
    id: "ep_01", episode_num: 1, title: "第一集", synopsis: "",
    script_text: "", storyboard: [], final_video_url: null,
    ...over,
  };
}

describe("六步状态", () => {
  it("空项目：六步全部未完成，但都有位置", () => {
    const st = computeStepStates(ep());
    expect(STEPS.every((s) => s.id in st)).toBe(true);
    expect(Object.values(st).every((s) => !s.done)).toBe(true);
  });

  it("剧本有字才算做过 —— 全是空格不算", () => {
    expect(computeStepStates(ep({ script_text: "   \n  " })).script.done).toBe(false);
    expect(computeStepStates(ep({ script_text: "从前有座山" })).script.done).toBe(true);
  });

  it("分镜数和首帧数分开报", () => {
    const st = computeStepStates(
      ep({ storyboard: [shot({ id: "a", image_url: "/a.png" }), shot({ id: "b" })] }),
    );
    expect(st.storyboard.done).toBe(true);
    expect(st.storyboard.note).toBe("2 镜 · 1 首帧 · 0 出片");
  });

  /**
   * **出片数并进分镜那一行。**
   *
   * 「动效」那一步于 2026-09-08 删掉（它只剩一个批量按钮，逐镜的活分镜页
   * 早就全包了）。删之后「出了几镜」没别的地方显示了 —— 而那是这一页
   * 最要紧的一个数，不能跟着一起没。
   */
  it("分镜那行同时报镜数、首帧数和出片数", () => {
    expect(computeStepStates(ep()).storyboard.note).toBeUndefined();
    const st = computeStepStates(
      ep({
        storyboard: [
          shot({ id: "a", image_url: "/a.png", video_url: "/a.mp4" }),
          shot({ id: "b" }),
        ],
      }),
    );
    expect(st.storyboard.note).toBe("2 镜 · 1 首帧 · 1 出片");
  });

  it("有成片才算组装完成", () => {
    expect(computeStepStates(ep({ final_video_url: "/f.mp4" })).assembly.done).toBe(true);
  });

  it("null 分集不炸", () => {
    expect(() => computeStepStates(null)).not.toThrow();
    expect(computeStepStates(null).script.done).toBe(false);
  });
});

describe("软锁是标记，不是门禁", () => {
  it("没有出片的镜头时，组装带软锁并说清原因", () => {
    const st = computeStepStates(ep());
    expect(st.assembly.softLock).toContain("第 4 步");
  });

  it("有一镜出片后软锁解除", () => {
    const st = computeStepStates(ep({ storyboard: [shot({ video_url: "/a.mp4" })] }));
    expect(st.assembly.softLock).toBeUndefined();
  });

  /**
   * **这条是整套改版最重要的不变量。**
   *
   * 旧短剧面板的六个格子是路障：角色资产没做完，点分镜/视频/音频/导出
   * 会被弹回第 2 步。用户说「流程太乱」，根子就在这里。
   *
   * 所以导航栏里**不许出现 disabled**。这条只能从源码上钉 ——
   * 项目里没有 jsdom，渲染测不了；而如果哪天有人加回一个门禁，
   * 界面上的表现是「点了没反应」，不报任何错。
   */
  it("导航栏源码里没有 disabled —— 六步永远都能点", () => {
    const src = readFileSync(
      fileURLToPath(new URL("./StepRail.tsx", import.meta.url)),
      "utf8",
    );
    // 去掉注释再查，免得文件头里提到这个词就误报
    const code = src.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/.*$/gm, "");
    expect(code).not.toMatch(/disabled/);
  });
});

describe("资产这一步什么时候算完成", () => {
  /**
   * **提取出来的实体默认落在分集级**（界面上写「本集待归属」）。
   * 只数项目级的话，刚提取完六个角色，侧栏还是个空圈 ——
   * 而用户明明看见六张卡片。这是 Phase 13 把提取改成写分集级之后
   * 漏掉的一处：AssetsStep、cinema_list_characters、save_cinema_character
   * 都改了，就这个计数器没改。
   */
  it("两级资产一起数（调用方负责合并）", () => {
    const s = computeStepStates(null, { assetCount: 6, assetsWithImage: 6 });
    expect(s.assets.done).toBe(true);
    expect(s.assets.note).toBe("6 个");
  });

  /**
   * **「有资产」不等于「这步做完了」。**
   *
   * 没有图的资产只贡献一段文字描述，锁不住人物身份 ——
   * 而这一步存在的全部理由就是给后面锁身份。六个空壳打上对号，
   * 等于告诉用户「这步过了」，直到出片时每一镜人脸都不一样才发现。
   */
  it("一张图都没有时不算完成", () => {
    const s = computeStepStates(null, { assetCount: 6, assetsWithImage: 0 });
    expect(s.assets.done).toBe(false);
    expect(s.assets.note).toBe("0/6 有图");
  });

  it("只有一部分有图：算完成，但把差距写出来", () => {
    const s = computeStepStates(null, { assetCount: 6, assetsWithImage: 2 });
    expect(s.assets.done).toBe(true);
    expect(s.assets.note).toBe("2/6 有图");
  });

  it("一个资产都没有时不写注脚，不写「0 个」", () => {
    const s = computeStepStates(null, {});
    expect(s.assets.done).toBe(false);
    expect(s.assets.note).toBeUndefined();
  });
});

describe("动效那一步删掉之后", () => {
  const read = (rel: string) =>
    readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");

  /** 步骤里不能再有 motion —— 留着的话点进去是空白。 */
  it("步骤表里没有 motion 了", () => {
    const rail = read("./StepRail.tsx");
    expect(rail).not.toContain('id: "motion"');
    // 序号要重排，不是挖个洞
    expect(rail).toContain('{ id: "assembly", num: 5');
  });
});
