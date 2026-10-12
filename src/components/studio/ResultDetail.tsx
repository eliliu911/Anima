import { useCallback, useEffect, useState } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { studioDeleteAsset } from "../../api/studio";
import { MediaThumb } from "./MediaThumb";
import { useStudioStore } from "./useStudioStore";
import { step, type Viewable } from "./lightboxNav";

/**
 * 结果详情。**左图右信息的双栏**，不是一张纯大图。
 *
 * 照搬 LumenX 的 `playground/DetailPanel`（MIT，Copyright 2026 Alibaba）的
 * 信息架构：媒体占 60%，右边 40% 分三段 —— 头部（模式徽章 / 短 id / 引擎名 /
 * 时间）、正文（提示词 + 参数 + 报错）、动作。
 *
 * # 动作换成我们真有的
 *
 * 它那几个按钮里有一半在我们这儿没有对应实现，**没有的就不放** ——
 * 摆一个点了没反应的按钮比没有这个按钮更糟：
 *
 * | LumenX | 我们 |
 * |---|---|
 * | 收藏到资产库 | 录入资产库（同一件事，走 `NewAssetDialog`） |
 * | Download | **在访达中显示** —— 文件本来就在本机，"下载"没有意义 |
 * | Generate Video | 用它生成视频（塞进 i2v 的首帧） |
 * | Delete | **有** —— `studio_delete_asset`，只允许删生成目录里的文件 |
 * | 精选 | 没有。我们没有「本批最好的一张」这个概念 |
 */
