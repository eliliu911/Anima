import { useEffect, useRef } from "react";
import { affects, onStudioChanged, type StudioChangeKind } from "../../api/studioEvents";

/**
 * 听 Studio 数据改动，对得上就调 `onChange`（通常是页面的 reload）。
 *
 * 回调放 ref：它每次渲染都是新函数，进依赖会反复重新订阅。
 */
export function useStudioChanged(onChange: () => void, opts: { kinds?: StudioChangeKind[]; projectId?: string } = {}) {
  const cb = useRef(onChange);
  cb.current = onChange;
  const kinds = opts.kinds?.join(",") ?? "";
  const projectId = opts.projectId;
  useEffect(() => {
    let alive = true;
    let off: (() => void) | undefined;
    void onStudioChanged((c) => {
      if (alive && affects(c, { kinds: kinds ? (kinds.split(",") as StudioChangeKind[]) : undefined, projectId })) cb.current();
    }).then((f) => {
      if (alive) off = f;
      else f();
    });
    return () => {
      alive = false;
      off?.();
    };
  }, [kinds, projectId]);
}
