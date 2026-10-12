import { describe, expect, it } from "vitest";
import {
  appendTag,
  buildPrompt,
  CHARACTER_TEMPLATES,
  finalNegative,
  finalPositive,
  flattenDescription,
  QUICK_TAGS,
  templateApplied,
  templateById,
  templateNegative,
  withoutSingleImageComposition,
  styleForKind,
  screenHint,
  negationWarning,
  withName,
  PROP_DEFAULT_TEMPLATE,
  PROP_TEMPLATES,
} from "./assetPrompt";

describe("构图模板", () => {
  /**
   * **`simple` 里必须有那句大头特写。**
   *
   * 它是身份参考图能用的唯一原因：实测三视图整排里脸只有 42~46px，
   * 可用下限是 150~200px。左半边给大头之后头部区域有 ~512px。
   * 这句话被改掉的话，出的图看着还是设定卡，但脸小到锁不住身份 ——
   * 不报错，只表现为「这个人不像」。
   */
  it("简洁三视图里有大头特写，而且占半边", () => {
    const t = templateById("simple");
    expect(t.composition).toContain("Left half: large head close-up portrait");
    expect(t.composition).toContain("Right half: three equally-sized full-body");
  });

  it("详细参考表里也有大脸特写", () => {
    expect(templateById("detailed").composition).toContain("large face close-up portrait");
  });

  /**
   * 通用那两条每个模板都要有；「挡边框」只对无边框的那两个成立 ——
   * **「角色设定图」的边框是故意要的**（它的构图就是带边框的分区），
   * 一刀切地挡掉等于让模板自相矛盾。
   */
  it("每个文生图模板都挡水印和「拆成多张图」", () => {
    const t2i = CHARACTER_TEMPLATES.filter((t) => !t.pipeline);
    for (const t of t2i) {
      expect(t.negative, `${t.id} 没有负向词`).toContain("watermark");
      expect(t.negative, `${t.id} 没挡「拆成多张」`).toContain("multiple separate images");
    }
    // **跳过的理由要钉住。** 否则以后有个文生图模板忘了写负向词，
    // 只要它顺手带上 `pipeline` 就能从这条护栏底下溜过去。
    for (const t of CHARACTER_TEMPLATES.filter((t) => t.pipeline)) {
      expect(t.composition, `${t.id} 声明了 pipeline 却还带构图段`).toBe("");
      expect(t.negative, `${t.id} 声明了 pipeline 却还带负向词`).toBe("");
    }
    expect(t2i.length, "文生图模板一个都不剩了？").toBeGreaterThanOrEqual(3);
  });

  it("无边框的那两个要挡边框，带边框的那个不挡", () => {
    expect(templateById("simple").negative).toContain("panel borders");
    expect(templateById("detailed").negative).toContain("panel borders");
    expect(templateById("panel5").negative).not.toContain("panel borders");
  });

  /**
   * **信息面板必须挡掉。** 实测：短中文标题（正面 / 表情特写）写得对，
   * 但面板里姓名年龄那些**值**必然是乱码（`Nirle` / `Killo`）——
   * 留着只会出一片乱码然后让人以为坏了。
   */
  it("角色设定图挡掉信息面板和文字段落", () => {
    const n = templateById("panel5").negative;
    expect(n).toContain("info panel");
    expect(n).toContain("text paragraphs");
    expect(templateById("panel5").composition).not.toContain("info panel");
  });

  /**
   * 实测只有 Z-Image 的中文标注是 6/6；Krea 2 是 4/6，FLUX.2 Klein 是 0/12。
   * **版面三个都撑得住** —— 卡的是标注，不是版面。
   * 不标出来的话，用户拿撑不住的引擎选了它，出一张带错字的图还不知道为什么。
   */
  it("角色设定图标明了只有 Z-Image 撑得住", () => {
    expect(templateById("panel5").engines).toEqual(["zimage"]);
    expect(templateById("simple").engines).toBeUndefined();
  });

  /** 方图会把「三个全身」出成 4 个 —— 实测过。 */
  it("角色设定图偏好横幅", () => {
    expect(templateById("panel5").prefersAspect).toBe("landscape");
  });

  /**
   * **禁令必须写进正向。** Z-Image 的负向结构性无效（cfg=1，
   * 负向通路被 `ConditioningZeroOut` 清零），写在负向框里等于没写。
   * `extra character body` 这一句治的是「左栏冒出第二个几乎一样的大头像」——
   * 实测有效。
   */
  it("角色设定图把禁令写进了正向", () => {
    const c = templateById("panel5").composition;
    expect(c).toContain("Do not create");
    expect(c, "少了那句治重复视图的").toContain("extra character body");
    expect(c).toContain("information table");
  });

  /**
   * 文生那几个各不相同 —— 写串了的话选哪个都出一样的图。
   *
   * **图生那组不参与**：它们没有构图段（提示词在 Rust 侧按版式拼），
   * 空串当然会重。跳过的理由在下面那条一起钉住，
   * 免得以后有个文生模板忘了写构图段也从这条底下溜过去。
   */
  it("文生模板的构图段互不相同", () => {
    const t2i = CHARACTER_TEMPLATES.filter((t) => !t.pipeline);
    const all = t2i.map((t) => t.composition);
    expect(new Set(all).size, "有模板的构图段写重了").toBe(all.length);
    expect(t2i.length).toBeGreaterThanOrEqual(3);
    for (const t of t2i) {
      expect(t.composition.length, `${t.id} 没有构图段`).toBeGreaterThan(50);
    }
    // 图生那组必须是空的 —— 有构图段说明有人把两条路的提示词混了
    for (const t of CHARACTER_TEMPLATES.filter((t) => t.pipeline)) {
      expect(t.composition, `${t.id} 是图生模板却带了构图段`).toBe("");
    }
  });

  /** 只有它是五分区带中文标注的，另外两个不该混进这些词。 */
  it("中文标注只出现在角色设定图里", () => {
    expect(templateById("panel5").composition).toContain("正面");
    expect(templateById("simple").composition).not.toContain("正面");
    expect(templateById("detailed").composition).not.toContain("正面");
  });

  it("认不出的模板 id 回落到简洁三视图，不是崩掉", () => {
    expect(templateById("乱写的" as never).id).toBe("simple");
  });
});

