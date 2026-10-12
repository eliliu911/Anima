/**
 * 资产库「公共素材」页签：现成的背景图，挑一个加进全局库或某个项目。
 *
 * **只读、不直接进资产列表** —— 300 多组背景混进「场景」里，自己建的场景就找不到了。
 * 加入之后才是普通场景资产（每个时段一张变体，封面白天），分镜、@ 引用都照常用。
 */
import { useEffect, useMemo, useState } from "react";
import { publicLibraryImport, publicLibraryList, type PublicLibrary, type PublicScene } from "../../../api/publicLibrary";
import type { Scope } from "../../../api/assets";
import { cinemaListProjects, type CinemaProject } from "../../../api/cinema";
import { MediaThumb } from "../MediaThumb";
import { categoryTabs, defaultAssetName, filterScenes, variantLabel } from "./publicScenes";

export function PublicLibraryView({
  query,
  onImported,
  fixedScope,
}: {
  query: string;
  onImported?: () => void;
  /** 从项目里打开时锁定加到这个项目，不给选 */
  fixedScope?: { scope: Scope; label: string };
}) {
  const [lib, setLib] = useState<PublicLibrary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [category, setCategory] = useState("all");
  const [selectedId, setSelectedId] = useState<string | null>(null);

  useEffect(() => {
    publicLibraryList().then(setLib).catch((e) => setError(String(e)));
  }, []);

  const scenes = lib?.scenes ?? [];
  const tabs = useMemo(() => categoryTabs(scenes), [scenes]);
  const shown = useMemo(() => filterScenes(scenes, category, query), [scenes, category, query]);
  const selected = scenes.find((s) => s.id === selectedId) ?? null;

  if (error) return <p className="py-6 text-[13px]" style={{ color: "var(--studio-danger)" }}>读取公共素材库失败：{error}</p>;
  if (!lib) return <p className="py-6 text-[13px]" style={{ color: "var(--studio-text-muted)" }}>正在读取…</p>;
  if (!scenes.length) {
    return (
      <div className="mt-10 rounded-2xl px-8 py-14 text-center" style={{ background: "var(--studio-bg-inset)" }}>
        <p className="text-[15px]">公共素材库是空的</p>
        <p className="mt-2 text-[12px]" style={{ color: "var(--studio-text-muted)" }}>
          素材放在 <code>{lib.root}</code>，每个合集一个文件夹
        </p>
      </div>
    );
  }

  return (
    <div className="flex min-h-0 min-w-0 flex-1">
      <div className="min-w-0 flex-1 overflow-y-auto pb-10">
        <div className="mb-3 flex flex-wrap items-center gap-1.5">
          {tabs.map((t) => (
            <button
              key={t.id}
              type="button"
              onClick={() => setCategory(t.id)}
              aria-pressed={category === t.id}
              className="rounded-full px-3 py-1 text-[12px]"
              style={{
                background: category === t.id ? "var(--studio-bg-elevated)" : "var(--studio-bg-inset)",
                border: "1px solid var(--studio-border-subtle)",
                color: category === t.id ? "var(--studio-text)" : "var(--studio-text-muted)",
              }}
            >
              {t.label} <span className="font-mono text-[0.6rem] opacity-60">{t.count}</span>
            </button>
          ))}
        </div>
        <p className="mb-3 text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
          {lib.collections.map((c) => `${c.name}${c.author ? `（${c.author}）` : ""}${c.license ? `：${c.license}` : ""}`).join("；")}
        </p>
        {shown.length === 0 ? (
          <p className="py-6 text-[13px]" style={{ color: "var(--studio-text-muted)" }}>没有对得上的素材，换个分类或搜索词</p>
        ) : (
          <div className="grid gap-3" style={{ gridTemplateColumns: "repeat(auto-fill, minmax(230px, 1fr))" }}>
            {shown.map((s) => (
              <button
                key={s.id}
                type="button"
                onClick={() => setSelectedId(s.id)}
                className="studio-tile flex flex-col text-left"
                data-active={s.id === selectedId}
              >
                <div className="studio-tile-media relative flex aspect-video w-full items-center justify-center" style={{ background: "var(--studio-bg-inset)" }}>
                  <MediaThumb path={s.variants[0]?.url} className="h-full w-full object-contain" />
                  {s.variants.length > 1 && (
                    <span className="studio-pill absolute bottom-1.5 right-1.5 font-mono">{s.variants.length} 张</span>
                  )}
                </div>
                <div className="flex min-w-0 items-baseline gap-1.5 px-2.5 py-2">
                  <p className="min-w-0 truncate text-[12px]" title={s.name}>{s.name}</p>
                  <span className="ml-auto shrink-0 text-[0.6rem]" style={{ color: "var(--studio-text-muted)" }}>{s.category}</span>
                </div>
              </button>
            ))}
          </div>
        )}
      </div>
      {selected && <ImportPanel key={selected.id} scene={selected} fixedScope={fixedScope} onClose={() => setSelectedId(null)} onImported={onImported} />}
    </div>
  );
}

