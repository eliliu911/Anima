/**
 * 新分镜页的纯函数：按场分组、三步进度、每镜的状态。
 *
 * 抽出来是为了能逐条测 —— 这几个数错了不报错，只表现为「进度条说好了，其实没好」。
 */
import { effectiveContinuity } from "./continuity";
import type { AssetView } from "../../../api/assets";
import type { Job } from "../../../api/jobs";
import type { PromptRoute, SceneRender, ShotIssue, StalePrompts, StoryboardShot } from "../../../api/cinema";

export interface SceneGroup {
  sceneNo: number;
  sceneName: string;
  shots: StoryboardShot[];
}

/** 按场分组，组内按镜号排。没有场号的镜头（手加的）归到最后一组「未分场」。 */
export function groupByScene(shots: StoryboardShot[]): SceneGroup[] {
  const groups = new Map<number, SceneGroup>();
  for (const shot of [...shots].sort((a, b) => a.shot_num - b.shot_num)) {
    const no = shot.scene_no && shot.scene_no > 0 ? shot.scene_no : 0;
    const g = groups.get(no) ?? { sceneNo: no, sceneName: no ? shot.scene_name || `第 ${no} 场` : "未分场", shots: [] };
    g.shots.push(shot);
    groups.set(no, g);
  }
  return [...groups.values()].sort((a, b) => (a.sceneNo || Infinity) - (b.sceneNo || Infinity));
}

/** 这一镜的问题。 */
export function issuesOf(issues: ShotIssue[], shot: StoryboardShot): ShotIssue[] {
  return issues.filter((i) => i.shot_num === shot.shot_num);
}

/** 这一镜在某条线路下是否已经合成过视频运动提示词。 */
export function isSynthesized(shot: StoryboardShot, route: PromptRoute): boolean {
  return !!shot.prompts?.[route]?.video?.trim();
}

/**
 * 镜头里用到、但还没有图的资产（去重，按出现顺序）。
 *
 * 「准备资产」那一步数的就是它：参考生视频要挂资产图，没图的资产那一镜就锁不住长相。
 */
export function assetsMissingImage(shots: StoryboardShot[], assets: AssetView[]): AssetView[] {
  const byId = new Map(assets.map((a) => [a.id, a]));
  const out: AssetView[] = [];
  for (const shot of shots) {
    const ids = [...(shot.character_ids ?? []), ...(shot.scene_id ? [shot.scene_id] : []), ...(shot.prop_ids ?? [])];
    for (const id of ids) {
      const a = byId.get(id);
      if (a && !a.cover_url && !out.includes(a)) out.push(a);
    }
  }
  return out;
}

export interface StepStatus {
  /** 1 确认镜头 */
  shots: number;
  shotsWithIssues: number;
  /** 2 准备资产：库里有但没图的 */
  missingAssets: number;
  /** 2 准备资产：镜头表里写了、资产库里没有的（去重） */
  unlinkedAssets: number;
  /** 3 合成提示词（当前线路） */
  synthesized: number;
  /** 合成过、但之后字段又改过的镜头数（要重新合成） */
  stale: number;
}

/**
 * 提示词那一层的问题，**归第 3 步**，不算第 1 步「确认镜头」的问题。
 *
 * 两个字段：
 * - `prompts`：合成之后字段改过 —— **真的不能出片**，要重新合成；
 * - `advice`：正文写得不够好（二选一、表演指示丢了、说话人串了）——
 *   **能出片**，只是值得改。
 */
export const isPromptIssue = (i: ShotIssue) => i.field === "prompts" || i.field === "advice";

/**
 * 这一条问题挡不挡出片。
 *
 * # 为什么必须和 `isPromptIssue` 分开
 *
 * 2026-09-20 出过事：合成正文的三条**启发式**建议（二选一、表演指示丢了、
 * 说话人串了）原来也挂 `field: "prompts"`，而
 * [`renderBlocker`] 把任何 `prompts` 问题都当成「合成之后改过字段」——
 * 于是正文里出现一个 `or`（哪怕是 "never move or fall" 这种否定并列），
 * 这一镜就进不了批量出片。八镜全中的时候，**整集一镜都出不了**，
 * 而界面给的理由是「改过字段或资产图」,和真实原因毫无关系。
 *
 * 「归哪一步」和「挡不挡路」是两件事，混成一个判据必然出这种事。
 */
export const blocksRender = (i: ShotIssue) => i.field === "prompts" || i.field === "route";

