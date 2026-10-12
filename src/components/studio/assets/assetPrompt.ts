/**
 * 资产出图的提示词拼装。
 *
 * 构图段**逐字照抄** LumenX 的 `CHARACTER_TEMPLATES`
 * （MIT，Copyright 2026 Alibaba）。那是他们调出来的东西 ——
 * 改一个词就可能换一种版面，而版面正是我们最在意的：
 * `simple` 的「左半边大头特写」是身份参考能用的**唯一原因**
 * （实测：三视图整排里脸只有 42~46px，可用下限是 150~200px）。
 *
 * 拼装单拎出来是因为这里的错**全是静默的**：风格没拼进去、
 * 构图段被覆盖掉、标签叠成一坨 —— 图照出，只是不对。
 */

export type AssetKind = "character" | "scene" | "prop";
export type TemplateId =
  | "avatar"
  | "portrait"
  | "simple"
  | "detailed"
  | "panel5"
  | "quadzh"
  | "bible"
  | "quad"
  | "panel7"
  | "propcard";

export interface Template {
  id: TemplateId;
  label: string;
  hint: string;
  /** 构图段。**英文原文**，模型对这段的服从度比中文高得多。 */
  composition: string;
  /** 这个构图专属的负向词。 */
  negative: string;
  /**
   * 只有这些引擎撑得住。空 = 都行。
   *
   * **卡的不是版面，是图上的中文标注。**
   *
   * 2026-09-08 用这个模板原文在 1216×832 同 seed 三方实测：
   *
   * | | 五分区 | 三视图 | 表情格数 | **图上中文标注** |
   * |---|---|---|---|---|
   * | Z-Image | ✅ | ✅ 正/侧/背 | ✅ 4 | ✅ **6/6 一字不差** |
   * | Krea 2 | ✅ | ✅ 正/侧/背 | ❌ 5 | ⚠️ **4/6**（`细节特写`→`谘忖特写`、`侧面`→`䐐们`） |
   * | FLUX.2 Klein | ✅ | ❌ 两个背面 | ❌ 3 | ❌ **0/12 全乱码** |
   *
   * **版面三个都撑得住** —— 界面上那句提示原来写的是「画不出来这种版面」，
   * 那是错的，已改成「的中文标注会出错」。
   *
   * **中文提示词读得懂 ≠ 中文字render得出来**，这两件事必须分开判：
   * Klein 对中文*内容*是原生的（「三个青苹果」数量都对），但汉字画不出来。
   * 所以「引擎吃中文」这个结论**不能**推广到设定卡上。
   *
   * 白名单只留 Z-Image，因为只有它是 6/6。Krea 2 差两个字 ——
   * 而设定卡上一个错字就是一张要重出的图。
   */
  engines?: string[];
  /** 这个构图适合的画幅。选错了数量会不准。 */
  prefersAspect?: string;
  /**
   * 引擎不在 `engines` 名单里时，界面上说的那句话。
   *
   * **必须逐模板写。** 原来是一句通用的「中文标注会出错」—— 对五分区是对的
   * （只有 Z-Image 是 6/6），对七分区完全不对：它的问题是**版式 LoRA
   * 只有 Krea 2 版**，和中文标注没关系。一句话套所有模板，
   * 用户会照着一个错的理由去换引擎。
   */
  mismatch?: string;
  /**
   * 这个模板走哪条链路。缺省 = 纯文生图（上面那些字段全部有效）。
   *
   * `"entity_sheet"` 那条**完全不同**：图生图（拿当前封面重排）、
   * 强制 Krea 2 + 版式 LoRA、约 7.5 分钟。走这条时 `composition` /
   * `negative` / `engines` / 快捷标签**全部不参与** —— 提示词由 Rust 侧
   * `entity_sheet::sheet_prompt` 按结构化模板拼，界面只送零件。
   *
   * # 为什么最后是「两个独立模板」
   *
   * 试过两版都不行：
   *
   * 1. **参考表放在工作台底部的折叠块** —— 顶上还留着一个同名的 t2i 模板，
   *    用户点「角色设定图」拿到的不是那个东西。
   * 2. **同一张卡按引擎分流**（选 Krea 2 就改走参考表）—— 更乱：
   *    一张卡两种产出，而提示词框里显示的是另一条路要用的那段，
   *    「五分区和七分区的提示词不一样，出来的内容会冲突」。
   *
   * 所以：**五分区和七分区是两张卡**，选哪张就是哪条路，提示词各归各。
   */
  pipeline?: "entity_sheet" | "quad_view";
  /**
   * **不在工作台的构图卡片里出现**，但版式本身还在用。
   *
   * 2026-09-26 用户把面板精简成「简洁三视图 + 设定卡（Qwen Only）」，
   * 去掉了头像 / 详细参考表 / 五分区 / 四视图（中文）。**只从面板上拿掉，不删**：
   * 批量出图（`batchAssets` / Rust `asset_batch`）还按引擎选 `quadzh`、
   * 按描述选 `avatar`，删了那两条路就断了。
   *
   * `portrait` 是「一张都没选」时用的版式，本来就没有卡片。
   */
  hidden?: boolean;
}

