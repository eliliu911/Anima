import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import type { AssetView } from "../../../api/assets";
import type { PromptRoute, ShotIssue, StoryboardShot } from "../../../api/cinema";
import type { Job } from "../../../api/jobs";
import { type AssemblyTile, effectiveLook, assetsMissingImage, assemblyTiles, batchRound, batchTargets, batchUnits, latestSceneRender, longTakeClip, isRepairable, needsSceneRegen, nextStep, runningRoute, scenesNeedingRegen, synthLine, synthTargets, toolbarHints, groupByScene, isSynthesized, framesFirst, promptRound, queuePosition, readyToRender, renderBlocker, renderTakes, sceneRows, stepStatus, takesNewestFirst } from "./storyboardView";
import { renderTargetsFor } from "./RenderOptionsDialog";

const shot = (n: number, extra: Partial<StoryboardShot> = {}): StoryboardShot =>
  ({ id: `s${n}`, shot_num: n, scale: "", camera_movement: "", dialogue: "", prompt: "", duration: 8, status: "pending", ...extra }) as StoryboardShot;

const asset = (id: string, cover: string | null): AssetView => ({ id, name: id, cover_url: cover }) as unknown as AssetView;

describe("分镜页：按场分组", () => {
  it("按场号分组、组内按镜号排，没场号的放最后", () => {
    const groups = groupByScene([
      shot(3, { scene_no: 2, scene_name: "金銮殿 · 日" }),
      shot(9),
      shot(2, { scene_no: 1, scene_name: "办公室 · 夜" }),
      shot(1, { scene_no: 1, scene_name: "办公室 · 夜" }),
    ]);
    expect(groups.map((g) => [g.sceneNo, g.sceneName, g.shots.map((s) => s.shot_num)])).toEqual([
      [1, "办公室 · 夜", [1, 2]],
      [2, "金銮殿 · 日", [3]],
      [0, "未分场", [9]],
    ]);
  });
});

describe("分镜页：三步进度", () => {
  const assets = [asset("char_shen", "x.png"), asset("scene_office", null), asset("prop_phone", null)];
  const shots = [
    shot(1, { character_ids: ["char_shen"], scene_id: "scene_office", prompts: { b: { frame: "", video: "B 正文", synthesized_at: 1 } } }),
    shot(2, { scene_id: "scene_office", prop_ids: ["prop_phone"], prompts: { a: { frame: "", video: "A 正文", synthesized_at: 1 } } }),
  ];
  const issues: ShotIssue[] = [
    { shot_num: 2, field: "dialogue", message: "x" },
    { shot_num: 2, field: "duration", message: "y" },
  ];

  it("没图的资产去重计数", () => {
    expect(assetsMissingImage(shots, assets).map((a) => a.id)).toEqual(["scene_office", "prop_phone"]);
  });

  it("合成数只算当前线路 —— 切线路需要重新合成", () => {
    expect(isSynthesized(shots[0], "b")).toBe(true);
    expect(isSynthesized(shots[0], "a")).toBe(false);
    expect(stepStatus(shots, issues, assets, "b")).toEqual({ shots: 2, shotsWithIssues: 1, missingAssets: 2, unlinkedAssets: 0, synthesized: 1, stale: 0 });
    expect(stepStatus(shots, issues, assets, "a").synthesized).toBe(1);
  });

  it("三步全部完成才能批量生成", () => {
    const done = { shots: 2, shotsWithIssues: 0, missingAssets: 0, unlinkedAssets: 0, synthesized: 2, stale: 0 };
    expect(readyToRender(done)).toBe(true);
    expect(readyToRender({ ...done, shotsWithIssues: 1 })).toBe(false);
    // 资产没图、库里没有：提示词按文字兜底，不挡出片
    expect(readyToRender({ ...done, missingAssets: 1, unlinkedAssets: 2 })).toBe(true);
    expect(readyToRender({ ...done, synthesized: 1 })).toBe(false);
    expect(readyToRender({ ...done, shots: 0, synthesized: 0 })).toBe(false);
    expect(readyToRender({ ...done, stale: 1 })).toBe(false);
  });

  /** 「字段改过要重新合成」是第 3 步的事，算进第 1 步会让人以为镜头表有错。 */
  it("库里没有的资产按名字去重计数", () => {
    const u = [
      shot(1, { unlinked_assets: [{ kind: "prop", name: "玉笏" }, { kind: "scene", name: "长安街市" }] }),
      shot(2, { unlinked_assets: [{ kind: "prop", name: "玉笏" }] }),
    ];
    expect(stepStatus(u, [], [], "b").unlinkedAssets).toBe(2);
  });

  it("要重新合成的归第 3 步，不算镜头问题", () => {
    const withStale: ShotIssue[] = [...issues, { shot_num: 1, field: "prompts", message: "要重新合成" }];
    const st = stepStatus(shots, withStale, assets, "b");
    expect(st.shotsWithIssues).toBe(1);
    expect(st.stale).toBe(1);
  });
});

