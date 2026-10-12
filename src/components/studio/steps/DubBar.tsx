/**
 * 组装页上的「配音」一块。
 *
 * **不做成操作条上的勾选框。** 配音是有代价的动作：一旦生成，拼接就用它
 * 替代 H3 的原声，而 H3 的口型是跟着它自己那段语音做的 —— 换了就对不上。
 * 这种事必须让人看见代价再按，不能混在一排开关里顺手勾上。
 *
 * （同一个教训 2026-09-18 在「合成时加片头封面」上刚踩过一次：
 * 勾选即执行，用户反馈「很 confuse，业务流程不对」。）
 */
import { useCallback, useEffect, useState } from "react";
import { cinemaClearDub, cinemaDub, cinemaDubbedShots, type DubOutcome } from "../../../api/cinema";
import type { EpisodeData, StoryboardShot } from "../../../api/cinema";
import { canDub, dubLabel, dubSummary, overflows, restitchNote, voicesUsed } from "./dubState";

export function DubBar({
  projectId,
  episode,
  hasFinal,
  onError,
}: {
  projectId: string;
  episode: EpisodeData;
  /** 这一集已经合成过成片。**配完音要再合成一次才会换上**，见 `restitchNote` */
  hasFinal: boolean;
  onError: (m: string) => void;
}) {
  const withLines = episode.storyboard.filter((s: StoryboardShot) => (s.dialogue_lines?.length ?? 0) > 0);
  const [dubbed, setDubbed] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<DubOutcome[]>([]);

  const refresh = useCallback(() => {
    void cinemaDubbedShots(projectId, episode.id).then(setDubbed).catch(() => setDubbed([]));
  }, [projectId, episode.id]);
  useEffect(refresh, [refresh]);

  const run = async () => {
    setBusy(true);
    try {
      setResult(await cinemaDub(projectId, episode.id));
      refresh();
    } catch (e) {
      onError(`配音没成：${e}`);
    } finally {
      setBusy(false);
    }
  };

  const clear = async () => {
    setBusy(true);
    try {
      const n = await cinemaClearDub(projectId, episode.id);
      setResult([]);
      refresh();
      if (n === 0) onError("本来就没有配音");
    } catch (e) {
      onError(`清除配音失败：${e}`);
    } finally {
      setBusy(false);
    }
  };

  const mine = dubbed.filter((id) => withLines.some((s: StoryboardShot) => s.id === id));
  const over = overflows(result);
  const voices = voicesUsed(result);

  return (
    <div className="flex flex-col gap-2 rounded-xl px-3.5 py-3" style={{ background: "var(--studio-bg-inset)" }}>
      <div className="flex flex-wrap items-center gap-2.5">
        <span className="studio-eyebrow">配音</span>
        <button
          type="button"
          disabled={!canDub(busy, withLines.length)}
          onClick={() => void run()}
          className="rounded-lg px-3 py-1.5 text-[12px] font-medium"
          style={{
            background: canDub(busy, withLines.length) ? "var(--studio-primary)" : "var(--studio-bg-surface)",
            color: canDub(busy, withLines.length) ? "var(--studio-on-primary)" : "var(--studio-text-muted)",
          }}
        >
          {dubLabel(busy, mine.length, withLines.length)}
        </button>
        {mine.length > 0 && (
          <button
            type="button"
            disabled={busy}
            onClick={() => void clear()}
            className="rounded-lg px-3 py-1.5 text-[12px]"
            style={{ background: "var(--studio-bg-surface)" }}
            title="删掉配音文件，换回 H3 生成的原声 —— 口型也就跟着对上了"
          >
            换回原声
          </button>
        )}
        {mine.length > 0 && (
          <span className="text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
            已配 {mine.length}/{withLines.length} 镜
          </span>
        )}
      </div>

      {/* **配完要重新合成一集** —— 配音只写 wav，不碰视频；换音轨发生在合成那一步。
          不说的话用户会以为配完就完了，而已经合成过的片子一点变化都没有。 */}
      {restitchNote(mine.length, hasFinal) && (
        <p className="text-[11px] leading-relaxed" style={{ color: "var(--studio-failed-fg)" }}>
          {restitchNote(mine.length, hasFinal)}
        </p>
      )}

      <p className="text-[11px] leading-relaxed" style={{ color: "var(--studio-text-muted)" }}>
        按角色资产里配的音色逐句合成，**替换掉 H3 生成的原声**。
        音色和方言归你管，代价是**口型对不上** —— H3 的嘴型是跟着它自己那段语音做的。
        没给角色配音色的，用设置里的全局音色。
        配音**不改分镜视频** —— H3 的原声原样留在里面，只是合成时不取它，
        所以「换回原声」随时能退回去。
      </p>

      {result.length > 0 && (
        <p className="text-[11px]" style={{ color: "var(--studio-text)" }}>
          {dubSummary(result)}
          {voices.length === 1 && withLines.length > 1 && (
            <span style={{ color: "var(--studio-failed-fg)" }}>
              {" "}· 全集只用了一把嗓子「{voices[0]}」—— 多半是角色音色没配上，回落到了全局设置
            </span>
          )}
        </p>
      )}

      {over.length > 0 && (
        <p className="text-[11px]" style={{ color: "var(--studio-failed-fg)" }}>
          这几镜配音比画面长，**多出来的那截会在合成时被砍掉**（要么改短台词，要么把镜头重出得长一点）：
          {over.map((o) => ` ${o.shot_id.slice(-6)} 超 ${o.secs.toFixed(1)}s`).join("；")}
        </p>
      )}
    </div>
  );
}
