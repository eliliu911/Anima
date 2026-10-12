/**
 * 分镜详情「导演世界」一栏的纯函数：这一镜有哪些人偶稿、每张稿里的人偶是谁。
 *
 * 人偶稿要换成角色才能当首帧（见 Rust `studio_director`）：换的时候按**颜色**对人。
 * - 「输出」那一张带 `frame_meta.actors`（颜色名、角色、在画面哪、姿势、朝向），原样用；
 * - 运镜首尾帧没带元数据，用场面里的人偶（颜色 → 调色板名字），不知道在画面哪就不写位置。
 */
import type { DirectorSceneState, DraftActor } from "../../../api/cinema";
import { ACTOR_PALETTE } from "../../../canvas/director/actorPalette";

export interface DirectorDraft {
  /** 界面上的名字 */
  label: string;
  path: string;
  /** 默认挂到哪一格 */
  slot: "first" | "last";
  actors: DraftActor[];
}

type SceneActor = { label?: unknown; color?: unknown; assetId?: unknown; kind?: unknown; imagePath?: unknown };

function sceneActors(state: DirectorSceneState | null | undefined): DraftActor[] {
  const raw = (state?.scene as { actors?: SceneActor[] } | undefined)?.actors;
  if (!Array.isArray(raw)) return [];
  return raw.map((a) => {
    const hex = typeof a.color === "string" ? a.color : "";
    return {
      asset_id: typeof a.assetId === "string" && a.assetId ? a.assetId : null,
      label: typeof a.label === "string" ? a.label : "",
      color_name: ACTOR_PALETTE.find((p) => p.hex.toLowerCase() === hex.toLowerCase())?.name ?? null,
      marker_color: hex,
      in_frame: true,
      role: a.kind === "prop" || a.kind === "extra" ? a.kind : "person",
      image_path: typeof a.imagePath === "string" && a.imagePath ? a.imagePath : null,
    };
  });
}

/** 这一镜能拿来出首帧 / 尾帧的人偶稿，新的在前 */
export function directorDrafts(state: DirectorSceneState | null | undefined): DirectorDraft[] {
  const out: DirectorDraft[] = [];
  const t = state?.trajectory;
  if (t?.first) out.push({ label: "运镜首帧（人偶稿）", path: t.first, slot: "first", actors: sceneActors(state) });
  if (t?.last) out.push({ label: "运镜尾帧（人偶稿）", path: t.last, slot: "last", actors: sceneActors(state) });
  const o = state?.outputs;
  if (o?.combined) out.push({ label: "导演合成图（人偶稿）", path: o.combined, slot: "first", actors: o.actors ?? [] });
  // 新的在前：运镜和输出谁晚谁排前面
  if (t && o && o.at > t.at) out.unshift(...out.splice(out.length - 1, 1));
  return out;
}

/** 人偶里有没有没绑角色的（出首帧时会被换成「路人」，要提示） */
export function anonymousCount(actors: DraftActor[]): number {
  // 群演本来就是路人、道具不是人：只数角色
  return actors.filter((a) => a.in_frame !== false && !a.asset_id && (a.role ?? "person") === "person").length;
}