describe("合成提示词：逐镜状态", () => {
  const job = (extra: Partial<Job> = {}): Job => ({
    id: "j", kind: "prompts", status: "running", project_id: "p", episode_id: "e", shot_id: "", shot_num: 0,
    prompt: "线路 B（H3 官方六段） · 整集", created_at: 1000, progress: { done: 3, total: 14, label: "第 4 镜" }, ...extra,
  });
  const synthesized = (n: number, at: number) => shot(n, { prompts: { b: { frame: "f", video: "v", synthesized_at: at } } });

  /** 用户截图：任务到 4/14，第 1 镜早合成好了，详情里还是「合成中」。 */
  it("整集任务：前面的已合成、当前的合成中、后面的排队", () => {
    expect(promptRound(synthesized(1, 1005), job(), "b")).toBe("done");
    expect(promptRound(shot(4), job(), "b")).toBe("running");
    expect(promptRound(shot(9), job(), "b")).toBe("queued");
  });

  /** 2026-10-06：「加入合成队列」追加在名单后面 —— 单镜任务里排着的也显示排队；这一轮合过又排进来的算排队不算已合成 */
  it("合成队列名单里还没轮到的显示排队", () => {
    const q = job({ prompt: "线路 B（H3 官方六段） · 1 镜", targets: [shot(4).id, shot(9).id, shot(1).id], progress: { done: 0, total: 3, label: "第 4 镜" } });
    expect(promptRound(shot(4), q, "b")).toBe("running");
    expect(promptRound(shot(9), q, "b")).toBe("queued");
    expect(promptRound(synthesized(1, 1005), q, "b")).toBe("queued");
    expect(promptRound(shot(7), q, "b")).toBe(null);
    expect(queuePosition(q, shot(1).id)).toBe(2);
  });

  it("上一轮合成的不算这一轮已合成", () => {
    expect(promptRound(synthesized(9, 900), job(), "b")).toBe("queued");
  });

  it("单镜任务没有排队；任务结束后只剩这一轮已合成的", () => {
    const single = job({ prompt: "线路 B（H3 官方六段） · 1 镜", progress: { done: 0, total: 1, label: "第 4 镜" } });
    expect(promptRound(shot(4), single, "b")).toBe("running");
    expect(promptRound(shot(9), single, "b")).toBe(null);
    const finished = job({ status: "done" });
    expect(promptRound(synthesized(1, 1005), finished, "b")).toBe("done");
    expect(promptRound(shot(9), finished, "b")).toBe(null);
    expect(promptRound(shot(9), null, "b")).toBe(null);
  });

  it("只看当前线路的合成时间", () => {
    expect(promptRound(synthesized(1, 1005), job(), "a")).toBe("queued");
  });
});

describe("出片", () => {
  const b = (n: number, extra: Partial<StoryboardShot> = {}) => shot(n, { prompts: { b: { frame: "f", video: "v", synthesized_at: 1 } }, ...extra });

  it("没合成、过期、有别的镜头在出片都不能出，说清原因", () => {
    expect(renderBlocker(b(1), "b", [], false)).toBeNull();
    expect(renderBlocker(shot(1), "b", [], false)).toContain("还没有合成线路 B");
    expect(renderBlocker(b(1), "a", [], false)).toContain("线路 A");
    expect(renderBlocker(b(1), "b", [{ shot_num: 1, field: "prompts", message: "x" }], false)).toContain("重新合成");
    expect(renderBlocker(b(1), "b", [{ shot_num: 2, field: "prompts", message: "x" }], false)).toBeNull();
    expect(renderBlocker(b(1), "b", [{ shot_num: 1, field: "dialogue", message: "x" }], false)).toBeNull();
    expect(renderBlocker(b(1), "b", [], true)).toContain("一次出一个");
  });

  /**
   * **建议不挡出片。**
   *
   * 2026-09-20 出过事：合成正文的三条启发式建议（二选一、表演指示丢了、
   * 说话人串了）原来也挂 `field: "prompts"`，而这里把任何 `prompts` 问题
   * 都当成「合成之后改过字段」—— 正文里出现一个 `or`（哪怕是
   * `never move or fall` 这种否定并列），这一镜就进不了批量出片。
   * 八镜全中时**整集一镜都出不了**，界面给的理由还是「改过字段或资产图」。
   *
   * 后端那半边由 `cinema.rs` 的同名测试钉（字段必须是 `advice`）。
   */
  it("建议类的问题不挡出片，也不算「要重新合成」", () => {
    const advice = [{ shot_num: 1, field: "advice", message: "正文里有二选一" }];
    expect(renderBlocker(b(1), "b", advice, false)).toBeNull();
    expect(batchTargets([b(1)], "b", advice).ids).toEqual([b(1).id]);
    expect(batchTargets([b(1)], "b", advice).blocked).toBe(0);
    // 真过期的仍然挡
    const stale = [{ shot_num: 1, field: "prompts", message: "改过字段" }];
    expect(batchTargets([b(1)], "b", stale).ids).toEqual([]);
    expect(batchTargets([b(1)], "b", stale).blocked).toBe(1);
  });

  /** 「N 镜要重新合成」这个数字只能数真挡路的 —— 否则是在把用户支去重合成八遍。 */
  it("stale 只数真挡路的", () => {
    const shots = [b(1), b(2)];
    const advice = [{ shot_num: 1, field: "advice", message: "x" }];
    expect(stepStatus(shots, advice, [], "b").stale).toBe(0);
    expect(stepStatus(shots, [{ shot_num: 1, field: "prompts", message: "x" }], [], "b").stale).toBe(1);
  });

  it("每一次新的在前", () => {
    const s = shot(1, { takes: [
      { route: "a", path: "1.mp4", created_at: 10, seed: 1, pictures: 2 },
      { route: "b", path: "2.mp4", created_at: 30, seed: 1, pictures: 2 },
      { route: "b", path: "3.mp4", created_at: 20, seed: 2, pictures: 0 },
    ] });
    expect(takesNewestFirst(s).map((t) => t.path)).toEqual(["2.mp4", "3.mp4", "1.mp4"]);
    expect(takesNewestFirst(shot(2))).toEqual([]);
  });
});

