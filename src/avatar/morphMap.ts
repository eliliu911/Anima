/**
 * 把模型里的 morph target **自动认出来**，对上我们那 15 个口型。
 *
 * ## 为什么这是单独一个文件
 *
 * 这段逻辑全是字符串名字的匹配 —— 是整条链路里最容易错、而且错了**完全没有
 * 报错**的一段（名字对不上就是"模型加载正常但嘴不动"）。埋在组件的
 * `useEffect` 里就没法测，所以抽成纯函数：进去一个 `Object3D`，出来一张
 * 落点表。
 *
 * ## 认哪几套命名
 *
 * 按精度从高到低试：
 *
 * 1. **原生 viseme**（`viseme_aa` / `aa`）。Oculus 那套 15 个，
 *    是我们的时间线直接对应的东西，一一对应，没有损失。
 * 2. **Character Creator 音素集**（`B_M_P` / `Ah` / `W_OO` …）。按音素分组，
 *    也是为发音设计的，精度接近原生。
 * 3. **ARKit blendshape**（`jawOpen` / `mouthPucker` …）。那套是给**表情**
 *    设计的、不是给发音设计的，所以只能拼近似。
 * 4. **五元音**（`Fcl_MTH_A` / `aa,ih,ou,ee,oh` / `あいうえお`）。
 *    VRM / VRoid / MMD 用的就是这套。
 *
 * 最后一套是 2026 年 1 月 Ready Player Me 关停之后**最重要的一套** ——
 * RPM 的公开 API 和 Avatar Creator 都下线了，现在还活着的免费渠道
 * （VRoid Studio 是主要的一个）导出的是 VRM。不认这套的话，用户从任何
 * 现存渠道拿到的模型都会显示"通道数 0"。
 */
import type { Object3D, Mesh } from "three";
import type { Viseme } from "../api/voice";
import { VISEMES } from "./mouth";

/** 认得出来的几套 morph target 命名 */
export type MorphScheme = "oculus" | "cc" | "arkit" | "vowel" | "mmd" | "none";

export const SCHEME_LABEL: Record<MorphScheme, string> = {
  oculus: "原生 viseme，精度最好",
  cc: "Character Creator 音素集，精度接近原生",
  arkit: "ARKit 降级合成，区分度有限",
  vowel: "五元音（VRM / VRoid），辅音靠近似",
  mmd: "MMD 英文表情集，只有一个张口形，元音靠开口度近似",
  none: "没认出任何口型通道",
};

/** 一个 morph target 的落点：某个 mesh 的某个下标 */
export interface Target {
  influences: number[];
  index: number;
  /** 这个口型落在这个通道上的最大幅度 */
  gain: number;
}

export interface MorphMap {
  /** 口型 → 落点。一个口型可能落在多个 mesh 上（头、牙、舌各有一份） */
  targets: Map<Viseme, Target[]>;
  blink: Target[];
  smile: Target[];
  joy: Target[];
  angry: Target[];
  sorrow: Target[];
  fun: Target[];
  surprised: Target[];
  scheme: MorphScheme;
  /** 模型里实际有的全部 morph target 名。认不出来时这是唯一能查的线索 */
  found: string[];
  /**
   * 带口型通道的那些 mesh —— **那就是脸**。
   *
   * 相机靠它定位。原先是按"高 > 宽 × 2"猜是不是全身，
   * 而 T-pose 的双臂展开跟身高差不多宽，判断直接失效，
   * 于是把整个人框进画面、脸只有几十个像素（实测踩到）。
   * 用带口型的 mesh 当基准就没有这个问题：不管什么姿势，脸就是脸。
   */
  faceMeshes: Object3D[];
}

/**
 * Character Creator / iClone 的口型集。
 *
 * **这套名字是查证过的，不是猜的。** 之前这里写的是 `V_aa`、`V_PP` 这种
 * "V_ 加上我们自己的名字"，那纯粹是我按 Oculus 那套推出来的，
 * CC 里根本没有这些名字 —— 也就是说从 CC 导出的模型一个通道都认不出来，
 * 而测试只能证明代码做了我写的事，证明不了名字是真的。
 *
 * CC 按**音素**分组（`B_M_P` 是双唇音那一组），和 Oculus 按口型分组的方式
 * 不完全一一对应，所以有几处是多对一（`T_L_D_N` 同时供 DD 和 nn 用）。
 * 但它是为发音设计的，精度接近原生，明显好过 ARKit 那种拿表情去凑。
 */