describe("初始提示词", () => {
  /**
   * **描述在前、构图在后。** 反过来的话 SDXL 那种分块取平均的编码器
   * 会让长构图段把「这个人长什么样」挤掉。
   */
  it("描述排在构图前面", () => {
    const p = buildPrompt("character", { name: "小花", description: "三花猫" });
    expect(p.indexOf("小花")).toBeLessThan(p.indexOf("Composition:"));
    expect(p).toContain("小花，三花猫");
  });

  it("没有描述时只有名字，不留一个孤零零的逗号", () => {
    const p = buildPrompt("character", { name: "小花" });
    expect(p.startsWith("小花\n\n")).toBe(true);
    expect(p).not.toContain("小花，\n");
  });

  it("场景和道具各有自己的构图，不套角色那套", () => {
    expect(buildPrompt("scene", { name: "老茶馆" })).toContain("wide establishing shot");
    expect(buildPrompt("prop", { name: "铜壶" })).toContain("product photography");
    expect(buildPrompt("scene", { name: "老茶馆" })).not.toContain("full-body");
  });

  it("场景道具的负向词不带三视图那套", () => {
    expect(templateNegative("scene")).not.toContain("multiple separate images");
    expect(templateNegative("character")).toContain("multiple separate images");
  });
});

describe("快捷标签", () => {
  it("三类各有一套，互不相同", () => {
    expect(QUICK_TAGS.character).not.toEqual(QUICK_TAGS.scene);
    expect(QUICK_TAGS.scene).not.toEqual(QUICK_TAGS.prop);
    for (const k of ["character", "scene", "prop"] as const) {
      expect(QUICK_TAGS[k].length, `${k} 的标签太少`).toBeGreaterThanOrEqual(6);
    }
  });

  /**
   * 不补分隔符的话，点两下会变成 `…illuminationfull bodyclose-up` ——
   * 模型当成一个生造的词，而且不报错。
   */
  it("追加标签会补上逗号和空格", () => {
    expect(appendTag("a portrait", "full body")).toBe("a portrait, full body");
    expect(appendTag("a portrait, full body", "close-up")).toBe(
      "a portrait, full body, close-up",
    );
  });

  it("末尾已经有逗号时不补第二个", () => {
    expect(appendTag("a portrait,", "full body")).toBe("a portrait, full body");
    expect(appendTag("一段中文，", "close-up")).toBe("一段中文， close-up");
  });

  it("空提示词时不以逗号开头", () => {
    expect(appendTag("", "full body")).toBe("full body");
    expect(appendTag("   ", "full body")).toBe("full body");
  });
});