describe("第 3 步进度：重新合成时", () => {
  /** 用户截图：重新合成刚到 3/14，页头已经显示 14/14 已完成。 */
  it("整集重新合成中，排队和正在合成的不算已合成", () => {
    const old = (n: number) => shot(n, { prompts: { b: { frame: "f", video: "旧", synthesized_at: 500 } } });
    const fresh = (n: number) => shot(n, { prompts: { b: { frame: "f", video: "新", synthesized_at: 1200 } } });
    const shots = [fresh(1), fresh(2), old(3), old(4)];
    const job = {
      id: "j", kind: "prompts", status: "running", project_id: "p", episode_id: "e", shot_id: "", shot_num: 0,
      prompt: "线路 B · 整集", created_at: 1000, progress: { done: 2, total: 4, label: "第 3 镜" },
    } as Job;
    expect(stepStatus(shots, [], [], "b", job).synthesized).toBe(2);
    expect(stepStatus(shots, [], [], "b", { ...job, status: "done" }).synthesized).toBe(4);
    expect(stepStatus(shots, [], [], "b").synthesized).toBe(4);
  });
});

describe("批量出片", () => {
  const syn = (n: number, extra: Partial<StoryboardShot> = {}) => shot(n, { prompts: { b: { frame: "f", video: "v", synthesized_at: 1 } }, ...extra });
  const take = (route: "a" | "b") => ({ route, path: `${route}.mp4`, created_at: 1, seed: 1, pictures: 1 });

  it("默认只出这条线路还没出过的，按镜号排；没合成、过期的跳过并计数", () => {
    const shots = [syn(3), syn(1, { takes: [take("b")] }), shot(2), syn(4, { takes: [take("a")] }), syn(5)];
    const issues: ShotIssue[] = [{ shot_num: 5, field: "prompts", message: "要重新合成" }];
    const b = batchTargets(shots, "b", issues);
    expect(b.ids).toEqual(["s3", "s4"]);
    expect(b.redo).toBe(false);
    expect(b.blocked).toBe(2);
  });

  it("都出过了就是全部重出", () => {
    const shots = [syn(2, { takes: [take("b")] }), syn(1, { takes: [take("b")] })];
    expect(batchTargets(shots, "b", [])).toEqual({ ids: ["s1", "s2"], longTakeScenes: [], redo: true, blocked: 0, done: 2 });
    expect(batchTargets([shot(1)], "b", [])).toEqual({ ids: [], longTakeScenes: [], redo: false, blocked: 1, done: 0 });
  });

  /**
   * **长镜头的场整段出，不再逐镜出一遍。**
   *
   * 不摘出来的话，勾了「这一场整段出」之后批量出片仍然会把这一场的镜头
   * 一镜一镜再出一遍：45 分钟 GPU 白烧，而且出来的 take 和整段出的成片
   * 是两套东西，谁也不知道该用哪个。
   */
  it("长镜头的场摘出来整段出，其余逐镜", () => {
    const shots = [syn(1, { scene_no: 1 }), syn(2, { scene_no: 1 }), syn(3, { scene_no: 2 })];
    const b = batchTargets(shots, "b", [], [1]);
    expect(b.longTakeScenes).toEqual([1]);
    expect(b.ids, "第 1 场的两镜还是被逐镜排上了").toEqual(["s3"]);
  });

  /**
   * 长镜头一次提交、不能只重来某一窗口。少一个窗口的提示词整条链就不成立 ——
   * 这时候不该排一个注定失败的任务。
   */
  it("一场里有镜头出不了，这一场就不整段出", () => {
    const shots = [syn(1, { scene_no: 1 }), shot(2, { scene_no: 1 }), syn(3, { scene_no: 2 })];
    const b = batchTargets(shots, "b", [], [1]);
    expect(b.longTakeScenes).toEqual([]);
    // 能出的那一镜仍然逐镜出 —— 不能因为这一场没整成就连它也丢掉
    expect(b.ids).toEqual(["s1", "s3"]);
  });

  /** 2026-09-30 第 5 场长镜头出完（9 小时），再点批量不能把它整场重跑。整条记在分集的 `scene_renders` 上，各镜名下没有 take */
  it("长镜头的场这条线路整段出过了就不再排，全都出过才整体重出", () => {
    const shots = [syn(1, { scene_no: 1 }), syn(2, { scene_no: 1 }), syn(3, { scene_no: 2 }), syn(4, { scene_no: 3 }), syn(5, { scene_no: 3 })];
    const renders = [{ scene_no: 1, route: "b" as const }];
    const b = batchTargets(shots, "b", [], [1, 3], shots, renders);
    expect(b.longTakeScenes).toEqual([3]);
    expect(b.ids).toEqual(["s3"]);
    expect(b.redo).toBe(false);
    // 另一条线路整段出过的不算
    expect(batchTargets(shots, "b", [], [1, 3], shots, [{ scene_no: 1, route: "a" }]).longTakeScenes).toEqual([1, 3]);
    // 全都出过了：逐镜和长镜头一起重出
    const all = [syn(1, { scene_no: 1 }), syn(2, { scene_no: 1 }), syn(3, { scene_no: 2, takes: [take("b")] })];
    expect(batchTargets(all, "b", [], [1], all, renders)).toEqual({ ids: ["s3"], longTakeScenes: [1], redo: true, blocked: 0, done: 2 });
    // 按场出片的勾选行：整段出过的场整场算出过
    expect(sceneRows(shots, "b", [], renders).find((r) => r.no === 1)).toMatchObject({ total: 2, rendered: 2 });
    expect(sceneRows(shots, "a", [], renders).find((r) => r.no === 1)).toMatchObject({ rendered: 0 });
  });

  /** 2026-10-03 审查：重新生成过这一场（镜头 id 换了），旧整段片不能当出过 —— 合成一集会拼进旧片、批量会跳过 */
  it("整段片要是这一场现在这几镜出的，镜头换了就不算", () => {
    const shots = [syn(1, { scene_no: 1 }), syn(2, { scene_no: 1 })];
    const ids = shots.map((s) => s.id);
    const same = [{ scene_no: 1, route: "b" as const, created_at: 1, shot_ids: ids }];
    const old = [{ scene_no: 1, route: "b" as const, created_at: 1, shot_ids: ["旧1", "旧2"] }];
    expect(batchTargets(shots, "b", [], [1], shots, same).longTakeScenes).toEqual([1]); // 出过：全部重出那一档
    expect(batchTargets(shots, "b", [], [1], shots, same).redo).toBe(true);
    expect(batchTargets(shots, "b", [], [1], shots, old)).toMatchObject({ longTakeScenes: [1], redo: false });
    expect(longTakeClip([1], same, 1, "b", shots)).not.toBe(null);
    expect(longTakeClip([1], old, 1, "b", shots)).toBe(null);
    expect(longTakeClip([1], old, 1, "b")).not.toBe(null); // 不给镜头就不核（老调用）
    // 老数据没记 shot_ids 的照旧算
    expect(longTakeClip([1], [{ scene_no: 1, route: "b" as const, created_at: 1 }], 1, "b", shots)).not.toBe(null);
    expect(sceneRows(shots, "b", [], old).find((r) => r.no === 1)).toMatchObject({ rendered: 0 });
    expect(assemblyTiles(shots, [1], old, "b").map((t) => t.kind)).toEqual(["shot", "shot"]);
  });

  it("这一场最新的整段成片：按时间取最新，给了线路只看这条线路", () => {
    const r = [
      { scene_no: 5, route: "b" as const, created_at: 1, path: "旧" },
      { scene_no: 5, route: "b" as const, created_at: 3, path: "新" },
      { scene_no: 5, route: "a" as const, created_at: 9, path: "A" },
      { scene_no: 6, route: "b" as const, created_at: 99, path: "别的场" },
    ];
    expect(latestSceneRender(r, 5, "b")?.path).toBe("新");
    expect(latestSceneRender(r, 5)?.path).toBe("A");
    expect(latestSceneRender(r, 7)).toBe(null);
  });

  it("勾了长镜头但那一场根本不存在，不凭空排任务", () => {
    expect(batchTargets([syn(1, { scene_no: 1 })], "b", [], [9]).longTakeScenes).toEqual([]);
  });

  it("镜头卡状态：自己在出片 = 出片中；批量还没轮到 = 排队；轮过的、不在这一批的 = 无", () => {
    const job = { id: "j", kind: "video_batch", status: "running", project_id: "p", episode_id: "e", shot_id: "", shot_num: 0, prompt: "", created_at: 1, targets: ["s1", "s2", "s3"], progress: { done: 1, total: 3, label: "第 2 镜" } } as Job;
    expect(batchRound(shot(2), job, true)).toBe("running");
    expect(batchRound(shot(3), job, false)).toBe("queued");
    expect(batchRound(shot(1), job, false)).toBe(null);
    expect(batchRound(shot(9), job, false)).toBe(null);
    expect(batchRound(shot(3), { ...job, status: "done" }, false)).toBe(null);
  });

  /** 2026-09-29 夜里长镜头单独先提交，后面那一下批量被拒，其余 32 镜一整夜没排上 */
  it("批量顺序：按镜号，整段出的场排在它第一镜的位置，一个列表交给同一个任务", () => {
    const shots = [shot(1, { scene_no: 1 }), shot(2, { scene_no: 1 }), shot(3, { scene_no: 2 }), shot(4, { scene_no: 2 }), shot(5, { scene_no: 3 })];
    // 第 2 场整段出：夹在第 1 场和第 3 场中间，不提前
    expect(batchUnits(shots, ["s5", "s1", "s2"], [2])).toEqual(["s1", "s2", "scene_2", "s5"]);
    expect(batchUnits(shots, [], [2])).toEqual(["scene_2"]);
    expect(batchUnits(shots, ["s1"], [])).toEqual(["s1"]);
  });

  it("镜头卡状态：整段出的那一场，每一镜跟着它那一项走", () => {
    const job = { id: "j", kind: "video_batch", status: "running", project_id: "p", episode_id: "e", shot_id: "", shot_num: 0, prompt: "", created_at: 1, targets: ["s1", "scene_2", "s5"], progress: { done: 0, total: 3, label: "第 1 镜" } } as Job;
    expect(batchRound(shot(3, { scene_no: 2 }), job, false)).toBe("queued");
    expect(batchRound(shot(3, { scene_no: 2 }), { ...job, progress: { done: 1, total: 3, label: "第 2 场 · 长镜头" } }, false)).toBe("running");
    expect(batchRound(shot(4, { scene_no: 2 }), { ...job, progress: { done: 2, total: 3, label: "第 5 镜" } }, false)).toBe(null);
    expect(batchRound(shot(9, { scene_no: 7 }), job, false)).toBe(null);
  });
});

