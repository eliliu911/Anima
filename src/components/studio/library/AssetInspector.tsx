/**
 * 资产详情面板。
 *
 * # 2026-09-15 重设计
 *
 * 以前是一条 360px 的竖线：候选图、名称描述、角色参数（大多是「—」）、引用、流转、删除 ——
 * **最常用的和最危险的排在一条线上**。现在按页签分：
 *
 * | 类型 | 页签 |
 * |---|---|
 * | 角色 | **形象**（默认）· 人物 · 引用 |
 * | 场景 / 道具 | **图片**（默认）· 信息 · 引用 |
 *
 * 危险和少用的动作（录入全局库、删除）收进头部「⋯」菜单。
 *
 * # 形象
 *
 * 一个角色 = 人物本身 + 多个形象（发型、服装、造型），每个形象自己的描述和候选图，可以标 NSFW。
 * 发给模型的是「人物本身 + 形象描述」。方案见 `docs/plans/2026-09-15-角色形象方案.md`。
 */
import { useEffect, useState } from "react";
import {
  assetsAddLook,
  assetsDelete,
  assetsFavoriteVariant,
  assetsPromote,
  assetsReferences,
  assetsRemoveLook,
  assetsRemoveVariant,
  assetsSelectVariant,
  assetsUpdate,
  assetsUpdateLook,
  KIND_LABEL,
  MAIN_LOOK,
  type AssetReference,
  type AssetView,
  type LookView,
} from "../../../api/assets";
import { cinemaVoiceTemplates, type VoiceTemplates } from "../../../api/cinema";
import { withVoiceId } from "../assets/disembodied";
import { VOICE_FIELD_HINT, VOICE_FIELD_LABEL, engineSourceNote, unsetOptionLabel } from "../assets/voiceFieldCopy";
import { UploadImageButton } from "../assets/UploadImageButton";
import { MenuIcon, MenuItem } from "../Menu";
import { useDismissable } from "../useDismissable";
import { NsfwThumb } from "./NsfwThumb";
import { MergeLookDialog } from "./MergeLookDialog";

type Tab = "looks" | "person" | "refs";

/** 项目上下文（项目资产步骤里打开时才有）：默认形象在这里设 */
export interface InspectorProject {
  defaultLookId?: string;
  onSetDefaultLook: (lookId: string) => Promise<void> | void;
}

export function AssetInspector({
  asset,
  onClose,
  onChanged,
  className = "w-[460px]",
  project,
  onGenerate,
}: {
  asset: AssetView;
  onClose: () => void;
  onChanged: () => void;
  className?: string;
  project?: InspectorProject;
  /** 打开出图工作台，选中这个形象（场景 / 道具是 `main`）。和工作区资产那一步是同一个工作台 */
  onGenerate?: (lookId: string) => void;
}) {
  const isCharacter = asset.kind === "character";
  // 道具也有形象：协议铭牌的量产款和「星澜专属」款（2026-09-28）。场景没有
  const hasLooks = isCharacter || asset.kind === "prop";
  const [tab, setTab] = useState<Tab>("looks");
  const [refs, setRefs] = useState<AssetReference[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);

  useEffect(() => {
    setTab("looks");
    setMessage(null);
    setRefs(null);
    assetsReferences(asset.id).then(setRefs).catch(() => setRefs([]));
  }, [asset.id]);

  const run = async (fn: () => Promise<unknown>, ok?: string) => {
    setBusy(true);
    setMessage(null);
    try {
      await fn();
      onChanged();
      if (ok) setMessage({ ok: true, text: ok });
      return true;
    } catch (e) {
      // **错误必须显示出来。** 这一页所有动作都会改磁盘，静默失败等于让用户以为改动生效了。
      setMessage({ ok: false, text: String(e) });
      return false;
    } finally {
      setBusy(false);
    }
  };

  const tabs: { id: Tab; label: string }[] = [
    { id: "looks", label: hasLooks ? `形象 ${asset.look_views.length}` : "图片" },
    { id: "person", label: isCharacter ? "人物" : "信息" },
    { id: "refs", label: `引用 ${refs?.length ?? asset.reference_count}` },
  ];

  return (
    <aside
      className={`flex shrink-0 flex-col overflow-hidden ${className}`}
      style={{ borderLeft: "1px solid var(--studio-border)", background: "var(--studio-bg-elevated)" }}
    >
      <Header asset={asset} busy={busy} onClose={onClose} run={run} onDeleted={() => { onChanged(); onClose(); }} setMessage={setMessage} />

      <div className="flex shrink-0 gap-1 px-5" role="tablist" style={{ borderBottom: "1px solid var(--studio-border-subtle)" }}>
        {tabs.map((t) => (
          <button
            key={t.id}
            type="button"
            role="tab"
            aria-selected={tab === t.id}
            onClick={() => setTab(t.id)}
            className="-mb-px px-2.5 py-2 text-[12.5px]"
            style={{
              color: tab === t.id ? "var(--studio-text)" : "var(--studio-text-muted)",
              borderBottom: `2px solid ${tab === t.id ? "var(--studio-primary)" : "transparent"}`,
            }}
          >
            {t.label}
          </button>
        ))}
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {tab === "looks" && (hasLooks
          ? <LooksTab asset={asset} busy={busy} run={run} project={project} onGoPerson={() => setTab("person")} onChanged={onChanged} onGenerate={onGenerate} />
          : <CandidatesSection asset={asset} look={asset.look_views[0]} busy={busy} run={run} onChanged={onChanged} onGenerate={onGenerate} />)}
        {tab === "person" && <PersonTab asset={asset} busy={busy} run={run} />}
        {tab === "refs" && <RefsTab refs={refs} />}
      </div>

      {message && (
        <p className="shrink-0 px-5 py-2.5 text-[12px]" style={{ color: message.ok ? "var(--studio-done-fg)" : "var(--studio-failed-fg)", borderTop: "1px solid var(--studio-border-subtle)" }}>
          {message.text}
        </p>
      )}
    </aside>
  );
}

