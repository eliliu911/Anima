import { describe, expect, it } from "vitest";
import type { Job, JobKind, JobStatus } from "../../../api/jobs";
import {
  isShotBusy,
  jobTitle,
  jobsKey,
  lastJobs,
  promptsState,
  rememberJobs,
  resetJobsMemory,
  runningKind,
  settledShots,
  shotTableState,
} from "./shotBusy";

const job = (id: string, shot: string, kind: JobKind, status: JobStatus): Job => ({
  id, kind, status, project_id: "p", episode_id: "e",
  shot_id: shot, shot_num: 1, prompt: "", created_at: 0,
});

describe("这一镜在跑什么", () => {
  it("按镜头分，不会串", () => {
    const jobs = [job("j1", "shot_1", "video", "running"), job("j2", "shot_2", "frame", "running")];
    expect(runningKind(jobs, "shot_1")).toBe("video");
    expect(runningKind(jobs, "shot_2")).toBe("frame");
    expect(runningKind(jobs, "shot_3")).toBeNull();
  });

  it("跑完的不算在跑", () => {
    const jobs = [job("j1", "shot_1", "video", "done"), job("j2", "shot_1", "frame", "failed")];
    expect(runningKind(jobs, "shot_1")).toBeNull();
    expect(isShotBusy(jobs, "shot_1")).toBe(false);
  });

  /** 出片慢得多，是用户真正在等的那个。 */
  it("首帧和出片同时在跑时显示出片", () => {
    const jobs = [job("j1", "shot_1", "frame", "running"), job("j2", "shot_1", "video", "running")];
    expect(runningKind(jobs, "shot_1")).toBe("video");
  });

  it("空表就是不忙", () => {
    expect(runningKind([], "shot_1")).toBeNull();
    expect(isShotBusy([], "shot_1")).toBe(false);
  });
});

describe("哪几镜刚落地", () => {
  it("running → done 算落地", () => {
    const prev = [job("j1", "shot_1", "video", "running")];
    const next = [job("j1", "shot_1", "video", "done")];
    expect(settledShots(prev, next)).toEqual(["shot_1"]);
  });

  /** **失败也要刷。** 失败原文在任务里，界面要能显示出来。 */
  it("running → failed 也算落地", () => {
    const prev = [job("j1", "shot_1", "video", "running")];
    const next = [job("j1", "shot_1", "video", "failed")];
    expect(settledShots(prev, next)).toEqual(["shot_1"]);
  });

  it("一直在跑的不算", () => {
    const prev = [job("j1", "shot_1", "video", "running")];
    expect(settledShots(prev, prev)).toEqual([]);
  });

  /**
   * **本来就没在跑的不算。** 首次拉到一批历史任务（全是 done）时，
   * 不该把它们当成「刚刚落地」而触发一次刷新。
   */
  it("第一次拉到一堆已完成的不触发", () => {
    const done = [job("j1", "shot_1", "video", "done"), job("j2", "shot_2", "frame", "done")];
    expect(settledShots([], done)).toEqual([]);
  });

  it("同一镜的多个任务只报一次", () => {
    const prev = [job("j1", "shot_1", "frame", "running"), job("j2", "shot_1", "video", "running")];
    const next = [job("j1", "shot_1", "frame", "done"), job("j2", "shot_1", "video", "done")];
    expect(settledShots(prev, next)).toEqual(["shot_1"]);
  });

  /** 任务从列表里被清掉（QUEUE 清空）不算落地 —— 那不是产物到位。 */
  /**
   * **「本来在跑」这个前提不能省。**
   *
   * 只测「prev 全空」的话，`wasRunning.size === 0` 那个提前返回会把
   * 漏判盖住。这里让 prev 里有一条在跑的，同时 next 里带一条**早就完成**
   * 的别的任务 —— 少了那个判断，它会被当成刚落地而触发一次多余的刷新。
   */
  it("别人早就完成的任务不算这一轮落地", () => {
    const prev = [job("j1", "shot_1", "video", "running")];
    const next = [
      job("j1", "shot_1", "video", "running"),
      job("j9", "shot_9", "frame", "done"),
    ];
    expect(settledShots(prev, next)).toEqual([]);
  });

  it("任务被清掉不算落地", () => {
    const prev = [job("j1", "shot_1", "video", "running")];
    expect(settledShots(prev, [])).toEqual([]);
  });
});

