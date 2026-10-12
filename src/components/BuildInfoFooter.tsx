/**
 * 设置页底部的构建信息。
 *
 * **「我在跑哪一版」必须能答得出来。** 在这之前界面上一处版本号都没有 ——
 * 用户报 bug 时说不清自己跑的是哪个构建，而这个项目一天能出十几个提交。
 *
 * 显示的不只是版本号：
 *
 * - **git sha** —— 同一个版本号可能对应很多次构建
 * - **`-dirty` 后缀** —— 带未提交改动编出来的，和发布的同名版本**不是一个东西**
 * - **`debug` 标记** —— 开发构建和正式构建要一眼分清
 *
 * 点一下复制全部，方便贴进 issue。
 */
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export interface BuildInfo {
  version: string;
  git_sha: string;
  built_at: string;
  debug: boolean;
}

/** 拼成一行给人看 / 贴进 issue 的文本。**纯函数，方便测。** */
export function buildLine(b: BuildInfo): string {
  const tags = [b.debug ? "debug" : null, b.git_sha.endsWith("-dirty") ? "dirty" : null]
    .filter(Boolean)
    .join(" · ");
  const sha = b.git_sha.replace(/-dirty$/, "");
  return `Anima ${b.version} · ${sha}${tags ? ` · ${tags}` : ""} · ${b.built_at}`;
}

export function BuildInfoFooter() {
  const [info, setInfo] = useState<BuildInfo | null>(null);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    // 拿不到就不显示 —— 显示一行「未知」只会让人以为坏了
    void invoke<BuildInfo>("build_info").then(setInfo).catch(() => setInfo(null));
  }, []);

  if (!info) return null;
  const line = buildLine(info);

  return (
    <button
      type="button"
      onClick={() => {
        void navigator.clipboard?.writeText(line).then(() => {
          setCopied(true);
          window.setTimeout(() => setCopied(false), 1600);
        });
      }}
      title="点一下复制，报问题时贴上"
      className="mt-2 w-fit px-1 py-1 text-left font-mono text-[10.5px] leading-relaxed transition-colors"
      style={{ color: copied ? "var(--celadon)" : "var(--ink-3)" }}
    >
      {copied ? "已复制" : line}
    </button>
  );
}