type Run = (fn: () => Promise<unknown>, ok?: string) => Promise<boolean>;

// ── 头部 ─────────────────────────────────────────────────────────────

function Header({ asset, busy, onClose, run, onDeleted, setMessage }: {
  asset: AssetView;
  busy: boolean;
  onClose: () => void;
  run: Run;
  onDeleted: () => void;
  setMessage: (m: { ok: boolean; text: string } | null) => void;
}) {
  const [menu, setMenu] = useState(false);
  /** 删除两步：第一次被引用拦下来，看过之后才允许强制删 */
  const [confirmDelete, setConfirmDelete] = useState<"none" | "arm" | "force">("none");
  const [merging, setMerging] = useState(false);
  const ref = useDismissable<HTMLDivElement>(menu, () => { setMenu(false); setConfirmDelete("none"); });

  const del = async () => {
    try {
      await assetsDelete(asset.scope, asset.id, confirmDelete === "force");
      onDeleted();
    } catch (e) {
      // 后端把「哪一集哪一镜」写在报错里了 —— 原样显示
      setMessage({ ok: false, text: String(e) });
      setConfirmDelete("force");
    }
  };

  return (
    <header className="flex shrink-0 items-start justify-between gap-2 px-5 pb-2.5 pt-5">
      <div className="min-w-0">
        <p className="studio-eyebrow truncate">{KIND_LABEL[asset.kind]} · {asset.scope_label}</p>
        <h2 className="mt-1 truncate text-[18px] font-semibold">{asset.name}</h2>
      </div>
      <div className="flex shrink-0 items-center gap-0.5">
        <IconButton
          label={asset.starred ? "取消星标" : "星标"}
          disabled={busy}
          onClick={() => void run(() => assetsUpdate({ scope: asset.scope, id: asset.id, starred: !asset.starred }))}
          color={asset.starred ? "var(--studio-primary)" : undefined}
        >
          {asset.starred ? "★" : "☆"}
        </IconButton>
        <div className="relative" ref={ref}>
          <IconButton label="更多操作" onClick={() => setMenu((v) => !v)}>⋯</IconButton>
          {menu && (
            <div className="studio-menu absolute right-0 top-full z-40 mt-1 w-[168px]">
              {asset.scope.kind !== "global" && (
                <MenuItem
                  icon={MenuIcon.series}
                  label="录入全局库"
                  hint="复制一份到共享池，原件保留"
                  onClick={() => {
                    setMenu(false);
                    void run(() => assetsPromote(asset.scope, asset.id), "已录入全局库（原件保留）");
                  }}
                />
              )}
              {asset.kind === "character" && (
                <MenuItem
                  icon={MenuIcon.canvas}
                  label="合并进另一个角色当形象"
                  hint="同一个人的另一个造型，先预览再合并"
                  onClick={() => {
                    setMenu(false);
                    setMerging(true);
                  }}
                />
              )}
              <div className="studio-menu-sep" />
              <MenuItem
                icon={MenuIcon.trash}
                danger
                label={confirmDelete === "none" ? "删除资产" : confirmDelete === "arm" ? "确认删除" : "强制删除（留下悬空引用）"}
                onClick={() => {
                  if (confirmDelete === "none") setConfirmDelete("arm");
                  else void del();
                }}
              />
            </div>
          )}
        </div>
        <IconButton label="关闭" onClick={onClose}>✕</IconButton>
      </div>
      {merging && (
        <MergeLookDialog
          source={asset}
          onClose={() => setMerging(false)}
          onMerged={() => {
            setMerging(false);
            setMessage({ ok: true, text: "已合并" });
            // 源角色已经删了：关掉详情、刷新列表
            onDeleted();
          }}
        />
      )}
    </header>
  );
}