describe("页头按钮的悬浮提示", () => {
  const base = { route: "b" as const, shots: 14, generating: false, synthesizing: false, videoRunning: false, batch: { ids: ["s1", "s2"], redo: false, blocked: 0 }, synthesized: 14, stale: 0 };

  /** 用户截图：「批量出片（先合成提示词）」灰着，其实是提示词过期了。 */
  it("批量出片点不了时说清是没合成还是过期", () => {
    expect(toolbarHints({ ...base, batch: { ids: [], redo: false, blocked: 14 }, stale: 14 }).batch).toContain("14 镜合成之后改过字段、资产图或风格，先点「重新合成提示词」");
    expect(toolbarHints({ ...base, batch: { ids: [], redo: false, blocked: 14 }, synthesized: 0 }).batch).toContain("先点「合成提示词」");
    expect(toolbarHints({ ...base, videoRunning: true }).batch).toContain("停止批量出片");
    expect(toolbarHints({ ...base, synthesizing: true }).batch).toContain("正在合成提示词");
  });

  it("当前线路没有能出的、另一条有：说清可以在弹窗里换过去", () => {
    expect(toolbarHints({ ...base, route: "a", batch: { ids: [], redo: false, blocked: 14 }, otherBatch: 12 }).batch).toContain("线路 B 可出 12 镜");
  });

  it("能点时说清会做什么", () => {
    const h = toolbarHints({ ...base, batch: { ids: ["s1", "s2"], redo: false, blocked: 3 }, stale: 3 });
    expect(h.batch).toContain("按顺序出 2 镜（线路 B）");
    expect(h.batch).toContain("失败会记下接着出");
    expect(h.batch).toContain("另有 3 镜没合成或过期");
    expect(toolbarHints({ ...base, batch: { ids: ["s1"], redo: true, blocked: 0 } }).batch).toContain("全部重出 1 镜");
    expect(h.synth).toContain("给整集 14 镜合成提示词");
    expect(h.synth).toContain("已合成的 14 镜会重新合成");
    expect(h.regen).toContain("会覆盖现在的 14 个镜头");
    expect(toolbarHints({ ...base, shots: 0 }).regen).toContain("由剧本生成镜头表");
  });
});