describe("最终提示词", () => {
  /** 和后端 `style::apply_style` 同一条规矩 —— 两处不一致的话，
   *  预览里看到的和真发出去的不是一个东西。 */
  it("没有构图段时就追加在后面", () => {
    expect(finalPositive("小花的设定卡", "水墨风", true)).toBe("小花的设定卡, 水墨风");
  });

  /**
   * **有构图段时风格要插在它前面。** 2026-09-05 同 seed 对照实测：
   * 风格排在 600 字符的构图段之后 → 出写实照片；插在它之前 → 出手绘插画。
   * 构图段里全是摄影语言（`detailed skin texture`、`studio lighting`），
   * 风格词跟在后面会被彻底淹没 —— 而且不报错，只是风格没生效。
   */
  it("有构图段时风格插在构图之前", () => {
    const p = buildPrompt("character", { name: "小花", description: "三花猫" });
    const out = finalPositive(p, "水彩手绘", true);
    expect(out.indexOf("水彩手绘")).toBeLessThan(out.indexOf("Composition:"));
    expect(out.indexOf("小花")).toBeLessThan(out.indexOf("水彩手绘"));
  });

  it("插进去之后构图段一个字都没少", () => {
    const p = buildPrompt("character", { name: "小花" });
    const out = finalPositive(p, "水彩手绘", true);
    const composition = p.slice(p.indexOf("Composition:"));
    expect(out).toContain(composition);
  });

  it("关掉开关就完全不带风格", () => {
    expect(finalPositive("小花的设定卡", "水墨风", false)).toBe("小花的设定卡");
  });

  it("风格词已经在里面就不重复叠", () => {
    expect(finalPositive("小花, 水墨风", "水墨风", true)).toBe("小花, 水墨风");
  });

  it("没有风格时不留尾巴逗号", () => {
    expect(finalPositive("小花", "", true)).toBe("小花");
    expect(finalPositive("小花", "   ", true)).toBe("小花");
  });

  it("负向：风格 + 构图拼起来", () => {
    expect(finalNegative("低质量", "watermark", true)).toBe("低质量, watermark");
  });

  /** 空的那段不能留下孤零零的逗号 —— 有些引擎会把 `, ` 当成一个 token。 */
  it("负向：任一段为空都不留孤逗号", () => {
    expect(finalNegative("", "watermark", true)).toBe("watermark");
    expect(finalNegative("低质量", "", true)).toBe("低质量");
    expect(finalNegative("低质量", "watermark", false)).toBe("watermark");
    expect(finalNegative("", "", true)).toBe("");
  });
});

/**
 * 风格必须排在构图段**之前** —— 这条是同 seed 对照实测定下来的
 * （见 `assetPrompt.ts` 里那张表和 memory `style-must-precede-composition`）。
 *
 * # 为什么要遍历所有模板 / 所有类型
 *
 * 2026-09-06 出过一次事：改了 `design_sheet` 的构图段头一句，
 * `Composition:` 不再顶在段首，插入锚点匹配不上，风格**静默**落回末尾 ——
 * 而当时那三条只针对 `simple` 写的断言全是绿的。
 *
 * 所以这里不挑模板：往 `CHARACTER_TEMPLATES` 里加一条新构图，
 * 就自动被这条盯上。少一条都不行。
 */
