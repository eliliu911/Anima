/**
 * 内置头的测试。
 *
 * 它的职责有两条，两条都要验：
 *
 * 1. 手上没有任何 .glb 时也能看见嘴在动（RPM 关停之后这是唯一的开箱可用路径）。
 * 2. 当一份"这 15 个通道分别长什么样"的参照。
 *
 * 第 2 条要求各个口型**互相看得出区别** —— 如果 U 和 O 长得一样，
 * 那它当参照就是误导的，而且没有任何办法从画面上确认这件事。
 */
import { describe, expect, it } from "vitest";
import type { BufferAttribute, Mesh } from "three";
import { buildDefaultHead } from "./defaultHead";
import { VISEMES } from "./mouth";

/** morph 位移数组。类型上是可选的，但对内置头它一定在 —— 拿不到就该炸 */
function morphs(mesh: Mesh): BufferAttribute[] {
  const list = mesh.geometry.morphAttributes.position;
  expect(list, "内置头的嘴必须带 morph").toBeTruthy();
  return list as BufferAttribute[];
}

function mouthMesh(): Mesh {
  const head = buildDefaultHead();
  const mesh = head.getObjectByName("BuiltinMouth") as Mesh;
  expect(mesh, "内置头里应当有一张嘴").toBeTruthy();
  return mesh;
}

/** 某个口型相对闭嘴的总位移量 */
function displacement(mesh: Mesh, index: number): number {
  const attr = morphs(mesh)[index];
  let sum = 0;
  for (let i = 0; i < attr.array.length; i++) sum += Math.abs(attr.array[i]);
  return sum;
}

/**
 * 某个口型的开口高度（上下唇之间）。
 *
 * 不能拿"总位移"当开口大小 —— 撮口音是**横向大幅收窄**的，总位移里横向那部分
 * 很大，结果 U 的总位移比 aa 还大。第一版就是这么写错的。
 */
function opening(mesh: Mesh, index: number): number {
  const base = mesh.geometry.attributes.position as BufferAttribute;
  const delta = morphs(mesh)[index];
  let maxY = 0;
  for (let i = 1; i < base.count; i++) {
    maxY = Math.max(maxY, Math.abs(base.getY(i) + delta.getY(i)));
  }
  return maxY;
}

/** 两个口型之间差多少 */
function distance(mesh: Mesh, a: number, b: number): number {
  const A = morphs(mesh)[a];
  const B = morphs(mesh)[b];
  let sum = 0;
  for (let i = 0; i < A.array.length; i++) {
    sum += Math.abs(A.array[i] - B.array[i]);
  }
  return sum;
}

describe("内置头", () => {
  it("嘴带齐 15 个 viseme morph，名字就是线上传的那套", () => {
    // 名字写错了不会报错，只会"嘴不动" —— 而且 Rust 那边用的是 serde 名
    // （`PP`/`aa`），Debug 名（`Pp`/`Aa`）是另一套，很容易拿错
    const dict = mouthMesh().morphTargetDictionary!;
    expect(Object.keys(dict).sort()).toEqual(
      VISEMES.map((v) => `viseme_${v}`).sort(),
    );
  });

  it("morph 是相对位移", () => {
    // 写成绝对坐标而 morphTargetsRelative 又是 false 的话，
    // 权重一给嘴就会飞到原点去
    expect(mouthMesh().geometry.morphTargetsRelative).toBe(true);
  });

  it("闭嘴那一档不动，其余都动", () => {
    const mesh = mouthMesh();
    const dict = mesh.morphTargetDictionary!;
    expect(displacement(mesh, dict["viseme_sil"])).toBe(0);
    for (const v of VISEMES) {
      if (v === "sil") continue;
      expect(displacement(mesh, dict[`viseme_${v}`]), `${v} 应当有位移`).toBeGreaterThan(0);
    }
  });

  it("元音之间互相看得出区别", () => {
    // 五个元音是"看起来像在说话"的主要承担者。它们要是彼此接近，
    // 整句就只剩"嘴在开合"
    const mesh = mouthMesh();
    const dict = mesh.morphTargetDictionary!;
    const vowels = ["aa", "E", "I", "O", "U"];
    for (let i = 0; i < vowels.length; i++) {
      for (let j = i + 1; j < vowels.length; j++) {
        const d = distance(mesh, dict[`viseme_${vowels[i]}`], dict[`viseme_${vowels[j]}`]);
        expect(d, `${vowels[i]} 和 ${vowels[j]} 太像了（差 ${d.toFixed(3)}）`).toBeGreaterThan(0.3);
      }
    }
  });

  it("撮口的 U 比张口的 aa 更前突、开口更小", () => {
    // 这两条是 U 之所以是 U 的全部内容。只按开口大小做的话 U 会变成小号的 aa
    const mesh = mouthMesh();
    const dict = mesh.morphTargetDictionary!;
    const zOf = (name: string) => {
      const attr = morphs(mesh)[dict[name]];
      // 中心点的 z 位移
      return attr.array[2];
    };
    expect(zOf("viseme_U")).toBeGreaterThan(zOf("viseme_aa"));
    expect(opening(mesh, dict["viseme_U"])).toBeLessThan(
      opening(mesh, dict["viseme_aa"]) * 0.6,
    );
  });

  it("双唇音几乎是闭着的", () => {
    // 说「妈」时嘴不闭是一眼就能看出来的假
    const mesh = mouthMesh();
    const dict = mesh.morphTargetDictionary!;
    expect(opening(mesh, dict["viseme_PP"])).toBeLessThan(
      opening(mesh, dict["viseme_aa"]) * 0.1,
    );
  });

  it("眼睛能闭", () => {
    const head = buildDefaultHead();
    for (const name of ["BuiltinEyeLeft", "BuiltinEyeRight"]) {
      const eye = head.getObjectByName(name) as Mesh;
      expect(eye, `应当有 ${name}`).toBeTruthy();
      expect(eye.morphTargetDictionary).toHaveProperty("eyesClosed");
    }
  });
});
