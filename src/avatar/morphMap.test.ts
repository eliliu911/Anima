/**
 * 自动映射的测试。
 *
 * 这里守的是整条链路里**最没有报错的一环**：morph target 的名字对不上时，
 * 模型照样加载、照样显示、就是嘴不动，Three.js 一个错都不报。
 *
 * 所以每套命名都要有一个"假模型"来验。假模型只造 `morphTargetDictionary`
 * 和 `morphTargetInfluences` —— 匹配逻辑只看这两样，不需要真的几何体。
 */
import { describe, expect, it } from "vitest";
import { Group, Mesh } from "three";
import { mapMorphTargets } from "./morphMap";
import { buildDefaultHead } from "./defaultHead";
import { VISEMES } from "./mouth";

/** 造一个只有 morph target 名字的假 mesh */
function fakeMesh(names: string[]): Mesh {
  const mesh = new Mesh();
  mesh.morphTargetDictionary = Object.fromEntries(names.map((n, i) => [n, i]));
  mesh.morphTargetInfluences = new Array(names.length).fill(0);
  return mesh;
}

function sceneWith(names: string[]): Group {
  const g = new Group();
  g.add(fakeMesh(names));
  return g;
}

describe("原生 viseme（Oculus / RPM 导出的那套）", () => {
  it("15 个通道全认得，判定为原生", () => {
    const root = sceneWith(VISEMES.map((v) => `viseme_${v}`));
    const map = mapMorphTargets(root);
    expect(map.scheme).toBe("oculus");
    expect(map.targets.size).toBe(15);
  });

  it("不带 viseme_ 前缀的裸名字也认", () => {
    const map = mapMorphTargets(sceneWith([...VISEMES]));
    expect(map.targets.size).toBe(15);
  });

});

describe("Character Creator / iClone 的音素集", () => {
  // **这组名字是查证过的。** 之前这里写的是 `V_aa`、`V_PP` —— 我按 Oculus
  // 那套推出来的，CC 里根本没有。测试全绿，因为它只验了代码做没做我写的事，
  // 验不了名字是不是真的。从 CC 导出的模型当时会一个通道都认不出来
  const CC_NAMES = [
    "AE", "Ah", "B_M_P", "Ch_J", "EE", "Er", "F_V", "Ih",
    "K_G_H_NG", "Oh", "R", "S_Z", "T_L_D_N", "Th", "W_OO",
  ];

  it("15 个音素通道全认得", () => {
    const map = mapMorphTargets(sceneWith(CC_NAMES));
    expect(map.scheme).toBe("cc");
    // sil 没有对应通道（闭嘴 = 全归零），所以是 14
    expect(map.targets.size).toBe(14);
  });

  it("双唇音落在 B_M_P 上", () => {
    // 这一条错了最显眼：说「妈」的时候嘴不闭，一眼就看出来
    const map = mapMorphTargets(sceneWith(CC_NAMES));
    expect(map.targets.get("PP")).toHaveLength(1);
  });

  it("不再认那个我编出来的 V_ 前缀", () => {
    // 留着这条是为了防止它被"顺手"加回来
    const map = mapMorphTargets(sceneWith(VISEMES.map((v) => `V_${v}`)));
    expect(map.scheme).toBe("none");
    expect(map.targets.size).toBe(0);
  });
});

describe("ARKit 降级", () => {
  it("只有 ARKit blendshape 时能拼出口型", () => {
    const map = mapMorphTargets(
      sceneWith([
        "jawOpen",
        "mouthClose",
        "mouthPucker",
        "mouthFunnel",
        "mouthSmileLeft",
        "mouthSmileRight",
        "mouthStretchLeft",
        "mouthStretchRight",
        "mouthShrugUpper",
        "eyeBlinkLeft",
        "eyeBlinkRight",
      ]),
    );
    expect(map.scheme).toBe("arkit");
    // sil 没有对应通道（闭嘴 = 全归零），所以是 14 而不是 15
    expect(map.targets.size).toBe(14);
    expect(map.blink.length).toBe(2);
  });

  it("同时带原生和 ARKit 时用原生", () => {
    // RPM 导出就是两套都带。用错的话精度会白白降一级，而且**看不出来**
    const map = mapMorphTargets(
      sceneWith([...VISEMES.map((v) => `viseme_${v}`), "jawOpen", "mouthPucker"]),
    );
    expect(map.scheme).toBe("oculus");
    const aa = map.targets.get("aa")!;
    expect(aa.every((t) => t.gain === 1)).toBe(true);
  });
});