export function ResultDetail({
  items,
  index,
  onIndex,
  onClose,
  onEnroll,
}: {
  items: Viewable[];
  index: number;
  onIndex: (i: number) => void;
  onClose: () => void;
  onEnroll: (v: Viewable) => void;
}) {
  const cur = items[index];
  const [copied, setCopied] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  /** 删除要点两次。**这是唯一一个会不可逆动文件的动作** ——
      和「设为封面」并排放一个一点就没的按钮，迟早出事。 */
  const [confirmDelete, setConfirmDelete] = useState(false);
  const setMode = useStudioStore((s) => s.setMode);
  const setImage = useStudioStore((s) => s.setImage);
  const removeResult = useStudioStore((s) => s.removeResult);

  const onKey = useCallback(
    (e: KeyboardEvent) => {
      if (e.key === "Escape") return onClose();
      if (e.key === "ArrowLeft") return onIndex(step(index, -1, items.length));
      if (e.key === "ArrowRight") return onIndex(step(index, 1, items.length));
    },
    [index, items.length, onIndex, onClose],
  );

  useEffect(() => {
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onKey]);

  // 换一张图时把「已复制」这类瞬时状态清掉，否则会串到下一张上。
  // **确认删除也要清** —— 不清的话，在第一张上点了「删除」再翻到第二张，
  // 那张的按钮已经是「确认删除」状态了，一点就没。
  useEffect(() => {
    setCopied(false);
    setNote(null);
    setConfirmDelete(false);
  }, [index]);

  if (!cur) return null;

  const copyPrompt = () => {
    navigator.clipboard
      .writeText(cur.prompt)
      .then(() => {
        setCopied(true);
        setTimeout(() => setCopied(false), 1500);
      })
      // 剪贴板被拒不能静默 —— 用户会以为复制成功了然后粘贴出旧内容
      .catch((e) => setNote(`复制失败：${e}`));
  };

  const reveal = () => {
    revealItemInDir(cur.path).catch((e) => setNote(`打开访达失败：${e}`));
  };

  /**
   * 删掉这一条。**先删文件，再从列表里去掉。**
   *
   * 反过来的话，文件删失败（被占用、权限）时列表里已经没有了 ——
   * 用户以为删干净了，磁盘上还躺着。
   */
  const remove = async () => {
    if (!confirmDelete) {
      setConfirmDelete(true);
      return;
    }
    try {
      await studioDeleteAsset(cur.path);
      removeResult(cur.id);
      // 删完之后停在同一个位置：后面还有就显示后面那张，没有就退回画廊
      if (items.length <= 1) onClose();
      else onIndex(Math.min(index, items.length - 2));
    } catch (e) {
      setNote(`删不掉：${e}`);
      setConfirmDelete(false);
    }
  };

  const toVideo = () => {
    setMode("i2v");
    setImage(0, cur.path);
    onClose();
  };

  return (
    <>
      <div
        className="fixed inset-0 z-50 backdrop-blur-md"
        style={{ background: "color-mix(in srgb, var(--studio-bg-base) 82%, black)" }}
        onClick={onClose}
        aria-hidden="true"
      />
      <div
        role="dialog"
        aria-modal="true"
        className="fixed inset-4 z-50 flex overflow-hidden rounded-[20px] md:inset-8"
        style={{
          background: "var(--studio-bg-surface)",
          boxShadow: "0 0 0 1px var(--studio-border), 0 24px 64px -20px rgba(0,0,0,0.6)",
        }}
      >
        {/* ── 左：媒体 ── */}
        <div
          className="relative flex h-full w-[60%] items-center justify-center"
          style={{ background: "var(--studio-bg-inset)" }}
        >
          <MediaThumb
            key={cur.id}
            path={cur.path}
            alt={cur.prompt.slice(0, 60)}
            kind={cur.isVideo ? "video" : "image"}
            className="max-h-full max-w-full object-contain"
          />
          {index > 0 && (
            <Chevron dir="prev" onClick={() => onIndex(step(index, -1, items.length))} />
          )}
          {index < items.length - 1 && (
            <Chevron dir="next" onClick={() => onIndex(step(index, 1, items.length))} />
          )}
          {items.length > 1 && (
            <div
              className="absolute bottom-3 left-1/2 -translate-x-1/2 rounded-full px-3 py-1 font-mono text-[11px] tabular-nums"
              style={{ background: "var(--studio-bg-elevated)", color: "var(--studio-text-2)" }}
            >
              {index + 1} / {items.length}
            </div>
          )}
        </div>

        {/* ── 右：信息 ── */}
        <div
          className="studio-scroll relative h-full w-[40%] overflow-y-auto"
          style={{ borderLeft: "1px solid var(--studio-border-subtle)" }}
        >
          <button
            type="button"
            onClick={onClose}
            aria-label="关闭"
            title="关闭 · Esc"
            className="absolute right-4 top-4 z-10 grid h-8 w-8 place-items-center rounded-lg"
            style={{ background: "var(--studio-bg-inset)", color: "var(--studio-text-muted)" }}
          >
            ✕
          </button>

          <header
            className="px-6 pb-4 pr-14 pt-6"
            style={{ borderBottom: "1px solid var(--studio-border-subtle)" }}
          >
            <div className="mb-1.5 flex items-center gap-2">
              <span className="studio-badge">{cur.mode.toUpperCase()}</span>
              <span
                className="font-mono text-[0.5625rem] uppercase tracking-[0.1em]"
                style={{ color: "var(--studio-text-muted)" }}
              >
                {cur.id.slice(0, 8)}
              </span>
            </div>
            <h2 className="text-[19px] font-semibold leading-tight tracking-tight">
              {cur.engineId}
            </h2>
            <p
              className="mt-1.5 font-mono text-[0.625rem]"
              style={{ color: "var(--studio-text-muted)" }}
            >
              {formatTime(cur.startedAt)}
            </p>
          </header>

          <div className="flex flex-col gap-5 px-6 py-5">
            <Section
              title="PROMPT"
              action={
                <button
                  type="button"
                  onClick={copyPrompt}
                  className="rounded px-2 py-1 text-[0.625rem]"
                  style={{ color: "var(--studio-text-muted)" }}
                >
                  {copied ? "已复制" : "复制"}
                </button>
              }
            >
              <Box>
                <p className="whitespace-pre-wrap break-words text-[13px] leading-relaxed">
                  {cur.prompt || "（空）"}
                </p>
              </Box>
            </Section>

            <Section title="参数">
              <Box>
                <dl className="grid grid-cols-2 gap-x-4 gap-y-3">
                  <Field label="引擎" value={cur.engineId} wide />
                  <Field
                    label="SEED"
                    value={cur.seed === undefined ? "随机" : String(cur.seed)}
                  />
                  <Field label="用时" value={formatDuration(cur.startedAt, cur.endedAt)} />
                </dl>
              </Box>
            </Section>

            {/* 自动加了激活词就说出来 —— 提示词被动过而人不知道，
                就是我们自己造的静默失败 */}
            {cur.triggersAdded && cur.triggersAdded.length > 0 && (
              <Section title="自动加进提示词的激活词">
                <Box>
                  <p className="font-mono text-[11px]" style={{ color: "var(--studio-primary)" }}>
                    {cur.triggersAdded.join("  ")}
                  </p>
                </Box>
              </Section>
            )}
          </div>

          <div
            className="flex flex-col gap-2.5 px-6 py-4"
            style={{ borderTop: "1px solid var(--studio-border-subtle)" }}
          >
            {!cur.isVideo && (
              <button
                type="button"
                onClick={() => onEnroll(cur)}
                className="w-full rounded-full px-4 py-2.5 text-[13px] font-medium"
                style={{
                  background: "var(--studio-primary)",
                  color: "var(--studio-on-primary)",
                }}
              >
                录入资产库
              </button>
            )}
            <p className="text-[0.625rem] leading-relaxed" style={{ color: "var(--studio-text-muted)" }}>
              录入之后提示词里可以用 <code>@</code> 引用它，跨项目复用。
            </p>
            <div className="flex gap-2">
              <GhostButton onClick={reveal}>在访达中显示</GhostButton>
              {!cur.isVideo && <GhostButton onClick={toVideo}>用它生成视频</GhostButton>}
            </div>
            {/* **删除单独一行、在最下面、点两次。**
                它和上面那些不是一类动作：那些都能撤，这个不能。
                后端只允许删生成目录里的文件（`studio_delete_asset`
                拿 `studio_assets` 的结果当白名单），路径穿越也一并挡住了。 */}
            <button
              type="button"
              onClick={() => void remove()}
              className="w-full rounded-full px-3 py-2 text-[12px]"
              style={{
                background: confirmDelete ? "var(--studio-failed-bg)" : "transparent",
                color: confirmDelete ? "var(--studio-failed-fg)" : "var(--studio-text-muted)",
                boxShadow: confirmDelete
                  ? "0 0 0 1px var(--studio-failed-bd)"
                  : "0 0 0 1px var(--studio-border-subtle)",
              }}
            >
              {confirmDelete ? "确认删除？文件会一起删掉" : "删除"}
            </button>
            {note && (
              <p className="text-[11px]" style={{ color: "var(--studio-failed-fg)" }}>
                {note}
              </p>
            )}
          </div>
        </div>
      </div>
    </>
  );
}

