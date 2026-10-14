import { useCallback, useEffect, useState } from "react";
import type { CanvasNode } from "../types";
import { useCanvasStore } from "../store/useCanvasStore";
import {
  canvasWorkflowDelete, canvasWorkflowList, canvasWorkflowLoad, canvasWorkflowSave,
  type WorkflowEntry,
} from "../../api/canvasWorkflows";
import { listLoras } from "../../api";
import { studioCapabilities } from "../../api/studio";
import "./CanvasImageAssetToolbar.css";
import { PRESETS } from "../utils/workflowPresets";
import { slugOf, templateNeeds, toTemplate, type WorkflowTemplate } from "../utils/workflowTemplate";
import { appendGraph } from "../utils/appendGraph";

/**
 * 工作流目录。
 *
 * # 这个面板存在的理由
 *
 * 画布本来是「一张布，上面画啥都行」—— 一套接法调好了也**存不下来**，
 * 换一集只能从头连一遍。ComfyUI 能把当前图存成独立工作流，我们不能。
 *
 * # 按工程分开
 *
 * 用户 2026-09-21：「不同工程下会混入其他工程的任务」。
 *
 * 默认只列当前工程的。但**别的工程那一栏留着** —— 模板存在的全部理由
 * 就是「换个项目、换套素材还能跑」（模板里本来就不带集/镜绑定和参考图路径）。
 * 硬过滤等于把这个功能砍掉一半。
 *
 * # 载入那一刻要列缺口，但缺口写在**节点上**
 *
 * 模板里没有集/镜绑定、没有参考图路径（见 `workflowTemplate`），
 * 所以载进来之后**一定有东西要填**。不列出来的话，失败的样子是
 * 「载入显示成功、一跑全是默认值」—— 这个项目的主病。
 *
 * 第一版把缺口列在这个面板里，于是面板不能关（用户 2026-09-21：
 * 「点击模板或者载入后，要自动关闭这个窗口」）。但直接关掉又会把清单扔了。
 *
 * 解法是把缺口**写进对应节点的 `notes`** —— 那本来就是它该在的地方：
 * 「还要填 2 张参考图」应该长在那个图片节点上，而不是一张关掉就没的清单里。
 * 然后面板就能像添加节点面板那样点完即关。
 */
