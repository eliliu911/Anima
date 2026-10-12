import type { SceneContract } from "../../../api/assets";
import { promptText } from "../../../shared/shotLanguage";

/**
 * 场景多方位宫格的方位表。
 *
 * # 方位是**词表**，不是自由文本
 *
 * 一开始这里是两个输入框让人自己写。2026-09-11 端到端实跑两轮否掉了
 * 那个做法 —— 四个手写方位实际只落在**两个机位**上：
 *
 * | 方位 | 写的描述 | 结果 |
 * |---|---|---|
 * | 入口正打 | 从卷帘门内侧朝店堂深处看，尽头是柜台 | ✅ |
 * | 柜台反打 | 从柜台内侧朝卷帘门看 | ❌ 两轮都没转过去 |
 * | 客桌侧看 | 从客桌之间朝柜台看 | ⚠️ 和上一格重了 |
 *
 * 问题不在「手写」本身，在**没有下限**：用户不知道哪种写法有效。
 * 而这个项目已经在镜头语言那边得出过同一个结论 ——
 * 「存的是词表 id，发的是经过实测的提示词；存显示串的老毛病是模型
 * 只收到『推镜头』四个字」。方位是同一件事。
 *
 * # 视角是**相机指令**，不是方位名
 *
 * 这是看了 liblib 的提示词才想明白的。同一格，他们写的是
 * 「**镜头在室内最深处向外拍摄**」，我们写的是「反打」——
 * 名字模型不当回事，动作它照做。我们那两轮没转过去，转不过去的是
 * 「反打」这两个字，不是模型。
 *
 * 侧面机位还要**锚在正面那一格的中心线上**（「以 1 的中心线为参考，
 * 移到左前 45 度」）。各自独立地说「左侧看」「右侧看」，两格会各转各的。
 *
 * # 为什么四视角就够，不用上 3D
 *
 * 用户定的：「3DGS 只有在需要非常细致定位下才需要，正常漫剧只要把
 * 相机镜头指令做好即可，而且我们以前的场景只有**单视角**，
 * 因此一致性问题严重，改成这种 4 视角，至少不会让场景和人物乱飘。」
 *
 * 一个场景只有一张图时，第二镜换个角度只能靠模型现编 —— 那才是乱飘的
 * 来源。给四个同源方位，飘的空间一下就小了，而且这一步便宜得多。
 *
 * # 每条词还带实测可靠度
 *
 * 「反打」实测两次都没真的转过去。**这件事要写在选项旁边**，
 * 不能让人选完等着失望 —— 和镜头语言词表里那些 `note`
 * （「大远景里脸的像素预算约等于零」）是同一个道理。
 *
 * # 这张表同时就是「空间契约」
 *
 * 选的每一条会存回 `asset.scene.views`。所以这不是一个临时的出图参数 ——
 * 镜头上下文节点、全景出图节点读的都是它。
 * 「我在哪儿定义场景」的答案就是这里。
 */

/** 一条方位实测有多可靠。**照实标**，不好用的也要列出来。 */
export type ViewReliability = "ok" | "partial" | "poor";

export interface ViewTerm {
  id: string;
  name: string;
  /**
   * 这一格**是一次机位**，不是一个方位。
   *
   * 词表 id 取自 `shared/shot-language.json` —— **和分镜用的是同一份**。
   * 场景四视角和分镜镜头本来就是同一件事：都是「摄影机放在哪、
   * 用什么焦段、什么景别」。分开维护两套的表现是同一个词在两边
   * 含义不一样，而那不报错。
   */
  shot: { shotSize: string; angle: string; lens?: string; depth?: string };
  /**
   * 词表管不到的那一半：**相机往哪站、朝哪拍**。
   *
   * `{锚点}` / `{入口}` 从契约里填。写成**可执行的相机动作**，
   * 不是方位名 —— 「镜头站在最深处向外拍」模型照做，「反打」它不当回事。
   */
  placement: string;
  reliability: ViewReliability;
  /** 实测注记。有就显示在选项旁边。 */
  note?: string;
}

