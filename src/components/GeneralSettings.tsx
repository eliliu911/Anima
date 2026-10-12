/**
 * 常规设置页（General Settings）：启动视图、系统语言一体化管理，纯正 Antigravity 极简风格
 */
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { setStartupWindow } from "../api";
import { useChat } from "../store/chat";
import { changeAppLanguage } from "../locales";
import { UpdatePanel } from "./UpdatePanel";

export function GeneralSettings() {
  const { t, i18n } = useTranslation(["settings", "common"]);
  const config = useChat((s) => s.config);
  const refreshConfig = useChat((s) => s.refreshConfig);
  const currentStartup = config?.startup_window || "avatar";

  const [saving, setSaving] = useState(false);
  const [successMsg, setSuccessMsg] = useState<string | null>(null);

  const currentLang = i18n.language || "en-US";
  const handleLanguageChange = (lang: string) => {
    changeAppLanguage(lang);
    // **提示语要用切换后的语言渲染。**
    //
    // `i18n.changeLanguage` 是异步的，而这一行的 `t` 是本次 render 闭包里捕获的，
    // 绑的还是旧语言 —— 于是中文切英文时弹出中文提示、英文切中文时弹出英文提示，
    // 正好反着。`getFixedT(lang)` 显式指定语言，不依赖异步完成或重渲染时机。
    const tNext = i18n.getFixedT(lang);
    setSuccessMsg(tNext("settings:general.savedNotice", { defaultValue: "已更新偏好设置" }));
    setTimeout(() => setSuccessMsg(null), 2500);
  };

  const handleSelectStartup = async (win: "avatar" | "chat" | "anima_studio") => {
    if (win === currentStartup || saving) return;
    setSaving(true);
    try {
      await setStartupWindow(win);
      await refreshConfig();
      setSuccessMsg(t("general.savedNotice", { defaultValue: "已更新偏好设置" }));
      setTimeout(() => setSuccessMsg(null), 2500);
    } catch (err) {
      console.error("Failed to update startup window:", err);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="flex-1 h-full overflow-y-auto p-7 flex flex-col gap-8 text-[var(--ink)] bg-[var(--surface-1)]">
      {/* 头部标题与说明 */}
      <div className="flex flex-col gap-1">
        <h2 className="text-[20px] font-bold tracking-tight">
          {t("general.title", { defaultValue: "常规设置" })}
        </h2>
        <p className="text-[13px] text-[var(--ink-3)]">
          {t("general.subtitle", {
            defaultValue: "配置启动视图形态、系统界面语言及基础运行环境偏好。",
          })}
        </p>
      </div>

      {/* 成功提示 */}
      {successMsg && (
        <div className="py-2.5 px-3.5 bg-emerald-500/10 border border-emerald-500/20 rounded-xl text-emerald-600 dark:text-emerald-400 text-[12.5px] flex items-center gap-2">
          <span>✓</span>
          <span>{successMsg}</span>
        </div>
      )}

      {/* 分区 1: 默认启动视图 */}
      <div className="flex flex-col gap-3">
        <div className="flex flex-col gap-0.5">
          <h3 className="text-[13px] font-bold uppercase tracking-wider text-[var(--ink-2)]">
            {t("general.startupSection", { defaultValue: "默认启动视图" })}
          </h3>
          <p className="text-[12px] text-[var(--ink-3)]">
            {t("general.startupDesc", {
              defaultValue: "选择系统启动时默认呈现的主窗口或交互形态。",
            })}
          </p>
        </div>

        <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
          {/* 选项 1: 虚拟人桌宠形态 */}
          <div
            onClick={() => handleSelectStartup("avatar")}
            className={`p-4 rounded-2xl border transition-all cursor-pointer flex flex-col justify-between gap-3 relative select-none ${
              currentStartup === "avatar"
                ? "bg-[var(--surface-2)] border-[var(--celadon)] ring-2 ring-[var(--celadon)]/20 shadow-xs"
                : "bg-[var(--surface-2)]/60 border-[var(--line)] hover:border-[var(--line-2)] hover:bg-[var(--surface-2)]"
            }`}
          >
            <div className="flex flex-col gap-2">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2.5">
                  <div className="w-8 h-8 rounded-lg bg-[var(--surface-3)] flex items-center justify-center text-[var(--ink)]">
                    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                      <path d="M12 2a4 4 0 1 0 0 8 4 4 0 0 0 0-8z" />
                      <path d="M6 21v-2a4 4 0 0 1 4-4h4a4 4 0 0 1 4 4v2" />
                      <path d="M9 3L7.5 1.5" />
                      <path d="M15 3L16.5 1.5" />
                    </svg>
                  </div>
                  <span className="text-[13.5px] font-bold text-[var(--ink)]">
                    {t("general.avatarTitle", { defaultValue: "虚拟人桌宠形态" })}
                  </span>
                </div>
                <span className="px-2 py-0.5 text-[10.5px] font-semibold tracking-wide uppercase rounded-full bg-[var(--celadon)]/10 text-[var(--celadon)] border border-[var(--celadon)]/20">
                  {t("general.avatarBadge", { defaultValue: "默认推荐" })}
                </span>
              </div>
              <p className="text-[12px] text-[var(--ink-3)] leading-relaxed">
                {t("general.avatarDesc", {
                  defaultValue:
                    "启动时以透明悬浮层展示 Live2D / 3D / 神经写实数字人，面板可随时从托盘打开。",
                })}
              </p>
            </div>

            <div className="flex items-center justify-between pt-2 border-t border-[var(--line)]/50">
              <span className="text-[11px] font-mono text-[var(--ink-3)]">window: avatar</span>
              <div
                className={`w-4 h-4 rounded-full border flex items-center justify-center transition-colors ${
                  currentStartup === "avatar"
                    ? "border-[var(--celadon)] bg-[var(--celadon)] text-white"
                    : "border-[var(--ink-3)]"
                }`}
              >
                {currentStartup === "avatar" && <div className="w-1.5 h-1.5 rounded-full bg-white" />}
              </div>
            </div>
          </div>

          {/* 选项 2: Chat 智能对话工作区 */}
          <div
            onClick={() => handleSelectStartup("chat")}
            className={`p-4 rounded-2xl border transition-all cursor-pointer flex flex-col justify-between gap-3 relative select-none ${
              currentStartup === "chat"
                ? "bg-[var(--surface-2)] border-[var(--celadon)] ring-2 ring-[var(--celadon)]/20 shadow-xs"
                : "bg-[var(--surface-2)]/60 border-[var(--line)] hover:border-[var(--line-2)] hover:bg-[var(--surface-2)]"
            }`}
          >
            <div className="flex flex-col gap-2">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2.5">
                  <div className="w-8 h-8 rounded-lg bg-[var(--surface-3)] flex items-center justify-center text-[var(--ink)]">
                    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                      <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
                    </svg>
                  </div>
                  <span className="text-[13.5px] font-bold text-[var(--ink)]">
                    {t("general.chatTitle", { defaultValue: "Chat 智能对话工作区" })}
                  </span>
                </div>
                <span className="px-2 py-0.5 text-[10.5px] font-semibold tracking-wide uppercase rounded-full bg-blue-500/10 text-blue-500 border border-blue-500/20">
                  {t("general.chatBadge", { defaultValue: "深度助手" })}
                </span>
              </div>
              <p className="text-[12px] text-[var(--ink-3)] leading-relaxed">
                {t("general.chatDesc", {
                  defaultValue:
                    "启动时直接打开全功能 Chat 对话与智能体工程面板，便于立即开展深度开发与会话。",
                })}
              </p>
            </div>

            <div className="flex items-center justify-between pt-2 border-t border-[var(--line)]/50">
              <span className="text-[11px] font-mono text-[var(--ink-3)]">window: chat</span>
              <div
                className={`w-4 h-4 rounded-full border flex items-center justify-center transition-colors ${
                  currentStartup === "chat"
                    ? "border-[var(--celadon)] bg-[var(--celadon)] text-white"
                    : "border-[var(--ink-3)]"
                }`}
              >
                {currentStartup === "chat" && <div className="w-1.5 h-1.5 rounded-full bg-white" />}
              </div>
            </div>
          </div>

          {/* 选项 3: Anima Studio。整条创作流水线都在这个窗口里 ——
              工作区 → 剧本 → 风格 → 资产 → 分镜 → 动效 → 组装。
              做创作的人开机就该落在这儿，而不是先开对话再去托盘找。 */}
          <div
            onClick={() => handleSelectStartup("anima_studio")}
            className={`p-4 rounded-2xl border transition-all cursor-pointer flex flex-col justify-between gap-3 relative select-none ${
              currentStartup === "anima_studio"
                ? "bg-[var(--surface-2)] border-[var(--celadon)] ring-2 ring-[var(--celadon)]/20 shadow-xs"
                : "bg-[var(--surface-2)]/60 border-[var(--line)] hover:border-[var(--line-2)] hover:bg-[var(--surface-2)]"
            }`}
          >
            <div className="flex flex-col gap-2">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2.5">
                  <div className="w-8 h-8 rounded-lg bg-[var(--surface-3)] flex items-center justify-center text-[var(--ink)]">
                    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
                      <rect x="2.5" y="4.5" width="19" height="15" rx="2" />
                      <path d="M2.5 9h19" />
                      <path d="M7 4.5V9M17 4.5V9" opacity=".55" />
                      <path d="m10.5 12.5 4 2.2-4 2.3z" />
                    </svg>
                  </div>
                  <span className="text-[13.5px] font-bold text-[var(--ink)]">
                    {t("general.studioTitle", { defaultValue: "Anima Studio 创作台" })}
                  </span>
                </div>
                <span className="px-2 py-0.5 text-[10.5px] font-semibold tracking-wide uppercase rounded-full bg-emerald-500/10 text-emerald-500 border border-emerald-500/20">
                  {t("general.studioBadge", { defaultValue: "出片" })}
                </span>
              </div>
              <p className="text-[12px] text-[var(--ink-3)] leading-relaxed">
                {t("general.studioDesc", {
                  defaultValue:
                    "启动时直接进工作区：剧本 → 风格 → 资产 → 分镜 → 动效 → 组装，整条创作流水线都在这一个窗口里。",
                })}
              </p>
            </div>

            <div className="flex items-center justify-between pt-2 border-t border-[var(--line)]/50">
              <span className="text-[11px] font-mono text-[var(--ink-3)]">window: anima_studio</span>
              <div
                className={`w-4 h-4 rounded-full border flex items-center justify-center transition-colors ${
                  currentStartup === "anima_studio"
                    ? "border-[var(--celadon)] bg-[var(--celadon)] text-white"
                    : "border-[var(--ink-3)]"
                }`}
              >
                {currentStartup === "anima_studio" && (
                  <div className="w-1.5 h-1.5 rounded-full bg-white" />
                )}
              </div>
            </div>
          </div>
        </div>
      </div>

      {/* 分区 2: 系统语言切换 */}
      <div className="flex flex-col gap-3">
        <div className="flex flex-col gap-0.5">
          <h3 className="text-[13px] font-bold uppercase tracking-wider text-[var(--ink-2)]">
            {t("general.languageSection", { defaultValue: "界面语言" })}
          </h3>
          <p className="text-[12px] text-[var(--ink-3)]">
            {t("general.languageDesc", {
              defaultValue: "选择应用于所有窗口、面板及系统托盘的界面展示语言。",
            })}
          </p>
        </div>

        <div className="p-4 rounded-2xl border border-[var(--line)] bg-[var(--surface-2)]/60 flex items-center justify-between gap-4">
          <div className="flex items-center gap-3">
            <div className="w-8 h-8 rounded-lg bg-[var(--surface-3)] flex items-center justify-center text-[var(--ink)]">
              <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <circle cx="12" cy="12" r="10" />
                <line x1="2" y1="12" x2="22" y2="12" />
                <path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z" />
              </svg>
            </div>
            <div className="flex flex-col">
              <span className="text-[13px] font-semibold text-[var(--ink)]">
                {currentLang.startsWith("zh") ? "简体中文 (Chinese Simplified)" : "English (US)"}
              </span>
              <span className="text-[11.5px] text-[var(--ink-3)]">
                {t("common:language", { defaultValue: "Language" })}
              </span>
            </div>
          </div>

          <select
            value={currentLang.startsWith("zh") ? "zh-CN" : "en-US"}
            onChange={(e) => handleLanguageChange(e.target.value)}
            className="bg-[var(--surface-3)] text-[var(--ink)] border border-[var(--line)] rounded-xl px-3 py-1.5 text-[12.5px] font-medium cursor-pointer outline-none hover:border-[var(--line-2)] transition-colors"
          >
            <option value="en-US">English (US)</option>
            <option value="zh-CN">简体中文</option>
          </select>
        </div>
      </div>

      {/* 「我在跑哪一版」+ 更新。构建信息在这一块的底部 */}
      <UpdatePanel />
    </div>
  );
}