export function stepStatus(
  shots: StoryboardShot[],
  issues: ShotIssue[],
  assets: AssetView[],
  route: PromptRoute,
  /** 最近一次合成提示词的任务。在跑的时候，排队中 / 合成中的镜头不算已合成 */
  promptJob: Job | null = null,
): StepStatus {
  return {
    shots: shots.length,
    shotsWithIssues: new Set(issues.filter((i) => !isPromptIssue(i)).map((i) => i.shot_num)).size,
    missingAssets: assetsMissingImage(shots, assets).length,
    unlinkedAssets: new Set(shots.flatMap((s) => (s.unlinked_assets ?? []).map((u) => `${u.kind}:${u.name}`))).size,
    // 重新合成整集时，旧的提示词还在盘上 —— 只看「有没有」会一开始就显示 14/14（用户截图）
    synthesized: shots.filter((s) => {
      const round = promptRound(s, promptJob, route);
      return isSynthesized(s, route) && round !== "queued" && round !== "running";
    }).length,
    // **只数真挡路的。** 这个数字的标签是「N 镜要重新合成」，
    // 建议类的问题算进来就是在骗人（而且会把用户支去重合成八遍）。
    stale: new Set(issues.filter(blocksRender).map((i) => i.shot_num)).size,
  };
}

/**
 * 能不能批量生成视频：有镜头、没有问题、当前线路全部合成且没过期。
 *
 * **资产没图不挡**：没图的资产在提示词里按文字兜底（用户要求），照样能出片，只是长相锁不住 ——
 * 第 2 步会一直显示还差几个，提醒但不拦。
 */
export function readyToRender(s: StepStatus): boolean {
  return s.shots > 0 && s.shotsWithIssues === 0 && s.synthesized === s.shots && s.stale === 0;
}

/** 一镜在「合成提示词」这一轮里的状态。 */
export type PromptRound = "done" | "running" | "queued" | null;

/**
 * 从合成任务推这一镜：本轮已合成 / 正在合成 / 排队中 / 不在这一轮里（`null`）。
 *
 * # 为什么要逐镜分
 *
 * 以前只有一个「整集在合成」的开关：任务跑到 4/14，第 1 镜早就合成好落了盘，
 * 详情里仍然显示「合成中…」—— 看起来像卡住了，其实是排队里前面的已经做完。
 *
 * - **已合成**：这条线路的合成时间不早于任务开始时间（秒）。
 * - **正在合成**：任务进度标签是「第 N 镜」，N 就是这一镜。
 * - **排队中**：整集任务还在跑、这一镜还没轮到；或者这一镜在合成队列的名单里还没轮到
 *   （「加入合成队列」追加的，见后端 `queue_synthesize`）—— 这一轮里合过一次又排进来的，也算排队，不算已合成。
 */
export function promptRound(shot: StoryboardShot, job: Job | null, route: PromptRoute): PromptRound {
  if (!job || job.kind !== "prompts") return null;
  if (job.status === "running") {
    const current = /第\s*(\d+)\s*镜/.exec(job.progress?.label ?? "")?.[1];
    if (current !== undefined && Number(current) === shot.shot_num) return "running";
    if (queuePosition(job, shot.id) !== null) return "queued";
  }
  const at = shot.prompts?.[route]?.synthesized_at ?? 0;
  if (at >= job.created_at) return "done";
  if (job.status !== "running") return null;
  return job.prompt.includes("整集") ? "queued" : null;
}

/**
 * 这一镜在出片队列里排第几（还没轮到的那些里，从 1 数）；不在队列里回 `null`。
 *
 * 队列就是这一集在跑的批量出片的名单（`targets`）：「加入队列」追加在后面（后端 `jobs::append_targets`）。
 * 正在出的是 `progress.done` 那一个；还在「排队中」（等前一镜出完）时一个都没开始。
 */
export function queuePosition(job: Pick<Job, "status" | "targets" | "progress"> | null | undefined, shotId: string): number | null {
  if (!job || job.status !== "running") return null;
  const current = job.progress && !job.progress.label?.startsWith("排队中") ? job.progress.done : -1;
  const at = (job.targets ?? []).slice(current + 1).indexOf(shotId);
  return at >= 0 ? at + 1 : null;
}

/**
 * 这一镜现在能不能出片，不能的话为什么。返回 `null` = 可以。
 *
 * 出片原样发送当前线路存下的提示词：没合成、合成后又改过字段（过期），都不能出 ——
 * 后端也会拦，这里先说清楚，免得点了等半天才报错。
 */