function ImportPanel({
  scene,
  fixedScope,
  onClose,
  onImported,
}: {
  scene: PublicScene;
  fixedScope?: { scope: Scope; label: string };
  onClose: () => void;
  onImported?: () => void;
}) {
  const [at, setAt] = useState(0);
  const [name, setName] = useState(defaultAssetName(scene));
  const [projects, setProjects] = useState<CinemaProject[]>([]);
  /** `global` 或项目 id */
  const [target, setTarget] = useState("global");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);

  useEffect(() => {
    if (!fixedScope) cinemaListProjects().then(setProjects).catch(() => setProjects([]));
  }, [fixedScope]);

  const scope: Scope = fixedScope?.scope ?? (target === "global" ? { kind: "global" } : { kind: "project", project_id: target });
  const where = fixedScope?.label ?? (target === "global" ? "全局资产库" : `「${projects.find((p) => p.id === target)?.title ?? ""}」`);

  const add = async () => {
    setBusy(true);
    setMessage(null);
    try {
      await publicLibraryImport(scene.id, scope, name);
      setMessage({ ok: true, text: `已加入${where}，是一个场景资产（${scene.variants.length} 张图，封面${variantLabel(scene.variants[0]?.label ?? "")}）` });
      onImported?.();
    } catch (e) {
      setMessage({ ok: false, text: String(e) });
    } finally {
      setBusy(false);
    }
  };

  const v = scene.variants[at] ?? scene.variants[0];
  return (
    <aside
      className="ml-4 flex w-[420px] shrink-0 flex-col gap-3 overflow-y-auto rounded-xl p-4"
      style={{ border: "1px solid var(--studio-border)", background: "var(--studio-bg-elevated)" }}
    >
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0">
          <p className="truncate text-[14px]">{scene.name}</p>
          <p className="text-[11px]" style={{ color: "var(--studio-text-muted)" }}>{scene.category} · {scene.variants.length} 张</p>
        </div>
        <button type="button" onClick={onClose} className="shrink-0 px-1 text-[16px]" style={{ color: "var(--studio-text-muted)" }} aria-label="关闭">
          ×
        </button>
      </div>
      <div className="aspect-video w-full overflow-hidden rounded-lg" style={{ background: "var(--studio-bg-inset)" }}>
        <MediaThumb path={v?.url} className="h-full w-full object-contain" />
      </div>
      {scene.variants.length > 1 && (
        <div className="flex flex-wrap gap-1.5">
          {scene.variants.map((x, i) => (
            <button
              key={x.path}
              type="button"
              onClick={() => setAt(i)}
              aria-pressed={i === at}
              className="rounded-full px-2.5 py-1 text-[11px]"
              style={{
                background: i === at ? "var(--studio-primary)" : "var(--studio-bg-inset)",
                color: i === at ? "var(--studio-on-primary)" : "var(--studio-text-2)",
              }}
            >
              {variantLabel(x.label)}
            </button>
          ))}
        </div>
      )}

      <label className="flex flex-col gap-1 text-[12px]">
        <span style={{ color: "var(--studio-text-2)" }}>资产名（剧本里怎么叫就写什么，@ 引用和分镜用它）</span>
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          className="rounded-lg px-3 py-2 text-[13px] outline-none"
          style={{ background: "var(--studio-bg-inset)", border: "1px solid var(--studio-border-subtle)", color: "var(--studio-text)" }}
        />
      </label>
      {!fixedScope && (
        <label className="flex flex-col gap-1 text-[12px]">
          <span style={{ color: "var(--studio-text-2)" }}>加到哪</span>
          <select
            value={target}
            onChange={(e) => setTarget(e.target.value)}
            className="rounded-lg px-3 py-2 text-[13px] outline-none"
            style={{ background: "var(--studio-bg-inset)", border: "1px solid var(--studio-border-subtle)", color: "var(--studio-text)" }}
          >
            <option value="global">全局资产库（所有项目都能用）</option>
            {projects.map((p) => (
              <option key={p.id} value={p.id}>
                {p.title}
              </option>
            ))}
          </select>
        </label>
      )}
      <button
        type="button"
        disabled={busy || !name.trim()}
        onClick={() => void add()}
        className="rounded-lg px-4 py-2 text-[13px] font-medium disabled:opacity-50"
        style={{ background: "var(--studio-primary)", color: "var(--studio-on-primary)" }}
      >
        {busy ? "正在加入…" : `加入${where}`}
      </button>
      <p className="text-[11px] leading-relaxed" style={{ color: "var(--studio-text-muted)" }}>
        所有时段一起加进去，每个时段一张图，封面是{variantLabel(scene.variants[0]?.label ?? "")}；之后在资产详情里可以换封面。
      </p>
      {message && (
        <p className="text-[12px]" style={{ color: message.ok ? "var(--studio-text-2)" : "var(--studio-failed-fg)" }}>
          {message.text}
        </p>
      )}
    </aside>
  );
}