export const VIEW_TERMS: ViewTerm[] = [
  {
    id: "center",
    name: "正面中心线",
    shot: { shotSize: "extreme_wide", angle: "eye_level", lens: "f24", depth: "deep" },
    placement:
      "镜头正对场景中心轴，构图严格居中对称，画面同时包含顶面与底面；视线尽头是{锚点}",
    reliability: "ok",
  },
  {
    id: "left45",
    name: "左前 45°",
    shot: { shotSize: "extreme_wide", angle: "eye_level", lens: "f24", depth: "deep" },
    placement:
      "**以第 1 格的中心线为参考**，摄像机移动到场景**左前方 45 度**，" +
      "画面主体换成**{锚点}**那一侧的墙面和陈设，空间连贯、比例一致；" +
      "**画面里看不到{主锚点}**",
    reliability: "ok",
  },
  {
    id: "right45",
    name: "右前 45°",
    shot: { shotSize: "extreme_wide", angle: "eye_level", lens: "f24", depth: "deep" },
    placement:
      "**以第 1 格的中心线为参考**，摄像机移动到场景**右前方 45 度**，" +
      "画面主体换成**{锚点}**那一侧的墙面和陈设，空间连贯、比例一致；" +
      "**画面里看不到{主锚点}**",
    reliability: "partial",
    note: "和「左前 45°」互为镜像时，模型有时只画出其中一种",
  },
  {
    id: "deep_out",
    name: "深处往外",
    shot: { shotSize: "extreme_wide", angle: "eye_level", lens: "f24", depth: "deep" },
    placement:
      "**镜头站在室内最深处向外拍摄**，画面主体是**{入口}和门外透进来的天光**，" +
      "以及入口两侧的墙；**画面里看不到{主锚点}**（镜头就站在它那儿）。" +
      "这一格的进光方向和前几格相反 —— 前几格朝暗处看，这一格朝亮处看",
    reliability: "partial",
    note: "这就是反打。写成「站到最深处往外拍」这种**相机动作**才转得过去 —— 写「反打」两个字不行（实测两轮都没转）；空间有一条两端都认得出的轴时最稳",
  },
  {
    id: "entry_in",
    name: "入口往里",
    shot: { shotSize: "wide", angle: "eye_level", lens: "f35", depth: "deep" },
    placement: "镜头站在{入口}内侧朝空间深处拍，视线尽头是{锚点}，前景是脚下的地面",
    reliability: "ok",
  },
  {
    id: "high",
    name: "高机位",
    shot: { shotSize: "wide", angle: "high_angle", lens: "f24", depth: "deep" },
    placement: "镜头升到接近天花板的高度俯看（**不是极端俯视**），地面分区和{锚点}的位置一览无余",
    reliability: "partial",
    note: "俯角实测容易退化成普通平视，只是稍微高一点",
  },
  {
    id: "low",
    name: "低机位",
    shot: { shotSize: "wide", angle: "low_angle", lens: "f24", depth: "deep" },
    placement: "镜头贴近地面仰看（**不是极端仰视**），{锚点}显得高大，天花板进入画面",
    reliability: "ok",
  },
  {
    id: "work_area",
    name: "活动区中景",
    shot: { shotSize: "medium", angle: "eye_level", lens: "f50", depth: "deep" },
    placement:
      "镜头站在{入口}和{锚点}之间，中景拍{锚点}周围那一块真正发生事情的区域，" +
      "看得出人在这儿会站在哪、怎么走动",
    reliability: "ok",
  },
  {
    id: "detail",
    name: "材质近景",
    shot: { shotSize: "close_up", angle: "eye_level", lens: "f85", depth: "shallow" },
    placement: "镜头贴近{锚点}拍它的表面，看得清材质、磨损和它周围一小圈",
    reliability: "ok",
  },
];