export function renderBlocker(shot: StoryboardShot, route: PromptRoute, issues: ShotIssue[], videoRunning: boolean): string | null {
  if (videoRunning) return "这一集有镜头正在出片，一次出一个，等它跑完";
  if (!isSynthesized(shot, route)) return `还没有合成线路 ${route.toUpperCase()} 的提示词`;
  // 首帧出片却有人中途进场（Rust `shot_route::keyframe_blocker`）：原话照搬，别说成「改过字段」
  const route_ = issues.find((i) => i.shot_num === shot.shot_num && i.field === "route");
  if (route_) return route_.message;
  if (issues.some((i) => i.shot_num === shot.shot_num && blocksRender(i))) return "合成之后改过字段或资产图，先重新合成这一镜";
  return null;
}

/**
 * 批量出片要出哪几镜（按镜号顺序）。
 *
 * - 能出的（当前线路合成过、没过期）里，**还没用这条线路出过片的**；
 * - 都出过了，就是能出的全部（「全部重出」）。
 * - 没合成或过期的不在里面，数量单独给出来提示。
 */
export interface BatchPlan {
  /** 逐镜出的那些 */
  ids: string[];
  /** 整段出的那几场（长镜头）。**按场号排好** */
  longTakeScenes: number[];
  redo: boolean;
  blocked: number;
  /**
   * 接上一镜、而上一镜这次出不了的（没合成 / 过期、又没出过片）。**不在 `ids` 里**，
   * 每条一句「第 4 镜接第 3 镜，第 3 镜出不了」—— 不摘的话它们会排进去各报一条失败，
   * 看不出根子在上一镜（2026-09-27 按场出第 2 场就是这样）。
   */
  chained?: string[];
  /** 能出、但这条线路已经出过的镜数（长镜头按场算一个）。弹窗据此给「连出过的一起重出」那个选项 */
  done?: number;
}

/**
 * 批量出片出什么。**长镜头的场整段出，其余逐镜出，一次排完。**
 *
 * # 为什么必须把长镜头的场摘出来
 *
 * 勾了「这一场整段出」之后，这一场的三镜仍然满足「提示词合成好了」——
 * 不摘的话批量出片会把它们**一镜一镜再出一遍**：45 分钟 GPU 白烧，
 * 而且出来的 take 和整段出的成片是两套东西，谁也不知道该用哪个。
 * 2026-09-20 用户问「能不能做成一起的」时查出来的。
 *
 * # 一场里有镜头出不了，整场就不整段出
 *
 * 长镜头是一次提交、不能只重来某一窗口。少一个窗口的提示词，
 * 整条链就不成立 —— 这时候把这一场留给「哪几镜没合成」那条提示，
 * 而不是排一个注定失败的任务。
 */
export function batchTargets(
  shots: StoryboardShot[],
  route: PromptRoute,
  issues: ShotIssue[],
  longTakeScenes: number[] = [],
  /** 整集的镜头（按场出时 `shots` 只有勾的那几场，找上一镜要在整集里找） */
  all: StoryboardShot[] = shots,
  /** 整段出过的场（分集的 `scene_renders`）—— 这条线路出过的场不再排 */
  sceneRenders: readonly (Pick<SceneRender, "scene_no" | "route"> & Partial<Pick<SceneRender, "shot_ids">>)[] = [],
  /**
   * 出过的也一起出（用户 2026-10-08：「批量出片不能选全部，已经出片的没办法在这里重新出」）。
   * 不勾时照旧：还有没出过的就只出没出过的，全都出过了才是「全部重出」。重出是**新增一条**，旧片留着
   */
  includeDone = false,
): BatchPlan {
  const sorted = [...shots].sort((a, b) => a.shot_num - b.shot_num);
  const ok0 = sorted.filter((s) => renderBlocker(s, route, issues, false) === null);
  const ok0Ids = new Set(ok0.map((s) => s.id));
  // 接上一镜的：上一镜有片子，或者这一批会先出它，才接得上。按镜号往下传（4 卡住 5 也卡住）
  const chained: string[] = [];
  const ok: StoryboardShot[] = [];
  const willHave = new Set<string>();
  for (const s of ok0) {
    const prev = needsPrevTake(s) ? prevShot(all, s) : null;
    if (prev && renderTakes(prev, route).length === 0 && !willHave.has(prev.id)) {
      const why = ok0Ids.has(prev.id)
        ? "也接不上"
        : shots.includes(prev)
          ? "出不了（没合成或要重新合成）"
          : "还没出片（不在这次要出的镜头里）";
      chained.push(`第 ${s.shot_num} 镜接第 ${prev.shot_num} 镜，第 ${prev.shot_num} 镜${why}`);
      continue;
    }
    ok.push(s);
    willHave.add(s.id);
  }
  const okIds = new Set(ok.map((s) => s.id));

  // 这一场的每一镜都能出，才整段出
  const longOk = [...new Set(longTakeScenes)]
    .filter((no) => {
      const inScene = sorted.filter((s) => (s.scene_no ?? 0) === no);
      return inScene.length > 0 && inScene.every((s) => okIds.has(s.id));
    })
    .sort((a, b) => a - b);

  const perShot = ok.filter((s) => !longOk.includes(s.scene_no ?? 0));
  const fresh = perShot.filter((s) => renderTakes(s, route).length === 0);
  // **长镜头的场也按「这条线路出没出过」算**：整条记在分集的 `scene_renders` 上（不切开、不记在各镜名下）。
  // 以前不看这个，第 5 场出完（9 小时）再点批量会整场重跑（2026-09-30）
  // **还得是这一场现在这几镜出的**：重新生成过这一场（镜头 id 换了）的旧整段片不算出过（和后端 `scene_render_matches` 同一条）
  const longFresh = longOk.filter(
    (no) => !sceneRenders.some((r) => r.scene_no === no && r.route === route && sceneRenderMatches(r, sceneShotIds(shots, no))),
  );
  // 逐镜和长镜头都出过了，才是「全部重出」
  const allDone = fresh.length === 0 && longFresh.length === 0 && (perShot.length > 0 || longOk.length > 0);
  const done = perShot.length - fresh.length + (longOk.length - longFresh.length);
  const every = allDone || includeDone;
  return {
    ids: (every ? perShot : fresh).map((s) => s.id),
    longTakeScenes: every ? longOk : longFresh,
    redo: allDone || (includeDone && done > 0),
    blocked: shots.length - ok0.length,
    ...(chained.length ? { chained } : {}),
    done,
  };
}