describe("风格插在构图之前（每个模板、每个类型）", () => {
  const STYLE = "温暖手绘动画";

  // **构图段按它自己的头几个字定位，不用写死的 `Composition:`。**
  //
  // 原来就是写死的，而那是**英文专属**的标记 —— 加第一个中文构图模板
  // （`quadzh`，开头是「高质量专业角色设定图」）的当场就塌了，
  // 报的还是 `expected -1 to be greater than 0`，看不出真正的原因。
  // 取模板自己那段的前 12 个字，中英文都成立。
  const head = (comp: string) => comp.slice(0, 12);
  const cases: { name: string; prompt: string; mark: string }[] = [
    // 只有文生图那几个走「主体 → 风格 → 构图」这套拼装。
    // `pipeline` 模板的提示词由 Rust 侧按结构化模板拼，不经过这里。
    ...CHARACTER_TEMPLATES.filter((t) => !t.pipeline).map((t) => ({
      name: `character/${t.id}`,
      prompt: buildPrompt("character", { name: "小花", description: "三花猫" }, t.id),
      mark: head(t.composition),
    })),
    {
      name: "scene",
      prompt: buildPrompt("scene", { name: "老巷", description: "黄昏" }),
      mark: "Composition:",
    },
    { name: "prop", prompt: buildPrompt("prop", { name: "铜铃" }), mark: "Composition:" },
  ];

  it.each(cases)("$name：风格在构图段之前", ({ prompt, mark }) => {
    const out = finalPositive(prompt, STYLE, true);
    const iStyle = out.indexOf(STYLE);
    const iComp = out.indexOf(mark);
    expect(iStyle).toBeGreaterThanOrEqual(0);
    expect(iComp, `找不到构图段的开头「${mark}」`).toBeGreaterThan(0);
    expect(iStyle).toBeLessThan(iComp);
  });

  /** 覆盖率断言：模板列表空了、或 buildPrompt 不再产出构图段，这条会塌。 */
  it("上面确实测了每一个文生图模板", () => {
    const t2i = CHARACTER_TEMPLATES.filter((t) => !t.pipeline);
    expect(t2i.length).toBeGreaterThanOrEqual(3);
    expect(cases.length).toBe(t2i.length + 2);
    // 至少有一个 pipeline 模板存在 —— 否则上面那个 filter 是死代码，
    // 而死掉的 filter 会在有人加回 pipeline 模板时静默漏测
    expect(CHARACTER_TEMPLATES.some((t) => t.pipeline)).toBe(true);
  });

  /** 主体描述必须还在风格前面 —— 风格不能把「这个人长什么样」挤到后面去。 */
  it("顺序是 主体 → 风格 → 构图", () => {
    const out = finalPositive(
      buildPrompt("character", { name: "小花", description: "三花猫" }, "panel5"),
      STYLE,
      true,
    );
    expect(out.indexOf("小花")).toBeLessThan(out.indexOf(STYLE));
    expect(out.indexOf(STYLE)).toBeLessThan(out.indexOf("Composition:"));
  });

  /** 用户把构图段删光、只剩一段时，回到末尾追加而不是丢掉风格。 */
  it("提示词只有一段时风格追加在末尾", () => {
    expect(finalPositive("小花，三花猫", STYLE, true)).toBe(`小花，三花猫, ${STYLE}`);
  });
});

describe("卡片和实际提示词是否一致", () => {
  const cat = { name: "小花", description: "三花猫" };

  it("刚生成的提示词：一致", () => {
    for (const t of CHARACTER_TEMPLATES) {
      expect(templateApplied(buildPrompt("character", cat, t.id), "character", t.id)).toBe(true);
    }
  });

  it("改过提示词之后换卡：不一致（这正是要提示的那种情况）", () => {
    const written = buildPrompt("character", cat, "simple") + ", 加了几个词";
    expect(templateApplied(written, "character", "simple")).toBe(true);
    expect(templateApplied(written, "character", "panel5")).toBe(false);
  });

  it("整段删干净了也算不一致", () => {
    expect(templateApplied("小花，三花猫", "character", "simple")).toBe(false);
  });

  it("场景 / 道具也认", () => {
    expect(templateApplied(buildPrompt("scene", { name: "老巷" }), "scene")).toBe(true);
    expect(templateApplied(buildPrompt("prop", { name: "铜铃" }), "prop")).toBe(true);
    expect(templateApplied(buildPrompt("scene", { name: "老巷" }), "prop")).toBe(false);
  });
});

describe("引擎不匹配时说的话", () => {
  /**
   * **有白名单就必须自己写理由。**
   *
   * 原来是一句通用的「中文标注会出错」套所有模板 —— 对五分区成立
   * （只有 Z-Image 是 6/6），对七分区完全不对：它卡的是**版式 LoRA
   * 只有 Krea 2 版**。一句话套所有模板的后果是用户照着一个错的理由去换引擎。
   */
  it("每个带白名单的模板都写了自己的理由", () => {
    for (const t of CHARACTER_TEMPLATES) {
      if (!t.engines) continue;
      expect(t.mismatch, `${t.id} 有 engines 白名单却没写 mismatch`).toBeTruthy();
      expect(t.mismatch!.length, `${t.id} 的理由太短，说不清`).toBeGreaterThan(8);
    }
    // 至少有两个模板带白名单 —— 否则上面那个循环是死代码
    expect(CHARACTER_TEMPLATES.filter((t) => t.engines).length).toBeGreaterThanOrEqual(2);
  });

  /** 两条理由必须不同 —— 相同就说明又退回「一句话套所有」了。 */
  it("五分区和七分区的理由不一样", () => {
    const p5 = templateById("panel5").mismatch;
    const p7 = templateById("panel7").mismatch;
    expect(p5).not.toBe(p7);
    // 各自说的是各自的病因
    expect(p5).toContain("中文");
    expect(p7).toContain("LoRA");
  });
});

