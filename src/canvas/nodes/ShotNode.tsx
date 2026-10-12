import { useEffect, useState } from "react";
import { Handle, Position, type NodeProps } from "@xyflow/react";
import { BaseNodeWrapper } from "./BaseNodeWrapper";
import { useCanvasStore } from "../store/useCanvasStore";
import { NodeHeader } from "../ui/NodeHeader";
import { Film } from "../ui/icons";
import {
  cinemaScripts,
  cinemaGetProjectDetail,
  cinemaShotPromptPreview,
  type EpisodeData,
  type ScriptSummary,
} from "../../api/cinema";
import type { ShotNode as ShotNodeType } from "../types";

/** 下拉里一镜怎么写。带上画面头一句 —— 光有编号挑不出要哪一镜。 */
export function shotOptionLabel(num: number, prompt: string): string {
  const head = prompt.replace(/\s+/g, "").slice(0, 18);
  return head ? `第 ${num} 镜 · ${head}` : `第 ${num} 镜`;
}

/**
 * 分镜节点。挑「哪个项目 / 哪一集 / 哪一镜」，把那一镜发给下游。
 *
 * 提示词**不在这儿拼** —— 向后端要，拿到的和出片那一刻发出去的是同一段。
 * 见 `types.ts` 里 `ShotNodeData` 的注释。
 */
