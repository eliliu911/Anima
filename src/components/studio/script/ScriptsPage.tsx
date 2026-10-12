import { useEffect, useRef, useState } from "react";
import { cinemaScripts, cinemaGetProjectDetail, cinemaSaveEpisode, cinemaCreateEpisode, cinemaWorkspace, type ScriptSummary, type EpisodeData, type WorkspaceItem } from "../../../api/cinema";
import { useScriptLibrary } from "./libraryStore";
import { ScriptEditor } from "./ScriptEditor";
import { ComposingInput } from "../../ComposingInput";
import { serializeDoc, textToDoc } from "./docConvert";
import { ChevronRight, Sparkles, BookText, FileText, Trash } from "../../../canvas/ui/icons";
import { ScriptTree, groupDocuments } from "./ScriptTree";
import type { ScriptDocKind } from "../../../api/scriptLibrary";
import { activeGroup, type Pane } from "./scriptPanes";
import { episodeLabel, nextEpisodeName } from "./episodeLabel";
import { sortDocs, naturalCompare, SORT_OPTIONS, DEFAULT_SORT, type SortKey } from "./sortDocs";
import { generateScreenplay } from "../../../api/scriptLibrary";
import { runTask } from "../toastStore";
import { resolveScriptProject } from "../series/scriptLink";
import { BindScriptProject } from "../series/BindScriptProject";
import { cinemaLinkScriptProject } from "../../../api/cinema";

