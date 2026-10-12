import type { SceneStage } from "../../../api/cinema";

/**
 * 这一场固定布景的显示行。和 Rust `SceneStage::lines` 同一个顺序、同一套标签 ——
 * 界面上看到的就是发给模型的那几行。空的项不显示。
 */
export function stageLines(stage: SceneStage | undefined | null): { label: string; text: string }[] {
  if (!stage) return [];
  const join = (xs: string[] | undefined) => (xs ?? []).map((x) => x.trim()).filter(Boolean).join("；");
  const rows: [string, string][] = [
    ["方向基准", (stage.facing ?? "").trim()],
    ["固定物", join(stage.anchors)],
    ["光源", join(stage.lights)],
    ["主关系与机位侧", (stage.axis ?? "").trim()],
    ["开场站位", join(stage.people)],
    ["时段天气", (stage.time ?? "").trim()],
  ];
  return rows.filter(([, text]) => text).map(([label, text]) => ({ label, text }));
}
