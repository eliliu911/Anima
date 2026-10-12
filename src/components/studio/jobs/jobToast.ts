/**
 * 后台任务 → 右下角通知条。**所有后台生成都走这一处**，页面自己不再各推各的。
 *
 * 任务状态在 Rust 的任务登记表里，变化时发 `cinema://job-updated`；
 * `useJobToasts` 听这个事件，把每个任务映射成一条 id 固定的通知（`job-<任务 id>`），
 * 所以「在跑 → 完成」是同一条就地换，切页面、界面刷新都不丢。
 */
import { holdLabel, shotCounts } from "./renderQueue";
import type { Job } from "../../../api/jobs";
import type { Toast } from "../toastStore";

export type JobToast = Pick<Toast, "kind" | "title" | "body">;

export const jobToastId = (job: Pick<Job, "id">) => `job-${job.id}`;

/**
 * 批量出片要借别的任务的进度：
 * - `current`：正在出的那一镜（见 `currentBatchChild`），ComfyUI 进度在它身上；
 * - `waiting`：批量还在排队、正在等的那一镜（见 `batchWaitingOn`）。
 */
export type BatchContext = { current?: Job; waiting?: Job };

export function toastForJob(job: Job, ctx: BatchContext = {}): JobToast {
  if (job.kind === "shot_table") return shotTableToast(job);
  if (job.kind === "prompts") return promptsToast(job);
  if (job.kind === "video_batch") return batchToast(job, ctx);
  if (job.kind === "frame_batch") return frameBatchToast(job);
  if (job.kind === "canvas") return canvasToast(job);
  if (job.kind === "stitch") return stitchToast(job);
  if (job.kind === "fcp_export") return fcpToast(job);
  if (job.kind === "asset_images") return assetImagesToast(job);
  if (job.kind === "timeline") return timelineToast(job);
  const what = job.kind === "video" ? "出片" : "出首帧";
  // 长镜头按场登记（`shot_id` 是 `scene_<场号>`、没有镜号）—— 以前这里显示成「第 ? 镜」
  const scene = /^scene_(\d+)$/.exec(job.shot_id);
  const shot = scene ? `第 ${scene[1]} 场 · 长镜头` : `第 ${job.shot_num || "?"} 镜`;
  switch (job.status) {
    case "running":
      return {
        kind: "progress",
        title: `${shot} · ${what}中`,
        body: scene
          ? withPipeline(job, "一场几个窗口连着出，要好几个小时（三个窗口的 720p 十几个小时），整条出完才有文件。只要 ComfyUI 还在报进度就一直等。")
          : job.kind === "video"
            ? withPipeline(job, videoProgressLine(job.progress) ?? "视频要跑几分钟到十几分钟，可以先去别的地方，跑完这里会告诉你。")
            : "可以先去别的地方，好了这里会告诉你。",
      };
    case "done":
      return { kind: "success", title: `${shot} · ${what}好了` };
    case "failed":
      // 失败原文照抄：「ComfyUI 没起」用户自己就能修，概括成「失败」就修不了
      return { kind: "error", title: `${shot} · ${what}失败`, body: job.error || "没有返回原因" };
  }
}

/**
 * 进度前面加上这一镜实际走的链路：「【FL2VA 首帧 · LightX2V 4 次】第 3/5 步 · …」。
 * 链路文字是后端从发给 ComfyUI 的那张图里读出来的（`weetodd::graph_pipeline`），**这里不按设置推** ——
 * 用户 2026-10-07：「不是硬编码出来的，而是返回的」。还没提交（没有这个字段）就不加。
 */
export function withPipeline(job: Pick<Job, "pipeline"> | undefined, line: string): string {
  return job?.pipeline ? `【${job.pipeline}】${line}` : line;
}

/** 两位数 */
const pad = (n: number) => String(n).padStart(2, "0");

/**
 * 出片卡片上 ComfyUI 的进度：「第 5/6 步 · 66% · 约还要 25 分钟 · 已跑 75 分钟 · 预计 21:45 完成」。
 * 文字是后端按 ComfyUI 的步数拼好的（`comfy_progress::video_line`），这里只把预计完成时刻换成本地时间。
 * 还没拿到进度（刚提交）回 null，用原来那句。
 */
