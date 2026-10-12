import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const read = (p: string) => readFileSync(new URL(p, import.meta.url), "utf8");

/**
 * 用户 2026-09-21：「增加取消功能，ComfyUI 已经取消了，画布里还在转。」
 *
 * 两件事，缺一件都不算修好：
 * ① 画布上能取消，而且是**真的**让 ComfyUI 撤；
 * ② ComfyUI 那边没了，画布自己要看得出来。
 */
describe("画布取消", () => {
  const wrapper = read("../nodes/BaseNodeWrapper.tsx");
  const button = read("./CancelJobButton.tsx");

  /** 写在壳上，图片/视频/超分/补帧/深度/长镜头就一处都不会漏。 */
  it("取消按钮挂在所有节点共用的壳上，只在生成时出现", () => {
    expect(wrapper).toContain("{isGenerating && <CancelJobButton nodeId={id} />}");
  });

  /** 没有 jobId 时后端找不到任何 prompt 可撤 —— 给一个撤不掉的按钮比没有更糟。 */
  it("没有 jobId 就不显示按钮", () => {
    expect(button).toContain("if (!jobId) return null;");
  });

  /**
   * **后端那句话要原样显示。** 它会在「没找到对应的 ComfyUI 任务」时明说，
   * 吞掉换成一句「已取消」就又回到「界面说停了、ComfyUI 还在跑」那个病。
   */
  it("把后端返回的原话显示出来，不自己编一句", () => {
    expect(button).toContain("setMsg(await canvasJobCancel(jobId))");
  });

  /** 节点里的按钮不加 nodrag，点一下节点就跟着鼠标跑 —— 09-21 撞过两次。 */
  it("按钮带 nodrag nopan", () => {
    expect(button).toMatch(/className="nodrag nopan[^"]*"\s*\n?\s*>/);
  });
});

/**
 * **后端必须既查 `/history` 也查 `/queue`。**
 *
 * 只查 history 的话，任务在 ComfyUI 那边被取消（或 ComfyUI 重启）之后
 * 什么都不会写进 history，轮询会一路等下去（分镜这条 09-30 起不再按时长判超时，看 ComfyUI 的进度）。
 */
describe("后端的下落判断", () => {
  const video = read("../../../src-tauri/src/agent_runtime/video_tool.rs");

  it("提交视频后轮询会查队列，不是只查 history", () => {
    const fn = video.slice(video.indexOf("pub(crate) async fn submit_and_fetch_video"));
    const body = fn.slice(0, fn.indexOf("\npub(crate) async fn render_shot_video"));
    // **查的是整条计数表达式，不是「lost_streak >= 2」那半句。**
    // 只查阈值的话，把累加改成 `lost_streak = 0` 照样绿 —— 而那就等于
    // 判断从来不会成立。09-21 的三条假绿全是这个形状：只查片段不查上下文。
    expect(body).toContain("lost_streak = if super::comfy_jobs::queue_contains(&q, &prompt_id) { 0 } else { lost_streak + 1 };");
    expect(body).toContain("if lost_streak >= 2 {");
  });

  /**
   * **连不上 ComfyUI ≠ 任务没了。** 读不出队列时必须当成「还在」——
   * 误判成没了会把已经跑了半小时的任务丢掉。
   */
  it("队列读不出来时继续等，不判成丢了", () => {
    const fn = video.slice(video.indexOf("pub(crate) async fn submit_and_fetch_video"));
    const body = fn.slice(0, fn.indexOf("\npub(crate) async fn render_shot_video"));
    // 读不出 history / 队列：只记成「这一轮连不上」—— 不碰丢失计数（不判丢），
    // 连着 15 分钟都连不上才算 ComfyUI 退了（`comfy_watch::UNREACHABLE`），不会无限等
    const unreachable = body.match(/None => super::comfy_watch::Seen::Unreachable,/g) ?? [];
    expect(unreachable.length).toBe(2);
    const lostAt = body.indexOf("lost_streak = if");
    expect(body.slice(body.indexOf("let queue = match"), lostAt)).toContain("None => super::comfy_watch::Seen::Unreachable,");
  });

  /** 取消标记每轮自己看一眼 —— 不用等 ComfyUI 那边有反应。 */
  it("轮询每轮检查取消标记", () => {
    expect(video).toContain("comfy_jobs::cancel_requested()");
  });

  /** prompt_id 要记到当前任务名下，取消时才找得到该撤哪一个。 */
  it("提交后把 prompt_id 记到当前任务名下", () => {
    expect(video).toContain("comfy_jobs::attach(&prompt_id)");
  });
});