const CC: Record<Viseme, string[]> = {
  sil: [],
  PP: ["B_M_P"],
  FF: ["F_V"],
  TH: ["Th"],
  DD: ["T_L_D_N"],
  kk: ["K_G_H_NG"],
  CH: ["Ch_J"],
  SS: ["S_Z"],
  nn: ["T_L_D_N"],
  RR: ["R", "Er"],
  aa: ["Ah"],
  E: ["AE"],
  I: ["EE", "Ih"],
  O: ["Oh"],
  U: ["W_OO"],
};

/**
 * ARKit blendshape 的降级映射。
 *
 * ARKit 那套是给表情设计的，不是给发音设计的，所以几个前元音（i / e）
 * 只能靠张口度和嘴角拉伸去近似，区分度有限。但它让"手上只有 ARKit 模型"
 * 这种情况可用，而不是干瞪眼。
 */
const ARKIT: Record<Viseme, Array<[string, number]>> = {
  sil: [],
  PP: [["mouthClose", 0.7]],
  FF: [
    ["mouthShrugUpper", 0.45],
    ["jawOpen", 0.12],
  ],
  TH: [
    ["jawOpen", 0.22],
    ["tongueOut", 0.3],
  ],
  DD: [["jawOpen", 0.26]],
  kk: [["jawOpen", 0.3]],
  CH: [
    ["jawOpen", 0.2],
    ["mouthFunnel", 0.35],
  ],
  SS: [
    ["jawOpen", 0.12],
    ["mouthStretchLeft", 0.3],
    ["mouthStretchRight", 0.3],
  ],
  nn: [
    ["jawOpen", 0.16],
    ["mouthClose", 0.3],
  ],
  RR: [
    ["jawOpen", 0.24],
    ["mouthFunnel", 0.3],
  ],
  aa: [["jawOpen", 0.78]],
  E: [
    ["jawOpen", 0.34],
    ["mouthStretchLeft", 0.25],
    ["mouthStretchRight", 0.25],
  ],
  I: [
    ["jawOpen", 0.18],
    ["mouthSmileLeft", 0.3],
    ["mouthSmileRight", 0.3],
  ],
  O: [
    ["jawOpen", 0.42],
    ["mouthFunnel", 0.55],
  ],
  U: [
    ["mouthPucker", 0.72],
    ["jawOpen", 0.14],
  ],
};

/**
 * MMD 英文表情集（`Mouth_A01` / `Eye_WinkA_L` / `Brow_Smily_L` 这一套）。
 *
 * 大量二次元模型（原神一类的同人模型是典型）导出后是这套名字。它既不是
 * Fcl_ 那套，也不是日文原名，所以前面四套一个都对不上 —— 表现是"通道数 0，
 * 嘴完全不动"。
 *
 * ## 这套的硬伤：**只有一个元音形**
 *
 * 五元音那套至少有 あいうえお 五个形，这套只有 `Mouth_A01`。所以 I/U/E/O
 * 没有对应的嘴形，只能拿开口度和展唇去凑。说话时看着是在动、口型跟得上
 * 节奏，但分不出具体元音 —— 这是模型的上限，不是代码的。
 *
 * ## 每个形状是**量出来的，不是按名字猜的**
 *
 * 按名字猜过一次就知道有多险：`Mouth_Open01` 听着像"张嘴"，实测**只有 31 个
 * 顶点在动、下张 7.7**（`Mouth_A01` 是 567 个点、下张 61.1）—— 它是个微调，
 * 拿它当张口形的话嘴基本不动。
 *
 * 实测数据（相对模型高度的千分比）：
 *
 * ```
 * Mouth_A01      动点 567  下张 61.1  前噘 39.4   ← 主力张口，带圆唇
 * Mouth_Smile01  动点 190  下张  0.0  横展 20.0   ← 闭口展唇，给 I / E 用
 * Mouth_Line01   动点 194  下张  0.1  横展 17.8   ← 抿平的闭唇
 * Mouth_Line02   动点 236  下张  0.4  横展 33.4   ← 更用力的展唇平线
 * Mouth_Open01   动点  31  下张  7.7             ← 名不副实，不用
 * ```
 */