export function CanvasWorkflowPanel({ open, onClose, projects = [], onPlaced }: {
  open: boolean;
  onClose: () => void;
  /** 载入 / 摆上之后，新加的那几个节点 id（画布把视角移过去 —— 追加在右边，不移过去用户会以为没载进来） */
  onPlaced?: (ids: string[]) => void;
  /** 工作区里的工程，用来把「别的工程」那一栏显示成名字而不是 id */
  projects?: { id: string; title: string }[];
}) {
  const projectId = useCanvasStore((s) => s.activeProjectId) ?? "";
  const [list, setList] = useState<WorkflowEntry[]>([]);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const [msg, setMsg] = useState("");
  /** 展开「别的工程」那一栏。**默认收着** —— 默认不混是用户要的 */
  const [showOthers, setShowOthers] = useState(false);

  const refresh = useCallback(() => {
    canvasWorkflowList().then(setList).catch((e) => setErr(String(e)));
  }, []);
  useEffect(() => { if (open) refresh(); }, [open, refresh]);

  if (!open) return null;

  const save = async () => {
    const { nodes, edges } = useCanvasStore.getState();
    if (!nodes.length) { setErr("画布上什么都没有"); return; }
    setBusy(true); setErr(""); setMsg("");
    try {
      // **存的是模板，不是整张画布。** 产物、集/镜绑定、参考图的绝对路径
      // 一律剥掉 —— 带过去每一样都不报错，但都是错的。
      const t = toTemplate(slugOf(name), nodes, edges);
      await canvasWorkflowSave(projectId, t.name, JSON.stringify(t));
      setMsg(`已存成「${t.name}」`); setName(""); refresh();
    } catch (e) { setErr(String(e)); } finally { setBusy(false); }
  };

  const load = async (owner: string, wf: string) => {
    setBusy(true); setErr(""); setMsg("");
    try {
      const t: WorkflowTemplate = JSON.parse(await canvasWorkflowLoad(owner, wf));
      const [loras, caps] = await Promise.all([
        listLoras().catch(() => [] as string[]),
        studioCapabilities().catch(() => null),
      ]);
      const items = templateNeeds(t, {
        loras,
        engines: caps?.engines.map((e) => e.id) ?? [],
      });
      // **缺口写到节点上，不是列在面板里。** 面板点完就关（见文件头），
      // 而「还要填 2 张参考图」应该长在那个图片节点上 —— 那才是要动手的地方。
      const missing = items.filter((i) => i.status === "missing");
      const nodes = t.nodes.map((n) => {
        const mine = missing.filter((i) => i.nodeId === n.id).map((i) => `载入后还要：${i.label}`);
        return (mine.length ? { ...n, data: { ...n.data, notes: mine } } : n) as CanvasNode;
      });
      // **追加，不清空**（用户 2026-10-13：「我们是无限画布，不是单工作流画布」）。
      // 以前这里整张替换 —— 画布上原来的东西一点「载入」就没了
      const store = useCanvasStore.getState();
      const g = appendGraph(store.nodes, store.edges, nodes, t.edges);
      store.setNodes(g.nodes);
      store.setEdges(g.edges);
      onPlaced?.(g.addedIds);
      setMsg(missing.length
        ? `已载入「${wf}」·${missing.length} 处要填，已经标在对应节点上`
        : `已载入「${wf}」·没有缺口，直接就能跑`);
      onClose();
    } catch (e) { setErr(String(e)); } finally { setBusy(false); }
  };

  const drop = async (owner: string, wf: string) => {
    setBusy(true); setErr("");
    try { await canvasWorkflowDelete(owner, wf); refresh(); }
    catch (e) { setErr(String(e)); } finally { setBusy(false); }
  };

  const mine = list.filter((w) => w.project_id === projectId);
  const others = list.filter((w) => w.project_id !== projectId);
  const ownerName = (id: string) =>
    id ? projects.find((p) => p.id === id)?.title ?? id : "没挂工程";

  const Row = ({ w, from }: { w: WorkflowEntry; from?: string }) => (
    <li key={`${w.project_id}/${w.name}`}>
      <span className="wf-name" title={new Date(w.saved_at).toLocaleString()}>
        {w.name}{from ? <span className="wf-from"> · {from}</span> : null}
      </span>
      <button type="button" disabled={busy} onClick={() => void load(w.project_id, w.name)}>载入</button>
      <button type="button" disabled={busy} onClick={() => void drop(w.project_id, w.name)} className="wf-drop">删除</button>
    </li>
  );

  return (
    <div className="canvas-workflow-panel nodrag nopan" role="dialog" aria-label="工作流目录"
      onPointerDown={(e) => e.stopPropagation()}>
      <header>
        <span>工作流</span>
        <button type="button" onClick={onClose} aria-label="关闭">×</button>
      </header>

      <div className="wf-save">
        <input value={name} onChange={(e) => setName(e.target.value)} placeholder="给这套接法起个名字"
          aria-label="工作流名字" onKeyDown={(e) => { if (e.key === "Enter") void save(); }} />
        <button type="button" disabled={busy} onClick={() => void save()}>存成工作流</button>
      </div>
      <p className="wf-hint">
        只存接法和参数。产物、挂的哪一镜、参考图都不进去 ——
        带过去的话画面里会出现上一集的人，而且不报错。
      </p>

      {/* **内置模板。** 工作流目录解决了「存得下来」，但新项目里它是空的 ——
          用户还是得先知道该怎么连。这几套是把验证过的路径固化下来。 */}
      <div className="wf-presets">
        <p className="wf-presets-title">从模板开始</p>
        {PRESETS.map((p) => (
          <button key={p.id} type="button" disabled={busy} title={p.hint}
            onClick={() => {
              const built = p.build();
              const store = useCanvasStore.getState();
              // **追加，不清空。** 画布上已经有的东西不该被一次点击抹掉；摆在现有内容右边，不叠在上面
              const g = appendGraph(store.nodes, store.edges, built.nodes, built.edges);
              store.setNodes(g.nodes);
              store.setEdges(g.edges);
              onPlaced?.(g.addedIds);
              setMsg(`已摆上「${p.name}」`);
              onClose();   // 和添加节点面板一样，点完就关
            }}>
            {p.name}
          </button>
        ))}
      </div>

      {err && <p className="wf-err" role="alert">{err}</p>}
      {msg && <p className="wf-ok" role="status">{msg}</p>}

      <ul className="wf-list">
        {mine.length === 0 && <li className="wf-empty">这个工程还没存过工作流</li>}
        {mine.map((w) => <Row key={`${w.project_id}/${w.name}`} w={w} />)}
      </ul>

      {/* **别的工程那一栏留着，但默认收起。** 模板本来就是拿来跨工程复用的，
          硬过滤等于把这个功能砍掉一半；默认收起来才是「不混」。 */}
      {others.length > 0 && (
        <>
          <button type="button" className="wf-others-toggle"
            onClick={() => setShowOthers((v) => !v)}>
            {showOthers ? "▾" : "▸"} 别的工程里还有 {others.length} 套（模板不带集/镜绑定，可以直接拿来用）
          </button>
          {showOthers && (
            <ul className="wf-list">
              {others.map((w) => <Row key={`${w.project_id}/${w.name}`} w={w} from={ownerName(w.project_id)} />)}
            </ul>
          )}
        </>
      )}

    </div>
  );
}
