import { describe, expect, it } from "vitest";
import { docToText, loadDoc, serializeDoc, textToDoc } from "./docConvert";

const SCRIPT = `【场景：老茶馆】
老茶师在窗边擦拭一只黄铜壶。
老茶师：这茶，要等。
小徒弟
（急切地）
师父，客人来了。
切至：`;

describe("纯文本 → 文档", () => {
  it("每一行变成一个对应类型的块", () => {
    const doc = textToDoc(SCRIPT);
    expect(doc.content.map((n) => n.type)).toEqual([
      "sceneHeading",
      "action",
      "characterCue",
      "dialogue",
      "characterCue",
      "parenthetical",
      "dialogue",
      "transition",
    ]);
  });

  it("空文本给一个空的动作块，不是空文档", () => {
    // ProseMirror 的 doc 要求 `block+`，真空文档它自己会去填，
    // 填的过程有爆栈的历史，不如我们直接给一个。
    const doc = textToDoc("");
    expect(doc.content).toHaveLength(1);
    expect(doc.content[0].type).toBe("action");
  });
});

describe("文档 → 纯文本", () => {
  it("内容一个字不丢", () => {
    const text = docToText(textToDoc(SCRIPT));
    for (const w of ["老茶馆", "黄铜壶", "这茶，要等。", "急切地", "客人来了", "切至"]) {
      expect(text, w).toContain(w);
    }
  });

  /**
   * **来回一趟必须稳定。** 纯文本是真相，结构是附加的 ——
   * 转一次换一个样的话，每次保存都会让剧本悄悄变形。
   */
  it("来回转换收敛：第二次和第一次一样", () => {
    const once = docToText(textToDoc(SCRIPT));
    const twice = docToText(textToDoc(once));
    expect(twice).toBe(once);
  });

  it("来回转换后结构也一样", () => {
    const a = textToDoc(SCRIPT).content.map((n) => n.type);
    const b = textToDoc(docToText(textToDoc(SCRIPT))).content.map((n) => n.type);
    expect(b).toEqual(a);
  });

  it("角色名和台词之间不插空行", () => {
    // 插了的话再解析时台词会退化成动作。
    const text = docToText(textToDoc("老茶师：这茶，要等。"));
    expect(text).toBe("老茶师\n这茶，要等。");
  });

  it("对白之后回到叙述要空行", () => {
    // 不空的话，那段叙述会被当成上一句台词的延续。
    const text = docToText(textToDoc("老茶师：等。\n他放下壶。"));
    expect(text).toBe("老茶师\n等。\n\n他放下壶。");
    expect(textToDoc(text).content.map((n) => n.type)).toEqual([
      "characterCue",
      "dialogue",
      "action",
    ]);
  });

  /**
   * **用户在编辑器里手动把一行标成「角色」时，那个身份不能在保存后丢掉。**
   *
   * 这一行的名字可能只出现这一次、后面也没有括号提示 ——
   * 两个上下文信号都不在，全靠 `docToText` 在它前面留一个空行来断开：
   * 不留的话，重新载入时它会退化成上一句台词的延续。
   */
  it("手动标的角色名，存完再读还是角色名", () => {
    const doc = {
      type: "doc",
      content: [
        { type: "characterCue", content: [{ type: "text", text: "老茶师" }] },
        { type: "dialogue", content: [{ type: "text", text: "等。" }] },
        { type: "characterCue", content: [{ type: "text", text: "小徒弟" }] },
        { type: "dialogue", content: [{ type: "text", text: "是。" }] },
      ],
    };
    const text = docToText(doc);
    expect(textToDoc(text).content.map((n) => n.type)).toEqual([
      "characterCue",
      "dialogue",
      "characterCue",
      "dialogue",
    ]);
  });

  it("不是文档时回空串，不炸", () => {
    expect(docToText(null)).toBe("");
    expect(docToText({ nope: 1 })).toBe("");
  });
});

describe("载入", () => {
  it("有结构文档就用它", () => {
    const doc = serializeDoc(textToDoc("老茶师：等。"));
    const loaded = loadDoc(doc, "完全不同的纯文本");
    expect(docToText(loaded)).toContain("老茶师");
  });

  /** 老数据没有 `script_doc` —— 不能因此给个空文档，那样剧本一打开就没了。 */
  it("没有结构文档就解析纯文本", () => {
    const loaded = loadDoc(null, SCRIPT);
    expect(loaded.content.map((n) => n.type)[0]).toBe("sceneHeading");
  });

  it("结构文档坏了也退回纯文本，不丢内容", () => {
    const loaded = loadDoc("{ 这不是 json", SCRIPT);
    expect(docToText(loaded)).toContain("老茶馆");
  });

  it("结构文档是空的也退回纯文本", () => {
    expect(docToText(loadDoc('{"type":"doc","content":[]}', SCRIPT))).toContain("老茶馆");
  });
});

describe("序列化", () => {
  it("只留下类型和文本，不带编辑器的其它字段", () => {
    const raw = {
      type: "doc",
      content: [
        { type: "action", attrs: { id: "x" }, content: [{ type: "text", text: "他", marks: [] }] },
      ],
    };
    expect(JSON.parse(serializeDoc(raw))).toEqual({
      type: "doc",
      content: [{ type: "action", content: [{ type: "text", text: "他" }] }],
    });
  });
});