/**
 * **标签是给人看的，不该发给出图模型。**
 *
 * 槽位化之后资产描述是「描述：… / 服装：… / 特征：…」三行。
 * 原样发出去的话：标签是画面里没有的词、硬换行把提示词切成三段、
 * 而 Z-Image 会把汉字画进画面 —— 标签有被渲染成标注的风险。
 */
describe("flattenDescription", () => {
  const 三行 =
    "描述：盛唐女官，女性，28岁，肤色苍白。\n服装：上衣男装官服上衣，黑色官靴，官帽，无配饰。\n特征：黑眼圈明显。";

  it("去掉标签和换行，拼成一句", () => {
    expect(flattenDescription(三行)).toBe(
      "盛唐女官，女性，28岁，肤色苍白。上衣男装官服上衣，黑色官靴，官帽，无配饰。黑眼圈明显。",
    );
  });

  it("**值一个字都不能少** —— 这是出图唯一的身份依据", () => {
    const out = flattenDescription(三行);
    for (const w of ["盛唐女官", "28岁", "官帽", "无配饰", "黑眼圈明显"]) {
      expect(out, `丢了「${w}」`).toContain(w);
    }
  });

  it("没有标签的旧描述原样通过", () => {
    expect(flattenDescription("白须，六十岁")).toBe("白须，六十岁");
  });

  it("进了提示词就不该再有标签和换行", () => {
    const p = buildPrompt("character", { name: "林婉清", description: 三行 }, "quadzh");
    const head = p.split("\n\n")[0];
    expect(head).not.toContain("描述：");
    expect(head).not.toContain("服装：");
    expect(head).not.toContain("\n");
  });
});

/**
 * **宫格不要「出一张图」那一段。**
 *
 * 场景的构图段是给单张场景图写的（`single unified image` /
 * `wide establishing shot`），而宫格那条路会把它当成「场景描述」嵌进
 * 宫格提示词 —— 两套指令打架。2026-09-12 实拍：四格出成同一个正面机位。
 */
describe("withoutSingleImageComposition", () => {
  it("把带 single unified image 的那一段整段去掉", () => {
    const p = buildPrompt("scene", { name: "金銮殿", description: "高约15米" });
    expect(p).toContain("single unified image");
    const out = withoutSingleImageComposition(p);
    expect(out).not.toContain("single unified image");
    expect(out).not.toContain("wide establishing shot");
  });

  it("**场景描述一个字不丢** —— 那是这张图唯一的内容来源", () => {
    const p = buildPrompt("scene", { name: "金銮殿", description: "高约15米、宽约30米，中央设龙椅" });
    const out = withoutSingleImageComposition(p);
    expect(out).toContain("金銮殿");
    expect(out).toContain("中央设龙椅");
  });

  it("用户自己加的段落留着 —— 只去带那句话的那一段", () => {
    const p = `金銮殿，高约15米\n\n${SCENE_SINGLE}\n\n我自己加的一句`;
    const out = withoutSingleImageComposition(p);
    expect(out).toContain("我自己加的一句");
    expect(out).not.toContain("single unified image");
  });

  /** 判据是关键词不是整段精确匹配 —— 用户改过提示词之后精确匹配就失效了。 */
  it("改过的构图段照样去得掉", () => {
    const p = "金銮殿\n\nComposition: my own words, single unified image, whatever";
    expect(withoutSingleImageComposition(p)).toBe("金銮殿");
  });

  it("本来就没有那一段时原样返回", () => {
    expect(withoutSingleImageComposition("金銮殿，高约15米")).toBe("金銮殿，高约15米");
  });
});

const SCENE_SINGLE =
  "Composition: wide establishing shot of the environment on neutral gray background, single unified image, no figures in foreground.";