const SHEET_NEGATIVE =
  "text, labels, watermark, UI overlay, panel borders, frames, multiple separate images";

export const CHARACTER_TEMPLATES: Template[] = [
  {
    // **这一版式要排在第一个** —— `templateById` 认不出 id 时回落到 [0]，
    // 而回落到一张全身转身图，对一个头像是最坏的回落方向。
    // 排在这里不影响界面顺序（那是按数组渲染的，头像卡就该第一个）。
    id: "avatar",
    label: "头像 / 标识（没有身体）",
    hidden: true,
    hint: "一个居中的图标，没有身体、没有三视图",
    composition:
      "Composition: a single centered avatar icon on a plain neutral background, front-facing, symmetrical, filling most of the frame. This subject has NO body: do not draw a torso, arms, legs, clothing, shoes or a standing pose. Do not draw a turnaround sheet, multiple views, panels or borders. Flat even lighting, clean silhouette, crisp edges, suitable for display on a screen.",
    negative:
      "full body, standing pose, turnaround sheet, multiple views, multiple separate images, human figure, torso, arms, legs, clothing, shoes, photorealistic human, panel borders, frames, text, watermark",
    prefersAspect: "square",
  },
  {
    // **构图卡一张都不选时就是它**（用户 2026-09-26：「不是必须选一个，不选择默认居中头像」）。
    //
    // 和 `avatar` 不是一回事：`avatar` 是「这个东西没有身体」（图标、一团光），
    // 构图段里明令不画躯干；这个是**有身体的角色**的一张居中胸像，
    // 当封面和身份参考用 —— 脸占的像素远多于任何多视图版式。
    id: "portrait",
    label: "居中头像",
    hint: "不选构图时的默认：一张居中的头肩像",
    composition:
      "Composition: a single centered head-and-shoulders portrait of this one character, front-facing, eye-level, on a plain clean neutral background, filling most of the frame. Sharp facial details, soft even studio lighting. One image only, no other views, panels, borders or text.",
    negative: SHEET_NEGATIVE,
    hidden: true,
    prefersAspect: "portrait",
  },
  {
    id: "simple",
    label: "简洁三视图",
    hint: "左半边大头特写 + 右半边三个全身",
    composition:
      "Composition: character reference sheet, single unified image, seamless layout without borders or frames, neutral gray background. Left half: large head close-up portrait (shoulders up, sharp facial details, front-facing, detailed skin texture). Right half: three equally-sized full-body standing poses arranged side by side (front view, side view, back view), head-to-toe fully visible, relaxed neutral pose. Consistent soft studio lighting across all views, no harsh shadows, even illumination.",
    negative: SHEET_NEGATIVE,
  },
  {
    id: "detailed",
    label: "详细参考表",
    hidden: true,
    hint: "三个全身 + 大脸特写 + 三个角度头像",
    composition:
      "Composition: detailed character reference sheet, single unified image, seamless layout without borders or frames, neutral gray background. Left section: three full-body standing views side by side (front / side / back), head-to-toe visible, neutral relaxed pose. Upper right: large face close-up portrait (shoulders up, detailed skin texture, sharp eyes, pores visible). Lower right: three smaller head shots showing different angles (front, three-quarter, profile). Consistent soft studio lighting, no harsh shadows, even illumination across all panels.",
    negative: SHEET_NEGATIVE,
  },
  {
    id: "panel5",
    label: "五分区",
    hidden: true,
    hint: "主视图 + 三视图 + 表情格 + 细节格 · 带正确中文标注 · 只有 Z-Image",
    // 改自 LumenX 的 `design_sheet`。**尽量贴着原文改**，只动三处：
    //
    // 1. 去掉写死的赛博朋克（深蓝黑霓虹电路底、发光义眼、颈部电路纹身、
    //    武器挂扣、战斗暴怒）—— 那是他们演示角色的设定，不是通用模板。
    // 2. 删掉「带文字字段的信息面板」：短中文标题写得对，但面板里
    //    姓名年龄那些**值**必然是乱码（`Nirle` / `Killo`）。
    // 3. 表情和细节的**具体目标**换成通用的（calm/smiling/angry/surprised、
    //    手/鞋/领口/腰带）。写成「不同的表情」「distinctive features」
    //    这种笼统说法的话，四格会长得一模一样 —— 实测过。
    //
    // 4. 头尾各加一句（Codex 的建议，实测有效）：
    //    · `Keep the same character identity across every panel.`
    //    · `Do not create an information table, stats card, UI panel,
    //       watermark, or extra character body.`
    //    **禁令必须写进正向。** Z-Image 的负向是结构性无效的（cfg=1），
    //    写在负向框里等于没写。`extra character body` 这一句正好治好了
    //    「左栏冒出第二个几乎一样的大头像」。
    //
    // **教训：我一度自己重写了句式，把 `standing views (front / side / back)`
    // 换成了 `only three full-body views side by side`。数量是压住了，
    // 但那几个词正是决定姿势的 —— 出来侧面画成正面、背面是身体朝前头拧过去。**
    // 现在的写法是原文那半句原样保留，数量约束加在它外面。
    composition:
      "Keep the same character identity across every panel. Composition: professional character design sheet, single unified image with dark neutral background (deep charcoal with a subtle gradient). Layout divided into labeled panels with thin border frames: - Top left: large dramatic character portrait (bust shot, three-quarter angle, moody rim lighting), unlabeled - Center: exactly three full-body standing views and no other figures in this row (front / side / back), labeled \"正面\" \"侧面\" \"背面\" - Top right: 4 expression close-ups in a row (calm, smiling, angry, surprised), labeled \"表情特写\" - Bottom left: 3-4 detail close-up panels showing the hands, the footwear, a collar and fastening detail, a belt or pocket detail, labeled \"细节特写\". Cinematic lighting, high detail, concept art quality, game character sheet aesthetic. Do not create an information table, stats card, UI panel, watermark, or extra character body.",
    negative:
      "text paragraphs, info panel, stat block, watermark, UI overlay, multiple separate images",
    // 五分区带中文标注只有 Z-Image 是 6/6（Krea 2 是 4/6，FLUX.2 Klein 是 0/12）
    engines: ["zimage"],
    mismatch: "图上的中文标注会出错 —— 只有 Z-Image 是一字不差的",
    // 方图会把「三个全身」出成 4 个；横幅正好 3 个
    prefersAspect: "landscape",
  },
  {
    id: "quadzh",
    label: "四视图（中文）",
    hidden: true,
    hint: "正面头部特写 + 正/左/背三个全身 · 纯文生图，不用底图不用 LoRA · Z-Image",
    // **这一段是对着 liblib 的角色设定规范改的**，不是自己编的。
    //
    // 和我们原有的 `quad` 是两条完全不同的路：
    //
    // | | `quad` | `quadzh` |
    // |---|---|---|
    // | 链路 | 图生图，要先有封面 | **纯文生图** |
    // | 依赖 | Krea 2 + 专属 LoRA | 无 |
    // | 耗时 | 约 5 分 | 一张的时间 |
    // | 提示词 | 一句固定触发句 | 整段中文规范 |
    //
    // 三处刻意的改动：
    //
    // 1. **禁令写在正向里**（「不得出现……」），不写进 `negative`。
    //    Z-Image 的负向是结构性无效的（cfg=1）—— 写在负向框里等于没写。
    //    liblib 那份原文也是这么干的，两边应该踩的是同一个坑。
    // 2. **不带视觉风格。** 风格由项目的风格定调注入（`finalPositive`
    //    会把它插在描述和构图之间）—— 模板里再写一份就是两个所有者。
    // 3. **不要标注文字。** 原文里就有「不得出现文字、水印、标签」，
    //    留着 —— 四视图靠位置就说得清谁是谁，而图上多一个字就多一处
    //    要重出的风险。这和五分区那条路相反，那条是故意要中文标注的。
    composition:
      "高质量专业角色设定图，横向构图，纯白色纯净背景，中性摄影棚灯光，平光布光。" +
      "布局结构（必须是角色四视图）：正面面部头部特写（占图片水平 1/3 的空间，" +
      "**竖向放大，从头顶到胸口占满这一栏的完整高度**）" +
      "＋ 全身正面视图、全身左侧面视图、全身背面视图（三个并列排列，" +
      "占图片水平剩余的 2/3 的空间），四个视图之间用淡灰色(#E2E2E2)的 2px 细线分割。" +
      "光影：柔和的前侧光，清晰的轮廓定义，自然的肤色，面部清晰、服装可辨识，" +
      "平视镜头，完整全身，无裁剪。" +
      "确保所有视图中的面部特征、发型、体型和服装保持一致。" +
      "不得出现任何道具、武器、食物、饮料、手持物（角色必须空手）；" +
      "不得出现复杂动作、情绪表情、面部遮挡；" +
      "不得出现环境背景（仅纯白）；不得出现其他角色；" +
      "不得出现文字、水印、标签、UI 元素；不得过度风格化。" +
      // **真正治住「左栏两个头像」的是上面那句几何，不是这句禁令。**
      //
      // 实测过程：第一版左侧 1/3 那一栏里上下排了两个一模一样的头部特写。
      // 先按 `panel5` 记过的老办法加了一句正向禁令（「只有一个头部特写，
      // 不得出现第二个头像」）—— **同一个 seed 重出，照样两个**。
      // 成因是版式本身：「占水平 1/3」是一条整栏高的竖条，一个胸像填不满，
      // 模型就拿第二个来填。改成「竖向放大占满这一栏的完整高度」之后才对。
      //
      // 这是这个仓库第三次撞上同一条：**禁令不如结构**。
      "不得出现第二个头像或第二个身体。",
    // **禁令在正向里，这一栏是冗余的** —— Z-Image 的负向 cfg=1
    // 结构性无效，后端也只在引擎声明 `negative_prompt` 时才下发。
    // 还是写上：五分区那条中文模板也写了，而且它零成本 ——
    // 哪天这个版式被允许换引擎，缺了它就是一处静默的退化。
    negative: SHEET_NEGATIVE,
    engines: ["zimage"],
    mismatch: "这段构图规范是中文长句 —— 只有 Z-Image 对长中文提示词是原生的",
    // 四个视图横排，方图会把三个全身挤成两个
    prefersAspect: "landscape",
  },
  {
    id: "bible",
    label: "设定卡（Qwen Only）",
    hint: "三视图 + 八格细节 + 状态照 + 色板 · 名字写在图上 · Qwen 2.1 · 约 5 分",
    // **改自用户给的陆川设定卡提示词**（2026-09-26），那一段在 Qwen-Image 2.1 上
    // 实测出了目前最好的设定卡：「陆川」一字不差、8 行英文资料全对、
    // 十几个格子是同一张脸（同题 Z-Image 写错名字且右侧格子换人，Krea 2 也写错字）。
    //
    // 抽成通用模板时动了这些：
    //
    // 1. **人物本身全部拿掉**（外貌、工装、激光刀、18 岁回忆）—— 那些由资产描述
    //    提供，`buildPrompt` 会把「名字，描述」放在这段前面。
    // 2. **画风拿掉**（原文的 semi-realistic 3D）—— 由项目风格注入，
    //    模板里再写一份就是两个所有者（和 `quadzh` 同一条理由）。
    // 3. **资料栏的值改成留空横线**：名字以外的年龄、身高、职业模板不知道，
    //    让模型填就是编。原文作者自己给的备选方案就是这一版。
    // 4. **细节格和状态照换成通用的**：原文是陆川专属的（根权限掌纹、上城伪装），
    //    这里是每个角色都有的（脸、眼神、手、配饰、鞋、标志物、面料、第二套造型）。
    //
    // `{name}` 由 `buildPrompt` 换成资产名 —— 名字能被一字不差写上去，
    // 前提是它作为**带引号的确切文字**出现在提示词里。
    composition:
      "Composition: a wide, premium, art-directed character-design sheet on a clean white and cool light-gray technical background with thin pale-cyan grid lines and restrained interface graphics. The same person described above appears consistently in every panel, with the same face, age, hairstyle, body proportions, clothing and accessories. The upper-left contains no emblem and no logo, only the exact title “{name}” and beneath it “CHARACTER SHEET”; the narrow profile column below the title is clean empty ruled lines reserved for later typesetting. The central area contains exactly three complete, equally sized, head-to-toe orthographic full-body views in a relaxed neutral A-pose, labeled “FRONT”, “SIDE” and “BACK”; the legs are straight and naturally proportioned and the hands are anatomically correct and relaxed. The right side is an orderly eight-panel section headed “DETAILS”: “FACE / EVERYDAY” shows the usual expression; “EYES / FOCUSED” shows the gaze sharp and concentrated; “HAND DETAIL” shows the hands doing something characteristic of this person; “GEAR” shows the most distinctive accessory up close; “FOOTWEAR” shows the shoes; “SIGNATURE ITEM” neatly displays the object most associated with this person; “FABRIC / MATERIAL” shows the main garment's texture up close; “ALTERNATE LOOK” shows the same person in a second outfit if one is described above, otherwise a three-quarter close-up. Along the bottom are three cinematic state portraits of the same face labeled “EVERYDAY”, “FOCUSED” and “UNDER PRESSURE”, beside organized sections labeled “COLOR PALETTE” and “MATERIAL BREAKDOWN” with swatches taken from the outfit. Clean editorial hierarchy like a meticulously art-directed AAA game character bible, accurate front-side-back turnaround, realistic materials, soft neutral studio light and extremely high material clarity; the three figures occupy most of the central height while every section stays spacious and readable. The sheet contains only this one identity; every full-body figure has two arms, two legs, natural five-fingered hands and complete visible head and feet; the central figures wear only the main outfit; uncertain secondary copy areas are left blank rather than filled with invented writing.",
    // cfg=1，负向结构性无效；**不能用 `SHEET_NEGATIVE`** —— 那里面有 `text, labels`，
    // 而这张卡要的正是图上的文字。哪天换到吃负向的引擎，那一条会把标注全擦掉。
    negative: "watermark, extra character body, multiple separate images",
    engines: ["qwen21"],
    mismatch: "名字和标注只有 Qwen-Image 2.1 写得对，别的引擎会写错字、格子里换人",
    prefersAspect: "landscape",
  },
  {
    id: "quad",
    label: "四视图（Krea 2 版式）",
    hint: "大脸特写 + 正/侧/背三全身 · 拿封面图扩视角 · Krea 2 · 约 5 分",
    // **这条不走提示词**：四视图的训练 caption 就是一句固定触发句
    //（`entity_sheet::QUAD_TRIGGER`），塞结构化模板进去反而让版式不稳。
    composition: "",
    negative: "",
    pipeline: "quad_view",
    engines: ["krea2"],
    mismatch: "四视图靠 Krea 2 专属的 LoRA，换引擎出不来这个版面",
    prefersAspect: "landscape",
  },
  {
    id: "panel7",
    label: "七分区",
    // 2026-09-26 用户点名从面板上清掉。版式本身还在：对话工具的
    // `layout: entity_sheet` 和能力目录的「参考表」模式都走它。
    hidden: true,
    hint: "多剪影 / 动作 / 六格细节 · 拿封面图重排 · Krea 2 · 约 7 分半",
    // **这条不走提示词**（`pipeline: "entity_sheet"`）。留空串而不是删字段：
    // `Template` 的形状对所有模板一致，少一个就得在每个读它的地方加判空。
    composition: "",
    negative: "",
    pipeline: "entity_sheet",
    // 版式 LoRA 只有 Krea 2 版，挂到别的引擎上加载「成功」但一点作用都没有
    engines: ["krea2"],
    mismatch: "七区版式靠 Krea 2 专属的 LoRA，换引擎出不来这个版面",
    // 七个区是按 3:2 排的，方图/竖图会挤成一团
    prefersAspect: "landscape",
  },
];

