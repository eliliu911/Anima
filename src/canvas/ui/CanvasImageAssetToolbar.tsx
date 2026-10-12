import { useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { NewAssetDialog } from "../../components/studio/library/NewAssetDialog";
import { pathFromMediaUrl } from "../../components/studio/MediaThumb";
import { useCanvasStore } from "../store/useCanvasStore";
import { Layers, Check } from "./icons";
import { assetsList, assetsAddVariant, KIND_LABEL, type AssetView, type Scope } from "../../api/assets";
import { inferCommitTarget, rolesFor, ROLE_LABEL, type CommitTarget } from "../utils/commitTarget";
import { upstreamShot, type ShotTarget } from "../utils/shotLink";
import { cinemaAdoptFrame } from "../../api/cinema";
import { IMAGE_TOOLS, spawnImageTool, spawnImageEdit } from "../utils/imageTools";
import "./CanvasImageAssetToolbar.css";

/**
 * 画布产物 → 资产库的提交。
 *
 * # 两件以前没有的事
 *
 * 1. **落点带类型。** 以前只能三选一（角色/场景/道具），主线拿到一张图
 *    只知道它是「场景」，不知道是**主镜**还是**反打** —— 而空间契约要的
 *    恰恰是这一对。「这张图是什么」只有提交的那一刻知道。
 * 2. **能追加到已有资产。** 以前只能新建，于是同一个角色的第二张图
 *    就变成了第二个角色。
 *
 * 落点由 `inferCommitTarget` **推断**，用户只需确认 —— 让人从六个选项里
 * 自己挑，等于把数据模型摊给他看。
 */
export function CanvasImageAssetToolbar({ selected, path, name, description, nodeId }: {
  selected?: boolean; path?: string; name?: string; description?: string; nodeId?: string;
}) {
  const [draft, setDraft] = useState<{path: string; name: string; description: string; scope: Scope} | null>(null);
  const [savedPath, setSavedPath] = useState<string | null>(null);
  const [panel, setPanel] = useState(false);
  const [assets, setAssets] = useState<AssetView[]>([]);
  const [target, setTarget] = useState<CommitTarget>({ kind: "scene", role: "reference" });
  const [attachTo, setAttachTo] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  /**
   * 这张图是给哪一镜出的。**上游没有分镜节点就不显示这个按钮** ——
   * 让人在没有落点的时候点一下再收到「不知道收到哪儿」，比不显示更糟。
   */
  const [shot, setShot] = useState<ShotTarget | null>(null);
  const [adopted, setAdopted] = useState<string | null>(null);

  const scope: Scope = useMemo(() => {
    const pid = useCanvasStore.getState().activeProjectId;
    return pid ? { kind: "project", project_id: pid } : { kind: "global" };
  }, [panel]);

  // 打开面板时才拉资产 —— 每个图片节点都预拉一遍是白花的
  useEffect(() => {
    if (!panel) return;
    const { nodes, edges } = useCanvasStore.getState();
    if (nodeId) setTarget(inferCommitTarget(nodeId, nodes, edges));
    assetsList(scope).then(setAssets).catch((e) => setError(String(e)));
  }, [panel, nodeId, scope]);

  // **选中时就要算**，不能等面板打开 —— 按钮本身就在工具条上。
  useEffect(() => {
    if (!selected || !nodeId) return;
    const { nodes, edges } = useCanvasStore.getState();
    setShot(upstreamShot(nodeId, nodes, edges));
  }, [selected, nodeId]);

  /**
   * 把这张图收成那一镜的首帧。
   *
   * 收完**不跳走也不关面板** —— 画布上这张图还在，人多半要接着调。
   */
  const adopt = async () => {
    if (!path || !shot || busy) return;
    setBusy(true); setError("");
    try {
      await cinemaAdoptFrame(shot.projectId, shot.episodeId, shot.shotId, pathFromMediaUrl(path));
      setAdopted(path);
    } catch (e) { setError(String(e)); } finally { setBusy(false); }
  };

  const candidates = assets.filter((a) => a.kind === target.kind);
  const allowed = rolesFor(target.kind);
  // 换了资产类型之后，旧角色可能已经不合法了 —— 回落到通用参考图
  const role = allowed.includes(target.role) ? target.role : "reference";

  const attach = async () => {
    if (!path || !attachTo || busy) return;
    setBusy(true); setError("");
    try {
      await assetsAddVariant({
        scope, id: attachTo,
        media: { kind: "path", path: pathFromMediaUrl(path) },
        prompt_used: description || null,
        role,
      });
      setSavedPath(path); setPanel(false); setAttachTo("");
    } catch (e) { setError(String(e)); } finally { setBusy(false); }
  };

  return <>
    {selected && path && <div className="canvas-image-asset-toolbar nodrag nopan" role="toolbar" aria-label="图片工具"
      onPointerDown={(e) => e.stopPropagation()} onClick={(e) => e.stopPropagation()}>
      <button type="button" title="存入资产库" onClick={() => { setPanel((v) => !v); setError(""); }}>
        <Layers width={14} height={14} />存入资产库
      </button>
      {/* **只有上游接了分镜才有这个按钮。** 有落点才给动作 ——
          否则点下去只能回一句「不知道收到哪儿」。 */}
      {shot && (
        <button type="button" disabled={busy}
          title={`收成「${shot.label}」的首帧，出片直接用这张`}
          onClick={() => void adopt()}>
          <Check width={13} height={13} />{busy ? "收图中…" : `收为${shot.label}的首帧`}
        </button>
      )}
      {/* **局部修改和上面那些工具不一样：它不立刻跑。**
          参数定死的工具点了就能跑；改什么得先说清楚。 */}
      {nodeId && (
        <button type="button"
          title="改图里的一小块，其余不动。在右边新建一个编辑节点，写清改什么再跑 · Qwen-Image-Edit"
          onClick={() => spawnImageEdit(nodeId)}>
          局部修改
        </button>
      )}
      {nodeId && IMAGE_TOOLS.map((t) => (
        <button key={t.id} type="button" title={t.hint} onClick={() => spawnImageTool(nodeId, t.id)}>
          {t.label}
        </button>
      ))}
      {savedPath === path && <span role="status"><Check width={13} height={13} />已保存</span>}
      {adopted === path && <span role="status"><Check width={13} height={13} />已收为首帧</span>}
      {error && !panel && <span role="alert">{error}</span>}
    </div>}

    {panel && selected && path && <div className="canvas-commit-panel nodrag nopan"
      onPointerDown={(e) => e.stopPropagation()} onClick={(e) => e.stopPropagation()}>
      <label>
        存成什么
        <select aria-label="资产类型" value={target.kind}
          onChange={(e) => setTarget({ kind: e.target.value as CommitTarget["kind"], role: "reference" })}>
          {(["character", "scene", "prop"] as const).map((k) => <option key={k} value={k}>{KIND_LABEL[k]}</option>)}
        </select>
        <select aria-label="用途" value={role}
          onChange={(e) => setTarget((t) => ({ ...t, role: e.target.value as CommitTarget["role"] }))}>
          {allowed.map((r) => <option key={r} value={r}>{ROLE_LABEL[r]}</option>)}
        </select>
      </label>

      <label>
        放到哪
        <select aria-label="目标资产" value={attachTo} onChange={(e) => setAttachTo(e.target.value)}>
          <option value="">新建资产…</option>
          {candidates.map((a) => <option key={a.id} value={a.id}>追加到「{a.name}」</option>)}
        </select>
      </label>

      {error && <p role="alert">{error}</p>}
      <div className="canvas-commit-actions">
        <button onClick={() => setPanel(false)}>取消</button>
        {attachTo
          ? <button disabled={busy} onClick={() => void attach()}>{busy ? "保存中…" : "追加"}</button>
          : <button onClick={() => {
              setDraft({ path: pathFromMediaUrl(path), name: name || "", description: description || "", scope });
              setPanel(false);
            }}>新建…</button>}
      </div>
    </div>}

    {draft && createPortal(<div className="anima-studio nodrag nopan" onPointerDown={(e) => e.stopPropagation()} onClick={(e) => e.stopPropagation()}>
      <NewAssetDialog initialMedia={draft.path} initialName={draft.name} initialDescription={draft.description}
        scope={draft.scope} onClose={() => setDraft(null)} onCreated={() => { setSavedPath(path || null); setDraft(null); }} />
    </div>, document.body)}
  </>;
}
