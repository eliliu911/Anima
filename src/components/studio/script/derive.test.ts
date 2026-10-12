import { describe, expect, it } from "vitest";
import { textToDoc } from "./docConvert";
import { derive, deriveShots, splitHeading } from "./derive";

const SCRIPT = `【场景：老茶馆】

老茶师在窗边擦拭一只黄铜壶。

老茶师：这茶，要等。

小徒弟
（急切地）
师父，客人来了。

老茶师：让他等。

【场景：门外街道】

雨下得更大了。

小徒弟：那我去回话。`;

const doc = textToDoc(SCRIPT);

describe("场景标题拆解", () => {
  it("西文格式拆出地点和时间", () => {
    expect(splitHeading("INT. 办公室 - 日")).toEqual({ location: "办公室", timeOfDay: "日" });
  });
  it("中文直译格式", () => {
    expect(splitHeading("内景. 会议室 - 黄昏")).toEqual({ location: "会议室", timeOfDay: "黄昏" });
  });
  it("中文方括号格式", () => {
    expect(splitHeading("【场景：老茶馆】")).toEqual({ location: "老茶馆", timeOfDay: "" });
  });
  it("只有前缀没有时间", () => {
    expect(splitHeading("INT. 办公室")).toEqual({ location: "办公室", timeOfDay: "" });
  });
  /** 拆不出来不算错 —— 地点就是整个标题，不要丢内容。 */
  it("拆不出来时整个标题当地点", () => {
    expect(splitHeading("老茶馆的雨天")).toEqual({ location: "老茶馆的雨天", timeOfDay: "" });
  });
});

describe("派生场景", () => {
  const { scenes } = derive(doc);

  it("场景数和肉眼数的一致", () => {
    expect(scenes.length).toBe(2);
    expect(scenes.map((s) => s.location)).toEqual(["老茶馆", "门外街道"]);
  });

  it("每一场知道自己有谁", () => {
    expect(scenes[0].characters).toEqual(["老茶师", "小徒弟"]);
    expect(scenes[1].characters).toEqual(["小徒弟"]);
  });

  it("每一场知道有几句台词", () => {
    expect(scenes[0].dialogueCount).toBe(3);
    expect(scenes[1].dialogueCount).toBe(1);
  });

  it("记下在文档里的范围 —— 点大纲要能跳过去", () => {
    expect(scenes[0].from).toBe(0);
    // **第一场的 to 必须停在第二场标题的前一个节点。**
    // 只断言最后一场的 to 的话，收尾那行会把它兜住，
    // 中途丢掉范围更新也测不出来（变异测试抓到的）。
    expect(scenes[0].to).toBe(scenes[1].from - 1);
    expect(scenes[1].to).toBe(doc.content.length - 1);
    // 每一场都得覆盖到自己那几个节点
    for (const sc of scenes) expect(sc.to).toBeGreaterThanOrEqual(sc.from);
  });

  /** 很多人开头先写一段引子再写第一个场景标题 —— 丢掉的话那段就没了。 */
  it("场景标题之前的内容也算一场", () => {
    const { scenes } = derive(textToDoc("一段引子。\n\n【场景：老茶馆】\n他推门进去。"));
    expect(scenes).toHaveLength(2);
    expect(scenes[0].heading).toBe("");
    expect(scenes[1].location).toBe("老茶馆");
  });

  it("空文档不炸", () => {
    expect(derive(null)).toEqual({ scenes: [], characters: [] });
    expect(derive({ type: "doc", content: [] })).toEqual({ scenes: [], characters: [] });
  });
});

describe("派生角色", () => {
  const { characters } = derive(doc);

  /** **判据：数量和肉眼数的一致。** 不问模型。 */
  it("角色数和肉眼数的一致，没有多余的", () => {
    expect(characters.map((c) => c.name).sort()).toEqual(["小徒弟", "老茶师"]);
  });

  it("台词条数算得对", () => {
    expect(characters.find((c) => c.name === "老茶师")!.lines).toBe(2);
    expect(characters.find((c) => c.name === "小徒弟")!.lines).toBe(2);
  });

  it("首次出场记的是场号", () => {
    expect(characters.find((c) => c.name === "小徒弟")!.firstScene).toBe(1);
  });

  it("台词多的排前面 —— 主角先出现", () => {
    // **台词少的那个必须先出现在文档里**，否则 Map 的插入顺序恰好
    // 就是想要的结果，把排序整个删掉测试照样绿（变异测试抓到的）。
    const many = derive(
      textToDoc("乙：四。\n\n甲：一。\n\n甲：二。\n\n甲：三。"),
    ).characters;
    expect(many.map((c) => c.name)).toEqual(["甲", "乙"]);
  });

  /** 动作和旁白里出现的人名**不算角色** —— 那是模型最容易多抽的地方。 */
  it("只有说过话的才算角色", () => {
    const { characters } = derive(textToDoc("张三看着李四走远。\n\n张三：走了。"));
    expect(characters.map((c) => c.name)).toEqual(["张三"]);
  });
});

describe("派生分镜草案", () => {
  const shots = deriveShots(doc);

  it("一句台词一镜，连续动作合一镜", () => {
    expect(shots.map((s) => s.dialogue)).toEqual([
      "",              // 老茶师擦壶
      "这茶，要等。",
      "师父，客人来了。",
      "让他等。",
      "",              // 雨下得更大了
      "那我去回话。",
    ]);
  });

  it("台词镜头知道是谁说的", () => {
    expect(shots[1].speaker).toBe("老茶师");
    expect(shots[2].speaker).toBe("小徒弟");
  });

  /**
   * **台词不写进画面描述。**
   * 它单独进 `dialogue`，由出片那一步决定怎么用 ——
   * 混进 prompt 的话，模型会把台词当画面内容画出来。
   */
  it("台词不混进画面描述", () => {
    for (const s of shots) {
      if (s.dialogue) expect(s.prompt).not.toContain(s.dialogue);
    }
  });

  it("每一镜知道属于第几场、在哪儿", () => {
    expect(shots[0].scene).toBe(1);
    expect(shots[0].sceneLocation).toBe("老茶馆");
    expect(shots[5].scene).toBe(2);
    expect(shots[5].sceneLocation).toBe("门外街道");
  });

  it("时长按台词长短算，并夹在合理区间", () => {
    const long = deriveShots(textToDoc("甲：" + "字".repeat(200)));
    expect(long[0].duration).toBeLessThanOrEqual(12);
    const short = deriveShots(textToDoc("甲：好"));
    expect(short[0].duration).toBeGreaterThanOrEqual(2);
  });

  it("台词给特写，动作给中景", () => {
    expect(shots[1].scale).toContain("特写");
    expect(shots[0].scale).toContain("中景");
  });

  it("空文档回空数组", () => {
    expect(deriveShots(null)).toEqual([]);
  });
});