// ── 形象页签 ─────────────────────────────────────────────────────────

function LooksTab({ asset, busy, run, project, onGoPerson, onChanged, onGenerate }: {
  onChanged: () => void;
  onGenerate?: (lookId: string) => void;
  asset: AssetView;
  busy: boolean;
  run: Run;
  project?: InspectorProject;
  onGoPerson: () => void;
}) {
  const looks = asset.look_views;
  const [selectedId, setSelectedId] = useState<string>(MAIN_LOOK);
  const [creating, setCreating] = useState(false);
  const look = looks.find((l) => l.id === selectedId) ?? looks[0];
  const projectDefault = project?.defaultLookId && looks.some((l) => l.id === project.defaultLookId) ? project.defaultLookId : MAIN_LOOK;

  useEffect(() => {
    // 换了资产、或者选中的形象被删了：回到主形象
    if (!looks.some((l) => l.id === selectedId)) setSelectedId(MAIN_LOOK);
  }, [looks, selectedId]);

  return (
    <div className="flex flex-col">
      {/* 形象条 */}
      <div className="flex gap-2 overflow-x-auto px-5 pb-3 pt-4">
        {looks.map((l) => (
          <button
            key={l.id}
            type="button"
            onClick={() => { setSelectedId(l.id); setCreating(false); }}
            className="flex w-[84px] shrink-0 flex-col overflow-hidden rounded-lg text-left"
            style={{
              background: "var(--studio-bg-inset)",
              boxShadow: `0 0 0 2px ${!creating && look?.id === l.id ? "var(--studio-primary)" : "transparent"}`,
            }}
          >
            <span className="relative block aspect-[3/4] w-full">
              {l.cover_url ? (
                <NsfwThumb path={l.cover_url} nsfw={l.nsfw} revealKey={`${asset.id}/${l.id}`} compact className="h-full w-full object-contain" />
              ) : (
                <span className="flex h-full items-center justify-center px-1 text-center text-[10px]" style={{ color: "var(--studio-text-muted)" }}>还没有图</span>
              )}
            </span>
            <span className="truncate px-1.5 pt-1 text-[11px]">{l.name}</span>
            <span className="flex gap-1 px-1.5 pb-1.5 font-mono text-[9px]" style={{ color: "var(--studio-text-muted)" }}>
              {l.is_main && <span>主</span>}
              {project && l.id === projectDefault && <span style={{ color: "var(--studio-primary)" }}>本项目默认</span>}
            </span>
          </button>
        ))}
        <button
          type="button"
          onClick={() => setCreating(true)}
          className="flex w-[84px] shrink-0 flex-col items-center justify-center gap-1 rounded-lg border border-dashed text-[11px]"
          style={{ borderColor: creating ? "var(--studio-primary)" : "var(--studio-border)", color: creating ? "var(--studio-primary)" : "var(--studio-text-2)" }}
        >
          <span className="text-[18px] leading-none">+</span>
          新形象
        </button>
      </div>

      {creating ? (
        <NewLookForm
          kind={asset.kind}
          busy={busy}
          onCancel={() => setCreating(false)}
          onCreate={async (name, description) => {
            let newId: string | null = null;
            const ok = await run(async () => {
              const before = new Set(asset.look_views.map((l) => l.id));
              const v = await assetsAddLook(asset.scope, asset.id, name, description);
              newId = v.look_views.find((l) => !before.has(l.id))?.id ?? null;
            }, `已加形象「${name}」—— 下一步上传或去出图`);
            if (ok) {
              setCreating(false);
              if (newId) setSelectedId(newId);
            }
          }}
        />
      ) : (
        look && (
          <LookEditor
            key={look.id}
            asset={asset}
            look={look}
            busy={busy}
            run={run}
            project={project}
            isProjectDefault={look.id === projectDefault}
            onGoPerson={onGoPerson}
            onRemoved={() => setSelectedId(MAIN_LOOK)}
            onChanged={onChanged}
            onGenerate={onGenerate}
          />
        )
      )}
    </div>
  );
}

