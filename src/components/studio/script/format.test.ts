import { describe, expect, it } from "vitest";
import { classifyLine, matchRate, parseScript, type LineType } from "./format";

const t = (s: string, prev: LineType | null = null) => classifyLine(s, prev).type;

describe("场景标题", () => {
  it("西文格式", () => {
    expect(t("INT. 办公室 - 日")).toBe("sceneHeading");
    expect(t("EXT. 老茶馆 - 雨天午后")).toBe("sceneHeading");
    expect(t("int. bedroom - night")).toBe("sceneHeading");
  });

  it("中文直译格式", () => {
    expect(t("内景. 会议室 - 黄昏")).toBe("sceneHeading");
    expect(t("外景 街道 - 夜")).toBe("sceneHeading");
  });

  it("中文剧本常见的方括号格式", () => {
    // 我们自己的语料就是这个写法，参考项目那份不认。
    expect(t("【场景：老茶馆】")).toBe("sceneHeading");
    expect(t("【地点：豪华顶层会议室】")).toBe("sceneHeading");
  });

  it("日式的 ○ 开头", () => {
    expect(t("○ 茶館・雨")).toBe("sceneHeading");
  });

  it("正文里提到 int 不算标题", () => {
    expect(t("他的 interest 在别处")).not.toBe("sceneHeading");
  });
});

describe("角色与对白", () => {
  it("「角色：台词」拆成两行", () => {
    const out = parseScript("老茶师：这茶，要等。");
    expect(out.map((l) => l.type)).toEqual(["characterCue", "dialogue"]);
    expect(out[0].text).toBe("老茶师");
    expect(out[1].text).toBe("这茶，要等。");
  });

  it("带造型后缀的名字也认", () => {
    const out = parseScript("张三（婚纱）：我愿意。");
    expect(out[0].text).toBe("张三（婚纱）");
    expect(out[1].text).toBe("我愿意。");
  });

  /**
   * **名字必须限长。**
   *
   * 不限的话，「他看着窗外，心想：这茶要等」整句会被当成一个叫
   * 「他看着窗外，心想」的角色 —— 资产库里凭空多一个人，而且不报错。
   */
  it("长句里的冒号不当成角色名", () => {
    expect(t("他久久地看着窗外的雨，心里想着：这茶要等")).toBe("action");
  });

  it("元信息不当成对白", () => {
    for (const s of ["时间：黄昏日落", "人物：林辰、苏沐雪", "镜头：推近"]) {
      expect(t(s), s).toBe("action");
    }
  });

  it("单独一行的短名字算角色名", () => {
    expect(t("老茶师")).toBe("characterCue");
    expect(t("YUMIKO")).toBe("characterCue");
  });

  it("紧跟角色名的一行是对白", () => {
    expect(t("这茶，要等。", "characterCue")).toBe("dialogue");
  });

  it("上一行是对白时，短句不再当角色名", () => {
    // 否则连续两句台词的第二句会被当成新角色。
    expect(t("我知道了", "dialogue")).toBe("action");
  });

  it("句末有标点的短句不是角色名", () => {
    expect(t("下雨了。")).toBe("action");
  });
});

describe("括号提示与转场", () => {
  it("括号提示", () => {
    expect(t("（低声）")).toBe("parenthetical");
    expect(t("(beat)")).toBe("parenthetical");
  });

  it("括号提示后面还是对白", () => {
    expect(t("这茶，要等。", "parenthetical")).toBe("dialogue");
  });

  it("转场", () => {
    for (const s of ["CUT TO:", "FADE OUT", "淡出", "切至："]) {
      expect(t(s), s).toBe("transition");
    }
  });

  it("整句里的「淡出」不算转场", () => {
    expect(t("灯光淡出，他转身离开")).toBe("action");
  });
});

describe("方括号标注不是对白", () => {
  /**
   * 中文剧本里 `【时间：黄昏】`、`【人物：林辰】` 很常见。
   * 不挡的话，「角色：台词」那条会把它读成一个叫「【时间」的角色 ——
   * 资产库里凭空多出这么个「人」，而且不报错。
   */
  it("整行包在方括号里的算标注", () => {
    for (const s of ["【时间：雨天午后】", "【人物：林辰、苏沐雪】", "【道具：黄铜壶】"]) {
      expect(t(s), s).toBe("action");
    }
  });

  it("但【场景：…】仍然是场景标题", () => {
    expect(t("【场景：老茶馆】")).toBe("sceneHeading");
  });
});

describe("对白后面那个短行 —— 格式本身的歧义", () => {
  /**
   * 紧跟在对白后的短行，既可能是下一句台词，也可能是新角色名。
   * 单看这一行分不出来，靠两个上下文信号补。
   */
  it("下一行是括号提示 → 一定是角色名", () => {
    const out = parseScript("老茶师：等。\n小徒弟\n（急切地）\n师父。");
    expect(out.map((l) => l.type)).toEqual([
      "characterCue", "dialogue", "characterCue", "parenthetical", "dialogue",
    ]);
  });

  it("这个名字在别处以角色身份出现过 → 也是角色名", () => {
    const out = parseScript("小徒弟：客人来了。\n老茶师：等。\n小徒弟\n是。");
    expect(out.map((l) => l.type)).toEqual([
      "characterCue", "dialogue", "characterCue", "dialogue", "characterCue", "dialogue",
    ]);
  });

  /**
   * 两个信号都没有时保守当台词延续 —— **猜错成角色名的代价更大**：
   * 资产库里会凭空多出一个叫「我知道了」的角色。
   */
  it("两个信号都没有时当台词延续，不当角色", () => {
    const out = parseScript("老茶师：这茶，要等。\n我知道了");
    expect(out.map((l) => l.type)).toEqual(["characterCue", "dialogue", "action"]);
  });
});

