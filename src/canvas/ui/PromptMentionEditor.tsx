import {
  forwardRef,
  useCallback,
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ClipboardEvent as ReactClipboardEvent,
  type KeyboardEvent as ReactKeyboardEvent,
  type MouseEvent as ReactMouseEvent,
} from "react";
import { createPortal } from "react-dom";
import { mediaSrc } from "../../components/studio/MediaThumb";
import "./PromptMentionEditor.css";

export interface MentionCandidate {
  key: string;
  name: string; // e.g. "图片1"
  imageUrl?: string;
  index: number;
  displayName?: string; // e.g. "角色名" or "参考图"
}

export interface PromptMentionEditorHandle {
  insertTextAtCursor: (text: string) => void;
  insertMentionAtCursor: (candidate: MentionCandidate) => void;
  focus: () => void;
}

interface PromptMentionEditorProps {
  value: string;
  onChange: (next: string) => void;
  candidates: MentionCandidate[];
  placeholder?: string;
  className?: string;
  rows?: number;
  /**
   * 最多长到几行，超了就内部滚动。**默认 10 行。**
   *
   * 不设上限的话，长提示词会把整个操作面板撑开，**底部那排参数被顶出屏幕**
   * —— H3 的三段格式一千多字，实测能撑到看不见「时长 / 画幅 / 生成」
   * （用户 2026-09-21 撞到）。
   */
  maxRows?: number;
  onKeyDown?: (event: React.KeyboardEvent<HTMLDivElement>) => void;
  onBlur?: () => void;
}

export function resolveDisplayImageUrl(urlOrPath?: string): string {
  if (!urlOrPath) return "";
  if (
    urlOrPath.startsWith("http://") ||
    urlOrPath.startsWith("https://") ||
    urlOrPath.startsWith("data:") ||
    urlOrPath.startsWith("anima-media://")
  ) {
    return urlOrPath;
  }
  return mediaSrc(urlOrPath);
}