function NewLookForm({ kind, busy, onCancel, onCreate }: { kind: AssetView["kind"]; busy: boolean; onCancel: () => void; onCreate: (name: string, description: string) => void }) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const prop = kind === "prop";
  return (
    <Section title="新形象">
      <Field label="名字">
        <input autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder={prop ? "比如：星澜专属、烧焦、打开状态" : "比如：盛唐男装、舞会礼服、短发"} className={INPUT} style={inputStyle} />
      </Field>
      <Field label="这个形象的描述" hint={prop ? "这一款长什么样：外形、颜色、材质、状态。用到它的镜头在分镜详情里换" : "只写和其他形象不一样的：发型、发色、服装、妆容、配饰。脸和身材写在「人物」里，所有形象共用"}>
        <textarea value={description} onChange={(e) => setDescription(e.target.value)} rows={4} placeholder={prop ? "椭圆形青色发光晶体，嵌在银色雕花外框里，由细银链挂在锁骨下方" : "高马尾束冠，深青色圆领官服，腰系黑色革带，黑色官靴，无配饰"} className={INPUT} style={inputStyle} />
      </Field>
      <div className="flex gap-2">
        <button type="button" disabled={busy || !name.trim()} onClick={() => onCreate(name.trim(), description)} className="rounded-lg px-3.5 py-1.5 text-[12px] font-medium disabled:opacity-40" style={{ background: "var(--studio-primary)", color: "var(--studio-on-primary)" }}>
          加这个形象
        </button>
        <button type="button" onClick={onCancel} className="rounded-lg px-3 py-1.5 text-[12px]" style={{ background: "var(--studio-bg-inset)", color: "var(--studio-text-2)" }}>
          取消
        </button>
      </div>
    </Section>
  );
}

function LookEditor({ asset, look, busy, run, project, isProjectDefault, onGoPerson, onRemoved, onChanged, onGenerate }: {
  onChanged: () => void;
  onGenerate?: (lookId: string) => void;
  asset: AssetView;
  look: LookView;
  busy: boolean;
  run: Run;
  project?: InspectorProject;
  isProjectDefault: boolean;
  onGoPerson: () => void;
  onRemoved: () => void;
}) {
  const [name, setName] = useState(look.name);
  const [description, setDescription] = useState(look.description);
  const [armRemove, setArmRemove] = useState(false);
  useEffect(() => {
    setName(look.name);
    setDescription(look.description);
  }, [look.name, look.description]);

  const update = (patch: { name?: string; description?: string; nsfw?: boolean }, ok?: string) =>
    run(() => assetsUpdateLook({ scope: asset.scope, id: asset.id, look_id: look.id, ...patch }), ok);
  // 「人物本身」只有角色有：道具的形象描述就是完整描述
  const noIdentity = asset.kind === "character" && !(asset.identity ?? "").trim();

  return (
    <>
      <Section>
        <div className="mb-3 flex items-center gap-2">
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            onBlur={() => {
              const v = name.trim();
              if (v && v !== look.name) void update({ name: v }, "已改名");
              else setName(look.name);
            }}
            aria-label="形象名字"
            className="min-w-0 flex-1 rounded-lg bg-transparent px-2 py-1 text-[14px] font-medium outline-none focus:bg-[var(--studio-bg-inset)]"
          />
          <Toggle label="NSFW" on={look.nsfw} disabled={busy} onChange={(on) => void update({ nsfw: on }, on ? "已标为 NSFW，缩略图默认模糊" : "已取消 NSFW")} />
        </div>

        {project && !isProjectDefault && (
          <button type="button" disabled={busy} onClick={() => void run(async () => project.onSetDefaultLook(look.id), `本项目默认用「${look.name}」—— 用到它的镜头要重新合成提示词`)}
            className="mb-3 w-full rounded-lg px-3 py-2 text-left text-[12px]" style={{ background: "var(--studio-bg-inset)", color: "var(--studio-primary)" }}>
            设为本项目的默认形象
            <span className="block text-[11px]" style={{ color: "var(--studio-text-muted)" }}>没单独换形象的镜头都用它；某一镜要换，在分镜的镜头详情里点</span>
          </button>
        )}

        {!look.is_main && noIdentity && (
          <p className="mb-3 rounded-lg px-3 py-2 text-[11.5px] leading-relaxed" style={{ background: "rgba(214,160,60,.12)", color: "var(--studio-running-fg)" }}>
            还没填「人物本身」：这个形象发给模型的只有衣服发型，脸每次会不一样。
            <button type="button" onClick={onGoPerson} className="ml-1 underline">去填</button>
          </p>
        )}

        <Field label={look.is_main ? "主形象描述" : "这个形象的描述"} hint={noIdentity && look.is_main ? "还没填人物本身时，这一段就是发给模型的完整描述" : asset.kind === "prop" ? "外形、颜色、材质、状态" : "发型、发色、服装、妆容、配饰"}>
          <textarea
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            onBlur={() => { if (description !== look.description) void update({ description }, "描述已保存"); }}
            rows={4}
            className={INPUT}
            style={inputStyle}
          />
        </Field>
        <details className="text-[11.5px]">
          <summary className="cursor-pointer select-none" style={{ color: "var(--studio-text-muted)" }}>发给模型的完整描述</summary>
          <p className="mt-1.5 whitespace-pre-wrap rounded-lg px-3 py-2 leading-relaxed" style={{ background: "var(--studio-bg-inset)", color: "var(--studio-text-2)" }}>
            {look.prompt || "（空）"}
          </p>
        </details>
      </Section>

      <CandidatesSection asset={asset} look={look} busy={busy} run={run} onChanged={onChanged} onGenerate={onGenerate} />

      {!look.is_main && (
        <Section>
          <button
            type="button"
            disabled={busy}
            onClick={async () => {
              if (!armRemove) return setArmRemove(true);
              if (await run(() => assetsRemoveLook(asset.scope, asset.id, look.id), `已删形象「${look.name}」`)) onRemoved();
            }}
            className="w-full rounded-lg px-3 py-2 text-[12px]"
            style={{ background: "var(--studio-bg-inset)", color: armRemove ? "var(--studio-danger)" : "var(--studio-text-2)" }}
          >
            {armRemove ? `确认删除「${look.name}」和它的 ${look.unit.image_variants.length} 张图` : "删除这个形象"}
          </button>
          {armRemove && (
            <p className="mt-1.5 text-[11px]" style={{ color: "var(--studio-text-muted)" }}>用到它的镜头会落回项目默认形象或主形象。</p>
          )}
        </Section>
      )}
    </>
  );
}