describe("自动修正写法", () => {
  /** 用户列的几条原文，都要能认出来 */
  it("认得出能由代码修掉的五类问题，别的不算", () => {
    const yes = [
      "第 2 段夹了英文「Pan」—— 全部写中文",
      "第 1 段写了「（或」—— 画面只能写一种，视频模型没法二选一",
      "第 1 段的声音或画面里又写了一遍台词「大逆不道！」—— 台词只写在台词栏，这里删掉",
      "说话人「林婉清」不是这一镜的出场人物（画外音要标出来）",
      "第 2 段写了「切至」—— 一镜内不切镜，要切就拆成下一镜",
    ];
    for (const message of yes) expect(isRepairable({ shot_num: 1, field: "segments", message })).toBe(true);
    expect(isRepairable({ shot_num: 1, field: "duration", message: "时长 3 秒，不在 4~15 秒之间" })).toBe(false);
    expect(isRepairable({ shot_num: 1, field: "source", message: "剧本里「殿外风起」没有任何一镜拍到" })).toBe(false);
  });
});

describe("合成提示词弹窗", () => {
  it("只合成没合成或过期的；整集就是全部，都按镜号排", () => {
    const syn = (n: number) => shot(n, { prompts: { b: { frame: "f", video: "v", synthesized_at: 1 } } });
    const shots = [syn(3), shot(1), syn(2)];
    const issues: ShotIssue[] = [{ shot_num: 3, field: "prompts", message: "要重新合成" }];
    expect(synthTargets(shots, "b", issues, true)).toEqual(["s1", "s3"]);
    expect(synthTargets(shots, "b", issues, false)).toEqual(["s1", "s2", "s3"]);
    // 线路 A 一镜都没合成
    expect(synthTargets(shots, "a", [], true)).toEqual(["s1", "s2", "s3"]);
  });

  /** 用户 2026-09-29：「合成其中一个，点另一个就变成不需要了」—— 问题清单只报当前线路的过期 */
  it("另一条线路过期也算：按线路问的过期名单说了算", () => {
    const both = (n: number) =>
      shot(n, { prompts: { a: { frame: "f", video: "v", synthesized_at: 1 }, b: { frame: "f", video: "v", synthesized_at: 1 } } });
    const shots = [both(1), both(2), both(3)];
    // 当前线路是 A、刚合成完：问题清单里一条 prompts 都没有，只有一条线路 B 正文的二选一提示
    const issues: ShotIssue[] = [{ shot_num: 3, field: "advice", message: "正文里有二选一" }];
    const stale = { a: [], b: ["s2"] };
    expect(synthTargets(shots, "b", issues, true, stale)).toEqual(["s2", "s3"]);
    // 二选一是线路 B 正文的事，不算在 A 上
    expect(synthTargets(shots, "a", issues, true, stale)).toEqual([]);
    // 其余提示（外貌偏薄、群演挂不全、没挂参考图……）合成成功也照样在，不算「要合成」（用户 10-07：显示 10 镜、其实 2 镜没过）
    const other: ShotIssue[] = [{ shot_num: 1, field: "advice", message: "「吴峥」的外貌只写了 31 个词" }, { shot_num: 2, field: "advice", message: "「邻居甲」有参考图，但这一镜没挂上" }];
    expect(synthTargets(shots, "b", other, true, { a: [], b: [] })).toEqual([]);
    // 名单还没取到：退回问题清单（只知道当前线路的）
    expect(synthTargets(shots, "b", [{ shot_num: 1, field: "prompts", message: "要重新合成" }], true, null)).toEqual(["s1"]);
  });

  it("弹窗打开时去问两条线路各自过期了哪些", () => {
    const src = readFileSync(fileURLToPath(new URL("./SynthesizeDialog.tsx", import.meta.url)), "utf8");
    expect(src).toContain("cinemaStalePrompts(project.id, episodeId)");
    expect(src).toContain("synthTargets(shots, route, issues, true, stale)");
  });

  /** 用户 2026-09-29：「直接分成左右两个分块，单独逻辑处理，左边 A，右边 B……不然现在有 confuse」 */
  it("左 A 右 B 各合各的：按钮带上自己的线路，合成不改出片线路", () => {
    const src = readFileSync(fileURLToPath(new URL("./SynthesizeDialog.tsx", import.meta.url)), "utf8");
    expect(src).toContain('(["a", "b"] as PromptRoute[]).map((r) => (');
    expect(src).toContain("onStart={(ids) => onStart(r, ids)}");
    // 以前合成哪条就把哪条存成出片线路 —— 这就是 confuse 的一半
    expect(src).not.toContain("cinemaSaveModelSettings");
    expect(src).toContain("出片用这条");
    // 一边在跑，另一边说「等它跑完」，而不是点了才报错
    expect(src).toContain("disabled={starting || mine || other || ids.length === 0}");
  });

  it("提示词卡片那一行：出片线路排前面，两条都齐了说都是最新的", () => {
    const both = (n: number) => shot(n, { prompts: { a: { frame: "f", video: "v", synthesized_at: 1 }, b: { frame: "f", video: "v", synthesized_at: 1 } } });
    const onlyB = (n: number) => shot(n, { prompts: { b: { frame: "f", video: "v", synthesized_at: 1 } } });
    expect(synthLine([both(1), both(2)], "b", { a: [], b: [] })).toBe("两条线路都是最新的");
    expect(synthLine([both(1), onlyB(2)], "b", { a: [], b: ["s1"] })).toBe("B 1 镜要重合成 · A 1 镜没合成");
    expect(synthLine([], "b", null)).toBe("");
  });

  it("认得出正在合成的是哪条线路", () => {
    expect(runningRoute({ status: "running", prompt: "线路 A（中文结构） · 3 镜" })).toBe("a");
    expect(runningRoute({ status: "running", prompt: "线路 B（H3 官方六段） · 整集" })).toBe("b");
    expect(runningRoute({ status: "done", prompt: "线路 B（H3 官方六段） · 整集" })).toBe(null);
    expect(runningRoute(null)).toBe(null);
  });
});