export function ScriptsPage({
  initialStory,
}: {
  /**
   * 从系列页的「故事原文」跳过来时带的系列：先按绑定的 id 找剧本项目，再按名字兜底
   * （去掉书名号比，见 `scriptLink.ts`）。按名字认出来的**当场存成绑定**；
   * 认不出来就在顶上让人选一次。
   */
  initialStory?: { cinemaProjectId: string; title: string; scriptProjectId?: string | null };
} = {}) {
  const library = useScriptLibrary();
  const [selected, setSelected] = useState<string | null>(null);
  /**
   * 左栏选中的是什么。`undefined` = 还没选过（右栏落到第一个故事）。
   *
   * **用带 kind 的联合类型，不用一个可空 id。** 之前 `null` 同时表示
   * 「未归档」和「还没选」，刚进页面默认停在最空的那一组 ——
   * 一个值两个意思，实跑第一眼就撞出来了。
   */
  const [pane, setPane] = useState<Pane | undefined>(undefined);
  /** 刚新建完项目 —— 右栏那个名字输入框自动聚焦并全选。 */
  const [renaming, setRenaming] = useState(false);
  /**
   * 原文和剧本**各自一个排法**。
   *
   * 章节按名称（自然序）最常用，分集也是；但「最近改动」在原文那边
   * （找上次写到哪）和剧本那边（找刚生成的那集）想看的不是同一批，
   * 所以不共用一个。
   */
  /**
   * 勾中要一起规划的原文。
   *
   * **章节和集数不是一对一。** 一章可能出两集，几章讲同一件事该并成
   * 一集，而十个字的一章撑不起任何一集 —— 得和相邻章并起来。
   * 不给这个勾选的话，用户只能一章一章地试，而「这两章该合并」
   * 这个判断只有他自己做得了。
   */
  const [chosen, setChosen] = useState<Set<string>>(new Set());
  const [sortSource, setSortSource] = useState<SortKey>(DEFAULT_SORT);
  const [sortScript, setSortScript] = useState<SortKey>(DEFAULT_SORT);
  const [projects, setProjects] = useState<ScriptSummary[]>([]);
  const [workspaces, setWorkspaces] = useState<WorkspaceItem[]>([]);
  const [projectDoc, setProjectDoc] = useState<{ pid: string; title: string; episode: EpisodeData } | null>(null);
  const [error, setError] = useState("");
  const [query, setQuery] = useState("");
  const [target, setTarget] = useState("");
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState("");
  /** 正在等第二次点击确认删除的那一行。null = 没有。 */
  const [confirmDel, setConfirmDel] = useState<string | null>(null);
  /**
   * 「导入」点的是哪个区的按钮 —— 文件选完才知道要放哪、当成什么。
   *
   * # 为什么要带 `kind`
   *
   * 原来写死成原文，理由是「用户导进来的是小说/大纲/素材」。
   * **那个前提不成立**：很多人手上只有剧本（剧本杀本子、买来的本子、
   * 别人写好的），根本没有原文。把剧本当原文导进来之后，
   * 界面会告诉他「下一步：先定改编方案」—— 整条路走反了。
   *
   * 不猜，**点哪个区的「导入」就是哪一种**。猜错的代价是下游拿一份
   * 没有分场分镜的东西去抽实体，而那不报错。
   */
  const [importTarget, setImportTarget] = useState<{ pid: string | null; kind: ScriptDocKind } | null>(null);
  const [genNote, setGenNote] = useState("");
  const input = useRef<HTMLInputElement>(null);
  const loadProjects = () => cinemaScripts().then(setProjects).catch((e) => setError(String(e)));
  // 进剧本页就重读一遍磁盘：应用外面导入的文档不用重启也看得见（见 `refresh`）
  useEffect(() => { void library.refresh(); void loadProjects(); void cinemaWorkspace().then(setWorkspaces).catch((e) => setError(String(e))); }, []);
  // 从系列跳过来：库读好之后落到绑定的项目（只做一次）
  const jumped = useRef(false);
  const [unbound, setUnbound] = useState(false);
  useEffect(() => {
    if (!initialStory || jumped.current || !library.ready) return;
    jumped.current = true;
    const hit = resolveScriptProject(library.projects, {
      title: initialStory.title,
      script_project_id: initialStory.scriptProjectId,
    });
    if (!hit) {
      setUnbound(true);
      return;
    }
    setPane({ kind: "story", id: hit.project.id });
    if (hit.via === "name") {
      void cinemaLinkScriptProject(initialStory.cinemaProjectId, hit.project.id).catch(() => {});
    }
  }, [initialStory, library.ready, library.projects]);
  useEffect(() => () => {
    const state = useScriptLibrary.getState();
    for (const d of state.documents) if (state.status[d.id] === "未保存") void state.flush(d.id);
  }, []);
  const document = library.documents.find((d) => d.id === selected);
  const openProject = async (row: ScriptSummary) => {
    setBusy(true); setError("");
    try {
      const detail = await cinemaGetProjectDetail(row.project_id);
      const episode = detail.episodes.find((e) => e.id === row.episode_id);
      if (!episode) throw new Error("分集不存在");
      setProjectDoc({ pid: row.project_id, title: row.project_title, episode }); setSelected(null);
    } catch (e) { setError(String(e)); } finally { setBusy(false); }
  };
  const destinations = workspaces.map((p) => [p.id, p.title]);
  /**
   * 原文 → 剧本。**不覆盖任何已有内容** —— 每次都是一份新文档，
   * 带着 `derived_from` 记住它从哪来。
   */
  const generate = async () => {
    if (!document || busy) return;
    setBusy(true); setError(""); setNotice("");
    try {
      if (!(await library.flush(document.id))) throw new Error("原文还没保存成功，先解决保存问题");
      const created = await runTask(
        {
          title: document.kind === "source" ? "正在由原文生成…" : "正在生成剧本…",
          body: "后台模型在写，通常一两分钟，可以先去别的地方。",
          success: (doc) => ({ title: doc.kind === "plan" ? "改编方案生成好了" : "剧本生成好了", body: `《${doc.title}》` }),
          failure: "生成失败",
        },
        () => generateScreenplay(document.id, genNote),
      );
      library.ingest(created);
      setSelected(created.id);
      setGenNote("");
      setNotice(
        created.kind === "plan"
          ? "改编方案已生成。看一遍分集对不对，再点「生成剧本」。"
          : "剧本已生成。它是一份新文档，上游没有被改动。",
      );
    } catch (e) { setError(String(e)); } finally { setBusy(false); }
  };
  const sourceOf = document?.derived_from
    ? library.documents.find((d) => d.id === document.derived_from?.source_doc_id)
    : undefined;
  /**
   * 把勾中的几章一起规划成一份方案。
   *
   * **主来源取章号最小的那一份** —— 它是这段材料的开头，
   * 「回到源头」跳过去才对得上。
   */
  const generatePlanFor = async (ids: string[]) => {
    if (!ids.length || busy) return;
    setBusy(true); setError(""); setNotice("");
    try {
      const docs = library.documents.filter((d) => ids.includes(d.id));
      const ordered = [...docs].sort((a, b) => naturalCompare(a.title ?? "", b.title ?? ""));
      for (const d of ordered) {
        if (!(await library.flush(d.id))) throw new Error(`《${d.title}》还没保存成功`);
      }
      const created = await runTask(
        {
          title: `正在规划 ${ordered.length} 章的改编方案…`,
          body: "后台模型在读，章数多会久一些，可以先去别的地方。",
          success: (doc) => ({ title: "改编方案生成好了", body: `《${doc.title}》，覆盖 ${ordered.length} 章` }),
          failure: "生成改编方案失败",
        },
        () => generateScreenplay(ordered[0].id, genNote, ordered.slice(1).map((d) => d.id)),
      );
      library.ingest(created);
      setSelected(created.id);
      setChosen(new Set());
      setGenNote("");
      setNotice(`改编方案已生成，覆盖 ${ordered.length} 章。看一遍分集对不对，再点「生成剧本」。`);
    } catch (e) { setError(String(e)); } finally { setBusy(false); }
  };

  const sendToProject = async () => {
    if (!document || !target || busy) return;
    // **原文不能直接变成分集。**
    //
    // 「下游只认 screenplay」是写在 `ScriptDocument.kind` 上的不变量：
    // 抽实体、生成分镜读的都是剧本，而原文没有分场分镜 ——
    // 喂过去不会报错，只会生成一堆对不上的镜头。
    //
    // 界面上这条路已经拦住了（原文那一屏只给「生成剧本」），
    // 这里再拦一次是因为**界面会改，不变量不该只靠界面守**。
    if ((document.kind ?? "screenplay") === "source") {
      setError("这是原文，不能直接建分集 —— 先用它生成剧本，再拿剧本建。");
      return;
    }
    setBusy(true); setError("");
    try {
      if (!(await library.flush(document.id))) return;
      await cinemaCreateEpisode({ project_id: target, title: document.title, script_text: document.text });
      setNotice("已创建新的分集制作副本；独立原稿保留，两者不会自动同步。");
      setTarget(""); await loadProjects();
    } catch (e) { setError(String(e)); } finally { setBusy(false); }
  };
  const groups = groupDocuments(library.projects, library.documents, query);
  const group = activeGroup(groups, pane === undefined ? undefined : pane.kind === "story" ? pane.id : undefined);

  /** 右栏的工具条 —— 打开一份稿子时用，**左栏不受影响**。 */
  const docToolbar = document && <div className="script-tools">
    <button disabled={busy} onClick={async () => { if (await library.flush(document.id)) { setSelected(null); setNotice(""); } }}>返回本剧</button>
    <span className="script-kind-badge">{
      (document.kind ?? "screenplay") === "source" ? <><BookText />原文</>
        : (document.kind ?? "screenplay") === "plan" ? <><Sparkles />改编方案</>
          : <><FileText />剧本</>
    }</span>
    <input aria-label="文档名称" value={document.title} onChange={(e) => library.edit(document.id, { title: e.target.value })} />
    <select aria-label="所属剧本项目" value={document.project_id ?? ""} onChange={(e) => library.edit(document.id, { project_id: e.target.value || null })}>
      <option value="">未归档</option>
      {library.projects.map((p) => <option key={p.id} value={p.id}>{p.title}</option>)}
    </select>
    <span className="script-library-status">{library.status[document.id] ?? "已保存"}</span>
    <button onClick={() => void library.flush(document.id)}>保存 / 重试</button>
  </div>;

  /**
   * 行尾那个删除键。
   *
   * **两步确认，不用 `window.confirm`。** 删文档是不可逆的（文件真的没了），
   * 而 Tauri 的 WKWebView 里对话框这条路本来就不可靠 —— `window.prompt`
   * 直接静默返回 null（这个坑在这一页的「新建项目」上踩过）。
   * 行内两步既确定能用，也不打断视线。
   */
  const DeleteDoc = ({ id, what }: { id: string; what: string }) => (
    <button
      className={`script-row-del${confirmDel === id ? " is-armed" : ""}`}
      disabled={busy}
      title={confirmDel === id ? "再点一次就真的删了" : `删除${what}`}
      aria-label={confirmDel === id ? `确认删除${what}` : `删除${what}`}
      onClick={(e) => {
        e.stopPropagation();
        if (confirmDel !== id) {
          setConfirmDel(id);
          setNotice("");
          return;
        }
        setConfirmDel(null);
        void library
          .removeDoc(id)
          .then((orphaned) => {
            // 删掉的正好是正在看的那份时，右栏要退回列表
            if (selected === id) setSelected(null);
            // **失去源头的那几份要说出来。** 不说的话，它们的
            // 「回到源头」会静默地跳不动。
            setNotice(
              orphaned.length
                ? `已删除。${orphaned.length} 份稿子失去了源头：${orphaned.join("、")} —— 它们还在，只是「回到源头」跳不回去了。`
                : "已删除。",
            );
          })
          .catch((err) => setError(String(err)));
      }}
    >
      {confirmDel === id ? "确定？" : <Trash />}
    </button>
  );

  return <div className="script-library">
    <header className="studio-banner flex shrink-0 items-start justify-between px-7 py-5 script-library-header">
      <div><small>ANIMA STUDIO · SCRIPTS</small><h1>剧本编辑器</h1></div>
      <div className="script-tools">
        <input aria-label="搜索剧本库" placeholder="搜索剧本 / 原文" value={query} onChange={(e) => setQuery(e.target.value)} />
      </div>
      <input hidden ref={input} type="file" accept=".md,.txt" onChange={async (e) => {
        const file = e.target.files?.[0]; e.target.value = ""; if (!file) return;
        setBusy(true);
        try {
          if (file.size > 5 * 1024 * 1024) throw new Error("单文件暂限 5 MB");
          const text = await file.text();
          // **点哪个区的「导入」就是哪一种**，不猜。
          const kind = importTarget?.kind ?? "source";
          setSelected(library.add(
            file.name.replace(/\.(md|txt)$/i, ""), text, serializeDoc(textToDoc(text)),
            { projectId: importTarget?.pid ?? null, kind },
          ));
          setNotice(
            kind === "source"
              ? "已作为原文导入。下一步：打开它，先定改编方案。"
              : "已作为剧本导入。下一步：打开它，用它建视频项目 —— 原文和方案这两步跳过就好。",
          );
        } catch (e) { setError(String(e)); } finally { setBusy(false); }
      }} />
    </header>
    {(error || library.error) && <p role="alert" className="script-library-error">{error || library.error} {!library.ready && <button onClick={() => void library.load()}>重试读取</button>}</p>}
    {notice && <p role="status" className="script-notice">{notice}</p>}
    {unbound && initialStory && (
      <div role="status" className="script-notice">
        <BindScriptProject
          cinemaProjectId={initialStory.cinemaProjectId}
          seriesTitle={initialStory.title}
          projects={library.projects}
          onBound={(p) => {
            setUnbound(false);
            setPane({ kind: "story", id: p.id });
          }}
        />
      </div>
    )}

    {/* **两栏永远在。**
    
        原来是 `document ? <编辑器> : <两栏>` —— 打开一份稿子，
        整页被编辑器替换，左边的树直接消失。用户的原话是
        「点击余波，应该在右侧加载余波的剧本」。
        现在编辑器搬进右栏，左栏始终是那几个故事。 */}
    <div className="script-explorer">
      <ScriptTree
        projects={library.projects}
        documents={library.documents}
        pane={pane}
        videoCount={projects.length}
        query={query}
        busy={busy || !library.ready}
        onSelectPane={(next) => {
          setPane(next);
          // 换故事时把打开的稿子收起来 —— 留着的话右栏显示的是**别的故事**
          // 的稿子，而左栏高亮在这个故事上，两边对不上。
          setSelected(null); setProjectDoc(null); setNotice(""); setChosen(new Set());
        }}
        onNewProject={() => {
          // **不用 `window.prompt`。**
          //
          // Tauri 的 WKWebView **没有实现 prompt** —— 它直接返回 null，
          // 于是 `if (title === null) return` 悄悄退出，按钮看起来是死的。
          // 用户报的就是这个：「新建项目按钮，无效」。
          //
          // 而且就算能弹，在桌面应用里弹一个浏览器原生 prompt 也不对。
          // 直接建出来、选中它、把名字交给右栏那个输入框改 ——
          // 少一步弹窗，而且建完立刻看得见。
          void library.addProject("新剧本项目")
            .then((id) => { if (typeof id === "string") { setPane({ kind: "story", id }); setRenaming(true); } })
            .catch((e) => setError(String(e)));
        }}
        onNewDoc={(pid, kind) => { setSelected(library.add(kind === "source" ? "未命名原文" : "未命名剧本", "", null, { projectId: pid, kind })); setNotice(""); }}
        onImport={(pid) => { setImportTarget({ pid, kind: "source" }); input.current?.click(); }}
        onDeleteProject={(id) => {
          if (!window.confirm("删除这个剧本项目？里面的原文和剧本会保留，回到「未归档」。")) return;
          void library.removeProject(id).catch((e) => setError(String(e)));
          if (pane?.kind === "story" && pane.id === id) setPane({ kind: "story", id: null });
        }}
      />

      <div className="script-library-list">
        {!library.ready && !library.error && <p>读取中…</p>}

        {pane?.kind === "video" && !document && !projectDoc ? <>
          {/* **它在左栏有自己的位置，不再挂在每个项目底下。**
              原来挂在项目页面底部，于是打开「余波」也能看到「米小团」
              的分集 —— 而两者之间没有任何链接字段，挂在哪个项目下都是假的。 */}
          <h2>视频项目的分集</h2>
          {/* **JSX 不认 markdown。** 星号会原样显示出来 —— 要加粗用标签。 */}
          <p className="script-hint">
            已经在制作中的分集，改动会<strong>直接影响成片</strong>。
            和左边那些独立原稿不是同一份 —— 用剧本建视频项目时复制过去的，两边不会自动同步。
          </p>
          {!projects.length && <p className="script-tree-empty">还没有视频项目分集。先写好剧本，再用它建视频项目。</p>}
          {projects.filter((x) => `${x.project_title}${x.episode_title}`.includes(query)).map((x) => (
            <button disabled={busy} className="script-library-row" key={`${x.project_id}/${x.episode_id}`} onClick={() => void openProject(x)}>
              <span>{x.episode_title}<small>{x.project_title} · {x.chars} 字</small></span>
              <ChevronRight />
            </button>
          ))}
        </> : document ? <>
          {docToolbar}
          {/* **下一步在这里。** 原文的下一步是生成剧本，剧本的下一步是建视频项目 ——
              每一屏只给一个主动作，不让用户自己回去找。 */}
          {/* **下一步由这份文档是什么决定。**
              原文 → 改编方案 → 剧本 → 视频项目，一屏只给一个主动作。 */}
          {(document.kind ?? "screenplay") === "source" ? (
            <div className="script-tools script-next-step">
              <Sparkles />
              <span>下一步：先定改编方案 —— 哪几章并成哪一集</span>
              <input aria-label="额外要求" placeholder="额外要求（可留空，如：分 6 集 / 只改前三章）" value={genNote} onChange={(e) => setGenNote(e.target.value)} />
              <button disabled={busy || !document.text.trim()} onClick={() => void generate()}>
                {busy ? "生成中…" : "生成改编方案"}
              </button>
            </div>
          ) : (document.kind ?? "screenplay") === "plan" ? (
            <div className="script-tools script-next-step">
              <Sparkles />
              <span>下一步：按这份方案写剧本{sourceOf && ` · 素材取自《${sourceOf.title}》`}</span>
              <input aria-label="额外要求" placeholder="额外要求（可留空，如：只写第 1 集）" value={genNote} onChange={(e) => setGenNote(e.target.value)} />
              <button disabled={busy || !document.text.trim()} onClick={() => void generate()}>
                {busy ? "生成中…" : "生成剧本"}
              </button>
            </div>
          ) : (
            <div className="script-tools script-next-step">
              <ChevronRight />
              <span>下一步：用这个剧本建视频项目{sourceOf && ` · 出自《${sourceOf.title}》`}</span>
              <select aria-label="制作副本目标项目" value={target} onChange={(e) => setTarget(e.target.value)}>
                <option value="">选择视频项目…</option>
                {destinations.map(([id, name]) => <option key={id} value={id}>{name}</option>)}
              </select>
              <button disabled={!target || busy} onClick={() => void sendToProject()}>新建分集副本</button>
            </div>
          )}
          <ScriptEditor key={document.id} episodeKey={document.id} scriptText={document.text} scriptDoc={document.doc} onChange={(text, doc) => library.edit(document.id, { text, doc })} />
        </> : projectDoc ? (
          <ProjectScript key={`${projectDoc.pid}/${projectDoc.episode.id}`} value={projectDoc} onBack={() => { setProjectDoc(null); void loadProjects(); }} />
        ) : <>
          {/* **项目名可以就地改。** 新建出来叫「新剧本项目」，
              建完这里自动进入编辑态 —— 不弹窗，建完立刻看得见、立刻能改。 */}
          {group?.id ? (
            // **`ComposingInput` 不是可选的。** 普通受控 input 在这里
            // 用拼音打中文会打出 `yyuyu'jyu'jiyu'…` —— 每敲一个字母就
            // 往上写一次，回灌的值把输入法的组合缓冲冲掉了。用户报过。
            <ComposingInput
              className="script-project-title"
              aria-label="项目名称"
              value={group.title}
              autoFocus={renaming}
              onFocus={(e) => renaming && e.currentTarget.select()}
              onBlur={() => setRenaming(false)}
              onCommit={(v) => void library.renameProject(group.id as string, v).catch((x) => setError(String(x)))}
            />
          ) : (
            <h2>{group?.title ?? "剧本库"}</h2>
          )}
          <p className="script-hint">{group?.note || "这个项目下的原文和剧本"}</p>

          {/* ── 原文 ──
              用户原话：「项目：余波，下面两个子目录，原文，剧本，
              然后各自里面有章节。」原来是一张平表，原文和剧本混在一起、
              靠一行小字区分 —— 看不出哪个生哪个，也看不出顺序。 */}
          <div className="script-section">
            <div className="script-section-head">
              <h3><BookText />原文</h3>
              <small>小说 / 大纲 / 素材 —— 剧本从这里生成</small>
              {/* **方向不让用户选。** 每种排法只有一个说得通的方向：
                  名字按顺序看、改动看最近的、字数看最长的。给一个 ▲▼
                  只是多一次要想的事，而且想错了列表就乱了。 */}
              <select aria-label="原文排序" className="script-sort" value={sortSource} onChange={(e) => setSortSource(e.target.value as SortKey)}>
                {SORT_OPTIONS.map((o) => <option key={o.id} value={o.id}>{o.label}</option>)}
              </select>
              <button disabled={busy} onClick={() => { setImportTarget({ pid: group?.id ?? null, kind: "source" }); input.current?.click(); }}>导入</button>
            </div>
            {!group?.sources.length && <p className="script-tree-empty">还没有原文。导入一份，就能一键改写成剧本。</p>}
            {sortDocs(group?.sources ?? [], sortSource).map((sdoc) => (
              <div key={sdoc.id} className="script-library-row">
                {/* 勾选是**次要动作**，一小格放最左；点行本身还是打开它 */}
                <input
                  type="checkbox"
                  aria-label={`选中「${sdoc.title || "未命名原文"}」一起规划`}
                  className="script-pick"
                  checked={chosen.has(sdoc.id)}
                  onChange={(e) => {
                    const next = new Set(chosen);
                    if (e.target.checked) next.add(sdoc.id);
                    else next.delete(sdoc.id);
                    setChosen(next);
                  }}
                />
                <button className="script-row-main" onClick={() => { setSelected(sdoc.id); setNotice(""); }}>
                  <span>{sdoc.title || "未命名原文"}<small>{sdoc.text.length} 字</small></span>
                  <ChevronRight />
                </button>
                <DeleteDoc id={sdoc.id} what="这份原文" />
              </div>
            ))}
            {/* 勾了两章以上才出现 —— 平时不占位置 */}
            {chosen.size >= 2 && (
              <div className="script-tools script-next-step">
                <Sparkles />
                <span>已选 {chosen.size} 章，一起规划成一份改编方案</span>
                <button disabled={busy} onClick={() => void generatePlanFor([...chosen])}>
                  {busy ? "生成中…" : "合并规划"}
                </button>
                <button disabled={busy} onClick={() => setChosen(new Set())}>取消</button>
              </div>
            )}
          </div>

          {/* ── 改编方案 ──
              **原文和剧本之间缺的那一层。** 没有它的话模型每次自己现分集，
              同一本小说跑两次得到两套完全不同的分法，而且都不报错。
              空的时候不占位置 —— 短素材直接写也是合法路径（上游 SKILL.md：
              「开发大纲和长篇分析都是**可选**上游；当前材料足够时直接写」）。 */}
          {!!group?.plans.length && (
            <div className="script-section">
              <div className="script-section-head">
                <h3><Sparkles />改编方案</h3>
                <small>哪几章并成哪一集 · 每集兑现什么</small>
              </div>
              {group.plans.map((pdoc) => {
                const from = pdoc.derived_from
                  ? library.documents.find((x) => x.id === pdoc.derived_from?.source_doc_id)?.title ?? "已删除的原文"
                  : null;
                return (
                  // **整行不能再是一个 `<button>`** —— 删除键要嵌在行里，
                  // 而按钮套按钮是非法的（浏览器会把内层拆出去，
                  // 点击落在谁身上全看实现）。改成和原文那一行同一个形状。
                  <div key={pdoc.id} className="script-library-row">
                    <button className="script-row-main" onClick={() => { setSelected(pdoc.id); setNotice(""); }}>
                      <span>{pdoc.title || "未命名方案"}<small>{pdoc.text.length} 字{from && ` · 出自《${from}》`}</small></span>
                      <ChevronRight />
                    </button>
                    <DeleteDoc id={pdoc.id} what="这份方案" />
                  </div>
                );
              })}
            </div>
          )}

          {/* ── 剧本（分集） ──
              编号写成 S01E01 —— 它带着**顺序**，而顺序是剧本最要紧的
              属性之一。平表里「余波」「第 1 集」「未命名剧本」看不出先后。 */}
          <div className="script-section">
            <div className="script-section-head">
              <h3><FileText />剧本</h3>
              <small>一集一份，S01E01 起</small>
              <select aria-label="剧本排序" className="script-sort" value={sortScript} onChange={(e) => setSortScript(e.target.value as SortKey)}>
                {SORT_OPTIONS.map((o) => <option key={o.id} value={o.id}>{o.label}</option>)}
              </select>
              {/* **不是所有人都有原文。** 剧本杀本子、买来的本子、别人写好的 ——
                  原文和方案这两步可以整个跳过，直接拿剧本进制作。 */}
              <button disabled={busy} onClick={() => { setImportTarget({ pid: group?.id ?? null, kind: "screenplay" }); input.current?.click(); }}>导入本子</button>
              <button disabled={busy} onClick={() => {
                setSelected(library.add(nextEpisodeName(group?.screenplays ?? []), "", null, { projectId: group?.id ?? null, kind: "screenplay" }));
                setNotice("");
              }}>+ 新增一集</button>
            </div>
            {!group?.screenplays.length && <p className="script-tree-empty">还没有分集。三条路都行：从原文生成、导入现成的本子、或者直接新增一集自己写。</p>}
            {sortDocs(group?.screenplays ?? [], sortScript).map((sdoc, i) => {
              const label = episodeLabel(sdoc, i);
              const from = sdoc.derived_from ? library.documents.find((x) => x.id === sdoc.derived_from?.source_doc_id)?.title ?? "已删除的原文" : null;
              return (
                <div key={sdoc.id} className="script-library-row">
                  <button className="script-row-main" onClick={() => { setSelected(sdoc.id); setNotice(""); }}>
                    {/* 编号单独一列 —— 结构那一列要对齐，扫一眼就知道缺了哪集 */}
                    <code className={`script-ep-code${label.inferred ? " is-inferred" : ""}`} title={label.inferred ? "标题里没写集号，这个是按位置补的" : undefined}>
                      {label.code}
                    </code>
                    <span>
                      {label.name || "未命名"}
                      <small>{sdoc.text.length} 字{from && ` · 出自《${from}》`}</small>
                    </span>
                    <ChevronRight />
                  </button>
                  <DeleteDoc id={sdoc.id} what={`${label.code} 这一集`} />
                </div>
              );
            })}
          </div>

        </>}
      </div>
    </div>
  </div>;
}

