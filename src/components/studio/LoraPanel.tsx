import { useEffect, useState } from "react";
import { isBakedIn } from "../../canvas/utils/collectLoras";
import { useTranslation } from "react-i18next";
import { foldSummary, shouldFold } from "./loraHintFold";
import {
  loraHint,
  setLoraNote,
  studioLoras,
  type EngineSpec,
  type LoraOption,
} from "../../api/studio";
import { useStudioStore } from "./useStudioStore";
import { ComposingInput } from "../ComposingInput";

/** 勾上时给的初始权重。 */
export const LORA_DEFAULT = 0.8;
/** 滑杆上限。再往上大多是过拟合出油画感，不是"更像"。 */
export const LORA_MAX = 1.5;
export const LORA_STEP = 0.05;

/**
 * 权重显示成固定两位。
 *
 * **不能直接把数字塞进 JSX。** 步进是 0.05，浮点累加会冒出
 * `0.7000000000000001` 这种值 —— 显示成一长串不说，宽度还会跳，
 * 滑杆跟着左右晃。两位定宽 + `tabular-nums` 才是稳的。
 */
export function formatStrength(v: number): string {
  return v.toFixed(2);
}

/**
 * 风格化（LoRA）。**这是我们比 LumenX 多的一块** —— 它没有。
 *
 * # 为什么必须按架构过滤
 *
 * LoRA 挂到不认识它的引擎上是「0 bypass adapters, 0 inj」——
 * 图照出、任务照样成功，只是那个风格**一点没生效**。所以不匹配的
 * 在这里直接不列出来，别给人选错的机会。架构是读 safetensors 的
 * 张量键名认的，不看文件名（目录名会进枚举名，只看名字会误判）。
 *
 * # 激活词
 *
 * 本机 13 个 LoRA 没有一个带 `trigger_words` 字段，激活词是从
 * `ss_tag_frequency` 里**猜**的。所以这里把猜到的显示出来、让人能关掉，
 * 提交时才并进提示词并告诉用户加了什么 —— 静默改提示词是我们自己
 * 造的静默失败。
 */
