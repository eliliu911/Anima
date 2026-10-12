/**
 * LoRA 的用途说明 —— 列表里只有文件名时，用户根本不知道哪个是干什么的。
 *
 * **还要标出"不该在这里选"的那些。** ComfyUI 的 LoRA 列表是全局的，里面混着
 * 视频模型的 LoRA（`minimax_h3_*`）。把它们挂到角色设定卡上**不生效、也不报错** ——
 * 又是那种"看起来成功了"的失败。这里明确标出来，并在界面上排到最后。
 *
 * 匹配用**前缀/关键词**而不是完整文件名：同一个 LoRA 会有 v1/v2、fp16/bf16
 * 各种命名变体，写死全名的话换个版本就失配、说明凭空消失。
 */
export interface LoraNote {
  /** 一句话说清用途 */
  text: string;
  /** 不适合用在角色设定卡上（架构不对/用途不对） */
  incompatible?: boolean;
}

const NOTES: { match: (n: string) => boolean; zh: LoraNote; en: LoraNote }[] = [
  {
    match: (n) => n.toLowerCase().includes("charturn"),
    zh: { text: "多视图版式 —— 设定卡的核心，工具会自动挂，不用手动加" },
    en: { text: "Turnaround layout — the sheet's core; added automatically, no need to pick it" },
  },
  {
    match: (n) => n.toLowerCase().includes("faceturn"),
    zh: { text: "多视图人脸 —— 只转头部，适合做表情/角度参考卡" },
    en: { text: "Face turnaround — head only, for expression and angle reference sheets" },
  },
  {
    // 作者填了 19 个触发词，注释里只放最常用的几个 —— 全列会把下拉撑爆
    match: (n) => n.toLowerCase().includes("nsfwpov") || n.toLowerCase().includes("nsfw_pov"),
    zh: { text: "成人姿势合集（SDXL）—— 触发词按动作写，如 doggy style / threesome / undressing；建议 0.6~0.9" },
    en: { text: "Adult pose pack (SDXL) — action triggers e.g. doggy style / threesome / undressing; weight 0.6-0.9" },
  },
  {
    // Z-Image 架构 —— 挂它**不会**回落 SDXL，长提示词和画质都保得住
    match: (n) => n.toLowerCase().includes("straight_hair_zimgt"),
    zh: { text: "Z-Image·角色｜锁定一位长直发亚洲女性 —— 触发词 j1v_w0m" },
    en: { text: "Z-Image character — long straight-hair Asian woman; trigger: j1v_w0m" },
  },
  {
    match: (n) => n.toLowerCase().includes("shorty_asian_zimgt"),
    zh: { text: "Z-Image·角色｜锁定一位日系亚洲女性 —— 触发词 sh0r7y_asian" },
    en: { text: "Z-Image character — Japanese-style Asian woman; trigger: sh0r7y_asian" },
  },
  {
    match: (n) => n.toLowerCase().includes("pintrest_zit") || n.toLowerCase().includes("pinterest_zit"),
    zh: { text: "Z-Image·风格｜Pinterest 风生活感人像，无触发词，直接生效" },
    en: { text: "Z-Image style — Pinterest-style candid portraits; no trigger word needed" },
  },
  {
    match: (n) => n.toLowerCase().includes("guofeng"),
    zh: { text: "国风插画风格 —— 触发词 guofeng, chinese style" },
    en: { text: "Chinese illustration style — trigger: guofeng, chinese style" },
  },
  {
    match: (n) => n.toLowerCase().includes("hanfu"),
    zh: { text: "汉服唐风服制 —— 触发词 hanfu, tang style outfits" },
    en: { text: "Tang-dynasty hanfu — trigger: hanfu, tang style outfits" },
  },
  {
    match: (n) => n.toLowerCase().includes("asian_beauty") || n.toLowerCase().includes("asian beauty"),
    zh: { text: "亚洲面容倾向 —— 把脸往东亚审美拉，权重别太高否则脸会同质化" },
    en: { text: "Asian facial bias — pulls faces toward East Asian features; keep the weight low" },
  },
  {
    match: (n) => n.toLowerCase().includes("add-detail") || n.toLowerCase().includes("add_detail"),
    zh: { text: "细节增强 —— 加布料纹理和皮肤细节，0.3~0.6 就够，高了会出噪点" },
    en: { text: "Detail enhancer — fabric and skin texture; 0.3–0.6 is enough, higher adds noise" },
  },
  // ── Krea 2（2026-09-17 逐个出图实测 + 联网核对出处）───────────────────
  //
  // 每一条都是**真的出图看过**的，不是照文件名猜的。方法和完整对照见
  // `docs/plans/2026-09-17-krea2-lora-inventory/README.md`。
  //
  // 两件那天学到的事，写在这里免得下次又踩：
  //
  // 1. **文件名不能当用途说明。** 三个服装 LoRA 按名字猜，猜错两个 ——
  //    「红肚兜」出的是中式立领长袖衫，「包臀裙」出的是 OL 通勤装。
  // 2. **测试方法要匹配被测对象。** 中性人像提示词能测风格/脸型/质感，
  //    但测不了工具类：`Purple_Graphics` 是海报 LoRA、`style_reference`
  //    要参考图，拿人像提示词去测，看到的是它们**空转的样子**。
  {
    match: (n) => n.toLowerCase().includes("nexblend") && n.toLowerCase().includes("krea2"),
    zh: { text: "Krea 2·强风格｜半写实东亚人像。实测：整张脸换成偶像脸（唇形饱满、颧骨立体）、皮肤瓷化但不假、光影变戏剧化、机位压低。作者原话是「针对东亚特别是韩式面部美学优化，避免塑料皮肤」。无触发词。出处 Civitai 2752066（NexBlend）" },
    en: { text: "Krea 2 strong style — semi-realistic East Asian portrait. Measured: swaps in an idol-like face, porcelain-but-not-plastic skin, dramatic light, lower camera. No trigger. Civitai 2752066" },
  },
  {
    match: (n) => n.toLowerCase().includes("cc-mj") || n.includes("影视柔光"),
    zh: { text: "Krea 2·强风格｜Midjourney 味的影视柔光滤镜。实测：逆光发丝边缘光、柔光罩面、背景从素墙变灰蓝渐变、毛衣变粗针织。**它会改姿势和构图**（手插兜、身体侧转）—— 做分镜时它会跟你抢镜头，留神。作者建议和人像 LoRA 搭配，两个都挂时各 1.0~1.1；单挂 0.5~1.0。出处 RunningHub「krea2-Cc」系列" },
    en: { text: "Krea 2 strong style — Midjourney-ish cinematic soft light. Measured: rim light, soft glow, gradient backdrop. It also changes pose and framing, so it fights your storyboard. Pair with a portrait LoRA at 1.0-1.1 each, or 0.5-1.0 alone" },
  },
  {
    match: (n) => n.toLowerCase().includes("krea2_style_reference"),
    zh: { text: "⚠️ **已于 2026-09-17 删除**（用户决定）。删之前查清了：它是 ostris 的**参考图风格迁移**工具，要 1~2 张参考图 + 专用 ComfyUI 节点，纯文生图下只会把画面抹平 —— 那是它空转的样子。出处 Civitai 2764349。重下回来的话这条说明直接可用 —— 原说明：Krea 2·工具｜**参考图风格迁移，不是文生图风格 LoRA**。作者是 ostris（AI Toolkit 作者），用 Krea2 reference 训练器训的，**要 1~2 张参考图 + 专用 ComfyUI 节点**才是它的用法。纯文字提示词下它只会把画面「抹平」（阴影消失、背景变纯色、电商棚拍感）—— 那是它空转的样子，不是用途。出处 Civitai 2764349" },
    en: { text: "Krea 2 tool — reference-image style transfer, NOT a text-to-image style LoRA. By ostris; needs 1-2 reference images plus a custom ComfyUI node. With text alone it just flattens the image — that is it idling, not its purpose. Civitai 2764349" },
  },
  {
    match: (n) => n.toLowerCase().includes("krea2_identity_edit"),
    zh: { text: "Krea 2·工具｜身份编辑（img2img）。换脸/头、角色表、扩图、内补、试衣、去人。**必须有源图**，纯文生图用不了。**这就是七分区参考表和四视图管线在用的那个**。作者建议 `ref_boost` 2~6（我们默认 4）。`_r128` 是同一个模型的 rank-128 版，更大更强。出处 Civitai 2761113（conrad_locke v1.2）" },
    en: { text: "Krea 2 tool — identity edit (img2img): head/face swap, character sheets, outpaint, inpaint, try-on. Needs a source image. This is what the entity-sheet and QuadView pipelines use. Author suggests ref_boost 2-6 (our default is 4). Civitai 2761113" },
  },
  {
    match: (n) => n.toLowerCase().includes("purple_graphics"),
    zh: { text: "⚠️ **已于 2026-09-17 删除**（用户决定）。它其实是**海报 / 平面设计** LoRA（Civitai 2326705，权重 0.8~1.2），不是人像风格 —— 我当初拿人像提示词测它，只看到「胶片质感」那种假象。做分集封面/标题卡时可以考虑重下 —— 原说明：Krea 2 / Z-Image·用途｜**海报与平面设计**，不是人像风格。作者做它是为了利用模型的文字理解力排海报。建议权重 0.8~1.2。⚠️ 拿人像提示词测它只会得到「胶片质感」那种假象 —— 要给它排版类的提示词才看得出本事。出处 Civitai 2326705（purplelady）" },
    en: { text: "Poster / graphic design LoRA, not a portrait style. Built to exploit the model's text rendering for posters. Weight 0.8-1.2. Testing it with a portrait prompt only shows a vague film look. Civitai 2326705" },
  },
  {
    match: (n) => n.toLowerCase().includes("asianmix") && n.toLowerCase().includes("krea2"),
    zh: { text: "Krea 2·脸型｜亚洲女性面部与表情调整。实测：下巴变尖、颧骨收窄、五官更精致，**肤质保持写实不磨皮**。触发词 `asian woman`，**作者推荐强度 0.8**（比默认 1.0 低）。和 Z-Image 版 `asianMix_zimgt_v5` 是同作者同数据集（hinablue）。出处 Civitai 2746042" },
    en: { text: "Krea 2 face shaping — Asian female features and expressions. Measured: narrower jaw and cheekbones, finer features, skin stays photoreal. Trigger `asian woman`; author recommends strength 0.8. Civitai 2746042" },
  },
  {
    match: (n) => n.toLowerCase().includes("kgirl"),
    zh: { text: "Krea 2·脸型｜韩系网红脸。实测：冷白皮、鼻梁挺、眼型细长、半磨皮，像加了 Instagram 滤镜。作者在发布页标了 NSFW，但**实测中性提示词下完全不往那边跑** —— 那个标记说的是能力不是倾向。疑似 Civitai 2777694「AdenKGirl」系列（文件名对不上，只能算同类）" },
    en: { text: "Krea 2 face shaping — Korean influencer look: cool fair skin, defined nose, narrow eyes, Instagram-filter smoothing. Tagged NSFW by its author but does not drift that way on a neutral prompt. Possibly the AdenKGirl family (filename does not match)" },
  },
  {
    match: (n) => n.toLowerCase().includes("krea2-realism") || n.toLowerCase().includes("krea2_realism"),
    zh: { text: "⚠️ **已于 2026-09-17 删除**（用户决定）。实测只是加真实细节（脸更立体、织物纤维、牛仔扣子），和底模本身差别不大，且出处对不上（最接近 Civitai 2781697 / 2929151，文件名都不匹配） —— 原说明：Krea 2·质感｜加真实细节。实测：脸更立体、织物纤维和牛仔布纹清晰、牛仔裤出了扣子和口袋。最接近的出处是 Civitai 2781697「Realism Slider」（**负值推向插画，正值推向写实**）或 2929151（gokaygokay），但**文件名都对不上，只能算同类**" },
    en: { text: "Krea 2 texture — adds realistic detail: more dimensional face, visible fabric fibres and denim weave. Closest match is the Realism Slider (negative pushes to illustration, positive to realism), but filenames do not match" },
  },
  {
    match: (n) => n.toLowerCase().includes("detail_slider") || n.toLowerCase().includes("detailer-krea"),
    zh: { text: "⚠️ **已于 2026-09-17 删除**（用户决定）。20 MB 的滑块型，1.0 细节微增、2.0 时针脚变粗棒针。出处对不上（最接近 Civitai 2729908） —— 原说明：Krea 2·质感（滑块型）｜只有 20 MB。实测 1.0 时细节微增；**2.0 时针脚变成粗棒针、发丝分离度明显提高**。这类是设计成区间调的，1.0 等于中档，别当开关用。疑似 Civitai 2729908「[KREA 2] Detail Slider」（文件名对不上）" },
    en: { text: "Krea 2 texture slider (20 MB). At 1.0 the change is subtle; at 2.0 knit stitches turn chunky and hair strands separate clearly. Designed to be swept, not switched on/off" },
  },
  {
    match: (n) => n.toLowerCase().includes("knp_v2"),
    zh: { text: "Krea 2·质感｜**全网搜不到出处** —— 文件名里两个 `copy_copy` 说明转手重命名过，原始发布页已经丢了。名字里的 `1.5~2` 是作者标的推荐权重区间，**1.0 下几乎看不出变化**。实测 1.75：画面推近、色调转暖、皮肤纹理和雀斑明显。名字带 nsfw，但**推到 1.75 依然完全穿着**，不主动跑偏" },
    en: { text: "Krea 2 texture — no provenance found online; the doubled `copy_copy` in the filename means it was renamed and passed around. The `1.5~2` is the author's recommended weight; at 1.0 almost nothing changes. At 1.75: warmer tone, visible skin texture and freckles. Despite the nsfw tag it does not drift on a neutral prompt" },
  },
  {
    match: (n) => n.toLowerCase().includes("reddudou"),
    zh: { text: "Krea 2·服装｜**中式立领斜襟盘扣红衫**（配黑裙、缠枝花刺绣）。触发词 `reddudou` —— **不写等于没挂**，不带触发词时它只会把场景变成街拍外景。⚠️ 别被名字里的「肚兜」误导，实测出的是长袖立领、完整遮蔽的传统服饰。作者 nickai（ModelScope / MuseAI），rank 32" },
    en: { text: "Krea 2 costume — Chinese stand-collar cross-front red jacket with black skirt. Trigger `reddudou`; without it you just get an outdoor street look. Despite the name it is a fully covering long-sleeved traditional garment. By nickai (ModelScope)" },
  },
  {
    match: (n) => n.toLowerCase().includes("cheongsam"),
    zh: { text: "Krea 2·服装｜**无袖挂脖旗袍**，牡丹晕染印花、盘扣领口。触发词 `cheongsam` —— 不写等于没挂。带触发词时**连脸也会被推得更偶像化**，不只是换衣服。合身贴体但完整遮蔽。作者 nickai（ModelScope / MuseAI），rank 32" },
    en: { text: "Krea 2 costume — sleeveless halter cheongsam with peony print. Trigger `cheongsam`; with it the face also gets pushed more idol-like, not just the outfit. Form-fitting but fully covering. By nickai (ModelScope)" },
  },
  {
    match: (n) => n.toLowerCase().includes("bodyconskirt") || n.includes("包臀裙"),
    zh: { text: "Krea 2·服装｜**职业通勤装** —— 实测出的是黑色及膝铅笔裙 + 米色真丝衬衫 + 黑丝袜 + 高跟鞋 + 手提包，全身构图。触发词 `bdq`，不写等于没挂。⚠️ 别被「包臀裙」这个名字误导成性化用途，实测是 OL 风。作者 nickai（ModelScope / MuseAI），rank 32" },
    en: { text: "Krea 2 costume — office wear: black knee-length pencil skirt, cream silk blouse, tights, heels, handbag, full-body framing. Trigger `bdq`. Despite the name it renders as workwear, not swimwear. By nickai (ModelScope)" },
  },
  {
    match: (n) => n.toLowerCase().includes("dynamiccharactersheet"),
    zh: { text: "Krea 2·版式｜**七分区参考表**（元数据 / 大幅主视图 / 正侧背转身 / 三动作 / 三剪影 / 表情 / 六格细节）。图生图，**必须有源图**，约 7.5 分钟。工具会自动挂，不用手动加。⚠️ 转身那排的脸只有约 110px，**当不了身份参考** —— 要转身参考用 QuadView" },
    en: { text: "Krea 2 layout — seven-zone reference sheet. img2img, needs a source image, ~7.5 min. Added automatically. The turnaround row's faces are only ~110px, too small to use as an identity reference — use QuadView for that" },
  },
  {
    match: (n) => n.toLowerCase().includes("quadview"),
    zh: { text: "Krea 2·版式｜**四视图**（大脸特写 + 正 / 侧 / 背全身）。图生图，必须有源图，约 5 分钟。**特写那格约 486px，是目前所有版式里最大的脸** —— 要做身份参考、分镜首帧、封面就用这张。工具自动挂，触发句写死在代码里" },
    en: { text: "Krea 2 layout — four views: face close-up plus front/side/back full body. img2img, ~5 min. The close-up is ~486px, the largest face any of our layouts produce — use this one for identity reference, storyboard first frames and covers" },
  },
  {
    match: (n) => n.toLowerCase().includes("teenymooddy") || n.toLowerCase().includes("teeny"),
    zh: { text: "Z-Image·表情｜「小情绪」表情微调，针对 20 岁上下的青春期情绪表达。作者 AiMetatron，声明支持 Z-Image Turbo 与 RedZ1.5。**还没出图验证过** —— 架构已核对（读张量键名），效果未看" },
    en: { text: "Z-Image expression LoRA — subtle moody expressions for young adults. By AiMetatron. Architecture verified from tensor keys; output not yet reviewed" },
  },
  {
    match: (n) => n.toLowerCase().includes("minimax") || n.toLowerCase().includes("_h3_"),
    zh: {
      text: "⚠️ MiniMax H3 视频加速 LoRA —— 不是出图用的，挂在设定卡上不生效也不报错",
      incompatible: true,
    },
    en: {
      text: "⚠️ MiniMax H3 video LoRA — not for stills; on a character sheet it silently does nothing",
      incompatible: true,
    },
  },
  {
    match: (n) => n.toLowerCase().includes("ltx") || n.toLowerCase().includes("wan"),
    zh: { text: "⚠️ 视频模型的 LoRA —— 不适合出图", incompatible: true },
    en: { text: "⚠️ A video model's LoRA — not for still images", incompatible: true },
  },
];

