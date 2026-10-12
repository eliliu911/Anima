import { useEffect, useMemo, useState } from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";
import { BaseNodeWrapper } from "./BaseNodeWrapper";
import { useCanvasStore } from "../store/useCanvasStore";
import { NodeHeader } from "../ui/NodeHeader";
import { Orbit } from "../ui/icons";
import { assetsList, type AssetView, type Scope } from "../../api/assets";
import { panoramaPlan, PANO_LONG_SIDE } from "../utils/panoramaPrompt";
import type { PanoramaNode } from "../types";

/**
 * 360° 全景节点。**接到图片节点上才出图** —— 它自己只编译提示词和画幅。
 *
 * 这样做而不是自带一套生成，是因为图片节点那条路（引擎选择、回落、
 * LoRA、进度、错误措辞）全都有测试盯着，再写一份必然分叉。
 */
export function PanoramaNodeComponent({ id, data, selected }: NodeProps<PanoramaNode>) {
  const updateNodeData = useCanvasStore((s) => s.updateNodeData);
  const removeNode = useCanvasStore((s) => s.removeNode);
  const projectId = useCanvasStore((s) => s.activeProjectId);
  const [scenes, setScenes] = useState<AssetView[]>([]);

  const scope: Scope = useMemo(
    () => (projectId ? { kind: "project", project_id: projectId } : { kind: "global" }),
    [projectId],
  );

  useEffect(() => {
    let alive = true;
    assetsList(scope)
      .then((list) => {
        if (alive) setScenes(list.filter((a) => a.kind === "scene"));
      })
      .catch(() => {
        if (alive) setScenes([]);
      });
    return () => {
      alive = false;
    };
  }, [scope]);

  const longSide = data.longSide || PANO_LONG_SIDE;

  const recompile = (assetId: string, side: number) => {
    const a = scenes.find((s) => s.id === assetId);
    const plan = panoramaPlan(a?.scene ?? null, a?.name ?? "", side);
    updateNodeData(id, {
      sceneAssetId: assetId || undefined,
      sceneName: a?.name ?? "",
      longSide: side,
      prompt: plan.prompt,
      warnings: plan.warnings,
    });
  };

  return (
    <div className="flex flex-col gap-2 select-none" style={{ width: 440 }}>
      <NodeHeader
        icon={<Orbit className="h-3.5 w-3.5 text-white/60" />}
        title="360° 全景"
        onDelete={() => removeNode(id)}
      />
      <BaseNodeWrapper id={id} selected={selected} sourceHandleId="pano-out">
        <Handle
          type="target"
          position={Position.Left}
          id="pano-in"
          className="!w-2.5 !h-5 !rounded-full !bg-zinc-300 hover:!bg-blue-400 hover:!border-white/50 !border !border-zinc-800 shadow-md transition-transform hover:scale-125 cursor-crosshair z-10"
        />
        <div className="flex w-full flex-col gap-2.5 p-4">
          <label className="flex flex-col gap-1">
            <span className="text-[11px] text-white/40">场景</span>
            <select
              value={data.sceneAssetId || ""}
              onChange={(e) => recompile(e.target.value, longSide)}
              className="nodrag nopan w-full rounded-md bg-white/5 px-2 py-1.5 text-[12px] text-white/85 focus:outline-none"
            >
              <option value="">（先挑一个场景）</option>
              {scenes.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.name}
                  {s.scene ? "" : "（没有空间契约）"}
                </option>
              ))}
            </select>
          </label>

          <label className="flex flex-col gap-1">
            <span className="text-[11px] text-white/40">
              长边 {longSide} px · 出图 {longSide}×{Math.round(longSide / 2)}
            </span>
            <input
              type="range"
              min={1024}
              max={4096}
              step={256}
              value={longSide}
              onChange={(e) => recompile(data.sceneAssetId || "", Number(e.target.value))}
              className="nodrag nopan w-full accent-white/60"
            />
          </label>

          {/* **画幅不让选，这是故意的。** 等距圆柱投影就是 2:1，
              别的比例当全景用会在上下两极撕开。 */}
          <p className="text-[11px] leading-relaxed text-white/40">
            接到图片节点上出图。画幅固定 2:1，下游节点自己设的会被顶掉 ——
            等距圆柱投影只有这一个比例。
          </p>

          {(data.warnings ?? []).map((w) => (
            <p
              key={w}
              className="rounded-md bg-amber-400/10 px-2 py-1.5 text-[11px] leading-relaxed text-amber-200/80"
            >
              {w}
            </p>
          ))}

          {data.prompt && (
            <pre className="nowheel max-h-[160px] overflow-auto whitespace-pre-wrap rounded-md bg-white/5 px-2 py-1.5 text-[11px] leading-relaxed text-white/55">
              {data.prompt}
            </pre>
          )}
        </div>
      </BaseNodeWrapper>
    </div>
  );
}