describe("加入出片队列", () => {
  /** 2026-10-05 用户：「有镜头正在出片无法点击重新出片，但是我不可能一直等着，需要可点击为加入列队」 */
  it("队列位置：只数还没轮到的；排队中时一个都没开始；不在跑的任务不算", () => {
    const job = (done: number, label: string, status: "running" | "done" = "running") => ({ status, targets: ["s1", "s2", "s3", "s4"], progress: { done, total: 4, label } });
    expect(queuePosition(job(1, "第 2 镜"), "s3")).toBe(1);
    expect(queuePosition(job(1, "第 2 镜"), "s4")).toBe(2);
    expect(queuePosition(job(1, "第 2 镜"), "s2")).toBe(null); // 正在出，不算排队
    expect(queuePosition(job(1, "第 2 镜"), "s1")).toBe(null); // 出过了
    expect(queuePosition(job(0, "排队中 · 等前一镜出完"), "s1")).toBe(1);
    expect(queuePosition(job(1, "第 2 镜", "done"), "s3")).toBe(null);
    expect(queuePosition(null, "s3")).toBe(null);
  });

  it("有片在出时按钮不灰、改成加入队列；出片走队列接口", () => {
    const detail = readFileSync(fileURLToPath(new URL("./ShotDetail.tsx", import.meta.url)), "utf8");
    expect(detail).toContain('const mine = rendering ? null : renderBlocker(shot, route, issues, false);');
    expect(detail).toContain('"加入队列"');
    expect(detail).toContain("已在队列 · 第 ${queued} 个");
    const step = readFileSync(fileURLToPath(new URL("../steps/StoryboardStep.tsx", import.meta.url)), "utf8");
    expect(step).toContain("cinemaQueueShotVideos(project.id, episode.id, [shotId])");
    expect(step).toContain("queued={queuePosition(batchJob, selected.id)}");
  });
});

describe("下一步该点哪个", () => {
  const t = { shots: 14, synthesized: 14, stale: 0, canRender: true, busy: false };
  /** 用户反馈：第一次用不知道先点哪个；原来唯一的蓝色按钮恰恰是会覆盖整张表的「重新生成镜头」。 */
  it("没镜头 → 生成；有没合成或过期 → 合成；都好了 → 出片", () => {
    expect(nextStep({ ...t, shots: 0, synthesized: 0 })).toBe("shots");
    expect(nextStep({ ...t, synthesized: 10 })).toBe("synth");
    expect(nextStep({ ...t, stale: 2 })).toBe("synth");
    expect(nextStep(t)).toBe("render");
  });

  /** 2026-10-04 用户：「先出了 B 的提示词，再出首帧，不还是要重新出 B 的提示词吗」—— 首帧排在提示词前面 */
  it("有没首帧也没合成的 → 先出首帧（出完自动合成），不先合成", () => {
    expect(nextStep({ ...t, synthesized: 0, framesFirst: 14 })).toBe("frames");
    expect(nextStep({ ...t, synthesized: 10, framesFirst: 0 })).toBe("synth");
    expect(nextStep({ ...t, shots: 0, synthesized: 0, framesFirst: 0 })).toBe("shots");
    const done = shot(1, { prompts: { b: { frame: "f", video: "v", synthesized_at: 1 } } });
    const bare = shot(2);
    const framed = shot(3, { keyframes: { first: true, last: false } });
    expect(framesFirst([done, bare, framed], "b").map((s) => s.shot_num)).toEqual([2]);
  });

  it("有任务在跑、或者没有能出的镜头时不高亮", () => {
    expect(nextStep({ ...t, busy: true })).toBe(null);
    expect(nextStep({ ...t, canRender: false })).toBe(null);
  });
});