export function termById(id: string): ViewTerm | undefined {
  return VIEW_TERMS.find((t) => t.id === id);
}

/**
 * 宫格只有这三档。
 *
 * **和 Rust 侧 `scene_sheet::grid_for` 对得上**：4 → 2×2、6 → 2×3、
 * 9 → 3×3。不给 3、5、7 这些数是因为它们会留空格，而空格子模型多半
 * 会自己找东西填满 —— 那一格就是废的，还占了一次采样的像素预算。
 */
export const GRID_CHOICES = [4, 6, 9] as const;
export type GridChoice = (typeof GRID_CHOICES)[number];

/** 一个格子里选了哪条方位。空串 = 这一格还没选。 */
export interface ViewRow {
  termId: string;
}

/** 换到这个档位时的格子数组，尽量保留已选的。 */
export function resizeSlots(rows: ViewRow[], size: GridChoice): ViewRow[] {
  const out = rows.slice(0, size);
  while (out.length < size) {
    // 补默认：按词表顺序挑一个还没用过的（前面几条最靠得住）
    const used = new Set(out.map((r) => r.termId));
    const next = VIEW_TERMS.find((t) => !used.has(t.id));
    out.push({ termId: next?.id ?? "" });
  }
  return out;
}

/**
 * 契约里的锚点，**按「适不适合当机位的目标」排**。第一个是主锚点。
 *
 * # 这套排序和地理锁那套故意不一样
 *
 * `SceneContract::lock_line`（Rust 侧）把 `wall_dominant` 排最前 ——
 * 占满一面墙的东西是**最强的地理参照**，那是对的。
 *
 * 但机位要的是**焦点**，不是背景。「正面中心线」那一格对着一整面墙
 * 拍出来是一堵墙；而 liblib 的公式写得明白：「格 1：正对**核心焦点**
 * 的中心轴正视图」。一个离散的大件（龙椅、柜台、祭坛）才是焦点。
 *
 * 所以这里 `large` 排在 `wall_dominant` 前面。实测撞到过：
 * 第 1 格去看「两侧的朱红巨柱与雕花窗棂」而不是龙椅。
 */
export function rankedAnchors(c: SceneContract | null | undefined): string[] {
  const weight = { large: 3, wall_dominant: 2, medium: 1, small: 0 } as const;
  return [...(c?.anchors ?? [])]
    .filter((a) => a.name.trim())
    .sort((a, b) => weight[b.size] - weight[a.size])
    .map((a) => a.name.trim());
}

/**
 * 把模板填成真正要发出去的一句话。
 *
 * `slot` 是这一格排第几（从 0 起）。**它决定这一格点名哪个锚点** ——
 * 见下面那段。
 */