/**
 * 道具的构图卡。**一张都不选时**是下面的 `PROP_COMPOSITION`（产品摄影：主视图 + 材质特写）。
 */
export const PROP_TEMPLATES: Template[] = [
  {
    id: "propcard",
    label: "道具信息卡（Qwen）",
    hint: "上排正 / 侧 / 背三视图 + 下排两格细节特写 · 16:9 · Qwen 2.1",
    // **照抄 Codex 出《回收师》道具卡的那段提示词**（`resources/回收师/道具设定/星澜专属协议铭牌-三视图-v1.png.generation.md`），
    // 用户 2026-09-30：「直接用那个提示词做模版就可以了」。同一版式的道具卡已经出了 7 张（激光刀、拆机台、工具车……）。
    //
    // 只动了两处：
    // 1. **星澜铭牌专属的内容换成占位**：主体、配色、材质由资产描述提供（`buildPrompt` 放在这段前面）；
    //    下排两个特写框原文写死的是「GEM / PROTOCOL LIGHT」「REAR INTERFACE」，改成让模型挑这件道具最有辨识度的两处；
    //    原文的「Input images」是那次多参考图生成的，文生图里没有，去掉。
    // 2. **「Avoid」那行改成正面写法**：Qwen 在我们这里负向不生效，写「不要人物」会被 `negationWarning` 拦下提醒，
    //    而且这个仓库实测过不止一次 —— 禁令不如直接写清画面里有什么。
    //
    // 画风那一行只留和版式有关的（工业产品渲染、美术资产生产图）；「国漫科幻」是《回收师》的，由项目风格注入。
    composition:
      "Use case: stylized-concept\n" +
      "Asset type: 道具三视图设定卡\n" +
      "Primary request: 生成“{name}”的高精度三视图道具卡，外观严格按上面的描述。\n" +
      "Scene/backdrop: 冷白略带浅蓝的无缝工业设计图背景，细灰蓝辅助线和分区线，无环境场景。\n" +
      "Style/medium: 高精度 3D 半写实工业产品渲染，美术资产生产图。\n" +
      "Composition/framing: 16:9 横向设定卡。上排从左到右是 FRONT 正面、SIDE 严格90度侧面、BACK 背面，比例一致、无遮挡。" +
      "下排两个大特写框：左右各放大展示这件道具最有辨识度的一处结构或细节（材质、机构、刻印或标签），每个框下方用简短的英文大写标注这处细节的名称。" +
      "主体占比适中，边缘完整，不裁切。\n" +
      "Lighting/mood: 冷白柔光、轻微轮廓光，金属反射克制，严谨、洁净。\n" +
      "Text (verbatim): “FRONT”, “SIDE”, “BACK”；道具本身上的文字、编号照描述原样写。\n" +
      "Constraints: 必须是同一个道具的严格三视图；正侧背结构相互对应；这是独立道具卡，画面里只有这一件道具本身；" +
      "所有小字尽量准确、清晰、简洁；除三视图标注、两个特写框的标注和道具本身的文字外，画面里只有干净的背景和辅助线。",
    // 负向在 Qwen 上不生效（`style::honors_negative`），写上是为了哪天换到吃负向的引擎时不退化。
    // **不能有 `text, labels`** —— 这张卡要的正是 FRONT / SIDE / BACK 那些标注（和 `bible` 同一条理由）
    negative: "watermark, signature, people, hands, human body, multiple separate images, cropped object",
    engines: ["qwen21"],
    mismatch: "这张卡的版式和标注是按 Qwen-Image 2.1 定的，别的引擎标注会写错字",
    prefersAspect: "landscape",
  },
];