const MMD_EN: Record<Viseme, Array<[string, number]>> = {
  sil: [],
  // 双唇音：抿住。这套难得有真正的闭唇形，比"所有通道归零"准
  PP: [["Mouth_Line01", 0.6]],
  FF: [["Mouth_Line02", 0.4]],
  TH: [["Mouth_A01", 0.22]],
  DD: [["Mouth_A01", 0.28]],
  kk: [["Mouth_A01", 0.4]],
  // A01 本身带 39 的前噘，所以圆唇类直接用它的小幅度
  CH: [["Mouth_A01", 0.3]],
  SS: [["Mouth_Line02", 0.5]],
  nn: [["Mouth_Line01", 0.5]],
  RR: [["Mouth_A01", 0.3]],
  aa: [["Mouth_A01", 1]],
  // 前元音靠"张口 + 展唇"合成 —— 这套没有 え/い 的形
  E: [
    ["Mouth_A01", 0.5],
    ["Mouth_Smile01", 0.35],
  ],
  I: [
    ["Mouth_Smile01", 0.6],
    ["Mouth_A01", 0.15],
  ],
  // O / U 只能是 A01 的不同幅度。**区分度确实有限，这是模型的上限** ——
  // 与其编一个假的圆唇形，不如让开口度跟着音节走
  O: [["Mouth_A01", 0.7]],
  U: [["Mouth_A01", 0.3]],
};

/** 五元音在各家里的叫法，外加一个「闭口」 */
const VOWEL_NAMES = {
  aa: ["Fcl_MTH_A", "aa", "A", "あ", "vrc.v_aa"],
  I: ["Fcl_MTH_I", "ih", "I", "い", "vrc.v_ih"],
  U: ["Fcl_MTH_U", "ou", "U", "う", "vrc.v_ou"],
  E: ["Fcl_MTH_E", "ee", "E", "え", "vrc.v_e"],
  O: ["Fcl_MTH_O", "oh", "O", "お", "vrc.v_oh"],
  // MMD 的「ん」是闭口鼻音，VRoid 有 Fcl_MTH_Close。
  // 有它就补上了五元音最大的短板 —— 那套本来没有闭嘴形，
  // 双唇音（m/b/p）只能靠"所有通道归零"凑，而归零和"抿住嘴"看起来不一样
  closed: ["ん", "Fcl_MTH_Close", "mouthClose"],
} as const;

/**
 * 每个口型用哪个元音、给多少。
 *
 * 辅音只能用元音近似 —— 五元音体系里本来就没有辅音形。这是 VTuber 那边
 * 通行的做法，看起来仍然像在说话，只是嘴唇的闭合动作没有原生 viseme 准。
 *
 * 双唇音（PP）故意映射成**空**：五元音里没有"闭嘴"这个形，而所有通道归零
 * 本身就是闭嘴 —— 这比拿一个小张口去凑要准。
 */
const VOWEL_APPROX: Record<
  Viseme,
  Array<[keyof typeof VOWEL_NAMES, number]>
> = {
  sil: [],
  // 有闭口形就用它；没有就留空 —— 所有通道归零本身也是闭嘴，
  // 比拿一个小张口去凑准
  PP: [["closed", 1]],
  FF: [["I", 0.25]],
  TH: [["aa", 0.3]],
  DD: [["aa", 0.35]],
  kk: [["aa", 0.45]],
  CH: [["U", 0.4]],
  SS: [["I", 0.35]],
  // 鼻音优先用闭口形，没有才拿小张口凑
  nn: [
    ["closed", 0.55],
    ["aa", 0.25],
  ],
  RR: [["U", 0.35]],
  aa: [["aa", 1]],
  E: [["E", 1]],
  I: [["I", 1]],
  O: [["O", 1]],
  U: [["U", 1]],
};

const BLINK_NAMES: Array<[string, number]> = [
  ["eyeBlinkLeft", 1],
  ["eyeBlinkRight", 1],
  ["eyesClosed", 1],
  ["Fcl_EYE_Close", 1],
  // MMD
  ["まばたき", 1],
  // MMD 英文集：实测 469 个顶点在动、明显下压，是真闭眼
  ["Eye_WinkA_L", 1],
  ["Eye_WinkA_R", 1],
];

const SMILE_NAMES: Array<[string, number]> = [
  ["mouthSmileLeft", 1],
  ["mouthSmileRight", 1],
  ["mouthSmile", 1],
  ["Fcl_ALL_Joy", 1],
  // MMD：笑 / 嘴角上扬
  ["笑い", 1],
  ["口角上げ", 0.6],
  // MMD 英文集
  ["Mouth_Smile01", 1],
  ["Brow_Smily_L", 0.5],
  ["Brow_Smily_R", 0.5],
];

