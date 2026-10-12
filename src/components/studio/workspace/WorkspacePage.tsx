/**
 * 工作区 —— 一切的起点。
 *
 * # 为什么必须先有这一页
 *
 * 上一版的分镜工作台**启动不了任何东西**：项目、分集、剧本、分镜全都
 * 只能去废弃的旧短剧面板里建。两个窗口一条流程，比改版前更乱。
 * 这一页把「新建」拿回来。
 *
 * # 系列 vs 独立项目
 *
 * 不是两种存储，是同一个东西的两种用法：独立项目就是只有一集的项目，
 * 界面上不显示分集列表。两者走同一条流水线。
 */
import { useCallback, useEffect, useRef, useState } from "react";
import {
  cinemaCreateProject,
  cinemaDeleteProject,
  cinemaGenerateCover,
  cinemaUploadCover,
  cinemaWorkspace,
  type WorkspaceItem,
} from "../../../api/cinema";
import { MediaThumb } from "../MediaThumb";
import { coverGradient } from "../coverGradient";
import { useDismissable } from "../useDismissable";
import { MenuIcon, MenuItem } from "../Menu";
import { runTask } from "../toastStore";
import { useStudioChanged } from "../useStudioChanged";

const STATUS_LABEL: Record<WorkspaceItem["status"], string> = {
  draft: "草稿",
  active: "进行中",
  done: "已完成",
};

type Filter = "all" | WorkspaceItem["status"];