/** 查一个 LoRA 的说明。没收录的返回 `null`，界面自己不渲染那一行。 */
export function loraNote(name: string, zh: boolean): LoraNote | null {
  const hit = NOTES.find((n) => n.match(name));
  if (!hit) return null;
  return zh ? hit.zh : hit.en;
}

/** 不适合出图的排到最后 —— 不隐藏（藏起来用户会以为丢了），但别挡路。 */
export function sortLorasForSheet(names: string[], zh: boolean): string[] {
  return [...names].sort((a, b) => {
    const ia = loraNote(a, zh)?.incompatible ? 1 : 0;
    const ib = loraNote(b, zh)?.incompatible ? 1 : 0;
    return ia - ib;
  });
}

/**
 * checkpoint 的用途说明。和 LoRA 同一个道理 —— 列表里只有文件名，
 * 而这两个底模的差别（欧美写实 vs 国风古装）恰恰是最影响出图的一项。
 */
const CKPT_NOTES: { match: (n: string) => boolean; zh: string; en: string }[] = [
  {
    match: (n) => n.toLowerCase().includes("realvis"),
    zh: "欧美写实 —— 皮肤质感和毛孔最真实，但脸偏欧美骨相，做国风古装不如 AWPortrait",
    en: "Western photoreal — the most convincing skin texture, but Western facial structure",
  },
  {
    match: (n) => n.toLowerCase().includes("awportrait"),
    zh: "国内社区人像系列 —— 亚洲面容和古装结构更准，做国风设定卡首选；皮肤偏修图感",
    en: "Chinese-community portrait series — better Asian faces and hanfu structure; more retouched skin",
  },
  {
    match: (n) => n.toLowerCase().includes("cyberrealistic"),
    zh: "写实向 SDXL 微调。出处 Civitai 443821（Cyberdelia）。注意 `cyberrealisticPony_*` 那个版本 **2026-09-22 已随 Pony 一起删掉**",
    en: "Photoreal SDXL finetune. Civitai 443821. Note: the cyberrealisticPony variant was deleted together with Pony on 2026-09-22",
  },
  {
    match: (n) => n.toLowerCase().includes("xxmix") || n.toLowerCase().includes("majic") || n.toLowerCase().includes("chillout"),
    zh: "⚠️ SD 1.5 架构 —— 这台机器上超过 640px 会塌成纯色，不要用",
    en: "⚠️ SD 1.5 — collapses to flat colour above 640px on this machine; do not use",
  },
];

