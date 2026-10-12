/**
 * 分镜详情里的「导演世界」一栏：这个地点的 3D 片场、打开导演世界、把人偶稿换成角色挂进关键帧。
 *
 * 入口本来只在画布上（画布 → 项目 → 建节点 → 进导演世界），用户 2026-09-27 要求工作区里直接有。
 */
import { useState } from "react";
import type { AssetView } from "../../../api/assets";
import { cinemaGenerateSceneWorld, cinemaRenderFrameFromDirector, type StoryboardShot } from "../../../api/cinema";
import { MediaThumb } from "../MediaThumb";
import { anonymousCount, directorDrafts } from "./directorFrames";

export function DirectorBlock({
  projectId,
  episodeId,
  shot,
  sceneAsset,
  frameRunning,
  onOpen,
  onChanged,
}: {
  projectId: string;
  episodeId: string;
  shot: StoryboardShot;
  /** 这一镜的场景资产。没有就只能在空世界里摆人 */
  sceneAsset: AssetView | null;
  frameRunning: boolean;
  onOpen: () => void;
  /** 片场生成完 / 关键帧挂上之后重读 */
  onChanged: () => void;
}) {
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const world = sceneAsset?.scene?.world ?? null;
  const drafts = directorDrafts(shot.director_scene);

  const run = async (key: string, work: () => Promise<unknown>) => {
    setBusy(key);
    setError(null);
    try {
      await work();
      onChanged();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const btn = "rounded-md px-2.5 py-1 text-[11.5px] disabled:opacity-40";
  const btnStyle = { background: "var(--studio-bg-elevated)", boxShadow: "0 0 0 1px var(--studio-border)", color: "var(--studio-text)" } as const;
  const locked = busy !== null || frameRunning;
  return (
    <div className="flex flex-col gap-2 text-[12px]">
      <div className="flex flex-wrap items-center gap-2">
        <span>
          {sceneAsset ? `场景「${sceneAsset.name}」` : "这一镜没有场景资产"}
          <span className="ml-1.5 text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
            {!sceneAsset ? "只能在空世界里摆人、定机位" : world ? "3D 片场已生成（这个地点的镜头共用）" : sceneAsset.cover_url ? "还没有 3D 片场，先用全景" : "还没有图"}
          </span>
        </span>
        {sceneAsset?.cover_url && (
          <button type="button" className={btn} style={btnStyle} disabled={locked}
            title="用这个场景的全景图生成 3D 片场（SHARP，十几秒）。单张图生成的片场只在拍摄点附近、左右 28° 内可靠"
            onClick={() => void run("world", () => cinemaGenerateSceneWorld(projectId, episodeId, sceneAsset.id))}>
            {busy === "world" ? "正在生成 3D 片场…" : world ? "重新生成 3D 片场" : "生成 3D 片场"}
          </button>
        )}
        <button type="button" className={btn} style={{ ...btnStyle, color: "var(--studio-primary)" }} disabled={busy !== null} onClick={onOpen}>
          打开导演世界
        </button>
      </div>
      <p className="text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
        在导演世界里摆人、定机位、做运镜，输出的是「片场 + 彩色人偶」的人偶稿。人偶稿要换成角色才能当首帧 —— 直接挂上去出片，会出一段人偶戏。
      </p>
      {drafts.length > 0 && (
        <div className="grid gap-2" style={{ gridTemplateColumns: "repeat(auto-fill, minmax(150px, 1fr))" }}>
          {drafts.map((d) => {
            const anon = anonymousCount(d.actors);
            return (
              <div key={d.path} className="flex min-w-0 flex-col gap-1">
                <span className="text-[11px]" style={{ color: "var(--studio-text-2)" }}>{d.label}</span>
                <div className="aspect-video overflow-hidden rounded-md" style={{ background: "var(--studio-bg-elevated)" }}>
                  <MediaThumb path={d.path} alt={d.label} className="h-full w-full object-contain" />
                </div>
                <div className="flex flex-wrap gap-1">
                  {(["first", "last"] as const).map((slot) => (
                    <button key={slot} type="button" className={btn} style={btnStyle} disabled={locked}
                      title="试验：用 Qwen 2.1 参考生图按人偶颜色换成角色（约 5~6 分钟），结果挂进这一镜的关键帧。换好之后要重新合成这一镜"
                      onClick={() => void run(`${d.path}:${slot}`, () => cinemaRenderFrameFromDirector(projectId, episodeId, shot.id, slot, d.path, d.actors))}>
                      {busy === `${d.path}:${slot}` ? "正在换人…" : slot === "first" ? "换成角色 → 首帧" : "换成角色 → 尾帧"}
                    </button>
                  ))}
                </div>
                {anon > 0 && <span className="text-[10.5px]" style={{ color: "var(--studio-accent)" }}>{anon} 个人偶没绑角色，会换成路人</span>}
              </div>
            );
          })}
        </div>
      )}
      {drafts.length > 0 && <p className="text-[10.5px]" style={{ color: "var(--studio-accent)" }}>「换成角色」是试验功能：人偶的位置、姿势守不守得住、脸像不像还没同题对照过，出完先看一眼再出片。</p>}
      {error && <p className="text-[11px]" style={{ color: "var(--studio-failed-fg)" }}>{error}</p>}
    </div>
  );
}