export function videoProgressLine(p: Job["progress"], now: Date = new Date()): string | null {
  if (!p?.label) return null;
  if (!p.eta_at) return p.label;
  const at = new Date(p.eta_at * 1000);
  const sameDay = at.toDateString() === now.toDateString();
  const clock = `${pad(at.getHours())}:${pad(at.getMinutes())}`;
  const day = sameDay ? "" : `${at.getMonth() + 1}/${at.getDate()} `;
  return `${p.label} · 预计 ${day}${clock} 完成`;
}

function batchToast(job: Job, { current, waiting }: BatchContext): JobToast {
  switch (job.status) {
    case "running": {
      const p = job.progress;
      // 还在等前一镜（单镜出片在跑 / 重启后在续接的那一镜）：把那一镜和后面排队的说成一件事
      if (waiting) {
        const queued = job.targets?.length ?? p?.total ?? 0;
        const p2 = videoProgressLine(waiting.progress);
        const line = p2 ? withPipeline(waiting, p2) : waiting.pipeline ? `【${waiting.pipeline}】` : null;
        return {
          kind: "progress",
          title: "正在批量出片",
          body: `还剩 ${queued + 1} 镜：第 ${waiting.shot_num || "?"} 镜在出${line ? `（${line}）` : ""}，后面还有 ${queued} 镜排队。`,
        };
      }
      // **停在暂停位上**：没在出片、不占 GPU，别再说「一镜要十几分钟，可以去睡觉」（2026-10-10 用户：「这两个进度信息我看不懂」）
      const hold = holdLabel(job);
      if (hold !== null) {
        const left = shotCounts(job).left;
        return { kind: "progress", title: "批量出片已暂停", body: `暂停中：${hold}。现在没在出片，不占 GPU；在任务队列里撤掉暂停位，就接着出后面的 ${left} 镜。` };
      }
      // 暂停位不是镜头：不算进「第几个 / 共几个」
      const c = shotCounts(job);
      const where = p && p.total > 0 ? `第 ${Math.max(c.position, 1)}/${c.total} 个${p.label ? ` · ${p.label}` : ""}` : "准备中";
      // **这一镜的 ComfyUI 进度**（第几步 / 百分比 / 还要多久 / 预计几点完成）写在这一镜自己的出片任务上，
      // 那条在批量里不单独弹 —— 不接过来，排队出的片就看不到进度（2026-10-06 用户截图）
      const raw = current?.kind === "video" ? videoProgressLine(current.progress) : null;
      const line = raw ? withPipeline(current, raw) : null;
      if (line) return { kind: "progress", title: "正在批量出片", body: `${where}。${line}` };
      if (current?.kind === "video" && current.pipeline) return { kind: "progress", title: "正在批量出片", body: `${where}。【${current.pipeline}】刚提交，等 ComfyUI 报进度。` };
      return { kind: "progress", title: "正在批量出片", body: `${where}。一镜要十几分钟，可以去睡觉，某一镜失败会接着出下一镜。` };
    }
    case "done": {
      const notes = job.notes?.length ?? 0;
      return {
        kind: notes > 0 ? "error" : "success",
        title: notes > 0 ? "批量出片跑完了，有几镜没出成" : "批量出片全部完成",
        body: [job.summary, notes > 0 ? `${notes} 镜跳过或失败，原因在分镜页。` : ""].filter(Boolean).join(" ") || undefined,
      };
    }
    case "failed":
      return { kind: "error", title: "批量出片没出成", body: job.error || "没有返回原因" };
  }
}

function frameBatchToast(job: Job): JobToast {
  switch (job.status) {
    case "running": {
      const p = job.progress;
      const where = p && p.total > 0 ? `第 ${Math.min(p.done + 1, p.total)}/${p.total} 个${p.label ? ` · ${p.label}` : ""}` : "准备中";
      return { kind: "progress", title: "正在批量出首帧", body: `${where}。参考生图一张几分钟，某一镜失败会接着出下一镜。` };
    }
    case "done": {
      const notes = job.notes?.length ?? 0;
      return {
        kind: notes > 0 ? "error" : "success",
        title: notes > 0 ? "首帧出完了，有几镜没出成" : "首帧全部出完",
        body: [job.summary, notes > 0 ? `${notes} 镜没出成，原因在队列里。` : "", "挂了首帧的镜头要重新合成提示词再出片。"].filter(Boolean).join(" "),
      };
    }
    case "failed":
      return { kind: "error", title: "批量出首帧没出成", body: job.error || "没有返回原因" };
  }
}

