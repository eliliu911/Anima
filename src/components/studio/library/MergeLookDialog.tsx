import { useEffect, useMemo, useState } from "react";
import {
  assetsList,
  assetsMergeApply,
  assetsMergePlan,
  assetsMergeSuggest,
  sameScope,
  type AssetView,
  type MergePlan,
  type MergeReq,
} from "../../../api/assets";
import { NsfwThumb } from "./NsfwThumb";

/**
 * 「合并进另一个角色当形象」。
 *
 * 形象功能之前同一个人的不同造型只能建成两个资产（「林婉清（现代）」「林婉清（盛唐）」）。
 * 合并会改分镜里的出场角色、参考图、台词说话人，**所以一定先预览**：列出每一镜改什么、有没有同框冲突，
 * 看过之后才能点合并。后端改之前自动备份到 `backups/merge-<时间>/`。
 */
export function MergeLookDialog({ source, onClose, onMerged }: { source: AssetView; onClose: () => void; onMerged: () => void }) {
  const [all, setAll] = useState<AssetView[]>([]);
  const [targetKey, setTargetKey] = useState("");
  const [newName, setNewName] = useState("");
  const [mainLookName, setMainLookName] = useState("");
  const [lookName, setLookName] = useState(source.name);
  const [plan, setPlan] = useState<MergePlan | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    assetsList().then(setAll).catch((e) => setError(String(e)));
  }, []);

  // 候选目标：别的角色。同名前缀的排前面（最可能是同一个人）
  const candidates = useMemo(() => {
    const base = source.name.split(/[（(]/)[0].trim();
    return all
      .filter((a) => a.kind === "character" && !(a.id === source.id && sameScope(a.scope, source.scope)))
      .sort((a, b) => Number(b.name.startsWith(base)) - Number(a.name.startsWith(base)) || a.name.localeCompare(b.name, "zh"));
  }, [all, source]);
  const keyOf = (a: AssetView) => `${JSON.stringify(a.scope)}|${a.id}`;
  const target = candidates.find((a) => keyOf(a) === targetKey) ?? null;

  // 选了目标就猜名字
  useEffect(() => {
    setPlan(null);
    if (!target) return;
    assetsMergeSuggest(source.name, target.name).then(([n, m, l]) => {
      setNewName(n);
      setMainLookName(m);
      setLookName(l);
    }).catch(() => void 0);
  }, [targetKey]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (!targetKey && candidates.length) setTargetKey(keyOf(candidates[0]));
  }, [candidates]); // eslint-disable-line react-hooks/exhaustive-deps

  const req = (): MergeReq | null =>
    target && { from_scope: source.scope, from_id: source.id, to_scope: target.scope, to_id: target.id, new_name: newName, main_look_name: mainLookName, look_name: lookName };

  const preview = async () => {
    const r = req();
    if (!r) return;
    setBusy(true);
    setError(null);
    try {
      setPlan(await assetsMergePlan(r));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const apply = async () => {
    const r = req();
    if (!r) return;
    setBusy(true);
    setError(null);
    try {
      await assetsMergeApply(r);
      onMerged();
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  };

  // 改了任何输入，预览就过期
  const edit = (f: (v: string) => void) => (e: React.ChangeEvent<HTMLInputElement>) => {
    f(e.target.value);
    setPlan(null);
  };

  return (
    <div className="studio-scrim fixed inset-0 z-50 flex items-center justify-center">
      <div className="studio-modal flex max-h-[86vh] w-[620px] max-w-[95vw] flex-col overflow-hidden" onClick={(e) => e.stopPropagation()}>
        <header className="shrink-0 px-6 pb-3 pt-5" style={{ borderBottom: "1px solid var(--studio-border-subtle)" }}>
          <p className="studio-eyebrow">合并成形象</p>
          <h2 className="mt-1 text-[17px] font-semibold">把「{source.name}」并进另一个角色</h2>
          <p className="mt-1 text-[12px]" style={{ color: "var(--studio-text-muted)" }}>
            同一个人的不同造型以前只能建成两个角色。合并后是一个角色的两个形象，分镜里用到它的镜头一起改过去。
          </p>
        </header>

        <div className="min-h-0 flex-1 overflow-y-auto px-6 py-4">
          <p className="studio-eyebrow mb-2">并进哪个角色</p>
          <div className="mb-4 grid grid-cols-4 gap-2">
            {candidates.map((a) => (
              <button key={keyOf(a)} type="button" onClick={() => setTargetKey(keyOf(a))} className="flex flex-col overflow-hidden rounded-lg text-left"
                style={{ background: "var(--studio-bg-inset)", boxShadow: `0 0 0 2px ${keyOf(a) === targetKey ? "var(--studio-primary)" : "transparent"}` }}>
                <span className="block aspect-[3/4] w-full">
                  {a.cover_url && <NsfwThumb path={a.cover_url} nsfw={Boolean(a.look_views?.[0]?.nsfw)} revealKey={`${a.id}/main`} compact className="h-full w-full object-contain" />}
                </span>
                <span className="truncate px-1.5 pt-1 text-[11.5px]">{a.name}</span>
                <span className="truncate px-1.5 pb-1.5 text-[10px]" style={{ color: "var(--studio-text-muted)" }}>{a.scope_label}</span>
              </button>
            ))}
            {candidates.length === 0 && <p className="col-span-4 text-[12px]" style={{ color: "var(--studio-text-muted)" }}>没有别的角色可以并进去。</p>}
          </div>

          {target && (
            <div className="mb-4 grid grid-cols-3 gap-2 text-[12px]">
              <Labeled label="合并后角色名">
                <input value={newName} onChange={edit(setNewName)} placeholder={target.name} className={INPUT} style={INPUT_STYLE} />
              </Labeled>
              <Labeled label={`「${target.name}」原来那套叫`}>
                <input value={mainLookName} onChange={edit(setMainLookName)} placeholder="主形象" className={INPUT} style={INPUT_STYLE} />
              </Labeled>
              <Labeled label={`「${source.name}」变成形象叫`}>
                <input value={lookName} onChange={edit(setLookName)} className={INPUT} style={INPUT_STYLE} />
              </Labeled>
            </div>
          )}

          {plan && (
            <div className="rounded-xl px-4 py-3 text-[12px]" style={{ background: "var(--studio-bg-inset)" }}>
              <p className="mb-2">
                合并后：<b>{plan.new_name}</b> = 主形象「{plan.main_look_name || "主形象"}」+ 形象「{plan.look_name}」（带过去 {plan.images} 张图）；「{plan.from_name}」从资产库删掉。
              </p>
              {plan.conflicts.length > 0 && (
                <div className="mb-2" style={{ color: "var(--studio-failed-fg)" }}>
                  <p className="font-medium">有同框冲突，不能合并：</p>
                  {plan.conflicts.map((c) => <p key={c}>· {c}</p>)}
                </div>
              )}
              <p className="studio-eyebrow mb-1 mt-2">会改 {plan.shots.length} 个镜头</p>
              <div className="flex max-h-[220px] flex-col gap-1 overflow-y-auto">
                {plan.shots.map((s) => (
                  <p key={`${s.project_title}/${s.episode_title}/${s.shot_num}`} style={{ color: "var(--studio-text-2)" }}>
                    {s.project_title} · {s.episode_title} · 第 {s.shot_num} 镜：{s.changes.join("；")}
                  </p>
                ))}
                {plan.shots.length === 0 && <p style={{ color: "var(--studio-text-muted)" }}>没有镜头用到这两个角色。</p>}
              </div>
              <p className="mt-2" style={{ color: "var(--studio-text-muted)" }}>{plan.note} 改之前会自动备份。</p>
            </div>
          )}
          {error && <p className="mt-3 text-[12px]" style={{ color: "var(--studio-failed-fg)" }}>{error}</p>}
        </div>

        <footer className="flex shrink-0 justify-end gap-2 px-6 py-3" style={{ borderTop: "1px solid var(--studio-border-subtle)" }}>
          <button type="button" onClick={onClose} className="rounded-lg px-3.5 py-1.5 text-[12px]" style={{ background: "var(--studio-bg-inset)", color: "var(--studio-text-2)" }}>取消</button>
          {!plan ? (
            <button type="button" disabled={busy || !target || !lookName.trim()} onClick={() => void preview()} className="rounded-lg px-3.5 py-1.5 text-[12px] font-medium disabled:opacity-40" style={{ background: "var(--studio-primary)", color: "var(--studio-on-primary)" }}>
              {busy ? "查看中…" : "预览会改什么"}
            </button>
          ) : (
            <button type="button" disabled={busy || plan.conflicts.length > 0} onClick={() => void apply()} className="rounded-lg px-3.5 py-1.5 text-[12px] font-medium disabled:opacity-40" style={{ background: "var(--studio-primary)", color: "var(--studio-on-primary)" }}>
              {busy ? "合并中…" : `确认合并（改 ${plan.shots.length} 个镜头）`}
            </button>
          )}
        </footer>
      </div>
    </div>
  );
}

function Labeled({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="flex min-w-0 flex-col gap-1">
      <span className="truncate text-[11px]" style={{ color: "var(--studio-text-muted)" }}>{label}</span>
      {children}
    </label>
  );
}

const INPUT = "w-full min-w-0 rounded-lg px-2.5 py-1.5 text-[12.5px] outline-none";
const INPUT_STYLE = { background: "var(--studio-bg-elevated)", border: "1px solid var(--studio-border-subtle)", color: "var(--studio-text)" } as const;
