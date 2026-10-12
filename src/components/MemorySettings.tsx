/**
 * 长期记忆的浏览与编辑。
 *
 * ## 为什么这个面板必须存在
 *
 * 之前只有两个能力：按关键词召回、以及清空全部。那意味着"它记住了我什么"这个
 * 问题**没法回答** —— 你得先猜一个查询词。而记忆系统不可见时，用户无法建立
 * 信任：它是不是记错了什么、有没有记了不该记的，都只能靠猜。
 *
 * 所以这里做三件事：列出来、删掉错的、手动补上漏的。
 */
import { useCallback, useEffect, useState } from "react";
import {
  memoryAddFact,
  memoryForgetFact,
  memoryListFacts,
  memoryStats,
  memoryClearFacts,
  type MemoryFact,
  type MemoryStats,
} from "../api/voice";

/** 一次翻多少条。这张表通常几百条，一屏能翻完 */
const PAGE = 50;

function when(unix: number): string {
  const d = new Date(unix * 1000);
  const now = Date.now();
  const days = Math.floor((now - d.getTime()) / 86_400_000);
  if (days === 0) return "今天";
  if (days === 1) return "昨天";
  if (days < 30) return `${days} 天前`;
  return d.toLocaleDateString();
}

export function MemorySettings() {
  const [facts, setFacts] = useState<MemoryFact[]>([]);
  const [stats, setStats] = useState<MemoryStats | null>(null);
  const [offset, setOffset] = useState(0);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  // 删除要二次确认，但**不要用系统弹窗** —— 那会打断整个界面，
  // 而删一条记忆是个小操作
  const [confirming, setConfirming] = useState<string | null>(null);

  const reload = useCallback(
    async (at = offset) => {
      setBusy(true);
      try {
        const [list, s] = await Promise.all([
          memoryListFacts(PAGE, at),
          memoryStats(),
        ]);
        setFacts(list);
        setStats(s);
        setOffset(at);
        setMessage(null);
      } catch (err) {
        setMessage(`${err}`);
      } finally {
        setBusy(false);
      }
    },
    [offset],
  );

  useEffect(() => {
    void reload(0);
    // 只在挂载时拉一次
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  async function forget(id: string) {
    setBusy(true);
    try {
      await memoryForgetFact(id);
      // 本地先摘掉，不等重新拉 —— 删一条却要等一次往返会显得很钝
      setFacts((prev) => prev.filter((f) => f.id !== id));
      setStats((s) => (s ? { ...s, facts: Math.max(0, s.facts - 1) } : s));
      setConfirming(null);
    } catch (err) {
      setMessage(`${err}`);
    } finally {
      setBusy(false);
    }
  }

  async function add() {
    const text = draft.trim();
    if (!text) return;
    setBusy(true);
    try {
      await memoryAddFact("", text);
      setDraft("");
      await reload(0);
      setMessage("已记住");
    } catch (err) {
      setMessage(`${err}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <div className="text-[14px] font-semibold text-[var(--ink)] mb-1">长期记忆</div>

      {stats && (
        <p className="text-[12.5px] text-[var(--ink-3)] leading-relaxed bg-[var(--surface-2)] p-3 rounded-lg m-0">
          记住 <strong className="text-[var(--ink)]">{stats.facts}</strong> 条事实，来自 <strong className="text-[var(--ink)]">{stats.sessions}</strong> 次对话、
          <strong className="text-[var(--ink)]">{stats.messages}</strong> 条消息。
          事实是后台模型从对话里提取的，会自动去重 ——
          <strong className="text-[var(--ink)]">但它会记错</strong>，所以这里能删。
        </p>
      )}

      {/* 手动补：提取是模型做的，会漏。用户明确想让它记住某件事的时候，
          不该只能等模型下次刚好提取到 */}
      <div className="flex items-center gap-2 bg-[var(--surface-2)] border border-[var(--line)] rounded-lg p-2">
        <span className="text-[12px] text-[var(--ink-3)] shrink-0 whitespace-nowrap">手动补记</span>
        <input
          className="flex-1 bg-transparent border-none outline-none text-[12.5px] text-[var(--ink)] min-w-0"
          value={draft}
          placeholder="比如「我不喝咖啡」"
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.nativeEvent.isComposing) void add();
          }}
        />
        <button
          type="button"
          className="px-3 py-1.5 text-[12px] bg-[var(--surface)] border border-[var(--line)] rounded-md text-[var(--ink-2)] hover:text-[var(--ink)] hover:border-[var(--ink-3)] transition-colors disabled:opacity-50"
          disabled={busy || !draft.trim()}
          onClick={() => void add()}
        >
          记住
        </button>
      </div>

      {facts.length === 0 && !busy && (
        <div className="text-[12.5px] text-[var(--ink-3)] text-center py-4 border border-dashed border-[var(--line)] rounded-lg bg-[var(--surface-2)]">
          还没记住任何事实。聊几轮之后后台会自动提取 ——
          只有"值得长期记住"的才会进来，闲聊不会。
        </div>
      )}

      <div className="flex flex-col gap-2">
        {facts.map((f) => (
          <div className="flex flex-col gap-1 p-3 border border-[var(--line)] rounded-lg bg-[var(--surface-3)] relative group hover:border-[var(--ink-3)] transition-colors" key={f.id}>
            <div className="text-[12.5px] font-semibold text-[var(--ink)] pr-[80px] whitespace-normal">
              {f.fact}
            </div>
            <div className="text-[11.5px] text-[var(--ink-3)]">
              {when(f.created_at)} · {f.subject}
            </div>
            <div className="absolute top-[9px] right-[11px] flex gap-1.5 opacity-0 group-hover:opacity-100 focus-within:opacity-100 transition-opacity">
              {confirming === f.id ? (
                <>
                  <button
                    type="button"
                    className="px-2 py-1 text-[11px] bg-[var(--surface-2)] border border-[color-mix(in_srgb,var(--clay)_32%,transparent)] rounded shadow-sm text-[var(--clay)] hover:bg-[var(--surface)]"
                    disabled={busy}
                    onClick={() => void forget(f.id)}
                  >
                    确认删除
                  </button>
                  <button
                    type="button"
                    className="px-2 py-1 text-[11px] bg-[var(--surface-2)] border border-[var(--line)] rounded shadow-sm text-[var(--ink)] hover:bg-[var(--surface)]"
                    onClick={() => setConfirming(null)}
                  >
                    取消
                  </button>
                </>
              ) : (
                <button
                  type="button"
                  className="px-2 py-1 text-[11px] bg-[var(--surface-2)] border border-[var(--line)] rounded shadow-sm text-[var(--ink)] hover:bg-[var(--surface)]"
                  onClick={() => setConfirming(f.id)}
                >
                  删除
                </button>
              )}
            </div>
          </div>
        ))}
      </div>

      {(offset > 0 || facts.length === PAGE) && (
        <div className="flex items-center justify-between border-t border-[var(--line)] pt-3 mt-1">
          <button
            type="button"
            className="px-3 py-1 text-[12px] bg-[var(--surface-2)] border border-[var(--line)] rounded text-[var(--ink-2)] hover:text-[var(--ink)] hover:border-[var(--ink-3)] transition-colors disabled:opacity-50"
            disabled={busy || offset === 0}
            onClick={() => void reload(Math.max(0, offset - PAGE))}
          >
            上一页
          </button>
          <span className="text-[11.5px] text-[var(--ink-3)] font-mono">
            {offset + 1} – {offset + facts.length}
          </span>
          <button
            type="button"
            className="px-3 py-1 text-[12px] bg-[var(--surface-2)] border border-[var(--line)] rounded text-[var(--ink-2)] hover:text-[var(--ink)] hover:border-[var(--ink-3)] transition-colors disabled:opacity-50"
            disabled={busy || facts.length < PAGE}
            onClick={() => void reload(offset + PAGE)}
          >
            下一页
          </button>
        </div>
      )}

      {/* 清空放在最后、样式最弱 —— 它是最危险的操作，不该抢眼 */}
      {stats && stats.facts > 0 && (
        <div className="flex justify-center mt-4">
          {confirming === "__all__" ? (
            <div className="text-[12.5px] text-[var(--clay)] bg-[color-mix(in_srgb,var(--clay)_10%,transparent)] border border-[color-mix(in_srgb,var(--clay)_32%,transparent)] p-3 rounded-lg flex flex-col items-center gap-3 w-full">
              要删掉全部 {stats.facts} 条记忆吗？这个动作不可撤销。
              <div className="flex gap-2">
                <button
                  type="button"
                  className="px-3 py-1.5 text-[12px] bg-[var(--clay)] text-white font-semibold rounded hover:bg-[#ff4d4d] disabled:opacity-50"
                  disabled={busy}
                  onClick={async () => {
                    setBusy(true);
                    try {
                      await memoryClearFacts();
                      setConfirming(null);
                      await reload(0);
                    } catch (err) {
                      setMessage(`${err}`);
                    } finally {
                      setBusy(false);
                    }
                  }}
                >
                  确认清空
                </button>
                <button
                  type="button"
                  className="px-3 py-1.5 text-[12px] bg-[var(--surface)] border border-[var(--line)] rounded text-[var(--ink-2)] hover:text-[var(--ink)] hover:border-[var(--ink-3)] transition-colors"
                  onClick={() => setConfirming(null)}
                >
                  取消
                </button>
              </div>
            </div>
          ) : (
            <button
              type="button"
              className="text-[12px] text-[var(--ink-3)] hover:text-[var(--clay)] transition-colors underline decoration-[var(--line)] hover:decoration-[var(--clay)] underline-offset-4 pb-1"
              onClick={() => setConfirming("__all__")}
            >
              清空全部记忆
            </button>
          )}
        </div>
      )}

      {message && <div className="text-[12px] text-[var(--amber)] bg-[color-mix(in_srgb,var(--amber)_10%,transparent)] px-3 py-2 rounded-md mt-2">{message}</div>}
    </div>
  );
}
