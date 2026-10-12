/**
 * 诊断窗口：把不可见的东西显出来。
 *
 * ## 这一页存在的理由
 *
 * 这套系统的失败**大多不报错** —— 嘴不动、没声音、动作没反应、记忆好像没生效。
 * 界面上什么都不说，日志里往往也没有，用户唯一能做的是"感觉不太对"。
 *
 * 所以这一页不判断"有没有异常"，它做的是**把当前实际生效的东西读出来**：
 * 认出了几个口型通道、语音引擎连不连得上、断句用的是 Silero 还是退回的能量 VAD。
 *
 * ## 两条规矩
 *
 * 1. **口型监视器在任何形态下都能用**，包括选中角色被锁住的时候 ——
 *    它是排查工具，锁掉它等于在最需要它的时候把手电筒收走。
 * 2. **告警要说清楚退回到了什么。** 缺东西本身不重要，重要的是缺了之后
 *    现在跑的是哪一套 —— 那才是你此刻听到、看到的东西。
 *    陶色是这一页唯一的告警色。
 */
import { useEffect, useRef, useState } from "react";

import { diagnostics, sidecarStatus, type DiagCheck, type SidecarStatus } from "../api/voice";
import { mouth, VISEMES } from "../avatar/mouth";
import { SCHEME_LABEL } from "../avatar/morphMap";
import { useFace } from "../store/face";

export function Diagnostics() {
  return (
    <div className="flex flex-col gap-4 overflow-auto">
      <SidecarsSection />
      <MouthMonitor />
      <Checks />
    </div>
  );
}

/**
 * 本地 Python Sidecars 进程监控
 */
function SidecarsSection() {
  const [sidecars, setSidecars] = useState<SidecarStatus[]>([]);
  const [loading, setLoading] = useState(true);

  const refresh = () => {
    setLoading(true);
    sidecarStatus()
      .then(setSidecars)
      .catch(() => {})
      .finally(() => setLoading(false));
  };

  useEffect(refresh, []);

  return (
    <section className="flex flex-col gap-2">
      <header className="flex items-center justify-between">
        <div className="flex items-center gap-1.5">
          <span className="text-[12.5px] text-[var(--ink)] font-medium">本地 Python Sidecar 守护状态</span>
          <span className="text-[10px] text-[var(--ink-3)]">(启动时由 Rust 自动拉起)</span>
        </div>
        <button
          type="button"
          onClick={refresh}
          disabled={loading}
          className="text-[11px] px-2 py-0.5 rounded-lg border border-[var(--line)] bg-[var(--surface-2)] text-[var(--ink-2)] hover:text-[var(--ink)] cursor-pointer disabled:opacity-50"
        >
          {loading ? "…" : "刷新"}
        </button>
      </header>

      <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
        {sidecars.map((sc) => {
          const isOnline = sc.status === "online" || sc.status === "external";
          return (
            <div
              key={sc.id}
              className="flex items-center justify-between p-2.5 rounded-xl bg-[var(--surface-3)] border border-[var(--line)] text-[12px]"
            >
              <div className="flex flex-col min-w-0 pr-2">
                <span className="font-semibold text-[var(--ink)] truncate">{sc.name}</span>
                <span className="text-[10.5px] text-[var(--ink-3)] font-mono">
                  端口 {sc.port} {sc.pid ? `· PID ${sc.pid}` : ""}
                </span>
              </div>
              <span
                className={`px-2 py-0.5 rounded-full text-[10.5px] font-medium shrink-0 flex items-center gap-1 ${
                  isOnline
                    ? "bg-emerald-500/15 text-emerald-400 border border-emerald-500/30"
                    : sc.status === "starting"
                    ? "bg-amber-500/15 text-amber-400 border border-amber-500/30 animate-pulse"
                    : "bg-zinc-500/15 text-zinc-400 border border-zinc-500/30"
                }`}
              >
                <span
                  className={`w-1.5 h-1.5 rounded-full ${
                    isOnline ? "bg-emerald-400" : sc.status === "starting" ? "bg-amber-400" : "bg-zinc-400"
                  }`}
                />
                {isOnline ? "运行中" : sc.status === "starting" ? "启动中" : "未启动"}
              </span>
            </div>
          );
        })}
      </div>
    </section>
  );
}

/**
 * 实时口型监视器。
 *
 * 柱子在跳 = 文本/音频到 viseme 这条链路是通的；模型仍然不动，
 * 那就是 morph target 命名不认识 —— 两种情况的修法完全不同，
 * 所以要能一眼分开。
 */
