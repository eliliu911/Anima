/**
 * 把 glTF 里丢掉的 morph target 名字补回来。
 *
 * ## 问题
 *
 * morph target 的名字在 glTF 里有**两个**可能的位置：
 *
 * ```jsonc
 * "meshes": [{
 *   "extras": { "targetNames": [...] },        // ← three.js 只读这里
 *   "primitives": [{
 *     "extras": { "targetNames": [...] }       // ← 很多导出器写在这里
 *   }]
 * }]
 * ```
 *
 * **three.js 的 GLTFLoader 只读 mesh 那一层。** 读不到就按下标编号，
 * 于是 `morphTargetDictionary` 变成 `{ "0": 0, "1": 1, ... }` —— 名字全没了，
 * 任何按名字做的匹配都是瞎的。
 *
 * 实测：用户导入的五个从 MMD 转过来的 VRM 全是这样，文件里明明写着
 * `27.あ`、`28.い`，加载完却只剩 880 个叫 `0`、`1`、`2` 的通道。
 *
 * ## 这件事怎么骗过测试的
 *
 * 我写的"对着真实模型核对"那条测试是**直接翻 glTF 的 JSON**，两个位置都翻了，
 * 所以它报告"七个模型全部认得"。而应用走的是 GLTFLoader，只拿得到一半。
 *
 * 教训是：验真实文件的时候，**读法必须和产品里的读法一致**，
 * 否则测的是另一条路径。
 */
import type { GLTF } from "three/examples/jsm/loaders/GLTFLoader.js";
import type { Mesh, Object3D } from "three";

/** 字典里的名字是不是全是纯数字 —— 那就是 three 编出来的占位名 */
function isUnnamed(dict: Record<string, number>): boolean {
  const keys = Object.keys(dict);
  return keys.length > 0 && keys.every((k) => /^\d+$/.test(k));
}

/**
 * 从 primitive 层把名字补回 `morphTargetDictionary`。
 *
 * 只在名字全是数字时才动手 —— 已经有正经名字的模型不该被覆盖。
 *
 * @returns 补好了几个 mesh。0 表示这个模型的名字真的不在文件里
 *          （那种要走 VRM 扩展段，见模块注释）
 */
export function restoreMorphNames(gltf: GLTF): number {
  const json = gltf.parser?.json as
    | {
        meshes?: Array<{
          primitives?: Array<{ extras?: { targetNames?: unknown } }>;
        }>;
      }
    | undefined;
  const associations = gltf.parser?.associations as
    | Map<Object3D, { meshes?: number; primitives?: number }>
    | undefined;
  if (!json?.meshes || !associations) return 0;

  let fixed = 0;
  gltf.scene.traverse((obj) => {
    const mesh = obj as Mesh;
    const dict = mesh.morphTargetDictionary;
    if (!dict || !isUnnamed(dict)) return;

    const assoc = associations.get(obj);
    if (assoc?.meshes === undefined) return;
    const names =
      json.meshes?.[assoc.meshes]?.primitives?.[assoc.primitives ?? 0]?.extras
        ?.targetNames;
    if (!Array.isArray(names)) return;

    const next: Record<string, number> = {};
    names.forEach((name, i) => {
      // 同名的 morph 保留第一个 —— 后面的多半是导出器的重复条目
      if (typeof name === "string" && name && next[name] === undefined) {
        next[name] = i;
      }
    });
    if (Object.keys(next).length === 0) return;

    mesh.morphTargetDictionary = next;
    fixed++;
  });
  return fixed;
}

/**
 * VRM 0.x 的模型**朝向 -Z**（背对相机），1.0 才是 +Z。
 *
 * 这是规格上的差别，不是模型做错了。用 GLTFLoader 裸加载时没人替我们处理它 ——
 * 表现是"模型背对着你"。@pixiv/three-vrm 也是这么干的：0.x 就把根节点转 180°。
 *
 * @returns 是不是转过了
 */
export function faceCamera(gltf: GLTF): boolean {
  const ext = (gltf.parser?.json as { extensions?: Record<string, unknown> } | undefined)
    ?.extensions;
  // 两个都在的话以 1.0 为准（有模型为兼容会同时写两段）
  const isVrm0 = !!ext && "VRM" in ext && !("VRMC_vrm" in ext);
  if (isVrm0) gltf.scene.rotateY(Math.PI);
  return isVrm0;
}
