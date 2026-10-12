/**
 * 忽略那条没被真正使用的顶点色属性。
 *
 * ## 这是"模型完全不显示 / 颜色乱掉"的真凶
 *
 * 从 MMD / FBX 转出来的 VRM 常常带一条 `COLOR_0` 顶点色属性、四个分量，
 * 而里面**整条都是垃圾数据**：alpha 全 0，RGB 也不是有意义的颜色。
 * 那个通道在原始管线里本来没被用到。
 *
 * 只要 glTF 里有 `COLOR_0`，GLTFLoader 就打开 `vertexColors`，
 * 于是 three 的着色器里
 *
 *     最终颜色 = 贴图 × 顶点色（含 alpha）
 *
 * 两种后果，实测都撞到了：
 *
 * 1. **完全不显示。** alpha 恒为 0，而这些模型的材质都是 `alphaMode: MASK`
 *    （alpha 测试，阈值 0.5）—— 每一个片元都被丢掉。模型加载成功、网格在场景里、
 *    骨骼正常、包围盒正常、控制台一句报错都没有，画面上什么都没有。
 * 2. **颜色乱掉。** 把 alpha 补成 1 之后模型出来了，但 RGB 仍然在相乘。
 *    实测 Anaxa 的画面平均色是 RGB(109,74,0) —— 蓝通道整个是 0，
 *    整个人是一片黄绿。忽略这条通道之后是 RGB(149,160,165)。
 *
 * ## 为什么可以直接忽略
 *
 * VRM 的 MToon 规格**不使用顶点色**。这些模型在正规 VRM 查看器里颜色正常，
 * 正是因为那些实现走 MToon、根本不看这条通道。我们用 glTF 的回退材质渲染，
 * 才会踩到它。
 *
 * ## 判据
 *
 * 用 alpha 是不是整条都低于阈值来判断"这条通道没被真正使用"。
 * 只要有一个顶点是不透明的，就说明作者真的在用它，那就不动 ——
 * 有些模型确实靠顶点色做局部染色。
 */
import type { Object3D, BufferAttribute, InterleavedBufferAttribute, Material, Mesh } from "three";

/** alpha 的最大值低于这个就算"这条通道没被用过" */
const DEGENERATE = 0.5;

/**
 * 关掉退化的顶点色。
 *
 * @returns 处理了几个网格
 */
export function disableDegenerateVertexColors(root: Object3D): number {
  let fixed = 0;
  root.traverse((obj) => {
    const mesh = obj as Mesh;
    if (!mesh.isMesh) return;
    const color = mesh.geometry?.getAttribute("color") as
      | BufferAttribute
      | InterleavedBufferAttribute
      | undefined;
    // 三分量的顶点色没有 alpha，判据不成立，保守起见不动
    if (!color || color.itemSize < 4) return;

    for (let i = 0; i < color.count; i++) {
      // 有一个点不透明就说明这条通道真在用，整个模型都别动
      if (color.getW(i) >= DEGENERATE) return;
    }

    const materials: Material[] = Array.isArray(mesh.material)
      ? mesh.material
      : [mesh.material];
    for (const material of materials) {
      // 材质可能被多个网格共享。共享的那些要么同样退化、要么本来就没有
      // 顶点色属性（关掉对它们无影响），所以直接关是安全的
      if ((material as Material & { vertexColors?: boolean }).vertexColors) {
        (material as Material & { vertexColors: boolean }).vertexColors = false;
        material.needsUpdate = true;
      }
    }
    fixed++;
  });
  return fixed;
}
