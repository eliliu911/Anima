/**
 * 站位跨镜头沿用（用户 2026-09-27：「后面几镜，或者某几个场，用的一个场景，每次都要重建是不是很麻烦啊？而且人物位置可能会放错。」）
 *
 * 拍戏的做法是人摆一次、换机位拍好几镜。所以：
 * - 一镜**还没摆过**时，打开就沿用同一个场景里最近那一镜的人、群演、道具（坐标原样）—— 机位不带，每镜景别角度不同；
 * - 随时能「从第 N 镜复制站位」，跳着沿用 —— **只限同一个场景**：站位是片场里的绝对坐标，
 *   换一个全景 / 3D 片场，原点、朝向、比例都不一样，原样搬过去人会在墙里、地下或画外（审查 2026-09-27）；
 * - 沿用过来之后按这一镜的出场人物核对：多出来的问删不删，缺的提示去放。
 *
 * 3D 片场本来就不用重建：它存在场景资产上，一个地点一份（`ShotDirectorWorld`）。
 */
import type { StoryboardShot } from "../../api/cinema";
import { isPerson, readActors, type DirectorActor } from "./scene";

export interface BlockingSource {
  shotId: string;
  shotNum: number;
  actors: DirectorActor[];
}

type ShotLike = Pick<StoryboardShot, "id" | "shot_num" | "scene_id" | "director_scene">;

function savedScene(shot: ShotLike): { actors?: unknown } | null {
  const s = shot.director_scene?.scene;
  return s && typeof s === "object" ? (s as { actors?: unknown }) : null;
}

/**
 * 这一镜摆过没有。**存过空的也算摆过** —— 用户点了「清空重摆」就是要空的，下次打开不能又沿用回来。
 */
export function hasSavedBlocking(shot: ShotLike): boolean {
  return Array.isArray(savedScene(shot)?.actors);
}

/**
 * 能复制站位的来源：这一集里**同一个场景**摆过人的镜头（不含自己），按离这一镜的远近。
 * 没有场景资产的镜头没有来源 —— 「空世界」之间谈不上同一个地方。
 */
export function blockingSources(shots: ShotLike[], current: ShotLike): BlockingSource[] {
  const scene = current.scene_id || null;
  if (!scene) return [];
  return shots
    .filter((s) => s.id !== current.id && s.scene_id === scene)
    .map((s) => ({ shotId: s.id, shotNum: s.shot_num, actors: readActors(savedScene(s)?.actors) }))
    .filter((s) => s.actors.length > 0)
    .sort((a, b) => Math.abs(a.shotNum - current.shot_num) - Math.abs(b.shotNum - current.shot_num) || a.shotNum - b.shotNum);
}

/**
 * 还没摆过的一镜打开时沿用哪一镜：**同一个场景**、镜号在它之前最近的那一镜；前面没有就用后面最近的。
 * 没有场景资产的镜头不沿用（「空世界」之间谈不上同一个地方）。
 */
export function inheritFrom(shots: ShotLike[], current: ShotLike): BlockingSource | null {
  if (hasSavedBlocking(current)) return null;
  const same = blockingSources(shots, current);
  const before = same.filter((s) => s.shotNum < current.shot_num).sort((a, b) => b.shotNum - a.shotNum);
  const after = same.filter((s) => s.shotNum > current.shot_num).sort((a, b) => a.shotNum - b.shotNum);
  return before[0] ?? after[0] ?? null;
}

/**
 * 按这一镜的分镜核对场上：
 * - `absent`：场上绑了角色、但这一镜分镜里没有的人（沿用过来最常见）；
 * - `missing`：这一镜要出场、场上还没有的角色（只在「能出镜」的颜色表里找）；
 * - `missingProps`：这一镜点了名、场上还没有的道具。
 *
 * 群演、没绑角色的人不核对；**场上多出来的道具不提示** —— 拆机台这种摆设每镜都在，分镜不一定每镜都点名。
 */
export function castCheck(
  actors: DirectorActor[],
  shot: { character_ids?: string[]; prop_ids?: string[] } | null,
  colorTable: { asset_id: string; name: string }[],
  propAssets: { id: string; name: string }[] = [],
): { absent: DirectorActor[]; missing: { asset_id: string; name: string }[]; missingProps: { id: string; name: string }[] } {
  const none = { absent: [], missing: [], missingProps: [] };
  if (!shot) return none;
  const onSet = new Set(actors.map((a) => a.assetId).filter(Boolean));
  const people = actors.filter(isPerson);
  let absent: DirectorActor[] = [];
  let missing: { asset_id: string; name: string }[] = [];
  if (Array.isArray(shot.character_ids)) {
    const want = new Set(shot.character_ids);
    absent = people.filter((a) => a.assetId && !want.has(a.assetId));
    missing = colorTable.filter((c) => want.has(c.asset_id) && !people.some((a) => a.assetId === c.asset_id));
  }
  const wantProps = new Set(Array.isArray(shot.prop_ids) ? shot.prop_ids : []);
  const missingProps = propAssets.filter((p) => wantProps.has(p.id) && !onSet.has(p.id));
  return { absent, missing, missingProps };
}
