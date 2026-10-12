import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import { CHARACTER_TEMPLATES, templateById } from "./assetPrompt";

/**
 * 工作台的几条性质。渲染测不了（没有 jsdom），从源码上钉 ——
 * 这几条错了都不报错，只是出的图不对。
 */
const SRC = readFileSync(
  fileURLToPath(new URL("./AssetWorkbench.tsx", import.meta.url)),
  "utf8",
);

describe("资产生成工作台", () => {
  /**
   * **预览里显示的必须就是发出去的那一份。**
   * 各算一遍的话，改了拼装规则只改一处，预览和实际会悄悄分家 ——
   * 而用户是照着预览判断的。
   */
  it("预览和提交用的是同一个变量", () => {
    expect(SRC).toContain("const outPositive = finalPositive(");
    expect(SRC).toContain("const outNegative = finalNegative(");
    expect(SRC).toContain("prompt: outPositive,");
    expect(SRC).toContain("negative_prompt: outNegative,");
  });

  /**
   * **场景有三种产物，发出去的提示词不一样 —— 预览要跟着切。**
   *
   * 宫格和全景都会去掉「出一张图」那一段（`single unified image` 和
   * 宫格版式打架，实拍四格出成同一个机位）。预览还显示原样的话，
   * 用户照着预览判断，而实际发出去的是另一段。
   */
  it("场景三种产物的预览跟着切，且和提交用同一个推导", () => {
    expect(SRC).toContain("const previewPositive =");
    expect(SRC).toContain("{previewPositive}");
    // 预览和提交用的是**同一个函数**，不是各写一遍
    const previewUses = SRC.includes(
      'sceneOut !== "single"\n      ? withoutSingleImageComposition(outPositive)',
    );
    expect(previewUses, "预览没走 withoutSingleImageComposition").toBe(true);
    expect(SRC).toContain("prompt: withoutSingleImageComposition(outPositive),");
  });

  /**
   * 引擎和画幅**从 `effectiveGen` 来**（项目设置打底 + 资产覆盖），
   * 不在这个文件里另算一遍。另算的话两处规则迟早漂 ——
   * 而表现只是「设置改了这里没跟着变」。
   *
   * 「哪一类用哪个画幅」的规则本身由 `effectiveGen.test.ts` 钉住。
   */
  it("引擎和画幅走 effectiveGen，不在组件里另算", () => {
    expect(SRC).toContain("effectiveGen(kind, asset.gen, settings)");
    // 道具默认选中信息卡、引擎用它要的那个（见下面「道具面板的构图卡」）；其余类型就是 `eff.engine`
    expect(SRC).toContain(": eff.engine,\n  );");
    expect(SRC).toContain("useState(eff.aspect)");
  });

  it("组件里不重复实现按类型取画幅那套 if-else", () => {
    expect(SRC, "又在组件里直接读 settings 了").not.toContain("settings?.scene_aspect");
  });

  /**
   * 尺寸必须从后端下发的 aspects 里查，**不能自己算**。
   * 自己算就会产出引擎不支持的尺寸 —— 正是那个
   * `WT resolution must be 384p or 512p` 的形状。
   */
  it("宽高来自后端下发的画幅表", () => {
    expect(SRC).toContain("capabilities.aspects.find");
    expect(SRC).toContain("width: aspect.width");
    expect(SRC).toContain("height: aspect.height");
  });

  /** 改过提示词之后换模板不该抹掉改动 —— 那是几分钟的手写。 */
  it("改过之后换模板不覆盖", () => {
    expect(SRC).toContain("if (!dirty) setPrompt(buildPrompt(");
  });

  /** 出图是逐张的：一张失败不该让整批算作失败。 */
  it("一张失败不影响其余的入池", () => {
    expect(SRC).toContain("continue;");
    expect(SRC).toContain("ok += 1;");
    expect(SRC).toMatch(/if \(ok === 0\)/);
  });
});

