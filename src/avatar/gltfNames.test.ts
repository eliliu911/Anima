/**
 * 补名字的测试。
 *
 * 这段代码存在的唯一原因就是 three.js 只读 mesh 层的 `targetNames`，
 * 而很多导出器写在 primitive 层。所以测的重点是：**该补的补、不该动的不动。**
 */
import { describe, expect, it } from "vitest";
import { Group, Mesh, Vector3 } from "three";
import type { GLTF } from "three/examples/jsm/loaders/GLTFLoader.js";
import { faceCamera, restoreMorphNames } from "./gltfNames";

/** 拼一个最小的 GLTF —— 只需要 scene / parser.json / parser.associations */
/** 只带扩展段的最小 GLTF —— faceCamera 只看这个 */
function gltfWithExtensions(extensions: Record<string, unknown>): GLTF {
  const scene = new Group();
  return { scene, parser: { json: { extensions } } } as unknown as GLTF;
}

function fakeGltf(opts: {
  dict: Record<string, number>;
  primitiveNames?: string[];
  meshIndex?: number;
}): { gltf: GLTF; mesh: Mesh } {
  const mesh = new Mesh();
  mesh.morphTargetDictionary = opts.dict;
  mesh.morphTargetInfluences = new Array(Object.keys(opts.dict).length).fill(0);
  const scene = new Group();
  scene.add(mesh);

  const associations = new Map<object, { meshes?: number; primitives?: number }>();
  associations.set(mesh, { meshes: opts.meshIndex ?? 0, primitives: 0 });

  const gltf = {
    scene,
    parser: {
      json: {
        meshes: [
          {
            primitives: [
              opts.primitiveNames
                ? { extras: { targetNames: opts.primitiveNames } }
                : {},
            ],
          },
        ],
      },
      associations,
    },
  } as unknown as GLTF;
  return { gltf, mesh };
}

describe("从 primitive 层补名字", () => {
  it("名字全是数字时补回来", () => {
    // 用户那五个 MMD 转来的 VRM 就是这样：文件里写着 27.あ，
    // 加载完只剩叫 "0"、"1"、"2" 的通道
    const { gltf, mesh } = fakeGltf({
      dict: { "0": 0, "1": 1, "2": 2 },
      primitiveNames: ["27.あ", "28.い", "29.う"],
    });
    expect(restoreMorphNames(gltf)).toBe(1);
    expect(Object.keys(mesh.morphTargetDictionary!)).toEqual([
      "27.あ",
      "28.い",
      "29.う",
    ]);
  });

  it("下标要对得上，不能只是换个名字", () => {
    // 顺序错了会驱动错误的通道 —— 嘴会动，但动的是别的表情
    const { gltf, mesh } = fakeGltf({
      dict: { "0": 0, "1": 1, "2": 2 },
      primitiveNames: ["あ", "い", "う"],
    });
    restoreMorphNames(gltf);
    expect(mesh.morphTargetDictionary!["い"]).toBe(1);
  });

  it("已经有正经名字的不动", () => {
    // 覆盖掉正确的名字是这段代码唯一能造成的破坏
    const dict = { viseme_aa: 0, viseme_PP: 1 };
    const { gltf, mesh } = fakeGltf({ dict, primitiveNames: ["x", "y"] });
    expect(restoreMorphNames(gltf)).toBe(0);
    expect(mesh.morphTargetDictionary).toBe(dict);
  });

  it("primitive 层也没有名字时安静地什么都不做", () => {
    // 那种模型的语义在 VRM 扩展段里，是另一条路
    const { gltf, mesh } = fakeGltf({ dict: { "0": 0, "1": 1 } });
    expect(restoreMorphNames(gltf)).toBe(0);
    expect(Object.keys(mesh.morphTargetDictionary!)).toEqual(["0", "1"]);
  });

  it("重名的保留第一个", () => {
    const { gltf, mesh } = fakeGltf({
      dict: { "0": 0, "1": 1, "2": 2 },
      primitiveNames: ["あ", "あ", "い"],
    });
    restoreMorphNames(gltf);
    expect(mesh.morphTargetDictionary!["あ"]).toBe(0);
  });

  it("缺 parser / associations 不炸", () => {
    const scene = new Group();
    expect(restoreMorphNames({ scene } as unknown as GLTF)).toBe(0);
  });
});

describe("VRM 0.x 的朝向", () => {
  it("0.x 要转 180 度，否则背对着你", () => {
    // 这是规格差别不是模型做错了：0.x 朝 -Z，1.0 朝 +Z。
    // 用 GLTFLoader 裸加载时没人替我们处理，表现就是"模型背对着你"
    const gltf = gltfWithExtensions({ VRM: {} });
    expect(faceCamera(gltf)).toBe(true);
    // 断言**效果**而不是 rotation.y —— 180° 的四元数分解成 Euler 有多种
    // 等价写法（这里实际落在 x=π, z=π 上，y 是 0），断言分量会误判成没转
    const forward = new Vector3(0, 0, 1).applyQuaternion(gltf.scene.quaternion);
    expect(forward.z).toBeCloseTo(-1, 5);
  });

  it("1.0 不能转 —— 转了就反而背过去了", () => {
    const gltf = gltfWithExtensions({ VRMC_vrm: {} });
    expect(faceCamera(gltf)).toBe(false);
    const forward = new Vector3(0, 0, 1).applyQuaternion(gltf.scene.quaternion);
    expect(forward.z).toBeCloseTo(1, 5);
  });

  it("两段都在时以 1.0 为准", () => {
    // 有模型为兼容会同时写两段扩展
    const gltf = gltfWithExtensions({ VRM: {}, VRMC_vrm: {} });
    expect(faceCamera(gltf)).toBe(false);
  });

  it("不是 VRM 的 glTF 不动", () => {
    expect(faceCamera(gltfWithExtensions({}))).toBe(false);
    const bare = { scene: new Group() } as unknown as GLTF;
    expect(faceCamera(bare)).toBe(false);
  });
});
