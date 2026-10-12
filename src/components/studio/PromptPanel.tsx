import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { EngineSpec } from "../../api/studio";
import { useStudioStore } from "./useStudioStore";
import { CastRow } from "./CastRow";
import { AssetRefPicker } from "./AssetRefPicker";
import { pickProse, proseAdvice } from "./prosePolish";
import { cinemaPolishPrompt } from "../../api/polish";

/** SDXL 的 CLIP 是 77 token 分块取平均，超了会被摊薄 —— 中文约 150 字。 */
const SHORT_PROMPT_CHARS = 150;

/**
 * 提示词。
 *
 * **负向提示词框只在真正起作用的引擎上出现。** Z-Image 的 cfg=1，
 * 负向被 `ConditioningZeroOut` 清零；LTX / H3 走 WeeTodd 那条根本没有入口。
 * 显示一个不起作用的框比不显示更糟 —— 用户会以为自己压住了什么。
 */
/** 提示词上限。SDXL 那条另有一个更小的「会被摊薄」阈值，见下。 */
const MAX_CHARS = 2000;

function IconTemplate() {
  return (
    <svg width="11" height="11" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.4">
      <rect x="2" y="2" width="9" height="11" rx="1.5" />
      <path d="M5 5.5h3M5 8h3" strokeLinecap="round" />
      <path d="M13 4.5v8a1.5 1.5 0 0 1-1.5 1.5H5" opacity=".5" />
    </svg>
  );
}

/** 改写成句子。两个方框变成一行连续的线 —— 关键词变句子。 */
function IconProse() {
  return (
    <svg width="11" height="11" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.4">
      <rect x="1.5" y="3" width="4" height="2.5" rx="1" />
      <rect x="7" y="3" width="3.5" height="2.5" rx="1" />
      <path d="M2 9.5h11M2 12h8" strokeLinecap="round" />
      <path d="M12.5 5.2l1.8-1.8" strokeLinecap="round" opacity=".6" />
    </svg>
  );
}