/**
 * 道具面板打开时默认选中的卡。用户 2026-09-30：道具加上这张信息卡模板，「默认使用 Qwen Image 生成」。
 * 再点一次取消，回到 `PROP_COMPOSITION`（产品摄影）
 */
export const PROP_DEFAULT_TEMPLATE: TemplateId = "propcard";

/** 场景只有一种构图 —— 没有「脸」的问题，不需要选。道具不选卡时也用下面这段 */
const SCENE_COMPOSITION =
  "Composition: wide establishing shot of the environment on neutral gray background, single unified image, no figures in foreground. Emphasize atmosphere, architecture and terrain structure. Lighting and color palette match the scene mood. Soft volumetric lighting, depth of field.";
const PROP_COMPOSITION =
  "Composition: product photography style on neutral gray background, single unified image, seamless layout without borders. The object alone is the only thing in the frame. Main view: object centered at slight angle. Secondary views: detail close-ups of material and texture. Clean even studio lighting, subtle shadow beneath object.";
const PLAIN_NEGATIVE = "text, labels, watermark, UI overlay, panel borders, frames";

/** 快捷标签。点一下往提示词末尾追加。 */
export const QUICK_TAGS: Record<AssetKind, string[]> = {
  character: [
    "full body",
    "close-up",
    "three-view",
    "dynamic pose",
    "soft lighting",
    "studio lighting",
    "white background",
    "detailed face",
  ],
  scene: [
    "wide angle",
    "establishing shot",
    "golden hour",
    "dramatic lighting",
    "aerial view",
    "depth of field",
    "atmospheric",
    "cinematic",
  ],
  prop: [
    "product shot",
    "white background",
    "multi-angle",
    "studio lighting",
    "macro detail",
    "floating",
    "transparent background",
    "clean",
  ],
};

