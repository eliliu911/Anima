/**
 * 提取结果的确认弹窗。
 *
 * # 为什么是「对比」而不是「清单」
 *
 * 用户真正要判断的是**系列的名单会变成什么样**，不是「模型说了哪几个词」。
 * 所以每一类给的是 `系列当前 → 应用后`，名字用一排 chip 摊开 ——
 * 一眼看完，不用读描述。描述进了 chip 的悬浮提示。
 *
 * # 默认进系列，同名并入（2026-09-25 用户定的）
 *
 * 每个 chip 点一下换一个去向：**进系列**（默认）→ **只留本集** → **不要**。
 *
 * **顺带出现的默认「不要」**（2026-09-30 用户定的）：模型标了 `incidental` 的龙套（邻居甲、打手乙）和
 * 一次性的普通物件（铁棍、电锅）不建资产，镜头里按文字描述出 —— EP02 一次进了 20 个这样的资产，删都没地方删。
 * 系列里已有同类同名的标「已有」—— 应用后是并进去，不会多长一个。
 * 以前全部落本集、等「对齐」，对齐一直没做，于是每集的角色各是各的。
 */
import { useEffect, useMemo, useState } from "react";
import {
  cinemaImportEntities,
  type ExtractedEntity,
  type ImportSummary,
} from "../../../api/cinema";
import { assetsList } from "../../../api/assets";
import { characterBase, entityKey, nextPick, seriesCount, type Pick } from "../series/seriesCopy";

const KINDS = [
  { key: "character" as const, label: "角色" },
  { key: "scene" as const, label: "场景" },
  { key: "prop" as const, label: "道具" },
];

const PICK_LABEL: Record<Pick, string> = { series: "进系列", episode: "只留本集", skip: "不要" };

/** 打开弹窗时每一条的去向：顺带出现的（`incidental`）默认不要，其余进系列 */
export function defaultPick(e: { incidental?: boolean }): Pick {
  return e.incidental ? "skip" : "series";
}