describe("道具和场景不带人（2026-09-14 手机屏幕里全是人脸）", () => {
  const CINEMATIC =
    "cinematic photorealism, live-action film still, natural human proportions, realistic skin texture, motivated lighting, 35mm lens, shallow depth of field, subtle film grain, volumetric atmosphere, professional color grading, realistic production design, grounded emotional performance, high-end movie frame";

  it("场景、道具只留风格里的质感和镜头感，去掉描述人的词；角色照旧全套", () => {
    const prop = styleForKind("prop", CINEMATIC);
    expect(prop).toBe("cinematic photorealism, motivated lighting, 35mm lens, shallow depth of field, subtle film grain, volumetric atmosphere, professional color grading, realistic production design, high-end movie frame");
    expect(styleForKind("scene", CINEMATIC)).toBe(prop);
    expect(styleForKind("character", CINEMATIC)).toBe(CINEMATIC);
    expect(styleForKind("prop", "温暖手绘动画，柔和色调，人物表情夸张")).toBe("温暖手绘动画, 柔和色调");
    expect(finalPositive("手机，现代智能手机\n\nComposition: x", CINEMATIC, true, "prop")).not.toMatch(/human|skin|performance|live-action/);
  });

  it("屏幕类道具正面写清屏幕上是什么", () => {
    expect(screenHint("手机", "现代智能手机，屏幕亮起显示提示")).toContain("纯界面");
    expect(screenHint("电脑", "现代台式电脑")).toBe("屏幕熄灭，黑色玻璃面反光");
    expect(screenHint("龙椅", "雕龙装饰")).toBe("");
    const p = buildPrompt("prop", { name: "手机", description: "现代智能手机，屏幕亮起显示提示" });
    expect(p.startsWith("手机，现代智能手机，屏幕亮起显示提示。屏幕画面是纯界面")).toBe(true);
    expect(p).toContain("The object alone is the only thing in the frame.");
    // 角色、场景不加屏幕描述
    expect(buildPrompt("scene", { name: "办公室", description: "电脑屏幕亮着" })).not.toContain("纯界面");
  });

  it("负向不生效的引擎上，正向里的否定写法要提醒", () => {
    expect(negationWarning("屏幕里显示不要显示任何人", false)).toContain("不要");
    expect(negationWarning("no people, a phone", false)).toContain("no people");
    expect(negationWarning("屏幕里显示不要显示任何人", true)).toBe("");
    expect(negationWarning("屏幕上是几条文字消息", false)).toBe("");
  });
});

/**
 * **构图面板 2026-09-26 精简成两张卡，而且可以一张都不选。**
 *
 * 用户原话：「去掉头像，详细参考表，五分区和四视图(中文)，增加刚才的设定卡(QwenOnly)。
 * 构图模版可以默认不选，不是必须选一个，不选择默认居中头像。」
 */
describe("构图面板（2026-09-26）", () => {
  // 工作台的筛选条件就是这一句（`!t.hidden`），不分文生 / 图生页
  const onPicker = () => CHARACTER_TEMPLATES.filter((t) => !t.hidden).map((t) => t.id);

  it("面板上一共三张：简洁三视图、Qwen 设定卡、四视图（图生）", () => {
    // 用户：「构图模版一共就留 3 个，不用分页了」—— 七分区也清掉
    expect(onPicker()).toEqual(["simple", "bible", "quad"]);
    // 只有四视图是图生（要封面图），卡上靠这个标「图生」
    expect(CHARACTER_TEMPLATES.filter((t) => !t.hidden && t.pipeline).map((t) => t.id)).toEqual(["quad"]);
  });

  it("拿掉的版式只是不显示，批量那条路还查得到", () => {
    for (const id of ["avatar", "detailed", "panel5", "quadzh", "panel7"] as const) {
      expect(templateById(id).id).toBe(id);
      expect(templateById(id).hidden).toBe(true);
    }
  });

  it("不选构图 = 居中头像：有身体、单张、竖幅", () => {
    const t = templateById("portrait");
    expect(t.hidden).toBe(true);
    expect(t.composition).toContain("single centered head-and-shoulders portrait");
    expect(t.composition).not.toMatch(/full-body|turnaround/);
    // 和 avatar 不是一回事：avatar 明令不画躯干，这个是有身体角色的胸像
    expect(t.composition).not.toContain("NO body");
    expect(t.prefersAspect).toBe("portrait");
  });

  it("设定卡只给 Qwen 2.1，横幅", () => {
    const t = templateById("bible");
    expect(t.engines).toEqual(["qwen21"]);
    expect(t.prefersAspect).toBe("landscape");
    // 这张卡要的就是图上的文字 —— 负向里不能有 text / labels
    expect(t.negative).not.toMatch(/\btext\b|labels/);
  });

  it("设定卡的标题是资产名，带引号的确切文字", () => {
    const p = buildPrompt("character", { name: "陆川", description: "26 岁，油污工装" }, "bible");
    expect(p).toContain("the exact title “陆川”");
    expect(p).not.toContain("{name}");
    // 换进名字之后仍然认得出用的是这张卡，不会误报「提示词改过」
    expect(templateApplied(p, "character", "bible")).toBe(true);
    // 真改掉构图段就认不出
    expect(templateApplied(p.replace("DETAILS", "细节"), "character", "bible")).toBe(false);
  });

  it("withName 只换占位符", () => {
    expect(withName("a “{name}” b {name}", " 星澜 ")).toBe("a “星澜” b 星澜");
    expect(withName("no placeholder", "x")).toBe("no placeholder");
  });
});

