import { useEffect, useMemo, useState } from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";
import { BaseNodeWrapper } from "./BaseNodeWrapper";
import { useCanvasStore } from "../store/useCanvasStore";
import { selectContentNodes } from "../store/selectContentNodes";
import { NodeHeader } from "../ui/NodeHeader";
import { Layers } from "../ui/icons";
import { studioLoras, type LoraOption } from "../../api/studio";
import { isBakedIn, loraFits } from "../utils/collectLoras";
import type { LoraNode } from "../types";

/**
 * LoRA 节点：挂一个 LoRA，接到出图 / 出片节点上。
 *
 * 用户 2026-09-20：「画布上还无法使用特定的 LoRA 作为节点」。
 *
 * 生成节点工具条里那个弹层还在，**两者共用同一份数据**
 * （执行时由 `collectLoras` 合并）—— 不做两套。
 *
 * # 兼容性当场标出来，不等到点生成
 *
 * 原来挂错架构要到 `canvasLoraConfig` 抛错才知道，那时候人已经点了生成。
 * 这里按下游生成节点的引擎当场判：挂不上就标红，并说清它属于哪个架构。
 */
export function LoraNodeComponent({ id, data, selected }: NodeProps<LoraNode>) {
  const removeNode = useCanvasStore((s) => s.removeNode);
  const updateNodeData = useCanvasStore((s) => s.updateNodeData);
  const nodes = useCanvasStore(selectContentNodes);
  const edges = useCanvasStore((s) => s.edges);
  const [catalog, setCatalog] = useState<LoraOption[] | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    let live = true;
    studioLoras()
      .then((l) => live && setCatalog(l))
      .catch((e) => live && setError(String(e)));
    return () => {
      live = false;
    };
  }, []);

  /** 下游那些生成节点用的引擎 —— 兼容性按它们判。 */
  const targets = useMemo(() => {
    const ids = edges.filter((e) => e.source === id).map((e) => e.target);
    return [
      ...new Set(
        ids
          .map((t) => nodes.find((n) => n.id === t))
          .filter((n) => n && (n.type === "imageGen" || n.type === "videoGen"))
          .map((n) => String((n!.data as { engine?: string }).engine ?? "")),
      ),
    ].filter(Boolean);
  }, [id, nodes, edges]);

  const entry = catalog?.find((l) => l.name === data.name);
  const bad = targets.filter((t) => !loraFits(entry?.engines, t));
  const baked = data.name ? isBakedIn(data.name) : false;

  return (
    <div className="flex flex-col gap-2 select-none" style={{ width: 300 }}>
      <NodeHeader icon={<Layers className="h-3.5 w-3.5 text-white/60" />} title="LoRA" onDelete={() => removeNode(id)} />
      <BaseNodeWrapper id={id} selected={selected} notes={data.notes} sourceHandleId="lora-out">
        {/* **`nodrag nopan` 是必须的。** 不加的话点下拉框那一下会被 ReactFlow
            当成拖节点：菜单打不开，节点跟着鼠标跑（2026-09-20 用户在试音色
            节点上撞到，这个节点是同一天写的、同一个毛病）。 */}
        <div className="nodrag nopan flex w-full flex-col gap-2 p-3 text-[12px]">
          {error ? (
            <p className="text-red-300">{error}</p>
          ) : !catalog ? (
            <p className="text-white/50">读取 LoRA 列表中…</p>
          ) : (
            <>
              <select
                value={data.name ?? ""}
                onChange={(e) => updateNodeData(id, { name: e.target.value })}
                className="w-full rounded-md px-2 py-1 bg-white/5 text-white outline-none"
              >
                <option value="" className="bg-zinc-900">
                  选一个 LoRA…
                </option>
                {catalog.map((l) => (
                  <option key={l.name} value={l.name} className="bg-zinc-900">
                    {l.label}（{l.engines.join(" / ") || "没有能挂的引擎"}）
                  </option>
                ))}
              </select>

              <label className="flex items-center gap-2">
                <span className="text-white/60 shrink-0">权重</span>
                <input
                  type="range"
                  min={0}
                  max={1.5}
                  step={0.05}
                  value={data.strength}
                  onChange={(e) => updateNodeData(id, { strength: Number(e.target.value) })}
                  className="min-w-0 flex-1"
                />
                <span className="w-8 text-right tabular-nums text-white/70">{data.strength.toFixed(2)}</span>
              </label>

              {entry && entry.triggers.length > 0 && (
                <label className="flex items-center gap-1.5 text-white/60">
                  <input
                    type="checkbox"
                    checked={data.injectTriggers !== false}
                    onChange={(e) => updateNodeData(id, { injectTriggers: e.target.checked })}
                  />
                  把激活词并进提示词（{entry.triggers.join("、")}）
                </label>
              )}

              {/* **兼容性当场说，不等到点生成才抛。** */}
              {bad.length > 0 && (
                <p className="text-red-300">
                  挂不上下游的 {bad.join(" / ")} —— 这个 LoRA 是{" "}
                  {entry?.engines.length ? entry.engines.join(" / ") : "没有能挂的引擎"}的
                </p>
              )}
              {baked && (
                <p className="text-amber-300">
                  采样预设已经内置挂了它，这里再挂一次出片会被 preflight 拒 —— 不用选它
                </p>
              )}
              {data.strength === 0 && <p className="text-white/40">权重 0：挂上但不生效（留着做对照）</p>}
              {targets.length === 0 && <p className="text-white/40">右边接一个出图或出片节点才会生效</p>}
            </>
          )}
        </div>
        <Handle
          type="source"
          position={Position.Right}
          id="lora-out"
          className="!w-2.5 !h-5 !rounded-full !bg-zinc-300 hover:!bg-blue-400 hover:!border-white/50 !border !border-zinc-800 shadow-md transition-transform hover:scale-125 cursor-crosshair z-10"
        />
      </BaseNodeWrapper>
    </div>
  );
}
