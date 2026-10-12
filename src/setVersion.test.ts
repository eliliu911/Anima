import { execFileSync } from "node:child_process";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * 版本号档位计算。
 *
 * # 为什么非要一个测试
 *
 * **版本号算错的后果从现象上查不回来。** 退了版本号，updater 会告诉所有人
 * 「已经是最新的」；预发布转正时多进一位，`0.9.1-beta.3` 会变成 `0.9.2`
 * 而 `0.9.1` 永远不存在。两种都不报错。
 *
 * 而且 2026-09-18 写这段时，我用临时 shell 脚本验了两次、两次都是**验证工具
 * 自己坏了**（一次是 `touch -t` 月份非法，一次是 zsh 默认不做单词拆分），
 * 差点据此去改正确的代码。所以这里跑真脚本、用 bash 显式调。
 */
const SCRIPT = join(new URL("..", import.meta.url).pathname, "scripts/set-version.sh");

const next = (kind: string, from: string): string =>
  execFileSync("bash", [SCRIPT, "--next", kind, from], { encoding: "utf8" }).trim();

const fails = (kind: string, from: string): string => {
  try {
    execFileSync("bash", [SCRIPT, "--next", kind, from], { encoding: "utf8", stdio: "pipe" });
    return "";
  } catch (e) {
    const err = e as { stderr?: Buffer; stdout?: Buffer };
    return String(err.stderr ?? "") + String(err.stdout ?? "");
  }
};

describe("版本号档位", () => {
  it("patch 只动最后一位", () => {
    expect(next("patch", "0.9.0")).toBe("0.9.1");
    expect(next("patch", "1.2.9")).toBe("1.2.10");
  });

  /** **预发布转正不是再进一位。** 多进一位的话 0.9.1 这个版本永远不存在 */
  it("预发布转正落在同一个版本上", () => {
    expect(next("patch", "0.9.1-beta.3")).toBe("0.9.1");
    expect(next("release", "0.9.1-beta.3")).toBe("0.9.1");
    expect(next("release", "0.9.1-beta.1")).toBe("0.9.1");
  });

  it("minor 清零 patch，major 清零两位", () => {
    expect(next("minor", "0.9.7")).toBe("0.10.0");
    expect(next("major", "0.9.7")).toBe("1.0.0");
    // 两位数不能被当成字符串比较
    expect(next("minor", "0.10.0")).toBe("0.11.0");
  });

  it("beta 第一次先进一位，之后只递增后缀", () => {
    expect(next("beta", "0.9.0")).toBe("0.9.1-beta.1");
    expect(next("beta", "0.9.1-beta.1")).toBe("0.9.1-beta.2");
    expect(next("beta", "0.9.1-beta.9")).toBe("0.9.1-beta.10");
  });

  /** 认不出的输入要**停下来**，不能猜一个版本号出来 */
  it("读不懂的输入直接失败", () => {
    expect(fails("patch", "v0.9.0")).toContain("读不懂");
    expect(fails("patch", "0.9")).toContain("读不懂");
    expect(fails("啥档位", "0.9.0")).toContain("不认识的档位");
    expect(fails("release", "0.9.0")).toContain("本来就不是预发布");
    // 非 beta.N 的预发布后缀不猜
    expect(fails("beta", "0.9.1-rc.1")).toContain("手工写完整版本号");
  });

  /** `--next` **只算不改** —— 它被 usage 和测试反复调用，不能有副作用 */
  it("--next 不改任何文件", () => {
    const before = execFileSync("git", ["status", "--porcelain", "-uno"], { encoding: "utf8" });
    next("minor", "0.9.0");
    next("beta", "0.9.0");
    const after = execFileSync("git", ["status", "--porcelain", "-uno"], { encoding: "utf8" });
    expect(after).toBe(before);
  });
});
