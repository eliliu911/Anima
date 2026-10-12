import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { episodeSteps, initialSel } from "./SeriesRail";

const read = (rel: string) => readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");

describe("系列项目的本集组", () => {
  it("没有风格（它是整部剧一份的），资产叫本集选角，序号从 1 连续", () => {
    const s = episodeSteps("scripted");
    expect(s.map((x) => x.id)).toEqual(["script", "assets", "storyboard", "assembly"]);
    expect(s.map((x) => x.num)).toEqual([1, 2, 3, 4]);
    expect(s[1].label).toBe("本集选角");
  });

  it("直接创作的项目没有剧本那一步，序号重排", () => {
    const s = episodeSteps("freeform");
    expect(s.map((x) => x.id)).toEqual(["assets", "storyboard", "assembly"]);
    expect(s[0].num).toBe(1);
  });
});

describe("从外面带进来的步骤落到哪", () => {
  it("没指定就到概览 —— 点进系列先看全局，不再掉进第 1 集的剧本", () => {
    expect(initialSel(undefined)).toEqual({ level: "series", page: "overview" });
  });
  it("风格、资产去系列组", () => {
    expect(initialSel("style")).toEqual({ level: "series", page: "style" });
    expect(initialSel("assets")).toEqual({ level: "series", page: "assets" });
  });
  it("剧本、分镜、组装去那一集", () => {
    expect(initialSel("storyboard")).toEqual({ level: "episode", step: "storyboard" });
  });
});

/**
 * **每一页都要有路到得了。** 这个仓库吃过亏：面板渲染了、测试绿了，
 * 但入口是个永远不出现的按钮，用户怎么点都找不到。
 */
describe("系列页的入口都接上了", () => {
  const page = read("./ProjectPage.tsx");
  const rail = read("./SeriesRail.tsx");

  it("系列项目用两组左栏，独立项目还是五步", () => {
    expect(page).toContain('project.kind !== "standalone"');
    expect(page).toContain("<SeriesRail");
    expect(page).toContain("<StepRail");
  });

  it("概览、角色与资产、风格、本集选角都渲染得出来", () => {
    expect(page).toContain('sel.page === "overview" && (');
    expect(page).toContain('<AssetsStep {...common} mode="series" episodes={episodes} />');
    expect(page).toContain('sel.page === "style" && <StyleStep');
    expect(page).toContain('mode="cast"');
  });

  it("左栏里有概览 / 角色与资产 / 风格 / 故事原文四项", () => {
    for (const p of ['page: "overview"', 'page: "assets"', 'page: "style"', 'page: "source"']) {
      expect(rail).toContain(p);
    }
  });

  it("概览的「去处理」和分集卡片都有去处", () => {
    expect(page).toContain('onGoAssets={() => setSel({ level: "series", page: "assets" })}');
    expect(page).toContain("onOpenEpisode={openEpisode}");
  });

  /** 用户 2026-09-26：系统下拉「不好看」，要画布引擎选择器那种浮层 */
  it("换集用自己的浮层，不用系统 select", () => {
    // 注释里写着「不用系统的 `<select>`」—— 只认代码里真用了的（前面不是反引号）
    expect(rail).not.toMatch(/(^|[^`])<select\b/m);
    expect(rail).toContain('role="listbox"');
    // 浮层挂在 .anima-studio 里：挂 body 拿不到主题色；给浮层自己加这个类会把 fixed 盖成 relative
    expect(rail).toContain('btn.current?.closest(".anima-studio")');
    expect(rail).not.toMatch(/className="anima-studio fixed/);
    // 同理：注释里解释了为什么不用它，只查真的调用
    expect(rail).not.toContain(".scrollIntoView(");
  });

  it("故事原文一路接到剧本编辑器", () => {
    // 带上工作区项目 id 和已绑定的剧本项目 id —— 只传名字的话，书名号就能让它对不上
    expect(page).toMatch(/onOpenScripts\?\.\(\{ cinemaProjectId: project\.id, title: project\.title, scriptProjectId: project\.script_project_id \}\)/);
    const shell = read("../StudioShell.tsx");
    expect(shell).toContain("onOpenScripts={(story) =>");
    expect(shell).toContain("initialStory={scriptsStory ?? undefined}");
    const sp = read("../script/ScriptsPage.tsx");
    expect(sp).toContain('setPane({ kind: "story", id: hit.project.id })');
    // 按名字认出来的要存成绑定；认不出来要能手动绑
    expect(sp).toMatch(/hit\.via === "name"[\s\S]{0,120}cinemaLinkScriptProject/);
    expect(sp).toContain("<BindScriptProject");
  });

  it("系列资产页有「从设定文档建资产」，而且打得开", () => {
    const assets = read("../steps/AssetsStep.tsx");
    expect(assets).toContain("onClick={() => setFromDocs(true)}");
    expect(assets).toMatch(/\{fromDocs && \(\s*<DesignDocsExtract/);
  });

  it("系列资产页里有待归属面板", () => {
    const assets = read("../steps/AssetsStep.tsx");
    expect(assets).toMatch(/mode === "series" && \(\s*<SeriesInboxPanel/);
  });
});
