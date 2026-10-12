/**
 * 右侧资源管理器那棵树的纯逻辑。
 *
 * # 为什么要按需再取
 *
 * 后端一次只往下走 4 层（见 `workspace.rs` 的 `DEFAULT_TREE_DEPTH`）——
 * 一次全展开 `novels/` 这种目录要遍历上千个文件，而用户只想看其中一个。
 *
 * 代价是超出那 4 层的目录回来时 `children` 是 `null`。**而这和「真的是空
 * 目录」在界面上长得一模一样** —— 都是展开之后什么都没有，不报任何错。
 * 用户看到的是「超过 5 层就看不到下面的内容了」。
 *
 * 后端因此多回一位 `truncated`：这个目录还有内容、只是没走下去。
 * 前端展开它的时候再取一层，深度从那里重新起算，所以整棵树没有深度上限。
 *
 * 抽成不依赖 React 的函数是为了能直接测 —— 合并子树时最容易出的错
 * （把整棵树替换掉、或者 key 对不上导致合了个寂寞）在界面上只表现为
 * 「点了没反应」。
 */

export interface TreeNode {
  name: string;
  path: string;
  is_dir: boolean;
  size_bytes: number;
  extension?: string | null;
  children?: TreeNode[] | null;
  truncated?: boolean;
}

/**
 * 这个目录展开时要不要去后端再取一层。
 *
 * 判据是**两条都成立**：它自报还有内容，而且这次确实没带回来。
 * 只看 `truncated` 的话，已经取过的目录每次展开都会再取一遍。
 */
export function needsFetch(node: TreeNode): boolean {
  return node.is_dir && !!node.truncated && !node.children;
}

/**
 * 把取回来的一层挂到树上的 `path` 那个节点下。
 *
 * **返回新的数组**（React 要靠引用变化重渲染），但没被碰到的分支
 * 原样复用 —— 整棵树深拷贝会让所有节点都重渲染。
 *
 * 挂上之后 `truncated` 清掉：内容已经在手里了，再标着的话
 * `needsFetch` 会一直是 true。
 */
export function mergeChildren(
  nodes: TreeNode[],
  path: string,
  children: TreeNode[],
): TreeNode[] {
  let changed = false;
  const next = nodes.map((n) => {
    if (n.path === path) {
      changed = true;
      return { ...n, children, truncated: false };
    }
    // 只往**是这条路径前缀**的分支里找 —— 否则每次合并都要走遍整棵树
    if (n.children && path.startsWith(`${n.path}/`)) {
      const kids = mergeChildren(n.children, path, children);
      if (kids !== n.children) {
        changed = true;
        return { ...n, children: kids };
      }
    }
    return n;
  });
  return changed ? next : nodes;
}

/**
 * 一个已展开的目录**现在该显示什么**。
 *
 * 四种状态在界面上必须能分开，否则就全塌成「展开了什么都没有」这一种 ——
 * 而那正是当初看不出第 5 层去哪了的原因。
 *
 * `failed` 是补上的第四种：取子树失败时不能继续显示「载入中…」。
 * 那条永远转不完的提示比报错更糟 —— 用户会一直等，而且没有重试的入口。
 */
export type FolderState = "loading" | "failed" | "empty" | "children";

export function folderState(
  node: TreeNode,
  loading: boolean,
  failed = false,
): FolderState {
  if (loading) return "loading";
  // 失败要排在 needsFetch 前面：失败之后 children 仍然是 null、
  // truncated 仍然是 true，只看 needsFetch 的话会一直说"载入中"
  if (failed) return "failed";
  if (needsFetch(node)) return "loading";
  if (!node.children || node.children.length === 0) return "empty";
  return "children";
}