export function templateById(id: TemplateId): Template {
  // **回落到 `simple`，不是数组第一个。** 第一个现在是 `avatar`（头像），
  // 把一个认不出的 id 回落成「没有身体」是比回落成全身图更坏的方向 ——
  // 真人角色会变成一个图标，而且不报错。
  return (
    CHARACTER_TEMPLATES.find((t) => t.id === id) ??
    PROP_TEMPLATES.find((t) => t.id === id) ??
    CHARACTER_TEMPLATES.find((t) => t.id === "simple")!
  );
}

/** 这个资产、这张构图卡要用的构图段。道具只有选了 `PROP_TEMPLATES` 里的卡才换，其余一律产品摄影那段 */
function compositionFor(kind: AssetKind, template: TemplateId): string {
  if (kind === "character") return templateById(template).composition;
  if (kind === "scene") return SCENE_COMPOSITION;
  return PROP_TEMPLATES.find((t) => t.id === template)?.composition ?? PROP_COMPOSITION;
}


/**
 * 由资产本身生成初始提示词：`名字，描述` + 空行 + 构图段。
 *
 * **描述在前、构图在后。** 反过来的话，SDXL 那种分块取平均的编码器
 * 会让长长的构图段把「这个人长什么样」挤掉 ——
 * 出来是一张版面很标准但不是这个人的图。
 */