export function describeTerm(
  term: ViewTerm,
  c: SceneContract | null | undefined,
  slot = 0,
): string {
  const ranked = rankedAnchors(c);
  const main = ranked[0] ?? "最显眼的那个不可移动物";
  // **每一格点名一个不同的锚点 —— 这是四格不同的决定性因素。**
  //
  // 2026-09-12 实测：四格出成同一个正面机位，查到最后原因在这儿 ——
  // 原来每一格填的都是**同一个**锚点（尺度最大的那个），于是四格都在说
  // 「镜头对准龙椅」。模型画四张龙椅正面**是在照做**。
  //
  // 排除过三个别的怀疑，都不是：
  // · 提示词里有「single unified image」和宫格打架 —— 修了，还是四格一样
  // · 四格描述开头那串镜头语言完全相同 —— 提到公共段说一次，还是一样
  // · Z-Image 服从度不够 —— 换 Krea 2（实测服从度最强）**更差**，
  //   四格几乎像素级重复、连分隔白线都没了
  //
  // 换成每格点名不同地标之后，一次就出了正打 / 侧墙 / 反打门外天光 /
  // 另一侧墙四个真机位。
  //
  // 锚点不够四个时循环取 —— 循环也比四格全填同一个强。
  const anchor = ranked.length ? ranked[slot % ranked.length] : main;
  const entrance = c?.entrances.find((e) => e.name.trim())?.name.trim() ?? "主入口";
  const placement = term.placement
    .replace(/\{锚点\}/g, anchor)
    .replace(/\{主锚点\}/g, main)
    .replace(/\{入口\}/g, entrance);
  // **镜头语言在前，机位在后。**
  // 前半段是「这是一个什么镜头」（景别/机位/焦段/景深），
  // 后半段是「相机站在哪」—— 导演就是这么说话的。
  const lang = [
    promptText("shotSize", term.shot.shotSize, ENGINE_FOR_SCENE),
    promptText("angle", term.shot.angle, ENGINE_FOR_SCENE),
    term.shot.lens ? promptText("lens", term.shot.lens, ENGINE_FOR_SCENE) : "",
    term.shot.depth ? promptText("depth", term.shot.depth, ENGINE_FOR_SCENE) : "",
  ]
    .map((x) => x.trim())
    .filter(Boolean)
    .join("，");
  return lang ? `${lang}。${placement}` : placement;
}

/**
 * 拼镜头语言时按哪个引擎取中英。
 *
 * 场景宫格默认走 Z-Image（中文原生），而这一段最后是拼进中文提示词的 ——
 * 中英混着发会让版式那一段的英文措辞失去优势。
 */
const ENGINE_FOR_SCENE = "zimage";

/** 契约里有没有填过锚点 —— 没有的话模板只能退到通用说法。 */
export function hasAnchors(c: SceneContract | null | undefined): boolean {
  return (c?.anchors ?? []).some((a) => a.name.trim());
}

/** 宫格的行列。**和 Rust 侧 `scene_sheet::grid_for` 必须一致。** */
export function gridFor(count: number): { rows: number; cols: number } {
  if (count <= 1) return { rows: 1, cols: 1 };
  if (count === 2) return { rows: 1, cols: 2 };
  if (count <= 4) return { rows: 2, cols: 2 };
  if (count <= 6) return { rows: 2, cols: 3 };
  return { rows: 3, cols: 3 };
}

/**
 * 「2×3」这种给人看的写法。**行×列**，和用户说的顺序一致
 * （「2x2(4 方位)，2x3(6 方位)，3x3(9 方位)」）。
 *
 * 只是显示 —— 真正的版式在 [`gridFor`]，它和 Rust 侧一一对应。
 */
export function gridLabel(count: number): string {
  const g = gridFor(Math.min(count, 9));
  return `${g.rows}×${g.cols}`;
}

/** 本机一次出 1536×1024，切成这个版式之后每格多大。 */
export function cellSize(count: number, width = 1536, height = 1024): { w: number; h: number } {
  const g = gridFor(Math.min(count, 9));
  return { w: Math.floor(width / g.cols), h: Math.floor(height / g.rows) };
}

/**
 * 每格**短边**低于这个数就只能当布局稿。
 *
 * **和 Rust 侧 `scene_sheet::PixelBudget::usable_as_final` 一致：
 * 判据是短边，不是宽度。** 本机 3×3 出来每格 512×341 —— 宽度看着够，
 * 短边只有 341。按宽度判的话这一格会被当成「可直接使用」，
 * 而它其实是布局稿：构图、景别、光线走向可信，材质和细节不可信。
 *
 * 不说的话这张布局稿会被当成品用下去 —— 角色设定卡那次就是没人算这笔账
 * （整排图的脸只有 40~46px）。
 */
export const LAYOUT_ONLY_BELOW = 512;

/** 一格的短边。判「够不够当成品」看的是它。 */
export function shortSide(count: number, width = 1536, height = 1024): number {
  const { w, h } = cellSize(count, width, height);
  return Math.min(w, h);
}

