import { create } from "zustand";
import {
  listScriptDocuments, saveScriptDocument, listScriptProjects, saveScriptProject,
  deleteScriptProject,
  deleteScriptDocument,
  type ScriptDocument, type ScriptProject, type ScriptDocKind,
} from "../../../api/scriptLibrary";

const timers = new Map<string, ReturnType<typeof setTimeout>>();
const queues = new Map<string, Promise<boolean>>();
let loading: Promise<void> | null = null;
interface LibraryState {
  documents: ScriptDocument[]; projects: ScriptProject[];
  ready: boolean; error: string; status: Record<string, string>;
  load: () => Promise<void>;
  /**
   * 重新读一遍磁盘，合进来（进剧本页时调）。
   *
   * 2026-10-05：在应用外面导入的文档（直接写进 `studio/scripts`）界面上看不见，要重启 —— `load` 只读一次。
   * **本地还没存下去的不覆盖**（「未保存」「保存中」的、还没落过盘的新建稿），其余以磁盘为准。
   */
  refresh: () => Promise<void>;
  add: (title?: string, text?: string, doc?: string | null,
        opts?: { projectId?: string | null; kind?: ScriptDocKind }) => string;
  edit: (id: string, patch: Partial<Pick<ScriptDocument,
        "title" | "text" | "doc" | "project_id" | "kind">>) => void;
  flush: (id: string) => Promise<boolean>;
  /** 新建剧本项目，返回它的 id。 */
  addProject: (title: string) => Promise<string>;
  renameProject: (id: string, title: string) => Promise<void>;
  /** 删项目**不删文档** —— 它们回到散落状态。 */
  removeProject: (id: string) => Promise<void>;
  /**
   * 删一份文档。返回**因此失去源头**的那几份的标题，界面上要说出来。
   *
   * 派生出去的稿子不跟着删 —— 它们可能已经被手改过很多，
   * 用户删的是素材，不是成果。
   */
  removeDoc: (id: string) => Promise<string[]>;
  /** 生成出来的剧本直接并进列表，不用重新拉一遍。 */
  ingest: (document: ScriptDocument) => void;
}
export const useScriptLibrary = create<LibraryState>((set, get) => ({
  documents: [], projects: [], ready: false, error: "", status: {},
  load: async () => {
    if (get().ready) return;
    if (!loading) loading = (async () => {
      try {
        // 两份一起拉 —— 只拉到一半的话，界面会把有归属的文档显示成散落的
        const [documents, projects] = await Promise.all([
          listScriptDocuments(), listScriptProjects(),
        ]);
        set({ documents, projects, ready: true, error: "" });
      } catch (error) { set({ error: String(error) }); }
      finally { loading = null; }
    })();
    await loading;
  },
  refresh: async () => {
    if (!get().ready) return get().load();
    try {
      const [disk, projects] = await Promise.all([listScriptDocuments(), listScriptProjects()]);
      set((s) => {
        const busy = new Set(Object.entries(s.status).filter(([, v]) => v === "未保存" || v === "保存中").map(([k]) => k));
        const onDisk = new Set(disk.map((d) => d.id));
        const local = new Map(s.documents.map((d) => [d.id, d]));
        const documents = [
          ...disk.map((d) => (busy.has(d.id) ? local.get(d.id) ?? d : d)),
          // 还没落盘的（新建、正在存）留着
          ...s.documents.filter((d) => !onDisk.has(d.id) && (busy.has(d.id) || d.revision === 0)),
        ];
        return { documents, projects, error: "" };
      });
    } catch (error) { set({ error: String(error) }); }
  },
  add: (title = "未命名剧本", text = "", doc = null, opts) => {
    const id = crypto.randomUUID();
    set((s) => ({ documents: [{
      id, title, text, doc, revision: 0,
      project_id: opts?.projectId ?? null,
      kind: opts?.kind ?? "screenplay",
      derived_from: null,
    }, ...s.documents] }));
    void get().flush(id); return id;
  },
  ingest: (document) => set((s) => ({
    documents: [document, ...s.documents.filter((d) => d.id !== document.id)],
  })),
  addProject: async (title) => {
    const id = crypto.randomUUID();
    const saved = await saveScriptProject({
      id, title, note: "", created_at: 0, updated_at: 0,
    });
    set((s) => ({ projects: [...s.projects, saved] }));
    return id;
  },
  renameProject: async (id, title) => {
    const current = get().projects.find((p) => p.id === id);
    if (!current) return;
    const saved = await saveScriptProject({ ...current, title });
    set((s) => ({ projects: s.projects.map((p) => (p.id === id ? saved : p)) }));
  },
  removeProject: async (id) => {
    await deleteScriptProject(id);
    set((s) => ({
      projects: s.projects.filter((p) => p.id !== id),
      // 文档留着，只是回到散落状态 —— 和后端的行为保持一致
      documents: s.documents.map((d) => (d.project_id === id ? { ...d, project_id: null } : d)),
    }));
  },
  removeDoc: async (id) => {
    const orphaned = await deleteScriptDocument(id);
    set((s) => ({
      documents: s.documents.filter((d) => d.id !== id),
      // 这份的保存状态也要清 —— 留着的话列表底下会一直挂着一个
      // 指向已删文档的「未保存」。
      status: Object.fromEntries(Object.entries(s.status).filter(([k]) => k !== id)),
    }));
    return orphaned;
  },
  edit: (id, patch) => {
    set((s) => ({ documents: s.documents.map((d) => d.id === id ? { ...d, ...patch } : d), status: { ...s.status, [id]: "未保存" } }));
    clearTimeout(timers.get(id));
    timers.set(id, setTimeout(() => { timers.delete(id); void get().flush(id); }, 400));
  },
  flush: (id) => {
    clearTimeout(timers.get(id)); timers.delete(id);
    const task = (queues.get(id) ?? Promise.resolve(true)).then(async () => {
      const snapshot = get().documents.find((d) => d.id === id);
      if (!snapshot) return false;
      set((s) => ({ status: { ...s.status, [id]: "保存中" } }));
      try {
        const saved = await saveScriptDocument(snapshot);
        set((s) => ({ documents: s.documents.map((d) => d.id === id ? { ...d, revision: saved.revision } : d), status: {
          ...s.status, [id]: s.documents.find((d) => d.id === id) === snapshot ? "已保存" : "未保存",
        } }));
        return true;
      } catch (error) { set((s) => ({ status: { ...s.status, [id]: `保存失败：${error}` } })); return false; }
    });
    queues.set(id, task);
    void task.finally(() => { if (queues.get(id) === task) queues.delete(id); });
    return task;
  },
}));
