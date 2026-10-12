/**
 * 顶点色的测试。
 *
 * 守的是一个**完全静默**的失败：这条通道退化时，alpha 测试会丢掉每一个片元
 * （模型完全不显示），或者 RGB 相乘把整个人染成一片黄绿。两种都不报错。
 * 靠看是查不出来的，只能靠这条。
 */
import { describe, expect, it } from "vitest";
import { BufferAttribute, BufferGeometry, Group, Mesh, MeshBasicMaterial } from "three";
import { disableDegenerateVertexColors } from "./vertexColors";

function meshWithColor(data: number[], itemSize: number, normalized = false): Mesh {
  const geometry = new BufferGeometry();
  const array = normalized ? new Uint8Array(data) : new Float32Array(data);
  geometry.setAttribute("color", new BufferAttribute(array, itemSize, normalized));
  const material = new MeshBasicMaterial();
  material.vertexColors = true;
  return new Mesh(geometry, material);
}

function rooted(mesh: Mesh): Group {
  const g = new Group();
  g.add(mesh);
  return g;
}

const vc = (mesh: Mesh) => (mesh.material as MeshBasicMaterial).vertexColors;

describe("退化的顶点色", () => {
  it("alpha 整条为 0 时关掉顶点色", () => {
    // 实测：用户七个模型里有这条属性的五个全都不显示，没有的两个正常。
    // 把 alpha 补成 1 之后能显示了，但整个人是一片黄绿 ——
    // RGB 也是垃圾数据，所以要整条忽略
    const mesh = meshWithColor([1, 1, 1, 0, 1, 1, 1, 0], 4);
    expect(disableDegenerateVertexColors(rooted(mesh))).toBe(1);
    expect(vc(mesh)).toBe(false);
  });

  it("真在用的顶点色不能关", () => {
    // 有些模型确实靠顶点色做局部染色。关了会破坏作者的意图
    const mesh = meshWithColor([1, 1, 1, 0, 1, 1, 1, 1], 4);
    expect(disableDegenerateVertexColors(rooted(mesh))).toBe(0);
    expect(vc(mesh)).toBe(true);
  });

  it("三分量的顶点色不动 —— 没有 alpha，判据不成立", () => {
    const mesh = meshWithColor([1, 1, 1], 3);
    expect(disableDegenerateVertexColors(rooted(mesh))).toBe(0);
    expect(vc(mesh)).toBe(true);
  });

  it("没有顶点色属性的网格不受影响", () => {
    const mesh = new Mesh(new BufferGeometry(), new MeshBasicMaterial());
    expect(disableDegenerateVertexColors(rooted(mesh))).toBe(0);
  });

  it("归一化的整型属性也要读对", () => {
    // glTF 里顶点色常是 normalized 的 ubyte/ushort
    const mesh = meshWithColor([255, 255, 255, 0], 4, true);
    expect(disableDegenerateVertexColors(rooted(mesh))).toBe(1);
    expect(vc(mesh)).toBe(false);
  });

  it("多材质的网格逐个处理", () => {
    const mesh = meshWithColor([1, 1, 1, 0], 4);
    const second = new MeshBasicMaterial();
    second.vertexColors = true;
    mesh.material = [mesh.material as MeshBasicMaterial, second];
    disableDegenerateVertexColors(rooted(mesh));
    expect(second.vertexColors).toBe(false);
  });
});