describe("五元音（VRM / VRoid）", () => {
  // 这一套在 2026 年 1 月 RPM 关停之后成了最主要的来源 ——
  // VRoid Studio 还活着而且免费，它导出的就是这套命名
  it("VRoid 的 Fcl_MTH_* 能认出来", () => {
    const map = mapMorphTargets(
      sceneWith([
        "Fcl_MTH_A",
        "Fcl_MTH_I",
        "Fcl_MTH_U",
        "Fcl_MTH_E",
        "Fcl_MTH_O",
        "Fcl_EYE_Close",
      ]),
    );
    expect(map.scheme).toBe("vowel");
    // 五个元音 + 用元音近似出来的辅音
    expect(map.targets.size).toBeGreaterThanOrEqual(12);
    expect(map.blink.length).toBe(1);
  });

  it("VRM 1.0 的预设名（aa/ih/ou/ee/oh）判定成五元音，不冒充原生", () => {
    // **这条不是凑数的。** VRM 1.0 的 `aa` 和 Oculus 的裸名字 `aa` 撞名，
    // 按"谁先匹配上"来判定的话，一个五元音模型会被报成"原生 viseme"，
    // 用户据此以为精度很好 —— 而实际上辅音全是近似的
    const map = mapMorphTargets(sceneWith(["aa", "ih", "ou", "ee", "oh"]));
    expect(map.scheme).toBe("vowel");
  });

  it("双唇音不去凑一个张口形", () => {
    // 五元音里没有"闭嘴"。硬拿一个小张口去凑，说「妈」的时候嘴就闭不上 ——
    // 而双唇音是所有口型里最显眼的一个
    const map = mapMorphTargets(
      sceneWith(["Fcl_MTH_A", "Fcl_MTH_I", "Fcl_MTH_U", "Fcl_MTH_E", "Fcl_MTH_O"]),
    );
    expect(map.targets.get("PP")).toBeUndefined();
  });

  it("辅音的近似幅度明显小于元音", () => {
    // 辅音用元音去凑，幅度必须压住。给满的话整句会看起来在夸张地张嘴
    const map = mapMorphTargets(
      sceneWith(["Fcl_MTH_A", "Fcl_MTH_I", "Fcl_MTH_U", "Fcl_MTH_E", "Fcl_MTH_O"]),
    );
    const vowel = map.targets.get("aa")![0].gain;
    const consonant = map.targets.get("nn")![0].gain;
    expect(consonant).toBeLessThan(vowel * 0.6);
  });
});

describe("MMD 转过来的 VRM（带编号前缀）", () => {
  // 用户实际导入的五个模型就是这种。名字里带一个序号，
  // **而且每个模型的序号还不一样**（あ 在不同模型里是 25 / 27 / 29）。
  // 原先精确相等匹配，这类模型一个通道都认不出来 —— 这就是"口型不兼容"
  const MMD = [
    "20.まばたき", "23.笑い", "26.笑い2",
    "27.あ", "28.い", "29.う", "30.え", "31.お", "32.ん",
    "39.い2", "40.え2", "43.お2",
  ];

  it("剥掉编号前缀之后认得出来", () => {
    const map = mapMorphTargets(sceneWith(MMD));
    expect(map.scheme).toBe("vowel");
    expect(map.targets.size).toBeGreaterThanOrEqual(13);
  });

  it("序号变了照样认 —— 同一批模型的序号并不一致", () => {
    const shifted = MMD.map((n) => n.replace(/^\d+/, (d) => String(+d + 7)));
    const map = mapMorphTargets(sceneWith(shifted));
    expect(map.targets.size).toBeGreaterThanOrEqual(13);
  });

  it("不会把 `い2` 当成 `い`", () => {
    // 剥完前缀必须**精确相等**，不能用包含匹配。
    // `28.い` 和 `39.い2` 同时存在，吃进两个的话嘴型会同时往两个方向拉
    const map = mapMorphTargets(sceneWith(MMD));
    expect(map.targets.get("I")).toHaveLength(1);
  });

  it("闭口的「ん」用来做双唇音，不再靠归零凑", () => {
    // 五元音本来没有闭嘴形，说「妈」只能靠所有通道归零 ——
    // 而归零和"抿住嘴"看起来不一样
    const map = mapMorphTargets(sceneWith(MMD));
    expect(map.targets.get("PP")).toHaveLength(1);
  });

  it("眨眼和笑也认得（MMD 的日文名）", () => {
    const map = mapMorphTargets(sceneWith(MMD));
    expect(map.blink.length).toBeGreaterThan(0);
    expect(map.smile.length).toBeGreaterThan(0);
  });
});

