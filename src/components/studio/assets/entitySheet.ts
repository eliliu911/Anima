/**
 * 完整参考表（entity sheet）—— 界面这一侧的纯逻辑。
 *
 * 这条路和「设定卡」（`sheet` 模式）**不是一回事**，别因为名字像就混用：
 *
 * | | 设定卡 | 参考表（这里） |
 * |---|---|---|
 * | 引擎 | SDXL | Krea 2 Turbo |
 * | 输入 | 只有文字 | **必须有源图** |
 * | 产出 | 一排三视图 | 七个区 |
 * | 脸的像素 | 42 px | hero 186 / 表情 178~189 / 六格细节 |
 * | 耗时 | 39s | 约 7.5 分钟 |
 *
 * Rust 侧的实现和硬约束在 `src-tauri/src/agent_runtime/entity_sheet.rs`。
 *
 * # 为什么身份锚点要用户能改
 *
 * 那七个区之间的一致性全靠 `IDENTITY LOCKS` 这几行撑着，而它们**必须具体
 * 可验**（「额头 M 纹加三道竖纹」「垂颊」「尾巴有深色环」）。
 * 写成「有特色的外形」这种笼统说法，各视角之间就会漂 —— 和构图模板里
 * 「表情写成『不同的表情』就四格长一样」是同一个坑。
 *
 * 资产描述是逗号/顿号分隔的短语，正好能当锚点的初值；但它未必写全了
 * （比如没提尾巴），所以这一栏可改。
 */

/** 实体类型。**和 Rust 侧 `ENTITY_TYPES` 是同一份名单，改要一起改。** */
export const ENTITY_TYPES = [
  "HUMAN",
  "STYLIZED_CHARACTER",
  "ANIMAL",
  "CREATURE",
  "ROBOT",
  "VEHICLE",
  "OBJECT",
] as const;

export type EntityType = (typeof ENTITY_TYPES)[number];

/** 给人看的名字。下拉里显示这个，发给模型的是上面那些英文。 */
export const ENTITY_TYPE_LABELS: Record<EntityType, string> = {
  HUMAN: "真人",
  STYLIZED_CHARACTER: "风格化角色",
  ANIMAL: "动物",
  CREATURE: "生物 / 怪物",
  ROBOT: "机械 / 机器人",
  VEHICLE: "载具",
  OBJECT: "物件",
};

/**
 * 按资产种类和描述猜一个实体类型。
 *
 * **只是初值，用户可改。** 猜错了不会静默出错 —— 下拉里看得见。
 * 认不出的一律给 `STYLIZED_CHARACTER`：那是训练分布的正中间，
 * 比塞一个模型没见过的词安全（Rust 侧 `normalize_entity_type` 同样口径）。
 */
export function guessEntityType(kind: string, description: string): EntityType {
  const d = description.toLowerCase();
  if (kind === "prop") {
    if (/车|船|飞机|坦克|摩托|vehicle|car|ship|plane/.test(d)) return "VEHICLE";
    return "OBJECT";
  }
  if (kind === "scene") return "OBJECT";
  if (/机器人|机械|robot|android|mech/.test(d)) return "ROBOT";
  if (/怪|兽|龙|妖|creature|monster|dragon|beast/.test(d)) return "CREATURE";
  if (/猫|狗|鸟|马|兔|狐|animal|cat|dog|bird|horse/.test(d)) return "ANIMAL";
  return "STYLIZED_CHARACTER";
}

/**
 * 把资产描述切成身份锚点的初值。
 *
 * 按中英文标点一起切，并**丢掉没有信息量的段**（「无」「未知」这类）——
 * 那些段进了提示词只会占位置。上限 8 条：作者模板写的是 4~7 条，
 * 多了会开始互相矛盾。
 */
export function locksFromDescription(description: string): string[] {
  return description
    .split(/[,，、;；\n]/)
    .map((s) => s.trim())
    .filter((s) => s.length > 0)
    .filter((s) => !/^(无|没有|未知|不详|none|n\/a|na|-)$/i.test(s))
    .slice(0, 8);
}

/**
 * 图生那两个模板（四视图 / 七分区）现在能不能用。
 *
 * **必须有源图** —— 它们是图生图，身份全部来自那张图。
 * 返回不能用的理由，能用就返回 `null`（惯例和别处一致）。
 *
 * 文案对两种版式通用 —— 说的是「这条路要什么」，不是某个版式的产出。
 */
export function sheetBlocker(hasImage: boolean, engineAvailable: boolean): string | null {
  if (!hasImage) {
    return "图生模板是拿现有的图扩视角，所以先得有一张封面图 —— 切到「文生」出一张、或者上传一张再来。";
  }
  if (!engineAvailable) {
    return "图生模板要 Krea 2 引擎，当前 ComfyUI 里没有它。装好底模再重启 ComfyUI。";
  }
  return null;
}

export const REF_BOOST_MAX = 10;
export const REF_BOOST_DEFAULT = 4;

/** 旋钮当前值对应的说明。**说结果，不说形容词。** */
export function refBoostHint(v: number): string {
  if (v <= 0.9) return "几乎不看源图 —— 会自由发挥，身份会漂";
  if (v < 2) return "宽松：构图更自由，动作几格可能长得差不多";
  if (v <= 6) return "推荐：动作三姿会真的不同，版式也更听话";
  return "很紧：版式最听话，但「换装/去掉某物」这类要求会开始失败";
}

/**
 * 四视图那几格。
 *
 * **它和七分区的分工是「身份参考」vs「取材」** —— 转身的脸 135~199 px，
 * 在可用下限（150~200）之上；而七分区的转身只有 110 px，当不了身份参考。
 * 拿去喂 R2V / 做分镜首帧 / 当封面的，该是这一张。
 */
export const QUAD_ZONES = [
  "一格大脸特写（约 486px —— 目前所有版式里最大）",
  "正面全身",
  "侧面全身",
  "背面全身",
] as const;

/** 七个区分别是什么 —— 界面上先讲清楚，免得等了 8 分钟发现不是想要的。 */
export const SHEET_ZONES = [
  "元数据栏（正文是涂鸦，只当装饰）",
  "大幅主视图",
  "正 / 侧 / 背转身排（脸只有约 110px，别当身份参考）",
  "三个动作姿态",
  "三个剪影",
  "表情研究（三张脸变化不大）",
  "六格细节裁切 —— 这格最值钱",
] as const;