function escapeRegex(input: string): string {
  return input.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function buildChipElement(candidate: MentionCandidate): HTMLElement {
  const span = document.createElement("span");
  span.contentEditable = "false";
  span.dataset.mention = candidate.key;
  span.dataset.name = candidate.name;
  span.className = "mention-chip";
  span.title = candidate.displayName ? `${candidate.name} · ${candidate.displayName}` : candidate.name;

  const displayUrl = resolveDisplayImageUrl(candidate.imageUrl);
  if (displayUrl) {
    span.dataset.imageUrl = displayUrl;
    const img = document.createElement("img");
    img.src = displayUrl;
    img.alt = "";
    img.draggable = false;
    span.appendChild(img);
  }

  const labelEl = document.createElement("span");
  labelEl.className = "mention-chip-label";
  labelEl.textContent = `@${candidate.name}`;
  span.appendChild(labelEl);

  return span;
}

function appendTextWithLineBreaks(root: HTMLElement, text: string): void {
  const parts = text.split("\n");
  parts.forEach((part, idx) => {
    if (part.length > 0) {
      root.appendChild(document.createTextNode(part));
    }
    if (idx < parts.length - 1) {
      root.appendChild(document.createElement("br"));
    }
  });
}

function selectionBelongsTo(root: HTMLElement, selection: Selection): boolean {
  const anchor = selection.anchorNode;
  const focus = selection.focusNode;
  return Boolean(
    anchor && root.contains(anchor) && (!focus || root.contains(focus))
  );
}

function rangeAtEndOf(root: HTMLElement): Range {
  const range = document.createRange();
  range.selectNodeContents(root);
  range.collapse(false);
  return range;
}

function rebuildDOM(root: HTMLElement, text: string, candidates: MentionCandidate[]): void {
  while (root.firstChild) {
    root.removeChild(root.firstChild);
  }
  if (!text) return;

  const names = candidates
    .map((c) => c.name)
    .filter((n) => n.length > 0)
    .sort((a, b) => b.length - a.length);

  // 同时也支持对已经存在的 @图片\d+ 模式匹配
  const pattern = names.length > 0
    ? new RegExp("@(" + names.map(escapeRegex).join("|") + "|图片\\d+)", "g")
    : /@(图片\d+)/g;

  let lastIndex = 0;
  let match: RegExpExecArray | null;
  while ((match = pattern.exec(text)) !== null) {
    if (match.index > lastIndex) {
      appendTextWithLineBreaks(root, text.slice(lastIndex, match.index));
    }
    const name = match[1];
    const candidate = candidates.find((c) => c.name === name) || {
      key: name,
      name,
      index: Number(name.replace(/\D/g, "")) || 1,
    };
    root.appendChild(buildChipElement(candidate));
    lastIndex = match.index + match[0].length;
  }
  if (lastIndex < text.length) {
    appendTextWithLineBreaks(root, text.slice(lastIndex));
  }
}

function serialize(root: HTMLElement): string {
  let out = "";
  const walk = (node: Node): void => {
    if (node.nodeType === Node.TEXT_NODE) {
      out += node.textContent ?? "";
      return;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return;
    const el = node as HTMLElement;
    if (el.dataset.mention) {
      out += "@" + (el.dataset.name ?? "");
      return;
    }
    if (el.tagName === "BR") {
      out += "\n";
      return;
    }
    if (el.tagName === "DIV") {
      if (out.length > 0 && !out.endsWith("\n")) out += "\n";
      for (const child of Array.from(el.childNodes)) walk(child);
      return;
    }
    for (const child of Array.from(el.childNodes)) walk(child);
  };
  for (const child of Array.from(root.childNodes)) walk(child);
  return out;
}

interface MentionContext {
  textNode: Text;
  atOffset: number;
  caretOffset: number;
  query: string;
  rect: DOMRect;
}

function detectMention(): MentionContext | null {
  const sel = window.getSelection();
  if (!sel || sel.rangeCount === 0) return null;
  const range = sel.getRangeAt(0);
  if (!range.collapsed) return null;
  const node = range.startContainer;
  if (node.nodeType !== Node.TEXT_NODE) return null;
  const textNode = node as Text;
  const caretOffset = range.startOffset;
  const textBefore = (textNode.textContent ?? "").slice(0, caretOffset);
  const match = textBefore.match(/@([^\s@]*)$/);
  if (!match) return null;
  const atIndex = textBefore.length - match[0].length;
  const rect = range.getBoundingClientRect();
  return {
    textNode,
    atOffset: atIndex,
    caretOffset,
    query: match[1],
    rect,
  };
}

export const PromptMentionEditor = forwardRef<PromptMentionEditorHandle, PromptMentionEditorProps>(
  function PromptMentionEditor(
    {
      value,
      onChange,
      candidates,
      placeholder,
      className,
      rows = 3,
      maxRows = 10,
      onKeyDown,
      onBlur,
    },
    ref,
  ) {
    const editorRef = useRef<HTMLDivElement | null>(null);
    const lastSerializedRef = useRef<string>("");
    const isComposingRef = useRef(false);
    const [mention, setMention] = useState<MentionContext | null>(null);
    const [activeIdx, setActiveIdx] = useState(0);
    const [hover, setHover] = useState<{ imageUrl: string; rect: DOMRect } | null>(null);
    const popoverRef = useRef<HTMLDivElement | null>(null);

    useLayoutEffect(() => {
      const el = editorRef.current;
      if (!el) return;
      if (value === lastSerializedRef.current) return;
      rebuildDOM(el, value, candidates);
      lastSerializedRef.current = value;
    }, [value, candidates]);

    const commitChange = useCallback(() => {
      const el = editorRef.current;
      if (!el) return;
      const next = serialize(el);
      if (next === lastSerializedRef.current) return;
      lastSerializedRef.current = next;
      onChange(next);
    }, [onChange]);

    const insertMentionAtCursor = useCallback(
      (candidate: MentionCandidate) => {
        const el = editorRef.current;
        if (!el) return;

        // **先看光标在不在框里，再 focus。** 反过来的话 focus 会把光标放到开头，
        // 「没光标就接在末尾」那条永远走不到 —— 从运镜窗口点进来的标签全插到了最前面（2026-09-25）
        const before = window.getSelection();
        const inside = before && before.rangeCount > 0 && selectionBelongsTo(el, before) ? before.getRangeAt(0).cloneRange() : null;
        el.focus();
        const selection = window.getSelection();
        const range = inside ?? rangeAtEndOf(el);
        range.deleteContents();
        const chip = buildChipElement(candidate);
        range.insertNode(chip);

        const space = document.createTextNode(" ");
        chip.parentNode?.insertBefore(space, chip.nextSibling);

        if (selection) {
          const after = document.createRange();
          after.setStartAfter(space);
          after.collapse(true);
          selection.removeAllRanges();
          selection.addRange(after);
        }
        setMention(null);
        commitChange();
      },
      [commitChange],
    );

    const insertTextAtCursor = useCallback(
      (text: string) => {
        const el = editorRef.current;
        if (!el || text.length === 0) return;

        el.focus();
        const selection = window.getSelection();
        const range =
          selection && selection.rangeCount > 0 && selectionBelongsTo(el, selection)
            ? selection.getRangeAt(0).cloneRange()
            : rangeAtEndOf(el);

        range.deleteContents();
        const node = document.createTextNode(text);
        range.insertNode(node);

        if (selection) {
          const after = document.createRange();
          after.setStartAfter(node);
          after.collapse(true);
          selection.removeAllRanges();
          selection.addRange(after);
        }
        setMention(null);
        commitChange();
      },
      [commitChange],
    );

    useImperativeHandle(
      ref,
      () => ({
        insertTextAtCursor,
        insertMentionAtCursor,
        focus: () => editorRef.current?.focus(),
      }),
      [insertTextAtCursor, insertMentionAtCursor],
    );

    const filtered = useMemo(() => {
      if (!mention) return candidates;
      const q = mention.query.toLowerCase();
      if (!q) return candidates;
      return candidates.filter(
        (c) =>
          c.name.toLowerCase().includes(q) ||
          (c.displayName && c.displayName.toLowerCase().includes(q)),
      );
    }, [mention, candidates]);

    useEffect(() => {
      setActiveIdx(0);
    }, [mention?.query]);

    const handleInput = useCallback(() => {
      if (isComposingRef.current) return;
      commitChange();
      setMention(detectMention());
    }, [commitChange]);

    const handlePaste = useCallback((event: ReactClipboardEvent<HTMLDivElement>) => {
      event.preventDefault();
      const plain = event.clipboardData.getData("text/plain");
      if (!plain) return;
      document.execCommand("insertText", false, plain);
    }, []);

    const insertChip = useCallback(
      (candidate: MentionCandidate) => {
        const ctx = mention;
        const el = editorRef.current;
        if (!ctx || !el) return;
        const sel = window.getSelection();
        if (!sel) return;

        const range = document.createRange();
        range.setStart(ctx.textNode, ctx.atOffset);
        const currentRange = sel.rangeCount > 0 ? sel.getRangeAt(0) : null;
        if (
          currentRange &&
          currentRange.endContainer === ctx.textNode &&
          currentRange.endOffset >= ctx.atOffset
        ) {
          range.setEnd(currentRange.endContainer, currentRange.endOffset);
        } else {
          range.setEnd(ctx.textNode, ctx.caretOffset);
        }
        range.deleteContents();
        const chip = buildChipElement(candidate);
        range.insertNode(chip);

        const space = document.createTextNode(" ");
        chip.parentNode?.insertBefore(space, chip.nextSibling);

        const after = document.createRange();
        after.setStartAfter(space);
        after.collapse(true);
        sel.removeAllRanges();
        sel.addRange(after);

        setMention(null);
        commitChange();
      },
      [mention, commitChange],
    );

    const handleKeyDown = useCallback(
      (event: ReactKeyboardEvent<HTMLDivElement>) => {
        const popoverOpen = Boolean(mention) && filtered.length > 0;
        if (popoverOpen) {
          if (event.key === "ArrowDown") {
            event.preventDefault();
            setActiveIdx((i) => (i + 1) % filtered.length);
            return;
          }
          if (event.key === "ArrowUp") {
            event.preventDefault();
            setActiveIdx((i) => (i - 1 + filtered.length) % filtered.length);
            return;
          }
          if (event.key === "Enter") {
            event.preventDefault();
            insertChip(filtered[activeIdx]);
            return;
          }
          if (event.key === "Escape") {
            event.preventDefault();
            setMention(null);
            return;
          }
        }
        if (event.key === "Escape") {
          setHover(null);
        }
        onKeyDown?.(event);
      },
      [mention, filtered, activeIdx, insertChip, onKeyDown],
    );

    const handleMouseOver = useCallback((event: ReactMouseEvent<HTMLDivElement>) => {
      const target = (event.target as HTMLElement | null)?.closest(".mention-chip");
      if (!(target instanceof HTMLElement)) {
        setHover(null);
        return;
      }
      const imageUrl = target.dataset.imageUrl;
      if (!imageUrl) return;
      setHover({ imageUrl, rect: target.getBoundingClientRect() });
    }, []);

    const handleMouseOut = useCallback((event: ReactMouseEvent<HTMLDivElement>) => {
      const target = (event.target as HTMLElement | null)?.closest(".mention-chip");
      if (!(target instanceof HTMLElement)) return;
      const related = event.relatedTarget as Node | null;
      if (related && target.contains(related)) return;
      setHover(null);
    }, []);

    const popoverStyle = useMemo(() => {
      const rect = mention?.rect ?? null;
      if (!rect) return null;
      return {
        top: rect.bottom + 4,
        left: rect.left,
      };
    }, [mention]);

    const previewStyle = useMemo(() => {
      if (!hover) return null;
      const left = Math.min(Math.max(8, hover.rect.left), window.innerWidth - 160);
      const top = hover.rect.top - 8;
      return { left, top };
    }, [hover]);

    // 点击外部关闭 mention popover
    useEffect(() => {
      if (!mention) return;
      const onDocMouseDown = (event: MouseEvent) => {
        const target = event.target as Node | null;
        if (popoverRef.current && target && popoverRef.current.contains(target)) {
          return;
        }
        setMention(null);
      };
      document.addEventListener("mousedown", onDocMouseDown, true);
      return () => document.removeEventListener("mousedown", onDocMouseDown, true);
    }, [mention]);

    return (
      <>
        <div
          ref={editorRef}
          contentEditable
          suppressContentEditableWarning
          className={`prompt-mention-editor nodrag nopan nowheel cursor-text ${className ?? ""}`}
          // **撑到上限就内部滚动。** `nowheel` 已经在 className 里 ——
          // 没有它的话在编辑区里滚滚轮会变成缩放画布。
          //
          // `overflowX: "hidden"` **不能省**。CSS 规范：`overflow-x` / `overflow-y`
          // 里只要有一个不是 `visible`，另一个的 `visible` 就**计算成 `auto`** ——
          // 所以光写 `overflowY: "auto"` 会让横向也变成 `auto`，内容差一个像素
          // 溢出就冒出一条横向滚动条（2026-09-21 用户撞到，是 0.31.1 带出来的）。
          // 文字本来就 `pre-wrap` + `break-word` 会换行，横向永远不需要滚。
          style={{
            minHeight: `${rows * 24}px`,
            maxHeight: `${maxRows * 24}px`,
            overflowY: "auto",
            overflowX: "hidden",
          }}
          data-placeholder={placeholder ?? ""}
          spellCheck={false}
          onInput={handleInput}
          onPaste={handlePaste}
          onCompositionStart={() => {
            isComposingRef.current = true;
          }}
          onCompositionEnd={() => {
            isComposingRef.current = false;
            commitChange();
            setMention(detectMention());
          }}
          onKeyDown={handleKeyDown}
          onBlur={onBlur}
          onMouseOver={handleMouseOver}
          onMouseOut={handleMouseOut}
        />

        {/* @ 提及候选悬浮列表 */}
        {mention && popoverStyle && filtered.length > 0 &&
          createPortal(
            <div
              ref={popoverRef}
              className="fixed z-[10000] flex min-w-[220px] max-w-[320px] flex-col overflow-y-auto rounded-lg border border-white/15 bg-zinc-900/95 p-1 shadow-2xl backdrop-blur-md"
              style={{
                top: popoverStyle.top,
                left: popoverStyle.left,
                maxHeight: 240,
              }}
              onMouseDown={(event) => event.preventDefault()}
            >
              <div className="px-2 py-1 text-[11px] font-medium text-white/40 border-b border-white/10 mb-1">
                引用参考素材
              </div>
              {filtered.map((candidate, idx) => {
                const imgUrl = resolveDisplayImageUrl(candidate.imageUrl);
                return (
                  <button
                    key={candidate.key}
                    type="button"
                    onMouseDown={(event) => {
                      event.preventDefault();
                      event.stopPropagation();
                      insertChip(candidate);
                    }}
                    onMouseEnter={() => setActiveIdx(idx)}
                    className={`flex items-center gap-2.5 rounded-md px-2.5 py-1.5 text-left text-xs transition-colors ${
                      idx === activeIdx
                        ? "bg-blue-600/30 text-white"
                        : "text-white/70 hover:bg-white/10 hover:text-white"
                    }`}
                  >
                    {imgUrl ? (
                      <img
                        src={imgUrl}
                        alt=""
                        className="h-8 w-8 shrink-0 rounded object-cover border border-white/10"
                        draggable={false}
                      />
                    ) : (
                      <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded bg-white/10 text-white/50">
                        📷
                      </div>
                    )}
                    <div className="flex flex-col min-w-0 flex-1">
                      <span className="font-medium truncate text-white">
                        @{candidate.name}
                      </span>
                      {candidate.displayName && (
                        <span className="text-[10px] text-white/50 truncate">
                          {candidate.displayName}
                        </span>
                      )}
                    </div>
                    {/* 运镜没有编号（index 0）—— 它不占 <Picture N> */}
                    {candidate.index > 0 && (
                      <span className="text-[11px] font-mono text-white/40">
                        #{candidate.index}
                      </span>
                    )}
                  </button>
                );
              })}
            </div>,
            document.body,
          )}

        {/* 悬停大图预览浮层 */}
        {hover && previewStyle &&
          createPortal(
            <div
              className="pointer-events-none fixed z-[10001] -translate-y-full overflow-hidden rounded-lg border border-white/20 bg-zinc-900/95 p-1 shadow-2xl backdrop-blur-md"
              style={{
                left: previewStyle.left,
                top: previewStyle.top,
                width: 160,
              }}
            >
              <img
                src={hover.imageUrl}
                alt=""
                className="block h-auto max-h-[220px] w-full rounded object-contain"
                draggable={false}
              />
            </div>,
            document.body,
          )}
      </>
    );
  },
);