export function viewsFromContract(c: SceneContract | null | undefined): ViewRow[] {
  // 契约里存的是填好的整句，反查回是哪条词。
  //
  // **认不出来的要补成默认词条，不能留空格子。**
  //
  // 注释一直是这么写的，代码却给的是空串 —— 而空串在界面上就是
  // 「（这一格空着）」，而且「空格子模型多半会自己找东西填满，
  // 那一格是废的」。2026-09-12 实拍：从剧本草拟出来的契约自带一套
  // 模型现起名字的 `views`，一个都对不上，四个格子里三个变成空的。
  const rows: ViewRow[] = (c?.views ?? [])
    .filter((v) => v.name.trim())
    .map((v) => ({ termId: VIEW_TERMS.find((t) => t.name === v.name.trim())?.id ?? "" }));
  const size: GridChoice =
    rows.length >= 9 ? 9 : rows.length >= 6 ? 6 : 4;
  return centerFirst(fillHoles(resizeSlots(rows, size)));
}

/**
 * 「正面中心线」永远排第一格。
 *
 * # 这不是排版偏好，是依赖关系
 *
 * 左前 / 右前 45° 的原文是「**以「正面中心线」那一格的中心线为参考**，
 * 摄像机移动到场景左前方 45 度」—— 它们**按名字引用另一格**。
 * 那一格排在后面的话，模型读到第 1 格时参考还不存在。
 *
 * liblib 那份原文的顺序是有依赖的：「1 为正面中心线大全景视图，
 * **2 以 1 的中心线为参考**……」。我们把这个关系写进了词条正文，
 * 却没有保证顺序。
 *
 * 2026-09-12 实拍：格子顺序是 左前/右前/深处/正面 —— 四格出来
 * 几乎一模一样。
 *
 * 没选中心线那一格时不动顺序 —— 那说明用户是故意不要参考基准的。
 */
export function centerFirst(rows: ViewRow[]): ViewRow[] {
  const at = rows.findIndex((r) => r.termId === "center");
  if (at <= 0) return rows;
  const out = rows.slice();
  const [center] = out.splice(at, 1);
  out.unshift(center);
  return out;
}

/**
 * 把空格子补成还没用过的默认词条。
 *
 * `resizeSlots` 只补**末尾**缺的那几格；中间认不出来的那些是空串，
 * 它一个都不管。两件事分开做，因为补末尾是「格子数不够」，
 * 补空洞是「这一格的内容认不出来」。
 */
export function fillHoles(rows: ViewRow[]): ViewRow[] {
  const used = new Set(rows.map((r) => r.termId).filter(Boolean));
  return rows.map((r) => {
    if (r.termId) return r;
    const next = VIEW_TERMS.find((t) => !used.has(t.id));
    if (next) used.add(next.id);
    return { termId: next?.id ?? "" };
  });
}

/**
 * 把选好的方位写回契约，**其余字段一个不动**。
 *
 * 存的是**填好的整句**（`toward`），不是模板 —— 下游（镜头上下文节点、
 * 全景出图）读的是文字，不该再各自去填一遍模板，填法不一致就会分叉。
 */
export function contractWithViews(
  base: SceneContract | null | undefined,
  rows: ViewRow[],
): SceneContract {
  const empty: SceneContract = {
    shape: "",
    zones: [],
    entrances: [],
    anchors: [],
    relations: [],
    materials: [],
    views: [],
    inferred: [],
    no_extra: [],
    empty_stage: "required",
  };
  const c = base ? { ...base } : empty;
  c.views = rows
    .map((r) => termById(r.termId))
    .filter((t): t is ViewTerm => Boolean(t))
    .map((t, i) => ({
      name: t.name,
      from: "",
      toward: describeTerm(t, base, i),
      far: "",
      visible: [],
      light: "",
    }));
  return c;
}