export function ExtractDialog({
  projectId,
  episodeId,
  entities,
  onClose,
  onImported,
  allowEpisode = true,
}: {
  projectId: string;
  episodeId: string;
  entities: ExtractedEntity[];
  onClose: () => void;
  onImported: (s: ImportSummary) => void;
  /** 系列页里提取（从设定文档）没有「只留本集」这一档 —— 那里不属于任何一集 */
  allowEpisode?: boolean;
}) {
  /** 每个实体的去向。默认进系列（已有的也是，后端并进去不会重复建）；顺带出现的默认不要 */
  const [picks, setPicks] = useState<Record<string, Pick>>(() =>
    Object.fromEntries(entities.map((e) => [entityKey(e), defaultPick(e)])),
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  /** 系列里现有的（`类型/名字`）。null = 读不到 —— 那就不显示对比数字，**不要显示 0**。 */
  const [existing, setExisting] = useState<Set<string> | null>(null);

  useEffect(() => {
    let alive = true;
    assetsList({ kind: "project", project_id: projectId })
      .then((list) => alive && setExisting(new Set(list.map((a) => entityKey(a)))))
      .catch(() => alive && setExisting(null));
    return () => {
      alive = false;
    };
  }, [projectId]);

  const groups = useMemo(() => {
    const g: Record<string, ExtractedEntity[]> = { character: [], scene: [], prop: [] };
    for (const e of entities) g[e.kind]?.push(e);
    return g;
  }, [entities]);

  const cycle = (e: ExtractedEntity) =>
    setPicks((p) => ({ ...p, [entityKey(e)]: nextPick(p[entityKey(e)] ?? "series", allowEpisode) }));

  const chosen = entities.filter((e) => picks[entityKey(e)] !== "skip");

  const apply = async () => {
    setBusy(true);
    setError(null);
    try {
      const episodeOnly = chosen
        .filter((e) => picks[entityKey(e)] === "episode")
        .map(entityKey);
      onImported(await cinemaImportEntities(projectId, episodeId, chosen, episodeOnly));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="studio-scrim fixed inset-0 z-50 flex items-center justify-center">
      <div
        className="studio-modal flex max-h-[86vh] w-[520px] max-w-[92vw] flex-col overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="shrink-0 px-6 pb-4 pt-5">
          <h2 className="studio-display text-[17px]">确认实体提取结果</h2>
          <p className="mt-1 text-[12px]" style={{ color: "var(--studio-text-muted)" }}>
            默认进系列，其它集也能用；系列里已有的会并进去，不会重复建。
            标「顺带」的（龙套、一次性的普通物件）默认不要，镜头里按文字描述出。
            {allowEpisode ? "点名字切换：进系列 → 只留本集 → 不要。" : "点名字可以去掉不要的。"}
            同一个人的几套造型（「星澜（战斗款）」）会合成一个角色加几个形象。
          </p>
        </header>

        {/* 实体一多列表就很长：只让中间这段滚，标题和「应用」按钮永远在窗口里 */}
        <div className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-6 pb-1">
          {KINDS.map(({ key, label }) => {
            const items = groups[key] ?? [];
            if (items.length === 0) return null;
            const count = existing ? seriesCount(items, picks, existing, key) : null;
            return (
              <section key={key}>
                <div className="mb-2 flex items-baseline justify-between">
                  <span className="text-[12.5px] font-medium">{label}</span>
                  {count && (
                    <span
                      className="font-mono text-[11px] tabular-nums"
                      style={{ color: "var(--studio-text-muted)" }}
                      title="系列里现在几个 → 应用后几个（已有的并进去，不加一）"
                    >
                      系列 {count.before} → {count.after}
                    </span>
                  )}
                </div>
                <div className="flex flex-wrap gap-1.5">
                  {items.map((e) => {
                    const k = entityKey(e);
                    const pick = picks[k] ?? "series";
                    // 造型按人算：系列里有「星澜」，「星澜（战斗款）」就是并进去加形象
                    const had =
                      existing?.has(e.kind === "character" ? `character/${characterBase(e)}` : k) ?? false;
                    return (
                      <button
                        key={k}
                        type="button"
                        onClick={() => cycle(e)}
                        title={`${PICK_LABEL[pick]}${had && pick === "series" ? "（系列里已有，并进去）" : ""} · ${e.description || e.persona || "模型没给描述"}`}
                        aria-label={`${e.name}：${PICK_LABEL[pick]}`}
                        className="rounded-lg px-2.5 py-1 text-[12px]"
                        style={{
                          background:
                            pick === "series"
                              ? "color-mix(in srgb, var(--studio-primary) 14%, transparent)"
                              : "var(--studio-bg-inset)",
                          color: pick === "skip" ? "var(--studio-text-muted)" : "var(--studio-text)",
                          textDecoration: pick === "skip" ? "line-through" : "none",
                          boxShadow:
                            pick === "series"
                              ? "0 0 0 1px color-mix(in srgb, var(--studio-primary) 40%, transparent)"
                              : pick === "episode"
                                ? "inset 0 0 0 1px var(--studio-border)"
                                : "none",
                          borderStyle: pick === "episode" ? "dashed" : undefined,
                        }}
                      >
                        {e.name}
                        {pick === "series" && had && (
                          <span className="ml-1 text-[10px]" style={{ color: "var(--studio-text-muted)" }}>
                            已有
                          </span>
                        )}
                        {pick === "episode" && (
                          <span className="ml-1 text-[10px]" style={{ color: "var(--studio-text-muted)" }}>
                            只留本集
                          </span>
                        )}
                        {e.disembodied && (
                          <span
                            className="ml-1 text-[10px]"
                            style={{ color: "var(--studio-text-muted)" }}
                          >
                            画外音
                          </span>
                        )}
                        {e.incidental && (
                          <span className="ml-1 text-[10px]" style={{ color: "var(--studio-text-muted)" }}>
                            顺带
                          </span>
                        )}
                      </button>
                    );
                  })}
                </div>
                {/* **标记不能只存在于数据里。** 分镜会因为这个标记少切好几个
                    镜头，用户在这儿看不到的话，成片短了也不知道是为什么。 */}
                {items.some((e) => e.disembodied && picks[entityKey(e)] !== "skip") && (
                  <p className="mt-1.5 text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
                    带「画外音」的角色只有声音、没有身体，分镜不会单独给它镜头 ——
                    它的台词会压在对手戏角色的画面上。标错了可以在资产里改。
                  </p>
                )}
              </section>
            );
          })}
        </div>

        {error && (
          <p className="shrink-0 px-6 pt-3 text-[11.5px]" style={{ color: "var(--studio-failed-fg)" }}>
            {error}
          </p>
        )}

        <footer className="mt-4 flex shrink-0 items-center justify-end gap-2 px-6 pb-5 pt-3">
          <button
            type="button"
            onClick={onClose}
            className="rounded-lg px-3.5 py-2 text-[12px]"
            style={{ color: "var(--studio-text-2)" }}
          >
            放弃
          </button>
          <button
            type="button"
            disabled={busy || chosen.length === 0}
            onClick={() => void apply()}
            className="rounded-lg px-4 py-2 text-[12.5px] font-medium"
            style={{
              background: chosen.length ? "var(--studio-primary)" : "var(--studio-bg-inset)",
              color: chosen.length ? "var(--studio-on-primary)" : "var(--studio-text-muted)",
              opacity: busy ? 0.7 : 1,
            }}
          >
            {busy ? "应用中…" : "应用"}
          </button>
        </footer>
      </div>
    </div>
  );
}
