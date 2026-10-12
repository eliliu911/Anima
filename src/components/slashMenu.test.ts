import { describe, expect, it } from "vitest";
import {
  completionFor,
  filterEntries,
  justOpened,
  moveIndex,
  parseSubmitted,
  slashQuery,
} from "./slashMenu";
import type { SlashEntry } from "../api/voice";

const entry = (
  id: string,
  kind: SlashEntry["kind"] = "skill",
  takes_args = false,
  title = id,
): SlashEntry => ({ id, title, description: "", kind, takes_args, action: null });

describe("什么时候弹", () => {
  it("正在敲命令名时弹", () => {
    expect(slashQuery("/")).toBe("");
    expect(slashQuery("/rev")).toBe("rev");
  });

  it("开始敲参数之后就收起", () => {
    // 不收的话 Enter 会被弹层劫走，`/review src/lru.ts` 永远发不出去
    expect(slashQuery("/review ")).toBeNull();
    expect(slashQuery("/review src/lru.ts")).toBeNull();
  });

  it("第二行开头的斜杠不弹", () => {
    // 多行里那通常是 Markdown 列表或路径
    expect(slashQuery("看这个\n/foo")).toBeNull();
    expect(slashQuery(" /foo")).toBeNull();
  });
});

describe("绝对路径不能被当成命令", () => {
  const entries = [entry("review"), entry("clear", "builtin")];

  it("形式上像命令但匹配不到任何候选", () => {
    // slashQuery 会认它（`/Users/x` 没有空白），但**匹配结果为空**，
    // 所以弹层不显示。靠匹配兜住而不是靠正则去猜路径长什么样。
    expect(slashQuery("/Users/you/AI")).toBe("Users/you/AI");
    expect(filterEntries(entries, "Users/you/AI")).toEqual([]);
  });

  it("提交时解析得出名字，但调用方查不到条目 → 当普通文本发", () => {
    const p = parseSubmitted("/Users/you/note.md 看一下");
    expect(p).toEqual({ name: "Users/you/note.md", args: "看一下" });
    expect(entries.some((e) => e.id === p!.name)).toBe(false);
  });
});

describe("匹配与排序", () => {
  const entries = [
    entry("clear", "builtin"),
    entry("code-reviewer", "skill", true, "代码审查"),
    entry("commit", "skill"),
  ];

  it("完全相等排最前，其次前缀", () => {
    expect(filterEntries(entries, "c").map((e) => e.id)).toEqual([
      "code-reviewer",
      "commit",
      "clear",
    ]);
    expect(filterEntries(entries, "clear")[0].id).toBe("clear");
  });

  it("同分时技能排在内置动作前面", () => {
    // 内置动作是固定的几个、位置很快被记住；技能是会增长的那部分
    const r = filterEntries([entry("x", "builtin"), entry("y", "skill")], "");
    expect(r.map((e) => e.id)).toEqual(["y", "x"]);
  });

  it("首字母缩写能命中", () => {
    expect(filterEntries(entries, "cr")[0].id).toBe("code-reviewer");
  });

  it("中文标题也能搜", () => {
    expect(filterEntries(entries, "代码")[0].id).toBe("code-reviewer");
  });

  it("查询为空时全部保留", () => {
    expect(filterEntries(entries, "").length).toBe(3);
  });
});

describe("选中之后", () => {
  it("需要参数就留一个空格等着打", () => {
    expect(completionFor(entry("review", "skill", true))).toBe("/review ");
  });

  it("不需要参数就立刻执行", () => {
    expect(completionFor(entry("clear", "builtin", false))).toBeNull();
  });
});

describe("上下键环绕", () => {
  it("到头折回另一端", () => {
    expect(moveIndex(0, -1, 3)).toBe(2);
    expect(moveIndex(2, 1, 3)).toBe(0);
    expect(moveIndex(0, 1, 3)).toBe(1);
  });

  it("空列表不越界", () => {
    expect(moveIndex(0, 1, 0)).toBe(0);
  });
});

describe("提交解析", () => {
  it("命令名与参数分开", () => {
    expect(parseSubmitted("/review src/a.ts src/b.ts")).toEqual({
      name: "review",
      args: "src/a.ts src/b.ts",
    });
    expect(parseSubmitted("/clear")).toEqual({ name: "clear", args: "" });
  });

  it("不是斜杠开头就返回 null", () => {
    expect(parseSubmitted("普通消息")).toBeNull();
    // 只有一个斜杠、后面什么都没有：不构成命令
    expect(parseSubmitted("/")).toBeNull();
  });
});

/**
 * **弹层打开的那一刻要重拉候选。**
 *
 * 技能是会在对话里被创建的 —— Agent 往 `.anima/skills/` 写完几个
 * SKILL.md，文件是好的、后端也认，但菜单还是进会话时那一份。
 * 真出过事：Agent 报告「两个 Skill 已完整落地」，用户打 `/`，一个都没有。
 */
describe("什么时候重拉候选", () => {
  it("只在从关到开的那一刻算", () => {
    expect(justOpened(null, "")).toBe(true);
    expect(justOpened(null, "sk")).toBe(true);
  });

  /** 已经开着的时候每按一次键都拉是浪费。 */
  it("开着的时候继续打字不算", () => {
    expect(justOpened("", "s")).toBe(false);
    expect(justOpened("s", "sk")).toBe(false);
    expect(justOpened("sk", "s")).toBe(false);
  });

  it("关掉的时候不算", () => {
    expect(justOpened("sk", null)).toBe(false);
    expect(justOpened(null, null)).toBe(false);
  });
});
