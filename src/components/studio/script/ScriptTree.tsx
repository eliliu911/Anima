import { useMemo } from "react";
import type { ScriptDocKind, ScriptDocument, ScriptProject } from "../../../api/scriptLibrary";
import type { Pane } from "./scriptPanes";
import { Film, Folder, FolderPlus, Trash, Upload } from "../../../canvas/ui/icons";

/**
 * 剧本库的资源树。
 *
 * # 为什么要有这个
 *
 * 以前是一张平铺的文档列表 —— 打开一份不知道它属于谁、从哪来、
 * 下一步该干什么。树把三件事显式化：
 * 属于哪个项目、是原文还是剧本、剧本是从哪份原文来的。
 */
export interface ScriptTreeProps {
  projects: ScriptProject[];
  documents: ScriptDocument[];
  /** 左栏选中的是什么。`undefined` = 还没选过（右栏显示第一个故事）。 */
  pane: Pane | undefined;
  /** 视频项目里有多少个分集 —— 左栏那一行要显示数量。 */
  videoCount: number;
  query: string;
  busy: boolean;
  /** 点了左栏的某一项。 */
  onSelectPane: (pane: Pane) => void;
  onNewProject: () => void;
  onNewDoc: (projectId: string | null, kind: ScriptDocKind) => void;
  onImport: (projectId: string | null) => void;
  onDeleteProject: (id: string) => void;
}

/** 一个项目（或「未归档」）下的分组。 */
export interface Group {
  id: string | null;
  title: string;
  note: string;
  sources: ScriptDocument[];
  /** 改编方案 —— 原文和剧本之间那一层。 */
  plans: ScriptDocument[];
  screenplays: ScriptDocument[];
}

export function groupDocuments(
  projects: ScriptProject[],
  documents: ScriptDocument[],
  query: string,
): Group[] {
  const q = query.trim();
  const hit = (d: ScriptDocument) => !q || d.title.includes(q) || d.text.includes(q);

  // **归属指向一个不存在的项目时，当成未归档。**
  //
  // 正常流程里 `removeProject` 会把归属解开，所以这不该发生 ——
  // 但「不该发生」的事真发生时，后果是**用户的稿子从界面上消失**，
  // 而且不报错。宁可显示在错的分组里，也不能让它人间蒸发。
  const known = new Set(projects.map((p) => p.id));
  const homeOf = (d: ScriptDocument) => {
    const pid = d.project_id ?? null;
    return pid && known.has(pid) ? pid : null;
  };
  const of = (pid: string | null, kind: string) =>
    documents.filter((d) => homeOf(d) === pid && (d.kind ?? "screenplay") === kind && hit(d));

  const groups: Group[] = projects.map((p) => ({
    id: p.id,
    title: p.title || "未命名项目",
    note: p.note,
    sources: of(p.id, "source"),
    plans: of(p.id, "plan"),
    screenplays: of(p.id, "screenplay"),
  }));

  // **散落的文档永远显示**，哪怕一份都没有也留着这一组 ——
  // 所有存量数据都在这里，藏起来等于让人以为东西丢了。
  groups.push({
    id: null,
    title: "未归档",
    note: "还没放进任何剧本项目",
    sources: of(null, "source"),
    plans: of(null, "plan"),
    screenplays: of(null, "screenplay"),
  });
  return groups;
}

/**
 * 剧本库的故事列表（左栏）。
 *
 * **只列故事，不嵌套稿子。** 稿子在右栏 —— 两边都列一遍的话，
 * 用户得先想「该点哪边的」，而两边点下去还不是一回事。
 */
export function ScriptTree(props: ScriptTreeProps) {
  const { projects, documents, pane, videoCount, query, busy } = props;
  const groups = useMemo(
    () => groupDocuments(projects, documents, query),
    [projects, documents, query],
  );

  return (
    <nav className="script-tree" aria-label="剧本库">
      <div className="script-tree-tools">
        <button disabled={busy} onClick={props.onNewProject} title="新建剧本项目">
          <FolderPlus />
          新建项目
        </button>
      </div>

      {groups.map((g) => {
        const count = g.sources.length + g.plans.length + g.screenplays.length;
        // 还没选过时高亮第一个 —— 右栏显示的就是它，两边必须一致，
        // 否则用户看到的是「左边没选中任何东西，右边却有内容」。
        const on =
          pane === undefined
            ? g === groups[0]
            : pane.kind === "story" && pane.id === (g.id ?? null);
        return (
          <section key={g.id ?? "__loose__"} className="script-tree-group">
            <div className={`script-tree-project${on ? " is-selected" : ""}`}>
              <button
                className="script-tree-project-name"
                onClick={() => props.onSelectPane({ kind: "story", id: g.id })}
                aria-current={on}
                title={g.note}
              >
                <Folder />
                <span>{g.title}</span>
                <small>{count}</small>
              </button>
              <span className="script-tree-project-actions">
                <button
                  disabled={busy}
                  title="在这个项目里导入原文"
                  onClick={() => props.onImport(g.id)}
                >
                  <Upload />
                </button>
                {g.id && (
                  <button
                    disabled={busy}
                    title="删除项目（里面的文档会保留，回到未归档）"
                    onClick={() => props.onDeleteProject(g.id as string)}
                  >
                    <Trash />
                  </button>
                )}
              </span>
            </div>
          </section>
        );
      })}

      {/* **视频项目里的分集是另一类东西，所以它在这儿而不是每个项目底下。**
          原来挂在项目页面底部，于是打开「余波」也能看到「米小团」的分集 ——
          而两者之间没有任何链接字段，挂在哪个项目下都是假的。 */}
      <div className="script-tree-divider" />
      <section className="script-tree-group">
        <div className={`script-tree-project${pane?.kind === "video" ? " is-selected" : ""}`}>
          <button
            className="script-tree-project-name"
            onClick={() => props.onSelectPane({ kind: "video" })}
            aria-current={pane?.kind === "video"}
            title="已经在制作中的分集，改动会直接影响成片"
          >
            <Film />
            <span>视频项目的分集</span>
            <small>{videoCount}</small>
          </button>
        </div>
      </section>
    </nav>
  );
}
