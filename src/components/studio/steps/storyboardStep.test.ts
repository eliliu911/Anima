import { renderButtonLabel } from "../storyboard/storyboardView";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const strip = (src: string) => src.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/.*$/gm, "");
const STEP = strip(readFileSync(fileURLToPath(new URL("./StoryboardStep.tsx", import.meta.url)), "utf8"));
const DETAIL = strip(readFileSync(fileURLToPath(new URL("../storyboard/ShotDetail.tsx", import.meta.url)), "utf8"));

/**
 * 分镜页的接线。这几处断了都不报错：按钮照样能点，只是状态不对或者没保存。
 */
describe("新分镜页接线", () => {
  it("生成镜头只提交后台任务，不在前端等结果", () => {
    const i = STEP.indexOf("const generate = async");
    const body = STEP.slice(i, STEP.indexOf("const saveShot", i));
    expect(body).toContain("cinemaGenerateShotTable(project.id, episode.id)");
    // 结果走任务登记表；前端等返回值就回到了「切走就丢」的老路
    expect(body).not.toContain("result");
  });

  it("生成状态、进度、备注都从任务登记表来，不存组件本地 state", () => {
    expect(STEP).toContain("shotTableState(jobs)");
    expect(STEP).not.toMatch(/useState<[^>]*Progress/);
    expect(STEP).not.toContain("setNotes");
    expect(STEP).not.toContain("cinema://shot-table-progress");
    // 任务落地时刷新集数据，否则镜头表生成完了界面还是空的
    expect(STEP).toMatch(/useJobs\(projectId, episodeId, queueOpen, \(\) => void reload\(\), "storyboard"\)/);
  });

  it("上次生成失败要显示原文", () => {
    expect(STEP).toContain('shotJob?.status === "failed"');
    expect(STEP).toContain("{shotJob.error}");
  });

  it("重新生成要点两下确认，不用 window.confirm（Tauri 里不弹）", () => {
    expect(STEP).not.toContain("window.confirm");
    expect(STEP).toContain("setConfirmRegen(true)");
  });

  it("存完重新校验", () => {
    expect(STEP).toContain("cinemaValidateStoryboard(projectId, episodeId)");
    const i = STEP.indexOf("useEffect(() => {\n    void refreshIssues();");
    expect(i, "镜头表变了没有重新校验").toBeGreaterThan(0);
  });

  it("详情面板：改完失焦保存这一镜（带上草稿从哪一份开始改的）；当前线路的提示词能改，改的写回当前线路", () => {
    expect(DETAIL).toContain("onBlur={() => save()}");
    expect(DETAIL).toContain("const ok = await onSave(next, base.current);");
    expect(DETAIL).toContain("const current = draft.prompts?.[route];");
    expect(DETAIL).toContain("[route]: { frame: \"\", video: \"\", synthesized_at: 0, ...current, ...p }");
    expect(DETAIL).toContain("onClick={onSynthesize}");
  });

  it("合成提示词走后台任务：整集按钮、单镜按钮、没通过的镜头要显示", () => {
    expect(STEP).toContain("promptsState(jobs)");
    expect(STEP).toContain("onClick={() => setSynthDialog(true)}");
    // 合成弹窗左 A 右 B：各自带上自己的线路发出去，不再按出片线路
    expect(STEP).toContain("onStart={(route, ids) => cinemaSynthesizePrompts(project.id, episode.id, ids, route)");
    expect(STEP).toContain("running={runningRoute(promptJob)}");
    // 卡片上的 A / B 标签：过期的（比如刚出了首帧）不能还显示绿色「已合成」
    expect(STEP).toContain('stale ? "要重合成"');
    expect(STEP).toContain("cinemaStalePrompts(projectId, episodeId)");
    // 单镜合成走「加入合成队列」：这一集在合成时排到那一轮后面，没在合成就直接开（2026-10-06）
    expect(STEP).toContain("cinemaQueueSynthesize(project.id, episode.id, [shotId])");
    expect(STEP).toContain("onSynthesize={() => queueSynthesize(selected.id)}");
    expect(STEP).toContain("synthQueued={queuePosition(promptJob, selected.id)}");
    // 没通过的镜头进提醒栏的「没成功」那一组（默认展开），不和「有话要说」混
    expect(STEP).toContain("items: synthNotes }");
  });

  /** 合成是一镜一镜落盘的：只在整轮结束时重读，前面合成好的镜头要干等到最后才显示。 */
  /** 2026-10-06 用户截图：卡片「批量出片中 · 1/1」、通知条「还剩 2 镜」—— 等前一镜时两处同一个说法 */
  it("批量还在等前一镜时，卡片和通知条说法一致", () => {
    expect(STEP).toContain("const waiting = batchJob ? batchWaitingOn(batchJob, jobs) : undefined;");
    expect(STEP).toContain("镜在出 · 还剩 ${(batchJob?.targets?.length ?? 0) + 1} 镜`");
  });

  it("合成进度一动就重读分集，逐镜显示状态", () => {
    expect(STEP).toContain("if (synthesizing && synthDone > 0) void reload();");
    expect(STEP).toContain("}, [synthesizing, synthDone]);");
    expect(STEP).toContain("round={promptRound(selected, promptJob, route)}");
    expect(DETAIL).toContain('const busy = round === "running" || round === "queued";');
    expect(DETAIL).toContain("disabled={busy}");
    expect(DETAIL).toContain('? "加入合成队列"');
  });

  it("出片：后台跑不等结果；保存详情时不拿草稿里的旧出片结果覆盖（后端 cinema_save_shot 以盘上为准，有 Rust 测试）", () => {
    expect(STEP).toContain("cinemaRenderShotVideo({ project_id: project.id, episode_id: episode.id, shot_id: shotId }).catch(");
    expect(STEP).toContain('rendering={runningKind(jobs, selected.id) === "video"}');
    expect(STEP).toContain("cinemaSaveShot(project.id, episode.id, edited, base)");
    // 有片在出时不再拿 videoRunning 灰掉按钮，改成「加入队列」（2026-10-05）
    expect(DETAIL).toContain("renderBlocker(shot, route, issues, false)");
    expect(DETAIL).toContain("<TakeList shot={shot} route={route}");
    // 每一「次」都能点回去 —— takes 是版本链，只陈列不给换等于单向操作
    expect(DETAIL).toContain("cinemaPickTake(projectId, episodeId, shot.id, path)");
    // 挂的是哪几张要看得见 —— 只说「挂 2 张图」答不出「用了哪张场景图」
    expect(DETAIL).toContain("t.picture_paths?.length");
  });

  it("批量出片：后台任务、点两下确认、跳过和失败的列出来、批量时单镜不能插队", () => {
    // **长镜头的场也在这一次里，按场次顺序夹在逐镜中间，一个任务出完**（顺序本身的测试在 storyboardView）。
    // 不能再单独先提交长镜头：后面那一下批量会被「这一集有镜头正在出片」拒掉（2026-09-29 夜里）
    expect(STEP).toContain("const units = batchUnits(shots, plan.ids, plan.longTakeScenes);");
    expect(STEP).toContain("cinemaRenderEpisodeVideos(project.id, episode.id, units)");
    const renderAll = STEP.slice(STEP.indexOf("const renderAll = async"), STEP.indexOf("const renderFrames = async"));
    expect(renderAll).not.toContain("cinemaRenderLongTake");
    expect(STEP).toContain('onClick={() => setRenderDialog({ kind: "batch" })}');
    expect(STEP).toContain("items: batchNotes }");
    expect(STEP).toContain('const videoRunning = batching || jobs.some((j) => j.kind === "video" && j.status === "running");');
  });

  it("批量出片可以停：当前这镜跑完就停，点过之后按钮说清在等", () => {
    expect(STEP).toContain("cinemaCancelJob(batchJob.id)");
    expect(STEP).toContain('batchJob.cancel_requested ? "停止中 · 当前这镜跑完就停" : "停止批量出片"');
  });

  it("批量出片和单镜出片都先弹出片选项卡，选好再开始", () => {
    expect(STEP).toContain('onRender={() => setRenderDialog({ kind: "single", shotId: selected.id })}');
    // 单镜走 render，批量和按场（2026-09-27）都走 renderAll
    expect(STEP).toContain('onStart={(p) => (renderDialog.kind === "single" ? p.ids[0] && render(p.ids[0]) : void renderAll(p))}');
    expect(STEP).toContain('setRenderDialog({ kind: "scenes", sceneNos: [] })');
    // 勾了整段出的那几场要传进对话框，否则它显示的镜数少算
    expect(STEP).toContain("longTakeScenes={episode.long_take_scenes ?? []}");
    expect(STEP).toContain("disabled={generating || synthesizing || videoRunning || !anyBatch}");
  });

  it("页头三个按钮都有悬浮提示；出过片的显示重新出片", () => {
    expect(STEP).toContain("title={hints.batch}");
    expect(STEP).toContain("title={hints.synth}");
    expect(STEP).toContain("title={hints.regen}");
    // 文案按行为钉（以前钉的是源码里那串三元表达式，换种写法就红、却测不到行为）
    expect(renderButtonLabel(false, false, false)).toBe("出片");
    expect(renderButtonLabel(false, true, false)).toBe("重新出片");
    // 整段出的场：这个按钮只出这一镜（用户 10-01）
    expect(renderButtonLabel(false, false, true)).toBe("单独出片");
    expect(renderButtonLabel(false, true, true)).toBe("重新单独出片");
    expect(renderButtonLabel(true, true, true)).toBe("出片中…");
    expect(DETAIL).toContain("renderButtonLabel(rendering, renderTakes(shot, route).length > 0, longTake)");
    // 线路在出片弹窗里选，按钮上不再写死线路
    expect(DETAIL).not.toContain("出片\"} · 线路");
    expect(STEP).not.toMatch(/批量出片"\} · \$\{batch\.ids\.length\} 镜 · 线路/);
  });

  /** 用户 2026-09-29：「顶上这两排按钮和进度，越来越 confuse……能不能重新设计一下」 */
  it("页头是一排四步：镜头 → 提示词 → 首帧 → 出片，只有下一步高亮、它的主按钮是主色", () => {
    const at = (n: number) => STEP.indexOf(`<StepCard\n          no={${n}}`);
    expect(at(1)).toBeGreaterThan(0);
    expect(at(1)).toBeLessThan(at(2));
    expect(at(2)).toBeLessThan(at(3));
    expect(at(3)).toBeLessThan(at(4));
    expect(STEP).toContain('current={next === "shots"}');
    expect(STEP).toContain('current={next === "synth"}');
    expect(STEP).toContain('current={next === "render"}');
    expect(STEP).toContain('primary={next === "shots"}');
    expect(STEP).toContain('primary={next === "synth"}');
    expect(STEP).toContain('primary={next === "render"}');
    // 原来那排编了号的按钮和那排编了号的状态都没了 —— 两套编号意思不一样，就是 confuse 的来源
    expect(STEP).not.toContain("<FlowButton");
    expect(STEP).not.toContain("<StepPill");
    // 顶上只留队列
    const action = STEP.slice(STEP.indexOf("<StepAction>"), STEP.indexOf("</StepAction>"));
    expect(action).toContain("<QueueButton");
    expect(action).not.toContain("<button");
  });

  it("首帧那一步说清出完会自动补合成；横幅合成一条提醒栏", () => {
    expect(STEP).toContain("出完自动合成那几镜（只合一次，不花第二轮）");
    // 首帧排在提示词前面（2026-10-04）：先合成的话出完首帧要再合一遍
    expect(STEP.indexOf('title="首帧"')).toBeLessThan(STEP.indexOf('title="提示词"'));
    expect(STEP).toContain('primary={next === "frames"}');
    expect(STEP).toContain("<NoticePanel");
    // 重新生成镜头收进「更多」，照样要点两次
    const more = STEP.slice(STEP.indexOf("<MoreMenu>"), STEP.indexOf("</MoreMenu>"));
    expect(more).toContain("再点一次确认");
    expect(more).toContain("这一集不分镜");
  });

  /** 详情面板有本地草稿：合成完不重挂，草稿还是旧提示词，一失焦就把新合成的覆盖掉。 */
  it("合成完详情面板重挂，不拿旧草稿覆盖新提示词", () => {
    expect(STEP).toContain("key={`${selected.id}:${selected.prompts?.a?.synthesized_at ?? 0}:${selected.prompts?.b?.synthesized_at ?? 0}:${detailRev}`}");
  });

  /** 导演世界关掉时也要重挂：草稿是打开之前的，一失焦就把导演世界写进分镜的站位、机位冲回去 */
  it("关掉导演世界后详情面板重挂", () => {
    const close = STEP.slice(STEP.indexOf("<ShotDirectorWorld"));
    expect(close.slice(0, close.indexOf("/>"))).toContain("setDetailRev((n) => n + 1)");
  });
});

/** 2026-09-30 EP02：「自动修正 5 处」点了没反应 —— 按问题文字猜能修的，后端一处都改不了。改时长后时间线不跟着变 */
describe("自动修正和改时长", () => {
  it("按钮的数字用后端试修出来的那份，点完说改了几处", () => {
    expect(STEP).toContain("setRepairable(await cinemaRepairableIssues(projectId, episodeId).catch(() => []));");
    expect(STEP).toContain("{repairable.length > 0 && (");
    expect(STEP).toContain("自动修正 {repairable.length} 处");
    expect(STEP).not.toContain("issues.filter(isRepairable)");
    expect(STEP).toContain('title: "没有能自动改的"');
  });

  it("时长框失焦时分段时间线跟着伸缩（retime.ts）", () => {
    expect(DETAIL).toContain("onBlur={() => void saveDuration()}");
    expect(DETAIL).toContain("const next = withDuration({ ...draft, duration: from }, to);");
  });
});

/** 2026-09-30 用户：「没有修复按钮了，建议在红色问题提示框右侧增加修复按钮」 */
describe("问题旁边的修复按钮", () => {
  it("改法由后端给，分镜页按镜头传给详情；「自动修正」顶上和问题旁边共用一个", () => {
    expect(STEP).toContain("setFixes(await cinemaIssueFixes(projectId, episodeId).catch(() => []));");
    expect(STEP).toContain("fixes={fixes.filter((f) => f.shot_num === selected.shot_num)}");
    expect(STEP).toContain("onRepairAll={() => void runRepair()}");
    expect(STEP).toContain("onClick={() => void runRepair()}");
  });

  it("按钮挂在对应那条问题右边；加长走改时长同一段伸缩，改说话人走 dialogue_lines，然后正常保存", () => {
    expect(DETAIL).toContain("const fix = fixes.find((f) => f.message === i.message);");
    expect(DETAIL).toContain("onClick={() => void applyFix(fix)}");
    expect(DETAIL).toContain("const next = withDuration(draft, f.secs);");
    expect(DETAIL).toContain("j === f.line ? { ...l, speaker: f.speaker, off_screen: l.off_screen || f.off_screen } : l");
    expect(DETAIL).toContain("await save(next);");
  });
});

