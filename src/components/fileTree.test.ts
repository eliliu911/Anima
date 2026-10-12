import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { folderState, mergeChildren, needsFetch, type TreeNode } from "./fileTree";

const dir = (path: string, over: Partial<TreeNode> = {}): TreeNode => ({
  name: path.split("/").pop()!,
  path,
  is_dir: true,
  size_bytes: 0,
  ...over,
});
const file = (path: string): TreeNode => ({
  name: path.split("/").pop()!,
  path,
  is_dir: false,
  size_bytes: 1,
});

describe("要不要再取一层", () => {
  it("自报还有内容、而且这次没带回来 —— 取", () => {
    expect(needsFetch(dir("a/b/c/d", { truncated: true }))).toBe(true);
  });

  /** 已经取过的不能每次展开都再取一遍。 */
  it("已经有 children 了就不取", () => {
    expect(needsFetch(dir("a", { truncated: true, children: [file("a/x.md")] }))).toBe(false);
  });

  /**
   * **真的空的目录不取。** 后端已经区分过了（`dir_has_visible_entries`），
   * 这里再取一次只会拿回空数组，而展开箭头会一直暗示"还有内容"。
   */
  it("真的空的目录不取", () => {
    expect(needsFetch(dir("a/空的", { children: [] }))).toBe(false);
    expect(needsFetch(dir("a/空的", { truncated: false }))).toBe(false);
  });

  it("文件不取", () => {
    expect(needsFetch({ ...file("a/x.md"), truncated: true })).toBe(false);
  });
});

describe("把取回来的一层挂上去", () => {
  const tree = (): TreeNode[] => [
    dir("novels", {
      children: [
        dir("novels/ai-last-command", {
          children: [
            dir("novels/ai-last-command/vol1", {
              children: [dir("novels/ai-last-command/vol1/剧本", { truncated: true })],
            }),
          ],
        }),
      ],
    }),
    dir("src", { children: [file("src/main.ts")] }),
  ];

  const find = (nodes: TreeNode[], path: string): TreeNode | undefined => {
    for (const n of nodes) {
      if (n.path === path) return n;
      const hit = n.children ? find(n.children, path) : undefined;
      if (hit) return hit;
    }
    return undefined;
  };

  it("挂到第 5 层那个节点下", () => {
    const kids = [file("novels/ai-last-command/vol1/剧本/第01章.md")];
    const next = mergeChildren(tree(), "novels/ai-last-command/vol1/剧本", kids);
    const node = find(next, "novels/ai-last-command/vol1/剧本")!;
    expect(node.children).toEqual(kids);
  });

  /**
   * **挂上之后要清掉 `truncated`。** 不清的话 `needsFetch` 永远是 true，
   * 每次折叠再展开都重新请求一次 —— 看起来像"卡了一下"。
   */
  it("挂上之后不再标还有内容", () => {
    const next = mergeChildren(tree(), "novels/ai-last-command/vol1/剧本", []);
    expect(find(next, "novels/ai-last-command/vol1/剧本")!.truncated).toBe(false);
  });

  /** React 要靠引用变化重渲染 —— 原地改的话点了没反应。 */
  it("路径上的每一层都是新对象", () => {
    const before = tree();
    const next = mergeChildren(before, "novels/ai-last-command/vol1/剧本", []);
    expect(next).not.toBe(before);
    expect(find(next, "novels")).not.toBe(find(before, "novels"));
    expect(find(next, "novels/ai-last-command/vol1")).not.toBe(
      find(before, "novels/ai-last-command/vol1"),
    );
  });

  /**
   * **不能原地改调用方的树。**
   *
   * 原地改的话 `before` 和 `next` 指向同一批节点，于是"改之前是什么样"
   * 就丢了 —— React 的 `setState(prev => ...)` 拿到的 prev 已经被改过，
   * 任何依赖前后对比的逻辑都会静默失效。这一条变异测试抓到过：
   * 只断言祖先是新对象的话，叶子被原地改仍然全绿。
   */
  it("传进去的树一点都没被改", () => {
    const before = tree();
    mergeChildren(before, "novels/ai-last-command/vol1/剧本", [file("x")]);
    const leaf = find(before, "novels/ai-last-command/vol1/剧本")!;
    expect(leaf.children).toBeUndefined();
    expect(leaf.truncated).toBe(true);
  });

  /** 没碰到的分支要原样复用，否则整棵树都重渲染。 */
  it("旁支原样复用", () => {
    const before = tree();
    const next = mergeChildren(before, "novels/ai-last-command/vol1/剧本", []);
    expect(find(next, "src")).toBe(find(before, "src"));
  });

  it("路径不存在时整棵树原样返回", () => {
    const before = tree();
    expect(mergeChildren(before, "不存在/的/路径", [])).toBe(before);
  });
});

describe("展开之后显示什么", () => {
  /**
   * **三种状态必须能分开。**
   *
   * 这条是这次修复的核心：`children: null`（没走下去）和 `children: []`
   * （真的空）原先都渲染成"什么都没有"，于是「第 5 层以下不见了」
   * 看起来和「这个目录本来就是空的」一模一样，还不报错。
   */
  it("没取回来是 loading，真的空是 empty", () => {
    expect(folderState(dir("a", { truncated: true }), false)).toBe("loading");
    expect(folderState(dir("a", { children: [] }), false)).toBe("empty");
    expect(folderState(dir("a", { children: [file("a/x")] }), false)).toBe("children");
  });

  it("正在请求中也是 loading", () => {
    expect(folderState(dir("a", { children: [] }), true)).toBe("loading");
  });

  /**
   * **取失败之后不能继续说"载入中"。**
   *
   * 失败时 `children` 仍然是 null、`truncated` 仍然是 true，所以只看
   * `needsFetch` 的话会永远显示「载入中…」—— 一条永远转不完的提示，
   * 用户会一直等，而且没有重试入口。这比直接报错糟得多。
   */
  it("失败是 failed，不是 loading", () => {
    expect(folderState(dir("a", { truncated: true }), false, true)).toBe("failed");
    // 正在重试的时候优先显示 loading
    expect(folderState(dir("a", { truncated: true }), true, true)).toBe("loading");
  });
});

describe("和 Rust 侧的约定", () => {
  /**
   * **`truncated` 这个字段名两边必须一致。** 拼错的表现是它永远读到
   * `undefined` → `needsFetch` 恒为 false → 第 5 层永远取不回来，
   * 也就是这次要修的那个症状本身，而且照样不报错。
   */
  it("字段名对得上", () => {
    const rust = readFileSync(new URL("../../src-tauri/src/workspace.rs", import.meta.url), "utf8");
    expect(rust).toContain("pub truncated: bool,");
    // serde 用 snake_case 直出，没有 rename
    expect(rust).not.toContain('rename = "truncated"');
  });
});
