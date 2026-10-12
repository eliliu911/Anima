import type { AssetKind, AssetView } from "../../../api/assets";
import { coverGradient } from "../coverGradient";
import { NsfwThumb } from "./NsfwThumb";
import { AssetCardMenu } from "./AssetCardMenu";

/**
 * 资产卡片。
 *
 * 封面之外必须显示：**形象（角色）/ 候选图数、归属、被引用数**。
 * 少了它们，卡片和「一张图」没有区别 —— 而资产和图的差别正在这里。
 *
 * # 比例跟类型走（2026-09-15 重设计）
 *
 * 以前一律正方形：角色设定卡、场景横图、道具混在一起，谁都显示不好。
 * 人是竖的 → 角色 3:4；场景是横的 → 16:9；道具 → 1:1。封面一律 `contain`，
 * 设定卡（一张图几个视图）裁成卡片比例会切掉一半，卡上只剩一条腿。
 */
export const CARD_ASPECT: Record<AssetKind, string> = {
  character: "aspect-[3/4]",
  scene: "aspect-video",
  prop: "aspect-square",
};

/**
 * 项目的出图画幅（`model_settings.character_aspect` 这几个）→ 卡片图片位的比例。
 *
 * 2026-09-30 用户：角色卡上下各一大块黑 —— 图片位写死 3:4 竖版，而《回收师》角色画幅设的是横版，
 * 四视图是 16:9，`object-contain` 塞进竖框就上下留黑。**在项目里就按项目的画幅来**；
 * 资产库页跨项目、没有一个统一的画幅，不传就还按类型的老规矩（`CARD_ASPECT`）
 */
export const ASPECT_CLASS: Record<string, string> = {
  landscape: "aspect-video",
  portrait: "aspect-[3/4]",
  square: "aspect-square",
};

/** 每类卡片的最小宽度：场景横图要宽一点才看得清 */
export const CARD_MIN_WIDTH: Record<AssetKind, number> = {
  character: 150,
  scene: 230,
  prop: 140,
};

export function AssetCard({
  asset,
  active,
  showScope,
  onOpen,
  onDeleted,
  aspect,
}: {
  asset: AssetView;
  active: boolean;
  /** 没按项目筛选时才需要标归属 */
  showScope: boolean;
  onOpen: () => void;
  /** 项目的出图画幅（`landscape` / `portrait` / `square`）。不给 = 按类型的老规矩 */
  aspect?: string;
  /**
   * 给了就在卡片右下角放一个「⋯」菜单，删除在菜单里（`AssetCardMenu`），删完调它刷新列表。
   * 卡片本身是个 `<button>`，菜单不能嵌在里面 —— 外面包一层放在旁边
   */
  onDeleted?: () => void;
}) {
  const card = <AssetCardBody asset={asset} active={active} showScope={showScope} onOpen={onOpen} aspect={aspect} menu={!!onDeleted} />;
  if (!onDeleted) return card;
  // **外层必须是 `flex`、卡片按钮 `w-full`。** 包这一层之前卡片按钮自己就是网格的格子，网格把它拉满整格；
  // 包了之后拉满的是外层，`<button>` 默认按内容收缩 —— 没图的卡（只写着「电锅」）缩成字那么宽，
  // 同一排高低不齐（2026-09-30 用户截图）。flex 让按钮在纵向自动拉到一排一样高，`w-full` 管横向
  return (
    <div className="group relative flex">
      {card}
      <div className="absolute bottom-1.5 right-1.5 z-10">
        <AssetCardMenu asset={asset} onDeleted={onDeleted} />
      </div>
    </div>
  );
}

function AssetCardBody({
  asset,
  active,
  showScope,
  onOpen,
  aspect,
  menu = false,
}: {
  asset: AssetView;
  active: boolean;
  showScope: boolean;
  onOpen: () => void;
  aspect?: string;
  /** 右下角有「⋯」菜单：名字那一栏右边让出位置，别被盖住 */
  menu?: boolean;
}) {
  const ratio = (aspect && ASPECT_CLASS[aspect]) || CARD_ASPECT[asset.kind];
  const looks = asset.look_views ?? [];
  const main = looks[0];
  const extraLooks = asset.kind === "character" || asset.kind === "prop" ? looks.length : 0;
  const candidates = asset.unit.image_variants.length + asset.unit.video_variants.length;
  const cover = main?.cover_url ?? asset.cover_url;
  return (
    <button type="button" onClick={onOpen} className="studio-tile flex w-full flex-col text-left" data-active={active}>
      <div
        className={`studio-tile-media studio-grain relative flex ${ratio} w-full items-center justify-center`}
        style={cover ? { background: "var(--studio-bg-inset)" } : { background: coverGradient(asset.id) }}
      >
        {cover ? (
          <NsfwThumb path={cover} nsfw={Boolean(main?.nsfw)} revealKey={`${asset.id}/${main?.id ?? "main"}`} className="h-full w-full object-contain" />
        ) : (
          // **没有图就显示占位，不要留一个碎图图标。**
          // 碎图会被当成「加载失败」，而这里其实是「还没生成」。
          <span className="studio-display px-3 text-center text-[13px] leading-tight" style={{ color: "rgba(255,255,255,.82)" }}>
            {asset.name}
          </span>
        )}
        {asset.starred && (
          <span className="absolute right-1.5 top-1.5 text-[12px]" style={{ color: "var(--studio-primary)" }}>
            ★
          </span>
        )}
        {/* 角色显示形象（2 个及以上才有意义）；场景道具显示候选图数 */}
        {extraLooks > 1 ? (
          <span className="absolute bottom-1.5 left-1.5 right-1.5 flex items-center gap-1">
            {looks.slice(0, 4).map((l) => (
              <span
                key={l.id}
                title={l.name}
                className="block h-6 w-6 shrink-0 overflow-hidden rounded-full"
                style={{ boxShadow: "0 0 0 1.5px var(--studio-bg-elevated)", background: "var(--studio-bg-inset)" }}
              >
                {l.cover_url && <NsfwThumb path={l.cover_url} nsfw={l.nsfw} revealKey={`${asset.id}/${l.id}`} compact className="h-full w-full object-contain" />}
              </span>
            ))}
            <span className="studio-pill ml-auto font-mono">{extraLooks} 形象</span>
          </span>
        ) : (
          candidates > 1 && <span className="studio-pill absolute bottom-1.5 right-1.5 font-mono">×{candidates}</span>
        )}
      </div>
      <div className={`flex flex-col gap-0.5 py-2 pl-2.5 ${menu ? "pr-8" : "pr-2.5"}`}>
        <p className="truncate text-[12px]">{asset.name}</p>
        {/* 一行放下：归属只写项目名（集数在详情里），过长截断；引用数不换行 */}
        <div className="flex min-w-0 items-center gap-1.5 whitespace-nowrap font-mono text-[0.55rem]" style={{ color: "var(--studio-text-muted)" }}>
          {showScope && <span className="min-w-0 truncate" title={asset.scope_label}>{asset.project_title ?? asset.scope_label}</span>}
          {asset.reference_count > 0 && <span className="shrink-0" title="有多少个镜头在用它">{showScope ? "· " : ""}{asset.reference_count} 镜引用</span>}
        </div>
      </div>
    </button>
  );
}