// ── 候选图 ───────────────────────────────────────────────────────────

function CandidatesSection({ asset, look, busy, run, onChanged, onGenerate }: {
  onGenerate?: (lookId: string) => void;
  asset: AssetView;
  look: LookView | undefined;
  busy: boolean;
  run: Run;
  onChanged: () => void;
}) {
  const [delArm, setDelArm] = useState<string | null>(null);
  if (!look) return null;
  const unit = look.unit;
  const aspect = asset.kind === "character" ? "aspect-[3/4]" : asset.kind === "scene" ? "aspect-video" : "aspect-square";
  return (
    <Section title={`候选图 ${unit.image_variants.length}`}>
      {unit.image_variants.length === 0 ? (
        <p className="mb-2 text-[12px]" style={{ color: "var(--studio-text-muted)" }}>
          还没有图。点下面「生成」出图，或者上传一张现成的。
        </p>
      ) : (
        <div className={`mb-2 grid gap-2 ${asset.kind === "scene" ? "grid-cols-2" : "grid-cols-4"}`}>
          {unit.image_variants.map((v, i) => {
            const on = v.id === (unit.selected_image_id ?? unit.image_variants[unit.image_variants.length - 1]?.id);
            return (
              <div key={v.id} className="group relative">
                <button
                  type="button"
                  title={on ? "正在用这张" : "用这张"}
                  disabled={busy}
                  onClick={() => void run(() => assetsSelectVariant(asset.scope, asset.id, v.id))}
                  className={`block w-full overflow-hidden rounded-lg ${aspect}`}
                  style={{ background: "var(--studio-bg-inset)", boxShadow: `0 0 0 2px ${on ? "var(--studio-primary)" : "transparent"}` }}
                >
                  <NsfwThumb path={look.image_urls[i]} nsfw={look.nsfw} revealKey={`${asset.id}/${look.id}`} compact className="h-full w-full object-contain" />
                </button>
                {on && (
                  <span className="pointer-events-none absolute bottom-1 left-1 rounded px-1 text-[9px]" style={{ background: "var(--studio-primary)", color: "var(--studio-on-primary)" }}>在用</span>
                )}
                <span className="absolute right-1 top-1 flex gap-0.5 opacity-0 transition-opacity group-hover:opacity-100" style={v.favorited ? { opacity: 1 } : undefined}>
                  <MiniButton
                    title={v.favorited ? "取消收藏" : "收藏（不会被自动清理）"}
                    color={v.favorited ? "var(--studio-primary)" : undefined}
                    onClick={() => void run(() => assetsFavoriteVariant(asset.scope, asset.id, v.id, !v.favorited))}
                  >★</MiniButton>
                  <MiniButton
                    title={delArm === v.id ? "再点一次删掉（连文件）" : "删除这张"}
                    color={delArm === v.id ? "var(--studio-danger)" : undefined}
                    onClick={() => {
                      if (delArm !== v.id) return setDelArm(v.id);
                      setDelArm(null);
                      void run(() => assetsRemoveVariant(asset.scope, asset.id, v.id));
                    }}
                  >{delArm === v.id ? "删?" : "✕"}</MiniButton>
                </span>
              </div>
            );
          })}
        </div>
      )}
      {onGenerate && (
        <button
          type="button"
          disabled={busy}
          onClick={() => onGenerate(look.id)}
          className="mb-2 flex w-full items-center justify-center gap-1.5 rounded-lg px-3 py-2.5 text-[12.5px] font-medium disabled:opacity-40"
          style={{ background: "var(--studio-primary)", color: "var(--studio-on-primary)" }}
        >
          ✦ {(asset.kind === "character" || (asset.kind === "prop" && asset.look_views.length > 1)) ? `生成「${look.name}」的图` : "生成图片"}
        </button>
      )}
      <UploadImageButton asset={asset} lookId={look.id} onDone={onChanged} label={(asset.kind === "character" || (asset.kind === "prop" && asset.look_views.length > 1)) ? `上传到「${look.name}」（点选或拖进来）` : undefined} />
      <p className="mt-1.5 text-[11px]" style={{ color: "var(--studio-text-muted)" }}>非收藏的候选图每个形象最多留 12 张，超出从最旧的删。</p>
    </Section>
  );
}

