import { useEffect, useMemo, useState } from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";
import { BaseNodeWrapper } from "./BaseNodeWrapper";
import { useCanvasStore } from "../store/useCanvasStore";
import { NodeHeader } from "../ui/NodeHeader";
import { Orbit } from "../ui/icons";
import { assetsList, type AssetView, type Scope } from "../../api/assets";
import { ComposingInput } from "../../components/ComposingInput";
import type { SceneContract } from "../../api/assets";
import type { ShotContextNode } from "../types";

/**
 * 一个视角的文字地图。**和 Rust 侧 `SceneView::map_line` 是同一套话术。**
 *
 * 两边分叉的表现是「画布上看到的和实际发出去的不一样」，而那不报错。
 * 这里是显示 + 缓存用；真正进成片链路的那份仍然由 Rust 侧算。
 */
export function viewMapLine(c: SceneContract | null, viewName: string): string {
  const v = c?.views.find((x) => x.name === viewName);
  if (!v) return "";
  const bits: string[] = [];
  if (v.from && v.toward) bits.push(`从${v.from}朝${v.toward}看`);
  else if (v.from) bits.push(`从${v.from}看`);
  else if (v.toward) bits.push(`朝${v.toward}看`);
  if (v.far) bits.push(`视线尽头是${v.far}`);
  if (v.visible.length) bits.push(`同时可见：${v.visible.join("、")}`);
  if (v.light) bits.push(v.light);
  return bits.join("，");
}

/**
 * 地理锁。**和 Rust 侧 `SceneContract::lock_line` 是同一套话术**，
 * 同样只取最强的三个锚点 —— 锁面要进每一条提示词，长了会把画面挤掉。
 */
export function lockLine(c: SceneContract | null): string {
  if (!c) return "";
  const weight = { wall_dominant: 3, large: 2, medium: 1, small: 0 } as const;
  const parts = [...c.anchors]
    .filter((a) => a.name.trim())
    .sort((a, b) => weight[b.size] - weight[a.size])
    .slice(0, 3)
    .map((a) => {
      let one = a.name.trim();
      if (a.zone.trim()) one += `在${a.zone.trim()}`;
      if (a.size === "wall_dominant" || a.size === "large") one += "（大件，不要缩小）";
      const nots = a.must_not_become.filter((x) => x.trim());
      if (nots.length) one += `，不要画成${nots.join("或")}`;
      return one;
    });
  const doors = c.entrances.map((e) => e.name.trim()).filter(Boolean);
  if (doors.length) {
    parts.push(`入口只有 ${doors.length} 处（${doors.join("、")}），不要再加门或通道`);
  }
  return parts.length ? `地理锁：${parts.join("；")}。` : "";
}

/**
 * 镜头上下文节点。
 *
 * # 它替掉了一个假按钮
 *
 * 添加菜单里本来就有「镜头上下文」，但它建出来的是个普通文本框 ——
 * 菜单承诺了一个不存在的能力。这是「导演契约不好用」最小的那个样本：
 * 界面看着有，点下去什么都没有，而且不报错。
 *
 * # 挑场景就把契约取下来，存进节点
 *
 * 下游拼提示词是同步的（`aggregateNodeInputs` 是纯函数，有测试钉着），
 * 所以契约不能在拼的时候现查。挑的那一刻取下来缓存住 ——
 * 代价是场景契约改了之后要重挑一次，节点上写明了这件事。
 */
