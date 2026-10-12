import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useStudioStore } from "./useStudioStore";
import { NewAssetDialog } from "./library/NewAssetDialog";
import { ResultCard } from "./ResultCard";
import { ResultDetail } from "./ResultDetail";
import { counts, filterResults, type FilterKey } from "./resultFilter";
import { indexOfId, viewables } from "./lightboxNav";
import { ResultQueuePanel } from "./ResultQueuePanel";

/**
 * 结果画廊。
 *
 * 结构照搬 LumenX 的 `playground/ResultGallery`（MIT，Copyright 2026 Alibaba）：
 * 顶部一条（标题 + 数量 + 全部/图片/视频 + 排列方式），下面是网格。
 *
 * **失败的卡片要留在原地并显示原因**，不是消失。这条链路的失败原因几乎都是
 * 可操作的（ComfyUI 没起、内存不够、模型缺文件），扔掉错误信息等于让人从头猜。
 */
export function ResultGallery() {
  const { t } = useTranslation(["studio", "common"]);
  const results = useStudioStore((s) => s.results);
  /** 正在录入的那一张（本机路径 + 生成它的提示词）。 */
  const [enrolling, setEnrolling] = useState<{ path: string; prompt: string } | null>(null);
  /** 正在看详情的是哪一张。**存 id 不存下标** —— 后面还会有新结果插到最前面。 */
  const [viewing, setViewing] = useState<string | null>(null);
  const [filter, setFilter] = useState<FilterKey>("all");
  const [layout, setLayout] = useState<"grid" | "large">("grid");
  const [note, setNote] = useState<string | null>(null);
  const [queueOpen, setQueueOpen] = useState(false);

  const n = counts(results);
  const shownResults = filterResults(results, filter);
  // 在跑的条数。 只按图/视频分，那是给页签用的。
  const running = results.filter((r) => r.status === "pending" || r.status === "running").length;

  // **详情的翻页范围跟着当前页签走。** 用全量的话，在「视频」页签里
  // 按 → 会翻到一张图片上 —— 看着像 bug，实际是范围搞错了。
  const gallery = viewables(shownResults);
  const detailIndex = viewing ? indexOfId(gallery, viewing) : null;

  const TABS: [FilterKey, string][] = [
    ["all", "全部"],
    ["image", "图片"],
    ["video", "视频"],
  ];

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <header
        className="flex shrink-0 flex-wrap items-center justify-between gap-3 px-6 py-3.5"
        style={{ borderBottom: "1px solid var(--studio-border-subtle)" }}
      >
        <div className="flex items-baseline gap-2">
          <span className="studio-eyebrow">RESULTS</span>
          <h2 className="text-[17px] font-semibold tracking-tight">生成结果</h2>
          <span
            className="rounded px-1.5 py-0.5 font-mono text-[10px]"
            style={{ background: "var(--studio-bg-inset)", color: "var(--studio-text-2)" }}
          >
            {shownResults.length}
          </span>
        </div>

        <div className="flex items-center gap-2">
          <Pills>
            {TABS.map(([key, label]) => (
              <Pill key={key} active={filter === key} onClick={() => setFilter(key)}>
                {label}
                <span className="ml-1 font-mono text-[10px] opacity-60">{n[key]}</span>
              </Pill>
            ))}
          </Pills>
          <Pills>
            <Pill active={layout === "grid"} onClick={() => setLayout("grid")} title="网格">
              <Icon>
                <path d="M2.5 2.5h4.2v4.2H2.5zM9.3 2.5h4.2v4.2H9.3zM2.5 9.3h4.2v4.2H2.5zM9.3 9.3h4.2v4.2H9.3z" />
              </Icon>
            </Pill>
            <Pill active={layout === "large"} onClick={() => setLayout("large")} title="大图">
              <Icon>
                <path d="M2.5 2.5h11v11h-11z" />
              </Icon>
            </Pill>
          </Pills>
          {/* 队列。**和分镜那边同一个壳**（`SidePanel`）——
              两处各写一份的话，宽度、动画、关闭方式都会对不齐。 */}
          <Pills>
            <Pill active={queueOpen} onClick={() => setQueueOpen((v) => !v)} title="队列">
              <Icon>
                <path d="M2 4h3M2 8h3M2 12h3M7 4h7M7 8h7M7 12h7" />
              </Icon>
              <span>队列</span>
              {running > 0 && (
                <span
                  className="ml-1 inline-flex min-w-[14px] items-center justify-center rounded-full px-1 text-[9px] font-semibold tabular-nums"
                  style={{ background: "var(--studio-primary)", color: "var(--studio-on-primary)" }}
                >
                  {running}
                </span>
              )}
            </Pill>
          </Pills>
        </div>
      </header>

      {queueOpen && <ResultQueuePanel onClose={() => setQueueOpen(false)} />}

      {note && (
        <p className="px-6 pt-2 text-[11px]" style={{ color: "var(--studio-failed-fg)" }}>
          {note}
        </p>
      )}

      {shownResults.length === 0 ? (
        // **「一条都没有」和「这一档没有」是两回事。** 混成一句的话，
        // 新用户会以为切个页签就能看到东西。
        <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 px-6">
          {results.length === 0 ? (
            <>
              <p className="text-[15px]" style={{ color: "var(--studio-text-2)" }}>
                {t("studio:emptyTitle", { defaultValue: "暂无生成结果" })}
              </p>
              <p className="text-[12px]" style={{ color: "var(--studio-text-muted)" }}>
                {t("studio:emptyHint", {
                  defaultValue: "写好提示词点生成，结果会出现在这里",
                })}
              </p>
            </>
          ) : (
            <p className="text-[12px]" style={{ color: "var(--studio-text-muted)" }}>
              这一档下面还没有东西 —— 切回「全部」看看。
            </p>
          )}
        </div>
      ) : (
        <div
          className={`studio-scroll grid min-h-0 flex-1 content-start gap-3 overflow-y-auto p-4 ${
            layout === "grid"
              ? "grid-cols-[repeat(auto-fill,minmax(240px,1fr))]"
              : "grid-cols-[repeat(auto-fill,minmax(min(520px,100%),1fr))]"
          }`}
        >
          {shownResults.map((r) => (
            <ResultCard
              key={r.id}
              r={r}
              onOpen={() => setViewing(r.id)}
              onEnroll={() => setEnrolling({ path: r.path!, prompt: r.prompt })}
              onError={setNote}
            />
          ))}
        </div>
      )}

      {detailIndex !== null && (
        <ResultDetail
          items={gallery}
          index={detailIndex}
          onIndex={(i) => setViewing(gallery[i]?.id ?? null)}
          onClose={() => setViewing(null)}
          onEnroll={(v) => {
            setViewing(null);
            setEnrolling({ path: v.path, prompt: v.prompt });
          }}
        />
      )}

      {enrolling && (
        <NewAssetDialog
          initialMedia={enrolling.path}
          initialDescription={enrolling.prompt}
          onClose={() => setEnrolling(null)}
          onCreated={() => setEnrolling(null)}
        />
      )}
    </div>
  );
}