function timelineToast(job: Job): JobToast {
  switch (job.status) {
    case "running": {
      const p = job.progress;
      const where = p && p.total > 0 ? `第 ${Math.min(p.done + 1, p.total)}/${p.total} 个${p.label ? ` · ${p.label}` : ""}` : "准备中";
      return { kind: "progress", title: "正在细化时间线", body: `${where}。一镜几十秒，某一镜没过会接着做下一镜。` };
    }
    case "done": {
      const notes = job.notes?.length ?? 0;
      return {
        kind: notes > 0 ? "error" : "success",
        title: notes > 0 ? "时间线细化完了，有几镜没成" : "时间线细化完了",
        body: [job.summary, notes > 0 ? `${notes} 条没成，原因在队列里。` : ""].filter(Boolean).join(" "),
      };
    }
    case "failed":
      return { kind: "error", title: "时间线没细化成", body: job.error || "没有返回原因" };
  }
}

function promptsToast(job: Job): JobToast {
  switch (job.status) {
    case "running": {
      const p = job.progress;
      const where = p && p.total > 0 ? `第 ${Math.min(p.done + 1, p.total)}/${p.total} 个 · ${p.label}` : "准备中";
      return { kind: "progress", title: "正在合成提示词", body: `${where}。线路 B 每镜要几十秒，可以先去别的地方。` };
    }
    case "done": {
      const notes = job.notes?.length ?? 0;
      const parts = [job.summary, notes > 0 ? `${notes} 镜没通过检查，原因在分镜页。` : ""].filter(Boolean);
      return { kind: notes > 0 ? "error" : "success", title: notes > 0 ? "提示词合成完，有几镜没成功" : "提示词合成好了", body: parts.join(" ") || undefined };
    }
    case "failed":
      return { kind: "error", title: "合成提示词失败", body: job.error || "没有返回原因" };
  }
}

function shotTableToast(job: Job): JobToast {
  switch (job.status) {
    case "running": {
      const p = job.progress;
      const where = p && p.total > 0
        ? `第 ${Math.min(p.done + 1, p.total)}/${p.total} 场${p.label ? ` · ${p.label}` : ""}`
        : "正在读剧本、分场";
      return { kind: "progress", title: "正在生成镜头", body: `${where}。可以先去别的步骤。` };
    }
    case "done": {
      const notes = job.notes?.length ?? 0;
      const parts = [job.summary, notes > 0 ? `另有 ${notes} 条备注，在分镜页看。` : ""].filter(Boolean);
      return {
        kind: "success",
        title: "镜头生成好了",
        body: parts.join(" ") || undefined,
      };
    }
    case "failed":
      return { kind: "error", title: "生成镜头失败", body: job.error || "没有返回原因" };
  }
}

/** 合成一集。`prompt` 里是「合成一集」或「合成一集 · 超分 1080P」 */
function stitchToast(job: Job): JobToast {
  const what = job.prompt || "合成一集";
  switch (job.status) {
    case "running": {
      const p = job.progress;
      const where = p && p.total > 0 ? (p.done >= p.total ? "正在拼接" : `第 ${p.done + 1}/${p.total} 镜`) : "准备中";
      const slow = what.includes("超分") ? "超分一镜要一两分钟，" : "";
      return { kind: "progress", title: `正在${what}`, body: `${where}。${slow}可以先去别的地方。` };
    }
    case "done":
      return { kind: "success", title: "成片合成好了", body: "在「组装」这一步预览。" };
    case "failed":
      return { kind: "error", title: "合成成片失败", body: job.error || "没有返回原因" };
  }
}

/**
 * 导出到 Final Cut Pro。`prompt` 里是「导出到 Final Cut Pro」或带「· 超分 1080P」。
 * 成功时 `summary` 是「14 镜 → 文件夹」，`remarks` 是没出片的镜号这类话 —— 都要让人看见
 */
