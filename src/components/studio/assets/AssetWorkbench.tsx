import { useEffect, useMemo, useState } from "react";
import { UploadImageButton } from "./UploadImageButton";
import { DeleteAssetButton } from "../library/DeleteAssetButton";
import {
  assetsAddVariant,
  assetsFavoriteVariant,
  assetsRemoveVariant,
  assetsSelectVariant,
  type AssetView,
} from "../../../api/assets";
import {
  styleCatalog,
  styleGet,
  styleTemplateThumbs,
  type ArtDirection,
  type StyleCatalog,
} from "../../../api/style";
import {
  studioCapabilities,
  studioGenerate,
  type CapabilityCatalog,
} from "../../../api/studio";
import type { CinemaProject } from "../../../api/cinema";
import { MediaThumb } from "../MediaThumb";
import { ImageLightbox } from "../ImageLightbox";
import { useToasts } from "../toastStore";
import {
  appendTag,
  buildPrompt,
  CHARACTER_TEMPLATES,
  PROP_DEFAULT_TEMPLATE,
  PROP_TEMPLATES,
  finalNegative,
  finalPositive,
  negationWarning,
  QUICK_TAGS,
  templateApplied,
  templateById,
  templateNegative,
  withoutSingleImageComposition,
  type AssetKind,
  type TemplateId,
} from "./assetPrompt";
import {
  QUAD_ZONES,
  REF_BOOST_DEFAULT,
  SHEET_ZONES,
  guessEntityType,
  locksFromDescription,
  sheetBlocker,
} from "./entitySheet";
import { characterTemplateByBody } from "../steps/batchAssets";
import { effectiveStyle } from "./effectiveStyle";
import {
  viewsFromContract,
  contractWithViews,
  toViewpoints,
  gridLabel,
  type ViewRow,
} from "./sceneViews";
import { effectiveGen, toOverrides } from "./effectiveGen";
import { assetsUpdate, type LoraPick } from "../../../api/assets";
import { withDisembodied, withVoiceId } from "./disembodied";
import { cinemaVoiceTemplates, type VoiceTemplates } from "../../../api/cinema";
import { VOICE_FIELD_HINT, VOICE_FIELD_LABEL, engineSourceNote, unsetOptionLabel } from "./voiceFieldCopy";
import { DISEMBODIED_HINT, DISEMBODIED_LABEL, DISEMBODIED_NOTE, showImageUi } from "./disembodiedCopy";
import { saveChatImageAttachment } from "../../../api";
import { studioLoras, type LoraOption } from "../../../api/studio";

/**
 * 资产生成工作台。点一张资产卡进来。
 *
 * 结构照 LumenX 的 `CastWorkbenchModal`（MIT，Copyright 2026 Alibaba）：
 * 左边是这个资产是什么 + 风格基线，中间是构图模板 → 提示词 → 标签 →
 * 最终预览 → 抽卡，右边是候选图。
 *
 * # 为什么提示词是可改的、而且默认填好
 *
 * 默认填好，是因为「描述 + 构图」这两段拼起来才出得对；
 * 让人从空白开始写，十有八九漏掉构图段，出来一张随便的半身照。
 *
 * 可改，是因为模板管不了具体角色的怪要求（「猫，不要拟人」）。
 * 改过之后**换模板不会覆盖你的改动** —— 要回到模板得点「还原模板」。
 */
export function AssetWorkbench({
  project,
  asset,
  onClose,
  onChanged,
  initialLookId,
  onDeleted,
}: {
  project: CinemaProject;
  asset: AssetView;
  onClose: () => void;
  onChanged: () => void;
  /** 在工作台里删了这个资产之后。不给 = 刷新并关掉工作台 */
  onDeleted?: () => void;
  /** 打开时选中哪个形象（从详情面板某个形象上点「生成」进来）。不给 = 本项目默认形象 */
  initialLookId?: string;
}) {
  // **角色按形象出图。** 默认打开本项目的默认形象；切换后出的图、上传的图都进那个形象。
  // 工作台里读的描述、候选图全部换成这个形象的 —— 用一个投影出来的 AssetView，里面几十处读 `asset.*` 的不用改。
  const looks = asset.look_views ?? [];
  const initial = initialLookId ?? project.default_looks?.[asset.id];
  const [lookId, setLookId] = useState<string>(initial && looks.some((l) => l.id === initial) ? initial : "main");
  const look = looks.find((l) => l.id === lookId) ?? looks[0];
  const view: AssetView = look
    ? { ...asset, description: look.prompt, unit: look.unit, image_urls: look.image_urls, image_paths: look.image_paths, cover_url: look.cover_url }
    : asset;
  const lookBar =
    (asset.kind === "character" || asset.kind === "prop") && looks.length > 1 ? (
      <div className="mt-3">
        <span className="studio-eyebrow">出到哪个形象</span>
        <div className="mt-1.5 flex flex-wrap gap-1">
          {looks.map((l) => (
            <button
              key={l.id}
              type="button"
              onClick={() => setLookId(l.id)}
              aria-pressed={l.id === look?.id}
              className="rounded-full px-2.5 py-1 text-[11.5px]"
              style={{
                background: l.id === look?.id ? "var(--studio-primary)" : "var(--studio-bg-inset)",
                color: l.id === look?.id ? "var(--studio-on-primary)" : "var(--studio-text-2)",
              }}
            >
              {l.name}
              {l.nsfw ? " · NSFW" : ""}
            </button>
          ))}
        </div>
      </div>
    ) : null;
  return (
    <WorkbenchForLook
      // 换形象重建：提示词草稿、候选图选择这些本地状态都是按形象的
      key={look?.id ?? "main"}
      project={project}
      asset={view}
      lookId={look?.id ?? "main"}
      lookBar={lookBar}
      onClose={onClose}
      onChanged={onChanged}
      onDeleted={onDeleted}
    />
  );
}

