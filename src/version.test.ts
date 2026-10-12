import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";

const read = (p: string) => readFileSync(new URL(`../${p}`, import.meta.url), "utf8");

/**
 * 版本号写在**四个文件**里，各有各的读者，谁都省不掉：
 *
 * - `package.json` —— npm / vite
 * - `src-tauri/Cargo.toml` —— `env!("CARGO_PKG_VERSION")`
 * - `src-tauri/tauri.conf.json` —— bundle 的版本、updater 比大小用的那个
 * - `src-tauri/Cargo.lock` —— **这一份 2026-09-18 才补上**。不改它的话
 *   下次 `cargo build` 会自己改，于是工作区凭空多一个已跟踪文件的改动，
 *   而 `anima update` 和 `release.sh` 都因为「工作区不干净」拒绝工作、
 *   界面还标一个 `-dirty` —— 完全想不到是改版本号留下的
 *
 * **四份手工同步，迟早漏一处。** 而漏了之后：界面显示的版本、更新器比较的版本、
 * `.app` 里的版本会互相打架 —— **且不报错**，只表现为「明明发了新版却提示没有更新」
 * 这种查起来要命的症状。
 *
 * 这个仓库栽在「一份清单抄在两处」上已经很多次（示例图清单、`engines_for_arch`
 * 的 `_` 兜底、`stepsFor.test` 的硬编码列表、我自己那条带硬编码名单的守卫测试…）。
 * 所以这里按同一个方子：**一个设置入口（`scripts/set-version.sh`）+ 一条读四份
 * 文件比对的测试**。
 */
/**
 * **`v<当前版本号>` 这个 tag 要么不存在，要么正好指向 HEAD。**
 *
 * 2026-09-18 当场撞上：`v0.10.0` 打完之后又提交了修复，而版本号没动 ——
 * 于是 HEAD **在冒充一个已发布的版本**。界面显示 0.10.0，而那根本不是
 * 打了 tag 的那一版；对外发布的话 updater 会说「已经是最新的」。
 *
 * 原来的规划只写了「自用重建不改版本号」，漏了「打完 tag 之后怎么办」。
 * 答案是：**tag 之后的第一个提交就该进下一位**。
 *
 * 这条不变量正好能表达它：打 tag 那一刻 HEAD == tag（过）；下一个提交
 * 让它们分开（挂）；把版本号进一位之后 `v<新版本>` 不存在（过）。
 */
function gitOut(args: string[]): string {
  try {
    return execFileSync("git", args, { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] }).trim();
  } catch {
    return "";
  }
}

describe("版本号和 tag", () => {
  it("HEAD 不能冒充一个已发布的版本", () => {
    const v = JSON.parse(read("package.json")).version as string;
    const tag = `v${v}`;
    if (!gitOut(["rev-parse", "--verify", `${tag}^{commit}`])) return; // 没这个 tag，说明还没发过
    const ahead = gitOut(["rev-list", "--count", `${tag}..HEAD`]);
    expect(
      ahead,
      `${tag} 已经打过了，而 HEAD 比它新 ${ahead} 个提交 —— 版本号该进一位了：
` +
        gitOut(["log", "--oneline", "--no-decorate", `${tag}..HEAD`]) +
        `
跑一句：./scripts/set-version.sh patch`,
    ).toBe("0");
  });
});

describe("版本号", () => {
  const pkg = JSON.parse(read("package.json")).version as string;
  const conf = JSON.parse(read("src-tauri/tauri.conf.json")).version as string;
  const cargo = (() => {
    const s = read("src-tauri/Cargo.toml");
    // 只取 [package] 段那个 —— 依赖也有 version =，不能瞎抓
    const head = s.slice(0, s.indexOf("[dependencies]") >= 0 ? s.indexOf("[dependencies]") : s.length);
    const m = head.match(/^version\s*=\s*"([^"]+)"/m);
    return m?.[1];
  })();
  const lock = (() => {
    // 只认 `name = "anima"` 紧跟着那一行 —— 锁文件里有几百个包
    const m = read("src-tauri/Cargo.lock").match(/^name = "anima"\nversion = "([^"]+)"/m);
    return m?.[1];
  })();

  it("四处都读得出来", () => {
    expect(pkg, "package.json").toBeTruthy();
    expect(conf, "tauri.conf.json").toBeTruthy();
    expect(cargo, "Cargo.toml").toBeTruthy();
    expect(lock, "Cargo.lock").toBeTruthy();
  });

  /** **这条是这个文件存在的全部理由。** */
  it("四处完全一致", () => {
    expect({ pkg, cargo, conf, lock }).toEqual({ pkg, cargo: pkg, conf: pkg, lock: pkg });
  });

  /**
   * updater 靠语义化版本比大小。写成 `0.9` 或 `v0.9.0` 会让它**永远判不出有新版**
   * —— 不报错，只是更新提示再也不出现。
   */
  it("是合法的语义化版本", () => {
    expect(pkg).toMatch(/^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/);
  });
});