function fcpToast(job: Job): JobToast {
  switch (job.status) {
    case "running": {
      const p = job.progress;
      const where = p && p.total > 0 ? (p.done >= p.total ? "正在写时间线" : `第 ${p.done + 1}/${p.total} 镜`) : "准备中";
      const slow = job.prompt.includes("超分") ? "超分一镜要一两分钟，" : "";
      return { kind: "progress", title: "正在导出到 Final Cut Pro", body: `${where}。${slow}可以先去别的地方。` };
    }
    case "done": {
      const extra = job.remarks?.length ? `。${job.remarks.join("；")}` : "";
      return { kind: "success", title: "导出好了，FCP 正在打开", body: `${job.summary || job.output || ""}${extra}` };
    }
    case "failed":
      return { kind: "error", title: "导出到 FCP 失败", body: job.error || "没有返回原因" };
  }
}

/** 资产批量出图。`prompt` 里是「资产出图 · 2 个角色形象、1 个场景」 */
function assetImagesToast(job: Job): JobToast {
  switch (job.status) {
    case "running": {
      const p = job.progress;
      const where = p && p.total > 0 ? `第 ${Math.min(p.done + 1, p.total)}/${p.total} 张${p.label ? ` · ${p.label}` : ""}` : "准备中";
      return { kind: "progress", title: job.prompt || "正在出资产设定图", body: `${where}。一张一两分钟，可以切去别的步骤，出好一张卡片上就换一张。` };
    }
    case "done": {
      const notes = job.notes?.length ?? 0;
      // 没出来的名字和原因进这张卡 —— 人可能已经切走了，看不到页面上那份清单
      return {
        kind: notes > 0 ? "error" : "success",
        title: job.summary || (notes > 0 ? "有几张没出来" : "设定图都出好了"),
        body: notes > 0 ? job.notes!.join("；") : "都在资产卡片上了。",
      };
    }
    case "failed":
      return { kind: "error", title: "资产出图没出成", body: job.error || "没有返回原因" };
  }
}

/** 画布进行中的任务合成一条用的 id（不是某一条任务的 id） */
export const CANVAS_GROUP_TOAST = "job-canvas-running";

/**
 * **在跑和排队的画布任务合成一条**（2026-10-11 用户：「这两个也算重复，因为没有具体事项，不然如果有 10 个排队，这里也会叠出 10 个一样的」）。
 * 正在 ComfyUI 里跑的那个写进度，排队的列名字；完成 / 失败仍各自弹一条（`canvasToast`）。没有在跑的回 `null`（调用方关掉这一条）。
 */
export function canvasGroupToast(running: Job[]): JobToast | null {
  if (running.length === 0) return null;
  // 后端写的是「ComfyUI 排队中 · 前面还有 N 个」（`comfy_progress::video_line`）
  const queued = (j: Job) => (j.progress?.label ?? "").includes("排队");
  const active = running.filter((j) => !queued(j));
  const waiting = running.filter(queued);
  const name = (j: Job) => j.prompt || "生成";
  const line = (j: Job) => {
    const p = videoProgressLine(j.progress);
    return p ? withPipeline(j, p) : "准备中";
  };
  if (running.length === 1) {
    const j = running[0];
    return { kind: "progress", title: `画布 · ${name(j)}`, body: line(j) };
  }
  const parts = active.map((j) => `${name(j)}：${line(j)}`);
  if (waiting.length) parts.push(`排队：${waiting.slice(0, 3).map(name).join("、")}${waiting.length > 3 ? ` 等 ${waiting.length} 个` : ""}`);
  return {
    kind: "progress",
    title: `画布 · ${running.length} 个任务（${active.length} 个在跑，${waiting.length} 个排队）`,
    body: parts.join("\n"),
  };
}

/** 画布节点的生成。`prompt` 里是给人看的名字（「第 5 步 · 第一段 · 对峙与追逐 · 生成」「动作迁移」） */
function canvasToast(job: Job): JobToast {
  const what = job.prompt || "生成";
  switch (job.status) {
    case "running":
      return { kind: "progress", title: `画布 · ${what}中`, body: "在后台跑，可以切到别的页面，刷新也不会丢。" };
    case "done":
      return { kind: "success", title: `画布 · ${what}好了` };
    case "failed":
      return { kind: "error", title: `画布 · ${what}失败`, body: job.error || "没有返回原因" };
  }
}
