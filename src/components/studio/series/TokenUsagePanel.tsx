/**
 * 概览页最下面：这部剧用了多少 token、花了约多少钱。
 *
 * 按「后端 · 模型」一行，下面按步骤（镜头表 / 细化时间线 / 线路 B）拆一行，最后一行是全部项目加对话的合计 ——
 * 只看本剧会漏掉 Azure 账单里别处花掉的那部分，对不上账。
 */
import type { UsageSummary, UsageTotals } from "../../../api/tokenUsage";
import { formatTokens, formatUsd, totalTokens } from "./tokenUsage";

export function TokenUsagePanel({ usage, onRefresh }: { usage: UsageSummary | null; onRefresh: () => void }) {
  const muted = { color: "var(--studio-text-muted)" };
  const cell = "px-2 py-1.5 text-right font-mono tabular-nums";
  const row = (key: string, label: string, sub: string, t: UsageTotals) => (
    <tr key={key} style={{ borderTop: "1px solid var(--studio-border-subtle)" }}>
      <td className="px-2 py-1.5 text-left">
        <span className="text-[12px]">{label}</span>
        {sub && (
          <span className="ml-1.5 text-[11px]" style={muted}>
            {sub}
          </span>
        )}
      </td>
      <td className={cell}>{t.calls.toLocaleString("en-US")}</td>
      <td className={cell}>{formatTokens(t.prompt_tokens)}</td>
      <td className={cell}>{formatTokens(t.completion_tokens)}</td>
      <td className={cell} style={muted}>
        {t.reasoning_tokens ? formatTokens(t.reasoning_tokens) : "—"}
      </td>
      <td className={cell}>{formatUsd(t) || <span style={muted}>本地</span>}</td>
    </tr>
  );

  return (
    <section className="mt-6">
      <div className="mb-2.5 flex items-baseline gap-2">
        <h2 className="text-[13px] font-medium">Token 用量</h2>
        {usage && totalTokens(usage.project) > 0 && (
          <span className="text-[11px]" style={muted}>
            本剧 {formatTokens(totalTokens(usage.project))} tok{formatUsd(usage.project) ? ` · ${formatUsd(usage.project)}` : ""}
          </span>
        )}
        <button type="button" onClick={onRefresh} className="ml-auto text-[12px]" style={{ color: "var(--studio-text-2)" }}>
          刷新
        </button>
      </div>
      {usage === null ? (
        <p className="text-[12px]" style={muted}>
          读取中…
        </p>
      ) : usage.models.length === 0 ? (
        <p className="text-[12px]" style={muted}>
          这部剧还没有记录。从这个版本起，生成镜头、细化时间线、合成提示词每调一次模型都会记在这里。
        </p>
      ) : (
        <div className="overflow-x-auto rounded-xl px-2 py-1" style={{ background: "var(--studio-bg-inset)" }}>
          <table className="w-full text-[12px]">
            <thead>
              <tr className="text-[11px]" style={muted}>
                <th className="px-2 py-1.5 text-left font-normal">后端 · 模型</th>
                <th className="px-2 py-1.5 text-right font-normal">调用</th>
                <th className="px-2 py-1.5 text-right font-normal">输入</th>
                <th className="px-2 py-1.5 text-right font-normal">输出</th>
                <th className="px-2 py-1.5 text-right font-normal">其中思考</th>
                <th className="px-2 py-1.5 text-right font-normal">估算费用</th>
              </tr>
            </thead>
            <tbody>
              {usage.models.map((m) =>
                row(`${m.provider}/${m.model}`, m.model, m.provider_label || m.provider, m.totals),
              )}
              {usage.steps.length > 1 &&
                usage.steps.map((s) => row(`step:${s.step}`, `  · ${s.step || "其它"}`, "按步骤", s.totals))}
              {row("all", "全部项目 + 对话", "和 Azure 账单对账看这一行", usage.all)}
            </tbody>
          </table>
        </div>
      )}
      <p className="mt-1.5 text-[11px] leading-relaxed" style={muted}>
        费用按 OpenAI 官方价估算（gpt-5.6 系列），Azure 以账单为准；认不出价的模型写「起」，本地模型不计费。
      </p>
    </section>
  );
}