export function ShotContextNodeComponent({ id, data, selected }: NodeProps<ShotContextNode>) {
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
        // 读不到库不该让节点变成一片空白 —— 手填的那几栏照样能用
        if (alive) setScenes([]);
      });
    return () => {
      alive = false;
    };
  }, [scope]);

  const picked = scenes.find((s) => s.id === data.sceneAssetId);
  const contract = picked?.scene ?? null;

  const pickScene = (assetId: string) => {
    const a = scenes.find((s) => s.id === assetId);
    const c = a?.scene ?? null;
    const firstView = c?.views[0]?.name ?? "";
    updateNodeData(id, {
      sceneAssetId: assetId || undefined,
      sceneName: a?.name ?? "",
      viewName: firstView,
      lock: c ? lockLine(c) : "",
      viewMap: firstView ? viewMapLine(c, firstView) : "",
    });
  };

  const field = (label: string, key: "blocking" | "state", placeholder: string) => (
    <label className="flex flex-col gap-1">
      <span className="text-[11px] text-white/40">{label}</span>
      {/* **站位和时段几乎必然是中文**，普通受控 input 会把拼音串原样
          打进去（每敲一个字母回灌一次，冲掉输入法的组合缓冲）——
          见 `ComposingInput` 的注释。 */}
      <ComposingInput
        value={(data[key] as string) || ""}
        onCommit={(v) => updateNodeData(id, { [key]: v })}
        onKeyDown={(e) => e.stopPropagation()}
        placeholder={placeholder}
        className="nodrag nopan w-full rounded-md bg-white/5 px-2 py-1.5 text-[12px] text-white/85 placeholder:text-white/25 focus:outline-none focus:ring-1 focus:ring-white/20"
      />
    </label>
  );

  return (
    <div className="flex flex-col gap-2 select-none" style={{ width: 440 }}>
      <NodeHeader
        icon={<Orbit className="h-3.5 w-3.5 text-white/60" />}
        title="镜头上下文"
        onDelete={() => removeNode(id)}
      />
      <BaseNodeWrapper id={id} selected={selected} sourceHandleId="context-out">
        <Handle
          type="target"
          position={Position.Left}
          id="context-in"
          className="!w-2.5 !h-5 !rounded-full !bg-zinc-300 hover:!bg-blue-400 hover:!border-white/50 !border !border-zinc-800 shadow-md transition-transform hover:scale-125 cursor-crosshair z-10"
        />
        <div className="flex w-full flex-col gap-2.5 p-4">
          <label className="flex flex-col gap-1">
            <span className="text-[11px] text-white/40">场景</span>
            <select
              value={data.sceneAssetId || ""}
              onChange={(e) => pickScene(e.target.value)}
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

          {contract && contract.views.length > 0 && (
            <label className="flex flex-col gap-1">
              <span className="text-[11px] text-white/40">看哪个方向</span>
              <select
                value={data.viewName || ""}
                onChange={(e) =>
                  updateNodeData(id, {
                    viewName: e.target.value,
                    viewMap: viewMapLine(contract, e.target.value),
                  })
                }
                className="nodrag nopan w-full rounded-md bg-white/5 px-2 py-1.5 text-[12px] text-white/85 focus:outline-none"
              >
                {contract.views.map((v) => (
                  <option key={v.name} value={v.name}>
                    {v.name}
                  </option>
                ))}
              </select>
            </label>
          )}

          {data.sceneAssetId && !contract && (
            // **说清楚是什么没有，以及去哪补。**
            // 只显示空白的话，用户以为节点坏了。
            <p className="rounded-md bg-amber-400/10 px-2 py-1.5 text-[11px] leading-relaxed text-amber-200/80">
              这个场景还没有空间契约，所以这个节点只能发下面手填的两栏。
              去资产库里给它生成一份，跨镜的地理才锁得住。
            </p>
          )}

          {data.viewMap && (
            <p className="rounded-md bg-white/5 px-2 py-1.5 text-[11px] leading-relaxed text-white/55">
              {data.viewMap}
            </p>
          )}
          {data.lock && (
            <p className="rounded-md bg-emerald-400/10 px-2 py-1.5 text-[11px] leading-relaxed text-emerald-200/75">
              {data.lock}
            </p>
          )}

          {field("站位（画面左 / 右，不是东西南北）", "blocking", "沈星晴在画面左侧，背对门")}
          {field("时段 / 天气（会变的那一半）", "state", "傍晚，雨停了，柜台的暖灯亮着")}
        </div>
      </BaseNodeWrapper>
    </div>
  );
}
