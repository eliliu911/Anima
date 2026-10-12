/**
 * 「从设定文档建资产」：系列页上，直接用剧本编辑器里的人物小传 / 场景设计 / 道具设计
 * 建系列资产 —— 项目阶段就把所有人物和场景定下来，不用每一集再提取一遍。
 *
 * 流程：选文档 → 后台模型一份一份认 → 同一个确认弹窗（没有「只留本集」）→ 建进系列。
 *
 * # 文档怎么找
 *
 * 先按系列绑定的剧本项目（`script_project_id`），再按名字兜底（去掉书名号比，见 `scriptLink.ts`）；
 * 按名字认出来的当场存成绑定。都对不上就让人选一次 —— 不猜。
 *
 * **读的是磁盘上的最新内容**（`listScriptDocuments`），不是剧本编辑器那个 store ——
 * store 在一个会话里只加载一次，编辑器外面改过的文档它看不见。
 */
import { useEffect, useState } from "react";
import type { CinemaProject, EpisodeData, ExtractedEntity } from "../../../api/cinema";
import { listScriptDocuments, listScriptProjects, type ScriptDocument, type ScriptProject } from "../../../api/scriptLibrary";
import { cinemaLinkScriptProject } from "../../../api/cinema";
import { resolveScriptProject } from "./scriptLink";
import { BindScriptProject } from "./BindScriptProject";
import { seriesExtractFromTexts } from "../../../api/seriesAssets";
import { runTask } from "../toastStore";
import { ExtractDialog } from "../steps/ExtractDialog";
import { importNote, isDesignDoc } from "./seriesCopy";

export function DesignDocsExtract({
  project,
  episodes,
  onClose,
  onDone,
}: {
  project: CinemaProject;
  /** 导入接口要一个分集 id（只在「只留本集」时用到，这里不会用） */
  episodes: EpisodeData[];
  onClose: () => void;
  onDone: (note: string) => void;
}) {
  const [docs, setDocs] = useState<ScriptDocument[] | null>(null);
  const [missing, setMissing] = useState(false);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [entities, setEntities] = useState<ExtractedEntity[] | null>(null);
  const [scriptProjects, setScriptProjects] = useState<ScriptProject[]>([]);
  const [allDocs, setAllDocs] = useState<ScriptDocument[]>([]);

  const useProject = (sp: ScriptProject, all: ScriptDocument[]) => {
    const mine = all
      .filter((d) => d.project_id === sp.id && (d.kind ?? "source") === "source")
      .sort((a, b) => a.title.localeCompare(b.title, "zh", { numeric: true }));
    setMissing(false);
    setDocs(mine);
    setPicked(new Set(mine.filter((d) => isDesignDoc(d.title)).map((d) => d.id)));
  };

  useEffect(() => {
    let alive = true;
    void Promise.all([listScriptProjects(), listScriptDocuments()])
      .then(([projects, all]) => {
        if (!alive) return;
        setScriptProjects(projects);
        setAllDocs(all);
        const hit = resolveScriptProject(projects, project);
        if (!hit) {
          setMissing(true);
          setDocs([]);
          return;
        }
        if (hit.via === "name") void cinemaLinkScriptProject(project.id, hit.project.id).catch(() => {});
        useProject(hit.project, all);
      })
      .catch((e) => alive && setError(String(e)));
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [project.id]);

  const run = async () => {
    const texts = (docs ?? []).filter((d) => picked.has(d.id)).map((d) => ({ title: d.title, text: d.text }));
    setBusy(true);
    setError("");
    try {
      const found = await runTask(
        {
          title: `正在从 ${texts.length} 份设定文档里认角色、场景、道具…`,
          body: "一份文档调一次后台模型，几份下来要几分钟，可以先去别的地方。",
          success: (es) => ({ title: "认出来了", body: `${es.length} 个，确认后建进系列` }),
          failure: "从设定文档提取失败",
        },
        () => seriesExtractFromTexts(project.id, texts),
      );
      setEntities(found);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  if (entities) {
    return (
      <ExtractDialog
        projectId={project.id}
        episodeId={episodes[0]?.id ?? ""}
        entities={entities}
        allowEpisode={false}
        onClose={onClose}
        onImported={(s) => onDone(importNote(s))}
      />
    );
  }

  return (
    <div className="studio-scrim fixed inset-0 z-50 flex items-center justify-center">
      <div className="studio-modal flex max-h-[86vh] w-[480px] max-w-[92vw] flex-col overflow-hidden" onClick={(e) => e.stopPropagation()}>
        <header className="shrink-0 px-6 pb-3 pt-5">
          <h2 className="studio-display text-[17px]">从设定文档建资产</h2>
          <p className="mt-1 text-[12px]" style={{ color: "var(--studio-text-muted)" }}>
            用剧本编辑器里「{project.title}」的设定文档，一次把整部剧的角色、场景、道具建进系列。
            默认勾的是人物 / 场景 / 道具 / 世界观这几份；每份只收它那一类（人物小传只出角色，场景设计只出场景）。
          </p>
        </header>
        <div className="min-h-0 flex-1 overflow-y-auto px-6">
          {docs === null && !error && (
            <p className="py-3 text-[12px]" style={{ color: "var(--studio-text-muted)" }}>
              读取中…
            </p>
          )}
          {missing && (
            <div className="py-3" style={{ color: "var(--studio-text-2)" }}>
              <BindScriptProject
                cinemaProjectId={project.id}
                seriesTitle={project.title}
                projects={scriptProjects}
                onBound={(sp) => useProject(sp, allDocs)}
              />
            </div>
          )}
          {docs?.map((d) => (
            <label key={d.id} className="flex cursor-pointer items-center gap-2.5 py-1.5 text-[12.5px]">
              <input
                type="checkbox"
                checked={picked.has(d.id)}
                onChange={() =>
                  setPicked((s) => {
                    const n = new Set(s);
                    if (n.has(d.id)) n.delete(d.id);
                    else n.add(d.id);
                    return n;
                  })
                }
              />
              <span className="min-w-0 flex-1 truncate">{d.title}</span>
              <span className="font-mono text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
                {d.text.length} 字
              </span>
            </label>
          ))}
        </div>
        {error && (
          <p className="shrink-0 px-6 pt-2 text-[11.5px]" style={{ color: "var(--studio-failed-fg)" }}>
            {error}
          </p>
        )}
        <footer className="mt-3 flex shrink-0 items-center justify-end gap-2 px-6 pb-5 pt-2">
          <button type="button" onClick={onClose} className="rounded-lg px-3.5 py-2 text-[12px]" style={{ color: "var(--studio-text-2)" }}>
            取消
          </button>
          <button
            type="button"
            disabled={busy || picked.size === 0}
            onClick={() => void run()}
            className="rounded-lg px-4 py-2 text-[12.5px] font-medium disabled:opacity-50"
            style={{ background: "var(--studio-primary)", color: "var(--studio-on-primary)" }}
          >
            {busy ? "提取中…" : `提取（${picked.size} 份）`}
          </button>
        </footer>
      </div>
    </div>
  );
}