function IconHistory() {
  return (
    <svg width="11" height="11" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.4">
      <path d="M8 4v4l2.5 1.5" strokeLinecap="round" strokeLinejoin="round" />
      <path d="M2.6 6.5A5.6 5.6 0 1 1 2.5 9" strokeLinecap="round" />
      <path d="M2 3.5v3h3" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

export function PromptPanel({
  engine,
  onOpenDrawer,
}: {
  engine: EngineSpec | null;
  onOpenDrawer: (k: "history" | "templates") => void;
}) {
  const { t } = useTranslation(["studio", "common"]);
  const [negOpen, setNegOpen] = useState(false);
  const [refPickerOpen, setRefPickerOpen] = useState(false);
  const prompt = useStudioStore((s) => s.prompt);
  const negative = useStudioStore((s) => s.negativePrompt);
  const setPrompt = useStudioStore((s) => s.setPrompt);
  const setNegative = useStudioStore((s) => s.setNegativePrompt);

  const tooLong = !!engine && !engine.long_prompt_ok && prompt.length > SHORT_PROMPT_CHARS;

  // 「改写成句子」。**按引擎决定给不给** —— SDXL 的 CLIP 要的就是短关键词，
  // 在那上面改写是让结果变差。见 prosePolish.ts 的文件头。
  const [prosing, setProsing] = useState(false);
  const [proseErr, setProseErr] = useState<string | null>(null);
  const advice = proseAdvice(engine?.id ?? "", prompt);
  const runProse = async () => {
    setProsing(true);
    setProseErr(null);
    try {
      const r = await cinemaPolishPrompt({ kind: "prose", draft: prompt, engine: engine?.id });
      const next = pickProse(r, engine?.id ?? "").trim();
      // **空结果不要覆盖原文。** 覆盖了就是把用户勾了半天的东西清空，
      // 而且没有撤销。
      if (next) setPrompt(next);
      if (r.warning) setProseErr(r.warning);
    } catch (e) {
      setProseErr(String(e));
    } finally {
      setProsing(false);
    }
  };

  return (
    <div className="flex flex-col gap-2">
      {/* 提示词里的 `@` 引用，渲染成头像行。挂没挂上必须看得见。 */}
      <CastRow prompt={prompt} />
      <textarea
        className="studio-input min-h-[110px] resize-y px-3 py-2 text-[13px]"
        placeholder={t("studio:promptPlaceholder", {
          defaultValue: "描述你想生成的内容…（打 @ 引用资产库里的角色/场景/道具）",
        })}
        value={prompt}
        onChange={(e) => {
          const next = e.target.value;
          setPrompt(next);
          // 刚打出一个 @ 就弹选择器 —— 不用先去别处选好再回来写。
          if (next.length === prompt.length + 1 && next.endsWith("@")) {
            setRefPickerOpen(true);
          }
        }}
      />
      {/* 改写失败 / 有话说的时候必须显示出来。
          最典型的是模型回了一句拒绝 —— 那不是提示词，不能悄悄用。 */}
      {proseErr && (
        <div
          className="rounded-lg px-3 py-2 text-[11px] leading-relaxed"
          style={{ background: "var(--studio-running-bg)", color: "var(--studio-running-fg)" }}
        >
          {proseErr}
        </div>
      )}
      {/* 底栏：模板/历史在左，字数在右 —— 照搬 LumenX 的排布 */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <button
            type="button"
            className="flex items-center gap-1 text-[11px]"
            style={{ color: "var(--studio-text-2)" }}
            onClick={() => onOpenDrawer("templates")}
          >
            <IconTemplate />
            {t("studio:templates", { defaultValue: "模板" })}
          </button>
          {/* **禁用时也要显示**，并且 title 说清楚为什么 ——
              直接藏起来的话，用户在 SDXL 上会以为这个功能不存在。 */}
          <button
            type="button"
            disabled={!advice.enabled || prosing}
            title={advice.title}
            className="flex items-center gap-1 text-[11px] disabled:cursor-not-allowed"
            style={{
              color: advice.suggested ? "var(--studio-primary)" : "var(--studio-text-2)",
              opacity: advice.enabled ? 1 : 0.4,
            }}
            onClick={() => void runProse()}
          >
            <IconProse />
            {prosing
              ? t("studio:prosing", { defaultValue: "改写中…" })
              : t("studio:toProse", { defaultValue: "改写成句子" })}
          </button>
          <button
            type="button"
            className="flex items-center gap-1 text-[11px]"
            style={{ color: "var(--studio-text-2)" }}
            onClick={() => onOpenDrawer("history")}
          >
            <IconHistory />
            {t("studio:history", { defaultValue: "历史" })}
          </button>
          <button
            type="button"
            className="text-[11px]"
            style={{ color: "var(--studio-text-2)" }}
            onClick={() => setRefPickerOpen(true)}
            title="把资产库里的角色/场景/道具插进提示词"
          >
            @ 引用资产
          </button>
        </div>
        <span
          className="font-mono text-[0.625rem]"
          style={{ color: tooLong ? "var(--studio-running-fg)" : "var(--studio-text-muted)" }}
        >
          {prompt.length} / {engine && !engine.long_prompt_ok ? SHORT_PROMPT_CHARS : MAX_CHARS}
        </span>
      </div>

      {refPickerOpen && (
        <AssetRefPicker
          onClose={() => setRefPickerOpen(false)}
          onPick={(tag) => {
            // 刚打的那个 `@` 本身要吃掉，否则会留下 `@[角色:老茶师]`。
            const base = prompt.endsWith("@") ? prompt.slice(0, -1) : prompt;
            setPrompt(base + tag);
            setRefPickerOpen(false);
          }}
        />
      )}

      {/* 这条是实测告警，不是风格建议：1251 字那次 SDXL 把 black silk 出成了深蓝 */}
      {tooLong && (
        <p className="text-[11px] leading-relaxed" style={{ color: "var(--studio-running-fg)" }}>
          {t("studio:promptDiluted", {
            defaultValue:
              "这个引擎的提示词超长会被分块取平均而摊薄 —— 压到 150 字以内的关键词最稳。",
          })}
        </p>
      )}

      {/* 负向提示词做成折叠行，默认收起 —— 照搬 LumenX 的节奏，省纵向空间。
          **不支持时不隐藏，是禁用 + 写明原因。** 全隐藏等于把能力藏了：
          用户不知道它存在过，也不知道为什么没有。真正坏的是显示一个
          「能输入但不起作用」的框。 */}
      <button
        type="button"
        onClick={() => setNegOpen((v) => !v)}
        className="mt-1 flex items-center gap-1.5 self-start text-[11px]"
        style={{ color: "var(--studio-text-muted)" }}
      >
        <span
          className="inline-block transition-transform"
          style={{ transform: negOpen ? "rotate(90deg)" : "none" }}
        >
          ▸
        </span>
        {t("studio:negativeLabel", { defaultValue: "负面提示词" })}
        {engine && !engine.negative_prompt && (
          <span style={{ opacity: 0.7 }}>
            · {t("studio:unsupported", { defaultValue: "此引擎不支持" })}
          </span>
        )}
        {/* 收起时也要能看出里面有没有内容，否则会忘掉自己写过什么 */}
        {!negOpen && negative.trim() !== "" && engine?.negative_prompt && (
          <span style={{ color: "var(--studio-primary)" }}>·</span>
        )}
      </button>

      {negOpen && (
        <textarea
          className="studio-input min-h-[60px] resize-y px-3 py-2 text-[13px]"
          placeholder={
            engine && !engine.negative_prompt
              ? t("studio:negativeInert", {
                  defaultValue: "换到 SDXL 才能用",
                })
              : t("studio:negativePlaceholder", {
                  defaultValue: "不想出现的元素…",
                })
          }
          disabled={!!engine && !engine.negative_prompt}
          style={engine && !engine.negative_prompt ? { opacity: 0.45 } : undefined}
          value={negative}
          onChange={(e) => setNegative(e.target.value)}
        />
      )}
    </div>
  );
}