/** 这一场现在的镜头 id，按镜号排 */
export function sceneShotIds(shots: readonly Pick<StoryboardShot, "id" | "scene_no" | "shot_num">[], sceneNo: number): string[] {
  return shots.filter((s) => (s.scene_no ?? 0) === sceneNo).sort((a, b) => a.shot_num - b.shot_num).map((s) => s.id);
}

/**
 * 整段片还是不是这一场现在这几镜出的。照抄 Rust `cinema::scene_render_matches`：
 * 重新生成过这一场（镜头 id 换了）的旧整段片不算；老数据没记 `shot_ids` 的照旧算。
 */
export function sceneRenderMatches(r: Partial<Pick<SceneRender, "shot_ids">>, ids: readonly string[]): boolean {
  return !r.shot_ids?.length || r.shot_ids.join("|") === ids.join("|");
}

/** 这一场最新的一次整段成片（长镜头）。`route` 不给就不挑线路 */
export function latestSceneRender<T extends Pick<SceneRender, "scene_no" | "route" | "created_at">>(renders: readonly T[], sceneNo: number, route?: PromptRoute): T | null {
  return renders.filter((r) => r.scene_no === sceneNo && (!route || r.route === route)).sort((a, b) => b.created_at - a.created_at)[0] ?? null;
}

/**
 * **这一场是不是按整条长镜头用。** 照抄 Rust `cinema::long_take_clip`，改的时候两边一起改 ——
 * 组装页显示和「合成一集」各判各的，就会出现「组装页说出过、合成出来缺这几镜」。
 *
 * 这一场设成了整段出、并且出过整段片（当前线路优先，另一条出过也用）。这时各镜自己
 * 单独出的片不用：长镜头不切开，没法把一镜换进整条里。要用单镜的，把这一场的出法改回分镜。
 *
 * 唯一的差别：前端看不到文件还在不在，后端合成时会再核一次（不在就按各镜算）。
 */
export function longTakeClip<T extends Pick<SceneRender, "scene_no" | "route" | "created_at"> & Partial<Pick<SceneRender, "shot_ids">>>(
  longTakeScenes: readonly number[],
  renders: readonly T[],
  sceneNo: number,
  route: PromptRoute,
  /** 这一集现在的镜头：给了就核对整段片是不是这几镜出的（见 `sceneRenderMatches`） */
  shots?: readonly Pick<StoryboardShot, "id" | "scene_no" | "shot_num">[],
): T | null {
  if (!longTakeScenes.includes(sceneNo)) return null;
  const r = latestSceneRender(renders, sceneNo, route) ?? latestSceneRender(renders, sceneNo);
  return r && (!shots || sceneRenderMatches(r, sceneShotIds(shots, sceneNo))) ? r : null;
}

/**
 * 镜头详情里出片按钮的字。出过片的叫「重新…」；**整段出的场里叫「单独出片」** ——
 * 这个按钮只出这一镜，叫「出片」会被当成出整场（用户 10-01）。
 */
export function renderButtonLabel(rendering: boolean, rendered: boolean, longTake: boolean): string {
  if (rendering) return "出片中…";
  return `${rendered ? "重新" : ""}${longTake ? "单独出片" : "出片"}`;
}

