import type { AssetView } from "../../../api/assets";
import type { AssetKind, TemplateId } from "../assets/assetPrompt";
import { templateById } from "../assets/assetPrompt";
import { hasAnchors, resizeSlots, type ViewRow } from "../assets/sceneViews";

/**
 * 批量出场景时默认出几格。
 *
 * **4 格**，和 liblib 那套 2×2 一致，也是 `GRID_CHOICES` 的第一档。
 * 6/9 格单张里每一格更小，批量时看不清 —— 要更多机位的逐个进工作台调。
 */
export const BATCH_GRID = 4;

/** 默认那四个机位（正面中心线 / 左前 45° / 右前 45° / 室内最深处向外）。 */
export function defaultViews(): ViewRow[] {
  return resizeSlots([], BATCH_GRID);
}

/**
 * 「一键把还没出图的资产全出一遍」横幅上的那几句话。
 *
 * **真正排活、拼提示词、出图在后端**（`src-tauri/src/asset_batch.rs`，任务表里的 `asset_images`），
 * 切页、刷新都不断；这里只算给人看的数。版式选择和画幅在 Rust `asset_prompt` 里有同一份，
 * 靠 `assetPromptFixtures.test.ts` 逐字对拍。
 *
 * # 为什么值得有
 *
 * 剧本抽完实体之后，资产库里一排卡片全是占位渐变。以前只能一张一张点进
 * 工作台出 —— 十个资产就是十次「点进去、选模板、出图、退出来」，
 * 而且**没有任何地方告诉你还差几个**：卡片上没图和有图长得差不多，
 * 漏掉一个要到出分镜时才发现（那时它只会以名字进提示词，长相不会带上）。
 *
 * 纯算账单拎出来，是因为下面每一条判断错了都是静默的：
 * 少算一个资产、挑错模板、给场景发了角色的版式 —— 图照出，只是不对。
 */

/**
 * **标了「从不出镜」的角色不参与出图。**
 *
 * 它根本没有能拍出来的身体（旁白员、电话那头的人、纯语音 AI）——
 * 出一张立绘既浪费一次生成，出来的图也没有任何地方会用：
 * 分镜不给它镜头，提示词里它只按文字出现。
 *
 * 2026-09-19 用户在 E05 上撞到：小青、小橙标了从不出镜，
 * 横幅照样说「还有 4 个角色形象没有设定图」，而「一键全出」
 * 会真的给它们排两次生成。后端 `asset_batch::plan` 同步排除。
 */
export function needsImage(a: AssetView): boolean {
  return !(a.kind === "character" && a.character?.disembodied);
}

/**
 * 还没有图的那些。
 *
 * **判据是封面**，不是变体数量 —— 变体可能只有一张被删了文件的记录，
 * 而封面是「界面上真的显示得出来东西」。
 */
export function missingCovers(items: AssetView[]): AssetView[] {
  return items.filter((a) => needsImage(a) && !a.cover_url);
}

/**
 * 按类分别还差几张。界面上要逐类说 —— 「还差 7 个」不告诉人该去哪儿找。
 *
 * **角色按形象数**：主形象有图、「现代」形象没图，也算差一张 —— 后台那批（`asset_batch::plan`）会出它。
 */
export function countMissing(items: AssetView[]): Record<AssetKind, number> {
  const out: Record<AssetKind, number> = { character: 0, scene: 0, prop: 0 };
  for (const a of items) {
    const k = a.kind as AssetKind;
    if (!(k in out)) continue;
    // 从不出镜的角色不算「还差一张」—— 它本来就不该有图
    if (!needsImage(a)) continue;
    const looks = a.look_views ?? [];
    out[k] += looks.length > 1 ? looks.filter((l) => !l.cover_url).length : a.cover_url ? 0 : 1;
  }
  return out;
}

/**
 * 角色批量用哪个版式。
 *
 * **中文四视图（`quadzh`）只有 Z-Image 撑得住** —— 那段构图规范是中文长句，
 * 换引擎出来的是一张普通立绘，而且不报错。所以引擎不对时退回
 * `simple`（英文、任何引擎都行），**并且要在界面上说出来** ——
 * 静默换版式的话，用户以为自己拿到的是四视图。
 */