describe("认不出来的时候", () => {
  it("完全没有 morph target：通道 0、名单为空", () => {
    const map = mapMorphTargets(new Group());
    expect(map.scheme).toBe("none");
    expect(map.targets.size).toBe(0);
    expect(map.found).toEqual([]);
  });

  it("有 morph target 但命名不认识：通道 0，但名单要留下来", () => {
    // 这两种情况的修法完全不同（一个是重新导出，一个是加映射），
    // 而界面上只能靠 found 区分
    const map = mapMorphTargets(sceneWith(["Key1", "Key2", "MouthThing"]));
    expect(map.scheme).toBe("none");
    expect(map.targets.size).toBe(0);
    expect(map.found).toEqual(["Key1", "Key2", "MouthThing"]);
  });
});

describe("内置头", () => {
  // 内置头存在的意义之一就是"不依赖任何外部素材也能验完这条链路"。
  // 那它自己就必须能被同一套匹配逻辑认出来 —— 否则它验的是另一条路
  it("走同一套匹配逻辑，15 个通道全认得", () => {
    const map = mapMorphTargets(buildDefaultHead());
    expect(map.scheme).toBe("oculus");
    expect(map.targets.size).toBe(15);
    expect(map.blink.length).toBe(2);
  });

  it("每个口型都有独立的 morph 落点", () => {
    const map = mapMorphTargets(buildDefaultHead());
    const indices = VISEMES.map((v) => map.targets.get(v)![0].index);
    expect(new Set(indices).size).toBe(15);
  });
});

describe("MMD 英文表情集（Mouth_A01 / Eye_WinkA_L 这一套）", () => {
  // 真实模型 Yae miko.glb 里的一部分名字
  const NAMES = [
    "Mouth_Default", "Mouth_A01", "Mouth_Open01", "Mouth_Smile01",
    "Mouth_Angry01", "Mouth_Line01", "Mouth_Line02",
    "Eye_WinkA_L", "Eye_WinkA_R", "Eye_Wail", "Eye_Hostility",
    "Brow_Smily_L", "Brow_Smily_R", "Brow_Angry_L", "Brow_Angry_R",
    "Brow_Trouble_L", "Brow_Trouble_R", "Brow_Up_L", "Brow_Up_R",
  ];

  it("认得出来，而不是通道 0", () => {
    const map = mapMorphTargets(sceneWith(NAMES));
    expect(map.scheme).toBe("mmd");
    expect(map.targets.size).toBeGreaterThan(10);
  });

  it("眨眼和表情也接上了", () => {
    const map = mapMorphTargets(sceneWith(NAMES));
    expect(map.blink.length).toBe(2);
    expect(map.smile.length).toBeGreaterThan(0);
    expect(map.angry.length).toBeGreaterThan(0);
    expect(map.sorrow.length).toBeGreaterThan(0);
    expect(map.surprised.length).toBeGreaterThan(0);
  });

  /// **`Mouth_Open01` 名不副实**：实测只有 31 个顶点在动、下张 7.7，
  /// 而 `Mouth_A01` 是 567 个点、下张 61.1。按名字猜的话会把它当张口形，
  /// 结果是"嘴基本不动"——又一次不报错的失败
  it("张口用的是 A01，不是名字唬人的 Open01", () => {
    const map = mapMorphTargets(sceneWith(NAMES));
    const aa = map.targets.get("aa")!;
    const openIdx = NAMES.indexOf("Mouth_Open01");
    const a01Idx = NAMES.indexOf("Mouth_A01");
    expect(aa.map((t) => t.index)).toContain(a01Idx);
    expect(aa.map((t) => t.index)).not.toContain(openIdx);
  });

  /// 这套只有一个元音形，O/U 只能是 A01 的不同幅度。
  /// **幅度必须真的不同** —— 都给 1 的话每个元音长得一模一样，
  /// 看着就像在机械开合
  it("开口度按元音分档，不是一个幅度打天下", () => {
    const map = mapMorphTargets(sceneWith(NAMES));
    const gain = (v: "aa" | "O" | "U") => map.targets.get(v)![0].gain;
    expect(gain("aa")).toBeGreaterThan(gain("O"));
    expect(gain("O")).toBeGreaterThan(gain("U"));
  });

  /// 双唇音要抿住。这套难得有真正的闭唇形（Line01），
  /// 用它比"所有通道归零"准
  it("闭嘴用的是抿唇形，不是靠归零", () => {
    const map = mapMorphTargets(sceneWith(NAMES));
    expect(map.targets.get("PP")!.map((t) => t.index))
      .toContain(NAMES.indexOf("Mouth_Line01"));
  });
});