/**
 * 把资产描述压成**一句给出图模型看的话**。
 *
 * # 标签是给人看的，不该发给模型
 *
 * 槽位化之后，资产描述长这样：
 *
 * ```text
 * 描述：盛唐女官，女性，28岁，……
 * 服装：上衣男装官服上衣，……
 * 特征：黑眼圈明显。
 * ```
 *
 * 这个形状在资产卡上好读，但**原样发给出图模型是三件坏事**：
 * `描述：` `服装：` 这些标签是画面内容里没有的词；中间的硬换行把一段
 * 提示词切成三段；而 Z-Image 是会把汉字画进画面的那一个 ——
 * 标签有被当成标注渲染出来的风险。
 *
 * 所以：去掉标签、去掉换行、拼成一句。**值全留着**，一个字不删。
 */
export function flattenDescription(desc: string): string {
  return desc
    .split("\n")
    .map((line) => line.replace(/^\s*(描述|服装|特征|外观|说明)\s*[:：]\s*/, "").trim())
    .filter(Boolean)
    .join("")
    .replace(/。，/g, "。");
}

/** 段落分隔。**风格插在第一个空行之前** —— 见 `finalPositive`。 */
const PARA_BREAK = "\n\n";

/**
 * 去掉「出一张图」那一段，给宫格用。
 *
 * # 为什么必须去
 *
 * 场景的构图段（`SCENE_COMPOSITION`）是给**单张**场景图写的，里面有：
 *
 * ```text
 * wide establishing shot ... single unified image ... depth of field
 * ```
 *
 * 而宫格那条路，Rust 侧会把它当成「场景描述」原样嵌进宫格提示词里，
 * 于是模型同时收到两套互相矛盾的指令：
 *
 * - `a 2x2 grid contact sheet, clean white borders separating panels`（宫格的）
 * - `single unified image` + `wide establishing shot`（这一段的）
 *
 * 「single unified image」就是**出一张**，`wide establishing shot` 又把机位
 * 钉死成一个。2026-09-12 实拍：四格出来是同一个正面机位，几乎一模一样。
 *
 * **版式和机位由宫格自己负责**，这里只该留场景描述。
 *
 * 判据是「这一段里有没有 `single unified image`」，不是整段精确匹配 ——
 * 用户改过提示词之后精确匹配就失效了，而那正是最需要它的时候。
 */
