import { describe, expect, it } from "vitest";
import { VOICE_FIELD_HINT, VOICE_FIELD_LABEL, engineSourceNote, unsetOptionLabel } from "./voiceFieldCopy";

describe("配音音色那一栏的措辞", () => {
  /**
   * 用户 2026-09-18 的原话：「音色跟全局设置是什么意思？没办法不选？
   * 那不是会一直用音色？」—— 担心的是「是不是常开」。
   */
  it("**说明第一句就要回答「不配音就用不到」** —— 那正是他担心的事", () => {
    expect(VOICE_FIELD_HINT.indexOf("只有在组装页点了「配音」才用得到")).toBe(0);
    expect(VOICE_FIELD_HINT).toContain("没有任何作用");
    expect(VOICE_FIELD_HINT).toContain("原声");
  });

  it("**栏目名必须带「配音」** —— 光写「音色」会和 H3 提示词里的音色描述混", () => {
    expect(VOICE_FIELD_LABEL).toContain("配音");
  });

  it("**空选项不许写「跟全局设置」** —— 听起来像此刻正在生效", () => {
    const l = unsetOptionLabel("Edge 朗读（云端）");
    expect(l).not.toContain("跟全局设置");
    expect(l).toContain("不指定");
    expect(l).toContain("配音时");
  });

  it("引擎名放括号里跟在后面，不嵌套 —— 两层括号本身就难读", () => {
    expect(unsetOptionLabel("Edge 朗读（云端）")).toBe("不指定 · 配音时用全局音色（Edge 朗读（云端））");
    // 引擎名拿不到时不留空括号
    expect(unsetOptionLabel("")).toBe("不指定 · 配音时用全局音色");
    expect(unsetOptionLabel("  ")).toBe("不指定 · 配音时用全局音色");
  });
});

describe("音色是哪来的", () => {
  /**
   * 用户 2026-09-18：「我只看到了一个引擎，另外两个引擎没有在列表里。
   * EDGE 里的音色都是比较刻板的」—— 六个引擎其实都在，
   * 列表只是跟着当前选中的那个走，而他不知道能换。
   */
  it("**必须说「换个引擎就是另一批」** —— 光写引擎名读起来像「音色就这些」", () => {
    const n = engineSourceNote("Edge 朗读（云端）", 14);
    expect(n).toContain("换个引擎就是另一批");
    expect(n).toContain("设置 → 语音");
    expect(n).toContain("14 个");
  });

  it("**必须说引擎是全局一个** —— 否则为配音换了引擎，助手的嗓子也变了会莫名其妙", () => {
    expect(engineSourceNote("Edge", 14)).toContain("全局一个");
    expect(engineSourceNote("Edge", 14)).toContain("语音助手");
  });

  it("没有音色可列时也要说清是哪个引擎没有，不是含糊带过", () => {
    const n = engineSourceNote("CosyVoice2（本地克隆）", 0);
    expect(n).toContain("CosyVoice2（本地克隆）没有可列的音色");
    expect(n).toContain("换个引擎");
  });

  it("引擎名拿不到时不留空", () => {
    expect(engineSourceNote("", 3)).toContain("当前引擎");
  });
});