// ── 接线 ──
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const strip = (t: string) =>
  t.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/.*$/gm, "");
const src = (rel: string) =>
  strip(readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8"));

/**
 * **按钮状态和防重复提交都要认服务端那份。**
 *
 * 原来只有组件本地 state：切页再回来组件重挂载，按钮回到「出视频」，
 * 看着像没在跑；而防重复的 ref 也一起丢了，再点一次会排第二个同样的
 * 任务 —— 视频串行跑，那是白等十几分钟。
 */
describe("分镜页接线", () => {
  const STEP = src("../steps/StoryboardStep.tsx");

  it("任务落地自动刷新分集", () => {
    const i = STEP.indexOf("useJobs(");
    expect(i).toBeGreaterThan(0);
    expect(STEP.slice(i, i + 220)).toContain("reload()");
  });

  /** 回调进依赖数组会把轮询循环重启，重启就丢掉「上一轮是什么」。 */
  it("落地回调放 ref，不进依赖数组", () => {
    const panel = src("./QueuePanel.tsx");
    expect(panel).toContain("settled.current = onSettled");
    // **要钉住它真的被调用**，不是「文件里有这个符号」——
    // 把调用整行删掉的话，上面那条依然成立
    expect(panel, "算出来了却没人用").toContain("settled.current?.(done)");
  });

  /**
   * **上一轮的快照不能放 ref。**
   *
   * ref 随卸载一起没，于是切走再回来时「上一轮」是空的，
   * `settledShots` 因为 `wasRunning.size === 0` 直接返回空 ——
   * 在别处跑完的任务永远检测不到（2026-09-18 现场抓到）。
   *
   * 而界面自己就在引导用户离开：「可以先去别的步骤」。
   */
  it("上一轮的快照存在模块级，**不是 ref**", () => {
    const panel = src("./QueuePanel.tsx");
    expect(panel).toContain("rememberJobs(key, next)");
    expect(panel).toContain("settledShots(lastJobs(key), next)");
    expect(panel, "又退回 ref 了 —— 切走再回来就检测不到落地").not.toContain("prev.current");
  });
});

describe("生成镜头表的状态从任务登记表推", () => {
  const table = (id: string, status: JobStatus, extra: Partial<Job> = {}): Job => ({
    ...job(id, "", "shot_table", status),
    shot_num: 0,
    ...extra,
  });

  it("没跑过就什么都没有", () => {
    const s = shotTableState([job("j1", "shot_1", "video", "running")]);
    expect(s).toEqual({ job: null, generating: false, progress: null, notes: [], remarks: [] });
  });

  /** 列表新的在前：上一次失败、这次正在跑，要显示在跑。 */
  it("认最近一次，不认更早的", () => {
    const jobs = [
      table("new", "running", { progress: { done: 1, total: 4, label: "金銮殿" } }),
      table("old", "failed", { error: "LM Studio 没起" }),
    ];
    const s = shotTableState(jobs);
    expect(s.job?.id).toBe("new");
    expect(s.generating).toBe(true);
    expect(s.progress).toEqual({ done: 1, total: 4, label: "金銮殿" });
  });

  /**
   * **上游重跑过，这一轮的提示就不再显示。**
   *
   * 用户 2026-09-20：「每次重新生成都会提示之前的错误提示……在点击重新
   * 生成、重新合成的时候就先清空错误提示状态」。重新生成镜头表之后，
   * 上一轮「合成有 4 镜没过」说的还是旧镜号，甚至那几镜已经不存在了。
   *
   * 后端在登记新任务时打 `superseded`（`jobs::supersede_downstream`），
   * **记录本身留着** —— 队列里还要能查「上次为什么失败」。
   */
  it("上游重跑过的那一轮，备注不再显示", () => {
    const stale = shotTableState([table("t", "done", { notes: ["旧镜号"], remarks: ["也是旧的"], superseded: true })]);
    expect(stale.notes).toEqual([]);
    expect(stale.remarks).toEqual([]);
    // 记录本身还在，队列里查得到
    expect(stale.job?.notes).toEqual(["旧镜号"]);
  });

  it("备注只在成功那次显示，进度只在跑的时候显示", () => {
    const done = shotTableState([table("t", "done", { notes: ["现代办公室没有图"], progress: { done: 4, total: 4, label: "" } })]);
    expect(done.notes).toEqual(["现代办公室没有图"]);
    expect(done.progress).toBeNull();
    expect(done.generating).toBe(false);
    const failed = shotTableState([table("t", "failed", { notes: ["不该出现"] })]);
    expect(failed.notes).toEqual([]);
    expect(failed.job?.status).toBe("failed");
  });

  /** 落地时靠 settledShots 非空触发刷新 —— 镜头表的任务没有镜头 id，也得算进去。 */
  it("镜头表生成完也会触发刷新", () => {
    expect(settledShots([table("t", "running")], [table("t", "done")])).toEqual([""]);
  });
});

describe("队列里的任务标题", () => {
  it("镜头表按场报进度，没有镜号", () => {
    const running: Job = { ...job("t", "", "shot_table", "running"), progress: { done: 1, total: 4, label: "" } };
    expect(jobTitle(running)).toBe("生成镜头 · 第 2/4 场");
    expect(jobTitle({ ...running, progress: { done: 4, total: 4, label: "" } })).toBe("生成镜头 · 第 4/4 场");
    expect(jobTitle({ ...running, progress: undefined })).toBe("生成镜头");
    expect(jobTitle({ ...running, status: "done" })).toBe("生成镜头");
  });

  it("出片和首帧照旧", () => {
    expect(jobTitle({ ...job("v", "s", "video", "running"), shot_num: 3 })).toBe("第 3 镜 · 出片");
    // 长镜头按场登记、没有镜号 —— 不能显示成「第 ? 镜」
    expect(jobTitle({ ...job("v", "scene_5", "video", "running"), shot_num: 0 })).toBe("第 5 场 · 长镜头");
    expect(jobTitle({ ...job("f", "s", "frame", "done"), shot_num: 0 })).toBe("第 ? 镜 · 首帧");
  });
});

describe("合成提示词任务", () => {
  const prompts = (id: string, status: JobStatus, extra: Partial<Job> = {}): Job => ({ ...job(id, "", "prompts", status), shot_num: 0, ...extra });

  it("和生成镜头表各认各的，不串", () => {
    const jobs = [prompts("p", "running", { progress: { done: 0, total: 3, label: "第 1 镜" } }), job("t", "", "shot_table", "done")];
    expect(promptsState(jobs).job?.id).toBe("p");
    expect(promptsState(jobs).generating).toBe(true);
    expect(shotTableState(jobs).job?.id).toBe("t");
    expect(shotTableState(jobs).generating).toBe(false);
  });

  it("队列标题按个报进度", () => {
    expect(jobTitle(prompts("p", "running", { progress: { done: 1, total: 3, label: "" } }))).toBe("合成提示词 · 第 2/3 个");
    expect(jobTitle(prompts("p", "done"))).toBe("合成提示词");
  });
});

describe("任务快照活过卸载", () => {
  /**
   * **2026-09-18 现场抓到的 bug。**
   *
   * 分镜页点「生成镜头」→ 界面说「可以先去别的步骤」→ 照做 →
   * 任务在后台跑完并写进了 episode.json（盘上确实有 6 个镜头）→
   * 回来一看「还没有镜头」，而任务队列里明明写着已完成。
   *
   * 根因：快照存在组件的 ref 里，切走一次就清零，回来时
   * `settledShots(prev=[], next)` 因为 `wasRunning.size === 0` 直接返回空。
   */
  it("**切走再回来，在别处跑完的任务要检测得到**", () => {
    resetJobsMemory();
    const key = jobsKey("storyboard", "p", "e");
    const running = [job("j1", "", "shot_table", "running")];
    const done = [job("j1", "", "shot_table", "done")];

    // 第一次挂载：看到它在跑，记下来
    expect(settledShots(lastJobs(key), running)).toEqual([]);
    rememberJobs(key, running);

    // 切走（组件卸载，什么都不做）—— 任务在后台跑完

    // 回来重挂：**这里原来永远返回空**
    expect(settledShots(lastJobs(key), done).length).toBeGreaterThan(0);
  });

  it("第一次进来不该误报落地 —— 本来就没有「上一轮」", () => {
    resetJobsMemory();
    const key = jobsKey("storyboard", "p", "e");
    expect(settledShots(lastJobs(key), [job("j1", "", "shot_table", "done")])).toEqual([]);
  });

  /**
   * **按项目/分集分键。** 不分的话切换分集会把上一集的快照当成本集的上一轮：
   * 「上一集刚跑完的任务」被算成本集的落地（多一次无谓重读），反过来也会漏。
   */
  it("不同分集各存各的快照", () => {
    resetJobsMemory();
    const a = jobsKey("storyboard", "p", "e1");
    const b = jobsKey("storyboard", "p", "e2");
    rememberJobs(a, [job("j1", "", "shot_table", "running")]);
    expect(lastJobs(b)).toEqual([]);
    expect(lastJobs(a)).toHaveLength(1);
  });

  it("项目或分集为空时也各有各的键，不共用一份", () => {
    expect(jobsKey("s", "", "")).not.toBe(jobsKey("s", "p", ""));
    expect(jobsKey("s", "p", "")).not.toBe(jobsKey("s", "", "p"));
  });

  /**
   * **不同消费方各存各的。** 三个步骤都在调 `useJobs`，同一时刻只挂一个 ——
   * 共用快照的话，你切到哪一页哪一页就把这次落地消费掉了
   * （`AssetsStep` 连回调都没传，只是静悄悄推进快照），
   * 等你回分镜页快照已经是 done，bug 原样还在。
   */
  it("不同调用点各存各的快照", () => {
    resetJobsMemory();
    const sb = jobsKey("storyboard", "p", "e");
    const assets = jobsKey("assets", "p", "e");
    const running = [job("j1", "", "shot_table", "running")];
    const done = [job("j1", "", "shot_table", "done")];

    // 分镜页看到它在跑
    rememberJobs(sb, running);
    // 切到资产页：**那一页也在轮询**，它看到并记下了 done
    rememberJobs(assets, done);

    // 回分镜页：它的快照还停在「在跑」，所以落地检测得到
    expect(settledShots(lastJobs(sb), done).length).toBeGreaterThan(0);
  });

  /** 记下的是副本 —— 存引用的话调用方之后改了数组，快照跟着变，比较就失真 */
  it("记下的是副本，不是引用", () => {
    resetJobsMemory();
    const key = jobsKey("storyboard", "p", "e");
    const list = [job("j1", "", "shot_table", "running")];
    rememberJobs(key, list);
    list.push(job("j2", "", "video", "running"));
    expect(lastJobs(key)).toHaveLength(1);
  });
});

describe("三个调用点的名字必须互不相同", () => {
  /**
   * 共用名字 = 共用快照 = 「你切到哪一页哪一页就把落地消费掉」，
   * 也就是 2026-09-18 那个 bug 原样回来。而这件事**不报错**，
   * 只表现为「回来一看什么都没有」，所以得有测试盯着。
   */
  it("assets / assembly / storyboard 各是各的", () => {
    const names = ["../steps/AssetsStep.tsx", "../steps/AssemblyStep.tsx", "../steps/StoryboardStep.tsx"].map((f) => {
      const t = src(f);
      const i = t.indexOf("useJobs(");
      expect(i, `${f} 里没有 useJobs`).toBeGreaterThan(0);
      // **取到行尾**，不是取到第一个 `)` —— 回调里的 `reload()` 会提前截断
      const call = t.slice(i, t.indexOf("\n", i));
      const m = call.match(/"([a-z_]+)"\s*\)/);
      expect(m, `${f} 的 useJobs 没给调用点名字：${call}`).not.toBeNull();
      return m![1];
    });
    expect(new Set(names).size, `重名了：${names.join(" / ")}`).toBe(names.length);
  });
});
