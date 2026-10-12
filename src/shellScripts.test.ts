import { describe, expect, it } from "vitest";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const DIR = new URL("../scripts/", import.meta.url).pathname;
const shells = readdirSync(DIR).filter((f) => f.endsWith(".sh"));

/**
 * **`$VAR` 后面直接跟中文会被 bash 吃掉。**
 *
 * `printf '%s（权限）' "$SECRET_FILE（"` —— bash 把 `（` 的字节当成变量名的一部分，
 * 于是变量名成了 `SECRET_FILE\xef`，在 `set -u` 下直接报「未绑定的变量」。
 *
 * 这个坑在这个仓库**咬过两次**（2026-09-17 的 `anima backup`、
 * 2026-09-18 的 `anima key`），两次都是写完没跑那一条分支才漏出去的。
 * 写成 `${VAR}` 就没事。
 *
 * 之所以值得一个测试：脚本里绝大多数分支平时不跑，`bash -n` 也查不出来 ——
 * 它是运行期错误，不是语法错误。
 */
describe("shell 脚本", () => {
  it.each(shells)("%s 里 $VAR 后面不能直接跟非 ASCII 字符", (name) => {
    const text = readFileSync(join(DIR, name), "utf8");
    const bad: string[] = [];
    text.split("\n").forEach((line, i) => {
      // 跳过注释行：注释里的 $VAR 不参与展开
      if (line.trim().startsWith("#")) return;
      for (const m of line.matchAll(/\$([A-Za-z_][A-Za-z_0-9]*)(?=[^\x00-\x7f])/g)) {
        bad.push(`${name}:${i + 1}  $${m[1]} → 写成 \${${m[1]}}`);
      }
    });
    expect(bad, bad.join("\n")).toEqual([]);
  });

  it("确实扫到了脚本 —— 空的说明路径错了，不是真没问题", () => {
    expect(shells.length).toBeGreaterThan(0);
  });
});

/**
 * **`git status --porcelain` 默认把未跟踪文件也算成「脏」。**
 *
 * 这个工作区里长期躺着一堆未跟踪的设计稿，所以不加 `-uno` 的话每一次判断
 * 都是「脏」—— 护栏永远红着、构建永远标 `-dirty`，等于没有。
 *
 * 这个坑 2026-09-18 咬了两次：先是 `anima update` 的工作区检查（当天修了），
 * 然后是 `build.rs` 烤 `-dirty` 标记（同一天下午才发现，表现是更新面板上
 * 一条永远消不掉的假警报）。所以钉住它。
 */
describe("git status 的脏判断", () => {
  // 公开版没有 scripts/release.sh（不发布安装包），在的才查
  const files = ["scripts/anima.sh", "scripts/release.sh", "src-tauri/build.rs"].filter((rel) => existsSync(join(DIR, "..", rel)));
  it.each(files)("%s 里的 git status --porcelain 必须带 -uno", (rel) => {
    const text = readFileSync(join(DIR, "..", rel), "utf8");
    const bad: string[] = [];
    text.split("\n").forEach((line, i) => {
      if (!line.includes("--porcelain")) return;
      if (line.trim().startsWith("#")) return;
      // 同一行里要出现 -uno（shell 是 `-uno`，Rust 是 `"-uno"`）
      if (!/-uno/.test(line)) bad.push(`${rel}:${i + 1}  ${line.trim()}`);
    });
    expect(bad, bad.join("\n")).toEqual([]);
  });
});
