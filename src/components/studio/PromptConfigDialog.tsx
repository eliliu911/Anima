/**
 * 提示词配置。
 *
 * # 为什么把这些外置
 *
 * 从剧本抽角色、把草稿改写成引擎吃的提示词、读剧本推荐风格 ——
 * 都是「AI 把我的话改成了别的话」。埋在代码里的话，用户既看不见它改了
 * 什么，也没法调。这里让每一段都能读、能改、能一键复原。
 *
 * # 模型不在这里选
 *
 * 走「设置 → 模型中枢 → 链路分配」里的「后台推理」那一条。在这儿再放一个选择器就是第二份真相 ——
 * 两处不一致时没人知道到底用的哪个。
 */
import { useEffect, useState } from "react";
import {
  promptSaveSlot,
  promptSlots,
  promptTry,
  type PromptSlotView,
  type TryOutcome,
} from "../../api/prompts";

export function PromptConfigDialog({ onClose }: { onClose: () => void }) {
  const [slots, setSlots] = useState<PromptSlotView[] | null>(null);
  const [activeKey, setActiveKey] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [trying, setTrying] = useState(false);
  const [tryOut, setTryOut] = useState<TryOutcome | null>(null);
  const [sample, setSample] = useState("老茶师在窗边擦茶盏，雨天午后");

  useEffect(() => {
    promptSlots()
      .then((s) => {
        setSlots(s);
        setActiveKey((k) => k ?? s[0]?.key ?? null);
      })
      .catch((e) => setMessage(String(e)));
  }, []);

  const active = slots?.find((s) => s.key === activeKey) ?? null;

  useEffect(() => {
    if (!active) return;
    // 没改过就把默认灌进编辑框 —— 空白框会让人以为「这里本来就没提示词」。
    setDraft(active.custom_text || active.default_text);
    setTryOut(null);
    setMessage(null);
  }, [active?.key, active?.custom_text, active?.default_text]);

  const dirty = !!active && draft !== (active.custom_text || active.default_text);
  const isDefault = !!active && draft.trim() === active.default_text.trim();

  const save = async (text: string) => {
    if (!active) return;
    setBusy(true);
    setMessage(null);
    try {
      setSlots(await promptSaveSlot(active.key, text));
      setMessage(text.trim() ? "已保存" : "已恢复默认");
    } catch (e) {
      setMessage(String(e));
    } finally {
      setBusy(false);
    }
  };

  const runTry = async () => {
    if (!active) return;
    setTrying(true);
    setTryOut(null);
    setMessage(null);
    try {
      // **试跑不落盘，而且永远传编辑框里那一份。**
      //
      // 两个坑，都是 Codex Astra 的审查抓到的（2026-09-19）：
      //
      // 1. 原来是「先存再跑，不然验的是旧的」—— 那样用户点一下「试跑」，
      //    **全局正式配置就已经改了**，报错了也留着。所以改成把 draft
      //    传过去，后端在内存里覆盖。
      // 2. 第一版又写成「和默认一样就传 undefined」—— 于是**已保存过
      //    自定义模板**的槽位，用户把编辑框改回默认正文、点试跑，
      //    后端读不到 draft 就去读那份旧覆盖，跑的和屏幕上显示的不是
      //    同一份，而且看不出来。现在 draft 恒传：空串 = 用内置默认。
      setTryOut(await promptTry(active.key, sample, draft));
    } catch (e) {
      setMessage(String(e));
    } finally {
      setTrying(false);
    }
  };

  return (
    <div
      className="studio-scrim fixed inset-0 z-50 flex items-center justify-center"
    >
      <div
        className="studio-modal flex h-[80vh] w-[860px] max-w-[94vw] overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        {/* 左：槽位列表 */}
        <aside
          className="flex w-[230px] shrink-0 flex-col overflow-y-auto py-3"
          style={{ borderRight: "1px solid var(--studio-border-subtle)" }}
        >
          <p className="studio-eyebrow px-4 pb-2">提示词槽位</p>
          {slots?.map((s) => (
            <button
              key={s.key}
              type="button"
              onClick={() => setActiveKey(s.key)}
              className="mx-2 flex items-center gap-1.5 rounded-lg px-3 py-2 text-left"
              style={{
                background: s.key === activeKey ? "var(--studio-bg-inset)" : "transparent",
              }}
            >
              <span className="min-w-0 flex-1 truncate text-[12px]">{s.label}</span>
              {s.overridden && (
                <span
                  className="shrink-0 text-[9px]"
                  style={{ color: "var(--studio-primary)" }}
                  title="已自定义"
                >
                  ●
                </span>
              )}
            </button>
          ))}
          <p
            className="mt-auto px-4 pt-4 text-[11px] leading-relaxed"
            style={{ color: "var(--studio-text-muted)" }}
          >
            模型走「设置 → 模型中枢 → 链路分配」里的「后台推理」那一条。这里只管说什么，不管找谁说。
          </p>
        </aside>

        {/* 右：编辑 */}
        <div className="flex min-w-0 flex-1 flex-col">
          <header
            className="flex shrink-0 items-start justify-between px-6 pb-3 pt-5"
            style={{ borderBottom: "1px solid var(--studio-border-subtle)" }}
          >
            <div className="min-w-0">
              <h2 className="truncate text-[16px] font-semibold">
                {active?.label ?? "提示词配置"}
              </h2>
              {active && (
                <p className="mt-1 font-mono text-[0.55rem]" style={{ color: "var(--studio-text-muted)" }}>
                  占位符：{active.placeholders.join("  ")}
                </p>
              )}
              {/*
                {METHOD} 背后是一整份技能原文，编辑框里只有这一个词 ——
                不写明的话，会以为这几行就是全部规则，改了没效果也不知道为什么。
              */}
              {active?.method_source && (
                <p className="mt-1 text-[11px] leading-relaxed" style={{ color: "var(--studio-text-2)" }}>
                  拍法规则不在这段里：<span className="font-mono">{"{METHOD}"}</span> 运行时换成
                  「{active.method_source}」的原文，那部分编译在程序里，这里改不了。
                  改模板时留着它；删掉就是不用这套方法，全靠你这段自己写。
                </p>
              )}
            </div>
            <button type="button" onClick={onClose} className="px-1 text-[15px] opacity-60">
              ✕
            </button>
          </header>

          <div className="flex min-h-0 flex-1 flex-col gap-2 px-6 py-4">
            <textarea
              value={draft}
              onChange={(e) => setDraft(e.target.value)}
              spellCheck={false}
              className="min-h-0 flex-1 resize-none rounded-xl px-3.5 py-3 font-mono text-[11.5px] leading-relaxed outline-none"
              style={field}
            />

            <div className="flex flex-wrap items-center gap-2">
              <button
                type="button"
                disabled={busy || !dirty}
                onClick={() => void save(isDefault ? "" : draft)}
                className="rounded-lg px-3.5 py-2 text-[12px] font-medium"
                style={{
                  background: dirty ? "var(--studio-primary)" : "var(--studio-bg-inset)",
                  color: dirty ? "var(--studio-on-primary)" : "var(--studio-text-muted)",
                }}
              >
                保存
              </button>
              <button
                type="button"
                disabled={busy || !active?.overridden}
                onClick={() => void save("")}
                title="丢掉自定义，回到内置默认"
                className="rounded-lg px-3.5 py-2 text-[12px]"
                style={{ background: "var(--studio-bg-inset)", color: "var(--studio-text-2)" }}
              >
                恢复默认
              </button>
              <div className="ml-auto flex items-center gap-2">
                <input
                  value={sample}
                  onChange={(e) => setSample(e.target.value)}
                  placeholder="试跑用的输入"
                  className="w-[220px] rounded-lg px-2.5 py-1.5 text-[11.5px] outline-none"
                  style={field}
                />
                <button
                  type="button"
                  disabled={trying}
                  onClick={() => void runTry()}
                  title="用当前提示词真跑一次，看模型返回什么"
                  className="rounded-lg px-3 py-1.5 text-[12px]"
                  style={{ background: "var(--studio-bg-inset)", color: "var(--studio-text-2)" }}
                >
                  {trying ? "跑中…" : "试一下"}
                </button>
              </div>
            </div>

            {message && (
              <p className="text-[11.5px]" style={{ color: "var(--studio-text-2)" }}>
                {message}
              </p>
            )}

            {/*
              试跑结果分两层：**先说正式链路吃不吃得下，再给原文**。
              只给原文的话，「试跑返回了内容」会被当成「正式合成能过」——
              而真正会拦住的是下面这些解析器。
            */}
            {tryOut !== null && (
              <div
                className="max-h-[180px] shrink-0 overflow-y-auto rounded-lg px-3 py-2 text-[10.5px] leading-relaxed"
                style={{ background: "var(--studio-bg-inset)" }}
              >
                <p
                  className="studio-eyebrow mb-1"
                  style={{
                    color: !tryOut.checked
                      ? "var(--studio-text-2)"
                      : tryOut.issues.length
                        ? "var(--studio-danger)"
                        : "var(--studio-done-fg)",
                  }}
                >
                  {!tryOut.checked
                    ? "这一槽没有生产解析器可对 —— 只能看输出，验不了"
                    : tryOut.issues.length
                      ? `正式链路解析不过（${tryOut.issues.length} 条）`
                      : "正式链路解析得过"}
                </p>
                {tryOut.issues.map((issue, i) => (
                  <p key={i} style={{ color: "var(--studio-text)" }}>
                    · {issue}
                  </p>
                ))}
                <p className="studio-eyebrow mb-1 mt-2">模型原样返回</p>
                <pre className="whitespace-pre-wrap break-all font-mono">{tryOut.raw}</pre>
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

const field: React.CSSProperties = {
  background: "var(--studio-bg-inset)",
  border: "1px solid var(--studio-border-subtle)",
  color: "var(--studio-text)",
};