/** 组装页的一格：一镜，或一整场长镜头。 */
export type AssemblyTile<S, R> =
  | { kind: "shot"; shot: S }
  | { kind: "long_take"; sceneNo: number; render: R; shots: S[] };

/**
 * 组装页按什么顺序排哪些格子。**照抄 Rust `cinema::episode_segments`** —— 合成一集、剪映、
 * FCP 都按那个排，这里排得不一样，看到的就不是合成出来的。
 *
 * 整段出的场放一格，位置在这一场**第一次出现**的地方，这一场其余镜头不再各占一格；成员按镜号排。
 */
export function assemblyTiles<
  S extends Pick<StoryboardShot, "id" | "scene_no" | "shot_num">,
  R extends Pick<SceneRender, "scene_no" | "route" | "created_at"> & Partial<Pick<SceneRender, "shot_ids">>,
>(shots: readonly S[], longTakeScenes: readonly number[], renders: readonly R[], route: PromptRoute): AssemblyTile<S, R>[] {
  const out: AssemblyTile<S, R>[] = [];
  const placed = new Set<number>();
  for (const s of shots) {
    const no = s.scene_no ?? 0;
    const render = no > 0 ? longTakeClip(longTakeScenes, renders, no, route, shots) : null;
    if (!render) {
      out.push({ kind: "shot", shot: s });
      continue;
    }
    if (placed.has(no)) continue;
    placed.add(no);
    const members = shots.filter((x) => (x.scene_no ?? 0) === no).sort((a, b) => a.shot_num - b.shot_num);
    out.push({ kind: "long_take", sceneNo: no, render, shots: members });
  }
  return out;
}

/** 镜号紧挨着的上一镜（整集里找）。 */
function prevShot(all: StoryboardShot[], shot: StoryboardShot): StoryboardShot | null {
  return [...all].filter((s) => s.shot_num < shot.shot_num).sort((a, b) => a.shot_num - b.shot_num).pop() ?? null;
}

/** 出片时要拿上一镜的成片吗：设了接上一镜、又没挂首帧（挂了首帧由首帧锁画面，不接） */
function needsPrevTake(shot: StoryboardShot): boolean {
  return effectiveContinuity(shot) !== "none" && !shot.keyframes?.first;
}

/**
 * 批量出片的提交顺序：**按镜号从前往后**，整段出的场写成 `scene_<场号>`、排在它第一镜的位置
 * （和 Rust `cinema::long_take_unit` 同一个写法）。
 *
 * 以前是先单独提交长镜头、再提交其余镜头的批量 —— 2026-09-29 夜里批量那一下被「这一集有镜头正在出片」拒了，
 * 其余 32 镜一整夜没排上，早上看到的是「为什么先出第五场」。现在一个批量任务从头到尾按场次出。
 */
export function batchUnits(shots: readonly Pick<StoryboardShot, "id" | "shot_num" | "scene_no">[], ids: readonly string[], longTakeScenes: readonly number[]): string[] {
  const num = new Map(shots.map((s) => [s.id, s.shot_num]));
  const first = (scene: number) => Math.min(Infinity, ...shots.filter((s) => s.scene_no === scene).map((s) => s.shot_num));
  const units = [
    ...ids.map((id) => ({ id, at: num.get(id) ?? Infinity })),
    ...longTakeScenes.map((n) => ({ id: `scene_${n}`, at: first(n) })),
  ];
  return units.sort((a, b) => a.at - b.at).map((u) => u.id);
}

/** 一镜在批量出片里的状态：正在出 / 排队 / 不在这一轮。正在出以这一镜自己的出片任务为准。 */
export function batchRound(shot: StoryboardShot, batch: Job | null, videoRunningHere: boolean): "running" | "queued" | null {
  if (videoRunningHere) return "running";
  if (!batch || batch.status !== "running" || !batch.targets) return null;
  // 整段出的场在批量里是一项 `scene_<场号>`（见 `batchUnits`），这一场的每一镜都跟着它走
  const scene = `scene_${shot.scene_no}`;
  const key = batch.targets.includes(shot.id) ? shot.id : batch.targets.includes(scene) ? scene : null;
  if (!key) return null;
  const done = batch.progress?.done ?? 0;
  const at = batch.targets.indexOf(key);
  // 长镜头没有这一镜自己的出片任务，轮到它那一项就是在出
  if (key === scene && at === done) return "running";
  return at >= done ? "queued" : null;
}

/**
 * 这条问题要**重新生成这一场**才能解决：漏拍了剧本句子、台词没进任何一镜（少了镜头，代码补不出画面）。
 */