const JOY_NAMES: Array<[string, number]> = [
  ["Fcl_ALL_Joy", 1],
  ["Fcl_MTH_Joy", 1],
  ["Fcl_EYE_Joy", 1],
  ["mouthSmile", 1],
  ["mouthSmileLeft", 1],
  ["mouthSmileRight", 1],
  ["cheekSquintLeft", 0.5],
  ["cheekSquintRight", 0.5],
  ["joy", 1],
  ["喜び", 1],
  ["Mouth_Smile01", 1],
  ["Brow_Smily_L", 0.6],
  ["Brow_Smily_R", 0.6],
];

const ANGRY_NAMES: Array<[string, number]> = [
  ["Fcl_ALL_Angry", 1],
  ["Fcl_MTH_Angry", 1],
  ["Fcl_EYE_Angry", 1],
  ["browDownLeft", 1],
  ["browDownRight", 1],
  ["mouthFrownLeft", 0.8],
  ["mouthFrownRight", 0.8],
  ["angry", 1],
  ["怒り", 1],
  ["Mouth_Angry01", 1],
  ["Brow_Angry_L", 1],
  ["Brow_Angry_R", 1],
  ["Eye_Hostility", 0.7],
];

const SORROW_NAMES: Array<[string, number]> = [
  ["Fcl_ALL_Sorrow", 1],
  ["Fcl_MTH_Sorrow", 1],
  ["Fcl_EYE_Sorrow", 1],
  ["browInnerUp", 1],
  ["mouthFrownLeft", 0.6],
  ["mouthFrownRight", 0.6],
  ["sorrow", 1],
  ["悲しみ", 1],
  ["Brow_Trouble_L", 1],
  ["Brow_Trouble_R", 1],
  ["Eye_Wail", 0.8],
];

const FUN_NAMES: Array<[string, number]> = [
  ["Fcl_ALL_Fun", 1],
  ["Fcl_MTH_Fun", 1],
  ["Fcl_EYE_Fun", 1],
  ["mouthSmile", 0.8],
  ["eyeSquintLeft", 0.5],
  ["eyeSquintRight", 0.5],
  ["fun", 1],
  ["楽しい", 1],
  ["Mouth_Smile01", 0.8],
  ["Brow_Smily_L", 0.5],
  ["Brow_Smily_R", 0.5],
];

const SURPRISED_NAMES: Array<[string, number]> = [
  ["Fcl_ALL_Surprised", 1],
  ["Fcl_MTH_Surprised", 1],
  ["Fcl_EYE_Surprised", 1],
  ["browOuterUpLeft", 1],
  ["browOuterUpRight", 1],
  ["eyeWideLeft", 1],
  ["eyeWideRight", 1],
  ["jawOpen", 0.3],
  ["surprised", 1],
  ["驚き", 1],
  ["Brow_Up_L", 1],
  ["Brow_Up_R", 1],
  ["Mouth_A01", 0.35],
];

/**
 * 去掉 MMD 那种编号前缀。
 *
 * **这是新模型"口型不兼容"的真正原因。** 从 MMD 转过来的 VRM，morph target
 * 叫 `27.あ`、`28.い` —— 名字前面带一个序号，而且**每个模型的序号还不一样**
 * （同一批模型里 あ 有的是 25、有的是 27、有的是 29）。
 *
 * 原先是精确相等匹配，`27.あ` 永远对不上 `あ`，于是这类模型一个通道都认不出来。
 *
 * 只剥掉前缀、剥完还要**精确相等**，不是包含匹配：这批模型里同时存在
 * `28.い` 和 `39.い2`，`い2` 是另一个口型的变体。用包含匹配会把两个都吃进来，
 * 嘴型会同时往两个方向拉。
 */
function stripIndexPrefix(name: string): string {
  return name.replace(/^\s*\d+\s*[.．]\s*/, "");
}

function collect(
  names: Array<[string, number]>,
  dict: Record<string, number>,
  influences: number[],
  into: Target[],
) {
  for (const [name, gain] of names) {
    const index = dict[name];
    if (index !== undefined) into.push({ influences, index, gain });
  }
}

