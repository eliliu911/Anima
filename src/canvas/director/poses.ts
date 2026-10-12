/**
 * 导演世界人偶的姿势表。
 *
 * 改写自 DramaClaw（ClaymoreLab, Elastic License 2.0）
 * frontend/src/features/viewer-kit/three-d/engine/poses.ts 和
 * engine/viewerApp.ts 的 QUATERNIUS_POSE_CLIPS。出处规则见 DECISIONS.md 2026-09-16。
 *
 * # 人偶从哪来
 *
 * Quaternius 的 Universal Animation Library（UAL1 / UAL2），**CC0 公有领域**，
 * 放在 `public/director/quaternius/`，许可证原文跟着文件走。两个库同一套骨骼（65 根），
 * 人偶模型取 UAL2，动作片段两个库合在一起用。
 *
 * # 姿势 = 某个动作片段在某一时刻的定格
 *
 * 没有专门的「姿势」资产，是从循环动作里取一帧。`sampleTime` 是 DC 挑好的那一刻；
 * 每个姿势给几个候选片段，前面的找不到才用后面的。
 */

export type PoseName =
  | "standing"
  | "talking"
  | "arms_crossed"
  | "sitting"
  | "eating"
  | "crouching"
  | "kneeling"
  | "lying"
  | "walking"
  | "running"
  | "pointing"
  | "holding"
  | "interacting"
  | "fighting"
  | "sword";

export const POSES: { id: PoseName; label: string }[] = [
  { id: "standing", label: "站立" },
  { id: "talking", label: "交谈" },
  { id: "arms_crossed", label: "抱臂" },
  { id: "sitting", label: "坐下" },
  { id: "eating", label: "进食" },
  { id: "crouching", label: "蹲伏" },
  { id: "kneeling", label: "下跪" },
  { id: "lying", label: "躺 / 倒地" },
  { id: "walking", label: "行走" },
  { id: "running", label: "奔跑" },
  { id: "pointing", label: "指向" },
  { id: "holding", label: "持物" },
  { id: "interacting", label: "操作 / 互动" },
  { id: "fighting", label: "格斗" },
  { id: "sword", label: "持械" },
];

export const POSE_CLIPS: Record<PoseName, { names: string[]; sampleTime: number }> = {
  standing: { names: ["Idle_Loop", "Idle_No_Loop", "Idle_FoldArms_Loop", "A_TPose"], sampleTime: 0.25 },
  talking: { names: ["Idle_Talking_Loop", "Idle_Rail_Call", "Yes"], sampleTime: 0.3 },
  arms_crossed: { names: ["Idle_FoldArms_Loop", "Idle_No_Loop"], sampleTime: 0.25 },
  sitting: { names: ["Sitting_Idle_Loop", "Sitting_Talking_Loop", "Idle_Rail_Loop"], sampleTime: 0.45 },
  eating: { names: ["Consume", "Idle_Talking_Loop", "Farm_Harvest"], sampleTime: 0.75 },
  crouching: { names: ["Crouch_Idle_Loop", "Crouch_Fwd_Loop"], sampleTime: 0.25 },
  kneeling: { names: ["Fixing_Kneeling", "Farm_PlantSeed"], sampleTime: 0.35 },
  lying: { names: ["LayToIdle", "Death01"], sampleTime: 0.02 },
  walking: { names: ["Walk_Loop", "Walk_Formal_Loop", "Walk_Carry_Loop"], sampleTime: 0.35 },
  running: { names: ["Sprint_Loop", "Jog_Fwd_Loop", "Shield_Dash_RM"], sampleTime: 0.25 },
  pointing: { names: ["Pistol_Aim_Neutral", "Spell_Simple_Shoot", "OverhandThrow"], sampleTime: 0.35 },
  holding: { names: ["Walk_Carry_Loop", "Idle_Lantern_Loop", "PickUp_Table"], sampleTime: 0.18 },
  interacting: { names: ["Interact", "Chest_Open", "Farm_Harvest", "Farm_PlantSeed"], sampleTime: 0.55 },
  fighting: { names: ["Punch_Cross", "Punch_Jab", "Melee_Hook"], sampleTime: 0.28 },
  sword: { names: ["Sword_Idle", "Sword_Block", "Sword_Regular_A"], sampleTime: 0.25 },
};

