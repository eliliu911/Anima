import { useMemo, useRef, useState } from "react";
import { EditorContent, useEditor } from "@tiptap/react";
import Document from "@tiptap/extension-document";
import Text from "@tiptap/extension-text";
import { Fragment, Slice } from "@tiptap/pm/model";
import { Placeholder, UndoRedo } from "@tiptap/extensions";
import { Upload, Download, RotateCcw, FileText, ChevronRight } from "../../../canvas/ui/icons";
import { scriptNodes } from "./nodes";
import { docToText, loadDoc, serializeDoc, textToDoc } from "./docConvert";
import { derive } from "./derive";

export function ScriptEditor({ scriptText, scriptDoc, episodeKey, onChange }: {
  scriptText: string; scriptDoc: string | null | undefined; episodeKey: string;
  onChange: (text: string, doc: string) => void;
}) {
  const initial = useMemo(() => loadDoc(scriptDoc, scriptText), [episodeKey]);
  const [revision, setRevision] = useState(0);
  const [outline, setOutline] = useState(true);
  const [query, setQuery] = useState("");
  const [pending, setPending] = useState<{ name: string; text: string } | null>(null);
  const [importMode, setImportMode] = useState<"script" | "prose">("script");
  const [message, setMessage] = useState("");
  const input = useRef<HTMLInputElement>(null);
  const keyRef = useRef(episodeKey);
  keyRef.current = episodeKey;
  const editor = useEditor({
    extensions: [Document, Text, UndoRedo, Placeholder.configure({ placeholder: "剧本正文" }), ...scriptNodes],
    content: initial,
    immediatelyRender: true,
    editorProps: {
      attributes: { class: "script-doc" },
      handlePaste(view, event) {
        const text = event.clipboardData?.getData("text/plain");
        if (!text?.includes("\n")) return false;
        const nodes = textToDoc(text).content.map((n) => view.state.schema.nodeFromJSON(n));
        // One transaction keeps paste ordering and undo atomic.
        view.dispatch(view.state.tr.replaceSelection(new Slice(Fragment.fromArray(nodes), 0, 0)).scrollIntoView());
        return true;
      },
    },
    onUpdate({ editor }) {
      const json = editor.getJSON();
      onChange(docToText(json), serializeDoc(json));
      setRevision((n) => n + 1);
    },
    onSelectionUpdate() { setRevision((n) => n + 1); },
  }, [episodeKey]);
  const doc = useMemo(() => loadDoc(editor ? JSON.stringify(editor.getJSON()) : null, scriptText), [editor, revision]);
  // **只留场次目录这一处用到结构。** 它是长剧本里的跳转导航，
  // 读一读就走，不参与任何下游判断 —— 抽实体 2026-09-12 起只听模型。
  const scenes = derive(doc).scenes.filter((s) => s.heading);
  const jump = (index: number) => {
    let position = 0;
    for (let i = 0; i < index; i++) position += editor?.state.doc.child(i).nodeSize ?? 0;
    editor?.chain().focus().setTextSelection(position + 1).scrollIntoView().run();
  };
  const search = () => {
    if (!editor || !query.trim()) return;
    const hits: number[] = [];
    editor.state.doc.descendants((node, pos) => {
      if (node.isText) {
        let from = 0;
        while ((from = (node.text ?? "").indexOf(query, from)) >= 0) { hits.push(pos + from); from += query.length; }
      }
    });
    if (!hits.length) { setMessage("未找到匹配文本"); return; }
    const pos = hits.find((p) => p >= editor.state.selection.to) ?? hits[0];
    editor.chain().focus().setTextSelection({ from: pos, to: pos + query.length }).scrollIntoView().run();
    setMessage(`${hits.length} 处匹配`);
  };
  const exportText = () => {
    const url = URL.createObjectURL(new Blob([docToText(doc)], { type: "text/plain;charset=utf-8" }));
    const a = document.createElement("a"); a.href = url; a.download = `${episodeKey}.md`; a.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  };
  return <div className="script-workbench">
    <div className="script-tools">
      <button title="场次目录" onClick={() => setOutline(!outline)} aria-pressed={outline}><FileText /></button>
      <button title="撤销" disabled={!editor?.can().undo()} onClick={() => { editor?.chain().focus().undo().run(); setRevision((n) => n + 1); }}><RotateCcw /></button>
      <button title="重做" disabled={!editor?.can().redo()} onClick={() => { editor?.chain().focus().redo().run(); setRevision((n) => n + 1); }}><RotateCcw style={{ transform: "scaleX(-1)" }} /></button>
      <input aria-label="搜索剧本" placeholder="搜索正文" value={query} onChange={(e) => setQuery(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") search(); }} />
      <button title="查找下一处" onClick={search}><ChevronRight /></button>
      <span className="script-tool-spacer" />
      <button onClick={() => { setPending({ name: "重新识别当前剧本", text: docToText(doc) }); setImportMode("script"); }}>识别结构</button>
      <button title="导入 Markdown 或文本" onClick={() => input.current?.click()}><Upload />导入</button>
      <button title="导出当前剧本" onClick={exportText}><Download /></button>
      <input hidden ref={input} type="file" accept=".md,.txt" onChange={async (e) => {
        const file = e.target.files?.[0]; e.target.value = "";
        if (!file) return;
        const key = episodeKey;
        try {
          if (file.size > 5 * 1024 * 1024) throw new Error("单文件暂限 5 MB");
          const text = await file.text();
          if (keyRef.current === key) { setPending({ name: file.name, text }); setImportMode("script"); }
        } catch (error) { setMessage(String(error)); }
      }} />
    </div>
    {message && <div className="script-notice" role="status">{message}</div>}
    {pending && <div className="script-import" role="dialog" aria-label="导入预览">
      <strong>{pending.name}</strong>
      <select aria-label="导入类型" value={importMode} onChange={(e) => setImportMode(e.target.value as "script" | "prose")}><option value="script">已有剧本</option><option value="prose">小说原文（不拆对白）</option></select>
      <pre>{pending.text}</pre>
      <p>替换当前分集正文，原文件不变。应用后可撤销。</p>
      <button onClick={() => setPending(null)}>取消</button>
      <button onClick={() => {
        const next = importMode === "script" ? textToDoc(pending.text) : { type: "doc", content: pending.text.split(/\r?\n/).map((text) => ({ type: "action", content: text ? [{ type: "text", text }] : [] })) };
        editor?.chain().focus().selectAll().insertContent(next.content).run(); setPending(null);
      }}>确认替换</button>
    </div>}
    <div className="script-layout">
      {outline && <aside className="script-outline">
        <h3>场次 <span>{scenes.length}</span></h3>
        {scenes.map((s) => <button key={s.from} onClick={() => jump(s.from)}><span>{s.heading}</span><small>{s.characters.length} 人 · {s.dialogueCount} 句</small></button>)}
        {!scenes.length && <p>尚未标注场次</p>}
      </aside>}
      <div className="script-paper studio-scroll"><EditorContent editor={editor} /></div>

    </div>
  </div>;
}
