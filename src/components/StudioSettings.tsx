/**
 * 设置 → Studio。
 *
 * # 为什么单独一页
 *
 * 2026-09-19 用户：「我需要在设置里单独分出一个 Studio 区域，这个区域里
 * 单独有一个 Prompt 设置……**后期很多时候都是对 Prompt 的微调整**。」
 *
 * 提示词配置**本来就完整存在**（每个槽位能改能存能试跑，落盘
 * `prompt_config.json`），入口在项目页右上角。但那是「做的时候顺手调」的位置，
 * 而用户找的是「设置里那个能改参数的地方」——
 * 他的原话：「按钮叫提示词，但是这个名字一直被我忽略了」。
 *
 * 所以两个入口指同一个对话框，不是两份配置。
 *
 * # 这一页只做归位和交代，不搬家
 *
 * 出图引擎、画幅这些是**项目级**的（每个项目可以不一样），
 * 搬进全局设置会变成两个所有者。这里只说它们在哪。
 */
import { useEffect, useState } from "react";
import { PromptConfigDialog } from "./studio/PromptConfigDialog";
import { promptExport, promptImport, promptSlots, type PromptSlotView } from "../api/prompts";

export function StudioSettings() {
  const [open, setOpen] = useState(false);
  const [slots, setSlots] = useState<PromptSlotView[] | null>(null);
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);

  const refresh = () => void promptSlots().then(setSlots).catch(() => setSlots(null));
  useEffect(refresh, [open]);

  const changed = slots?.filter((s) => s.overridden) ?? [];
  const stale = changed.filter((s) => s.default_changed);

  const doExport = async () => {
    try {
      const text = await promptExport();
      await navigator.clipboard.writeText(text);
      setMsg({ ok: true, text: "整份配置已复制到剪贴板，粘到文件里存起来就行" });
    } catch (e) {
      setMsg({ ok: false, text: `导出失败：${e}` });
    }
  };

  const doImport = async () => {
    try {
      const text = await navigator.clipboard.readText();
      if (!text.trim()) {
        setMsg({ ok: false, text: "剪贴板是空的 —— 先把导出的那段 JSON 复制上" });
        return;
      }
      setSlots(await promptImport(text));
      setMsg({ ok: true, text: "导入完成。**整份替换**，不是合并" });
    } catch (e) {
      setMsg({ ok: false, text: `导入失败：${e}` });
    }
  };

  return (
    <div className="flex-1 overflow-y-auto px-8 py-7">
      <h2 className="text-[15px] font-semibold text-[var(--ink)]">Studio</h2>
      <p className="mt-1 text-[12px] text-[var(--ink-3)]">漫剧的 AI 环节：提示词、生成设置、素材位置</p>

      {/* ── 提示词 ── */}
      <section className="mt-6 rounded-xl border border-[var(--line)] p-4">
        <div className="flex items-center justify-between gap-3">
          <div className="min-w-0">
            <h3 className="text-[13px] font-medium text-[var(--ink)]">提示词设置</h3>
            <p className="mt-0.5 text-[11.5px] leading-relaxed text-[var(--ink-3)]">
              抽实体 / 分场 / 生成镜头 / B 路写手 / 各类润色 —— 每个 AI 环节都能改，改完能试跑。
              和项目页右上角那个「提示词设置」是**同一份配置**，不是两处。
            </p>
          </div>
          <button
            type="button"
            onClick={() => setOpen(true)}
            className="shrink-0 rounded-lg bg-[var(--surface-3)] px-3 py-1.5 text-[12px] text-[var(--ink)]"
          >
            打开
          </button>
        </div>

        {slots && (
          <p className="mt-3 text-[11.5px] text-[var(--ink-3)]">
            {changed.length === 0
              ? `13 个槽位全部用内置默认`
              : `已自定义 ${changed.length} / ${slots.length} 个：${changed.map((s) => s.label).join("、")}`}
          </p>
        )}

        {/* **内置默认改进了要说一声。** 不自动迁移（那是用户改过的文本），
            但也不能装作没事 —— 否则他会一直用着一份缺了后来新增规则的提示词，
            而且什么迹象都没有。 */}
        {stale.length > 0 && (
          <p className="mt-2 rounded-lg px-3 py-2 text-[11.5px] leading-relaxed"
             style={{ background: "color-mix(in srgb, var(--warn, #d88) 12%, transparent)", color: "var(--warn, #d88)" }}>
            这 {stale.length} 个槽位是照**旧版内置默认**改的，后来内置的那份更新了：
            {stale.map((s) => s.label).join("、")}。
            打开看一眼新的默认加了什么，要不要并进你的版本 —— 不会自动改你的文本。
          </p>
        )}

        <div className="mt-3 flex items-center gap-2">
          <button type="button" onClick={() => void doExport()}
            className="rounded-lg bg-[var(--surface-3)] px-2.5 py-1.5 text-[11.5px] text-[var(--ink-2)]"
            title="整份复制到剪贴板：换机器、回滚、分享调参结果">
            导出到剪贴板
          </button>
          <button type="button" onClick={() => void doImport()}
            className="rounded-lg bg-[var(--surface-3)] px-2.5 py-1.5 text-[11.5px] text-[var(--ink-2)]"
            title="从剪贴板整份替换（不是合并）">
            从剪贴板导入
          </button>
        </div>
        {msg && (
          <p className="mt-2 text-[11.5px]" style={{ color: msg.ok ? "var(--ok, #6a9)" : "var(--warn, #d88)" }}>
            {msg.text}
          </p>
        )}
      </section>

      {/* ── 在别处的那些，只说位置 ── */}
      <section className="mt-4 rounded-xl border border-[var(--line)] p-4">
        <h3 className="text-[13px] font-medium text-[var(--ink)]">在别处的设置</h3>
        <p className="mt-0.5 text-[11.5px] leading-relaxed text-[var(--ink-3)]">
          这些是**项目级**的（每个项目可以不一样），搬进全局设置会变成两个所有者，所以留在原地：
        </p>
        <dl className="mt-2.5 flex flex-col gap-1.5 text-[11.5px]">
          {[
            ["生成设置（出图/出片引擎、画幅、LoRA）", "项目页右上角「生成设置」"],
            ["TTS 引擎和全局音色", "设置 → Application"],
            ["角色的配音音色 / H3 声音设定", "资产库 → 选中角色 → 人物页签"],
            ["本机参考音频库", "项目根目录的 voice-library/"],
            ["各服务在不在跑", "设置 → 服务"],
          ].map(([what, where]) => (
            <div key={what} className="flex items-baseline gap-2">
              <dt className="w-[240px] shrink-0 text-[var(--ink-2)]">{what}</dt>
              <dd className="min-w-0 flex-1 text-[var(--ink-3)]">{where}</dd>
            </div>
          ))}
        </dl>
      </section>

      {open && <PromptConfigDialog onClose={() => { setOpen(false); refresh(); }} />}
    </div>
  );
}
