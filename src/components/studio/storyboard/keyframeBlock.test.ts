import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

/** 用户 2026-09-28：「没有首尾帧提示词吗？直接就出来了，不对都没法修改。」 */
describe("关键帧：首帧 / 尾帧提示词看得见、改得了、先存再出", () => {
  const block = readFileSync(fileURLToPath(new URL("./KeyframeBlock.tsx", import.meta.url)), "utf8");
  const detail = readFileSync(fileURLToPath(new URL("./ShotDetail.tsx", import.meta.url)), "utf8");
  it("首帧、尾帧下面各有提示词框；首帧是当前线路的分镜提示词，尾帧是「结束那一刻的画面」，留空时说明用最后一段", () => {
    expect((block.match(/<PromptBox/g) ?? []).length).toBe(2);
    expect(detail).toContain("framePrompt={draft.prompts?.[route]?.frame ?? \"\"}");
    expect(detail).toContain("onEndFrame={(v) => patch({ end_frame: v })}");
    expect(block).toContain("留空 = 用最后一段画面：");
  });
  // 2026-10-03：首帧提示词空着也能出 —— 后端按镜头表现拼一份（不调模型），省掉「先合成再出首帧再合成」的第一轮
  it("出首帧 / 出尾帧 / 查看实际发送都先把草稿存下来（后端读盘上的）；首帧提示词空着也能出", () => {
    expect((block.match(/await saved\(\);/g) ?? []).length).toBe(3);
    expect(block).not.toContain("disabled: !framePrompt.trim(),");
    expect(block).toContain("留空 = 按镜头表自动拼一份");
  });
  it("查看实际发送走后端同一个拼参数的函数；最下面那份重复的首帧框去掉了", () => {
    expect(block).toContain("cinemaKeyframePreview(projectId, episodeId, shot.id, which)");
    expect(detail).not.toContain("分镜提示词（出首帧用）");
  });
  it("挂了首帧时说清楚：出片只发首帧，参考图不再单独发", () => {
    // 2026-10-07 起在「出片方式」那一块说，参考图区折成「出首帧用的参考图（出片不发）」
    expect(detail).toContain("挂了首帧：出片只发首帧");
    expect(detail).toContain("出首帧用的参考图（出片不发）");
  });
});