describe("重新生成这一场", () => {
  it("漏拍、台词没进任何一镜才需要；按问题所在镜头的场号", () => {
    const miss = { shot_num: 12, field: "source", message: "剧本里「殿外风起。」没有任何一镜拍到 —— 要么并进某一镜" };
    const line = { shot_num: 3, field: "dialogue", message: "剧本台词「我还能撑」没有写进这一场任何一镜的台词栏" };
    const fake = { shot_num: 7, field: "source", message: "原文「接上镜」不是逐字出自这一场剧本" };
    expect(needsSceneRegen(miss)).toBe(true);
    expect(needsSceneRegen(line)).toBe(true);
    expect(needsSceneRegen(fake)).toBe(false);
    const shots = [shot(3, { scene_no: 1 }), shot(7, { scene_no: 2 }), shot(12, { scene_no: 4 })];
    expect(scenesNeedingRegen(shots, [miss, line, fake])).toEqual([1, 4]);
  });
});

describe("单镜形象", () => {
  const lv = (id: string) => ({ id }) as AssetView["look_views"][number];
  const shen = { id: "a", look_views: [lv("main"), lv("look_tang"), lv("look_ball")] };
  it("单镜覆盖 → 项目默认 → 主形象，指着删掉的形象跳过", () => {
    expect(effectiveLook({}, undefined, shen)).toEqual({ id: "main", overridden: false });
    expect(effectiveLook({}, { a: "look_tang" }, shen)).toEqual({ id: "look_tang", overridden: false });
    expect(effectiveLook({ look_overrides: { a: "look_ball" } }, { a: "look_tang" }, shen)).toEqual({ id: "look_ball", overridden: true });
    expect(effectiveLook({ look_overrides: { a: "look_gone" } }, { a: "look_tang" }, shen)).toEqual({ id: "look_tang", overridden: false });
    expect(effectiveLook({}, { a: "look_gone" }, shen)).toEqual({ id: "main", overridden: false });
  });
});

/**
 * **「数这一镜出过几次片」这条判据只能有一个所有者。**
 *
 * 四个地方要数（批量出片、单镜按钮文案、出片弹窗、镜头卡角标）。
 * 各写一遍的后果 2026-09-20 已经出过一次雏形：加了画布回填之后，
 * 漏掉任何一处，那一处就会把回填的片当成「这条线路出过片」——
 * 表现是批量出片跳过这一镜、或者按钮显示「重新出片」，**都不报错**。
 */
describe("数 take 的判据只有一个所有者", () => {
  const files = [
    "./RenderOptionsDialog.tsx",
    "./ShotDetail.tsx",
    "../steps/StoryboardStep.tsx",
    "./storyboardView.ts",
  ];

  it("没有人绕过 renderTakes 自己数", () => {
    for (const f of files) {
      const src = readFileSync(new URL(f, import.meta.url), "utf8");
      // 允许 renderTakes 自己的实现，别处不准再写一遍这个过滤
      const body = f.endsWith("storyboardView.ts")
        ? src.replace(/export function renderTakes[\s\S]*?\n}/, "")
        : src;
      expect(body, `${f} 自己数了一遍 take`).not.toMatch(/takes[\s\S]{0,40}t\.route === route/);
    }
  });

  it("renderTakes 把画布回填的排除在外", () => {
    const shot = { takes: [
      { route: "b", path: "/出片.mp4", origin: "render" },
      { route: "b", path: "/画布.mp4", origin: "canvas" },
      { route: "b", path: "/老数据.mp4" },
    ] } as unknown as StoryboardShot;
    const got = renderTakes(shot, "b" as PromptRoute).map((t) => t.path);
    // 老数据没有 origin 字段 —— 当 render，不然历史全部作废
    expect(got).toEqual(["/出片.mp4", "/老数据.mp4"]);
  });
});

/** 用户 2026-09-27：批量出片是全部分镜，要加「按场出片」，可以复选第几场、多场一起出。 */
describe("按场出片", () => {
  const syn = (n: number, scene: number, extra: Partial<StoryboardShot> = {}) =>
    shot(n, { scene_no: scene, scene_name: `场景：第${scene}处`, prompts: { b: { frame: "f", video: "v", synthesized_at: 1 } }, ...extra });
  const take = { route: "b" as const, path: "b.mp4", created_at: 1, seed: 1, pictures: 1 };
  const shots = [syn(1, 1), syn(2, 1, { takes: [take] }), syn(3, 2), shot(4, { scene_no: 2 }), syn(5, 3)];

  it("场清单：每场几镜、已出几镜、几镜能出；场名去掉「场景：」", () => {
    expect(sceneRows(shots, "b", [])).toEqual([
      { no: 1, name: "第1处", total: 2, rendered: 1, ready: 2 },
      { no: 2, name: "第2处", total: 2, rendered: 0, ready: 1 },
      { no: 3, name: "第3处", total: 1, rendered: 0, ready: 1 },
    ]);
  });

  it("只出勾选的场，其余规则照批量走（没出过的优先、没合成的跳过）", () => {
    expect(renderTargetsFor({ kind: "scenes", sceneNos: [1, 3] }, shots, [], "b").ids).toEqual(["s1", "s5"]);
    expect(renderTargetsFor({ kind: "scenes", sceneNos: [2] }, shots, [], "b")).toMatchObject({ ids: ["s3"], blocked: 1 });
    expect(renderTargetsFor({ kind: "scenes", sceneNos: [] }, shots, [], "b").ids).toEqual([]);
  });
});