function MouthMonitor() {
  const bars = useRef<Array<HTMLElement | null>>([]);
  const report = useFace((s) => s.report);

  // **直接写 style，不走 React state。** 这是每帧 60 次的更新，
  // 走 setState 会把整个窗口重渲染 60 次
  useEffect(() => {
    let raf = 0;
    const tick = () => {
      const w = mouth.sample();
      VISEMES.forEach((v, i) => {
        const el = bars.current[i];
        if (el) el.style.height = `${Math.max(2, w[v] * 100)}%`;
      });
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, []);

  return (
    <section className="flex flex-col gap-2">
      <header className="flex items-center justify-between">
        <span className="text-[12.5px] text-[var(--ink)] font-medium">口型监视器</span>
        <ChannelChip report={report} />
      </header>

      <div className="flex flex-col bg-[var(--surface-3)] border border-[var(--line)] rounded-xl p-3">
        <div className="flex items-end h-14 gap-[3px]" aria-hidden="true">
          {VISEMES.map((v, i) => (
            <i
              key={v}
              ref={(el) => {
                bars.current[i] = el;
              }}
              className="flex-1 bg-[var(--celadon)] rounded-t-[2px] min-w-[2px]"
              style={{ height: "2%" }}
            />
          ))}
        </div>
        <div className="flex mt-1 border-t border-dashed border-[var(--line)] pt-1" aria-hidden="true">
          {VISEMES.map((v) => (
            <span key={v} className="flex-1 text-center text-[9px] font-mono text-[var(--ink-3)]">
              {v}
            </span>
          ))}
        </div>
      </div>

      <p className="m-0 text-[11px] leading-snug text-[var(--ink-3)]">
        说话或试听时柱子在跳，说明文本到口型这条链路是通的。柱子在跳而模型的嘴不动 ——
        那是 morph target 命名不认识，把名字加进 <code className="font-mono">morphMap.ts</code> 即可。
      </p>
    </section>
  );
}

/**
 * 「认出几个通道」这个数字。
 *
 * 没有 report 不等于没问题 —— 也可能是根本没加载过 3D/Live2D 模型
 * （电子云形态就没有口型通道这回事）。**这两种情况必须分开说**，
 * 否则用户会把"还没加载"当成"识别失败"。
 */
function ChannelChip({ report }: { report: ReturnType<typeof useFace.getState>["report"] }) {
  if (!report) {
    return <span className="text-[10.5px] text-[var(--ink-3)]">未加载模型</span>;
  }
  const warn = report.channels === 0;
  return (
    <span
      className={`text-[10.5px] px-2 py-0.5 rounded-full border ${
        warn
          ? "text-[var(--clay)] border-[var(--clay)]/30 bg-[color-mix(in_srgb,var(--clay)_16%,transparent)]"
          : "text-[var(--celadon)] border-[var(--celadon)]/30 bg-[var(--celadon-2)]"
      }`}
    >
      {report.channels} / {VISEMES.length} 通道 · {SCHEME_LABEL[report.scheme]}
    </span>
  );
}

/** 后端那几项。判断逻辑在 Rust 侧，这里只负责显示 */
function Checks() {
  const [checks, setChecks] = useState<DiagCheck[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const load = () => {
    setLoading(true);
    diagnostics()
      .then((c) => {
        setChecks(c);
        setError(null);
      })
      .catch((e) => setError(String(e)))
      .finally(() => setLoading(false));
  };

  useEffect(load, []);

  return (
    <section className="flex flex-col gap-2">
      <header className="flex items-center justify-between">
        <span className="text-[12.5px] text-[var(--ink)] font-medium">当前实际生效的</span>
        <button
          type="button"
          onClick={load}
          disabled={loading}
          className="text-[11px] px-2 py-1 rounded-lg border border-[var(--line)] bg-[var(--surface-2)] text-[var(--ink-2)] hover:text-[var(--ink)] hover:border-[var(--ink-3)] disabled:opacity-50 transition-colors cursor-pointer"
        >
          {loading ? "检查中…" : "重新检查"}
        </button>
      </header>

      {error && <Row label="体检本身失败了" value={error} level="warn" />}

      {/* 探活可能要等好几秒，先给个占位，别让窗口空着 */}
      {!checks && loading && (
        <p className="m-0 text-[11.5px] text-[var(--ink-3)]">正在读配置、探引擎…（引擎没反应时最多等 6 秒）</p>
      )}

      <div className="flex flex-col gap-1.5">
        {checks?.map((c) => (
          <Row key={c.id} label={c.label} value={c.value} level={c.level} detail={c.detail} />
        ))}
      </div>
    </section>
  );
}

function Row({
  label,
  value,
  level,
  detail,
}: {
  label: string;
  value: string;
  level: "ok" | "warn";
  detail?: string;
}) {
  const warn = level === "warn";
  return (
    <div className="flex flex-col gap-1 py-1.5 border-b border-[var(--line)] last:border-b-0">
      <div className="flex items-baseline justify-between gap-3">
        <span className="text-[11.5px] text-[var(--ink-3)] shrink-0">{label}</span>
        <span
          className={`text-[11px] px-2 py-0.5 rounded-full text-right ${
            warn
              ? "text-[var(--clay)] bg-[color-mix(in_srgb,var(--clay)_16%,transparent)]"
              : "text-[var(--ink-2)] bg-[var(--surface-2)]"
          }`}
        >
          {value}
        </span>
      </div>
      {/* 后果，不是修复步骤 —— 用户是带着症状来看这一页的 */}
      {detail && <p className="m-0 text-[11px] leading-snug text-[var(--ink-3)] pr-1">{detail}</p>}
    </div>
  );
}
