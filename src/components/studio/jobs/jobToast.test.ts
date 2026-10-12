import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import type { Job, JobKind, JobStatus } from "../../../api/jobs";
import { jobToastId, toastForJob, videoProgressLine } from "./jobToast";
import { batchWaitingOn, currentBatchChild } from "./jobBuckets";

const job = (kind: JobKind, status: JobStatus, extra: Partial<Job> = {}): Job => ({
  id: "j1", kind, status, project_id: "p", episode_id: "e",
  shot_id: "s", shot_num: 3, prompt: "", created_at: 0, ...extra,
});

describe("后台任务 → 通知条", () => {
  it("id 按任务 id 固定：在跑和完成是同一条", () => {
    expect(jobToastId(job("video", "running"))).toBe(jobToastId(job("video", "done")));
    expect(jobToastId({ id: "a" })).not.toBe(jobToastId({ id: "b" }));
  });

  it("状态映射成对应的类型", () => {
    for (const kind of ["frame", "video", "shot_table"] as JobKind[]) {
      expect(toastForJob(job(kind, "running")).kind).toBe("progress");
      expect(toastForJob(job(kind, "done")).kind).toBe("success");
      expect(toastForJob(job(kind, "failed")).kind).toBe("error");
    }
  });

  it("出图出片带镜号，分得清是哪一镜、首帧还是视频", () => {
    expect(toastForJob(job("video", "done")).title).toBe("第 3 镜 · 出片好了");
    expect(toastForJob(job("frame", "running")).title).toBe("第 3 镜 · 出首帧中");
  });

  /** 用户 2026-10-07：进度前面显示实际链路，而且是后端返回的（`Job.pipeline`），不是前端按设置推的 */
  it("出片进度前面显示后端返回的链路；没返回就不加", () => {
    const progress = { done: 3, total: 5, label: "第 3/5 步 · 60%" };
    const run = (extra: Partial<Job>) => toastForJob(job("video", "running", { progress, ...extra })).body;
    expect(run({ pipeline: "FL2VA 首帧 · LightX2V 4 次" })).toBe("【FL2VA 首帧 · LightX2V 4 次】第 3/5 步 · 60%");
    expect(run({})).toBe("第 3/5 步 · 60%");
    // 批量：当前那一镜的链路接到批量那条上
    const batch = job("video_batch", "running", { id: "b", progress: { done: 0, total: 4, label: "" } });
    const cur = job("video", "running", { progress, pipeline: "Ref2VA 兼容（FL2VA 权重）· Larry v4 6 次" });
    expect(toastForJob(batch, { current: cur }).body).toContain("【Ref2VA 兼容（FL2VA 权重）· Larry v4 6 次】第 3/5 步");
    // 前端没有自己拼链路名的地方
    const src = readFileSync(fileURLToPath(new URL("./jobToast.ts", import.meta.url)), "utf8");
    expect(src).not.toMatch(/"FL2VA|"Ref2VA/);
  });

  /** 长镜头按场登记，没有镜号 —— 2026-09-30 失败提示显示成「第 ? 镜」，看不出是哪个任务 */
  it("长镜头写第几场，不写第 ? 镜", () => {
    const lt = (status: JobStatus) => toastForJob(job("video", status, { shot_id: "scene_5", shot_num: 0 }));
    expect(lt("failed").title).toBe("第 5 场 · 长镜头 · 出片失败");
    expect(lt("running").title).toBe("第 5 场 · 长镜头 · 出片中");
    expect(lt("running").body).toContain("好几个小时");
  });

  /** 「ComfyUI 没起」用户自己能修，概括成「失败」就修不了。 */
  it("失败原文照抄", () => {
    const e = "ComfyUI 没有响应：connection refused";
    expect(toastForJob(job("video", "failed", { error: e })).body).toBe(e);
    expect(toastForJob(job("shot_table", "failed", { error: e })).body).toBe(e);
  });

  it("合成一集：按镜报进度，拼到最后说在拼接，超分时提醒慢", () => {
    const t = toastForJob(job("stitch", "running", { prompt: "合成一集 · 超分 1080P", progress: { done: 2, total: 14, label: "第 3 镜" } }));
    expect(t.title).toBe("正在合成一集 · 超分 1080P");
    expect(t.body).toContain("第 3/14 镜");
    expect(t.body).toContain("超分一镜要一两分钟");
    expect(toastForJob(job("stitch", "running", { progress: { done: 14, total: 14, label: "拼接" } })).body).toContain("正在拼接");
    expect(toastForJob(job("stitch", "done")).title).toBe("成片合成好了");
    expect(toastForJob(job("stitch", "failed", { error: "第 3 镜超分失败：x" })).body).toBe("第 3 镜超分失败：x");
  });

  it("导出到 FCP：导完说导到哪、哪几镜留了空；失败原文照抄", () => {
    const running = toastForJob(job("fcp_export", "running", { prompt: "导出到 Final Cut Pro · 超分 1080P", progress: { done: 4, total: 35, label: "第 5 镜" } }));
    expect(running.body).toContain("第 5/35 镜");
    expect(running.body).toContain("超分一镜要一两分钟");
    const done = toastForJob(
      job("fcp_export", "done", { summary: "34 镜 → /Volumes/素材盘/S01E01_第三次", remarks: ["第 7 镜还没出片，时间线上按分镜秒数留了空、带待办标记"] }),
    );
    expect(done.body).toContain("/Volumes/素材盘/S01E01_第三次");
    expect(done.body).toContain("第 7 镜还没出片");
    expect(toastForJob(job("fcp_export", "failed", { error: "导出位置找不到：/Volumes/x —— 外置硬盘是不是没挂上？" })).body).toContain("外置硬盘");
  });

  it("资产出图：按张报进度；有没出来的名字和原因进卡片", () => {
    const run = toastForJob(job("asset_images", "running", { prompt: "资产出图 · 2 个角色形象", progress: { done: 1, total: 3, label: "林婉清 · 现代" } }));
    expect(run.title).toBe("资产出图 · 2 个角色形象");
    expect(run.body).toContain("第 2/3 张 · 林婉清 · 现代");
    expect(toastForJob(job("asset_images", "done", { summary: "3 张设定图都出好了" })).kind).toBe("success");
    const partial = toastForJob(job("asset_images", "done", { summary: "出了 2 张，1 张没出来", notes: ["皇帝：ComfyUI 没开"] }));
    expect(partial.kind).toBe("error");
    expect(partial.body).toContain("皇帝：ComfyUI 没开");
  });

  it("批量出片：有跳过或失败的不能报「全部完成」", () => {
    expect(toastForJob(job("video_batch", "running", { progress: { done: 2, total: 14, label: "第 3 镜" } })).body).toContain("第 3/14 个 · 第 3 镜");
    expect(toastForJob(job("video_batch", "done", { summary: "出片 14/14 镜" })).title).toBe("批量出片全部完成");
    const partial = toastForJob(job("video_batch", "done", { summary: "出片 12/14 镜", notes: ["a", "b"] }));
    expect(partial.kind).toBe("error");
    expect(partial.body).toContain("2 镜跳过或失败");
  });

  it("批量出首帧：有自己的措辞，跑完提醒要重新合成", () => {
    expect(toastForJob(job("frame_batch", "running", { progress: { done: 0, total: 5, label: "第 1 镜" } })).title).toBe("正在批量出首帧");
    const ok = toastForJob(job("frame_batch", "done", { summary: "出首帧 5/5 镜" }));
    expect(ok.title).toBe("首帧全部出完");
    expect(ok.body).toContain("重新合成");
    expect(toastForJob(job("frame_batch", "done", { notes: ["第 2 镜：x"] })).kind).toBe("error");
  });

  it("合成提示词：有镜头没通过时算失败提示，不能报「好了」", () => {
    expect(toastForJob(job("prompts", "running", { progress: { done: 0, total: 12, label: "第 1 镜" } })).body).toContain("第 1/12 个 · 第 1 镜");
    const ok = toastForJob(job("prompts", "done", { summary: "线路 B：合成 12/12 镜" }));
    expect(ok).toMatchObject({ kind: "success", title: "提示词合成好了", body: "线路 B：合成 12/12 镜" });
    const partial = toastForJob(job("prompts", "done", { summary: "线路 B：合成 10/12 镜", notes: ["第 3 镜…", "第 7 镜…"] }));
    expect(partial.kind).toBe("error");
    expect(partial.body).toContain("2 镜没通过检查");
  });

  it("生成镜头按场报进度，完成时带结果和备注数", () => {
    const running = toastForJob(job("shot_table", "running", { progress: { done: 1, total: 4, label: "金銮殿" } }));
    expect(running.body).toContain("第 2/4 场 · 金銮殿");
    expect(toastForJob(job("shot_table", "running")).body).toContain("正在读剧本、分场");
    const done = toastForJob(job("shot_table", "done", { summary: "4 场 10 个镜头，0 个问题待改", notes: ["a", "b"] }));
    expect(done.body).toContain("4 场 10 个镜头");
    expect(done.body).toContain("另有 2 条备注");
    expect(toastForJob(job("shot_table", "done")).body).toBeUndefined();
  });
});

