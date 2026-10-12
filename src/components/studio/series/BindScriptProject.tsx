/**
 * 系列还没绑剧本编辑器项目、按名字也认不出来时，让人选一次。选完存成绑定。
 */
import { useState } from "react";
import type { ScriptProject } from "../../../api/scriptLibrary";
import { cinemaLinkScriptProject } from "../../../api/cinema";

export function BindScriptProject({
  cinemaProjectId,
  seriesTitle,
  projects,
  onBound,
}: {
  cinemaProjectId: string;
  seriesTitle: string;
  projects: ScriptProject[];
  onBound: (p: ScriptProject) => void;
}) {
  const [pick, setPick] = useState("");
  const [error, setError] = useState("");
  return (
    <div className="flex flex-wrap items-center gap-2 text-[12px]">
      <span>系列「{seriesTitle}」还没绑定剧本编辑器里的项目，选一个：</span>
      <select
        value={pick}
        onChange={(e) => setPick(e.target.value)}
        className="rounded-md px-2 py-1"
        style={{ background: "var(--studio-bg-inset)", color: "var(--studio-text)" }}
      >
        <option value="">选择项目…</option>
        {projects.map((p) => (
          <option key={p.id} value={p.id}>
            {p.title}
          </option>
        ))}
      </select>
      <button
        type="button"
        disabled={!pick}
        onClick={async () => {
          setError("");
          try {
            await cinemaLinkScriptProject(cinemaProjectId, pick);
            const p = projects.find((x) => x.id === pick);
            if (p) onBound(p);
          } catch (e) {
            setError(String(e));
          }
        }}
        className="rounded-md px-2.5 py-1 font-medium disabled:opacity-40"
        style={{ background: "var(--studio-primary)", color: "var(--studio-on-primary)" }}
      >
        绑定
      </button>
      {error && <span style={{ color: "var(--studio-failed-fg)" }}>{error}</span>}
    </div>
  );
}