// ── 人物 / 信息页签 ───────────────────────────────────────────────────

function PersonTab({ asset, busy, run }: { asset: AssetView; busy: boolean; run: Run }) {
  const isCharacter = asset.kind === "character";
  const [name, setName] = useState(asset.name);
  const [identity, setIdentity] = useState(asset.identity ?? "");
  const [description, setDescription] = useState(asset.description);
  useEffect(() => {
    setName(asset.name);
    setIdentity(asset.identity ?? "");
    setDescription(asset.description);
  }, [asset.id, asset.name, asset.identity, asset.description]);
  const save = (patch: Parameters<typeof assetsUpdate>[0], ok: string) => void run(() => assetsUpdate(patch), ok);

  return (
    <>
      <Section>
        <Field label="名称">
          <input value={name} onChange={(e) => setName(e.target.value)} disabled={busy}
            onBlur={() => { const v = name.trim(); if (v && v !== asset.name) save({ scope: asset.scope, id: asset.id, name: v }, "已改名"); else setName(asset.name); }}
            className={INPUT} style={inputStyle} />
        </Field>
        {isCharacter ? (
          <Field label="人物本身" hint="性别、年龄、身高体型、脸型、五官、肤色 —— 所有形象共用，只写一次。发型服装写在各个形象里">
            <textarea value={identity} onChange={(e) => setIdentity(e.target.value)} rows={5}
              /* **占位必须一眼看出是例子。**「女，28 岁，身高 165 厘米……」原样摆在
                 那儿，读起来就是这个角色的数据 —— 2026-09-18 用户在阎王（男，50 岁）
                 身上看到它，来问「是不是有点矛盾？还是说这个内容是假的」。
                 他是对的：那一栏在盘上是空串。占位越像真内容越危险。 */
              placeholder="例：女，28 岁，身高 165 厘米，体型偏瘦，鹅蛋脸，杏眼，黑色瞳孔，皮肤白皙"
              onBlur={() => { if (identity !== (asset.identity ?? "")) save({ scope: asset.scope, id: asset.id, identity }, "人物本身已保存 —— 用到这个角色的镜头要重新合成提示词"); }}
              className={INPUT} style={inputStyle} />
          </Field>
        ) : (
          <Field label="描述" hint="外观、陈设、用途 —— 发给模型的就是这一段">
            <textarea value={description} onChange={(e) => setDescription(e.target.value)} rows={5}
              onBlur={() => { if (description !== asset.description) save({ scope: asset.scope, id: asset.id, description }, "描述已保存"); }}
              className={INPUT} style={inputStyle} />
          </Field>
        )}
      </Section>
      {isCharacter && <VoiceSection asset={asset} busy={busy} save={save} />}
      {isCharacter && asset.character && <AdvancedSection asset={asset} />}
    </>
  );
}

/**
 * 角色的 TTS 音色模板。
 *
 * # 为什么绑在角色上
 *
 * 同一个人跨集必须是同一把嗓子。绑在分镜上的话一集三十镜要配三十次，
 * 改一次要改三十处。个别例外（回忆里的少年版、电话失真）走台词行上的覆盖。
 *
 * # 和「这一句怎么说」不是一回事
 *
 * 这里是**哪把嗓子**（模板名，发给 TTS 引擎）。语气是逐句的，在镜头表里。
 *
 * # 为什么清单可能是空的
 *
 * 只有 GPT-SoVITS 读本机共享音色库；别的引擎的音色名在服务端，
 * 这边拿不到。空的时候让用户自己填 —— 给一份**这个引擎不认**的清单让人挑，
 * 挑了不报错、只是没效果，那比没有清单更糟。
 */
