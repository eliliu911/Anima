import type { AssetKind, VariantRole } from "../../api/assets";
import type { CanvasNode, CanvasEdge } from "../types";

/**
 * 画布产物提交到资产库时的「落点」。
 *
 * # 为什么要有类型
 *
 * 以前只能三选一（角色 / 场景 / 道具），主线拿到一张图只知道它是「场景」，
 * 不知道它是**主镜**还是**反打** —— 而空间契约要的恰恰是这一对。
 * 说不清是哪种的，下游就用不了它。
 *
 * 而「这张图是什么」**只有提交的那一刻知道**，事后没人认得出来。
 */
export type { VariantRole };

export interface CommitTarget {
  kind: AssetKind;
  role: VariantRole;
}

export const ROLE_LABEL: Record<VariantRole, string> = {
  portrait: "角色定妆",
  turnaround: "角色三视图",
  scene_master: "场景主镜",
  scene_reverse: "场景反打",
  scene_pano: "场景全景",
  reference: "参考图",
};

/** 这个角色能落在哪类资产上。和 Rust 侧 `VariantRole::allows` 必须一致。 */
export function roleAllows(role: VariantRole, kind: AssetKind): boolean {
  switch (role) {
    case "portrait":
    case "turnaround":
      return kind === "character";
    case "scene_master":
    case "scene_reverse":
    case "scene_pano":
      return kind === "scene";
    case "reference":
      return true;
  }
}

/** 某类资产可选的角色。界面上只列这些 —— 列出来又拒绝是最差的体验。 */
export function rolesFor(kind: AssetKind): VariantRole[] {
  return (Object.keys(ROLE_LABEL) as VariantRole[]).filter((r) => roleAllows(r, kind));
}

/**
 * 从产物的来源**推断**它该去哪个落点。
 *
 * 抄 DramaClaw 的 `inferDefaultTarget`：**系统猜，用户只需确认**。
 * 让用户从六个选项里自己挑，等于把我们的数据模型摊给他看。
 *
 * 推断只看得见的线索，看不出来就老实回落到「场景 · 参考图」——
 * **猜错比不猜更糟**，所以每一条都要有实打实的依据。
 */
export function inferCommitTarget(
  nodeId: string,
  nodes: CanvasNode[],
  edges: CanvasEdge[],
): CommitTarget {
  const node = nodes.find((n) => n.id === nodeId);
  const prompt = String((node?.data as { prompt?: unknown })?.prompt ?? "");
  const layout = String((node?.data as { layout?: unknown })?.layout ?? "");

  // ① 出图时用的版式最硬 —— 设定卡就是三视图，没有别的可能
  if (layout === "character_sheet" || layout === "entity_sheet" || layout === "quad_view") {
    return { kind: "character", role: "turnaround" };
  }

  // ② 上游连着角色节点 = 这张图是为那个角色出的
  const upstream = edges
    .filter((e) => e.target === nodeId)
    .map((e) => nodes.find((n) => n.id === e.source))
    .filter((n): n is CanvasNode => Boolean(n));
  if (upstream.some((n) => n.type === "character")) {
    return { kind: "character", role: "portrait" };
  }

  // ③ 提示词里的关键词。**放在最后**，因为它最弱 ——
  // 「反打」两个字也可能只是出现在一句描写里。
  const has = (...words: string[]) => words.some((w) => prompt.includes(w));
  if (has("三视图", "设定卡", "turnaround", "character sheet")) {
    return { kind: "character", role: "turnaround" };
  }
  if (has("反打", "reverse shot", "reverse master")) {
    return { kind: "scene", role: "scene_reverse" };
  }
  if (has("全景图", "360", "panorama", "equirectangular")) {
    return { kind: "scene", role: "scene_pano" };
  }
  if (has("主镜", "master shot", "establishing")) {
    return { kind: "scene", role: "scene_master" };
  }

  // 看不出来就别猜死。**回落是「参考图」不是某个具体角色** ——
  // 标错了下游会当真，标成通用的至少是诚实的。
  return { kind: "scene", role: "reference" };
}
