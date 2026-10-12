import { describe, expect, it } from "vitest";
import { parseScript, speakerInfo } from "./format";
import { derive, deriveShots } from "./derive";
import { textToDoc, docToText, serializeDoc, loadDoc } from "./docConvert";

const script = `# 第3集 · 录音笔
---
3-1 屏蔽室内部 夜 内
人物：林深、零
△ 光点缓缓亮起。
林深 OS：等等。
零（中性、温和）：你好。
苏晚（录音，极轻）：替我看看这个世界。
3-2 屏蔽室桌旁 内 深夜
林深（声音沙哑）：我听见了。
字幕：四年后。`;
describe("中文制片格式", () => {
  it("recognizes numbered scenes in both field orders without phantom title scenes", () => {
    const result = derive(textToDoc(script));
    expect(result.scenes).toHaveLength(2);
    expect(result.scenes.map((s) => [s.location, s.timeOfDay])).toEqual([["屏蔽室内部", "夜"], ["屏蔽室桌旁", "深夜"]]);
    expect(result.characters.map((c) => c.name).sort()).toEqual(["林深", "苏晚", "零"].sort());
  });
  it("accepts fused location/time from EP06 and counts silent cast members", () => {
    const result = derive(textToDoc("6-11 废墟清晨 外\n人物：林深、零\n林深：早。"));
    expect(result.scenes[0]).toMatchObject({ location: "废墟", timeOfDay: "清晨", characters: ["林深", "零"] });
    expect(result.characters.find((c) => c.name === "零")?.lines).toBe(0);
  });
  it("keeps annotations and OS stable through text round trips", () => {
    const once = docToText(textToDoc(script));
    expect(docToText(textToDoc(once))).toBe(once);
    expect(derive(textToDoc(once))).toEqual(derive(textToDoc(script)));
  });
  it("stores speaker, delivery and audio metadata without changing source wording", () => {
    const doc = loadDoc(serializeDoc(textToDoc(script)), "");
    const cue = doc.content.find((n) => n.attrs?.name === "苏晚");
    expect(cue?.attrs).toEqual({ name: "苏晚", delivery: "录音，极轻", audioType: "recording" });
    expect(docToText(doc)).toContain("苏晚（录音，极轻）");
    expect(speakerInfo("零 OS").audioType).toBe("inner");
  });
  it("does not treat metadata or captions as characters", () => {
    expect(parseScript("字幕：四年后。")[0].type).toBe("caption");
    const doc = textToDoc("---\ntitle: 小说\nstatus: draft\n---\n# 第一章\n从前有座山。");
    expect(derive(doc).characters).toEqual([]);
  });
  it("keeps off-screen speech out of visual character instructions in derived drafts", () => {
    const shot = deriveShots(textToDoc(script)).find((s) => s.speaker === "苏晚");
    expect(shot?.audioType).toBe("recording");
    expect(shot?.prompt).not.toContain("苏晚");
  });
});
