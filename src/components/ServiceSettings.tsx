/**
 * 设置 → 服务：这台机器上 Anima 依赖的东西，现在谁在跑。
 *
 * # 为什么值得单独一页
 *
 * 这份信息原来只有 `anima status` 打得出来 —— **只有开着终端的人看得到**。
 * 而它恰恰是排查的第一步：出图失败多半是 ComfyUI 没开，语音没声音多半是
 * Kokoro 没起，「模型列表空的」多半是 LM Studio 没开。
 *
 * # 「离线」不等于「有问题」
 *
 * CosyVoice2 没开是完全正常的状态。全部标红会让人去修不存在的问题，
 * 而真正该开没开的那个反而淹没在红色里。所以分三档：
 * 在线（绿）/ 该开没开（黄）/ 没开也没事（灰）。
 *
 * # 每一条都写「离线会怎么样」
 *
 * 这是这一页最有用的一列。只说「离线」没有信息量 ——
 * 人要知道的是「那我现在还能不能出图」。
 */
import { useCallback, useEffect, useState } from "react";
import { serviceStatus, type ServiceStatus } from "../api/services";
import { byGroup, levelOf, summary } from "./serviceView";

const DOT: Record<string, string> = {
  ok: "bg-emerald-500",
  warn: "bg-amber-500",
  idle: "bg-[var(--line-2)]",
};

export function ServiceSettings() {
  const [list, setList] = useState<ServiceStatus[]>([]);
  const [busy, setBusy] = useState(false);
  const [at, setAt] = useState<string>("");

  const refresh = useCallback(async () => {
    setBusy(true);
    try {
      setList(await serviceStatus());
      setAt(new Date().toLocaleTimeString());
    } finally {
      setBusy(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
    // **不自动轮询。** 每次探测要连十个端口，后台一直跑没有意义 ——
    // 人是打开这一页才关心它的
  }, [refresh]);

  const sum = summary(list);

  return (
    <div className="flex flex-col gap-5">
      <div>
        <div className="flex flex-wrap items-center gap-2.5">
          <h2 className="text-[15px] font-bold text-[var(--ink)]">服务</h2>
          <button
            type="button"
            className="rounded-xl border border-[var(--line)] bg-[var(--surface-2)] px-3 py-1 text-[12px] hover:bg-[var(--surface-3)] disabled:opacity-40"
            disabled={busy}
            onClick={() => void refresh()}
          >
            {busy ? "探测中…" : "重新探测"}
          </button>
          <span className={`text-[12px] ${sum.bad > 0 ? "text-amber-600" : "text-[var(--ink-3)]"}`}>{sum.text}</span>
          {at && <span className="text-[11px] text-[var(--ink-3)] opacity-60">{at}</span>}
        </div>
        <p className="mt-1 text-[11.5px] text-[var(--ink-3)]">
          判据是<b>端口上有没有人在听</b>，不是 HTTP 打不打得通 —— 各家健康路径不一样，
          按路径查会因为对方改了路由就误报离线。
        </p>
      </div>

      {byGroup(list).map((g) => (
        <section key={g.key}>
          <p className="mb-2 text-[11px] font-bold uppercase tracking-wide text-[var(--ink-3)]">{g.label}</p>
          <div className="flex flex-col gap-1.5">
            {g.items.map((s) => {
              const lv = levelOf(s);
              return (
                <div
                  key={s.id}
                  className="rounded-xl border border-[var(--line)] bg-[var(--surface-2)] px-3 py-2.5"
                >
                  <div className="flex flex-wrap items-center gap-2">
                    <span className={`h-2 w-2 shrink-0 rounded-full ${DOT[lv]}`} />
                    <span className="text-[13px] font-semibold text-[var(--ink)]">{s.name}</span>
                    <span className="font-mono text-[11px] text-[var(--ink-3)]">:{s.port}</span>
                    <span
                      className={`text-[11px] ${
                        lv === "ok" ? "text-emerald-600" : lv === "warn" ? "text-amber-600" : "text-[var(--ink-3)]"
                      }`}
                    >
                      {s.online ? "在线" : s.optional ? "没开（不影响）" : "该开没开"}
                    </span>
                  </div>
                  <p className="mt-1 text-[11.5px] leading-snug text-[var(--ink-2)]">{s.what}</p>
                  {/* **离线会怎么样** —— 在线时不显示，那时它是噪音 */}
                  {!s.online && (
                    <p className="mt-0.5 text-[11.5px] leading-snug text-[var(--ink-3)]">
                      不开的话：{s.degraded}
                      {s.how && <span className="ml-1 opacity-75">· {s.how}</span>}
                    </p>
                  )}
                </div>
              );
            })}
          </div>
        </section>
      ))}

      {list.length === 0 && !busy && (
        <p className="text-[12px] text-[var(--ink-3)]">探测不出来 —— 后端没起来？</p>
      )}
    </div>
  );
}