function Pills({ children }: { children: React.ReactNode }) {
  return (
    <div
      className="flex shrink-0 items-center gap-0.5 rounded-full p-1"
      style={{ background: "var(--studio-bg-inset)" }}
    >
      {children}
    </div>
  );
}

function Pill({
  active,
  onClick,
  title,
  children,
}: {
  active: boolean;
  onClick: () => void;
  title?: string;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={title}
      aria-pressed={active}
      // **`inline-flex` + `whitespace-nowrap`，别用默认的行内排版。**
      // 普通 `<button>` 里的图标和文字是按行内内容走的，横向一挤就断行 ——
      // 「队列」那个图标和两个字被拆成上下两行，看着像坏了。
      className="inline-flex shrink-0 items-center gap-1 whitespace-nowrap rounded-full px-3 py-1.5 text-[12px] font-medium transition-colors"
      style={{
        background: active ? "var(--studio-bg-surface)" : "transparent",
        color: active ? "var(--studio-text)" : "var(--studio-text-muted)",
        boxShadow: active ? "0 0 0 1px var(--studio-border-subtle)" : "none",
      }}
    >
      {children}
    </button>
  );
}

function Icon({ children }: { children: React.ReactNode }) {
  return (
    <svg
      width="15"
      height="15"
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.3"
      strokeLinejoin="round"
    >
      {children}
    </svg>
  );
}
