/**
 * 「待归属」：各集里还没进系列池的资产，逐条决定去向。
 *
 * - **加入系列**：系列里没有同名的就原样搬过去（id 不变，镜头不用改）；
 *   有同名的就并进那一个（镜头引用改指过去）。
 * - **只留本集**：一次性的路人、只出现一次的场景。打个标记，不再提醒。
 * - 顶上一个「全部加入系列」：同名自动合并，按集号顺序处理。
 *
 * 用行不用卡片：这里要做的是**逐条决定**，一行一个、按钮在同一行，
 * 比卡片网格好扫。
 */
import { useCallback, useEffect, useState } from "react";
import {
  seriesAssetsAdopt,
  seriesAssetsAdoptAll,
  seriesAssetsInbox,
  seriesAssetsKeepEpisode,
  type InboxItem,
  type SeriesInbox,
} from "../../../api/seriesAssets";
import { useStudioChanged } from "../useStudioChanged";
import { adoptAllNote, adoptNote } from "./seriesCopy";

const KIND_LABEL: Record<string, string> = { character: "角色", scene: "场景", prop: "道具" };

export function SeriesInboxPanel({
  projectId,
  seriesNames,
  onChanged,
  onOpen,
}: {
  projectId: string;
  /** 系列里现有资产的 id → 名字，用来说「会并进『陆川』」 */
  seriesNames: Record<string, string>;
  onChanged: () => void;
  onOpen: (item: InboxItem) => void;
}) {
  const [inbox, setInbox] = useState<SeriesInbox | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [note, setNote] = useState("");
  const [error, setError] = useState("");
  const [showKept, setShowKept] = useState(false);

  const reload = useCallback(() => {
    void seriesAssetsInbox(projectId)
      .then(setInbox)
      .catch((e) => setError(String(e)));
  }, [projectId]);
  useEffect(reload, [reload]);
  useStudioChanged(reload, { kinds: ["assets"], projectId });

  const run = async (key: string, f: () => Promise<string>) => {
    setBusy(key);
    setError("");
    try {
      setNote(await f());
      reload();
      onChanged();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  if (!inbox || (inbox.pending.length === 0 && inbox.kept.length === 0 && !note && !error)) return null;

  const row = (it: InboxItem, kept: boolean) => {
    const a = it.asset;
    const key = `${it.episode_id}/${a.id}`;
    const into = it.match_id ? seriesNames[it.match_id] : null;
    return (
      <div
        key={key}
        className="flex items-center gap-3 rounded-lg px-2 py-1.5"
        style={{ borderBottom: "1px solid var(--studio-border-subtle)" }}
      >
        <button
          type="button"
          onClick={() => onOpen(it)}
          className="h-9 w-9 shrink-0 overflow-hidden rounded-md"
          style={{ background: "var(--studio-bg-inset)" }}
          title="打开看看"
        >
          {a.cover_url && <img src={a.cover_url} alt="" className="h-full w-full object-contain" />}
        </button>
        <div className="min-w-0 flex-1">
          <p className="truncate text-[12.5px]">
            {a.name}
            <span className="ml-1.5 text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
              {KIND_LABEL[a.kind] ?? a.kind}
            </span>
          </p>
          <p className="truncate text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
            E{String(it.episode_num).padStart(2, "0")} {it.episode_title.replace(/^E\d+\s*[-·—:：]?\s*/, "")}
            {!kept && (into ? ` · 系列里已有，会并进「${into}」` : " · 系列里没有，原样加进去")}
            {a.reference_count ? ` · ${a.reference_count} 镜在用` : ""}
          </p>
        </div>
        {kept ? (
          <button
            type="button"
            disabled={!!busy}
            onClick={() =>
              void run(key, async () => {
                await seriesAssetsKeepEpisode(projectId, it.episode_id, a.id, false);
                return `「${a.name}」回到待归属`;
              })
            }
            className="shrink-0 rounded-lg px-2.5 py-1 text-[11.5px] disabled:opacity-50"
            style={{ color: "var(--studio-text-2)" }}
          >
            撤销
          </button>
        ) : (
          <>
            <button
              type="button"
              disabled={!!busy}
              onClick={() =>
                void run(key, async () => {
                  await seriesAssetsKeepEpisode(projectId, it.episode_id, a.id, true);
                  return `「${a.name}」只留在 E${String(it.episode_num).padStart(2, "0")}`;
                })
              }
              className="shrink-0 rounded-lg px-2.5 py-1 text-[11.5px] disabled:opacity-50"
              style={{ color: "var(--studio-text-2)" }}
            >
              只留本集
            </button>
            <button
              type="button"
              disabled={!!busy}
              onClick={() => void run(key, async () => adoptNote(await seriesAssetsAdopt(projectId, it.episode_id, a.id)))}
              className="shrink-0 rounded-lg px-2.5 py-1 text-[11.5px] font-medium disabled:opacity-50"
              style={{ background: "var(--studio-bg-inset)", color: "var(--studio-text)" }}
            >
              {busy === key ? "…" : into ? "并入系列" : "加入系列"}
            </button>
          </>
        )}
      </div>
    );
  };

  return (
    <section
      className="mb-7 rounded-xl px-4 py-3"
      style={{ background: "var(--studio-pending-bg)", border: "1px solid var(--studio-pending-bd)" }}
    >
      <div className="mb-2 flex items-center gap-2">
        <h2 className="text-[13px] font-medium" style={{ color: "var(--studio-pending-fg)" }}>
          待归属 {inbox.pending.length}
        </h2>
        <span className="min-w-0 flex-1 truncate text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
          在各集里、还没进系列的 —— 其它集用不到，下一集提取也认不出
        </span>
        {inbox.pending.length > 0 && (
          <button
            type="button"
            disabled={!!busy}
            onClick={() => void run("all", async () => adoptAllNote(await seriesAssetsAdoptAll(projectId)))}
            className="shrink-0 rounded-lg px-3 py-1.5 text-[12px] font-medium disabled:opacity-50"
            style={{ background: "var(--studio-primary)", color: "var(--studio-on-primary)" }}
            title="系列里没有同名的原样加进去，有同名的并进已有的那一个。改之前自动备份"
          >
            {busy === "all" ? "处理中…" : "全部加入系列（同名自动合并）"}
          </button>
        )}
      </div>
      {note && (
        <p className="mb-1.5 text-[11.5px]" style={{ color: "var(--studio-text-2)" }}>
          {note}
        </p>
      )}
      {error && (
        <p className="mb-1.5 text-[11.5px]" style={{ color: "var(--studio-failed-fg)" }}>
          {error}
        </p>
      )}
      <div className="flex flex-col">{inbox.pending.map((it) => row(it, false))}</div>
      {inbox.kept.length > 0 && (
        <div className="mt-2">
          <button
            type="button"
            onClick={() => setShowKept((v) => !v)}
            className="text-[11.5px]"
            style={{ color: "var(--studio-text-muted)" }}
          >
            {showKept ? "▾" : "▸"} 只留本集的 {inbox.kept.length} 个
          </button>
          {showKept && <div className="mt-1 flex flex-col">{inbox.kept.map((it) => row(it, true))}</div>}
        </div>
      )}
    </section>
  );
}