/**
 * 2026-09-27 按场出第 2 场：第 3 镜过期，4~7 镜都「接上一镜」，全排进去各报一条失败。
 * 现在接不上的不排，说清楚卡在谁身上。
 */
describe("批量出片：接上一镜的链", () => {
  const b = (n: number, extra: Partial<StoryboardShot> = {}) =>
    shot(n, { scene_no: n <= 2 ? 1 : 2, prompts: { b: { frame: "f", video: "v", synthesized_at: 1 } }, ...extra } as Partial<StoryboardShot>);
  const ref = { continuity: "ref_video" } as Partial<StoryboardShot>;
  const stale3: ShotIssue[] = [{ shot_num: 3, field: "prompts", message: "改过字段" }];

  it("上一镜出不了，接它的一路跟着摘掉", () => {
    const shots = [b(3), b(4, ref), b(5, ref)];
    const p = batchTargets(shots, "b", stale3);
    expect(p.ids).toEqual([]);
    expect(p.blocked).toBe(1);
    expect(p.chained).toEqual(["第 4 镜接第 3 镜，第 3 镜出不了（没合成或要重新合成）", "第 5 镜接第 4 镜，第 4 镜也接不上"]);
  });

  it("上一镜这一批会先出、或已经出过片，就接得上；挂了首帧的不接", () => {
    expect(batchTargets([b(3), b(4, ref)], "b", []).ids).toEqual(["s3", "s4"]);
    const done3 = b(3, { takes: [{ route: "b", path: "x.mp4", created_at: 1, origin: "render" }] } as Partial<StoryboardShot>);
    expect(batchTargets([done3, b(4, ref)], "b", stale3).ids).toEqual(["s4"]);
    expect(batchTargets([b(3), b(4, { ...ref, keyframes: { first: true, last: false } })], "b", stale3).ids).toEqual(["s4"]);
  });

  it("按场出：上一镜在没勾的场里、又没出过片", () => {
    const all = [b(2), b(3, ref)];
    const p = renderTargetsFor({ kind: "scenes", sceneNos: [2] }, all, [], "b");
    expect(p.ids).toEqual([]);
    expect(p.chained).toEqual(["第 3 镜接第 2 镜，第 2 镜还没出片（不在这次要出的镜头里）"]);
  });
});


/**
 * **组装页的格子和「合成一集」排的是同一份**（Rust `cinema::episode_segments`）。
 * 2026-10-01 用户截图：第 5 场整段出过，组装页 22~24 镜一直显示「未出片」。
 */
describe("组装页：整段出的场放一格", () => {
  const sh = (id: string, shot_num: number, scene_no: number) => ({ id, shot_num, scene_no });
  const r = (scene_no: number, route: "a" | "b", created_at: number) => ({ scene_no, route, created_at, path: `/lt/${scene_no}-${route}-${created_at}.mp4` });
  type S = { id: string; shot_num: number; scene_no?: number };
  const shape = (tiles: AssemblyTile<S, unknown>[]) =>
    tiles.map((t) => (t.kind === "shot" ? t.shot.id : `场${t.sceneNo}[${t.shots.map((s) => s.id).join(",")}]`));

  it("放在这一场第一次出现的位置，成员按镜号排，其余镜头不再各占一格", () => {
    // 第 3 镜被挪到了最前面（组装页能调顺序）
    const shots = [sh("c", 3, 2), sh("a", 1, 1), sh("b", 2, 2), sh("d", 4, 3)];
    expect(shape(assemblyTiles(shots, [2], [r(2, "b", 1)], "b"))).toEqual(["场2[b,c]", "a", "d"]);
  });

  it("当前线路没出过、另一条出过：照样用（和后端一致，总比空着强）；新的优先", () => {
    expect(longTakeClip([2], [r(2, "a", 1)], 2, "b")?.route).toBe("a");
    expect(longTakeClip([2], [r(2, "a", 5), r(2, "b", 1)], 2, "b")?.route).toBe("b");
    expect(longTakeClip([2], [r(2, "b", 1), r(2, "b", 9)], 2, "b")?.created_at).toBe(9);
  });

  it("这一场改回分镜了：出过的整段片不再用，各镜各占一格", () => {
    const shots = [sh("a", 1, 1), sh("b", 2, 2), sh("c", 3, 2)];
    expect(shape(assemblyTiles(shots, [], [r(2, "b", 1)], "b"))).toEqual(["a", "b", "c"]);
  });

  it("设成整段出但还没出过：各镜各占一格（显示未出片）", () => {
    const shots = [sh("b", 2, 2), sh("c", 3, 2)];
    expect(shape(assemblyTiles(shots, [2], [], "b"))).toEqual(["b", "c"]);
  });

  it("没有场号的老镜头不会被当成第 0 场的长镜头", () => {
    const shots = [{ id: "x", shot_num: 1, scene_no: undefined }];
    expect(shape(assemblyTiles(shots, [0], [r(0, "b", 1)], "b"))).toEqual(["x"]);
  });
});
