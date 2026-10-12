import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

import { IGNORED_DIRS } from "../vite.config";

/**
 * 守住一个**没有任何报错**的失败方式。
 *
 * Rust 侧凡是 `find_project_*` 的函数，都优先用项目内的目录，于是应用
 * 运行时会往那儿写文件。而 vite 在 watch 整个项目根 —— 那些 JSON 不在
 * 模块图里，vite 处理不了就退回整页 reload，还广播给每个窗口。
 * 用户看到的是「改个音色，所有面板白屏，模型重载」，控制台一行错误都没有。
 *
 * 所以这里不抄一份名单，而是**从 Rust 源码里现抽** —— 以后谁多认了一个
 * 项目目录、忘了同步 vite 配置，这个测试会先炸。
 *
 * 2026-09-18 抽取方式改过一次：原来每个目录各有一个
 * `fn find_project_xxx()`，里面写死 `dir.join("...")`；后来那五处合并成了
 * `app_root::find_plain("xxx", ..)` / `find_dir("xxx", ..)`（打包版找不到
 * 项目目录那个 bug 的修复）。**这个测试当场就炸了，抓得对** ——
 * 现在目录名只有一处，抽取反而更准。
 */
const ROOT = join(__dirname, "..");

function rustFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return rustFiles(path);
    return name.endsWith(".rs") ? [path] : [];
  });
}

/** 抽出 `app_root::find_plain("xxx", ..)` / `find_dir("xxx", ..)` 里的目录名 */
function projectDirsUsedByRust(): string[] {
  const found = new Set<string>();
  for (const file of rustFiles(join(ROOT, "src-tauri", "src"))) {
    const src = readFileSync(file, "utf8");
    for (const m of src.matchAll(/app_root::find_(?:plain|dir)\(\s*"([^"]+)"/g)) {
      found.add(m[1]);
    }
  }
  return [...found];
}

describe("vite 不能 watch 应用运行时写入的目录", () => {
  it("Rust 侧用到的项目目录都被 vite 忽略", () => {
    const fromRust = projectDirsUsedByRust();

    // 抽取本身失效了（正则跟不上代码改动）就等于测试失去意义，先钉住
    expect(fromRust).toContain("characters");
    expect(fromRust.length).toBeGreaterThanOrEqual(4);

    for (const dir of fromRust) {
      expect(IGNORED_DIRS, `${dir}/ 会被应用写入，但 vite 还在 watch 它`).toContain(dir);
    }
  });

  it("角色绑定写的那三个文件不会触发整页 reload", async () => {
    const config = (await import("../vite.config")).default;
    const ignored = config.server!.watch!.ignored as string[];

    const written = [
      "characters/ad_xuejie/voice.json",
      "characters/ad_xuejie/avatar.json",
      "characters/ad_xuejie/skills.json",
    ];
    for (const path of written) {
      const hit = ignored.some((pattern) => {
        const re = new RegExp("^" + pattern.replace(/\*\*/g, "@@").replace(/\*/g, "[^/]*").replace(/@@/g, ".*") + "$");
        return re.test(path) || re.test(join(ROOT, path));
      });
      expect(hit, `保存绑定会写 ${path}，它必须被 vite 忽略`).toBe(true);
    }
  });
});
