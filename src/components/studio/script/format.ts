/**
 * 剧本行的类型识别。
 *
 * # 为什么这块要单独抽出来
 *
 * 用户是**粘小说和剧本进来的**，不会手敲格式。所以「这一行是什么」
 * 必须靠猜 —— 而猜错了不报错，只表现为排版乱、后面抽实体也跟着乱。
 * 抽成纯函数是为了能逐条测。
 *
 * 规则改自 LumenX 的 `usePasteHandler`（MIT，Copyright 2026 Alibaba），
 * 中文那几条按我们的实际语料加过：中文剧本常写「【场景：老茶馆】」
 * 和「老茶师：这茶，要等。」，它那份只认「内景.」和西文格式。
 */

/** 剧本里的一行是什么。和 TipTap 的节点名一一对应。 */
export type LineType =
  /** 场景标题：`INT. 办公室 - 日` / `【场景：老茶馆】` */
  | "sceneHeading"
  | "metadata"
  | "caption"
  /** 动作/描写。**兜底类型** —— 认不出的都算它 */
  | "action"
  /** 角色名（对白的说话人） */
  | "characterCue"
  /** 对白 */
  | "dialogue"
  /** 括号提示：`（低声）` */
  | "parenthetical"
  /** 转场：`CUT TO:` / `切至：` */
  | "transition";

export interface ParsedLine {
  type: LineType;
  text: string;
  /** `characterCue` 时是角色名；由「角色：台词」一行拆出来时才有 */
  character?: string;
}

/* ── 场景标题 ─────────────────────────────────────────────────────────────
   三种写法都认：
     INT. 办公室 - 日        西文
     内景. 办公室 - 日        中文直译
     【场景：老茶馆】         中文剧本常见（我们自己的语料就是这个）
   ──────────────────────────────────────────────────────────────────────── */
const SCENE_WESTERN = /^\s*(INT|EXT|INT\.?\/EXT|I\/E)[.．\s]/i;
const SCENE_CN_PREFIX = /^\s*(内景|外景|内外景|内\/外)[.．\s]/;
const SCENE_CN_BRACKET = /^\s*【\s*(场景|地点|场)\s*[:：]/;
/** 日式剧本用 `○` 开头。 */
const SCENE_JP = /^\s*○/;

/* ── 转场 ─────────────────────────────────────────────────────────────── */
const TRANSITION = /^\s*(CUT TO|FADE (IN|OUT|TO)|DISSOLVE TO|SMASH CUT|切至|淡入|淡出|溶解|转场)\s*[:：]?\s*$/i;

/* ── 括号提示 ─────────────────────────────────────────────────────────── */
const PARENTHETICAL = /^\s*[（(][^)）]{0,30}[)）]\s*$/;

/**
 * 「角色：台词」。
 *
 * **必须限长**。不限的话，「他看着窗外，心想：这茶要等」整句会被当成
 * 角色名叫「他看着窗外，心想」的一句台词 —— 于是资产库里凭空多出
 * 一个这样的「角色」，而且不报错。
 *
 * 12 个字是按中文人名（含括号造型，如「张三（婚纱）」）留的余量。
 */
const CHARACTER_LINE = /^\s*([^\s:：（(]{1,12}(?:\s+(?:OS|VO|O\.S\.|V\.O\.))?(?:[（(][^)）]{1,60}[)）])?)\s*[:：]\s*(.+)$/i;

export function speakerInfo(cue: string) {
  const delivery = /[（(]([^）)]+)[）)]/.exec(cue)?.[1] ?? "";
  const name = cue.replace(/[（(][^）)]*[）)]/g, "").replace(/\s+(?:OS|VO|O\.S\.|V\.O\.)$/i, "").trim();
  const audioType = /录音/.test(delivery) ? "recording" : /(?:\sOS\b|O\.S\.|内心|独白)/i.test(cue) ? "inner" : /(?:\sVO\b|V\.O\.|画外)/i.test(cue) ? "voiceover" : name === "旁白" ? "narration" : "dialogue";
  return { name, delivery, audioType };
}

export function numberedHeading(text: string) {
  return /^\s*\d+[-.－．]\d+\s+(.+?)\s*(?:(日|夜|深夜|凌晨|黄昏|清晨|白天|傍晚)\s+(内|外)|(内|外)\s+(日|夜|深夜|凌晨|黄昏|清晨|白天|傍晚))\s*$/.exec(text);
}

/**
 * 整行包在 `【】` 里的是**标注**，不是对白。
 *
 * 中文剧本里 `【时间：黄昏】`、`【人物：林辰、苏沐雪】` 很常见。
 * 不挡的话，「角色：台词」那条规则会把它读成一个叫「【时间」的角色 ——
 * 于是资产库里凭空多出这么个「人」，而且不报错。
 * （场景标题那条在前面已经先命中了，所以这里只剩其余标注。）
 *
 * **只在这里挡一次。** 早先「角色名的字符类里也排除 `【】`」是第二道，
 * 结果两道互相兜底，拆掉任何一道测试都不变红 —— 等于两道都没被测到。
 */