export function withoutSingleImageComposition(prompt: string): string {
  return prompt
    .split(PARA_BREAK)
    .filter((part) => !/single unified image/i.test(part))
    .join(PARA_BREAK)
    .trim();
}

/** 带屏幕的道具。 */
const SCREEN_DEVICE = /手机|电脑|显示器|屏幕|平板|电视|笔记本|监控|phone|laptop|monitor|tablet|screen|\btv\b/i;
/** 描述里说了屏幕是亮的、在显示东西。 */
const SCREEN_ON = /亮|显示|提示|通知|消息|界面|画面|播放/;

/**
 * 带屏幕的道具，**正面写清屏幕上是什么**。
 *
 * 2026-09-14 实测：「手机，屏幕亮起显示提示，屏幕里不要显示任何人」出了 5 张，
 * 5 张屏幕里都是一张男人的脸。只写「显示提示」，模型会自己往屏幕里填内容；
 * 而 Z-Image 负向不生效，「不要……人」里它只认得「人」。
 *
 * 描述里说了亮屏 / 显示什么，就写成纯界面；没说就写熄屏。返回空 = 不是屏幕类道具。
 */
export function screenHint(name: string, description: string): string {
  if (!SCREEN_DEVICE.test(`${name} ${description}`)) return "";
  return SCREEN_ON.test(description)
    // 消息列表左边要写死成小圆点：只写「消息条」时左边会画带人脸的小头像（实测 3 张里 2 张），
    // 写「纯色字母圆标」还剩 1 张；写成蓝色小圆点才没有（实测，见提交说明）
    ? "屏幕画面是纯界面：几行文字通知，每行前面一个蓝色小圆点，界面平铺在屏幕上"
    : "屏幕熄灭，黑色玻璃面反光";
}

export function buildPrompt(
  kind: AssetKind,
  entity: { name: string; description?: string },
  template: TemplateId = "simple",
): string {
  const name = entity.name.trim();
  const desc = flattenDescription((entity.description ?? "").trim());
  const screen = kind === "prop" ? screenHint(name, desc) : "";
  const head = [desc ? `${name}，${desc}` : name, screen].filter(Boolean).join("。");
  return `${head}\n\n${withName(compositionFor(kind, template), name)}`;
}

/**
 * 构图段里的 `{name}` 换成资产名。
 *
 * 设定卡要把名字**一字不差**写在图上，而模型只照抄带引号的确切文字 ——
 * 写成「标题是这个角色的名字」它会自己编一个。
 */
export function withName(composition: string, name: string): string {
  return composition.split("{name}").join(name.trim());
}

/** 这个构图专属的负向词。 */
export function templateNegative(kind: AssetKind, template: TemplateId = "simple"): string {
  if (kind === "character") return templateById(template).negative;
  if (kind === "prop") return PROP_TEMPLATES.find((t) => t.id === template)?.negative ?? PLAIN_NEGATIVE;
  return PLAIN_NEGATIVE;
}

/**
 * 往提示词末尾追加一个标签。
 *
 * **要自己补分隔符。** 不补的话点两下就变成
 * `…even illuminationfull bodyclose-up` —— 模型把它当成一个生造的词，
 * 而且不报错。
 */
export function appendTag(prompt: string, tag: string): string {
  const base = prompt.trimEnd();
  if (!base) return tag;
  const needsComma = !/[,，]$/.test(base);
  return `${base}${needsComma ? "," : ""} ${tag}`;
}

/**
 * 构图段的起点 = **第一个空行**，不是某个关键词。
 *
 * # 为什么不认 `Composition:`
 *
 * 原来这里找的是字符串 `"\n\nComposition:"`。2026-09-06 采纳 Codex 的建议
 * 之后，`design_sheet` 的构图段变成了
 * `Keep the same character identity across every panel. Composition: …` ——
 * `Composition:` 不再顶在段首，锚点匹配不上，风格**静默回落到末尾追加**，
 * 把实测出来的「风格必须在构图之前」那条结论整个抵消掉了。
 * 图照出、不报错，只是风格没生效。
 *
 * 段落边界不会因为改一句英文而变：`buildPrompt` 永远是
 * `主体\n\n构图段`。所以认段落，不认词。
 */

