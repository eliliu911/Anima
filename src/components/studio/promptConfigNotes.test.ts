import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const read = (rel: string) => readFileSync(fileURLToPath(new URL(rel, import.meta.url)), "utf8");

/**
 * 2026-09-29 用户问「这个提示词设置页面里的内容，还在生效是吧」。
 *
 * 生效，但只是一半：镜头表 / 细化时间线 / 线路 B 的拍法规则在 `{METHOD}` 背后，
 * 是编译进程序的技能原文，编辑框里只有那一个词。页面不说，就会以为那几行是全部。
 * 同一页的页脚还写着「默认 LM Studio」—— 后台默认早换成了 Azure 的 gpt-5.6-sol。
 */
describe("提示词设置页说清楚规则从哪来", () => {
  const dialog = read("./PromptConfigDialog.tsx");

  it("有 {METHOD} 的槽位标出是哪家技能的原文、这里改不了", () => {
    // 来源由后端给（和实际灌进去的那份有测试对着），前端不自己写一张对照表
    expect(dialog).toContain("active?.method_source &&");
    expect(dialog).toContain("{active.method_source}");
    expect(dialog).toContain("这里改不了");
    expect(dialog).not.toMatch(/ai-film|cinematic-video-prompt-engineer/);
  });

  it("页脚不再写死某个后端", () => {
    expect(dialog).not.toContain("默认 LM Studio");
    expect(dialog).toContain("链路分配");
    expect(dialog).toContain("后台推理");
  });
});