const strip = (src: string) => src.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/.*$/gm, "");
const HOOK = strip(readFileSync(fileURLToPath(new URL("./useJobToasts.ts", import.meta.url)), "utf8"));
const SHELL = strip(readFileSync(fileURLToPath(new URL("../StudioShell.tsx", import.meta.url)), "utf8"));

describe("通知条接线", () => {
  it("听任务事件，进来先恢复在跑的，恢复时不盖掉事件已经给过的新状态", () => {
    expect(HOOK).toContain("listen<Job>(JOB_EVENT");
    expect(HOOK).toContain("cinemaJobsRunning()");
    expect(HOOK).toContain("!seen.has(j.id)");
    expect(HOOK).toContain("upsert(jobToastId(job), toastForJob(job, ctx))");
  });

  /** 挂在页面里的话，切页面就退订，任务结束时没人接。 */
  it("只挂在 Studio 外壳，且在进/出项目两个分支之前", () => {
    const hook = SHELL.indexOf("useJobToasts();");
    expect(hook).toBeGreaterThan(0);
    expect(hook).toBeLessThan(SHELL.indexOf("if (projectId)"));
  });
});

describe("通知条：批量出片里的一镜不单独弹", () => {
  it("桥接先记下批量任务再判断子任务，子任务的通知关掉", () => {
    const src = readFileSync(fileURLToPath(new URL("./useJobToasts.ts", import.meta.url)), "utf8");
    expect(src).toContain("const all = [...known.values()];");
    expect(src).toContain("if (isBatchChild(job, all)) {");
    expect(src).toContain("useToasts.getState().dismiss(jobToastId(job));");
    expect(src).toContain("fresh.forEach((j) => known.set(j.id, j));");
  });
});