/**
 * 最终正向。**风格插在主体之后、构图段之前。**
 *
 * # 这条和镜头那边的规矩不一样，是实测定的
 *
 * 后端 `style::apply_style` 的规矩是「画面在前、风格在后」，那条针对的是
 * **短提示词**（一句画面描述 + 一段风格词）。
 *
 * 资产不一样：中间夹着 600 字符的构图段，而那段全是摄影语言 ——
 * `detailed skin texture`、`sharp facial details`、`studio lighting`、
 * `even illumination`。风格词排在它后面会被彻底淹没。
 *
 * 2026-09-05 同 seed 对照实测（「温暖手绘动画」+ 小花的设定卡）：
 *
 * | 风格词位置 | 出来是什么 |
 * |---|---|
 * | 构图段**之后**（原来的做法） | **写实照片猫**，风格几乎没生效 |
 * | 构图段**之前**、紧跟主体 | **手绘插画猫**，风格生效 |
 *
 * 所以这里插在**第一个空行之前**（= 主体描述之后、构图段之前）。
 * 用户把构图段整个删了、提示词只剩一段的话，回到末尾追加。
 *
 * 风格词已经在提示词里出现过就不再叠（用户可能自己抄进去了）。
 */
export function finalPositive(prompt: string, stylePositive: string, applyStyle: boolean, kind: AssetKind = "character"): string {
  const base = prompt.trim();
  const style = applyStyle ? styleForKind(kind, stylePositive) : "";
  if (!style) return base;
  if (!base) return style;
  if (base.includes(style)) return base;
  const at = base.indexOf(PARA_BREAK);
  if (at < 0) return `${base}, ${style}`;
  return `${base.slice(0, at)}，${style}${base.slice(at)}`;
}

/** 风格词里描述人的那些。 */
const HUMAN_STYLE_WORD = /human|skin|face|facial|actor|actress|performance|portrait|body|proportion|\bhands?\b|fingers?|\beyes?\b|hair|emotion|people|person|character|pose|人物|人体|皮肤|脸|面部|表情|演员|表演|身材|五官|发型|姿势/i;

/**
 * 场景、道具只取风格里的**质感、色调、镜头感**，去掉描述人的词。
 *
 * 2026-09-14 实测：项目风格 Cinematic Realism 的正向词里有 `live-action film still`、
 * `natural human proportions`、`realistic skin texture`、`grounded emotional performance`，
 * 原样插进手机道具的提示词 —— 5 张图屏幕里全是人脸。角色照旧用全套。
 *
 * 按逗号拆开逐条判断，整条丢，不在一条里删半截词。
 */
export function styleForKind(kind: AssetKind, stylePositive: string): string {
  const style = stylePositive.trim();
  if (kind === "character" || !style) return style;
  return style
    .split(/[,，]/)
    .map((t) => t.trim())
    .filter((t) => t && !HUMAN_STYLE_WORD.test(t) && !/live-action/i.test(t))
    .join(", ");
}

/** 否定写法。 */
const NEGATION = /不要|不能|不许|别出现|不出现|没有人|无人|禁止|避免|\bno\s+\w+|\bwithout\b|\bavoid\b/i;

/**
 * 负向不生效的引擎上，正向里写了「不要……」要提醒：模型不认否定，只认后面那个词。
 * 返回空 = 没问题。
 */
export function negationWarning(prompt: string, negativeWorks: boolean): string {
  if (negativeWorks) return "";
  const m = NEGATION.exec(prompt);
  if (!m) return "";
  return `提示词里有「${m[0].trim()}」这种否定写法 —— 这个引擎不认否定，只认后面那个词，写「不要人」反而会画出人。改成正面描述画面里有什么，比如「屏幕上是几条文字消息」。`;
}

/**
 * 最终负向 = 风格负向 + 构图负向。
 *
 * 顺序无所谓（负向没有主体），但**空的那段不能留下一个孤零零的逗号** ——
 * 有些引擎会把 `, ` 当成一个 token。
 */
export function finalNegative(
  styleNegative: string,
  compositionNegative: string,
  applyStyle: boolean,
): string {
  return [applyStyle ? styleNegative.trim() : "", compositionNegative.trim()]
    .filter(Boolean)
    .join(", ");
}

/**
 * 选中的构图卡，和**实际要发出去的提示词**，是不是同一回事。
 *
 * # 为什么要问这个
 *
 * 提示词被改过（`dirty`）之后再点另一张构图卡，卡片会亮、画幅会跟着切，
 * 但提示词**不动**（那是对的 —— 一次点击不该抹掉用户写的东西）。
 * 结果是界面说「角色设定图」、请求里还是上一个版面 ——
 * 又一个「点了、亮了、没生效」。
 *
 * 这里**从提示词本身推**，不另外记一个状态位：状态位会和真实内容跑偏，
 * 而这个问题的本质就是「显示和真实跑偏了」。
 */
export function templateApplied(
  prompt: string,
  kind: AssetKind,
  template: TemplateId = "simple",
): boolean {
  // 带 `{name}` 的构图段拼进提示词时名字已经换进去了 —— 按占位符切开逐段找
  return compositionFor(kind, template).split("{name}").every((part) => prompt.includes(part));
}