function WorkbenchForLook({
  project,
  asset,
  lookId,
  lookBar,
  onClose,
  onChanged,
  onDeleted,
}: {
  project: CinemaProject;
  asset: AssetView;
  lookId: string;
  lookBar: React.ReactNode;
  onClose: () => void;
  onChanged: () => void;
  onDeleted?: () => void;
}) {
  const kind = asset.kind as AssetKind;
  /** 出图那一整套要不要显示。勾了「没有身体」就不该再问怎么画他的身体 */
  const imageUi = showImageUi(kind, !!asset.character?.disembodied);
  const voiceId = asset.character?.voice_id ?? "";
  const [voiceTpl, setVoiceTpl] = useState<VoiceTemplates | null>(null);
  useEffect(() => {
    if (kind !== "character") return;
    void cinemaVoiceTemplates().then(setVoiceTpl).catch(() => setVoiceTpl(null));
  }, [kind]);
  /** 改过之后换模板不覆盖。 */
  const [dirty, setDirty] = useState(false);
  const [batch, setBatch] = useState(2);
  const [applyStyle, setApplyStyle] = useState(true);
  const [previewOpen, setPreviewOpen] = useState(false);
  /**
   * 场景要出哪一种。**三个产物共用底部那一个生成按钮。**
   *
   * 原来「出 2×2 宫格」和「出 360° 全景」各自是一个按钮，摆在方位表
   * 那一段里 —— 加上底部的「生成形象」，一页上三个生成按钮，
   * 用户的原话是「满页面都是生成按钮」。
   *
   * **`single` 那一档不能省。** 只留宫格和全景的话，一张普通的场景
   * establishing 图就再也出不出来了 —— 那是悄悄少掉一个能力。
   */
  const [sceneOut, setSceneOut] = useState<"single" | "sheet" | "pano">("sheet");
  /**
   * LoRA 那一栏展开没有。**默认折叠** —— 九条带说明的列表占掉大半页，
   * 而场景和道具几乎用不到（挂的是画风，场景要的是地理）。
   *
   * 挂了的那几个名字照样显示在折叠标题上：**折叠不等于藏起来**。
   * 一个看不见却在生效的 LoRA 正是今天踩过的坑（设置里那两个常驻的
   * 把 11 个资产全带跑了）。
   */
  const [lorasOpen, setLorasOpen] = useState(false);
  /** 候选图删除的两步确认：正在等第二次点击的那张。null = 没有。 */
  const [delArm, setDelArm] = useState<string | null>(null);
  const [art, setArt] = useState<ArtDirection | null>(null);
  const [cat, setCat] = useState<StyleCatalog | null>(null);
  const [caps, setCaps] = useState<CapabilityCatalog | null>(null);
  const [thumbs, setThumbs] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** 正在放大看的是第几张。设定卡缩到 140px 时脸只剩十几个像素，看不全等于瞎选。 */
  const [zoom, setZoom] = useState<number | null>(null);
  const pushToast = useToasts((s) => s.push);
  const resolveToast = useToasts((s) => s.resolve);

  useEffect(() => {
    void styleGet(project.id).then(setArt).catch(() => setArt(null));
    void styleCatalog().then(setCat).catch(() => setCat(null));
    void studioCapabilities().then(setCaps).catch(() => setCaps(null));
    void styleTemplateThumbs().then(setThumbs).catch(() => setThumbs({}));
  }, [project.id]);

  const style = useMemo(() => effectiveStyle(art, cat), [art, cat]);
  const settings = project.model_settings;
  /** 这个资产实际用什么 —— 项目设置打底，资产自己的覆盖优先。 */
  const eff = effectiveGen(kind, asset.gen, settings);
  // 道具默认选中信息卡（`PROP_DEFAULT_TEMPLATE`），引擎跟着它要的那个走 —— 和点卡时 `pickTemplate` 同一条规矩
  const [engine, setEngine] = useState(
    kind === "prop" ? (templateById(PROP_DEFAULT_TEMPLATE).engines?.[0] ?? eff.engine) : eff.engine,
  );
  // **这一段必须排在 `engine` 后面。**
  //
  // 2026-09-20 出过事：它原来在文件上方，而 `engine` 声明在下面 ——
  // `useMemo` 的回调是**渲染时同步执行**的，于是读到一个还在 TDZ 里的 const，
  // 点开任意一张角色卡就白屏报 `Cannot access 'ge' before initialization`。
  //
  // **`tsc` 拦不住这一类**：变量是在闭包里用的，编译器没法断定闭包什么时候跑，
  // 所以 `npm run build`（含 `tsc --noEmit`）一路绿灯。
  /**
   * 初始版式**按描述挑**，不是写死 `"simple"`。
   *
   * 写死的后果实测过：小青的描述是「蓝色卡通 AI 助理头像」，
   * 而工作台一打开就是「简洁三视图」，构图段里写着
   * 「Right half: three equally-sized full-body standing poses」——
   * **光这个构图就足以把任何描述拉成一个人**，和描述写什么无关。
   * 用户点生成，拿到一个真人，而且看不出是哪一步决定的。
   *
   * 批量出图那条路（`asset_batch`）2026-09-19 就改成按描述挑了，
   * 这里漏了 —— 同一件事两条路，只改了一条。
   */
  //
  // 2026-09-26 起**默认一张构图卡都不选**（用户：「不是必须选一个，不选择默认居中头像」）：
  // 有身体的角色 = `portrait`（居中头肩像），没有身体的 = `avatar`（图标）。
  // 判「有没有身体」照旧用批量那份（`characterTemplateByBody`），只取它 avatar 那一支。
  //
  // 道具不选卡 = `simple`（在道具上就是产品摄影那段构图，不带画幅偏好）—— 不能用 `portrait`，
  // 取消选择时会把画幅切成竖图。
  const noTemplate: TemplateId =
    kind !== "character"
      ? "simple"
      : characterTemplateByBody(engine, asset.description ?? "") === "avatar"
        ? "avatar"
        : "portrait";
  const initialTemplate = useMemo<TemplateId>(
    () => (kind === "character" ? noTemplate : kind === "prop" ? PROP_DEFAULT_TEMPLATE : "simple"),
    // 打开这一张资产时定一次就够 —— 之后换引擎不该把用户选好的版式顶掉
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [asset.id],
  );
  const [template, setTemplate] = useState<TemplateId>(initialTemplate);
  const [prompt, setPrompt] = useState(() => buildPrompt(kind, asset, initialTemplate));
  const [aspectId, setAspectId] = useState(eff.aspect);
  const [picks, setPicks] = useState<LoraPick[]>(eff.loras);
  const [allLoras, setAllLoras] = useState<LoraOption[]>([]);
  const [savedNote, setSavedNote] = useState<string | null>(null);
  // 构图模板原来分「文生 / 图生」两页。2026-09-26 精简到三张之后并成一页
  // （用户：「构图模版一共就留 3 个，不用分页了」），图生那张卡上标「图生」。

  useEffect(() => {
    void studioLoras().then(setAllLoras).catch(() => setAllLoras([]));
  }, []);

  /**
   * 存成这个资产的覆盖。**和项目设置一样的那几项不会被存下来** ——
   * 存了的话以后改项目设置，这个资产会莫名其妙不跟着变。
   */
  /**
   * 存「只有声音」这个开关。
   *
   * `assets_update` 的 `character` 是**整份替换**，所以要把现有的那份
   * 铺开再改一位 —— 只传一个 `{disembodied}` 会把 persona、声音 id、
   * 视觉 DNA 全清掉，而且不报错。
   */
  const saveDisembodied = async (on: boolean) => {
    try {
      await assetsUpdate({
        scope: asset.scope,
        id: asset.id,
        character: withDisembodied(asset.character, on),
      });
      onChanged();
    } catch (e) {
      setError(String(e));
    }
  };

  /** 改音色。**必须铺开整份** —— 见 `withVoiceId` 的注释 */
  const saveVoice = async (v: string) => {
    try {
      await assetsUpdate({ scope: asset.scope, id: asset.id, character: withVoiceId(asset.character, v) });
      onChanged();
    } catch (e) {
      setError(String(e));
    }
  };

  const saveGen = async () => {
    try {
      await assetsUpdate({
        scope: asset.scope,
        id: asset.id,
        gen: toOverrides(kind, { engine, aspect: aspectId, loras: picks }, settings),
      });
      setSavedNote("已记住这个资产的出图配置");
      setTimeout(() => setSavedNote(null), 2500);
      onChanged();
    } catch (e) {
      setError(String(e));
    }
  };

  /** 只列这个引擎能用的 —— 架构不匹配挂上去不生效**且不报错**。 */
  const usableLoras = allLoras.filter((l) => l.engines.includes(engine));
  const toggleLora = (name: string) =>
    setPicks((p) =>
      p.some((x) => x.name === name)
        ? p.filter((x) => x.name !== name)
        : [...p, { name, strength: 0.8 }],
    );

  const negativeBase = templateNegative(kind, template);
  /** 这个引擎到底吃不吃负向词。能力表里现成的，别在前端另写一份。 */
  const negativeWorks = caps?.engines.find((e) => e.id === engine)?.negative_prompt ?? true;
  const outPositive = finalPositive(prompt, style.positive, applyStyle, kind);
  const negation = negationWarning(prompt, negativeWorks);
  /**
   * **预览里显示的必须就是这一种产物真正发出去的那一段。**
   *
   * 场景有三种产物，发出去的提示词各不相同：
   * · 单张 —— 原样
   * · 2×2 宫格 —— 去掉「出一张图」那一段（`single unified image` 会和
   *   宫格版式打架，实拍四格出成同一个机位）；版式和逐格点名由 Rust 侧
   *   `scene_sheet::grid_prompt` 拼，这里看不到，但那一段是固定的。
   * · 360° 全景 —— 同样去掉，另外后端会把 `360 view` 触发词拼在最前
   *   （`PANO_TRIGGER`），画幅锁 1536×768。
   *
   * 各算各的话，预览和实际会悄悄分家 —— 而用户是照着预览判断的。
   */
  const previewPositive =
    kind === "scene" && sceneOut !== "single"
      ? withoutSingleImageComposition(outPositive)
      : outPositive;
  const outNegative = finalNegative(style.negative, negativeBase, applyStyle);

  const pickTemplate = (id: TemplateId) => {
    // **再点一次已选中的卡 = 取消选择**，回到不选构图时的默认（居中头像）。
    // 引擎不跟着动：没选构图就没有「这个版式只能用哪个引擎」这回事。
    if (id === template && id !== noTemplate) {
      setTemplate(noTemplate);
      if (!dirty) setPrompt(buildPrompt(kind, asset, noTemplate));
      const d = templateById(noTemplate);
      if (d.prefersAspect) setAspectId(d.prefersAspect);
      return;
    }
    setTemplate(id);
    // 没改过才跟着换 —— 改过的内容不该被一次点击抹掉
    if (!dirty) setPrompt(buildPrompt(kind, asset, id));
    const t = templateById(id);
    // **构图对画幅有要求就跟着切。** 五分区在方图上会把
    // 「三个全身视图」出成 4 个 —— 实测过，不是猜的。
    if (t.prefersAspect) setAspectId(t.prefersAspect);
    // **引擎也跟着切。**
    //
    // 判据是模板自己的 `engines` 白名单，**不是写死的 id 名单** ——
    // 白名单本来就是「这个模板只能用哪个引擎」的唯一所有者，
    // 再抄一份 id 名单在这儿，加模板时忘了改就又是「选了没反应」。
    //
    // 没白名单的（简洁三视图 / 详细参考表）回默认的 Z-Image：
    // 它最快、也是这些模板调出来时用的那个。
    setEngine((t.engines?.[0] ?? "zimage") as typeof engine);
  };

  /**
   * 场景的方位表 —— **同时就是空间契约**。
   *
   * 填的每一行会存回 `asset.scene.views`，下次出图、镜头上下文节点、
   * 360° 全景节点读的都是它。「在哪儿定义场景」的答案就是这儿。
   */
  const [views, setViews] = useState<ViewRow[]>(() => viewsFromContract(asset.scene));
  // **逐格精修拆了。** 它默认就是关的，而开着要多四次出图；
  // 真要精修的话点进单张改图那条路更直接。用户 2026-09-12 点名去掉。
  const refine = false;
  // 换了资产要重读它自己的方位表，否则会把上一个场景的方位带过来存错地方
  useEffect(() => {
    setViews(viewsFromContract(asset.scene));
  }, [asset.id, asset.scene]);

  /** 选满了的格子。没选的不发 —— 发一个空方位出去那一格就是废的。 */
  const pickedViewpoints = toViewpoints(views, asset.scene);

  /**
   * 出一张 360° 全景，按 90° 切成四方位，**四张都入库**。
   *
   * # 和宫格是两条路，不是替代关系
   *
   * | | 宫格 | 全景 |
   * |---|---|---|
   * | 四个机位怎么来 | **求**模型画四个不同的 | 从一张球面投影里**算** |
   * | 一致性 | 提示词约束（实拍出过四格一样的） | **几何保证** |
   * | 依赖 | 无 | 360Redmond LoRA（缺了当场停） |
   * | 缺陷 | 机位可能拉不开 | 左右接缝不闭合，「背面」那张带缝 |
   *
   * 方位名固定是正/右/背/左（90° 等分），**不读方位表** ——
   * 那张表是给宫格选机位用的，全景是整圈都有，切哪几个是纯粹的取样。
   */
  const generatePanoViews = async () => {
    setBusy(true);
    setError(null);
    const tid = pushToast({
      kind: "progress",
      title: `正在出「${asset.name}」的 360° 全景${batch > 1 ? ` × ${batch}` : ""}…`,
      body: "每张出完按 90° 切成正面 / 右侧 / 背面 / 左侧四张，四张同源。",
    });
    // **「生成数量」对全景也生效**：×N 就是独立出 N 张全景（各自切四方位）。
    // 以前这里不读 batch，选 ×2 也只出一张（2026-09-17 用户发现）。
    let ok = 0;
    let added = 0;
    let lastErr = "";
    let seamNote = "";
    try {
      for (let i = 0; i < batch; i++) {
        const out = await studioGenerate({
          mode: "pano_views",
          engine,
          // 画幅、引擎、触发词都由后端锁死 —— 这条路上用户选的不作数
          prompt: withoutSingleImageComposition(outPositive),
          negative_prompt: outNegative,
          width: 1536,
          height: 768,
          images: [],
          loras: picks,
          auto_loras: false,
        });
        if (!out.ok) {
          lastErr = out.message || "引擎没有返回图片";
          continue;
        }
        // `paths[0]` 是全景本身 —— 留着，出了问题要回去看是切错了还是画错了
        const files = out.paths?.length ? out.paths : out.path ? [out.path] : [];
        for (const f of files) {
          await assetsAddVariant({
            scope: asset.scope,
            id: asset.id,
            look_id: lookId,
            media: { kind: "path", path: f },
            prompt_used: outPositive,
          });
        }
        if (out.message?.includes("接缝")) seamNote = out.message;
        ok += 1;
        added += files.length;
        onChanged();
      }
      if (ok === 0) {
        resolveToast(tid, { kind: "error", title: "全景没出来", body: lastErr });
        setError(lastErr);
        return;
      }
      resolveToast(tid, {
        kind: "success",
        title: `${ok} 张全景（共 ${added} 张图）都入库了`,
        // **接缝提示原样透传。** 后端算出来的那句话要让人看见 ——
        // 不说的话「背面」那张带的缝会被当成构图问题重出一遍。
        body: lastErr
          ? `有 ${batch - ok} 张失败：${lastErr}`
          : seamNote || "每组四张取自同一张球面投影，光线和材质必然同源。",
      });
    } catch (e) {
      resolveToast(tid, { kind: "error", title: "全景没出来", body: String(e) });
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  /**
   * 出一张多方位宫格，切开之后**每一格都入库**，并把方位写回契约。
   *
   * 写回契约是这条路的一半价值：下次不用重填，而且别的地方（镜头上下文、
   * 全景节点）读得到同一份地理。
   */
  const generateSceneSheet = async () => {
    setBusy(true);
    setError(null);
    const tid = pushToast({
      kind: "progress",
      title: `正在出「${asset.name}」的 ${gridLabel(views.length)} 宫格${batch > 1 ? ` × ${batch}` : ""}…`,
      body: `${engine} · 每张一次采样出 ${pickedViewpoints.length} 个机位，然后切开。`,
    });
    // **「生成数量」对宫格也生效**：×N 就是独立出 N 张宫格（各自切开入库）。
    // 以前这里不读 batch，选 ×2 也只出一张（2026-09-17 用户发现）。
    let ok = 0;
    let added = 0;
    let lastErr = "";
    const notes: string[] = [];
    try {
      for (let i = 0; i < batch; i++) {
        const out = await studioGenerate({
          mode: "scene_sheet",
          engine,
          // **去掉「出一张图」那一段。** 场景的构图段是给单张场景图写的
          // （`single unified image` / `wide establishing shot`），而宫格那条路
          // 会把这里传的当成「场景描述」嵌进宫格提示词 —— 两套指令打架，
          // 实拍四格出成同一个正面机位。版式和机位由宫格自己负责。
          prompt: withoutSingleImageComposition(outPositive),
          negative_prompt: outNegative,
          // 画幅固定横幅：格子越接近正方越好读，而 1536×1024 是本机常用尺寸。
          // 用户选的比例在这条路上不生效 —— 界面上说了。
          width: 1536,
          height: 1024,
          images: [],
          loras: picks,
          // 设置里那两个常驻 LoRA 不参与资产出图 —— 见 `auto_loras`
          auto_loras: false,
          viewpoints: pickedViewpoints,
          refine,
        });
        if (!out.ok) {
          lastErr = out.message || "引擎没有返回图片";
          continue;
        }
        // **联合图之外的每一张都入库。** `paths[0]` 是那张联合图本身 ——
        // 它也留着，出了问题要回去看是切错了还是画错了。
        const files = out.paths?.length ? out.paths : out.path ? [out.path] : [];
        for (const f of files) {
          await assetsAddVariant({
            scope: asset.scope,
            id: asset.id,
            look_id: lookId,
            media: { kind: "path", path: f },
            prompt_used: outPositive,
          });
        }
        for (const n of out.notes ?? []) if (!notes.includes(n)) notes.push(n);
        ok += 1;
        added += files.length;
        onChanged();
      }
      if (ok === 0) {
        resolveToast(tid, { kind: "error", title: "宫格没出来", body: lastErr });
        setError(lastErr);
        return;
      }
      // 方位写回契约。**失败不算整件事失败** —— 图已经出来入库了。
      try {
        await assetsUpdate({
          scope: asset.scope,
          id: asset.id,
          scene: contractWithViews(asset.scene, views),
        });
      } catch (e) {
        console.warn("方位没存回场景契约", e);
      }
      onChanged();
      resolveToast(tid, {
        kind: "success",
        title: `出了 ${added} 张`,
        body: lastErr
          ? `有 ${batch - ok} 张宫格失败：${lastErr}`
          : notes.length ? notes.join(" · ") : "点一张设为封面。",
      });
    } catch (e) {
      const msg = String(e);
      resolveToast(tid, { kind: "error", title: "宫格没出来", body: msg });
      setError(msg);
    } finally {
      setBusy(false);
    }
  };

  const resetPrompt = () => {
    setPrompt(buildPrompt(kind, asset, template));
    setDirty(false);
  };

  const generate = async () => {
    setBusy(true);
    setError(null);
    const tid = pushToast({
      kind: "progress",
      title: `正在为「${asset.name}」出图…`,
      body: `${engine} · ${batch} 张 · 一张几十秒，期间可以继续做别的。`,
    });
    let ok = 0;
    let lastErr = "";
    try {
      const capabilities = caps ?? (await studioCapabilities());
      const aspect = capabilities.aspects.find((a) => a.id === aspectId) ?? capabilities.aspects[0];
      for (let i = 0; i < batch; i++) {
        const out = await studioGenerate({
          mode: "t2i",
          engine,
          prompt: outPositive,
          negative_prompt: outNegative,
          width: aspect.width,
          height: aspect.height,
          images: [],
          loras: picks,
          // 设置里那两个常驻 LoRA 不参与资产出图 —— 见 `auto_loras`
          auto_loras: false,
        });
        if (!out.ok || !out.path) {
          lastErr = out.message || "引擎没有返回图片";
          continue;
        }
        await assetsAddVariant({
          scope: asset.scope,
          id: asset.id,
          look_id: lookId,
          media: { kind: "path", path: out.path },
          prompt_used: outPositive,
        });
        ok += 1;
        onChanged();
      }
      if (ok === 0) {
        resolveToast(tid, { kind: "error", title: "一张都没出来", body: lastErr });
        setError(lastErr);
      } else {
        resolveToast(tid, {
          kind: "success",
          title: `出了 ${ok} 张`,
          body: lastErr ? `有 ${batch - ok} 张失败：${lastErr}` : "点一张设为封面。",
        });
      }
    } catch (e) {
      resolveToast(tid, { kind: "error", title: "出图失败", body: String(e) });
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const variants = asset.unit.image_variants;

  // ── 参考表（七区 entity sheet）──
  //
  // **七分区是独立的一张卡**（`pipeline: "entity_sheet"`），选它就是那条路。
  //
  // 前两版都被否了：放在底部折叠块 → 顶上同名模板出的不是那个东西；
  // 同一张卡按引擎分流 → 一张卡两种产出，而提示词框里显示的是另一条路的那段，
  // 「五分区和七分区的提示词不一样，出来的内容会冲突」。
  const isSheet = !!templateById(template).pipeline;
  // 实体类型和身份锚点**自动推**，不再给用户加控件 —— 类型猜错了图上看得出，
  // 锚点用资产描述切出来的够用。要调的话改资产描述，那才是它们的来源。
  const entityType = guessEntityType(asset.kind, asset.description);
  const locks = locksFromDescription(asset.description);
  /**
   * 图生那条用哪张底图。
   *
   * **默认跟着封面走**（右边点「设为封面」就是在选它），但可以临时改 ——
   * 有时候封面是张定妆照，而你想拿另一张角度更好的去扩视角。
   * 改了只在这次生成里算数，不动封面。
   *
   * `null` = 跟着封面。
   */
  const [srcOverride, setSrcOverride] = useState<string | null>(null);
  const [srcPicking, setSrcPicking] = useState(false);
  const [uploadErr, setUploadErr] = useState<string | null>(null);
  const coverIdx = variants.findIndex((v) => v.id === asset.unit.selected_image_id);
  const coverPath = coverIdx >= 0 ? asset.image_paths[coverIdx] : asset.image_paths[0];
  const sourcePath = srcOverride ?? coverPath;
  const coverUrl = coverIdx >= 0 ? asset.image_urls[coverIdx] : asset.image_urls[0];
  const srcUrl =
    srcOverride === null
      ? coverUrl
      : (asset.image_urls[asset.image_paths.indexOf(srcOverride)] ?? srcOverride);

  /**
   * 上传一张当底图。
   *
   * **必须落盘换成本地路径** —— Tauri 的 webview 里 `<input type="file">`
   * 拿不到真实路径（安全限制），而后端要的是路径。走已有的
   * `save_chat_image_attachment`，那个命令正是为这条鸿沟写的。
   */
  const uploadSource = async (file: File) => {
    setUploadErr(null);
    try {
      const dataUrl = await new Promise<string>((ok, no) => {
        const r = new FileReader();
        r.onload = () => ok(String(r.result));
        r.onerror = () => no(new Error("读不出这个文件"));
        r.readAsDataURL(file);
      });
      const saved = await saveChatImageAttachment({ dataUrl, filename: file.name });
      setSrcOverride(saved.path);
      setSrcPicking(false);
    } catch (e) {
      setUploadErr(String(e));
    }
  };
  const krea2Ready = (caps?.engines ?? []).some((e) => e.id === "krea2");
  // caps 还没读回来时不要说「引擎没装」—— 那是「还不知道」，不是「没有」。
  const sheetWhy = sheetBlocker(!!sourcePath, caps ? krea2Ready : true);

  const generateSheet = async () => {
    if (sheetWhy || !sourcePath) return;
    setBusy(true);
    setError(null);
    // **模式跟着模板走，不能写死。** 写死的话选了四视图也出七分区 ——
    // 图照出、任务成功，只是不是你点的那个。
    const tpl = templateById(template);
    const mode = tpl.pipeline ?? "entity_sheet";
    const quad = mode === "quad_view";
    const tid = pushToast({
      kind: "progress",
      title: `正在为「${asset.name}」出${quad ? "四视图" : "参考表"}…`,
      body: quad
        ? "Krea 2 · 大脸特写 + 正/侧/背 · 约 5 分，期间可以继续做别的。"
        : "Krea 2 · 七个区 · 约 7 分半，再补三个表情约 5 分，期间可以继续做别的。",
    });
    try {
      const out = await studioGenerate({
        mode,
        engine: "krea2",
        // **提示词模板只有一个所有者，在 Rust 的 `entity_sheet::sheet_prompt`。**
        // 这里只送零件 —— 自己拼一份的表现是「改了界面这份，模型收到的还是旧的」。
        prompt: "",
        negative_prompt: "",
        entity_name: asset.name,
        entity_type: entityType,
        identity_locks: locks,
        style_line: style.positive.slice(0, 160),
        // 画幅由后端定（七个区按 3:2 排，方图会挤成一团），这里给的值会被盖掉
        width: 1536,
        height: 1024,
        images: [sourcePath],
        ref_boost: REF_BOOST_DEFAULT,
      });
      if (!out.ok || !out.path) {
        const msg = out.message || "引擎没有返回图片";
        resolveToast(tid, { kind: "error", title: `${quad ? "四视图" : "参考表"}没出来`, body: msg });
        setError(msg);
        return;
      }
      await assetsAddVariant({
        scope: asset.scope,
        id: asset.id,
        look_id: lookId,
        media: { kind: "path", path: out.path },
        prompt_used: `[${quad ? "四视图" : "七区参考表"} ${entityType}] ref_boost=${REF_BOOST_DEFAULT}`,
      });
      onChanged();
      resolveToast(tid, {
        kind: "success",
        title: `${quad ? "四视图" : "参考表"}出来了`,
        body: out.notes.length ? out.notes.join("；") : "在右边候选图里，点开看大图。",
      });
    } catch (e) {
      resolveToast(tid, { kind: "error", title: `${quad ? "四视图" : "参考表"}失败`, body: String(e) });
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="studio-scrim fixed inset-0 z-50 flex items-center justify-center">
      <div
        className="studio-modal flex h-[86vh] w-[1420px] max-w-[97vw] overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        {/* ── 左：这是什么 + 风格基线 ── */}
        <aside
          className="studio-scroll w-[240px] shrink-0 overflow-y-auto px-5 py-5"
          style={{ borderRight: "1px solid var(--studio-border-subtle)" }}
        >
          <p className="studio-eyebrow">{KIND_LABEL[kind]}</p>
          <h2 className="mt-1 text-[17px] font-semibold leading-tight">{asset.name}</h2>
          {lookBar}
          {asset.description && (
            <p className="mt-2 text-[11.5px] leading-relaxed" style={{ color: "var(--studio-text-2)" }}>
              {asset.description}
            </p>
          )}

          {/* 有现成的图就直接传，不用生成 */}
          <div className="mt-4">
            <UploadImageButton asset={asset} lookId={lookId} onDone={onChanged} />
          </div>
          {/* 2026-09-30 用户：资产页没地方删。两步确认、被镜头引用时先拦下（见 `DeleteAssetButton`） */}
          <div className="mt-2">
            <DeleteAssetButton
              asset={asset}
              onDeleted={onDeleted ?? (() => {
                onChanged();
                onClose();
              })}
            />
          </div>

          <div className="mt-5">
            <div className="mb-2 flex items-center justify-between">
              <span className="studio-eyebrow">风格基线</span>
              <button
                type="button"
                onClick={() => setApplyStyle((v) => !v)}
                aria-pressed={applyStyle}
                title={applyStyle ? "点一下不带风格出图" : "点一下带上风格"}
                className="h-[18px] w-[32px] rounded-full transition-colors"
                style={{
                  background: applyStyle ? "var(--studio-primary)" : "var(--studio-bg-inset)",
                }}
              >
                <span
                  className="block h-[14px] w-[14px] rounded-full transition-transform"
                  style={{
                    background: "var(--studio-bg-surface)",
                    transform: `translateX(${applyStyle ? 16 : 2}px)`,
                  }}
                />
              </button>
            </div>
            {style.positive ? (
              <>
                <p className="mb-1.5 text-[12px] font-medium">{style.name || "自定义"}</p>
                <StyleBox label="正向" text={style.positive} on={applyStyle} />
                {style.negative && (
                  <StyleBox
                    label="负向"
                    text={style.negative}
                    on={applyStyle && negativeWorks}
                    danger
                  />
                )}
                {/* **引擎不吃负向词时必须说出来。**
                    Z-Image 是 cfg=1，负向通路被 `ConditioningZeroOut`
                    清零 —— 传了等于没传。把负向词大大方方显示出来却不说这件事，
                    就是界面在说谎：用户以为「photorealistic」被挡住了，
                    实际一点作用没有，然后去反复改风格词。 */}
                {style.negative && !negativeWorks && (
                  <p
                    className="mb-2 text-[10.5px] leading-relaxed"
                    style={{ color: "var(--studio-failed-fg)" }}
                  >
                    {engine} 的负向提示词结构性无效（cfg=1，负向通路被清零）——
                    上面这些词传了也不起作用。要靠负向压写实，换 SDXL。
                  </p>
                )}
              </>
            ) : (
              <p className="text-[11.5px]" style={{ color: "var(--studio-text-muted)" }}>
                这个项目还没定调 —— 去第 2 步选一个，资产和镜头才会长得像一部剧。
              </p>
            )}
          </div>
        </aside>

        {/* ── 中：模板 → 提示词 → 标签 → 预览 → 抽卡 ── */}
        <div className="studio-scroll min-w-0 flex-1 overflow-y-auto px-6 py-5">
          {/* **场景这一段整个不显示。**

              以前这里有：2×3 / 3×3 档位、四个可手填的方位输入框、逐格精修、
              一段解释宫格原理的文字、三条实测告警、和一个「从剧本草拟」。
              用户 2026-09-12 判掉了这些，**2×2 固定、方位固定**。
              我当时留下了四行只读的「四个机位 + 它点名的锚点」，理由是
              「锚点要看得见，那是四格不同的决定性因素」。

              同一天用户第二次说：「上面这段不要显示出来了」。他是对的 ——
              方位是写死的四条，用户改不了，**看它没有任何可操作性**；
              而「四格为什么会长得一样」属于出问题时才需要的信息，
              不该每天占着页面顶上。

              **代价说清楚**：跟着一起没的还有「这个场景没填锚点 → 四格会长得
              一样」那条提示。现在锚点缺失是完全无声的，只能从出来的图看出来。
              要补的话该补在左边场景描述那一栏（在那儿才改得动），不是这里。 */}
          {/* **这个开关排在出图那一套前面**，因为它决定这个角色要不要图。
              勾上之后底下那一整套（构图模板 / 生成配置 / 生成）会整块收起来。

              2026-09-18 之前只是注释里写着「对它都没意义」，**代码没真的做** ——
              于是勾选框正压在三张构图模板卡上面，读起来像「在同一组设置里二选一」。
              用户就是这么读的：「这个角色有可能出镜，有可能旁白，你说你是勾选
              还不勾选？没意义啊。」措辞和联动都在 `disembodiedCopy.ts`，有测试钉着。 */}
          {kind === "character" && (
            <section className="mb-4">
              <p className="studio-eyebrow mb-2">声音与出镜</p>
              {/* **音色也放这儿。** 它原来只在资产库的详情面板里，而「资产」这一步
                  点卡片直接开的是本工作台 —— 那个面板在这一页**根本打不开**
                  （`AssetsStep` 的 `selectedId` 除了置空没有任何地方设值）。
                  用户原话：「角色绑定语音的入口我没看到」。 */}
              <label className="mb-3 flex items-center gap-2 text-[12px]">
                <span className="shrink-0" style={{ color: "var(--studio-text-muted)" }}>{VOICE_FIELD_LABEL}</span>
                {voiceTpl && voiceTpl.voices.length > 0 ? (
                  <select
                    value={voiceTpl.voices.includes(voiceId) || voiceId === "" ? voiceId : "__other"}
                    onChange={(e) => e.target.value !== "__other" && void saveVoice(e.target.value)}
                    className="studio-input flex-1"
                  >
                    <option value="">{unsetOptionLabel(voiceTpl.engine)}</option>
                    {voiceTpl.voices.map((v) => (
                      <option key={v} value={v}>{v}</option>
                    ))}
                    {voiceId !== "" && !voiceTpl.voices.includes(voiceId) && (
                      <option value="__other">{voiceId}（当前引擎里没有）</option>
                    )}
                  </select>
                ) : (
                  <input
                    defaultValue={voiceId}
                    onBlur={(e) => void saveVoice(e.target.value)}
                    placeholder="不指定 = 配音时用全局音色"
                    className="studio-input flex-1"
                  />
                )}
              </label>
              {/* **这一句必须常在。** 用户看到下拉框写着「跟全局设置」时问
                  「那不是会一直用音色？」—— 答案是不会（不点配音就用不到），
                  而那时候这里什么都没说。 */}
              <p className="mb-3 text-[11px] leading-relaxed" style={{ color: "var(--studio-text-muted)" }}>
                {VOICE_FIELD_HINT}
              </p>
              {voiceTpl && (
                <p className="mb-3 text-[11px] leading-relaxed" style={{ color: "var(--studio-text-muted)" }}>
                  {engineSourceNote(voiceTpl.engine, voiceTpl.voices.length)}
                </p>
              )}
              {voiceTpl?.hint && (
                <p className="mb-3 text-[11px] leading-relaxed" style={{ color: "var(--studio-text-muted)" }}>
                  {voiceTpl.hint}
                </p>
              )}
              <label className="flex cursor-pointer items-start gap-2 text-[12px]">
                <input
                  type="checkbox"
                  className="mt-0.5"
                  checked={!!asset.character?.disembodied}
                  onChange={(e) => void saveDisembodied(e.target.checked)}
                />
                <span>
                  {DISEMBODIED_LABEL}
                  <span className="ml-1" style={{ color: "var(--studio-text-muted)" }}>
                    —— {DISEMBODIED_HINT}
                  </span>
                </span>
              </label>
            </section>
          )}

          {kind === "character" && !imageUi && (
            <p
              className="mb-4 rounded-lg px-3 py-2.5 text-[12px]"
              style={{ background: "var(--studio-bg-inset)", color: "var(--studio-text-muted)" }}
            >
              {DISEMBODIED_NOTE}
            </p>
          )}

          {(kind === "character" || kind === "prop") && imageUi && (
            <section className="mb-4">
              <p className="studio-eyebrow mb-2">
                构图模板
                <span className="ml-2 normal-case tracking-normal" style={{ color: "var(--studio-text-muted)" }}>
                  {template === noTemplate
                    ? `没选 —— 出一张${kind === "prop" ? "产品图（主视图 + 材质特写）" : noTemplate === "avatar" ? "居中图标" : "居中头像"}`
                    : "再点一次选中的卡可以取消"}
                </span>
              </p>
              {/* 四张卡一行。窗口宽到放不下时退成 2×2 —— 而不是挤成一排
                  看不清缩略图（那是选构图的唯一依据）。 */}
              {/* 三张卡一行：简洁三视图、Qwen 设定卡、四视图（图生）。 */}
              <div className="flex items-stretch gap-2">
              <div className="grid flex-1 grid-cols-3 gap-2">
                {(kind === "prop" ? PROP_TEMPLATES : CHARACTER_TEMPLATES).filter((t) =>
                  !t.hidden,
                ).map((t) => {
                  const on = t.id === template;
                  // 引擎撑不住的话，**选中之后**要说出来（见下面卡片底部那段）
                  const fits = !t.engines || t.engines.includes(engine);
                  return (
                    <button
                      key={t.id}
                      type="button"
                      onClick={() => pickTemplate(t.id)}
                      // **`flex-col justify-start` 不能省。** 网格把三张卡拉成一样高，
                      // 而 `<button>` 默认把内容垂直居中 —— 说明文字短的那张，
                      // 示例图就整体往下沉，三张图高低不齐（用户 2026-09-26 截图指出）。
                      className="flex flex-col justify-start overflow-hidden rounded-xl text-left"
                      style={{
                        background: on
                          ? "color-mix(in srgb, var(--studio-primary) 12%, transparent)"
                          : "var(--studio-bg-inset)",
                        boxShadow: `0 0 0 ${on ? 1.5 : 1}px ${
                          on ? "var(--studio-primary)" : "var(--studio-border-subtle)"
                        }`,
                      }}
                    >
                      {/* **示例图。** 三个模板的差别是版面，光靠一句话说不清
                          「大头特写 + 三视图」和「三视图 + 大脸 + 三角度」
                          到底差在哪 —— 一眼看图就知道。
                          这些图是我们自己的引擎出的，同一个角色同一个 seed，
                          所以差异里只有构图这一个变量。 */}
                      <span
                        className="relative block aspect-[3/2] w-full"
                        style={{ background: "var(--studio-bg-base)" }}
                      >
                        {thumbs[t.id] ? (
                          <MediaThumb
                            path={thumbs[t.id]}
                            alt=""
                            // 图是 3:2、容器也是 3:2 —— `contain` 和 `cover`
                            // 看着一样，那就用不裁的那个，别给规则开口子。
                            className="h-full w-full object-contain"
                          />
                        ) : (
                          <span
                            className="flex h-full w-full items-center justify-center text-[10px]"
                            style={{ color: "var(--studio-text-muted)" }}
                          >
                            还没出示例图
                          </span>
                        )}
                        {on && (
                          <span
                            className="absolute right-1.5 top-1.5 grid h-5 w-5 place-items-center rounded-full text-[10px]"
                            style={{
                              background: "var(--studio-primary)",
                              color: "var(--studio-on-primary)",
                            }}
                          >
                            ✓
                          </span>
                        )}
                      </span>
                      <span className="block px-3 pb-2.5 pt-2">
                      <span className="flex items-center gap-1.5 text-[12.5px] font-medium">
                        {t.label}
                        {/* 图生的卡要先有封面图 —— 和另外两张凭空造的混在一行，
                            不标出来会以为是同一档的三个选择 */}
                        {t.pipeline && (
                          <span
                            className="rounded px-1.5 py-px text-[10px] font-normal"
                            style={{
                              background: "color-mix(in srgb, var(--studio-primary) 14%, transparent)",
                              color: "var(--studio-primary)",
                            }}
                          >
                            图生
                          </span>
                        )}
                      </span>
                      <span
                        className="mt-0.5 block text-[11px] leading-snug"
                        style={{ color: "var(--studio-text-muted)" }}
                      >
                        {t.hint}
                      </span>
                      {/* **卡片上默认不挂红字**（用户 2026-09-26：「把红字去掉」）。
                          点卡时引擎会自动切到它要的那个，所以平时永远对得上；
                          只有选中之后又手动把引擎换走，才在这张卡上说出来 ——
                          不说就是静默出一张错字 / 没版式的图。
                          没底图的理由不在卡上说，选中后下面那块说明里有。 */}
                      {on && !fits && (
                        <span
                          className="mt-1 block text-[10.5px] leading-snug"
                          style={{ color: "var(--studio-failed-fg)" }}
                        >
                          {engine} {t.mismatch ?? "撑不住这个版面"} —— 换成{" "}
                          {t.engines?.join(" / ")}
                        </span>
                      )}
                      </span>
                    </button>
                  );
                })}
              </div>
              </div>
            </section>
          )}

          {/* **选中的卡 ≠ 实际用的构图** 时必须说出来。
              提示词改过之后再换卡，卡片亮了、画幅切了，但提示词没动 ——
              不提示的话就是「点了、亮了、没生效」。 */}
          {(kind === "character" || kind === "prop") && !templateApplied(prompt, kind, template) && (
            <p
              className="mb-2 rounded-lg px-2.5 py-1.5 text-[11px] leading-snug"
              style={{
                background: "color-mix(in srgb, var(--studio-failed-fg) 12%, transparent)",
                color: "var(--studio-failed-fg)",
              }}
            >
              提示词是你改过的，没跟着换构图 —— 这次出图用的是下面这段，不是「
              {templateById(template).label}」。要套用请点「还原模板」。
            </p>
          )}

          {isSheet && (
            <section
              className="mb-3 rounded-xl px-3.5 py-3"
              style={{ background: "var(--studio-bg-inset)" }}
            >
              <p className="studio-eyebrow mb-1.5">
                {templateById(template).label}：拿封面图扩视角
              </p>
              <p className="text-[11.5px] leading-relaxed">
                这条是图生图，身份全部来自<b>当前封面那张图</b>。提示词由后端按结构化
                模板拼（身份锚点取自资产描述）—— 和五分区那条<b>不共用提示词</b>，
                所以下面的提示词框和标签这次都不显示。
              </p>
              <ol
                className="mt-2 ml-4 list-decimal text-[10.5px] leading-relaxed"
                style={{ color: "var(--studio-text-muted)" }}
              >
                {(templateById(template).pipeline === "quad_view"
                  ? QUAD_ZONES
                  : SHEET_ZONES
                ).map((z) => (
                  <li key={z}>{z}</li>
                ))}
              </ol>
              <p className="mt-2 text-[10.5px] leading-snug" style={{ color: "var(--studio-text-muted)" }}>
                封面最好用<b>单张形象图</b>：源图上带中文标签时，那些字会被临摹进去并画歪。
              </p>
              {sheetWhy && (
                <p className="mt-2 text-[11px] leading-snug" style={{ color: "var(--studio-failed-fg)" }}>
                  {sheetWhy}
                </p>
              )}
            </section>
          )}

          <section className={isSheet ? "mb-3 hidden" : "mb-3"}>
            <div className="mb-2 flex items-center justify-between">
              <span className="studio-eyebrow">生成提示词（可改）</span>
              {dirty && (
                <button
                  type="button"
                  onClick={resetPrompt}
                  className="text-[11px]"
                  style={{ color: "var(--studio-text-muted)" }}
                >
                  还原模板
                </button>
              )}
            </div>
            <textarea
              value={prompt}
              onChange={(e) => {
                setPrompt(e.target.value);
                setDirty(true);
              }}
              rows={9}
              className="w-full resize-y rounded-xl px-3.5 py-3 text-[12.5px] leading-relaxed outline-none"
              style={{
                background: "var(--studio-bg-inset)",
                border: "1px solid var(--studio-border-subtle)",
                color: "var(--studio-text)",
              }}
            />
            {negation && (
              <p className="mt-1.5 text-[11px] leading-relaxed" style={{ color: "var(--studio-accent)" }}>
                {negation}
              </p>
            )}
            <div className="mt-2 flex flex-wrap gap-1.5">
              {QUICK_TAGS[kind].map((tag) => (
                <button
                  key={tag}
                  type="button"
                  onClick={() => {
                    setPrompt((p) => appendTag(p, tag));
                    setDirty(true);
                  }}
                  className="rounded-lg px-2.5 py-1 font-mono text-[11px]"
                  style={{ background: "var(--studio-bg-inset)", color: "var(--studio-text-muted)" }}
                >
                  + {tag}
                </button>
              ))}
            </div>
          </section>

          {/* **真发出去的那一份。** 预览和实际不一致是这条链路最阴的失败 ——
              这里显示的就是 `finalPositive/finalNegative` 的结果本身。 */}
          <section className="mb-4">
            <button
              type="button"
              onClick={() => setPreviewOpen((v) => !v)}
              className="studio-eyebrow mb-1.5 flex items-center gap-1"
            >
              最终提示词{previewOpen ? " ▾" : " ▸"}
            </button>
            {previewOpen && (
              <div
                className="rounded-xl px-3.5 py-3 text-[11.5px] leading-relaxed"
                style={{ background: "var(--studio-bg-inset)" }}
              >
                <p className="whitespace-pre-wrap break-words">{previewPositive}</p>
                {/* 后端还会再拼一层，那一层这里看不到 —— 说出来，
                    否则用户会以为预览就是全部。 */}
                {kind === "scene" && sceneOut === "sheet" && (
                  <p className="mt-2" style={{ color: "var(--studio-text-muted)" }}>
                    出图时后端会在这一段前面拼上版式（2×2 网格、白线分隔）和四格
                    逐个点名的机位，以及跨视角的一致性清单。
                  </p>
                )}
                {kind === "scene" && sceneOut === "pano" && (
                  <p className="mt-2" style={{ color: "var(--studio-text-muted)" }}>
                    出图时后端会在最前面拼上 <code>360 view</code> 触发词，
                    画幅锁 1536×768（等距圆柱必须 2:1），并自动挂 360Redmond LoRA。
                  </p>
                )}
                {outNegative && (
                  <p
                    className="mt-2 whitespace-pre-wrap break-words border-t pt-2"
                    style={{
                      color: "var(--studio-text-muted)",
                      borderColor: "var(--studio-border-subtle)",
                    }}
                  >
                    负向：{outNegative}
                  </p>
                )}
              </div>
            )}
          </section>

          {/* ── 生成配置。**放在生成按钮旁边，不是埋进项目设置** ——
                「这个角色要挂专属 LoRA」「这张场景要横幅」是出图当下才想调的事。
                默认全部跟项目设置走，改了才算这个资产的覆盖。 ── */}
          {imageUi && (
          <>
          <section className="mb-4">
            <p className="studio-eyebrow mb-2">生成配置</p>

            {/* **底图只在图生那条出现** —— 文生模板没有源图这回事，
                摆出来只会让人以为文生也要先有图。 */}
            {isSheet && (
              <div className="mb-3">
                <p className="mb-1.5 text-[11.5px]" style={{ color: "var(--studio-text-muted)" }}>
                  底图 —— 身份全部来自它
                </p>
                <div className="flex items-center gap-2">
                  {srcUrl ? (
                    <MediaThumb
                      path={srcUrl}
                      alt="底图"
                      className="h-12 w-12 shrink-0 rounded-lg object-contain"
                    />
                  ) : (
                    <div
                      className="flex h-12 w-12 shrink-0 items-center justify-center rounded-lg text-[10px]"
                      style={{
                        background: "var(--studio-bg-inset)",
                        color: "var(--studio-text-muted)",
                      }}
                    >
                      没有
                    </div>
                  )}
                  <div className="min-w-0 flex-1">
                    <p className="text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
                      {srcOverride === null ? "跟着封面走" : "这次临时换了一张（不动封面）"}
                    </p>
                    <div className="mt-1 flex flex-wrap gap-1.5">
                      <button
                        type="button"
                        onClick={() => setSrcPicking((v) => !v)}
                        className="rounded-lg px-2 py-1 text-[11px]"
                        style={{
                          background: "var(--studio-bg-inset)",
                          color: "var(--studio-text)",
                        }}
                      >
                        {srcPicking ? "收起" : "换一张"}
                      </button>
                      {srcOverride !== null && (
                        <button
                          type="button"
                          onClick={() => setSrcOverride(null)}
                          className="rounded-lg px-2 py-1 text-[11px]"
                          style={{
                            background: "var(--studio-bg-inset)",
                            color: "var(--studio-text-muted)",
                          }}
                        >
                          用回封面
                        </button>
                      )}
                    </div>
                  </div>
                </div>

                {srcPicking && (
                  <div
                    className="mt-2 rounded-xl p-2"
                    style={{ background: "var(--studio-bg-inset)" }}
                  >
                    {variants.length > 0 && (
                      <>
                        <p
                          className="mb-1.5 text-[10.5px]"
                          style={{ color: "var(--studio-text-muted)" }}
                        >
                          从这个资产已有的图里挑
                        </p>
                        <div className="mb-2 flex flex-wrap gap-1.5">
                          {variants.map((v, i) => (
                            <button
                              key={v.id}
                              type="button"
                              onClick={() => {
                                setSrcOverride(asset.image_paths[i] ?? null);
                                setSrcPicking(false);
                              }}
                              className="overflow-hidden rounded-lg"
                              style={{
                                boxShadow:
                                  asset.image_paths[i] === sourcePath
                                    ? "0 0 0 2px var(--studio-primary)"
                                    : "0 0 0 1px var(--studio-border-subtle)",
                              }}
                            >
                              <MediaThumb
                                path={asset.image_urls[i]}
                                alt=""
                                className="h-12 w-12 object-contain"
                              />
                            </button>
                          ))}
                        </div>
                      </>
                    )}
                    <label
                      className="flex cursor-pointer items-center justify-center rounded-lg border border-dashed py-2 text-[11px]"
                      style={{
                        borderColor: "var(--studio-border-subtle)",
                        color: "var(--studio-text-muted)",
                      }}
                    >
                      从本地上传一张
                      <input
                        type="file"
                        accept="image/*"
                        className="hidden"
                        onChange={(e) => {
                          const f = e.target.files?.[0];
                          if (f) void uploadSource(f);
                          e.target.value = "";
                        }}
                      />
                    </label>
                    {uploadErr && (
                      <p className="mt-1 text-[10.5px]" style={{ color: "var(--studio-failed-fg)" }}>
                        {uploadErr}
                      </p>
                    )}
                  </div>
                )}
              </div>
            )}

            <div className="mb-3">
              <p className="mb-1.5 text-[11.5px]" style={{ color: "var(--studio-text-muted)" }}>
                画面比例 · 分辨率
              </p>
              <div className="flex flex-wrap gap-1.5">
                {(caps?.aspects ?? []).map((a) => {
                  const on = a.id === aspectId;
                  return (
                    <button
                      key={a.id}
                      type="button"
                      onClick={() => setAspectId(a.id)}
                      className="rounded-lg px-2.5 py-1.5 font-mono text-[11px]"
                      style={{
                        background: on
                          ? "color-mix(in srgb, var(--studio-primary) 14%, transparent)"
                          : "var(--studio-bg-inset)",
                        color: on ? "var(--studio-text)" : "var(--studio-text-2)",
                        boxShadow: on
                          ? "0 0 0 1px color-mix(in srgb, var(--studio-primary) 45%, transparent)"
                          : "none",
                      }}
                    >
                      {a.label}
                    </button>
                  );
                })}
              </div>
            </div>

            <div className="mb-3">
              <p className="mb-1.5 text-[11.5px]" style={{ color: "var(--studio-text-muted)" }}>
                生成模型
              </p>
              <div className="flex flex-wrap gap-1.5">
                {(caps?.engines ?? [])
                  .filter((e) => e.kind === "image")
                  .map((e) => {
                    const on = e.id === engine;
                    return (
                      <button
                        key={e.id}
                        type="button"
                        onClick={() => {
                          setEngine(e.id);
                          // **换引擎要清掉挂不上的 LoRA。** 留着的话它会被
                          // 静默忽略（0 bypass adapters）—— 图照出、不报错、
                          // 只是那个风格一点没生效。
                          setPicks((p) =>
                            p.filter((x) =>
                              allLoras.find((l) => l.name === x.name)?.engines.includes(e.id),
                            ),
                          );
                        }}
                        title={`约 ${e.secs}s / 张${e.negative_prompt ? "" : " · 无负向"}`}
                        className="rounded-lg px-2.5 py-1.5 text-[11.5px]"
                        style={{
                          background: on
                            ? "color-mix(in srgb, var(--studio-primary) 14%, transparent)"
                            : "var(--studio-bg-inset)",
                          color: on ? "var(--studio-text)" : "var(--studio-text-2)",
                          boxShadow: on
                            ? "0 0 0 1px color-mix(in srgb, var(--studio-primary) 45%, transparent)"
                            : "none",
                        }}
                      >
                        {e.label}
                      </button>
                    );
                  })}
              </div>
            </div>

            <div>
              {/* **默认折叠。**
                  九个 LoRA 每条带一段说明，展开后占掉大半页，而场景和道具
                  几乎用不到它们（挂的是画风，场景要的是地理）。
                  挂了的那几个照样显示在标题上 —— 折叠不等于藏起来，
                  一个看不见却在生效的 LoRA 正是今天踩过的坑。 */}
              <button
                type="button"
                onClick={() => setLorasOpen((v) => !v)}
                className="mb-1.5 flex w-full items-center gap-1.5 text-left text-[11.5px]"
                style={{ color: "var(--studio-text-muted)" }}
                aria-expanded={lorasOpen}
              >
                <span>{lorasOpen ? "▾" : "▸"}</span>
                <span>
                  LoRA{usableLoras.length === 0 ? " —— 没有适配这个引擎的" : ` · ${usableLoras.length} 个可用`}
                </span>
                {picks.length > 0 && (
                  <span style={{ color: "var(--studio-primary)" }}>
                    已挂 {picks.map((x) => x.name.split("/").pop()?.replace(/\.safetensors$/, "")).join("、")}
                  </span>
                )}
              </button>
              <div className="flex flex-col gap-1" hidden={!lorasOpen}>
                {usableLoras.map((l) => {
                  const pick = picks.find((x) => x.name === l.name);
                  const trig = l.confirmed_triggers.length ? l.confirmed_triggers : l.triggers;
                  return (
                    <div key={l.name} className="flex items-start gap-2">
                      {/* **说明放进那片空白里。** 原来只挂在悬浮提示上 ——
                          一行名字加一大片空白，而「这个 LoRA 是干嘛的」
                          恰恰是选它时唯一要判断的事。 */}
                      <button
                        type="button"
                        onClick={() => toggleLora(l.name)}
                        className="min-w-0 flex-1 rounded-lg px-2.5 py-1.5 text-left"
                        style={{
                          background: pick
                            ? "color-mix(in srgb, var(--studio-primary) 12%, transparent)"
                            : "var(--studio-bg-inset)",
                        }}
                        title={l.note ?? "没有说明"}
                      >
                        <span className="flex items-baseline gap-2">
                          <span className="truncate text-[11.5px]">{l.label}</span>
                          {/* **成人向要标出来。** 按发布页作者标的记，不拦截 ——
                              不标的问题是：挂着它去出角色资产，出来的东西
                              不对劲而看不出为什么。 */}
                          {l.nsfw && (
                            <span
                              className="shrink-0 rounded px-1 font-mono text-[0.5625rem]"
                              style={{
                                background: "var(--studio-failed-bg)",
                                color: "var(--studio-failed-fg)",
                              }}
                              title="发布页标为成人向。只是提示，不影响能不能用。"
                            >
                              18+
                            </span>
                          )}
                          {trig.length > 0 && (
                            <span
                              className="shrink-0 font-mono text-[0.5625rem]"
                              style={{ color: "var(--studio-primary)" }}
                            >
                              {trig.join(" ")}
                              {l.confirmed_triggers.length ? "" : "?"}
                            </span>
                          )}
                        </span>
                        <span
                          // 一行就够 —— 六个 LoRA 各占两三行的话，这一栏比
                          // 提示词还长。全文在悬浮提示里。
                          className="mt-0.5 block truncate text-[10.5px] leading-snug"
                          style={{ color: "var(--studio-text-muted)" }}
                        >
                          {l.note ?? "没有说明 —— 去创作台的「风格化 LORA」那栏写一句"}
                        </span>
                      </button>
                      {pick && (
                        <>
                          <input
                            type="range"
                            min={0}
                            max={1.5}
                            step={0.05}
                            value={pick.strength}
                            onChange={(e) =>
                              setPicks((p) =>
                                p.map((x) =>
                                  x.name === l.name
                                    ? { ...x, strength: Number(e.target.value) }
                                    : x,
                                ),
                              )
                            }
                            className="mt-1.5 w-[92px]"
                          />
                          <span
                            className="mt-1 w-8 shrink-0 text-right font-mono text-[10px] tabular-nums"
                            style={{ color: "var(--studio-primary)" }}
                          >
                            {pick.strength.toFixed(2)}
                          </span>
                        </>
                      )}
                    </div>
                  );
                })}
              </div>
              {picks.some((p) => {
                const l = allLoras.find((x) => x.name === p.name);
                return l && l.confirmed_triggers.length > 0;
              }) && (
                <p className="mt-1.5 text-[10.5px]" style={{ color: "var(--studio-text-muted)" }}>
                  挂了带激活词的 LoRA —— 激活词要自己写进上面的提示词里，我们不替你偷偷塞。
                </p>
              )}
              <button
                type="button"
                onClick={() => void saveGen()}
                className="mt-2 rounded-lg px-2.5 py-1 text-[11px]"
                style={{ background: "var(--studio-bg-inset)", color: "var(--studio-text-2)" }}
              >
                记住这个资产的配置
              </button>
              {savedNote && (
                <span className="ml-2 text-[11px]" style={{ color: "var(--studio-done-fg)" }}>
                  {savedNote}
                </span>
              )}
            </div>
          </section>

          <section className="flex flex-wrap items-center gap-3">
            {/* **场景有三种产物，共用一个生成按钮。**
                原来宫格和全景各自是一个按钮、摆在方位表那一段里，加上这里的
                「生成形象」一页三个 —— 用户原话「满页面都是生成按钮」。
                改成单选：选哪一种，右边那个按钮就干哪一种。 */}
            {kind === "scene" && (
              <div
                className="flex items-center gap-0.5 rounded-full p-1"
                style={{ background: "var(--studio-bg-inset)" }}
              >
                {([
                  ["single", "单张", "一张普通的场景图"],
                  ["sheet", "2×2 宫格", "一次采样出四个机位再切开，靠提示词让它们拉开"],
                  ["pano", "360° 全景", "出一张球面全景按 90° 切四方位，一致性是几何保证的；要 360Redmond LoRA"],
                ] as const).map(([id, label, hint]) => (
                  <button
                    key={id}
                    type="button"
                    title={hint}
                    onClick={() => setSceneOut(id)}
                    aria-pressed={sceneOut === id}
                    className="rounded-full px-3 py-1 text-[11.5px]"
                    style={{
                      background: sceneOut === id ? "var(--studio-bg-surface)" : "transparent",
                      color: sceneOut === id ? "var(--studio-text)" : "var(--studio-text-muted)",
                    }}
                  >
                    {label}
                  </button>
                ))}
              </div>
            )}

            {/* **先选怎么出，再选出几份。** 数量对单张 / 宫格 / 全景都生效（×N = 独立出 N 次）。
                参考表 / 四视图一次就是几分钟的一整条流水线，不给数量 —— 以前这里显示了却不生效。 */}
            {!isSheet && (
              <>
              <span className="studio-eyebrow">生成数量</span>
              <div
                className="flex items-center gap-0.5 rounded-full p-1"
                style={{ background: "var(--studio-bg-inset)" }}
              >
                {[1, 2, 3, 4].map((n) => (
                  <button
                    key={n}
                    type="button"
                    onClick={() => setBatch(n)}
                    className="rounded-full px-3 py-1 font-mono text-[11.5px]"
                    style={{
                      background: batch === n ? "var(--studio-bg-surface)" : "transparent",
                      color: batch === n ? "var(--studio-text)" : "var(--studio-text-muted)",
                    }}
                  >
                    ×{n}
                  </button>
                ))}
              </div>
              <span className="font-mono text-[10.5px]" style={{ color: "var(--studio-text-muted)" }}>
                {engine} · {aspectId}
              </span>
              </>
            )}

            <button
              type="button"
              // 参考表那条不吃提示词框（提示词在 Rust 侧拼），所以别用它当禁用条件。
              // 宫格要至少两个机位 —— 一个机位的「宫格」没有意义。
              disabled={
                busy ||
                (isSheet
                  ? !!sheetWhy
                  : kind === "scene" && sceneOut === "sheet"
                    ? pickedViewpoints.length < 2
                    : !prompt.trim())
              }
              onClick={() =>
                void (isSheet
                  ? generateSheet()
                  : kind === "scene" && sceneOut === "sheet"
                    ? generateSceneSheet()
                    : kind === "scene" && sceneOut === "pano"
                      ? generatePanoViews()
                      : generate())
              }
              className="ml-auto rounded-lg px-4 py-2 text-[12.5px] font-medium"
              style={{
                background: "var(--studio-primary)",
                color: "var(--studio-on-primary)",
                opacity: busy ? 0.65 : 1,
              }}
            >
              {busy
                ? "出图中…"
                : kind === "scene" && sceneOut === "sheet"
                  ? `出 ${gridLabel(views.length)} 宫格`
                  : kind === "scene" && sceneOut === "pano"
                    ? "出 360° 全景"
                    : !isSheet
                      ? "生成形象"
                      : templateById(template).pipeline === "quad_view"
                        ? "生成四视图"
                        : "生成完整参考表"}
            </button>
          </section>
          </>
          )}

          {error && (
            <p className="mt-3 text-[11.5px]" style={{ color: "var(--studio-failed-fg)" }}>
              {error}
            </p>
          )}
        </div>

        {/* ── 右：候选图 ── */}
        <aside
          className="studio-scroll w-[320px] shrink-0 overflow-y-auto px-5 py-5"
          style={{ borderLeft: "1px solid var(--studio-border-subtle)" }}
        >
          <div className="mb-3 flex items-center justify-between">
            <span className="studio-eyebrow">候选图 {variants.length}</span>
            <button
              type="button"
              onClick={onClose}
              aria-label="关闭"
              className="text-[13px]"
              style={{ color: "var(--studio-text-muted)" }}
            >
              ✕
            </button>
          </div>
          {variants.length === 0 ? (
            <p className="text-[11.5px]" style={{ color: "var(--studio-text-muted)" }}>
              还没有图。左边写好提示词，点「生成形象」。
            </p>
          ) : (
            <div className="grid grid-cols-2 gap-2">
              {variants.map((v, i) => {
                const on = v.id === asset.unit.selected_image_id;
                return (
                  <div key={v.id} className="relative">
                    <button
                      type="button"
                      title="设为封面（@ 引用时用的就是它）"
                      onClick={() =>
                        void assetsSelectVariant(asset.scope, asset.id, v.id).then(onChanged)
                      }
                      className="block w-full overflow-hidden rounded-lg"
                      style={{
                        background: "var(--studio-bg-inset)",
                        boxShadow: `0 0 0 ${on ? 2 : 1}px ${
                          on ? "var(--studio-primary)" : "var(--studio-border-subtle)"
                        }`,
                      }}
                    >
                      {/* **`contain` 不是 `cover`。** 设定卡是宽版面，
                          裁成正方形会把三视图那半边切掉 —— 而那正是要看的东西。 */}
                      <MediaThumb
                        path={asset.image_urls[i]}
                        className="aspect-square w-full object-contain"
                      />
                    </button>
                    <button
                      type="button"
                      title="放大看"
                      aria-label="放大看"
                      onClick={() => setZoom(i)}
                      className="studio-media-btn absolute bottom-1 right-1 grid h-6 w-6 place-items-center rounded-full"
                    >
                      <svg
                        width="13"
                        height="13"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        strokeWidth="1.8"
                        strokeLinecap="round"
                      >
                        <circle cx="10.5" cy="10.5" r="6.5" />
                        <path d="M15.5 15.5 21 21M10.5 7.5v6M7.5 10.5h6" />
                      </svg>
                    </button>
                    <button
                      type="button"
                      title={v.favorited ? "取消收藏" : "收藏（不会被自动清理）"}
                      onClick={() =>
                        void assetsFavoriteVariant(
                          asset.scope,
                          asset.id,
                          v.id,
                          !v.favorited,
                        ).then(onChanged)
                      }
                      className="absolute -right-1 -top-1 rounded-full px-1 text-[10px]"
                      style={{
                        background: "var(--studio-bg-elevated)",
                        color: v.favorited ? "var(--studio-primary)" : "var(--studio-text-muted)",
                      }}
                    >
                      ★
                    </button>
                    {/* **删除要两步。** 连文件一起删、不可逆，而这一栏里
                        每张卡只有几十像素见方 —— 一次点击的误触率太高。
                        第一下变成「删?」并变红，再点一下才真删。 */}
                    <button
                      type="button"
                      title={delArm === v.id ? "再点一次就真的删了" : "删掉这张（连文件一起删）"}
                      onClick={() => {
                        if (delArm !== v.id) {
                          setDelArm(v.id);
                          return;
                        }
                        setDelArm(null);
                        void assetsRemoveVariant(asset.scope, asset.id, v.id)
                          .then(onChanged)
                          .catch((e) => setError(String(e)));
                      }}
                      onMouseLeave={() => delArm === v.id && setDelArm(null)}
                      className="absolute -left-1 -top-1 rounded-full px-1 text-[10px]"
                      style={{
                        background: "var(--studio-bg-elevated)",
                        color:
                          delArm === v.id
                            ? "var(--studio-failed-fg)"
                            : "var(--studio-text-muted)",
                        fontWeight: delArm === v.id ? 600 : 400,
                      }}
                    >
                      {delArm === v.id ? "删?" : "×"}
                    </button>
                  </div>
                );
              })}
            </div>
          )}
          <p className="mt-3 text-[10.5px] leading-relaxed" style={{ color: "var(--studio-text-muted)" }}>
            非收藏的最多留 12 张，超出会连文件一起删。
          </p>
        </aside>
      </div>

      {zoom !== null && (
        <ImageLightbox
          paths={asset.image_urls}
          index={zoom}
          onIndex={setZoom}
          onClose={() => setZoom(null)}
          caption={`${asset.name} · 候选 ${zoom + 1}`}
          actions={
            <>
              <button
                type="button"
                onClick={() => {
                  const v = variants[zoom];
                  if (v) void assetsSelectVariant(asset.scope, asset.id, v.id).then(onChanged);
                }}
                className="rounded-lg px-3.5 py-2 text-[12px] font-medium"
                style={{
                  background:
                    variants[zoom]?.id === asset.unit.selected_image_id
                      ? "var(--studio-bg-inset)"
                      : "var(--studio-primary)",
                  color:
                    variants[zoom]?.id === asset.unit.selected_image_id
                      ? "var(--studio-text-muted)"
                      : "var(--studio-on-primary)",
                }}
              >
                {variants[zoom]?.id === asset.unit.selected_image_id ? "已是封面" : "设为封面"}
              </button>
              <button
                type="button"
                onClick={() => {
                  const v = variants[zoom];
                  if (v)
                    void assetsFavoriteVariant(
                      asset.scope,
                      asset.id,
                      v.id,
                      !v.favorited,
                    ).then(onChanged);
                }}
                className="rounded-lg px-3.5 py-2 text-[12px]"
                style={{
                  background: "var(--studio-bg-inset)",
                  color: variants[zoom]?.favorited
                    ? "var(--studio-primary)"
                    : "var(--studio-text-2)",
                }}
              >
                {variants[zoom]?.favorited ? "★ 已收藏" : "☆ 收藏"}
              </button>
            </>
          }
        />
      )}
    </div>
  );
}

const KIND_LABEL: Record<AssetKind, string> = {
  character: "角色",
  scene: "场景",
  prop: "道具",
};

function StyleBox({
  label,
  text,
  on,
  danger,
}: {
  label: string;
  text: string;
  on: boolean;
  danger?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const long = text.length > 70;
  return (
    <div
      className="mb-2 rounded-lg px-2.5 py-2"
      style={{
        background: "var(--studio-bg-inset)",
        opacity: on ? 1 : 0.4,
        borderLeft: `2px solid ${danger ? "var(--studio-failed-fg)" : "var(--studio-primary)"}`,
      }}
    >
      <p className="studio-eyebrow mb-1">{label}</p>
      <p
        className="text-[11px] leading-relaxed"
        style={{ color: "var(--studio-text-2)", wordBreak: "break-word" }}
      >
        {open || !long ? text : `${text.slice(0, 70)}…`}
      </p>
      {long && (
        <button
          type="button"
          onClick={() => setOpen((v) => !v)}
          className="mt-1 text-[10.5px]"
          style={{ color: "var(--studio-primary)" }}
        >
          {open ? "收起" : "展开"}
        </button>
      )}
    </div>
  );
}