export function needsSceneRegen(i: ShotIssue): boolean {
  return /没有任何一镜拍到|没有写进这一场任何一镜的台词栏/.test(i.message);
}

/** 哪几场有要重新生成才能解决的问题（按问题所在镜头的场号）。 */
export function scenesNeedingRegen(shots: StoryboardShot[], issues: ShotIssue[]): number[] {
  const nos = new Set<number>();
  for (const i of issues.filter(needsSceneRegen)) {
    const shot = shots.find((s) => s.shot_num === i.shot_num);
    if (shot?.scene_no) nos.add(shot.scene_no);
  }
  return [...nos].sort((a, b) => a - b);
}

/** 这条问题能不能由「自动修正写法」修掉（和 Rust `storyboard_gen::repair_shots` 的五类对应）。 */
export function isRepairable(i: ShotIssue): boolean {
  return /夹了英文|写了「（或|写了「或者|写了「或是|写了「抑或|写了「，或|写了「根据前文|写了「根据剧情|写了「视情况|写了「可根据|又写了一遍台词|写了「切至|写了「切回|写了「切到|写了「切换到|不是这一镜的出场人物|不是逐字出自这一场剧本/.test(i.message);
}

/**
 * 合成提示词要合成哪几镜（按镜号顺序）。
 *
 * - `onlyNeeded`：只要这条线路还没合成、或者合成后过期的；
 * - 否则整集。
 */
/**
 * 合成弹窗里「没合成或过期的」那些镜。
 *
 * **过期要按线路问**（`stale`，来自 `cinemaStalePrompts`）。问题清单 `issues` 只报**当前线路**的过期，
 * 而合成哪条线路就把哪条存成当前线路 —— 只靠它的话，合成完 A 再切到 B，B 过期了也显示「不需要」
 * （用户 2026-09-29：「合成其中一个，点另一个就变成不需要了」）。`stale` 还没取到时退回问题清单。
 * 英文二选一这类提示（`advice`）只是线路 B 正文的问题，只算在 B 上。
 */
/**
 * 只有这一种提示是**重新合成就能消掉的**（后端 `cinema_issue_fixes` 给它的按钮也是「重新合成这一镜」）。
 *
 * 2026-10-07 用户截图：合成跑完「14/16 镜成功、2 镜没过」，弹窗却说「没合成或过期的 10 镜」——
 * 以前任何 `advice` 都算进来：群演挂不全、参考图没挂上、外貌偏薄、没选造型……这些合成成功也照样在，
 * 那几镜永远显示「要合成」，合几遍都消不掉。它们在分镜页提示，不算进这个数。
 */
export const RESYNTH_ADVICE = "正文里有二选一";

export function synthTargets(shots: StoryboardShot[], route: PromptRoute, issues: ShotIssue[], onlyNeeded: boolean, stale?: StalePrompts | null): string[] {
  const has = (s: StoryboardShot, field: string) => issues.some((i) => i.shot_num === s.shot_num && i.field === field);
  const needs = (s: StoryboardShot) =>
    !isSynthesized(s, route) ||
    (stale ? stale[route].includes(s.id) : has(s, "prompts")) ||
    (route === "b" && issues.some((i) => i.shot_num === s.shot_num && i.field === "advice" && i.message.startsWith(RESYNTH_ADVICE)));
  return [...shots]
    .sort((a, b) => a.shot_num - b.shot_num)
    .filter((s) => !onlyNeeded || needs(s))
    .map((s) => s.id);
}

/**
 * 正在合成的是哪条线路（没在合成回 null）。任务上只有给人看的标题「线路 A（中文结构） · 3 镜」，从它认。
 *
 * 合成弹窗左右两块各合各的，但**同一集同一时间只能跑一个合成任务**（后端 `start_exclusive`）——
 * 一边在跑时另一边的按钮要说「等它跑完」，而不是点了才报错。
 */
export function runningRoute(job: { status: string; prompt: string } | null | undefined): PromptRoute | null {
  if (!job || job.status !== "running") return null;
  return job.prompt.startsWith("线路 A") ? "a" : "b";
}

/**
 * 「提示词」卡片上那一行：两条线路各差什么（出片线路排前面）。都齐了说「两条线路都是最新的」。
 * `stale` 是按线路问的过期名单（`cinemaStalePrompts`）；还没取到时只报没合成的
 */
