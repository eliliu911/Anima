import { execFileSync } from "node:child_process";
import { mkdtempSync, readdirSync, rmSync, writeFileSync, mkdirSync, utimesSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

/**
 * 备份回收的「删哪几个」逻辑。
 *
 * # 为什么值得一个测试
 *
 * **删错方向是不可逆的。** `.app` 备份删错了还能重新构建，数据备份删错了
 * 就永远没了 —— 而 `ls -t | tail -n +N` 这种写法看一眼根本分不出
 * 「留最新的」还是「留最旧的」。
 *
 * 2026-09-18 写这段时我用临时脚本验了一次，结论是「它删了新的」——
 * **那个结论是错的**，真正坏掉的是我的测试夹具（`touch -t 260918010000`
 * 被当成「26 年 09 月 18 月」，非法，四个文件 mtime 全一样，`ls -t` 退化成
 * 按名字排）。差点据此改坏正确的代码。
 *
 * 所以这里用真实 mtime 跑真实命令。
 */
const CMD = (dir: string, keep: number) =>
  `ls -t "${dir}"/anima-backup-*.tar.gz 2>/dev/null | tail -n +$((${keep} + 1))`;

function fixture(): string {
  const dir = mkdtempSync(join(tmpdir(), "anima-bk-"));
  mkdirSync(join(dir, "apps"));
  writeFileSync(join(dir, "apps", "Anima-x.app"), "");
  writeFileSync(join(dir, "别动我.txt"), "");
  // 四份备份，11 最旧、14 最新
  ["11", "12", "13", "14"].forEach((d, i) => {
    const f = join(dir, `anima-backup-202609${d}-000000.tar.gz`);
    writeFileSync(f, "x");
    const t = new Date(2026, 8, 18, 0, i + 1);
    utimesSync(f, t, t);
  });
  return dir;
}

const picked = (dir: string, keep: number) =>
  execFileSync("bash", ["-c", CMD(dir, keep)], { encoding: "utf8" })
    .split("\n")
    .filter(Boolean)
    .map((p) => p.split("/").pop()!);

describe("备份回收", () => {
  /** **留最新的，删最旧的。** 反了的话每次备份都会把刚存的那份删掉 */
  it("选中的是最旧的那几个", () => {
    const dir = fixture();
    try {
      expect(picked(dir, 2)).toEqual([
        "anima-backup-20260912-000000.tar.gz",
        "anima-backup-20260911-000000.tar.gz",
      ]);
      // 保留数变了要跟着变
      expect(picked(dir, 3)).toEqual(["anima-backup-20260911-000000.tar.gz"]);
      expect(picked(dir, 4)).toEqual([]);
      expect(picked(dir, 9)).toEqual([]);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  /** **只认 anima-backup-*.tar.gz。** 同目录下还有 apps/ 和别的文件 */
  it("不碰同目录下的别的东西", () => {
    const dir = fixture();
    try {
      for (const f of picked(dir, 2)) expect(f).toMatch(/^anima-backup-.*\.tar\.gz$/);
      // 删完之后那些文件还在
      execFileSync("bash", ["-c", `${CMD(dir, 2)} | while read -r f; do rm -f "$f"; done`]);
      const left = readdirSync(dir).sort();
      expect(left).toContain("apps");
      expect(left).toContain("别动我.txt");
      expect(left.filter((x) => x.endsWith(".tar.gz"))).toEqual([
        "anima-backup-20260913-000000.tar.gz",
        "anima-backup-20260914-000000.tar.gz",
      ]);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  /** 一份都没有时不能报错 —— 第一次备份就会走到这里 */
  it("没有备份时安静返回", () => {
    const dir = mkdtempSync(join(tmpdir(), "anima-bk-empty-"));
    try {
      expect(picked(dir, 2)).toEqual([]);
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});