function VoiceSection({
  asset,
  busy,
  save,
}: {
  asset: AssetView;
  busy: boolean;
  save: (patch: Parameters<typeof assetsUpdate>[0], ok: string) => void;
}) {
  const current = asset.character?.voice_id ?? "";
  const [voice, setVoice] = useState(current);
  const [tpl, setTpl] = useState<VoiceTemplates | null>(null);
  useEffect(() => setVoice(current), [asset.id, current]);
  useEffect(() => {
    void cinemaVoiceTemplates()
      .then(setTpl)
      .catch((e) => setTpl({ engine: "", voices: [], hint: `拉音色列表失败：${e}` }));
  }, []);

  const currentH3 = asset.character?.h3_voice ?? "";
  const [h3, setH3] = useState(currentH3);
  useEffect(() => setH3(currentH3), [asset.id, currentH3]);
  const commitH3 = () => {
    const v = h3.trim();
    if (v === currentH3) return;
    // **整份替换**（assetsUpdate 的契约）—— 不带上现有的会把设定卡洗掉
    save(
      { scope: asset.scope, id: asset.id, character: { ...(asset.character ?? {}), h3_voice: v || null } as never },
      v ? "H3 声音设定已保存 —— 重新生成分镜才生效" : "已清掉 H3 声音设定",
    );
  };

  const commit = () => commitValue(voice);
  const commitValue = (raw: string) => {
    const v = raw.trim();
    if (v === current) return;
    // **整份替换，不是逐字段合并**（assetsUpdate 的契约）——
    // 不带上现有的那份，触发词、种子、LoRA 会一起被清空。
    save(
      { scope: asset.scope, id: asset.id, character: withVoiceId(asset.character, v) },
      v ? `音色已设为「${v}」—— 重新配音才生效` : "已清掉音色，配音时用全局设置里的",
    );
  };

  return (
    <Section>
      <Field label={VOICE_FIELD_LABEL} hint={VOICE_FIELD_HINT}>
        {/* **能列就列出来，别让人猜。** 原来无论哪个引擎都只写「填引擎认的
            音色名」—— 用户原话「我哪知道音色名是啥」。现在问引擎自己要。 */}
        {tpl && tpl.voices.length > 0 ? (
          <select
            value={tpl.voices.includes(voice) || voice === "" ? voice : "__other"}
            onChange={(e) => {
              if (e.target.value === "__other") return;
              setVoice(e.target.value);
              commitValue(e.target.value);
            }}
            disabled={busy}
            className={INPUT}
            style={inputStyle}
          >
            <option value="">{unsetOptionLabel(tpl.engine)}</option>
            {tpl.voices.map((t) => (
              <option key={t} value={t}>{t}</option>
            ))}
            {voice !== "" && !tpl.voices.includes(voice) && (
              <option value="__other">{voice}（当前引擎里没有这个音色）</option>
            )}
          </select>
        ) : (
          <input
            value={voice}
            onChange={(e) => setVoice(e.target.value)}
            onBlur={commit}
            disabled={busy}
            placeholder="不指定 = 配音时用全局音色"
            className={INPUT}
            style={inputStyle}
          />
        )}
        {/* **音色是哪来的、想换别的去哪换。** 只写引擎名读起来像「音色就这些」，
            而六个引擎其实都在，换一个就是另一批（2026-09-18 用户撞上）。 */}
        {tpl && (
          <p className="mt-1.5 text-[11px] leading-relaxed" style={{ color: "var(--studio-text-muted)" }}>
            {engineSourceNote(tpl.engine, tpl.voices.length)}
          </p>
        )}
        {/* 空列表时还要说**为什么**空、**该怎么办** —— 空框本身什么都没说 */}
        {tpl?.hint && (
          <p className="mt-1.5 text-[11px] leading-relaxed" style={{ color: "var(--studio-text-muted)" }}>
            {tpl.hint}
          </p>
        )}
      </Field>

      {/* **这一条管的是 H3 原声，和上面那个配音音色是两条不同的链路。**

          绑在角色上，是因为镜头表**一场一次调用**，模型看不到别的场 ——
          要求它「整集 voice 一致」等于要求它遵守一个它看不见的约束。
          实测 E04：李白第 2 镜「嗓音略带沙哑的中年男声」，第 8 镜变成
          「清亮略带醉意的青年男声」—— 不是醉意的差别，是换了个人
          （2026-09-19 Codex Astra 的审查指出）。 */}
      <Field
        label="H3 声音设定"
        hint="给 H3 看的中文描述：音高、音色、语速、口音。整集一份、跨场不变 —— 分镜时逐句只写这一句的变化（带醉意、压着嗓子）。和上面的「配音音色」是两条链路：这条管 H3 自己生成的原声，那条管 TTS 配音"
      >
        <input
          value={h3}
          onChange={(e) => setH3(e.target.value)}
          onBlur={commitH3}
          disabled={busy}
          placeholder="例：嗓音沙哑偏低的中年男声，语速偏慢"
          className={INPUT}
          style={inputStyle}
        />
      </Field>
    </Section>
  );
}