describe("出片卡片上的 ComfyUI 进度", () => {
  /** 2026-10-05 用户：「能不能把 ComfyUI 里的进度列出来，比如 1/4、83%、预计完成于」 */
  it("后端拼好的进度原样显示，预计完成时刻换成本地时间；没进度用原来那句", () => {
    const now = new Date(2026, 9, 5, 20, 0);
    const eta = Math.floor(new Date(2026, 9, 5, 21, 45).getTime() / 1000);
    const p = { done: 4, total: 6, label: "第 5/6 步 · 66% · 约还要 25 分钟", eta_at: eta };
    expect(videoProgressLine(p, now)).toBe("第 5/6 步 · 66% · 约还要 25 分钟 · 预计 21:45 完成");
    const tomorrow = Math.floor(new Date(2026, 9, 6, 1, 5).getTime() / 1000);
    expect(videoProgressLine({ ...p, eta_at: tomorrow }, now)).toContain("预计 10/6 01:05 完成");
    expect(videoProgressLine({ done: 0, total: 0, label: "ComfyUI 排队中 · 前面还有 2 个" }, now)).toBe("ComfyUI 排队中 · 前面还有 2 个");
    expect(videoProgressLine(undefined, now)).toBe(null);
    const job = { id: "j", kind: "video", status: "running", project_id: "p", episode_id: "e", shot_id: "s", shot_num: 2, prompt: "", created_at: 0, progress: p } as unknown as Job;
    expect(toastForJob(job).body).toContain("第 5/6 步");
  });
});


describe("批量出片的通知条带上这一镜的 ComfyUI 进度", () => {
  /** 2026-10-06 用户截图：批量出片只剩「第 1/3 个 · 第 2 镜。一镜要十几分钟…」，步数和预计完成时间没了 */
  it("正在出的那一镜有进度就显示进度，没有就用原来那句", () => {
    const batch = job("video_batch", "running", { targets: ["s1", "s2", "s3"], progress: { done: 0, total: 3, label: "第 2 镜" } });
    const child = job("video", "running", { id: "c2", shot_id: "s2", shot_num: 2, progress: { done: 4, total: 6, label: "第 5/6 步 · 66% · 约还要 25 分钟" } });
    const other = job("video", "running", { id: "x", shot_id: "s9" });
    const otherEp = job("video", "running", { id: "y", shot_id: "s2", episode_id: "e2" });
    expect(currentBatchChild(batch, [batch, other, otherEp, child])?.id).toBe("c2");
    expect(currentBatchChild(batch, [batch, other, otherEp])).toBeUndefined();
    const body = toastForJob(batch, { current: child }).body ?? "";
    expect(body).toContain("第 1/3 个 · 第 2 镜");
    expect(body).toContain("第 5/6 步 · 66% · 约还要 25 分钟");
    expect(toastForJob(batch).body).toContain("一镜要十几分钟");
    expect(toastForJob(batch, { current: job("video", "running", { shot_id: "s2" }) }).body).toContain("一镜要十几分钟");
  });

  it("桥接：子任务的进度更新也刷新批量那一条", () => {
    const src = readFileSync(fileURLToPath(new URL("./useJobToasts.ts", import.meta.url)), "utf8");
    expect(src).toContain("upsert(jobToastId(batch), toastForJob(batch, { current: job }))");
    expect(src).toContain("const ctx = { current: currentBatchChild(job, all), waiting: batchWaitingOn(job, all) };");
  });
});

