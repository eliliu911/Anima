import { useEffect, useMemo, useState } from "react";
import { assetsList, KIND_LABEL, type AssetKind, type AssetView } from "../../api/assets";
import { useCanvasStore } from "../store/useCanvasStore";
import { Layers, ChevronRight, ChevronDown } from "./icons";
import "./CanvasAssetLibrary.css";

/**
 * 画布左侧的资产库。
 *
 * # 为什么非有不可
 *
 * 画布和资产库以前是**单向**的：能把画布出的图存进库，但**不能从库里取出来用**。
 * 于是每次要用一个已有角色，都得重新上传一遍图 —— 而那张图和库里那份
 * 从此是两个东西，改了一个另一个不动。
 *
 * 拖进画布建的是**绑定**，不是拷贝。
 *
 * # 列哪些
 *
 * **所有库**（全局 + 每个项目 / 分集），和节点里「资产库」弹窗一样。以前按画布所在项目只列那一级：
 * 自由画布只看得到全局那 3 个，切到一个自己没建资产的项目就是「这个库还是空的」——
 * 而节点里的弹窗明明列得出 49 个（用户 2026-09-25 报的）。
 * 当前项目的排最前，别的库的在名字下面标出处。
 */
export interface CanvasAssetLibraryProps {
  /** 画布当前所在项目。它的资产排最前；`null` = 自由画布，全局的排最前。 */
  projectId: string | null;
  onError?: (message: string) => void;
  /**
   * 放节点用的函数，由画布给 —— **按当前视口中心摆**。
   * 以前这里直接 `addNode(…, undefined)`，节点落在整张画布的默认位置，
   * 画布拖远之后点资产根本看不见新节点（添加菜单早修过同样的问题，这里漏了）。
   */
  onAddNode?: (type: "imageGen", data: Record<string, unknown>) => void;
}

const KINDS: AssetKind[] = ["character", "scene", "prop"];

/** 这条资产属不属于画布当前所在的库（项目 / 它下面的分集；自由画布 = 全局） */
export function inCurrentScope(a: AssetView, projectId: string | null): boolean {
  if (!projectId) return a.scope.kind === "global";
  return a.scope.kind !== "global" && a.scope.project_id === projectId;
}

/** 当前库的排前面，其余保持原顺序（后端已按更新时间倒序） */
export function orderAssets(list: AssetView[], projectId: string | null): AssetView[] {
  const mine = list.filter((a) => inCurrentScope(a, projectId));
  return [...mine, ...list.filter((a) => !inCurrentScope(a, projectId))];
}

export function CanvasAssetLibrary({ projectId, onError, onAddNode }: CanvasAssetLibraryProps) {
  const [assets, setAssets] = useState<AssetView[]>([]);
  const [query, setQuery] = useState("");
  // 默认收起（用户 2026-09-25：「画布资产库默认关闭」）—— 左上角留一个图标按钮，点开再看
  const [open, setOpen] = useState(false);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let alive = true;
    setLoading(true);
    assetsList()
      .then((list) => { if (alive) { setAssets(orderAssets(list, projectId)); setLoading(false); } })
      .catch((e) => { if (alive) { setLoading(false); onError?.(String(e)); } });
    return () => { alive = false; };
    // onError 每次渲染都是新函数，放进依赖会无限拉取
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [projectId]);

  const groups = useMemo(() => {
    const q = query.trim();
    return KINDS.map((kind) => ({
      kind,
      items: assets.filter(
        (a) => a.kind === kind && (!q || a.name.includes(q) || (a.description ?? "").includes(q)),
      ),
    }));
  }, [assets, query]);

  /**
   * 拖进画布 = 建一个绑定到这个资产的参考节点。
   *
   * **绑定而不是拷贝** —— 库里那份换了图，画布上跟着换。
   */
  const drop = (asset: AssetView) => {
    const url = asset.image_urls?.[0];
    if (!url) {
      onError?.(`「${asset.name}」还没有图，先去资产库里生成一张`);
      return;
    }
    const data = {
      referenceOnly: true,
      assetId: asset.id,
      label: asset.name,
      references: [{ path: url, name: asset.name, assetId: asset.id }],
      outputImageUrl: url,
      prompt: asset.description ?? "",
    };
    if (onAddNode) onAddNode("imageGen", data);
    else useCanvasStore.getState().addNode("imageGen", undefined, data);
  };

  if (!open) {
    return (
      <button className="canvas-asset-library-collapsed" onClick={() => setOpen(true)} title="资产库">
        <Layers width={16} height={16} />
      </button>
    );
  }

  return (
    <aside className="canvas-asset-library nodrag nopan" onPointerDown={(e) => e.stopPropagation()}>
      <header>
        <button onClick={() => setOpen(false)} title="收起">
          <ChevronDown />
          <Layers width={14} height={14} />
          <span>资产库</span>
        </button>
        <small>{projectId ? "全部 · 本项目在前" : "全部"}</small>
      </header>
      <input
        aria-label="搜索资产"
        placeholder="搜索角色 / 场景 / 道具"
        value={query}
        onChange={(e) => setQuery(e.target.value)}
      />
      <div className="canvas-asset-library-body">
        {loading && <p>读取中…</p>}
        {!loading && !assets.length && <p>这个库还是空的</p>}
        {groups.map((g) =>
          g.items.length ? (
            <section key={g.kind}>
              <h4>
                {KIND_LABEL[g.kind]}
                <small>{g.items.length}</small>
              </h4>
              {g.items.map((a) => (
                <button
                  key={`${a.scope_label}/${a.id}`}
                  className="canvas-asset-row"
                  title={a.description || a.name}
                  onClick={() => drop(a)}
                >
                  {a.image_urls?.[0] ? (
                    <img src={a.image_urls[0]} alt="" loading="lazy" />
                  ) : (
                    <span className="canvas-asset-noimg">无图</span>
                  )}
                  <span className="canvas-asset-name">
                    {a.name}
                    {!inCurrentScope(a, projectId) && <small className="canvas-asset-scope">{a.scope_label}</small>}
                  </span>
                  <ChevronRight />
                </button>
              ))}
            </section>
          ) : null,
        )}
      </div>
    </aside>
  );
}
