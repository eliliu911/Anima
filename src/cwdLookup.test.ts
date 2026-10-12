import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * **不许再出现新的「从 cwd 往上找项目目录」。**
 *
 * # 为什么要这个扫描
 *
 * 双击的 `.app` 启动时 cwd 是 `/`，所以任何靠 `current_dir()` 去找项目目录的
 * 代码在打包版里**必然落空，而且不报错** —— 症状是「角色形态只剩一个」
 * 「技能页空白」「风格缩略图一张不显示」这种和路径毫无关联的样子。
 *
 * 2026-09-18 这个坑**修了三轮**，每一轮都是「修当时看到的那几处，
 * 没去扫还有谁用同样的写法」：
 *
 * 1. 夜里：`tools/` 找不到 → 只修了 sidecar 那一处
 * 2. 早上：`characters/` `skills/` `voice-library/` `avatars/` 四处一起爆
 * 3. 下午：风格缩略图 —— `cinema::workspace_root()` 也是同一个写法
 *
 * 光靠「记得去扫」是不行的，所以钉成测试。正确的做法是走
 * `crate::app_root`：它把指针文件排在 cwd 之前，那是双击启动唯一能用的路。
 *
 * # 白名单里的那几处为什么可以
 *
 * 它们用 cwd 的语义是「用户在哪儿跑这个命令」，不是「项目在哪儿」——
 * 那是对的用法。
 */
const SRC = join(new URL("..", import.meta.url).pathname, "src-tauri/src");

/** 这些文件里的 `current_dir()` 是有意的，不算违规 */
const ALLOWED = new Set([
  // 候选列表自己就要用 cwd —— 开发期靠它
  "app_root.rs",
  // Agent 运行时的默认工作区 = 用户在哪儿跑的，本来就该是 cwd
  "agent_runtime/service.rs",
  // 调用方没传工作区时的兜底，语义同上
  "workspace.rs",
]);

/** 只看 `#[cfg(test)]` 之前的部分 —— 测试里的 cwd 和假路径都是夹具，不算 */
function prodPart(text: string): string {
  const at = text.indexOf("#[cfg(test)]");
  return at >= 0 ? text.slice(0, at) : text;
}

function rustFiles(dir: string, base = ""): string[] {
  return readdirSync(dir).flatMap((name) => {
    const full = join(dir, name);
    const rel = base ? `${base}/${name}` : name;
    if (statSync(full).isDirectory()) return name === "bin" ? [] : rustFiles(full, rel);
    return name.endsWith(".rs") ? [rel] : [];
  });
}

describe("不许从 cwd 找项目目录", () => {
  it("只有白名单里的文件能用 current_dir()", () => {
    const bad: string[] = [];
    for (const rel of rustFiles(SRC)) {
      if (ALLOWED.has(rel)) continue;
      prodPart(readFileSync(join(SRC, rel), "utf8"))
        .split("\n")
        .forEach((line, i) => {
          if (!line.includes("current_dir()")) return;
          const t = line.trim();
          if (t.startsWith("//") || t.startsWith("*")) return;
          bad.push(`${rel}:${i + 1}  ${t}`);
        });
    }
    expect(
      bad,
      `这些地方在用 cwd 找路径，打包版里会落空且不报错。改走 crate::app_root：\n${bad.join("\n")}`,
    ).toEqual([]);
  });

  /** 扫描本身失效了就等于没有这个测试 */
  it("确实扫到了文件", () => {
    const files = rustFiles(SRC);
    expect(files.length).toBeGreaterThan(30);
    expect(files).toContain("app_root.rs");
  });

  /**
   * **硬编码的绝对路径同样是打包版的雷** —— 换台机器、换个用户名就断。
   * 这里只钉住「不要再新增」，已有的那几处在 cinema.rs 里，单独列出来。
   */
  /**
   * **硬编码开发者的家目录是打包版的另一类雷** —— 换台机器、换个用户名就断，
   * 而且同样不报错（只是那条路径永远不存在）。
   *
   * 只认 `/Users/you`（这台机器的真实家目录）：测试夹具里的
   * `/Users/me`、`/Users/x` 是有意写死的假路径，不算。
   *
   * `cinema.rs` 里还剩几处（LivePortrait 脚本、ComfyUI 的 venv），
   * 单独列出来 —— 它们指向的是**别的项目**，不是 Anima 自己的路径。
   */
  it("不许新增硬编码的开发者家目录", () => {
    const known = new Set(["cinema.rs", "commands.rs"]);
    const bad: string[] = [];
    for (const rel of rustFiles(SRC)) {
      if (known.has(rel)) continue;
      prodPart(readFileSync(join(SRC, rel), "utf8"))
        .split("\n")
        .forEach((line, i) => {
          if (!line.includes("/Users/you")) return;
          const t = line.trim();
          if (t.startsWith("//") || t.startsWith("*")) return;
          bad.push(`${rel}:${i + 1}  ${t}`);
        });
    }
    expect(bad, `硬编码开发者家目录，换台机器就断：\n${bad.join("\n")}`).toEqual([]);
  });
});