/** 角色参数。大多数人用不到，默认折叠 —— **不要为了「对齐参考项目」把它们删掉**，它们是我们独有的 */
function AdvancedSection({ asset }: { asset: AssetView }) {
  const c = asset.character!;
  const sheet = c.asset_sheet as { turnarounds?: unknown[]; expressions?: unknown[]; poses?: unknown[]; angles?: unknown[] } | null;
  const rows: [string, string][] = [
    ["触发词", c.trigger_words || "—"],
    ["固定种子", c.seed === null ? "未固定" : String(c.seed)],
    ["一致性方案", c.consistency_mode || "—"],
    ["LoRA", c.lora_model ? `${c.lora_model} @ ${c.lora_weight ?? 1}` : "—"],
    ["只有声音", c.disembodied ? "是（台词做成画外音）" : "否"],
  ];
  return (
    <Section>
      <details>
        <summary className="studio-eyebrow cursor-pointer select-none">高级</summary>
        <dl className="mt-2.5 flex flex-col gap-1">
          {rows.map(([k, v]) => (
            <div key={k} className="flex items-baseline gap-2 text-[12px]">
              <dt className="w-[70px] shrink-0" style={{ color: "var(--studio-text-muted)" }}>{k}</dt>
              <dd className="min-w-0 flex-1 truncate">{v}</dd>
            </div>
          ))}
        </dl>
        {sheet && (
          <p className="mt-2 rounded-lg px-2.5 py-2 text-[11px]" style={{ background: "var(--studio-bg-inset)" }}>
            设定卡：三视图 {sheet.turnarounds?.length ?? 0} · 表情 {sheet.expressions?.length ?? 0} · 姿势 {sheet.poses?.length ?? 0} · 机位 {sheet.angles?.length ?? 0}
          </p>
        )}
      </details>
    </Section>
  );
}

function RefsTab({ refs }: { refs: AssetReference[] | null }) {
  return (
    <Section>
      {refs === null && <p className="text-[12px]" style={{ color: "var(--studio-text-muted)" }}>正在查…</p>}
      {refs?.length === 0 && <p className="text-[12px]" style={{ color: "var(--studio-text-muted)" }}>还没有镜头用到它。</p>}
      {refs?.map((r) => (
        <div key={`${r.project_id}/${r.episode_id}/${r.shot_id}`} className="mb-1 rounded-lg px-2.5 py-1.5 text-[12px]" style={{ background: "var(--studio-bg-inset)" }}>
          {r.project_title} · {r.episode_title} · 第 {r.shot_num} 镜
        </div>
      ))}
    </Section>
  );
}

// ── 小零件 ───────────────────────────────────────────────────────────

function Section({ title, children }: { title?: string; children: React.ReactNode }) {
  return (
    <section className="px-5 py-4" style={{ borderTop: "1px solid var(--studio-border-subtle)" }}>
      {title && <p className="studio-eyebrow mb-2.5">{title}</p>}
      {children}
    </section>
  );
}

function Field({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  return (
    <label className="mb-3 block">
      <span className="studio-eyebrow mb-1 block">{label}</span>
      {children}
      {hint && <span className="mt-1 block text-[11px] leading-relaxed" style={{ color: "var(--studio-text-muted)" }}>{hint}</span>}
    </label>
  );
}

function Toggle({ label, on, disabled, onChange }: { label: string; on: boolean; disabled?: boolean; onChange: (on: boolean) => void }) {
  return (
    <button type="button" role="switch" aria-checked={on} disabled={disabled} onClick={() => onChange(!on)} className="flex shrink-0 items-center gap-1.5 text-[11.5px]" style={{ color: on ? "var(--studio-danger)" : "var(--studio-text-muted)" }}>
      <span className="relative inline-block h-4 w-7 rounded-full transition-colors" style={{ background: on ? "var(--studio-danger)" : "var(--studio-bg-inset)", boxShadow: "inset 0 0 0 1px var(--studio-border)" }}>
        <span className="absolute top-0.5 h-3 w-3 rounded-full bg-white transition-all" style={{ left: on ? 14 : 2 }} />
      </span>
      {label}
    </button>
  );
}

function IconButton({ label, onClick, disabled, color, children }: { label: string; onClick: () => void; disabled?: boolean; color?: string; children: React.ReactNode }) {
  return (
    <button type="button" aria-label={label} title={label} disabled={disabled} onClick={onClick} className="flex h-7 w-7 items-center justify-center rounded-md text-[14px] hover:bg-white/10" style={{ color: color ?? "var(--studio-text-2)" }}>
      {children}
    </button>
  );
}

function MiniButton({ title, onClick, color, children }: { title: string; onClick: () => void; color?: string; children: React.ReactNode }) {
  return (
    <button type="button" title={title} onClick={(e) => { e.stopPropagation(); onClick(); }} className="rounded px-1 text-[10px]" style={{ background: "var(--studio-bg-elevated)", color: color ?? "var(--studio-text-muted)" }}>
      {children}
    </button>
  );
}

const INPUT = "w-full resize-y rounded-lg px-3 py-2 text-[13px] outline-none";

const inputStyle: React.CSSProperties = {
  background: "var(--studio-bg-inset)",
  border: "1px solid var(--studio-border-subtle)",
  color: "var(--studio-text)",
};