const ANNOTATION = /^\s*【[\s\S]*】\s*$/;

/**
 * 冒号前面是这些词的，是**字段名**，不是人。
 *
 * 用户粘进来的常常是漫剧企划案，开头一片
 * 「类型： 玄幻、宿命爱情」「时间线： 双帝之战结束五年后」——
 * 「角色：台词」那条规则会把它们读成两个叫「类型」和「时间线」的角色，
 * 然后一路带进资产库。实测就是这么发生的。
 *
 * **这是一份挡词表，不是规则**，遇到新的字段名还得往里加。试过更"聪明"的
 * 判据（值里带顿号、没有句末标点……），但「由美子：我是」也长这样，
 * 分不干净 —— 宁可漏挡，不能误挡真台词。
 *
 * **只有这一份。** 早先 `classifyLine` 和 `parseScript` 各写了一份一样的，
 * 两边迟早漂。而且要在**每一个**「认定这是人名」的入口上都过一遍 ——
 * 少一个入口就漏一条路：单独成行的 `类型` 走的是 `CHARACTER_ONLY`，
 * 不是「角色：台词」，第一版就漏在这儿。
 */
/**
 * 元信息标签。
 *
 * **后缀是可选的。** 第一版只列了裸词，于是 `时长建议` 漏了 ——
 * 裸词「时长」在表里，但 `^时长$` 匹配不上「时长建议」，
 * 那一行就变成了一个叫「时长建议」的角色（2026-09-12 实拍）。
 * 模板换个写法就换一套后缀，所以后缀单独一组，不要往主表里塞。
 *
 * **这份词表是这条判据最弱的一环**，心里有数：它只挡得住见过的写法。
 *
 * 但它现在**不再影响抽实体**了：2026-09-12 起抽实体只听模型，
 * 段落类型只管排版和场次目录。漏一个新写法的后果从「资产库里多一个
 * 假角色」降成「这一行排版不对」。
 */
const META_FIELD =
  /^(时间|时间线|地点|人物|场景|镜头|画面|类型|类别|题材|形式|风格|基调|篇幅|集数|时长|字数|受众|标签|主题|简介|梗概|背景|设定|世界观|结构|核心|卖点|目标|备注|说明|logline|Logline)(建议|说明|参考|要求|范围|方向|定位)?$/;

/**
 * 剥掉 Markdown 的强调记号。
 *
 * **元信息判定必须在剥完之后做。** `META_FIELD` 认得「类型」，
 * 但实际拿到的是 `**类型**` —— `^类型$` 匹配不上，于是放行，
 * 然后 `**类型**` 变成一个角色、进了资产库、出了设定卡。
 * 用户 2026-09-12 的截图里就是三个这样的假角色。
 */