/**
 * 道具信息卡（2026-09-30）：**照抄 Codex 出《回收师》道具卡的那段提示词**，只把星澜铭牌专属的内容换成占位。
 * 用户：「直接用那个提示词做模版就可以了」
 */
describe("道具信息卡", () => {
  const laser = { name: "激光刀", description: "手持式工业切割器，手柄是磨旧的黑色橡胶、缠着电工胶布；刀头是一段细细的红色光刃。" };

  it("上排正 / 侧 / 背三视图、下排两个特写框，名字写进请求，外观由描述给", () => {
    const p = buildPrompt("prop", laser, "propcard");
    expect(p.startsWith("激光刀，手持式工业切割器")).toBe(true);
    expect(p).toContain("生成“激光刀”的高精度三视图道具卡");
    expect(p).toContain("上排从左到右是 FRONT 正面、SIDE 严格90度侧面、BACK 背面，比例一致、无遮挡");
    expect(p).toContain("下排两个大特写框");
    expect(p).toContain("冷白略带浅蓝的无缝工业设计图背景，细灰蓝辅助线和分区线，无环境场景");
    // 星澜铭牌专属的不能漏进通用模板
    expect(p).not.toMatch(/星澜|BY-0000|GEM \/ PROTOCOL|REAR INTERFACE|EDEN|回收师/);
    expect(templateApplied(p, "prop", "propcard")).toBe(true);
    expect(templateApplied(buildPrompt("prop", laser, "simple"), "prop", "propcard")).toBe(false);
  });

  it("不选卡还是原来的产品摄影构图；选了卡负向不能擦掉 FRONT / SIDE / BACK 标注", () => {
    expect(buildPrompt("prop", laser, "simple")).toContain("product photography style");
    expect(buildPrompt("prop", laser, "propcard")).not.toContain("product photography style");
    expect(templateNegative("prop", "simple")).toContain("labels");
    expect(templateNegative("prop", "propcard")).not.toMatch(/text|label/);
  });

  /** Qwen 在我们这儿负向不生效，Codex 原文的「Avoid: 不要人物……」改成了正面写法 —— 不然每次都弹否定写法警告 */
  it("没有会触发否定写法警告的句子；默认 Qwen 2.1、横幅", () => {
    expect(negationWarning(buildPrompt("prop", laser, "propcard"), false)).toBe("");
    const t = PROP_TEMPLATES.find((x) => x.id === "propcard")!;
    expect(t.engines).toEqual(["qwen21"]);
    expect(t.prefersAspect).toBe("landscape");
    expect(templateById(PROP_DEFAULT_TEMPLATE).id).toBe("propcard");
  });

  it("风格插在描述之后、版式段之前（和角色卡同一条规矩）", () => {
    const out = finalPositive(buildPrompt("prop", laser, "propcard"), "国漫 3D 半写实, cinematic lighting", true, "prop");
    expect(out.indexOf("国漫 3D 半写实")).toBeLessThan(out.indexOf("Use case: stylized-concept"));
    expect(out.indexOf("红色光刃")).toBeLessThan(out.indexOf("国漫 3D 半写实"));
  });
});