export function ShotNodeComponent({ id, data, selected }: NodeProps<ShotNodeType>) {
  const updateNodeData = useCanvasStore((s) => s.updateNodeData);
  const removeNode = useCanvasStore((s) => s.removeNode);
  const [all, setAll] = useState<ScriptSummary[]>([]);
  const activeProjectId = useCanvasStore((s) => s.activeProjectId);
  const [episode, setEpisode] = useState<EpisodeData | null>(null);

  /**
   * 下拉里列哪些分集。
   *
   * **默认只列当前工程的**（用户 2026-09-21：「不同工程下会混入其他工程的
   * 任务」）。但已经挂在这个节点上的那一集**永远留着** —— 它可能来自
   * 别的工程，筛掉的话下拉会显示成空，看起来像分镜丢了，而数据还在。
   */
  const episodes = all.filter(
    (e) => !activeProjectId || e.project_id === activeProjectId || e.project_id === data.projectId,
  );
  const hiddenCount = all.length - episodes.length;


  useEffect(() => {
    void cinemaScripts()
      .then(setAll)
      // 读不到列表时留空，节点上会显示「还没有分集」—— 比一片空白强
      .catch(() => setAll([]));
  }, []);

  // 画布是存下来的，重新打开时 data 里有 id 但 `episode` 是空的 ——
  // 不重新拉一次的话镜头下拉是空的，看起来像分镜丢了。
  useEffect(() => {
    if (!data.projectId || !data.episodeId) return;
    let alive = true;
    void cinemaGetProjectDetail(data.projectId)
      .then((d) => {
        if (alive) setEpisode(d.episodes.find((e) => e.id === data.episodeId) ?? null);
      })
      .catch(() => alive && setEpisode(null));
    return () => {
      alive = false;
    };
  }, [data.projectId, data.episodeId]);

  const projectTitle =
    episodes.find((e) => e.project_id === data.projectId && e.episode_id === data.episodeId);

  /** 取这一镜的提示词。**取不到要说出来** —— 静默留空的话下游会出一张无关的图。 */
  const loadPrompt = async (projectId: string, episodeId: string, shotId: string, label: string) => {
    updateNodeData(id, { loading: true, error: undefined });
    try {
      const p = await cinemaShotPromptPreview(projectId, episodeId, shotId);
      updateNodeData(id, {
        projectId,
        episodeId,
        shotId,
        label,
        frame: p.frame,
        unresolved: [],
        dialogueWarning: undefined,
        loading: false,
      });
    } catch (e) {
      // **出错时把旧提示词清掉。** 留着的话下游会拿一段属于上一镜的提示词
      // 继续出图，而节点上写着新镜号 —— 那种不一致查起来最费劲。
      updateNodeData(id, { loading: false, error: String(e), frame: "", unresolved: [] });
    }
  };

  const pickEpisode = async (value: string) => {
    if (!value) {
      setEpisode(null);
      updateNodeData(id, {
        projectId: undefined, episodeId: undefined, shotId: undefined,
        label: "", frame: "", unresolved: [], dialogueWarning: undefined, error: undefined,
      });
      return;
    }
    const [pid, eid] = value.split("/");
    try {
      const detail = await cinemaGetProjectDetail(pid);
      const ep = detail.episodes.find((e) => e.id === eid) ?? null;
      setEpisode(ep);
      updateNodeData(id, {
        projectId: pid, episodeId: eid,
        // 换集就把镜头清掉 —— 留着上一集的镜头 id 会取到「找不到分镜」，
        // 或者更糟：两集的 id 撞上，取到另一集的画面。
        shotId: undefined, label: "", frame: "", unresolved: [],
        dialogueWarning: undefined, error: undefined,
      });
    } catch (e) {
      updateNodeData(id, { error: String(e) });
    }
  };

  const pickShot = (shotId: string) => {
    if (!shotId || !data.projectId || !data.episodeId) return;
    const s = episode?.storyboard?.find((x) => x.id === shotId);
    const title = projectTitle
      ? `${projectTitle.project_title} · ${projectTitle.episode_title}`
      : "";
    void loadPrompt(
      data.projectId, data.episodeId, shotId,
      [title, `第 ${s?.shot_num ?? "?"} 镜`].filter(Boolean).join(" · "),
    );
  };

  const shots = episode?.storyboard ?? [];

  return (
    <div className="flex flex-col gap-2 select-none" style={{ width: 440 }}>
      <NodeHeader
        icon={<Film className="h-3.5 w-3.5 text-white/60" />}
        title="分镜"
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
            <span className="text-[11px] text-white/40">分集</span>
            <select
              value={data.projectId && data.episodeId ? `${data.projectId}/${data.episodeId}` : ""}
              onChange={(e) => void pickEpisode(e.target.value)}
              className="nodrag nopan w-full rounded-md bg-white/5 px-2 py-1.5 text-[12px] text-white/85 focus:outline-none"
            >
              <option value="">{episodes.length ? "（挑一集）" : "（这个工程还没有分集）"}</option>
              {episodes.map((e) => (
                <option key={`${e.project_id}/${e.episode_id}`} value={`${e.project_id}/${e.episode_id}`}>
                  {e.project_title} · {e.episode_title}（{e.shot_count} 镜）
                </option>
              ))}
            </select>
            {hiddenCount > 0 && (
              <span className="text-[10.5px] text-white/30">
                只列当前工程的；别的工程还有 {hiddenCount} 集，换工程后可见
              </span>
            )}
          </label>

          <label className="flex flex-col gap-1">
            <span className="text-[11px] text-white/40">镜头</span>
            <select
              value={data.shotId || ""}
              onChange={(e) => pickShot(e.target.value)}
              disabled={!shots.length}
              className="nodrag nopan w-full rounded-md bg-white/5 px-2 py-1.5 text-[12px] text-white/85 focus:outline-none disabled:opacity-40"
            >
              <option value="">{shots.length ? "（挑一镜）" : "（这一集还没有分镜）"}</option>
              {shots.map((s) => (
                <option key={s.id} value={s.id}>
                  {shotOptionLabel(s.shot_num, s.prompt || "")}
                </option>
              ))}
            </select>
          </label>

          {data.loading && <p className="text-[11px] text-white/40">正在取这一镜的提示词…</p>}
          {data.error && <p className="text-[11px] text-rose-300/90">{data.error}</p>}

          {data.frame && (
            <div className="rounded-md bg-white/5 p-2.5">
              <p className="mb-1 text-[10.5px] text-white/35">
                下游收到的就是这一段 —— 和出片那一刻发出去的是同一份
              </p>
              <p className="max-h-28 overflow-y-auto whitespace-pre-wrap text-[11.5px] leading-relaxed text-white/75">
                {data.frame}
              </p>
            </div>
          )}

          {/* **对不上的引用必须说。** 对不上时提示词里只剩一个名字，
              角色的长相描述根本没进去 —— 出来是个陌生人，而不报错。 */}
          {data.unresolved?.length > 0 && (
            <p className="text-[11px] text-amber-300/85">
              资产库里没有：{data.unresolved.join("、")} —— 这几个只会以名字进提示词，
              长相和设定不会带上。
            </p>
          )}
          {data.dialogueWarning && (
            <p className="text-[11px] text-amber-300/85">{data.dialogueWarning}</p>
          )}
        </div>
      </BaseNodeWrapper>
    </div>
  );
}
