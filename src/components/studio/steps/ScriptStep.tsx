/**
 * Step 1 · 剧本。
 *
 * 一个大文本框，粘小说或剧本。**保存是显式的** —— 自动保存会让
 * 「我改了什么」变得不可追，而这一段文字后面每一步都要用。
 *
 * 「提取实体」（剧本 → 角色/场景/道具）在第 5 期接上。
 */
import { useEffect, useRef, useState } from "react";
import {
  cinemaExtractEntities,
  cinemaSaveEpisode,
  type ExtractedEntity,
} from "../../../api/cinema";
import { importNote } from "../series/seriesCopy";
import { ExtractDialog } from "./ExtractDialog";
import { ScriptEditor } from "../script/ScriptEditor";
import { describeCounts, useToasts } from "../toastStore";
import type { StepProps } from "./types";

export function ScriptStep({ project, episode, reload, onError }: StepProps) {
  const [text, setText] = useState(episode?.script_text ?? "");
  const [doc, setDoc] = useState<string | null>(episode?.script_doc ?? null);
  const [title, setTitle] = useState(episode?.title ?? "");
  const [synopsis, setSynopsis] = useState(episode?.synopsis ?? "");
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);
  const [synopsisOpen, setSynopsisOpen] = useState(false);
  const [extracting, setExtracting] = useState(false);
  const [preview, setPreview] = useState<ExtractedEntity[] | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const queueRef = useRef<Promise<unknown>>(Promise.resolve());
  const activeRef = useRef("");
  activeRef.current = `${project?.id}/${episode?.id}`;
  const pushToast = useToasts((s) => s.push);
  const resolveToast = useToasts((s) => s.resolve);

  useEffect(() => {
    setText(episode?.script_text ?? "");
    setDoc(episode?.script_doc ?? null);
    setTitle(episode?.title ?? "");
    setSynopsis(episode?.synopsis ?? "");
    setSaved(false);
  }, [project?.id, episode?.id]);

  const dirty =
    !!episode &&
    (text !== episode.script_text ||
      doc !== (episode.script_doc ?? null) ||
      title !== episode.title ||
      synopsis !== episode.synopsis);

  /**
   * **自动保存。** 原来是一个「保存」按钮 —— 但剧本是边写边改的东西，
   * 让人记着按保存，只会制造「改了没存就点提取，抽的是旧文本」这种事
   * （提取读的是磁盘上那份）。停手 800ms 才写盘。
   *
   * **这两个 hook 必须在早退之前。** 放在下面的话，
   * 「还没有分集」那条路会少跑两个 hook ——
   * React 报 "Rendered more hooks than during the previous render"，
   * 整个窗口白屏。（我第一版就是这么挂的。）
   */
  const saveRef = useRef<() => Promise<boolean>>(async () => false);
  useEffect(() => {
    if (!dirty) return;
    const t = setTimeout(() => void saveRef.current(), 800);
    return () => clearTimeout(t);
  }, [dirty, text, doc, title, synopsis]);

  if (!project || !episode) {
    return <Empty>还没有分集</Empty>;
  }

  const save = async () => {
    const scope = `${project.id}/${episode.id}`;
    setBusy(true);
    onError(null);
    try {
      // **两个一起写。** 纯文本是真相（下游全读它），结构是附加的。
      const write = queueRef.current.catch(() => {}).then(() => cinemaSaveEpisode(project.id, {
        ...episode,
        title,
        synopsis,
        script_text: text,
        script_doc: doc,
      }));
      queueRef.current = write;
      await write;
      if (activeRef.current === scope) { await reload(); setSaved(true); }
      return true;
    } catch (e) {
      if (activeRef.current === scope) onError(String(e));
      return false;
    } finally {
      if (activeRef.current === scope) setBusy(false);
    }
  };

  /**
   * 提取实体。**只问模型。**
   *
   * 用户粘进来的是没有格式的散文 —— 那是常态。结构派生在这类文本上
   * 读不出任何东西，所以它**不能当门槛**：早先的版本先派生、读不到就
   * 报红字返回，压根不问模型，于是正常输入全被挡在门外。
   * （实测 LumenX 也是直接问模型的，它连结构派生都没有。）
   *
   * # 2026-09-12：结构派生整条拆掉了
   *
   * 中间有一版是「结构 + 模型」两份合并，理由是有格式的剧本里
   * `角色：台词` 的名字比模型确定。实际跑下来：
   *
   * - 它唯一的产出是三个假角色 —— 剧本开头的 `**类型**`
   *   `**时长建议**` `**基调**` 被自动标成了「角色」段落。
   * - 真正的角色模型本来就都抽得出来，没有一个是靠结构才找到的。
   * - 两条路意味着**每加一道过滤都要在两边各加一次**，而这件事当场
   *   就漏了：后端拦掉了假角色，结构那条没拦，它们照样出现在名单最前面。
   *
   * 用户的原话是「这个剧本就俩作用，一个粘贴进剧本内容，一个提取实体」。
   * 段落类型现在只管排版和场次目录，不参与任何下游判断。
   */
  const extract = async () => {
    // 模型读的是磁盘上那份，所以改了没存要先存
    if (dirty && !(await save())) return;
    setExtracting(true);
    setNote(null);
    onError(null);
    // **进度条就地变成结果，不是再推一条。** 各推一条的话，
    // 屏幕上会同时挂着「分析中」和「完成」。
    const tid = pushToast({
      kind: "progress",
      title: "智能分析中…",
      body: "AI 正在拆解剧本、抽取角色 / 场景 / 道具，通常 20~60 秒，期间可以继续写。",
    });
    try {
      const byModel = await cinemaExtractEntities(project.id, episode.id);
      // **只听模型。** 这里原来还会合进「结构派生」的一份
      // （按段落类型直接读出的名字）。砍掉的理由：
      //
      // 它这两天唯一的产出是三个假角色（`**类型**` `**时长建议**`
      // `**基调**` —— 剧本开头的 Markdown 元信息被自动标成了「角色」段落），
      // 而真正的角色模型本来就都抽得出来。两条路合并还意味着
      // **每加一道过滤都要在两边各加一次**，2026-09-12 就漏了一次：
      // 后端拦掉了，结构那条没拦，假角色照样出现在名单最前面。
      //
      // 这个文件早先的注释里就写着：实测 LumenX 也是直接问模型的，
      // 它连结构派生都没有。
      const merged = byModel;
      if (merged.length === 0) {
        resolveToast(tid, {
          kind: "error",
          title: "没读出东西",
          body: "模型没从这段剧本里找到角色或场景 —— 剧本里至少要有人物和地点。",
        });
        onError("模型没从这段剧本里读出角色或场景 —— 剧本里至少要有人物和地点");
        return;
      }
      resolveToast(tid, {
        kind: "success",
        title: "实体提取完成",
        body: describeCounts([
          { label: "角色", n: merged.filter((x) => x.kind === "character").length },
          { label: "场景", n: merged.filter((x) => x.kind === "scene").length },
          { label: "道具", n: merged.filter((x) => x.kind === "prop").length },
        ]),
      });
      setPreview(merged);
    } catch (e) {
      resolveToast(tid, { kind: "error", title: "提取失败", body: String(e) });
      onError(String(e));
    } finally {
      setExtracting(false);
    }
  };

  saveRef.current = save;


  return (
    <div className="flex h-full flex-col gap-3 px-7 py-5">
      <div className="flex flex-wrap items-center gap-2">
        <input
          value={title}
          onChange={(e) => setTitle(e.target.value)}
          placeholder="这一集叫什么"
          className="min-w-[180px] flex-1 rounded-lg px-3 py-2 text-[13px] outline-none"
          style={field}
        />
        <button
          type="button"
          onClick={() => setSynopsisOpen((v) => !v)}
          title="一句话梗概 —— 生成下一集时会用到"
          className="rounded-lg px-2.5 py-2 text-[11.5px]"
          style={{
            background: "var(--studio-bg-inset)",
            color: synopsis.trim() ? "var(--studio-text-2)" : "var(--studio-text-muted)",
          }}
        >
          梗概
        </button>
        <span
          className="font-mono text-[0.6rem]"
          style={{ color: "var(--studio-text-muted)" }}
        >
          {/* **只数字数。**
              这里原来还显示「N 场 · N 人」，数的是结构派生那一份 ——
              而抽实体 2026-09-12 起只听模型了，两个数字对不上。
              注释当时写的是「数的就是下游会拿到的那份」，现在那是句假话。
              对不上的数字比没有数字糟：它看起来像个承诺。 */}
          {text.length} 字
          {busy ? " · 保存中" : saved && !dirty ? " · 已保存" : ""}
        </span>
        {/* **只有一个提取按钮。**
            用不用模型是实现细节，不该变成两个按钮让人选 ——
            默认直接从结构读（快且确定），要不要让模型再补一遍，
            在弹窗里问。 */}
        {/* **第 1 步唯一的动作，所以一直是主按钮。**
            以前它在「读不出结构」时会变灰 —— 而那正是最需要点它的时候。 */}
        <button
          type="button"
          disabled={!text.trim() || extracting}
          title={text.trim() ? "让 AI 从剧本里读出角色 / 场景 / 道具" : "先写点剧本"}
          onClick={() => void extract()}
          className="flex items-center gap-1.5 rounded-lg px-3.5 py-2 text-[12px] font-medium"
          style={{
            background: text.trim() ? "var(--studio-primary)" : "var(--studio-bg-inset)",
            color: text.trim() ? "var(--studio-on-primary)" : "var(--studio-text-muted)",
            opacity: extracting ? 0.75 : 1,
          }}
        >
          <svg
            aria-hidden="true"
            width="13"
            height="13"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="1.8"
            strokeLinecap="round"
            strokeLinejoin="round"
            style={{
              animation: extracting ? "studio-spin 900ms linear infinite" : undefined,
            }}
          >
            <path d="M12 3l1.9 5.1L19 10l-5.1 1.9L12 17l-1.9-5.1L5 10l5.1-1.9z" />
          </svg>
          {extracting ? "智能分析中…" : "提取实体"}
        </button>
      </div>

      {note && (
        <p className="text-[11.5px]" style={{ color: "var(--studio-text-2)" }}>
          {note}
        </p>
      )}

      {synopsisOpen && (
        <input
          autoFocus
          value={synopsis}
          onChange={(e) => setSynopsis(e.target.value)}
          placeholder="一句话梗概 —— 生成下一集时会读它"
          className="rounded-lg px-3 py-2 text-[13px] outline-none"
          style={field}
        />
      )}

      {preview && (
        <ExtractDialog
          projectId={project.id}
          episodeId={episode.id}
          entities={preview}
          onClose={() => setPreview(null)}
          onImported={(s) => {
            setPreview(null);
            setNote(importNote(s));
          }}
        />
      )}

      <ScriptEditor
        key={`${project.id}/${episode.id}`}
        episodeKey={`${project.id}-${episode.id}`}
        scriptText={episode.script_text}
        scriptDoc={episode.script_doc}
        onChange={(t, d) => {
          setText(t);
          setDoc(d);
        }}
      />
    </div>
  );
}

function Empty({ children }: { children: React.ReactNode }) {
  return (
    <p className="px-7 py-6 text-[13px]" style={{ color: "var(--studio-text-muted)" }}>
      {children}
    </p>
  );
}

const field: React.CSSProperties = {
  background: "var(--studio-bg-inset)",
  border: "1px solid var(--studio-border-subtle)",
  color: "var(--studio-text)",
};
