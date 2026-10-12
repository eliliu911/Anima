/**
 * 人物调色盘。**和 `src-tauri/src/actor_colors.rs` 的 `PALETTE` 必须逐项一致**
 * （`actorPalette.test.ts` 直接读 Rust 源码核对）。
 *
 * 取自 DramaClaw（ClaymoreLab, Elastic License 2.0）
 * `src/novelvideo/generators/episode_optimizer.py` 的 `BRIDGMAN_CHARACTER_PALETTE`。
 *
 * 分配在后端做（`cinema_assign_actor_colors`）；前端只在两处用它：
 * 手动改色时列出可选颜色，以及自由模式（没有分集上下文）下给匿名人物配色。
 */
export const ACTOR_PALETTE: { hex: string; name: string }[] = [
  { hex: "#FF00FF", name: "FLUORESCENT MAGENTA" },
  { hex: "#00FFFF", name: "FLUORESCENT CYAN" },
  { hex: "#CCFF00", name: "FLUORESCENT LIME" },
  { hex: "#FF6B00", name: "FLUORESCENT ORANGE" },
  { hex: "#7C4DFF", name: "ELECTRIC VIOLET" },
  { hex: "#00FF66", name: "NEON MINT" },
  { hex: "#00A2FF", name: "ELECTRIC AZURE" },
  { hex: "#FFD400", name: "SIGNAL YELLOW" },
  { hex: "#9D00FF", name: "NEON PURPLE" },
  { hex: "#00FFCC", name: "FLUORESCENT AQUA" },
  { hex: "#39FF14", name: "LASER GREEN" },
  { hex: "#5C6BC0", name: "INDIGO" },
];

/** 自由模式下一个匿名人物的颜色：调色盘里第一个没被占用的，都占了就按序轮 */
export function nextFreeColor(used: string[]): string {
  const taken = new Set(used.map((c) => c.toUpperCase()));
  return ACTOR_PALETTE.find((p) => !taken.has(p.hex))?.hex ?? ACTOR_PALETTE[used.length % ACTOR_PALETTE.length].hex;
}
