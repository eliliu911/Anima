import { useEffect, useRef, useState } from "react";
import type { PolishKind } from "../../api/polish";
import { useCanvasStore } from "../store/useCanvasStore";
import { polishCanvasPrompt } from "./polishPrompt";

/**
 * 画布节点的「AI 润色」。
 *
 * `refs` 是这个节点**实际会发出去的参考图叫什么名字**，按顺序。
 * 它走 `{SLOTS}`（参考生视频）/ `{ASSETS}`（出图）进提示词 ——
 * 参考生视频那一份提示词的任务就是「把草稿里提到的资产补成
 * `[角色:名字]` 标签」，不给名单的话模型只能瞎猜，补出来的标签
 * 解析不了，方括号会原样进引擎被画进画面
 * （2026-09-19 Codex Astra 的第三份审查抓到：调用只传了草稿、引擎和类型）。
 */
export function useCanvasPolish(
  nodeId: string,
  draft: string,
  engine: string,
  kind: PolishKind,
  apply: (text: string) => void,
  refs?: string[],
) {
  const [isPolishing, setIsPolishing] = useState(false);
  const [polishMessage, setMessage] = useState("");
  const busy = useRef(false);
  const mounted = useRef(true);
  const refsKey = (refs ?? []).join("\u0000");
  const latest = useRef({ draft, engine, kind, refsKey });
  latest.current = { draft, engine, kind, refsKey };
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);

  const handlePolish = async () => {
    if (busy.current || !draft.trim()) return;
    busy.current = true;
    setIsPolishing(true); setMessage("");
    const store = useCanvasStore.getState();
    const projectId = store.activeProjectId;
    const original = store.nodes.find((node) => node.id === nodeId)?.data;
    let changed = false;
    // Even switching away and back invalidates this panel's pending response.
    const unsubscribe = useCanvasStore.subscribe((state) => {
      if (state.activeProjectId !== projectId || state.nodes.find((node) => node.id === nodeId)?.data !== original) changed = true;
    });
    try {
      const named = (refs ?? []).filter((r) => r.trim());
      const message = await polishCanvasPrompt(
        {
          draft: draft.trim(),
          engine,
          kind,
          // 两个键名后端是 `assets.or(slots)`，给同一份名单即可；
          // 按 kind 分开传是为了让调用点读起来和提示词里的占位符对得上。
          ...(named.length ? (kind === "r2v" ? { slots: named } : { assets: named }) : {}),
        },
        () =>
          mounted.current &&
          !changed &&
          latest.current.draft === draft &&
          latest.current.engine === engine &&
          latest.current.kind === kind &&
          latest.current.refsKey === refsKey,
        apply,
      );
      if (mounted.current) setMessage(message);
    } catch (error) {
      if (mounted.current) setMessage(`润色失败，原文已保留：${String(error)}`);
    } finally {
      unsubscribe(); busy.current = false;
      if (mounted.current) setIsPolishing(false);
    }
  };
  return { isPolishing, polishMessage, handlePolish };
}