describe("整段解析", () => {
  const SCRIPT = `【场景：老茶馆】
【时间：雨天午后】

老茶师在窗边擦拭一只黄铜壶。

老茶师：这茶，要等。

小徒弟
（急切地）
师父，客人来了。

切至：`;

  it("认出各种类型", () => {
    const types = parseScript(SCRIPT).map((l) => l.type);
    expect(types).toEqual([
      "sceneHeading",
      "action", // 【时间：…】不是场景标题也不是对白
      "action",
      "characterCue",
      "dialogue",
      "characterCue",
      "parenthetical",
      "dialogue",
      "transition",
    ]);
  });

  it("空行丢掉，但会断开上下文", () => {
    // 不断开的话，跨段的描写会被当成上一段的对白。
    const out = parseScript("老茶师\n\n他擦着壶。");
    expect(out.map((l) => l.type)).toEqual(["characterCue", "action"]);
  });

  it("识别率能反映内容", () => {
    expect(matchRate(parseScript(SCRIPT))).toBeGreaterThan(0.5);
    // 一段散文一条规则都不该命中
    expect(matchRate(parseScript("从前有座山，山里有座庙。庙里有个老和尚。"))).toBe(0);
  });

  it("空输入不炸", () => {
    expect(parseScript("")).toEqual([]);
    expect(matchRate([])).toBe(0);
  });
});

describe("企划案的字段名不是角色", () => {
  /**
   * 用户粘了一份漫剧企划，资产库里凭空多出「类型」和「时间线」两个"角色"。
   * 这是实测出来的，不是假想。
   */
  it("类型 / 时间线 这类表头读成动作，不是对白", () => {
    const raw = [
      "类型： 玄幻、宿命爱情、悬疑、战斗",
      "时间线： 双帝之战结束五年后",
      "篇幅： 12 集",
    ].join("\n");
    const lines = parseScript(raw);
    expect(lines.map((l) => l.type)).toEqual(["action", "action", "action"]);
  });

  it("但真的角色名还是要认出来 —— 别为了挡表头把台词也挡了", () => {
    const lines = parseScript("云韵：我等你很久了。");
    expect(lines[0]).toMatchObject({ type: "characterCue", text: "云韵" });
    expect(lines[1]).toMatchObject({ type: "dialogue", text: "我等你很久了。" });
  });

  /**
   * 挡词表只有一份。两份的话，改了一处另一处还在放行 ——
   * 而表现只是「资产库里偶尔多个怪人」，没人会立刻发现。
   */
  /**
   * 认定人名有**三条**路：`角色：台词`、单独成行的短名字、
   * 以及整篇识别时那遍「这个名字在别处出现过」。挡词表要三条都拦住 ——
   * 第一版只拦了第一条，单独一行的「类型」照样变成角色。
   */
  it("三条认定路径都要拦住", () => {
    // 1. 角色：台词
    expect(classifyLine("类型： 玄幻", null).type).toBe("action");
    // 2. 单独成行的短名字
    expect(classifyLine("时间线", null).type).toBe("action");
    // 3. 整篇识别：先收集已知名字，再回头补判
    const lines = parseScript("类型： 玄幻\n\n类型");
    expect(lines.every((l) => l.type !== "characterCue")).toBe(true);
  });
});

/**
 * **Markdown 的强调记号会打败元信息判定。**
 *
 * 2026-09-12 实拍：用户粘进带 Markdown 头的剧本，
 * `**类型**：古风 / 穿越 / 爽文漫剧` 被认成了「角色：台词」——
 * `META_FIELD` 认得「类型」，但拿到的是 `**类型**`，`^类型$` 匹配不上。
 * 于是 `**类型**` 成了角色、进了资产库、出了设定卡，全程不报错。
 */
describe("带 Markdown 的元信息行", () => {
  it("加粗的元信息标签不是角色", () => {
    expect(t("**类型**：古风 / 穿越 / 爽文漫剧")).not.toBe("characterCue");
    expect(t("**基调**：热血 × 盛唐史诗感")).not.toBe("characterCue");
  });

  /** 词表里只有裸词「时长」，`时长建议` 第一版就是这么漏掉的。 */
  it("带后缀的元信息标签也不是角色", () => {
    expect(t("**时长建议**：60–90秒")).not.toBe("characterCue");
    expect(t("类型说明：古风")).not.toBe("characterCue");
  });

  it("单独成行的加粗标签也不是角色", () => {
    expect(t("**类型**")).not.toBe("characterCue");
    expect(t("**时长建议**")).not.toBe("characterCue");
  });

  /** **别误伤真名字。** 加粗的说话人要认出来，而且名字要剥干净 —— */
  it("加粗的真名字认得出来，而且剥干净", () => {
    const p = classifyLine("**林婉清**：这是……唐朝？", null);
    expect(p.type).toBe("characterCue");
    expect(p.text).toBe("林婉清");
  });

  it("单独成行的加粗名字也剥干净", () => {
    const p = classifyLine("**皇帝**", null);
    expect(p.type).toBe("characterCue");
    expect(p.text).toBe("皇帝");
  });
});