describe("排队中的批量和它正在等的那一镜并成一条", () => {
  /** 2026-10-06 用户截图：重启后一条「第 3 镜 · 出片中」、一条「第 1/1 个 · 排队中」——「之前的 1/3 呢？列队恢复哪儿去了」 */
  it("等的那一镜和后面排队的说成一件事，带上那一镜的进度", () => {
    const batch = job("video_batch", "running", { id: "b", targets: ["s5"], progress: { done: 0, total: 1, label: "排队中 · 等前一镜出完" } });
    const resumed = job("video", "running", { id: "v3", shot_id: "s3", shot_num: 3, progress: { done: 0, total: 0, label: "ComfyUI 正在跑 · 已跑 1 分钟" } });
    expect(batchWaitingOn(batch, [batch, resumed])?.id).toBe("v3");
    expect(batchWaitingOn(batch, [batch, job("video", "running", { shot_id: "s3", episode_id: "e2" })])).toBeUndefined();
    // 已经开跑的批量不算在等
    expect(batchWaitingOn({ ...batch, progress: { done: 0, total: 1, label: "第 5 镜" } }, [batch, resumed])).toBeUndefined();
    // 名单里的那一镜是批量自己的子任务，不是在等的
    expect(batchWaitingOn(batch, [batch, { ...resumed, shot_id: "s5" }])).toBeUndefined();
    const body = toastForJob(batch, { waiting: resumed }).body ?? "";
    expect(body).toBe("还剩 2 镜：第 3 镜在出（ComfyUI 正在跑 · 已跑 1 分钟），后面还有 1 镜排队。");
  });

  it("桥接：等的那一镜不单独弹，进度刷新批量那一条", () => {
    const src = readFileSync(fileURLToPath(new URL("./useJobToasts.ts", import.meta.url)), "utf8");
    expect(src).toContain("upsert(jobToastId(waitingBatch), toastForJob(waitingBatch, { waiting: job }))");
    expect(src).toContain("if (ctx.waiting) useToasts.getState().dismiss(jobToastId(ctx.waiting));");
  });
});

describe("批量停在暂停位上（2026-10-10 用户：「这两个进度信息我看不懂」）", () => {
  it("说已暂停、不占 GPU，不说「可以去睡觉」；暂停位不算镜头", async () => {
    const { toastForJob } = await import("./jobToast");
    const job = { id: "b", kind: "video_batch", status: "running", project_id: "p", episode_id: "e", shot_id: "", shot_num: 0, prompt: "", created_at: 0,
      targets: ["hold:冒烟测试", "s15", "s16"], progress: { done: 0, total: 3, label: "暂停 · 冒烟测试" } } as never;
    const t = toastForJob(job) as { title: string; body: string };
    expect(t.title).toBe("批量出片已暂停");
    expect(t.body).toContain("冒烟测试");
    expect(t.body).toContain("后面的 2 镜");
    expect(t.body).not.toContain("睡觉");
  });
});

describe("画布任务合成一条（2026-10-11 用户：「如果有 10 个排队，这里也会叠出 10 个一样的」）", () => {
  const job = (id: string, prompt: string, label: string, created: number) =>
    ({ id, kind: "canvas", status: "running", project_id: "p", episode_id: "", shot_id: id, shot_num: 0, prompt, created_at: created,
       pipeline: label.includes("排队") ? undefined : "Ref2VA", progress: { done: 1, total: 4, label } }) as never;
  it("一个：写节点名和它自己的进度", async () => {
    const { canvasGroupToast } = await import("./jobToast");
    const t = canvasGroupToast([job("a", "第 5 步 · 第一段 · 生成", "第 2/4 步 · 约还要 15 分钟", 1)])!;
    expect(t.title).toBe("画布 · 第 5 步 · 第一段 · 生成");
    expect(t.body).toContain("【Ref2VA】第 2/4 步");
  });
  it("几个：一条里写谁在跑、谁在排队", async () => {
    const { canvasGroupToast } = await import("./jobToast");
    const t = canvasGroupToast([
      job("a", "第 5 步 · 第一段 · 生成", "第 2/4 步 · 约还要 15 分钟", 1),
      job("b", "第 6 步 · 第二段 · 生成", "ComfyUI 排队中 · 前面还有 1 个", 2),
    ])!;
    expect(t.title).toBe("画布 · 2 个任务（1 个在跑，1 个排队）");
    expect(t.body).toContain("第 5 步 · 第一段 · 生成：【Ref2VA】第 2/4 步");
    expect(t.body).toContain("排队：第 6 步 · 第二段 · 生成");
    expect(canvasGroupToast([])).toBeNull();
  });
});