export function WorkspacePage({
  onOpen,
  onOpenCanvas,
}: {
  onOpen: (projectId: string) => void;
  onOpenCanvas?: (projectId: string) => void;
}) {
  const [items, setItems] = useState<WorkspaceItem[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState<Filter>("all");
  const [query, setQuery] = useState("");
  const [creating, setCreating] = useState<null | "series" | "standalone">(null);
  const [menuOpen, setMenuOpen] = useState(false);
  const [deleting, setDeleting] = useState<WorkspaceItem | null>(null);
  const newMenuRef = useDismissable<HTMLDivElement>(menuOpen, () => setMenuOpen(false));
  /** 正在出封面的项目。出完从这里拿掉 —— 卡片上据此显示「封面生成中」、菜单项变灰 */
  const [covering, setCovering] = useState<Set<string>>(new Set());

  const reload = useCallback(async () => {
    try {
      setItems(await cinemaWorkspace());
      setError(null);
    } catch (e) {
      setItems([]);
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);
  // Chat 建了项目、加了分集：不切页也刷出来
  useStudioChanged(() => void reload(), { kinds: ["project", "episode"] });

  // 走右下角通知条：一两分钟，切走页面也会报结果
  /** 用户自己的图当封面。读成 data URL 交给后端，后端转 PNG 存进项目目录 */
  const uploadCover = async (item: WorkspaceItem, file: File) => {
    try {
      const dataUrl = await new Promise<string>((ok, no) => {
        const r = new FileReader();
        r.onload = () => ok(String(r.result));
        r.onerror = () => no(new Error(`读不出「${file.name}」`));
        r.readAsDataURL(file);
      });
      await runTask(
        {
          title: `正在换「${item.title}」的封面…`,
          body: file.name,
          success: () => ({ title: "封面换好了", body: item.title }),
          failure: "封面没换成",
        },
        () => cinemaUploadCover(item.id, dataUrl),
      );
      await reload();
    } catch {
      // 失败原文已经在通知条里
    }
  };

  const generateCover = async (item: WorkspaceItem) => {
    setCovering((s) => new Set(s).add(item.id));
    try {
      await runTask(
        {
          title: `正在给「${item.title}」出封面…`,
          body: "读剧本写提示词，再出图，约一两分钟。可以先去别的地方。",
          success: () => ({ title: "封面生成好了", body: item.title }),
          failure: "封面没生成出来",
        },
        () => cinemaGenerateCover(item.id),
      );
      await reload();
    } catch {
      // 失败原文已经在通知条里
    } finally {
      setCovering((s) => {
        const n = new Set(s);
        n.delete(item.id);
        return n;
      });
    }
  };

  const all = items ?? [];
  const counts = {
    all: all.length,
    done: all.filter((i) => i.status === "done").length,
    active: all.filter((i) => i.status === "active").length,
    draft: all.filter((i) => i.status === "draft").length,
  };
  const q = query.trim().toLowerCase();
  const shown = all.filter(
    (i) =>
      (filter === "all" || i.status === filter) &&
      (!q || i.title.toLowerCase().includes(q) || i.genre.toLowerCase().includes(q)),
  );
  const series = shown.filter((i) => i.kind === "series");
  const standalone = shown.filter((i) => i.kind !== "series");

  return (
    <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
      <header
        className="studio-banner flex shrink-0 items-start justify-between px-7 py-5"
        style={{ borderBottom: "1px solid var(--studio-border-subtle)" }}
        data-tauri-drag-region
      >
        <div className="flex flex-col gap-1">
          <span className="studio-eyebrow" style={{ letterSpacing: "0.2em" }}>
            WORKSPACE
            <span style={{ color: "var(--studio-primary)", fontWeight: 600 }}>
              {" · "}系列与项目
            </span>
          </span>
          <h1 className="studio-display text-[2rem]">我的工作区</h1>
        </div>
        <div className="relative" ref={newMenuRef}>
          <button
            type="button"
            onClick={() => setMenuOpen((v) => !v)}
            className="rounded-full px-4 py-2 text-[13px] font-medium"
            style={{ background: "var(--studio-primary)", color: "var(--studio-on-primary)" }}
          >
            + 新建 ▾
          </button>
          {menuOpen && (
            <>
              <div className="studio-menu absolute right-0 z-20 mt-1.5 w-[196px]">
                <MenuItem
                  icon={MenuIcon.series}
                  label="新建系列"
                  hint="多集，共用角色与风格"
                  onClick={() => {
                    setMenuOpen(false);
                    setCreating("series");
                  }}
                />
                <MenuItem
                  icon={MenuIcon.file}
                  label="新建独立项目"
                  hint="单集，从剧本开始"
                  onClick={() => {
                    setMenuOpen(false);
                    setCreating("standalone");
                  }}
                />
              </div>
            </>
          )}
        </div>
      </header>

      <div className="flex shrink-0 flex-wrap items-center gap-2.5 px-7 py-3.5">
        <div className="flex items-center gap-1 rounded-full p-0.5" style={chip}>
          {(["all", "done", "active", "draft"] as Filter[]).map((f) => (
            <button
              key={f}
              type="button"
              onClick={() => setFilter(f)}
              className="rounded-full px-3 py-1.5 text-[12px]"
              style={{
                background: filter === f ? "var(--studio-bg-elevated)" : "transparent",
                color: filter === f ? "var(--studio-text)" : "var(--studio-text-muted)",
              }}
            >
              {f === "all" ? "全部" : STATUS_LABEL[f]}{" "}
              <span className="font-mono text-[0.6rem] opacity-60">{counts[f]}</span>
            </button>
          ))}
        </div>
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="搜索项目 / 系列…"
          className="min-w-[200px] flex-1 rounded-full px-4 py-1.5 text-[12px] outline-none"
          style={{ ...chip, color: "var(--studio-text)" }}
        />
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto px-7 pb-10">
        {error && (
          <p className="py-4 text-[13px]" style={{ color: "var(--studio-danger)" }}>
            读取失败：{error}
          </p>
        )}
        {items === null && <Hint>读取中…</Hint>}
        {items !== null && all.length === 0 && !error && (
          <div
            className="mt-10 rounded-2xl px-8 py-14 text-center"
            style={{ background: "var(--studio-bg-inset)" }}
          >
            <p className="studio-eyebrow mb-3" style={{ letterSpacing: "0.2em" }}>
              SERIES · PROJECTS
            </p>
            <p className="text-[15px]">还没有项目</p>
            <p className="mt-2 text-[12px]" style={{ color: "var(--studio-text-muted)" }}>
              「新建系列」适合多集短剧，角色和风格跨集共用；「新建独立项目」是单集，直接从剧本开始
            </p>
          </div>
        )}

        <Group
          title="系列"
          items={series}
          onOpen={onOpen}
          onOpenCanvas={onOpenCanvas}
          onDelete={setDeleting}
          onGenerateCover={generateCover}
          onUploadCover={uploadCover}
          covering={covering}
        />
        <Group
          title="独立项目"
          items={standalone}
          onOpen={onOpen}
          onOpenCanvas={onOpenCanvas}
          onDelete={setDeleting}
          onGenerateCover={generateCover}
          onUploadCover={uploadCover}
          covering={covering}
          onNew={() => setCreating("standalone")}
        />
      </div>

      {deleting && (
        <DeleteProjectDialog
          item={deleting}
          onClose={() => setDeleting(null)}
          onDeleted={async () => {
            setDeleting(null);
            await reload();
          }}
        />
      )}

      {creating && (
        <NewProjectDialog
          kind={creating}
          onClose={() => setCreating(null)}
          onCreated={async (id) => {
            setCreating(null);
            await reload();
            onOpen(id);
          }}
        />
      )}
    </div>
  );
}

function Group({
  title,
  items,
  onOpen,
  onOpenCanvas,
  onDelete,
  onGenerateCover,
  onUploadCover,
  covering,
  onNew,
}: {
  title: string;
  items: WorkspaceItem[];
  onOpen: (id: string) => void;
  onOpenCanvas?: (id: string) => void;
  onDelete: (item: WorkspaceItem) => void;
  onGenerateCover: (item: WorkspaceItem) => void;
  onUploadCover: (item: WorkspaceItem, file: File) => void;
  covering: Set<string>;
  onNew?: () => void;
}) {
  if (items.length === 0 && !onNew) return null;
  return (
    <section className="mb-7">
      <div className="mb-3 flex items-baseline gap-2">
        <h2 className="studio-display text-[15px]">{title}</h2>
        <span className="font-mono text-[0.6rem]" style={{ color: "var(--studio-text-muted)" }}>
          {items.length}
        </span>
      </div>
      <div className="grid grid-cols-[repeat(auto-fill,minmax(210px,1fr))] gap-3">
        {items.map((it) => (
          <ProjectCard
            key={it.id}
            item={it}
            onOpen={() => onOpen(it.id)}
            onOpenCanvas={onOpenCanvas ? () => onOpenCanvas(it.id) : undefined}
            onDelete={() => onDelete(it)}
            onGenerateCover={() => onGenerateCover(it)}
            onUploadCover={(f) => onUploadCover(it, f)}
            coverBusy={covering.has(it.id)}
          />
        ))}
        {onNew && (
          <button
            type="button"
            onClick={onNew}
            className="flex aspect-[4/3] flex-col items-center justify-center gap-1.5 rounded-xl"
            style={{ border: "1px dashed var(--studio-border)" }}
          >
            <span className="text-[20px]" style={{ color: "var(--studio-text-muted)" }}>
              +
            </span>
            <span className="text-[12px]">新建独立项目</span>
            <span className="text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
              从剧本开始
            </span>
          </button>
        )}
      </div>
    </section>
  );
}

function ProjectCard({
  item,
  onOpen,
  onOpenCanvas,
  onDelete,
  onGenerateCover,
  onUploadCover,
  coverBusy,
}: {
  item: WorkspaceItem;
  onOpen: () => void;
  onOpenCanvas?: () => void;
  onDelete: () => void;
  onGenerateCover: () => void;
  onUploadCover: (file: File) => void;
  coverBusy: boolean;
}) {
  const [menu, setMenu] = useState(false);
  const coverInput = useRef<HTMLInputElement | null>(null);
  const menuRef = useDismissable<HTMLDivElement>(menu, () => setMenu(false));
  const stamp = new Date(item.updated_at * 1000);
  return (
    <div className="studio-tile relative flex flex-col" data-menu-open={menu}>
      {/* 「上传封面」用的选文件框，藏着；PNG / JPG / WebP */}
      <input
        ref={coverInput}
        type="file"
        accept="image/png,image/jpeg,image/webp"
        className="hidden"
        onChange={(e) => {
          const f = e.target.files?.[0];
          e.target.value = "";
          if (f) onUploadCover(f);
        }}
      />
      <button
        type="button"
        onClick={onOpen}
        className="flex flex-1 flex-col text-left"
      >
        <div
          className="studio-tile-media studio-grain relative flex aspect-[4/3] w-full items-end p-3.5"
          style={
            item.cover_image
              ? { background: "var(--studio-bg-inset)" }
              : { background: coverGradient(item.id) }
          }
        >
          {item.cover_image && (
            <MediaThumb
              path={item.cover_image}
              className="absolute inset-0 h-full w-full object-cover"
            />
          )}
          <span
            className="studio-pill absolute left-3 top-3"
            style={{
              color:
                item.status === "done"
                  ? "var(--studio-primary)"
                  : item.status === "active"
                    ? "var(--studio-running-fg)"
                    : "var(--studio-text-2)",
            }}
          >
            {STATUS_LABEL[item.status]}
          </span>
          {coverBusy && (
            <span className="studio-pill absolute right-3 top-3" style={{ color: "var(--studio-running-fg)" }}>
              封面生成中…
            </span>
          )}
          <span className="relative z-10 flex flex-col">
            {/* 封面用的是专用深色渐变（`--studio-cover-*`），两套主题都偏暗，
                所以这里的白字是安全的。 */}
            <span className="studio-display text-[17px] leading-tight text-white drop-shadow">
              {item.title}
            </span>
            <span
              className="mt-0.5 font-mono text-[0.55rem]"
              style={{ color: "rgba(255,255,255,.62)" }}
            >
              {item.shot_count} 镜
            </span>
          </span>
        </div>
      </button>

      <div className="flex items-center justify-between px-3 py-2.5">
        <span
          className="truncate font-mono text-[0.55rem]"
          style={{ color: "var(--studio-text-muted)" }}
        >
          {STATUS_LABEL[item.status]} · {stamp.toLocaleDateString("zh-CN")} ·{" "}
          {item.episode_count} 集
          {item.rendered_count > 0 ? ` · ${item.rendered_count} 已出片` : ""}
        </span>
        {/* 卡片操作菜单。**删除只有这一个入口** —— 之前压根没有，
            想删一个项目只能去 Finder 里翻目录。 */}
        <div className="relative shrink-0" ref={menuRef}>
          <button
            type="button"
            aria-label="更多操作"
            aria-expanded={menu}
            onClick={(e) => {
              e.stopPropagation();
              setMenu((v) => !v);
            }}
            className="rounded-md px-1.5 py-0.5 text-[13px] leading-none transition-colors hover:bg-white/10"
            style={{ color: "var(--studio-text-muted)" }}
          >
            ⋮
          </button>
          {/* 关闭靠 `useDismissable`（document 上听 pointerdown），
              **不插任何遮罩元素** —— 遮罩会盖住卡片，把 hover 吃掉，
              于是抬升动画反复横跳、菜单也关不掉。 */}
          {menu && (
            <div className="studio-menu absolute bottom-full right-0 z-40 mb-1.5 w-[140px]">
              <MenuItem
                icon={MenuIcon.open}
                label="打开"
                onClick={() => {
                  setMenu(false);
                  onOpen();
                }}
              />
              {onOpenCanvas && (
                <MenuItem
                  icon={MenuIcon.canvas}
                  label="打开画布"
                  onClick={() => {
                    setMenu(false);
                    onOpenCanvas();
                  }}
                />
              )}
              <MenuItem
                icon={MenuIcon.image}
                label={coverBusy ? "封面生成中…" : item.cover_image ? "重新生成封面" : "生成封面"}
                onClick={() => {
                  setMenu(false);
                  if (!coverBusy) onGenerateCover();
                }}
              />
              <MenuItem
                icon={MenuIcon.image}
                label="上传封面"
                onClick={() => {
                  setMenu(false);
                  if (!coverBusy) coverInput.current?.click();
                }}
              />
              <div className="studio-menu-sep" />
              <MenuItem
                icon={MenuIcon.trash}
                label="删除"
                danger
                onClick={() => {
                  setMenu(false);
                  onDelete();
                }}
              />
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function NewProjectDialog({
  kind,
  onClose,
  onCreated,
}: {
  kind: "series" | "standalone";
  onClose: () => void;
  onCreated: (id: string) => void | Promise<void>;
}) {
  const [title, setTitle] = useState("");
  const [genre, setGenre] = useState("");
  const [aspect, setAspect] = useState("9:16");
  /** 有没有剧本 —— 决定第 1 步在不在流程里。 */
  const [contentMode, setContentMode] = useState<"scripted" | "freeform">("scripted");
  /** 新建镜头默认走首帧还是直接出片。 */
  const [shotMode, setShotMode] = useState<"first_frame" | "reference">("first_frame");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    if (!title.trim()) {
      setError("先起个名字");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const p = await cinemaCreateProject({
        title: title.trim(),
        genre: genre.trim(),
        aspect_ratio: aspect,
        description: "",
        kind,
        content_mode: contentMode,
        default_shot_mode: shotMode,
      });
      await onCreated(p.id);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      className="studio-scrim fixed inset-0 z-50 flex items-center justify-center"
    >
      <div
        className="studio-modal max-h-[86vh] w-[420px] max-w-[92vw] overflow-y-auto"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="px-6 pb-4 pt-5" style={{ borderBottom: "1px solid var(--studio-border-subtle)" }}>
          <h2 className="studio-display text-[17px]">
            {kind === "series" ? "新建系列" : "新建独立项目"}
          </h2>
          <p className="mt-0.5 text-[12px]" style={{ color: "var(--studio-text-muted)" }}>
            {kind === "series"
              ? "多集共用角色与风格，适合连载短剧"
              : "单集，从剧本开始。之后想加集也可以"}
          </p>
        </header>
        <div className="flex flex-col gap-4 px-6 py-5">
          <Field label="名称">
            <input
              autoFocus
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && void submit()}
              placeholder={kind === "series" ? "这部剧叫什么" : "这个项目叫什么"}
              className="w-full rounded-lg px-3 py-2 text-[13px] outline-none"
              style={field}
            />
          </Field>
          <Field label="题材（可选）">
            <input
              value={genre}
              onChange={(e) => setGenre(e.target.value)}
              placeholder="都市逆袭 / 悬疑 / 玄幻…"
              className="w-full rounded-lg px-3 py-2 text-[13px] outline-none"
              style={field}
            />
          </Field>
          {/* **这两项决定后面整条流程长什么样**，所以放在建项目时问，
              而不是埋进设置里。抄自 LumenX 的新建系列对话框。 */}
          <Field label="内容模式">
            <Choice
              value={contentMode}
              onChange={(v) => setContentMode(v as "scripted" | "freeform")}
              options={[
                {
                  v: "scripted",
                  title: "我有剧本",
                  hint: "先写/粘剧本 → 提取角色场景 → 拆分镜",
                  tag: "推荐",
                },
                {
                  v: "freeform",
                  title: "我直接创作",
                  hint: "跳过剧本，直接管理资产、手写分镜提示词",
                },
              ]}
            />
          </Field>
          <Field label="视觉控制">
            <Choice
              value={shotMode}
              onChange={(v) => setShotMode(v as "first_frame" | "reference")}
              options={[
                {
                  v: "first_frame",
                  title: "画面优先",
                  hint: "新建镜头先出首帧，看过再出视频，画面最可控",
                  tag: "推荐",
                },
                {
                  v: "reference",
                  title: "节奏优先",
                  hint: "用 @ 引用的资产参考图直接出视频，出片更快",
                },
              ]}
            />
          </Field>
          <Field label="画幅">
            <div className="flex gap-1">
              {["9:16", "16:9"].map((a) => (
                <button
                  key={a}
                  type="button"
                  onClick={() => setAspect(a)}
                  className="rounded-full px-3 py-1.5 text-[12px]"
                  style={{
                    background: aspect === a ? "var(--studio-primary)" : "var(--studio-bg-inset)",
                    color: aspect === a ? "var(--studio-on-primary)" : "var(--studio-text-2)",
                  }}
                >
                  {a === "9:16" ? "竖屏 9:16" : "横屏 16:9"}
                </button>
              ))}
            </div>
          </Field>
          {error && (
            <p className="text-[12px]" style={{ color: "var(--studio-danger)" }}>
              {error}
            </p>
          )}
        </div>
        <footer
          className="flex justify-end gap-2 px-6 py-4"
          style={{ borderTop: "1px solid var(--studio-border-subtle)" }}
        >
          <button
            type="button"
            onClick={onClose}
            className="rounded-lg px-4 py-2 text-[13px]"
            style={{ background: "var(--studio-bg-inset)", color: "var(--studio-text-2)" }}
          >
            取消
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={() => void submit()}
            className="rounded-lg px-4 py-2 text-[13px] font-medium"
            style={{ background: "var(--studio-primary)", color: "var(--studio-on-primary)", opacity: busy ? 0.6 : 1 }}
          >
            {busy ? "创建中…" : "创建并开始"}
          </button>
        </footer>
      </div>
    </div>
  );
}

/** 两选一的卡片。选项之间的差别要写在卡片里 —— 只给两个名字，没人分得清。 */
function Choice({
  value,
  onChange,
  options,
}: {
  value: string;
  onChange: (v: string) => void;
  options: { v: string; title: string; hint: string; tag?: string }[];
}) {
  return (
    <div className="grid grid-cols-2 gap-2">
      {options.map((o) => {
        const on = value === o.v;
        return (
          <button
            key={o.v}
            type="button"
            onClick={() => onChange(o.v)}
            className="relative rounded-xl px-3 py-2.5 text-left"
            style={{
              background: on
                ? "color-mix(in srgb, var(--studio-primary) 12%, transparent)"
                : "var(--studio-bg-inset)",
              boxShadow: `0 0 0 ${on ? 1.5 : 1}px ${
                on ? "var(--studio-primary)" : "var(--studio-border-subtle)"
              }`,
            }}
          >
            <span className="block text-[12.5px] font-medium">{o.title}</span>
            <span
              className="mt-0.5 block text-[11px] leading-snug"
              style={{ color: "var(--studio-text-muted)" }}
            >
              {o.hint}
            </span>
            {o.tag && (
              <span
                className="absolute right-2 top-2 rounded px-1 py-0.5 text-[9px]"
                style={{
                  background: "color-mix(in srgb, var(--studio-primary) 20%, transparent)",
                  color: "var(--studio-primary)",
                }}
              >
                {o.tag}
              </span>
            )}
          </button>
        );
      })}
    </div>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div>
      <label className="studio-eyebrow mb-1 block">{label}</label>
      {children}
    </div>
  );
}

/**
 * 删项目的确认框。
 *
 * **先问后端「里面有什么」再问用户。** 一个按钮直接删掉几十分钟的
 * 生成结果，是这一页里唯一真正危险的操作 —— 摘要必须具体到
 * 「几集几镜几个已出片」，不能只说「确定删除吗」。
 */
function DeleteProjectDialog({
  item,
  onClose,
  onDeleted,
}: {
  item: WorkspaceItem;
  onClose: () => void;
  onDeleted: () => void | Promise<void>;
}) {
  const [summary, setSummary] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    cinemaDeleteProject(item.id, false)
      .then(setSummary)
      .catch((e) => setError(String(e)));
  }, [item.id]);

  return (
    <div
      className="studio-scrim fixed inset-0 z-50 flex items-center justify-center"
    >
      <div
        className="studio-modal max-h-[86vh] w-[400px] max-w-[92vw] overflow-y-auto"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="px-6 pb-2 pt-5">
          <h2 className="studio-display text-[17px]">删除项目</h2>
          <p className="mt-2 text-[13px]" style={{ color: "var(--studio-text-2)" }}>
            {summary ?? "正在数里面有什么…"}
          </p>
          <p className="mt-2 text-[12px]" style={{ color: "var(--studio-text-muted)" }}>
            连同它的剧本、分镜、关键帧、视频和资产一起删掉，不可恢复。
          </p>
          {error && (
            <p className="mt-2 text-[12px]" style={{ color: "var(--studio-danger)" }}>
              {error}
            </p>
          )}
        </div>
        <footer className="flex justify-end gap-2 px-6 py-4">
          <button
            type="button"
            onClick={onClose}
            className="rounded-lg px-4 py-2 text-[13px]"
            style={{ background: "var(--studio-bg-inset)", color: "var(--studio-text-2)" }}
          >
            不删了
          </button>
          <button
            type="button"
            disabled={busy || !summary}
            onClick={async () => {
              setBusy(true);
              setError(null);
              try {
                await cinemaDeleteProject(item.id, true);
                await onDeleted();
              } catch (e) {
                setError(String(e));
                setBusy(false);
              }
            }}
            className="rounded-lg px-4 py-2 text-[13px] font-medium"
            style={{
              background: "var(--studio-danger)",
              color: "var(--studio-on-danger)",
              opacity: busy || !summary ? 0.5 : 1,
            }}
          >
            {busy ? "删除中…" : "确认删除"}
          </button>
        </footer>
      </div>
    </div>
  );
}

function Hint({ children }: { children: React.ReactNode }) {
  return (
    <p className="py-4 text-[12px]" style={{ color: "var(--studio-text-muted)" }}>
      {children}
    </p>
  );
}

const chip: React.CSSProperties = {
  background: "var(--studio-bg-inset)",
  border: "1px solid var(--studio-border-subtle)",
};
const field: React.CSSProperties = {
  background: "var(--studio-bg-inset)",
  border: "1px solid var(--studio-border-subtle)",
  color: "var(--studio-text)",
};

export default WorkspacePage;