export interface LoraPanelBinding {
  loras: { name: string; strength: number }[];
  setLora: (name: string, strength: number | null) => void;
  injectTriggers: boolean;
  setInjectTriggers: (enabled: boolean) => void;
}
export function LoraPanel({ engine, binding }: { engine: EngineSpec; binding?: LoraPanelBinding }) {
  const { t } = useTranslation(["studio", "common"]);
  const [all, setAll] = useState<LoraOption[] | null>(null);
  /** 正在改说明的是哪一条，以及改到哪儿了。null = 都没在改。 */
  const [editing, setEditing] = useState<{ name: string; draft: string } | null>(null);
  /** 展开了全文说明的那几条。说明现在带实测数据和出处，全铺开列表会长到没法扫。 */
  const [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [err, setErr] = useState<string | null>(null);
  const studio = useStudioStore();
  const { loras, setLora, injectTriggers, setInjectTriggers } = binding ?? studio;

  useEffect(() => {
    studioLoras().then(setAll).catch((e) => setErr(String(e)));
  }, []);

  if (err) {
    return (
      <p className="text-[11px]" style={{ color: "var(--studio-failed-fg)" }}>
        {err}
      </p>
    );
  }
  if (!all) {
    return (
      <p className="text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
        {t("studio:loading", { defaultValue: "读取中…" })}
      </p>
    );
  }

  // 只列这个引擎能用的 —— 架构不匹配挂上去不生效且不报错
  const usable = all.filter((l) => l.engines.includes(engine.id));
  if (usable.length === 0) {
    return (
      <p className="text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
        {t("studio:noLora", {
          engine: engine.label,
          defaultValue: "没有适配 {{engine}} 的 LoRA",
        })}
      </p>
    );
  }

  const picked = (name: string) => loras.find((l) => l.name === name);

  /**
   * 就地改说明。
   *
   * **不用 `window.prompt`** —— Tauri 的 WKWebView **没有实现它**：
   * 不弹框、不报错，直接返回 null，于是这个按钮点下去什么都不发生。
   * 同一个坑在剧本库的「新建项目」上踩过一次
   * （见 `ScriptsPage.tsx`），这是仓库里最后几处之一。
   */
  async function saveNote() {
    if (!editing) return;
    const { name, draft } = editing;
    setEditing(null);
    await setLoraNote(name, draft);
    setAll(await studioLoras());
  }

  return (
    <div className="flex flex-col gap-2">
      {usable.map((l) => {
        const on = picked(l.name);
        return (
          <div key={l.name} className="flex flex-col gap-1">
            <div className="flex items-center gap-2 text-[12px]">
              <input
                type="checkbox"
                id={`lora-${l.name}`}
                checked={!!on}
                onChange={(e) => setLora(l.name, e.target.checked ? LORA_DEFAULT : null)}
              />
              {/* 悬浮提示说清这个 LoRA 是干嘛的：
                  用户备注 > 训练标签 > 明说没有。文件名本身也带上，
                  因为同名不同目录的 LoRA 是两个东西 */}
              <label
                htmlFor={`lora-${l.name}`}
                className="flex-1 cursor-pointer truncate"
                title={`${loraHint(l)}\n\n${l.name}`}
              >
                {l.label}
              </label>
              {/* **采样预设已经内置挂了 H3 的 turbo LoRA。**
                  再选一次 WeeTodd 的 preflight 会拒 —— 而本机唯一的
                  H3 架构 LoRA 正好就是它，用户在视频节点上打开这个面板，
                  看到的就是这一个「选了会失败」的东西（2026-09-20 查实）。
                  不标出来的话，只能等到出片报错才知道。 */}
              {isBakedIn(l.name) && (
                <span className="shrink-0 rounded px-1 py-0.5 text-[0.5625rem]" style={{ color: "var(--studio-running-fg)", background: "var(--studio-running-bg)" }}>
                  预设已内置
                </span>
              )}
              {/* **确认的和猜的要分开说。**
                  作者标的激活词（从 Civitai 抄的）和从训练标签里猜的，
                  可信度差一大截。混成一样显示的话，用户会把猜错的那个
                  当成必须写的词照抄进提示词 —— 然后奇怪为什么没效果。 */}
              {l.confirmed_triggers.length > 0 ? (
                <span
                  className="rounded px-1 py-0.5 font-mono text-[0.5625rem]"
                  style={{
                    background: "color-mix(in srgb, var(--studio-primary) 16%, transparent)",
                    color: "var(--studio-primary)",
                  }}
                  title="作者标的激活词 —— 要写进提示词里才生效"
                >
                  {l.confirmed_triggers.join(" ")}
                </span>
              ) : (
                l.triggers.length > 0 && (
                  <span
                    className="font-mono text-[0.5625rem]"
                    style={{ color: "var(--studio-text-muted)" }}
                    title={t("studio:triggerGuess", {
                      defaultValue: "从训练标签里猜的，可能不准",
                    })}
                  >
                    {l.triggers.join(" ")}?
                  </span>
                )
              )}
              <button
                type="button"
                onClick={() =>
                  setEditing((e) =>
                    e?.name === l.name ? null : { name: l.name, draft: l.note ?? "" },
                  )
                }
                className="shrink-0 font-mono text-[0.5625rem] uppercase"
                style={{ color: "var(--studio-text-muted)" }}
                title={t("studio:noteEdit", { defaultValue: "写一句说明" })}
              >
                {l.note ? "✎" : "+"}
              </button>
            </div>
            {editing?.name === l.name ? (
              // **说明几乎必然是中文**，普通受控 input 会把拼音串原样打进去
              // （每敲一个字母回灌一次，冲掉输入法的组合缓冲）—— 见 `ComposingInput`。
              //
              // **落盘在回车和失焦，不在 `onCommit`。** `onCommit` 每敲一个
              // 字就来一次，照它落盘等于每个字写一次磁盘、还会把编辑框关掉。
              <ComposingInput
                autoFocus
                value={editing.draft}
                onCommit={(v) => setEditing({ name: l.name, draft: v })}
                onBlur={() => void saveNote()}
                onKeyDown={(e) => {
                  if (e.key === "Escape") setEditing(null);
                  if (e.key === "Enter") e.currentTarget.blur();
                }}
                placeholder={t("studio:notePrompt", {
                  defaultValue: `「${l.label}」是干嘛的？一句话`,
                })}
                className="ml-6 rounded bg-black/20 px-1.5 py-1 text-[10px]"
              />
            ) : (
              /* 有说明就显示出来，别只藏在悬浮里 —— 悬浮要移上去才看得到。
                 但说明现在带实测数据、出处和推荐权重，二十条全铺开列表就没法扫了，
                 所以超长的默认折一行、点开看全文。**短的不折** —— 折起来
                 等于多一次点击换零收益，而且展开后长得一样会让人以为没反应。 */
              (() => {
                const hint = loraHint(l);
                const fold = shouldFold(hint);
                const open = !!expanded[l.name];
                if (!fold) {
                  return (
                    <p
                      className="pl-6 text-[10px] leading-snug"
                      style={{ color: "var(--studio-text-muted)" }}
                    >
                      {hint}
                    </p>
                  );
                }
                return (
                  <button
                    type="button"
                    onClick={() => setExpanded((e) => ({ ...e, [l.name]: !open }))}
                    className="pl-6 pr-2 text-left text-[10px] leading-snug"
                    style={{ color: "var(--studio-text-muted)" }}
                    title={open ? "收起" : "展开全文"}
                  >
                    {open ? hint : foldSummary(hint)}
                    <span className="ml-1 whitespace-nowrap" style={{ color: "var(--studio-primary)" }}>
                      {open ? "收起" : "展开"}
                    </span>
                  </button>
                );
              })()
            )}
            {on && (
              // 数值必须**看得见**。原来只写在 title 里，要把鼠标停上去才知道
              // 拖到了多少 —— 而 LoRA 权重是个需要反复微调、还要跟上次比的值。
              <div className="flex items-center gap-2">
                <input
                  type="range"
                  min={0}
                  max={LORA_MAX}
                  step={LORA_STEP}
                  value={on.strength}
                  onChange={(e) => setLora(l.name, Number(e.target.value))}
                  className="min-w-0 flex-1"
                />
                <span
                  className="w-8 shrink-0 text-right font-mono text-[10px] tabular-nums"
                  style={{
                    // 偏离默认值时点亮 —— 一眼看出这条是调过的
                    color:
                      on.strength === LORA_DEFAULT
                        ? "var(--studio-text-muted)"
                        : "var(--studio-primary)",
                  }}
                  title={t("studio:loraStrength", {
                    defaultValue: `强度（默认 ${formatStrength(LORA_DEFAULT)}，上限 ${formatStrength(LORA_MAX)}）`,
                  })}
                >
                  {formatStrength(on.strength)}
                </span>
              </div>
            )}
          </div>
        );
      })}

      {usable.some((l) => l.triggers.length > 0 || l.confirmed_triggers.length > 0) && (
        <label
          className="mt-1 flex items-center gap-2 text-[11px]"
          style={{ color: "var(--studio-text-muted)" }}
        >
          <input
            type="checkbox"
            checked={injectTriggers}
            onChange={(e) => setInjectTriggers(e.target.checked)}
          />
          {t("studio:injectTriggers", {
            defaultValue: "自动把激活词加进提示词（会显示加了什么）",
          })}
        </label>
      )}
    </div>
  );
}