export function checkpointNote(name: string, zh: boolean): string | null {
  const hit = CKPT_NOTES.find((n) => n.match(name));
  return hit ? (zh ? hit.zh : hit.en) : null;
}

/**
 * 这条 LoRA 是 Z-Image 架构的吗。
 *
 * **两个下拉必须各自只列自己架构的。** 混着列的话，把 SDXL 的挂进 Z-Image 常挂、
 * 或者反过来，都是**一声不吭什么都不做** —— 用户以为风格生效了，实际没有。
 * 这正是当初否掉「全局常挂」时最担心的那一条。
 *
 * 判据和 Rust 侧 `image_engine::lora_arch` 保持一致：认不出的一律当 SDXL。
 */
export function isZImageLora(name: string): boolean {
  const n = name.toLowerCase();
  return ["zimgt", "_zit", "zimage", "z_image", "z-image"].some((m) => n.includes(m));
}

/**
 * 当前选的出图底模下，设置里该突出显示哪一块。
 *
 * 三块全列出来太吵（用户原话「全列出来不合适」），但**不能直接藏掉 SDXL 那两块**：
 * 角色设定卡、控姿势、SDXL 架构的 LoRA 都会强制回落 SDXL，**和选了什么底模无关** ——
 * 藏掉就等于这些设置再也配不到了。
 *
 * 所以：跟当前底模有关的那块展开，另一块收进折叠区，可达但不占地方。
 */
export function loraSectionsFor(engine: string): {
  /** 展开 Z-Image 常挂 LoRA */
  zimage: boolean;
  /** SDXL 底模与设定卡 LoRA 作为主区展开（否则收进折叠） */
  sdxlPrimary: boolean;
} {
  return {
    zimage: engine === "zimage",
    sdxlPrimary: engine === "sdxl",
  };
}
