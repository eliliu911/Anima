import type { CanvasEdge, CanvasNode } from "../types";

/**
 * 顺着连线往上游找第一个**说得出话**的节点。
 *
 * # 为什么是共用的一个
 *
 * 画布上「某个能力挂在上游某处，中间隔着几层」是常态：全景的场景归属、
 * 分镜的镜头归属，接法都是 `源 → 图片 → 图片 → 用的地方`。
 * 各写各的遍历就会各漏各的环保护 —— 第二份抄第一份时最容易漏掉的正是
 * 那两道防线，而漏了不报错，表现是整个画布冻死。
 *
 * `pick` 返回非 `null` 就停。**广度优先** —— 近的先说话，
 * 接了两个来源时取更近的那个才符合直觉。
 *
 * # 两道防线，都要，而且没有哪个测试能钉住其中任何一条
 *
 * `seen` 管正确性（环不重复走）；`budget` 管最坏情况（真转起来也会停）。
 *
 * 变异测试验过：**单独拿掉任何一条，行为都还是对的** —— 去掉 `seen`
 * 时上限兜住，去掉上限时 `seen` 兜住。所以这是刻意的纵深防御，
 * 不是「忘了测」。
 *
 * 为什么值得两条：去掉 `seen` 而没有上限时，这个函数不是返回错值而是
 * **挂住**，而它跑在界面线程上 —— 挂住等于整个画布冻死，还不报错。
 * 一条查不出的死循环比一个错误的返回值危险得多，而挂住的测试同样
 * 抓不到回归（它只会让整个套件停在那儿，那一点也是实测出来的）。
 */
export function findUpstream<T>(
  nodeId: string,
  nodes: CanvasNode[],
  edges: CanvasEdge[],
  pick: (node: CanvasNode) => T | null,
): T | null {
  const seen = new Set<string>([nodeId]);
  const queue = edges.filter((e) => e.target === nodeId).map((e) => e.source);
  let budget = nodes.length * 4 + 64;
  while (queue.length && budget-- > 0) {
    const id = queue.shift() as string;
    if (seen.has(id)) continue;
    seen.add(id);
    const n = nodes.find((x) => x.id === id);
    if (!n) continue;
    const hit = pick(n);
    if (hit !== null) return hit;
    for (const e of edges.filter((x) => x.target === id)) queue.push(e.source);
  }
  return null;
}