export function isPoseName(v: unknown): v is PoseName {
  return typeof v === "string" && POSES.some((p) => p.id === v);
}

export function poseLabel(p: PoseName | undefined): string {
  return POSES.find((x) => x.id === p)?.label ?? "站立";
}

/** 这个姿势实际用哪个片段。一个候选都没有返回 `null`（调用方报错，不静默站着）。 */
export function resolvePoseClip(pose: PoseName, available: Set<string>): { name: string; time: number } | null {
  const c = POSE_CLIPS[pose] ?? POSE_CLIPS.standing;
  const name = c.names.find((n) => available.has(n));
  return name ? { name, time: c.sampleTime } : null;
}

/**
 * 两个库里实际有的片段名（2026-09-16 用 three 的 GLTFLoader 读出来的）。
 * 测试拿它核对每个姿势都至少有一个片段 —— 映射表写错一个字，那个姿势就会
 * 静默退回 T-pose。
 */
export const KNOWN_CLIPS = [
  // UAL1
  "A_TPose", "Crouch_Fwd_Loop", "Crouch_Idle_Loop", "Dance_Loop", "Death01", "Driving_Loop", "Fixing_Kneeling",
  "Hit_Chest", "Hit_Head", "Idle_Loop", "Idle_Talking_Loop", "Idle_Torch_Loop", "Interact", "Jog_Fwd_Loop",
  "Jump_Land", "Jump_Loop", "Jump_Start", "PickUp_Table", "Pistol_Aim_Down", "Pistol_Aim_Neutral", "Pistol_Aim_Up",
  "Pistol_Idle_Loop", "Pistol_Reload", "Pistol_Shoot", "Punch_Cross", "Punch_Jab", "Push_Loop", "Roll", "Roll_RM",
  "Sitting_Enter", "Sitting_Exit", "Sitting_Idle_Loop", "Sitting_Talking_Loop", "Spell_Simple_Enter",
  "Spell_Simple_Exit", "Spell_Simple_Idle_Loop", "Spell_Simple_Shoot", "Sprint_Loop", "Swim_Fwd_Loop",
  "Swim_Idle_Loop", "Sword_Attack", "Sword_Attack_RM", "Sword_Idle", "Walk_Formal_Loop", "Walk_Loop",
  // UAL2
  "Chest_Open", "ClimbUp_1m_RM", "Consume", "Farm_Harvest", "Farm_PlantSeed", "Farm_Watering", "Hit_Knockback",
  "Hit_Knockback_RM", "Idle_FoldArms_Loop", "Idle_Lantern_Loop", "Idle_No_Loop", "Idle_Rail_Call", "Idle_Rail_Loop",
  "Idle_Shield_Break", "Idle_Shield_Loop", "Idle_TalkingPhone_Loop", "LayToIdle", "Melee_Hook", "Melee_Hook_Rec",
  "NinjaJump_Idle_Loop", "NinjaJump_Land", "NinjaJump_Start", "OverhandThrow", "Shield_Dash_RM", "Shield_OneShot",
  "Slide_Exit", "Slide_Loop", "Slide_Start", "Sword_Block", "Sword_Dash_RM", "Sword_Regular_A", "Sword_Regular_A_Rec",
  "Sword_Regular_B", "Sword_Regular_B_Rec", "Sword_Regular_C", "Sword_Regular_Combo", "TreeChopping_Loop",
  "Walk_Carry_Loop", "Yes", "Zombie_Idle_Loop", "Zombie_Scratch", "Zombie_Walk_Fwd_Loop",
];
