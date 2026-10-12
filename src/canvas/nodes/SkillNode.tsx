import { Handle, Position, type NodeProps } from "@xyflow/react";
import { BaseNodeWrapper } from "./BaseNodeWrapper";
import { useCanvasStore } from "../store/useCanvasStore";
import { NodeHeader } from "../ui/NodeHeader";
import { Layers } from "../ui/icons";
import { shotLanguage } from "../../shared/shotLanguage";
import type { SkillNode } from "../types";

/**
 * 技能节点 —— 把镜头语言词表接到画布上。
 *
 * # 为什么不是一个自由文本框
 *
 * 自由文本框已经有了（就是「文本」节点）。这个节点的价值在于
 * **它发出去的不是用户打的字，是词表里那条经过实测的提示词**：
 * 「推镜头」四个字发给模型基本不起作用，`slow dolly in, camera pushes
 * toward the subject` 才起作用。同一件事在分镜卡片上已经踩过一次
 * （显示串同时当 id 和提示词用）。
 *
 * 词表是 `shared/shot-language.json`，Rust / 前端 / 生成 skills markdown
 * 的脚本读的是同一份字节。这里不另建一套。
 */
export function SkillNodeComponent({ id, data, selected }: NodeProps<SkillNode>) {
  const updateNodeData = useCanvasStore((s) => s.updateNodeData);
  const removeNode = useCanvasStore((s) => s.removeNode);
  const groups = shotLanguage().groups;
  const picks = data.picks ?? [];

  /** 一组里挑中的那条（空 = 不用这一组）。**一组只能挑一条** —— 同时发
   * 「大远景」和「特写」是互相矛盾的，模型会随机听一个。 */
  const pickOf = (groupId: string) =>
    picks.find((p) => p.startsWith(`${groupId}:`))?.slice(groupId.length + 1) ?? "";

  const setPick = (groupId: string, termId: string) => {
    const rest = picks.filter((p) => !p.startsWith(`${groupId}:`));
    updateNodeData(id, {
      picks: termId ? [...rest, `${groupId}:${termId}`] : rest,
    });
  };

  return (
    <div className="flex flex-col gap-2 select-none" style={{ width: 440 }}>
      <NodeHeader
        icon={<Layers className="h-3.5 w-3.5 text-white/60" />}
        title="技能"
        onDelete={() => removeNode(id)}
      />
      <BaseNodeWrapper id={id} selected={selected} sourceHandleId="skill-out">
        <Handle
          type="target"
          position={Position.Left}
          id="skill-in"
          className="!w-2.5 !h-5 !rounded-full !bg-zinc-300 hover:!bg-blue-400 hover:!border-white/50 !border !border-zinc-800 shadow-md transition-transform hover:scale-125 cursor-crosshair z-10"
        />
        <div className="flex w-full flex-col gap-2 p-4">
          <p className="text-[11px] leading-relaxed text-white/40">
            挑中的会按下游引擎的语言发出去（中文引擎发中文）。留空的组不发。
          </p>
          <div className="grid grid-cols-2 gap-2">
            {groups.map((g) => (
              <label key={g.id} className="flex flex-col gap-1">
                <span className="text-[11px] text-white/40">{g.name}</span>
                <select
                  value={pickOf(g.id)}
                  onChange={(e) => setPick(g.id, e.target.value)}
                  className="nodrag nopan w-full rounded-md bg-white/5 px-2 py-1.5 text-[12px] text-white/85 focus:outline-none"
                >
                  <option value="">—</option>
                  {g.items.map((t) => (
                    <option key={t.id} value={t.id}>
                      {t.name}
                    </option>
                  ))}
                </select>
              </label>
            ))}
          </div>
          {/* **实测注记要显示出来。** 「大远景里脸的像素预算约等于零」
              这种话不写在眼前，用户就会在那一档指望长相。 */}
          {picks
            .map((p) => {
              const i = p.indexOf(":");
              const g = groups.find((x) => x.id === p.slice(0, i));
              return g?.items.find((t) => t.id === p.slice(i + 1));
            })
            .filter((t) => t?.note)
            .map((t) => (
              <p
                key={t!.id}
                className="rounded-md bg-amber-400/10 px-2 py-1.5 text-[11px] leading-relaxed text-amber-200/80"
              >
                {t!.name}：{t!.note}
              </p>
            ))}
        </div>
      </BaseNodeWrapper>
    </div>
  );
}
