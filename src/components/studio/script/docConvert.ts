/**
 * 纯文本 ↔ 编辑器文档 的双向转换。
 *
 * **纯文本是真相**（`episode.script_text`），下游全部读它；
 * 结构文档是附加的。所以这两个方向都必须无损到「再转一次还是同一份」。
 */
import { parseScript, speakerInfo, type LineType } from "./format";

/** ProseMirror 文档的最小形状 —— 只用到我们自己这六种块。 */
export interface DocNode {
  type: string;
  attrs?: { name: string; delivery: string; audioType: string };
  content?: { type: "text"; text: string }[];
}
export interface ScriptDoc {
  type: "doc";
  content: DocNode[];
}

const EMPTY: ScriptDoc = { type: "doc", content: [{ type: "action" }] };

/** 纯文本 → 文档。走行识别那一套。 */
export function textToDoc(text: string): ScriptDoc {
  const lines = parseScript(text);
  if (lines.length === 0) return EMPTY;
  return {
    type: "doc",
    content: lines.map((l) => ({
      type: l.type,
      ...(l.type === "characterCue" ? { attrs: speakerInfo(l.text) } : {}),
      ...(l.text ? { content: [{ type: "text" as const, text: l.text }] } : {}),
    })),
  };
}

/**
 * 文档 → 纯文本。
 *
 * **角色名和对白之间不空行**，场景之间空一行 —— 这样再 `textToDoc`
 * 一次能得到同样的结构（空行会断开上下文，夹在角色名和台词之间
 * 会让台词退化成动作）。
 */
export function docToText(doc: unknown): string {
  const content = (doc as ScriptDoc | null)?.content;
  if (!Array.isArray(content)) return "";
  const out: string[] = [];
  let prev: string | null = null;
  for (const node of content) {
    const text = (node.content ?? []).map((c) => c.text ?? "").join("").trim();
    if (!text) continue;
    const type = node.type as LineType;
    // 场景标题和转场前面空一行，读起来有段落感
    if (out.length > 0 && (type === "sceneHeading" || type === "transition")) {
      out.push("");
    } else if (
      out.length > 0 &&
      type === "action" &&
      (prev === "dialogue" || prev === "parenthetical")
    ) {
      // 一段对白结束、回到叙述 —— 这里要空行，否则再解析时
      // 这段动作会被当成上一句台词的延续
      out.push("");
    } else if (
      out.length > 0 &&
      type === "characterCue" &&
      (prev === "action" || prev === "dialogue" || prev === "parenthetical")
    ) {
      // 新角色名前面必须空行。**紧贴在上一句台词下面的话，
      // 再解析时分不出它是新角色还是那句台词的延续。**
      out.push("");
    }
    out.push(text);
    prev = type;
  }
  return out.join("\n");
}

/** 编辑器给的 JSON（可能带更多字段）转成我们要存的最小形状。 */
export function normalizeDoc(raw: unknown): ScriptDoc {
  const content = (raw as ScriptDoc | null)?.content;
  if (!Array.isArray(content) || content.length === 0) return EMPTY;
  return {
    type: "doc",
    content: content.map((n) => {
      const text = (n.content ?? []).map((c) => c.text ?? "").join("");
      return {
        type: n.type,
        ...(n.type === "characterCue" ? { attrs: speakerInfo(text) } : {}),
        ...(text ? { content: [{ type: "text" as const, text }] } : {}),
      };
    }),
  };
}

/** 存进 `episode.script_doc` 的字符串。 */
export function serializeDoc(doc: unknown): string {
  return JSON.stringify(normalizeDoc(doc));
}

/**
 * 从 `episode` 恢复文档。
 *
 * `script_doc` 有就用它；没有（老数据、或者别处直接写的纯文本）
 * 就把 `script_text` 解析成结构 —— **不能因为没有 JSON 就给个空文档**，
 * 那样老剧本一打开就没了。
 */
export function loadDoc(scriptDoc: string | null | undefined, scriptText: string): ScriptDoc {
  if (scriptDoc && scriptDoc.trim()) {
    try {
      const parsed = JSON.parse(scriptDoc);
      // **看原始内容长度，不是 normalize 之后的** ——
      // normalizeDoc 对空文档会补一个空块，长度永远 > 0，
      // 于是「结构是空的」这条永远走不到回退。
      const rawLen = (parsed as { content?: unknown[] } | null)?.content?.length ?? 0;
      if (rawLen > 0) return normalizeDoc(parsed);
    } catch {
      // JSON 坏了就退回纯文本 —— 剧本在，不能因为结构读不出来就丢内容
    }
  }
  return textToDoc(scriptText);
}