/** `2026-09-05 17:44:52` */
export function formatTime(ms: number): string {
  const d = new Date(ms);
  const p = (n: number) => String(n).padStart(2, "0");
  return (
    `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ` +
    `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`
  );
}

/** 出这张图花了多久。还没结束就显示「—」，不要显示一个从 0 开始乱跳的数。 */
export function formatDuration(startedAt: number, endedAt?: number): string {
  if (!endedAt || endedAt < startedAt) return "—";
  const s = Math.round((endedAt - startedAt) / 1000);
  return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m${String(s % 60).padStart(2, "0")}s`;
}

function Section({
  title,
  action,
  children,
}: {
  title: string;
  action?: React.ReactNode;
  children: React.ReactNode;
}) {
  return (
    <section>
      <div className="mb-2 flex items-center justify-between">
        <h3 className="studio-eyebrow">{title}</h3>
        {action}
      </div>
      {children}
    </section>
  );
}

function Box({ children }: { children: React.ReactNode }) {
  return (
    <div
      className="rounded-[14px] p-4"
      style={{
        background: "var(--studio-bg-inset)",
        border: "1px solid var(--studio-border-subtle)",
      }}
    >
      {children}
    </div>
  );
}

function Field({ label, value, wide }: { label: string; value: string; wide?: boolean }) {
  return (
    <div className={wide ? "col-span-2" : ""}>
      <dt
        className="mb-1 font-mono text-[0.625rem] uppercase tracking-[0.08em]"
        style={{ color: "var(--studio-text-muted)" }}
      >
        {label}
      </dt>
      <dd className="break-words font-mono text-[13px]">{value}</dd>
    </div>
  );
}

function GhostButton({ onClick, children }: { onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex-1 rounded-full px-3 py-2.5 text-[12.5px] font-medium"
      style={{
        background: "var(--studio-bg-inset)",
        border: "1px solid var(--studio-border-subtle)",
        color: "var(--studio-text-2)",
      }}
    >
      {children}
    </button>
  );
}

function Chevron({ dir, onClick }: { dir: "prev" | "next"; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={dir === "prev" ? "上一张" : "下一张"}
      className={`absolute top-1/2 grid h-9 w-9 -translate-y-1/2 place-items-center rounded-full text-[18px] leading-none ${
        dir === "prev" ? "left-3" : "right-3"
      }`}
      style={{ background: "var(--studio-bg-elevated)", color: "var(--studio-text-2)" }}
    >
      {dir === "prev" ? "‹" : "›"}
    </button>
  );
}