/** 遍历整棵场景树，认出所有能驱动的通道 */
export function mapMorphTargets(root: Object3D): MorphMap {
  const targets = new Map<Viseme, Target[]>();
  const blink: Target[] = [];
  const smile: Target[] = [];
  const joy: Target[] = [];
  const angry: Target[] = [];
  const sorrow: Target[] = [];
  const fun: Target[] = [];
  const surprised: Target[] = [];
  const found: string[] = [];
  const faceMeshes: Object3D[] = [];
  // 按**命中数**判定用的是哪套，不是"谁先匹配上"。
  // 因为命名之间有重叠：VRM 1.0 的元音预设就叫 `aa`，和 Oculus 的裸名字撞上 ——
  // 只看第一次匹配的话，一个 VRM 模型会被报成"原生 viseme"，
  // 用户据此以为精度很好，实际上是五元音近似
  const hits = { oculus: 0, cc: 0, arkit: 0, mmd: 0, vowel: 0 };

  root.traverse((obj) => {
    const mesh = obj as Mesh;
    const dict = mesh.morphTargetDictionary;
    const infl = mesh.morphTargetInfluences;
    if (!dict || !infl) return;
    found.push(...Object.keys(dict));

    // 在原名之外再放一份剥掉编号前缀的。原名优先 ——
    // 万一有模型的通道本来就叫 `27.あ` 这种名字，不该被剥出来的那份顶掉
    const lookup: Record<string, number> = { ...dict };
    for (const [name, index] of Object.entries(dict)) {
      const bare = stripIndexPrefix(name);
      if (bare !== name && lookup[bare] === undefined) lookup[bare] = index;
    }

    for (const v of VISEMES) {
      const bucket = targets.get(v) ?? [];
      const before = bucket.length;

      collect(
        [
          [`viseme_${v}`, 1],
          [v, 1],
        ],
        lookup,
        infl,
        bucket,
      );
      if (bucket.length > before) {
        hits.oculus++;
      } else {
        collect(
          CC[v].map((n) => [n, 1] as [string, number]),
          lookup,
          infl,
          bucket,
        );
        if (bucket.length > before) {
          hits.cc++;
        } else {
          collect(ARKIT[v], lookup, infl, bucket);
          if (bucket.length > before) {
            hits.arkit++;
          } else {
            collect(MMD_EN[v], lookup, infl, bucket);
            if (bucket.length > before) {
              hits.mmd++;
            } else {
              // 候选按优先级试，先命中的算数（鼻音优先用闭口形，没有才拿张口凑）
              for (const [vowel, gain] of VOWEL_APPROX[v]) {
                collect(
                  VOWEL_NAMES[vowel].map((n) => [n, gain] as [string, number]),
                  lookup,
                  infl,
                  bucket,
                );
                if (bucket.length > before) {
                  hits.vowel++;
                  break;
                }
              }
            }
          }
        }
      }
      if (bucket.length) targets.set(v, bucket);
    }

    collect(BLINK_NAMES, lookup, infl, blink);
    collect(SMILE_NAMES, lookup, infl, smile);
    collect(JOY_NAMES, lookup, infl, joy);
    collect(ANGRY_NAMES, lookup, infl, angry);
    collect(SORROW_NAMES, lookup, infl, sorrow);
    collect(FUN_NAMES, lookup, infl, fun);
    collect(SURPRISED_NAMES, lookup, infl, surprised);

    // 这个 mesh 有没有贡献口型通道 —— 有就是脸的一部分
    if (
      VISEMES.some((v) => targets.get(v)?.some((t) => t.influences === infl))
    ) {
      faceMeshes.push(obj);
    }
  });

  // 命中最多的那套算数，不是"谁先匹配上"。命名之间有重叠：
  // VRM 1.0 的元音预设就叫 `aa`，和 Oculus 的裸名字撞上 —— 按先后判定的话
  // 五元音模型会被报成"原生 viseme"，用户据此以为精度很好，实际辅音全是近似的
  const winner = (
    Object.entries(hits) as Array<[keyof typeof hits, number]>
  ).reduce((best, cur) => (cur[1] > best[1] ? cur : best));
  const scheme: MorphScheme = winner[1] === 0 ? "none" : winner[0];

  return {
    targets,
    blink,
    smile,
    joy,
    angry,
    sorrow,
    fun,
    surprised,
    scheme,
    found,
    faceMeshes,
  };
}