function ProjectScript({ value, onBack }: { value: { pid: string; title: string; episode: EpisodeData }; onBack: () => void }) {
  const [draft, setDraft] = useState(value.episode);
  const [dirty, setDirty] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const revision = useRef(0);
  const save = async () => {
    const version = revision.current; setBusy(true); setError("");
    try {
      const latest = (await cinemaGetProjectDetail(value.pid)).episodes.find((e) => e.id === draft.id);
      if (!latest) throw new Error("分集已删除");
      await cinemaSaveEpisode(value.pid, { ...latest, title: draft.title, script_text: draft.script_text, script_doc: draft.script_doc });
      if (version === revision.current) setDirty(false);
    } catch (e) { setError(String(e)); } finally { setBusy(false); }
  };
  return <>
    <div className="script-tools"><button disabled={busy} onClick={() => { if (dirty) setError("请先保存，或放弃本次修改后返回"); else onBack(); }}>返回剧本库</button><span>{value.title} · {draft.title} · {dirty ? "未保存" : "已保存"}</span><button disabled={busy || !dirty} onClick={() => void save()}>保存原分集</button>{dirty && <button onClick={() => { if (window.confirm("放弃本次未保存修改？")) onBack(); }}>放弃修改</button>}</div>
    {error && <p role="alert" className="script-library-error">{error}</p>}
    <ScriptEditor episodeKey={`${value.pid}-${draft.id}`} scriptText={draft.script_text} scriptDoc={draft.script_doc} onChange={(text, doc) => { revision.current++; setDraft((d) => ({ ...d, script_text: text, script_doc: doc })); setDirty(true); }} />
  </>;
}
