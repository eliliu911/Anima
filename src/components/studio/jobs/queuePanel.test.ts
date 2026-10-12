import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const strip = (src: string) => src.replace(/\/\*[\s\S]*?\*\//g, "").replace(/^\s*\/\/.*$/gm, "");
const PANEL = strip(readFileSync(fileURLToPath(new URL("./QueuePanel.tsx", import.meta.url)), "utf8"));

describe("任务队列接线", () => {
  /** 只靠轮询的话，点完「生成」最长要等 6 秒按钮和角标才动。 */
  it("听后端的任务事件立刻刷新，只认本集的，卸载时退订", () => {
    const i = PANEL.indexOf("export function useJobs");
    const body = PANEL.slice(i, PANEL.indexOf("export function QueueButton", i));
    expect(body).toContain("listen<Job>(JOB_EVENT");
    expect(body).toContain("e.payload.project_id !== projectId || e.payload.episode_id !== episodeId");
    expect(body).toContain("void off.then((f) => f())");
  });

  it("任务行标题走 jobTitle，镜头表任务才有名字", () => {
    expect(PANEL).toContain("{jobTitle(job)}");
  });
});