export function stripEmphasis(s: string): string {
  return s.replace(/[*_`~]/g, "").trim();
}

/**
 * 这个名字能不能当角色。**认定人名的每个入口都要过这里。**
 *
 * 判两件事：剥掉强调记号之后是不是元信息标签，以及剥完还剩不剩东西
 * （`****` 剥完是空的 —— 那不是名字）。
 */
export function isCharacterName(name: string): boolean {
  const bare = stripEmphasis(name);
  return bare.length > 0 && !META_FIELD.test(bare);
}

/** 纯粹的角色名行（下一行才是台词）——全大写英文，或短且无标点的中文。 */
const CHARACTER_ONLY = /^\s*([A-Z][A-Z\s.'-]{1,20}|[一-龥]{1,6})(?:\s+(?:OS|VO|O\.S\.|V\.O\.))?(?:[（(][^)）]{1,60}[)）])?\s*$/i;

const SENTENCE_END = /[。！？!?…]\s*$/;

/**
 * 认一行是什么。
 *
 * `prev` 是上一行的类型 —— 判断「角色名行」必须看上下文：
 * 一个短句单独成行，只有在**不是**紧跟在对白后面时才可能是角色名。
 */
export function classifyLine(raw: string, prev: LineType | null): ParsedLine {
  const text = raw.trim();
  if (!text) return { type: "action", text: "" };
  if (/^#{1,6}\s|^[-*_]{3,}$|^第[一二三四五六七八九十百\d]+集(?:\s|$)/.test(text)) return { type: "metadata", text };
  if (/^字幕\s*[:：]/.test(text)) return { type: "caption", text };

  if (
    numberedHeading(text) || SCENE_WESTERN.test(text) ||
    SCENE_CN_PREFIX.test(text) ||
    SCENE_CN_BRACKET.test(text) ||
    SCENE_JP.test(text)
  ) {
    return { type: "sceneHeading", text };
  }
  if (TRANSITION.test(text)) return { type: "transition", text };
  // 标注要在「角色：台词」之前挡掉
  if (ANNOTATION.test(text)) return { type: "action", text };
  if (PARENTHETICAL.test(text)) return { type: "parenthetical", text };

  const m = CHARACTER_LINE.exec(text);
  if (m) {
    const [, name, line] = m;
    // 「时间：黄昏」这类元信息不是对白 —— 冒号后面没有句子的语气时更像标注，
    // 但这条很难分干净，所以只挡掉明显的：名字里带「时间/地点/人物」。
    if (isCharacterName(name)) {
      // **存剥干净的名字。** 留着星号的话，资产库里是「**林婉清**」，
      // 而分镜里的 `[角色:林婉清]` 对不上它 —— 对不上时提示词里
      // 只剩名字，长相根本不会带上。
      return { type: "characterCue", text: stripEmphasis(name), character: line };
    }
  }

  // 单独一行的短名字：上一行不是对白时，多半是角色名。
  //
  // **先剥强调记号再判。** `CHARACTER_ONLY` 的字符类里没有 `*`，
  // 所以 `**皇帝**` 直接落到「动作」里 —— 一个说话人就这么丢了。
  // 剥完再判是安全的：剥出来是元信息标签的，下一句 `isCharacterName`
  // 照样挡得住。
  const bare = stripEmphasis(text);
  if (
    prev !== "dialogue" &&
    CHARACTER_ONLY.test(bare) &&
    !SENTENCE_END.test(bare) &&
    isCharacterName(bare)
  ) {
    return { type: "characterCue", text: bare };
  }

  // 紧跟在角色名后面的一行是对白
  if (prev === "characterCue" || prev === "parenthetical") {
    return { type: "dialogue", text };
  }

  return { type: "action", text };
}

/**
 * 整段文本按行识别。空行丢掉 —— 段落靠节点本身分隔。
 *
 * # 一处真实的歧义
 *
 * 紧跟在对白后面的一个短行，既可能是**下一句台词**（「我知道了」），
 * 也可能是**新角色名**（「小徒弟」）。单看这一行分不出来，所以用两个
 * 上下文信号补：
 *
 * 1. **下一行是括号提示** → 一定是角色名（`（急切地）` 只会跟在角色名后面）
 * 2. **这个名字在别处以角色身份出现过** → 也是角色名
 *
 * 两个都不满足时保守地当台词延续 —— 猜错成角色名的代价更大：
 * 资产库里会凭空多出一个「我知道了」。
 */
export function parseScript(raw: string): ParsedLine[] {
  const raws = raw.split(/\r?\n/);
  // 先扫一遍，收集冒号前面出现过的名字（「角色：台词」这种形式）。
  //
  // **这里不过挡词表。** 下面唯一用到 `known` 的地方已经过了一道
  // `isCharacterName`，这里再挡一次就是两道互相兜底 —— 拆掉任何一道
  // 测试都不变红，等于两道都没被测到。（`ANNOTATION` 上面踩过同一个坑。）
  const known = new Set<string>();
  for (const line of raws) {
    const m = CHARACTER_LINE.exec(line.trim());
    if (m && !ANNOTATION.test(line.trim())) {
      known.add(m[1]);
    }
  }

  const out: ParsedLine[] = [];
  let prev: LineType | null = null;
  let frontmatter = raws[0]?.trim() === "---";
  for (let i = 0; i < raws.length; i++) {
    const line = raws[i];
    if (frontmatter) {
      out.push({ type: "metadata", text: line });
      if (i > 0 && line.trim() === "---") frontmatter = false;
      continue;
    }
    if (!line.trim()) {
      // 空行断开上下文，免得跨段把描写当成对白
      prev = null;
      continue;
    }
    let parsed = classifyLine(line, prev);

    // 保守规则把它判成了动作/对白，但上下文说它是角色名
    if (
      parsed.type !== "characterCue" &&
      CHARACTER_ONLY.test(line.trim()) &&
      isCharacterName(line.trim())
    ) {
      const text = line.trim();
      const next = raws.slice(i + 1).find((l) => l.trim());
      if (known.has(text) || (next && PARENTHETICAL.test(next.trim()))) {
        parsed = { type: "characterCue", text };
      }
    }

    out.push(parsed);
    // 「角色：台词」一行拆成两个节点
    if (parsed.type === "characterCue" && parsed.character) {
      out.push({ type: "dialogue", text: parsed.character });
      prev = "dialogue";
    } else {
      prev = parsed.type;
    }
  }
  return out;
}

/**
 * 识别率：**认出来的行占非空行的比例**。
 *
 * 用来决定粘贴时要不要提示「帮你排版」——全是动作（一条规则都没命中）
 * 时不要弹，那多半是粘了一段散文。
 */
export function matchRate(lines: ParsedLine[]): number {
  if (lines.length === 0) return 0;
  const hit = lines.filter((l) => l.type !== "action").length;
  return hit / lines.length;
}