describe("负向提示词的诚实", () => {
  const SRC2 = readFileSync(
    fileURLToPath(new URL("./AssetWorkbench.tsx", import.meta.url)),
    "utf8",
  );

  /**
   * **引擎不吃负向词时必须说出来。**
   *
   * Z-Image 是 cfg=1，负向通路被 `ConditioningZeroOut` 清零 ——
   * 传了等于没传。把负向词大大方方显示在左栏却不说这件事，就是界面在说谎：
   * 用户以为 `photorealistic` 被挡住了，实际一点作用没有，
   * 然后去反复改风格词。（用户问「我选了风格，生成的依然是写实」，
   * 这是其中一半原因。）
   */
  it("引擎吃不吃负向词，从能力表里读，不在前端另写一份", () => {
    expect(SRC2).toContain("caps?.engines.find((e) => e.id === engine)?.negative_prompt");
  });

  it("不吃的时候有明确提示，而不是静静地不生效", () => {
    expect(SRC2).toContain("!negativeWorks");
    expect(SRC2).toContain("结构性无效");
  });

  /**
   * 用到 `engine` 的那几个 const 必须排在它声明之后。
   * 排在前面是 TDZ —— 运行时整个弹窗白屏，而 tsc 这次没拦住。
   */
  it("engine 的声明排在用它的那几行前面", () => {
    const decl = SRC2.indexOf("const [engine, setEngine] = useState(");
    const use = SRC2.indexOf("const negativeWorks");
    expect(decl).toBeGreaterThan(-1);
    expect(use, "negativeWorks 用了还没声明的 engine（TDZ，会白屏）").toBeGreaterThan(decl);
  });
});

describe("生成配置", () => {
  const SRC3 = readFileSync(
    fileURLToPath(new URL("./AssetWorkbench.tsx", import.meta.url)),
    "utf8",
  );

  it("引擎、画幅、LoRA 都能在这一页改", () => {
    expect(SRC3).toContain("setEngine(");
    expect(SRC3).toContain("setAspectId(");
    expect(SRC3).toContain("toggleLora(");
  });

  /**
   * **换引擎要清掉挂不上的 LoRA。**
   * 留着的话它会被静默忽略（`0 bypass adapters, 0 inj`）——
   * 图照出、不报错、只是那个风格一点没生效。
   */
  it("换引擎会清掉架构不匹配的 LoRA", () => {
    const i = SRC3.indexOf("setEngine(e.id)");
    const after = SRC3.slice(i, i + 400);
    expect(after, "换引擎之后没有清理 picks").toContain("setPicks(");
    expect(after).toContain("engines.includes(e.id)");
  });

  /** 只列这个引擎能用的 —— 列出来就等于邀请人挂一个不生效的。 */
  it("LoRA 列表按引擎过滤", () => {
    expect(SRC3).toContain("allLoras.filter((l) => l.engines.includes(engine))");
  });

  /** 挂的 LoRA 必须真的送进引擎，否则这一整块都是摆设。 */
  it("挂的 LoRA 真的送进生成请求", () => {
    expect(SRC3).toContain("loras: picks,");
  });

  /**
   * 存的时候走 `toOverrides` —— 它会把和项目设置一样的项剔掉。
   * 全存下来的话，以后改项目设置这个资产不跟着变，而用户不记得设过。
   */
  it("保存走 toOverrides，不是整份塞进去", () => {
    expect(SRC3).toContain("gen: toOverrides(");
  });
});