export function synthLine(shots: StoryboardShot[], route: PromptRoute, stale: StalePrompts | null): string {
  if (shots.length === 0) return "";
  const parts: string[] = [];
  for (const r of [route, route === "a" ? "b" : "a"] as PromptRoute[]) {
    const missing = shots.filter((s) => !isSynthesized(s, r)).length;
    const st = stale ? stale[r].filter((id) => shots.some((s) => s.id === id)).length : 0;
    const bits = [missing ? `${missing} 镜没合成` : "", st ? `${st} 镜要重合成` : ""].filter(Boolean).join("、");
    if (bits) parts.push(`${r.toUpperCase()} ${bits}`);
  }
  return parts.length ? parts.join(" · ") : "两条线路都是最新的";
}

/**
 * 还没首帧、这条线路也还没合成的镜头。
 *
 * **它们该先出首帧再合成**：先合成的话，出完首帧提示词就过期（要按首帧重写），同一镜要合两遍、白花一轮 token。
 * 先出首帧（按镜头表现拼画面，不花 token），出完自动合成一次（2026-10-03 起的顺序，用户：「这不是犯傻吗」）。
 * 不出首帧、直接用参考图出片的镜头照常合成 —— 首帧是可选的，这里只是提醒顺序。
 */
export function framesFirst(shots: readonly StoryboardShot[], route: PromptRoute): StoryboardShot[] {
  return shots.filter((s) => !s.keyframes?.first && !isSynthesized(s, route));
}

/** 分镜页的四步流程按钮。 */
export type FlowStep = "shots" | "frames" | "synth" | "render";

/**
 * 现在该点哪一个。**高亮成主色的只有这一个**，其余是普通按钮。
 *
 * 用户反馈（2026-09-14）：三个按钮一样的样子（只有「重新生成镜头」是蓝的），第一次用不知道先点哪个 ——
 * 而那个蓝色的恰恰是会覆盖整张表的那个。
 *
 * 顺序：没镜头 → 生成镜头；有没首帧也没合成的 → 出首帧（见 `framesFirst`）；有没合成或过期的 → 合成提示词；
 * 都合成好了 → 批量出片。有任务在跑时不高亮。
 *
 * **首帧排在提示词前面**（2026-10-04 用户：「先出了 B 的提示词，再出首帧，不还是要重新出 B 的提示词吗」）。
 * 原来提示词是第 2 步、首帧第 3 步，照着点就是合两遍。
 */
export function nextStep(t: { shots: number; synthesized: number; stale: number; canRender: boolean; busy: boolean; framesFirst?: number }): FlowStep | null {
  if (t.busy) return null;
  if (t.shots === 0) return "shots";
  if ((t.framesFirst ?? 0) > 0) return "frames";
  if (t.synthesized < t.shots || t.stale > 0) return "synth";
  return t.canRender ? "render" : null;
}

/** 页头三个按钮当下的状态，给悬浮提示用。 */
export interface ToolbarState {
  route: PromptRoute;
  shots: number;
  generating: boolean;
  synthesizing: boolean;
  videoRunning: boolean;
  batch: { ids: string[]; redo: boolean; blocked: number };
  /** 另一条线路这一批能出几镜（出片弹窗里可以换过去） */
  otherBatch?: number;
  synthesized: number;
  stale: number;
}

/**
 * 页头三个按钮的悬浮提示：**按了会做什么**，点不了的时候**为什么**。
 *
 * 用户反馈：按钮上只有几个字，灰掉了也不知道缺什么（截图里「批量出片（先合成提示词）」灰着，其实是提示词过期了）。
 */
export function toolbarHints(t: ToolbarState): { batch: string; synth: string; regen: string } {
  const R = t.route.toUpperCase();
  const busy = t.generating ? "正在生成镜头表，等它跑完" : t.synthesizing ? "正在合成提示词，等它跑完" : "";
  const batch = busy
    ? busy
    : t.videoRunning
      ? "正在出片，一次出一批 —— 要停就点「停止批量出片」"
      : t.batch.ids.length === 0 && t.otherBatch
        ? `当前线路 ${R} 没有能出的镜头；线路 ${t.route === "a" ? "B" : "A"} 可出 ${t.otherBatch} 镜，点了在出片弹窗里换过去`
        : t.batch.ids.length === 0
        ? t.stale > 0
          ? `没有能出的镜头：${t.stale} 镜合成之后改过字段、资产图或风格，先点「重新合成提示词」`
          : `没有能出的镜头：先点「合成提示词」，用线路 ${R} 把提示词合成出来`
        : `${t.batch.redo ? "全部重出" : "按顺序出"} ${t.batch.ids.length} 镜（线路 ${R}），点了先选画质、清晰度，再开始。` +
          `某一镜失败会记下接着出下一镜。${t.batch.redo ? "" : "已经出过的不再出。"}${t.batch.blocked ? `另有 ${t.batch.blocked} 镜没合成或过期，不在这一批里。` : ""}`;
  const synth = busy
    ? busy
    : `用线路 ${R} 给整集 ${t.shots} 镜合成提示词（线路 B 每镜要几十秒，后台跑）。` +
      `会写进当前的风格、嗓音、服装一致和上一镜的衔接。${t.synthesized ? `已合成的 ${t.synthesized} 镜会重新合成。` : ""}${t.stale ? `现在有 ${t.stale} 镜过期。` : ""}`;
  const regen = t.generating
    ? "正在生成镜头表，等它跑完"
    : t.shots
      ? `由剧本重新生成整张镜头表：会覆盖现在的 ${t.shots} 个镜头，改过的字段、合成的提示词都会没有（已出的片文件还在）。要点两下确认。`
      : "由剧本生成镜头表：分场、拆镜头、校验，一场一场来，后台跑";
  return { batch, synth, regen };
}