/** 送进 `studioGenerate` 的方位表。 */
export function toViewpoints(
  rows: ViewRow[],
  c: SceneContract | null | undefined,
): { name: string; description: string }[] {
  return rows
    .map((r) => termById(r.termId))
    .filter((t): t is ViewTerm => Boolean(t))
    .map((t, i) => ({ name: t.name, description: describeTerm(t, c, i) }));
}

/**
 * 填得够不够用。**返回的是给人看的话，不阻断出图。**
 *
 * 每一条都对应一种实际见过的废片，而且**都不报错** ——
 * 图照出、任务成功，只是几个格子是同一个机位。
 */
export function viewIssues(
  rows: ViewRow[],
  c?: SceneContract | null,
): string[] {
  const out: string[] = [];
  const picked = rows.map((r) => termById(r.termId)).filter((t): t is ViewTerm => Boolean(t));

  const blanks = rows.length - picked.length;
  if (blanks > 0) {
    out.push(
      `还有 ${blanks} 个格子没选方位。空格子模型多半会自己找东西填满 —— ` +
        "那一格是废的，还占了一次采样的像素预算",
    );
  }

  const seen = new Set<string>();
  for (const t of picked) {
    if (seen.has(t.id)) out.push(`「${t.name}」选了两次，切出来的文件会重名`);
    seen.add(t.id);
  }

  // **注记不分档全都显示。** 原来只显示 `poor` 那一档，
  // 而「实测一般」的注记同样值得看（「横移和 45 度两格容易撞」这种，
  // 选完才发现两格一样就白花一次采样）。
  for (const t of picked) {
    if (t.reliability !== "ok" && t.note) out.push(`${t.name}：${t.note}`);
  }

  if (c !== undefined && !hasAnchors(c)) {
    out.push(
      "这个场景还没填不可移动的锚点，方位描述只能用「最显眼的那个不可移动物」这种通用说法 —— " +
        "区分度会差一截。锚点写清楚了，「视线尽头是什么」才有东西可填",
    );
  }

  const { w, h } = cellSize(rows.length);
  if (shortSide(rows.length) < LAYOUT_ONLY_BELOW) {
    out.push(
      `${gridLabel(rows.length)} 每格只有 ${w}×${h}（短边 ${Math.min(w, h)}）—— ` +
        "这是**布局稿**：构图、景别和光线走向可信，材质和细节不可信。" +
        "要当成品用就开逐格精修",
    );
  }
  return out;
}

/**
 * 把草拟出来的契约并进现有的那份，**机位一个不动**。
 *
 * # 为什么机位不能跟着草拟走
 *
 * 契约里有两样东西，所有者不同：
 *
 * - **空间**（形状 / 分区 / 锚点 / 入口 / 关系 / 材质）—— 从剧本读出来的，
 *   草拟的产物就是这个。
 * - **机位**（`views`）—— 用户在 2×2 / 2×3 / 3×3 的格子里选的，
 *   用的是 `VIEW_TERMS` 那套固定词汇。
 *
 * 草拟出来的那份自带一套 `views`，名字是模型现起的（「殿门内侧」之类）。
 * 而 `viewsFromContract` 是**按名字精确匹配** `VIEW_TERMS` 反查的 ——
 * 对不上的就成了空格子。
 *
 * 2026-09-12 实拍：用户点「从剧本草拟」存完，四个格子里三个变成
 * 「（这一格空着）」，剩下那个还落在第 4 格。整份替换把选好的机位
 * 冲掉了，而空格子「模型多半会自己找东西填满，那一格是废的」。
 *
 * 所以这个方向只搬空间，`contractWithViews` 只搬机位 —— 两个方向各管一半。
 */
export function mergeDraftKeepingViews(
  current: SceneContract | null | undefined,
  draft: SceneContract,
): SceneContract {
  return { ...draft, views: current?.views ?? [] };
}