export function characterTemplateFor(engine: string): TemplateId {
  const quad = templateById("quadzh");
  return quad.engines?.includes(engine) ? "quadzh" : "simple";
}

/**
 * 描述里说这个角色**没有身体**的信号。
 *
 * # 为什么是正向识别，不是「列出有身体的词」
 *
 * 第一版反过来写：列一串「有身体」的关键词，一个都不中就当头像。
 * 「唐装」不在表里，于是一个穿唐装的人被判成了头像。
 * 现在只在**明确读出无实体信号**时才切 `avatar`，认不出的一律走原版式 ——
 * 保守方向是「多画一个身体」，不是「把人变成图标」。
 */
const BODILESS = [
  "头像",
  "图标",
  "标识",
  "logo",
  "icon",
  "avatar",
  "一团光",
  "光团",
  "光点",
  "光球",
  "没有轮廓",
  "无实体",
  "纯语音",
];

/**
 * 按描述挑角色版式。**和 Rust 的 `character_template_by_body` 是同一份判断**
 * （有对拍测试钉着）。
 *
 * 2026-09-19 出过事：Rust 那边加了 `avatar` 并接进批量出图，**而工作台这边
 * 写死 `"simple"`** —— 用户在工作台点生成，拿到的还是
 * 「Right half: three equally-sized full-body standing poses」，
 * 小青照样出成一个人。修一处而不扫同类，这是这个项目里第 N 次。
 */
export function characterTemplateByBody(engine: string, description: string): TemplateId {
  const d = description.toLowerCase();
  if (BODILESS.some((w) => d.includes(w.toLowerCase()))) return "avatar";
  return characterTemplateFor(engine);
}

/**
 * 版式想要的画幅。没写偏好的用竖版 —— 单人立绘竖版最省像素。
 */
export function templateAspect(id: TemplateId): string {
  return templateById(id).prefersAspect ?? "portrait";
}

/**
 * 底部那句话。**逐类说清楚，并且说明会用什么版式。**
 *
 * 空的时候返回空串 —— 「还差 0 个」是句废话，而且会让人以为按钮能点。
 */
export function batchSummary(items: AssetView[], engine: string): string {
  const n = countMissing(items);
  const bits: string[] = [];
  if (n.character) bits.push(`${n.character} 个角色形象`);
  if (n.scene) bits.push(`${n.scene} 个场景`);
  if (n.prop) bits.push(`${n.prop} 个道具`);
  if (!bits.length) return "";
  const tpl = templateById(characterTemplateFor(engine)).label;
  const how = n.character ? `角色用「${tpl}」，` : "";
  const scenes = n.scene ? `场景出 ${BATCH_GRID} 格宫格，` : "";
  // **没有空间契约的场景要单独说。**
  //
  // 没有锚点时机位描述退化成笼统说法（「从一侧看」），四个格子的区分度
  // 会差一截 —— 而**图照出、任务成功、不报错**，只有把四格并排看才发现
  // 它们像同一个机位。这一条不阻断出图，但必须说。
  const bare = missingCovers(items).filter(
    (a) => a.kind === "scene" && !hasAnchors(a.scene),
  );
  const weak = bare.length
    ? `其中 ${bare.map((a) => a.name).join("、")} 还没有空间契约 —— 四个机位会长得比较像，先填锚点效果好得多。`
    : "";
  return `还有 ${bits.join("、")}没有设定图。${how}${scenes}可以一次全出，也可以逐个点进去调。${weak}`;
}

/**
 * 全都有图之后那句话。
 *
 * **必须还有个入口。** 判据只有「有没有封面」的话，一批出砸之后
 * 十一张废图也算「有图」，横幅撤掉，没有任何地方能重来。
 *
 * 说清楚「旧的留着」—— 不说的话这个按钮读起来像会毁掉已有的图。
 */
export function redoSummary(items: AssetView[]): string {
  const n = items.length;
  const m = countMissing(items);
  if (!n || m.character + m.scene + m.prop) return "";
  return `${n} 个资产都有图了。不满意可以全部重出一遍 —— 旧的那版留在变体里，不会丢。`;
}