/** 出过的每一次，新的在前。 */
/**
 * 这一镜在某条线路上**由分镜页出过**的那几「次」。
 *
 * **画布回填的不算。** 它没走线路 A / B，算进去的两个后果都不报错：
 * 批量出片会跳过这一镜；按钮会从「出片」变成「重新出片」，
 * 让人以为这条线路已经出过了。Rust 侧同一条判据在 `TakeOrigin`。
 *
 * 四个地方要数 take（批量、单镜按钮、出片弹窗、镜头卡角标），
 * **判据只能有一个所有者** —— 各写一遍必然漏掉一处。
 */
export function renderTakes(shot: StoryboardShot, route: PromptRoute) {
  return (shot.takes ?? []).filter((t) => t.route === route && t.origin !== "canvas");
}

export function takesNewestFirst(shot: StoryboardShot): NonNullable<StoryboardShot["takes"]> {
  return [...(shot.takes ?? [])].sort((a, b) => b.created_at - a.created_at);
}

/** 秒数显示：整数不带小数点。 */
export function secs(n: number): string {
  return Number.isInteger(n) ? `${n}` : n.toFixed(1);
}

/**
 * 这一镜里某个角色实际用的形象：**单镜覆盖 → 项目默认 → 主形象**。指着不存在的形象就跳过这一级。
 * 和 Rust `looks::apply` / `for_shot` 同一套规则。回形象 id 和它是不是本镜覆盖的。
 */
export function effectiveLook(
  shot: Pick<StoryboardShot, "look_overrides">,
  defaults: Record<string, string> | undefined,
  asset: Pick<AssetView, "id" | "look_views">,
): { id: string; overridden: boolean } {
  const exists = (id: string | undefined): id is string => !!id && (asset.look_views ?? []).some((l) => l.id === id);
  const own = shot.look_overrides?.[asset.id];
  if (exists(own)) return { id: own, overridden: true };
  const def = defaults?.[asset.id];
  if (exists(def)) return { id: def, overridden: false };
  return { id: "main", overridden: false };
}


/** 按场出片里一场的样子：场号、场名、几镜、已经出过几镜、这次能出几镜 */
export interface SceneRow {
  no: number;
  name: string;
  total: number;
  rendered: number;
  ready: number;
}

/**
 * 按场出片的场清单（按场号排）。
 *
 * 用户 2026-09-27：「现在的批量出片是全部分镜，要加一个按场出片，点开后可以复选第几场和多个场来生成。」
 * 同一场的镜头常设成「接上一镜」，一场一场出也更好盯 —— 出完一场看过再出下一场。
 */
export function sceneRows(
  shots: StoryboardShot[],
  route: PromptRoute,
  issues: ShotIssue[],
  /** 整段出过的场：这条线路出过就整场算出过（片子记在场上，不在各镜名下） */
  sceneRenders: readonly (Pick<SceneRender, "scene_no" | "route"> & Partial<Pick<SceneRender, "shot_ids">>)[] = [],
): SceneRow[] {
  const map = new Map<number, SceneRow>();
  for (const s of [...shots].sort((a, b) => a.shot_num - b.shot_num)) {
    const no = s.scene_no ?? 0;
    const row = map.get(no) ?? { no, name: (s.scene_name ?? "").replace(/^场景[：:]\s*/, ""), total: 0, rendered: 0, ready: 0 };
    row.total += 1;
    if (renderTakes(s, route).length > 0 || sceneRenders.some((r) => r.scene_no === no && r.route === route && sceneRenderMatches(r, sceneShotIds(shots, no)))) row.rendered += 1;
    if (renderBlocker(s, route, issues, false) === null) row.ready += 1;
    map.set(no, row);
  }
  return [...map.values()].sort((a, b) => a.no - b.no);
}