describe("LoRA 行", () => {
  const SRC4 = readFileSync(
    fileURLToPath(new URL("./AssetWorkbench.tsx", import.meta.url)),
    "utf8",
  );

  /**
   * **说明要显示在行里，不能只挂悬浮提示。**
   *
   * 一行只有名字加一大片空白，而「这个 LoRA 是干嘛的」恰恰是选它时
   * 唯一要判断的事 —— 藏在 title 里等于没有：要先猜它有说明、
   * 再把鼠标停上去。
   */
  it("说明显示在行里", () => {
    expect(SRC4).toContain("{l.note ?? \"没有说明");
    // 而且不是只在 title 里
    const inTitleOnly = SRC4.match(/title=\{l\.note[^}]*\}/g) ?? [];
    expect(inTitleOnly.length).toBeGreaterThan(0);
    // 一行截断 —— 六个 LoRA 各占两三行的话，这一栏比提示词还长
    expect(SRC4).toContain("block truncate text-[10.5px]");
  });

  it("没有说明时告诉人去哪写，而不是留一片空白", () => {
    expect(SRC4).toMatch(/没有说明[^"]*写一句/);
  });

  /**
   * 截断了就必须有悬浮全文 —— 一行放不下这些说明（「实测：出设定卡这种
   * 带长构图段的提示词时…」），看不到全文等于没写。
   */
  it("截断的同时挂了全文的悬浮提示", () => {
    expect(SRC4).toContain("title={l.note");
  });
});

describe("构图模板的引擎限制", () => {
  const SRC5 = readFileSync(
    fileURLToPath(new URL("./AssetWorkbench.tsx", import.meta.url)),
    "utf8",
  );

  /**
   * **撑不住就说出来，别静静地让人选。**
   * 直接禁掉的话人不知道为什么；让选又不说，出来一张乱码图更糟。
   */
  it("引擎撑不住的模板会标出来", () => {
    expect(SRC5).toContain("!t.engines || t.engines.includes(engine)");
    // **钉的是那句提示由 `fits` 驱动，不只是这段文案存在于文件里。**
    // 只查文案的话，把条件改成 `{false && …}` 照样绿 —— 文案还在，
    // 只是永远不显示。第一版就是这么写的。
    const i = SRC5.indexOf("{t.mismatch ??");
    const before = SRC5.slice(Math.max(0, i - 300), i);
    // 2026-09-26 起只在选中的卡上说（卡片上默认不挂红字）
    expect(before, "那句提示不是由 fits 驱动的").toContain("{on && !fits && (");
  });

  /** 方图会把「三个全身」出成 4 个 —— 选模板时就把画幅切对。 */
  it("选模板会切到它偏好的画幅", () => {
    expect(SRC5).toContain("if (t.prefersAspect) setAspectId(t.prefersAspect)");
  });

  /**
   * **选模板也要把引擎切对。**
   *
   * 四视图 / 七分区只有 Krea 2 出得了（版式 LoRA 只有它这一版）；
   * 选了模板而引擎还停在 Z-Image 的话，卡片上会跳一句红字让人自己去换 ——
   * 那一步本来就该自动。
   *
   * **判据必须是模板自己的 `engines` 白名单，不能再抄一份 id 名单** ——
   * 2026-09-08 这一天里「清单抄两处」已经出过四次。
   */
  it("选模板会切到它能用的引擎", () => {
    expect(SRC5).toContain('setEngine((t.engines?.[0] ?? "zimage")');
    expect(SRC5, "又抄了一份模板 id 名单").not.toMatch(
      /setEngine[\s\S]{0,80}(quad|panel7)"\s*(\?|===|\|\|)/,
    );
  });
});

describe("模板和引擎的对应关系", () => {
  /**
   * 白名单第一项就是「选这个模板该切到哪个引擎」——
   * 所以它不能为空数组，否则 `engines?.[0]` 会变成 undefined 再回落 zimage，
   * 而那个模板明明是有要求的。
   */
  it("有白名单的模板，第一项就是它要的引擎", () => {
    for (const t of CHARACTER_TEMPLATES) {
      if (!t.engines) continue;
      expect(t.engines.length, `${t.id} 的白名单是空的`).toBeGreaterThan(0);
      expect(typeof t.engines[0]).toBe("string");
    }
    // 图生那两个必须是 krea2 —— 版式 LoRA 只有它这一版
    for (const id of ["quad", "panel7"] as const) {
      expect(templateById(id).engines?.[0]).toBe("krea2");
    }
    // 五分区必须是 zimage —— 只有它的中文标注是 6/6
    expect(templateById("panel5").engines?.[0]).toBe("zimage");
    // 没白名单的回落 zimage（默认引擎）
    for (const id of ["simple", "detailed"] as const) {
      expect(templateById(id).engines).toBeUndefined();
    }
  });
});

describe("构图模板的示例图", () => {
  const SRC6 = readFileSync(
    fileURLToPath(new URL("./AssetWorkbench.tsx", import.meta.url)),
    "utf8",
  );

  /**
   * 三个模板的差别是**版面**。光靠一句话说不清「大头特写 + 三视图」
   * 和「三视图 + 大脸 + 三角度」到底差在哪 —— 一眼看图就知道。
   */
  it("卡片上有示例图", () => {
    expect(SRC6).toContain("styleTemplateThumbs()");
    expect(SRC6).toContain("thumbs[t.id]");
  });

  /**
   * **没出过示例图时显示占位，不能借别的模板的图。**
   * 借来的话，用户按着看到的图去选，出来完全不是那回事。
   */
  it("没有示例图时显示占位，不借别人的", () => {
    expect(SRC6).toContain("还没出示例图");
    const i = SRC6.indexOf("thumbs[t.id] ? (");
    expect(i, "示例图不是按 id 取的").toBeGreaterThan(-1);
  });
});

describe("七分区和五分区不共用提示词", () => {
  const SRC6 = readFileSync(
    fileURLToPath(new URL("./AssetWorkbench.tsx", import.meta.url)),
    "utf8",
  );

  /**
   * **两条路的提示词绝不能混。**
   *
   * 五分区那段构图写的是「四个带边框的分区、中文标注」，七分区的结构化模板
   * 写的是「七个区、English only」。两段一起发出去，模型会同时收到两套互相
   * 矛盾的版面指令 —— 出来的图不报错，只是版面乱掉，而看不出是哪来的。
   *
   * 这条钉的是「参考表那次调用里，`prompt` 是空串」。看起来像 bug（谁会发空
   * 提示词？），所以特别容易被人「顺手修好」成 `prompt: outPositive` ——
   * 那一改就把五分区那段塞进去了。
   */
  /**
   * **mode 必须跟着模板走，不能写死。**
   *
   * 写死成 `entity_sheet` 的话，点了四视图也出七分区 ——
   * 图照出、任务成功，只是不是你点的那个。第一版就是写死的。
   */
  it("mode 由模板的 pipeline 决定", () => {
    expect(SRC6).toContain('const mode = tpl.pipeline ?? "entity_sheet"');
    expect(SRC6).toContain("mode,");
    expect(SRC6, "mode 又被写死了").not.toContain('mode: "entity_sheet",');
    // 两种版式的等待提示不能一样 —— 一个 5 分钟一个 7 分半
    expect(SRC6).toContain('const quad = mode === "quad_view"');
  });

  it("参考表那条调用发的是空 prompt，不是拼好的那段", () => {
    const call = SRC6.slice(SRC6.indexOf("mode,"));
    const body = call.slice(0, call.indexOf("});"));
    expect(body).toContain('prompt: ""');
    expect(body, "把 outPositive 发进参考表了 —— 会和七区模板打架").not.toContain(
      "outPositive",
    );
    expect(body, "把负向词发进参考表了").toContain('negative_prompt: ""');
    // 而零件必须都在，否则模板拼出来没有身份锚点
    for (const f of ["entity_name", "entity_type", "identity_locks", "style_line"]) {
      expect(body, `参考表没送 ${f}`).toContain(f);
    }
  });

  /**
   * 走参考表时，提示词框和快捷标签**不显示** —— 显示一个不参与的输入框
   * 比不显示更糟：用户会在里面写东西然后发现「写了没用」。
   */
  it("走参考表时提示词区被藏起来", () => {
    expect(SRC6).toContain('className={isSheet ? "mb-3 hidden" : "mb-3"}');
    // 而判据是模板本身，不是引擎 —— 按引擎分流那版被否了（一张卡两种产出）
    expect(SRC6).toContain("const isSheet = !!templateById(template).pipeline;");
  });

  /**
   * 出图按钮必须按模板分流，否则点了七分区仍然走文生图。
   *
   * **断言的是分流这件事，不是那个三元表达式长什么样。**
   * 原来钉的是字面 `void (isSheet ? generateSheet() : generate())` ——
   * 2026-09-12 底部按钮改成「三种产物共用一个按钮」（场景多了宫格和
   * 全景两档）时它就红了，而被测的不变量一个字没变。
   */
  it("出图按钮按模板分流", () => {
    expect(SRC6).toContain("isSheet\n                  ? generateSheet()");
    // 参考表不吃提示词框，所以别拿它当禁用条件
    expect(SRC6).toContain("isSheet\n                  ? !!sheetWhy");
  });

  /**
   * **场景那三种产物共用底部那一个按钮。**
   *
   * 原来宫格和全景各自是一个按钮、摆在方位表那一段里，加上底部的
   * 「生成形象」一页三个 —— 用户原话「满页面都是生成按钮」。
   */
  it("场景的三种产物共用一个生成按钮", () => {
    for (const call of ["generateSceneSheet()", "generatePanoViews()", "generate()"]) {
      expect(SRC6, `底部按钮不会调 ${call}`).toContain(call);
    }
    // 方位表那一段里不该再有自己的生成按钮
    expect(SRC6).not.toContain("onClick={() => void generateSceneSheet()}");
    expect(SRC6).not.toContain("onClick={() => void generatePanoViews()}");
  });
});

/**
 * 构图模板原来分「文生 / 图生」两页；2026-09-26 精简到三张后并成一页
 * （用户：「构图模版一共就留 3 个，不用分页了」），图生那张卡上标「图生」。
 */
describe("构图模板一页三张", () => {
  const SRC7 = readFileSync(
    fileURLToPath(new URL("./AssetWorkbench.tsx", import.meta.url)),
    "utf8",
  );

  it("不再分页", () => {
    expect(SRC7).not.toContain("tplPage");
    expect(SRC7).not.toContain("studio-pager");
  });

  /**
   * **「图生」标签按 `pipeline` 判，不是写死的 id 名单。**
   * 写死的话，加一个新的图生模板忘了加进去，它就不带标签混在文生卡里 ——
   * 用户点了才发现要底图。
   */
  it("图生标签按 pipeline 判", () => {
    const i = SRC7.indexOf("{t.pipeline && (");
    expect(i, "卡片上没有按 pipeline 挂标签").toBeGreaterThan(0);
    expect(SRC7.slice(i, i + 600)).toContain("图生");
    expect(SRC7, "标签用了写死的 id").not.toMatch(/t\.id === "quad"[\s\S]{0,200}图生/);
  });

  /**
   * **卡片上默认没有红字**（用户：「把红字去掉」），但选中后引擎被手动换走时必须说 ——
   * 否则静默出一张错字 / 没版式的图。没底图的理由在选中后的说明块里。
   */
  it("红字只在选中且引擎对不上时出现", () => {
    expect(SRC7).toContain("{on && !fits && (");
    expect(SRC7).not.toContain("{!fits && (");
    expect(SRC7).not.toContain("{t.pipeline && sheetWhy && (");
    // 没底图的理由还在别处说
    expect(SRC7).toContain("{sheetWhy && (");
  });
});

describe("图生模板的底图", () => {
  const SRC8 = readFileSync(
    fileURLToPath(new URL("./AssetWorkbench.tsx", import.meta.url)),
    "utf8",
  );

  /**
   * **换了底图必须真的发出去。**
   *
   * 界面上换了、发出去的还是封面 —— 那是「点了没反应」，
   * 而且因为两张都是这个角色的图，出来的东西看着也像对的，最难查。
   */
  it("发出去的是覆盖后的那张，不是封面", () => {
    expect(SRC8).toContain("const sourcePath = srcOverride ?? coverPath;");
    const call = SRC8.slice(SRC8.indexOf("mode,"));
    const body = call.slice(0, call.indexOf("});"));
    expect(body).toContain("images: [sourcePath]");
    expect(body, "发的是封面而不是选中的底图").not.toContain("coverPath");
  });

  /** 默认必须跟着封面 —— 不设默认的话第一次点开是空的。 */
  it("默认跟着封面走", () => {
    expect(SRC8).toContain("useState<string | null>(null)");
    expect(SRC8).toContain("srcOverride === null");
  });

  /**
   * **上传必须落盘换成本地路径。**
   *
   * Tauri 的 webview 里 `<input type="file">` 拿不到真实路径，
   * 直接把 File 名字传下去的表现是「找不到源图」。
   */
  it("上传走 saveChatImageAttachment 换路径", () => {
    expect(SRC8).toContain("saveChatImageAttachment({ dataUrl");
    expect(SRC8).toContain("setSrcOverride(saved.path)");
  });

  /** 底图只在图生那条出现 —— 文生没有源图这回事。 */
  it("底图选择器只在图生模板显示", () => {
    const i = SRC8.indexOf("底图 —— 身份全部来自它");
    expect(i).toBeGreaterThan(0);
    const before = SRC8.slice(Math.max(0, i - 400), i);
    expect(before, "底图那块不是由 isSheet 驱动的").toContain("{isSheet && (");
  });

  /**
   * **「生成数量」对单张、宫格、全景都要生效。**
   * 以前只有单张读了 batch：选 ×2 出宫格 / 全景还是只出一份，界面上看不出任何异常（2026-09-17 用户发现）。
   */
  it("生成数量对单张、宫格、全景都生效", () => {
    for (const fn of ["const generate = async", "const generateSceneSheet = async", "const generatePanoViews = async"]) {
      const body = SRC.slice(SRC.indexOf(fn), SRC.indexOf("\n  };\n", SRC.indexOf(fn)));
      expect(body, fn).toContain("i < batch");
    }
  });

  it("先选出图方式再选数量；参考表不显示用不上的数量", () => {
    expect(SRC.indexOf('["sheet", "2×2 宫格"')).toBeLessThan(SRC.indexOf("生成数量</span>"));
    expect(SRC).toMatch(/\{!isSheet && \(\s*<>\s*<span className="studio-eyebrow">生成数量<\/span>/);
  });
});

/** 道具面板：默认选中道具信息卡、引擎用 Qwen 2.1（用户 2026-09-30：「默认使用 Qwen Image 生成」），取消后回产品摄影、不切画幅 */
describe("道具面板的构图卡", () => {
  it("道具也渲染构图卡，默认选中信息卡，引擎跟着切到它要的那个", () => {
    expect(SRC).toContain('(kind === "prop" ? PROP_TEMPLATES : CHARACTER_TEMPLATES).filter');
    expect(SRC).toContain('{(kind === "character" || kind === "prop") && imageUi && (');
    expect(SRC).toContain('kind === "prop" ? PROP_DEFAULT_TEMPLATE : "simple"');
    expect(SRC).toContain('kind === "prop" ? (templateById(PROP_DEFAULT_TEMPLATE).engines?.[0] ?? eff.engine) : eff.engine');
    // 取消选择回 `simple`（没有画幅偏好）—— 回 `portrait` 的话会把道具的画幅切成竖图
    expect(SRC).toMatch(/kind !== "character"\s*\?\s*"simple"/);
    expect(templateById("simple").prefersAspect).toBeUndefined();
  });
});

