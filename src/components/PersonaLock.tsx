import { useTranslation } from "react-i18next";
import { useEffect, useState } from "react";
import { personaView, type PersonaView } from "../api/voice";

/**
 * 「这块面板现在归谁管」。
 *
 * ## 为什么需要它
 *
 * 面容和语音声学**只在默认形态下可编辑**。选了具体角色之后，这两块由
 * 角色决定 —— 允许在全局面板上改的话就有两个写入口，谁后写谁赢。
 * 之前那一串查不明白的"没声音"全是这么来的：设置里显示 Edge、
 * 实际跑的是上一个角色绑的 SoVITS。
 *
 * ## 禁用要说清楚原因
 *
 * 只把控件灰掉、不说为什么，用户只会以为坏了。所以这里同时给出
 * 当前角色的名字和一句"去哪儿改"。
 */
export function usePersona() {
  const [view, setView] = useState<PersonaView | null>(null);
  const [error, setError] = useState<string | null>(null);

  const reload = () =>
    personaView()
      .then((v) => {
        setView(v);
        setError(null);
      })
      .catch((e) => setError(`${e}`));

  useEffect(() => {
    void reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return { view, error, reload };
}

/** 当前形态的名字。默认形态就叫「默认形态」 */
export function formName(view: PersonaView | null): string {
  if (!view) return "…";
  if (!view.active) return "默认形态";
  return view.characters.find((c) => c.id === view.active)?.name ?? view.active;
}

/**
 * 面板顶部那条横幅。默认形态下不显示 —— 没有需要解释的东西。
 */
export function PersonaLockBanner({
  view,
  what,
  onGoToCharacters,
}: {
  view: PersonaView | null;
  /** 这块面板管的是什么，用于文案：「面容」「语音声学」 */
  what: string;
  onGoToCharacters?: () => void;
}) {
  const { t } = useTranslation(["settings", "common"]);
  if (!view || view.resolved.is_default_form) return null;

  const whatLabel = what === "面容" ? t("lockBanner.faceWhat", { defaultValue: "面容" }) : t("lockBanner.voiceWhat", { defaultValue: "语音声学" });

  return (
    <div className="flex items-start gap-3 p-3.5 px-4 rounded-2xl bg-amber-500/10 border border-amber-500/25 text-[12.5px] leading-relaxed text-[var(--ink)] shadow-2xs mb-2">
      <span className="text-[16px] shrink-0 mt-0.5" aria-hidden="true">🔒</span>
      <div className="flex-1">
        {t("lockBanner.text", { what: whatLabel, name: formName(view), defaultValue: `${what}当前由角色「${formName(view)}」绑定接管，此处为只读预览。` })}
        <div className="text-[11.5px] text-[var(--ink-2)] mt-0.5">
          {t("lockBanner.hint", { defaultValue: "如需调整，请前往「形态/角色」进行配置，或切回默认形态。" })}
        </div>
      </div>
      {onGoToCharacters && (
        <button
          type="button"
          className="px-3 py-1.5 rounded-xl bg-[var(--paper)] hover:bg-[var(--surface-2)] border border-amber-500/30 text-[12px] font-semibold text-amber-700 shadow-2xs cursor-pointer transition-colors shrink-0"
          onClick={onGoToCharacters}
        >
          前往角色设置
        </button>
      )}
    </div>
  );
}

/** 某一项是不是角色定的 —— 控件据此禁用 */
export function lockedBy(view: PersonaView | null, field: "engine" | "voice" | "avatar" | "skills") {
  if (!view) return false;
  return view.resolved[field].origin === "character";
}
