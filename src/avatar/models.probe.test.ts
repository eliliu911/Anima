/**
 * 对着 `avatars/` 里**真实的模型文件**核对口型识别。
 *
 * ## 为什么这不是普通测试
 *
 * 别的测试用的是手写的假名字 —— 那只能验"代码做了我写的事"，验不了
 * 那些名字在真实模型里长什么样。实测吃过两次亏：
 *
 * - CC 的口型我编成了 `V_aa`，真实的是 `B_M_P` 那一套；
 * - MMD 转过来的 VRM 叫 `27.あ`，带一个**每个模型都不一样**的序号前缀，
 *   精确匹配一个都对不上。
 *
 * 两次都是测试全绿、真实模型全废。所以要有一条对着真文件跑的。
 *
 * ## 而这条测试自己也骗过我一次
 *
 * 第一版直接翻 glTF 的 JSON，mesh 层和 primitive 层的 `targetNames` 都翻，
 * 于是报告"七个模型全部认得"—— 而应用走的是 GLTFLoader，**它只读 mesh 那一层**，
 * 拿到的是一堆叫 `0`、`1`、`2` 的通道。测试绿着，用户那边一片空白。
 *
 * 所以下面只读 mesh 层，再单独把 primitive 层的名字按
 * `gltfNames.restoreMorphNames` 的规则补上 —— **和产品里的读法完全一致**。
 * 验真实文件的时候读法一旦和产品不同，测的就是另一条路径。
 *
 * `avatars/` 已 gitignore（模型是个人素材），所以目录空的时候这条自动跳过。
 *
 * 加了新模型之后跑一遍：`npm test`，看输出的通道数。
 */
import { describe, expect, it } from "vitest";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { Group, Mesh } from "three";
import { mapMorphTargets, SCHEME_LABEL } from "./morphMap";

const ROOT = join(process.cwd(), "avatars");

/**
 * 取出 morph target 的名字，**按 GLTFLoader + restoreMorphNames 的规则**：
 * 优先 mesh 层（three 唯一会读的地方），没有才用 primitive 层补。
 */
function morphNames(path: string): { names: string[]; from: string } {
  const buf = readFileSync(path);
  if (buf.subarray(0, 4).toString() !== "glTF") return { names: [], from: "非 GLB" };
  // GLB：12 字节头 + [4 字节长度 + 4 字节类型 + JSON]
  const length = buf.readUInt32LE(12);
  const json = JSON.parse(buf.subarray(20, 20 + length).toString("utf8"));

  const meshLevel: string[] = [];
  const primLevel: string[] = [];
  for (const mesh of json.meshes ?? []) {
    for (const n of mesh.extras?.targetNames ?? []) meshLevel.push(n);
    for (const prim of mesh.primitives ?? []) {
      for (const n of prim.extras?.targetNames ?? []) primLevel.push(n);
    }
  }
  if (meshLevel.length) return { names: [...new Set(meshLevel)], from: "mesh 层" };
  if (primLevel.length) {
    return { names: [...new Set(primLevel)], from: "primitive 层（补名）" };
  }
  return { names: [], from: "无名（要走 VRM 扩展段）" };
}

function models(): string[] {
  try {
    return readdirSync(ROOT, { recursive: true } as never)
      .map((f) => join(ROOT, f as string))
      .filter((f) => /\.(glb|vrm)$/i.test(f) && statSync(f).isFile());
  } catch {
    return [];
  }
}

const files = models();

describe.skipIf(files.length === 0)("真实模型的口型覆盖", () => {
  for (const file of files) {
    it(file.replace(ROOT + "/", ""), () => {
      const { names, from } = morphNames(file);
      const group = new Group();
      const mesh = new Mesh();
      mesh.morphTargetDictionary = Object.fromEntries(names.map((n, i) => [n, i]));
      mesh.morphTargetInfluences = new Array(names.length).fill(0);
      group.add(mesh);

      const map = mapMorphTargets(group);
      console.log(
        `    ${String(map.targets.size).padStart(2)}/15 通道  ` +
          `${SCHEME_LABEL[map.scheme]}  眨眼 ${map.blink.length}  笑 ${map.smile.length}  ` +
          `[名字来自 ${from}]`,
      );

      // 一个都认不出来就是真的不能用了 —— 那时候把 morph target 名字
      // 加进 morphMap.ts 的表里
      expect(
        map.targets.size,
        `认不出任何口型。这个模型的通道名是：${names.slice(0, 12).join("、")}`,
      ).toBeGreaterThan(0);
    });
  }
});
