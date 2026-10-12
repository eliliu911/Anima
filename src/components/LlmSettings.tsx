/**
 * 模型中枢设置页：现代极简 Antigravity 风格卡片设计
 */
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import * as api from "../api";
import type { ProviderStatus, Tier } from "../api/types";
import { useChat, type ModelList } from "../store/chat";
import { keySourceHint, keySourceLabel } from "./keySource";

/** 三条链路。第三条不是 Tier —— 它是工具调用的备用后端 */
type Route = Tier | "tool_fallback";

export function LlmSettings() {
  const { t } = useTranslation(["settings", "common"]);
  const config = useChat((s) => s.config);
  const models = useChat((s) => s.models);
  const loadModels = useChat((s) => s.loadModels);
  const refreshConfig = useChat((s) => s.refreshConfig);
  const tier = useChat((s) => s.tier);
  const setTier = useChat((s) => s.setTier);
  const systemPrompt = useChat((s) => s.systemPrompt);
  const setSystemPrompt = useChat((s) => s.setSystemPrompt);

  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [keys, setKeys] = useState<Record<string, string>>({});

  const providers = config?.providers ?? [];

  useEffect(() => {
    if (providers.length === 0) return;
    let cancelled = false;
    (async () => {
      for (const p of providers) {
        if (cancelled) return;
        await loadModels(p.id);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [providers.map((p) => p.id).join(","), loadModels]);

  async function run(key: string, what: string, fn: () => Promise<unknown>) {
    setBusy(key);
    setMessage(`Saving ${what}…`);
    try {
      await fn();
      await refreshConfig();
      setMessage(`Updated ${what}`);
      setTimeout(() => setMessage(null), 2500);
    } catch (err) {
      setMessage(`Save failed: ${String(err)}`);
    } finally {
      setBusy(null);
    }
  }

  function routeTarget(r: Route): string {
    if (!config) return "";
    if (r === "realtime") return config.realtime_provider;
    if (r === "background") return config.background_provider;
    return config.tool_fallback_provider ?? "";
  }

  function routeModel(r: Route): string {
    if (!config) return "";
    if (r === "realtime") return config.realtime_model ?? "";
    if (r === "background") return config.background_model ?? "";
    return config.tool_fallback_model ?? "";
  }

  function providerOf(id: string) {
    return providers.find((p) => p.id === id);
  }

  function modelsOf(providerId: string): string[] {
    const list = models[providerId]?.list;
    if (list && list.length > 0) return list;
    const fallback = providerOf(providerId)?.model;
    return fallback ? [fallback] : [];
  }

  const routesConfig: Array<{ id: Route; label: string; note: string }> = [
    {
      id: "realtime",
      label: t("llm.realtime", { defaultValue: "实时对话" }),
      note: t("llm.realtimeDesc", { defaultValue: "语音和文字对话核心链路。响应极速，首字延迟低。" }),
    },
    {
      id: "background",
      label: t("llm.background", { defaultValue: "后台推理" }),
      note: t("llm.backgroundDesc", { defaultValue: "记忆提取、事实归纳、技能编排。注重深度逻辑与上下文。" }),
    },
    {
      id: "tool_fallback",
      label: t("llm.toolFallback", { defaultValue: "工具调用备用" }),
      note: t("llm.toolFallbackDesc", { defaultValue: "主后端不支持工具调用时的自动回退。留空则智能挑选。" }),
    },
  ];

  if (!config) {
    return (
      <div className="flex items-center justify-center py-12 text-[13px] text-[var(--ink-3)]">
        加载配置中…
      </div>
    );
  }

  return (
    <div className="flex flex-col gap-6 max-w-4xl pb-8">
      {/* ── ① 谁来干活 ── */}
      <div className="flex flex-col gap-3">
        <div className="flex flex-col">
          <span className="text-[14px] font-bold text-[var(--ink)] tracking-tight">
            {t("llm.routesTitle", { defaultValue: "链路分配" })}
          </span>
          <span className="text-[12px] text-[var(--ink-2)] mt-0.5 leading-relaxed">
            {t("llm.routesDesc", { defaultValue: "每条链路独立指定后端与模型。例如：实时对话使用极速 8B 模型，后台推理使用高质量 35B/70B 模型。" })}
          </span>
        </div>

        <div className="flex flex-col gap-2.5">
          {routesConfig.map((route) => {
            const target = routeTarget(route.id);
            const provider = providerOf(target);
            const options =
              route.id === "tool_fallback"
                ? providers.filter((p) => p.supports_tools)
                : providers;

            return (
              <div
                key={route.id}
                className="flex flex-col md:flex-row md:items-center justify-between gap-3 p-3.5 bg-[var(--paper)] border border-[var(--line)] rounded-2xl shadow-2xs transition-all hover:border-[var(--ink-3)]/40"
              >
                <div className="flex flex-col min-w-[140px] max-w-[200px] shrink-0">
                  <div className="flex items-center gap-1.5">
                    <span className="text-[13px] font-bold text-[var(--ink)]">
                      {route.label}
                    </span>
                    {route.id === "realtime" && (
                      <span className="text-[10px] bg-[var(--celadon-2)] text-[var(--celadon)] px-1.5 py-0.2 rounded-full font-medium">
                        {t("llm.realtimeCore", { defaultValue: "核心" })}
                      </span>
                    )}
                  </div>
                  <span className="text-[11px] text-[var(--ink-3)] mt-0.5 leading-snug">
                    {route.note}
                  </span>
                </div>

                <div className="flex flex-col sm:flex-row gap-2 flex-1 min-w-0">
                  {/* 选择后端 */}
                  <select
                    className="flex-1 min-w-0 bg-[var(--surface)] hover:bg-[var(--surface-2)] border border-[var(--line)] rounded-xl px-3 py-1.5 text-[12.5px] text-[var(--ink)] outline-none focus:border-[var(--celadon)] focus:ring-2 focus:ring-[var(--celadon)]/15 transition-all cursor-pointer shadow-2xs disabled:opacity-50"
                    value={target}
                    disabled={busy !== null}
                    onChange={(e) => {
                      const value = e.target.value;
                      if (route.id === "tool_fallback") {
                        void run("route", "fallback provider", () =>
                          api.setToolFallback(value === "" ? null : value),
                        );
                      } else {
                        void run("route", `${route.label} provider`, async () => {
                          await api.setTierProvider(route.id as Tier, value);
                          // **换了后端，模型名清回「跟随后端」**：留着的是上一个后端的名字
                          // （09-29：LM Studio 的 openai/gpt-oss-120b 留在了 Azure 上，后台任务全部失败，下拉框还标着「当前使用」）
                          if (routeModel(route.id)) await api.setRouteModel(route.id, "");
                        });
                      }
                    }}
                  >
                    {route.id === "tool_fallback" && <option value="">{t("llm.autoPick", { defaultValue: "自动挑选" })}</option>}
                    {options.map((p) => (
                      <option key={p.id} value={p.id}>
                        {t("providers." + p.id, { defaultValue: p.label })}
                      </option>
                    ))}
                  </select>

                  {/* 选择模型 */}
                  <select
                    className="flex-1 min-w-0 bg-[var(--surface)] hover:bg-[var(--surface-2)] border border-[var(--line)] rounded-xl px-3 py-1.5 text-[12.5px] text-[var(--ink)] outline-none focus:border-[var(--celadon)] focus:ring-2 focus:ring-[var(--celadon)]/15 transition-all cursor-pointer shadow-2xs disabled:opacity-50"
                    value={routeModel(route.id)}
                    disabled={busy !== null || !provider}
                    onChange={(e) =>
                      void run("route", `${route.label} model`, () =>
                        api.setRouteModel(route.id, e.target.value),
                      )
                    }
                  >
                    <option value="">
                      {provider?.model ? t("llm.followBackend", { model: provider.model, defaultValue: `跟随后端（${provider.model}）` }) : t("llm.followBackendDefault", { defaultValue: "跟随后端默认" })}
                    </option>
                    {routeModel(route.id) &&
                      !modelsOf(target).includes(routeModel(route.id)) && (
                        <option value={routeModel(route.id)}>
                          {t("llm.currentlyUsed", { model: routeModel(route.id), defaultValue: `${routeModel(route.id)}（当前使用）` })}
                        </option>
                      )}
                    {modelsOf(target).map((m) => (
                      <option key={m} value={m}>
                        {m}
                      </option>
                    ))}
                  </select>
                </div>
              </div>
            );
          })}
        </div>
      </div>

      {/* ── ② 后端怎么连 ── */}
      <div className="flex flex-col gap-3">
        <div className="flex flex-col">
          <span className="text-[14px] font-bold text-[var(--ink)] tracking-tight">
            {t("llm.providersTitle", { defaultValue: "后端服务配置" })}
          </span>
          <span className="text-[12px] text-[var(--ink-2)] mt-0.5 leading-relaxed">
            {t("llm.providersDesc", { defaultValue: "配置各 LLM 引擎的 API 接入地址与密钥凭证。" })}
          </span>
        </div>

        <div className="flex flex-col gap-3">
          {providers.map((p) => (
            <BackendCard
              key={p.id}
              provider={p}
              usedBy={routesConfig.filter((r) => routeTarget(r.id) === p.id).map((r) => r.label)}
              models={models[p.id]}
              busy={busy}
              draftUrl={drafts[p.id] ?? p.base_url}
              onDraftUrl={(v) => setDrafts((d) => ({ ...d, [p.id]: v }))}
              draftKey={keys[p.id] ?? ""}
              onDraftKey={(v) => setKeys((k) => ({ ...k, [p.id]: v }))}
              onSaveUrl={() =>
                run(p.id, `${p.label} 的地址`, async () => {
                  await api.updateProvider({ id: p.id, baseUrl: drafts[p.id] ?? p.base_url });
                  // 端点换了：旧列表是旧端点的，清掉重拉（不然看着像没生效）
                  void loadModels(p.id, { reset: true });
                })
              }
              onSaveKey={() =>
                run(p.id, `${p.label} 的密钥`, async () => {
                  await api.setApiKey(p.id, keys[p.id] ?? "");
                  setKeys((k) => ({ ...k, [p.id]: "" }));
                  void loadModels(p.id, { reset: true });
                })
              }
              onRefresh={() => void loadModels(p.id)}
            />
          ))}
        </div>
      </div>

      {/* ── ③ 怎么说话 ── */}
      <div className="flex flex-col gap-3">
        <div className="flex flex-col">
          <span className="text-[14px] font-bold text-[var(--ink)] tracking-tight">
            {t("llm.globalInstructionsTitle", { defaultValue: "全局指令与行为" })}
          </span>
        </div>

        <div className="flex flex-col gap-3.5 p-4 bg-[var(--paper)] border border-[var(--line)] rounded-2xl shadow-2xs">
          <div className="flex flex-col gap-1.5">
            <label className="text-[12px] font-bold text-[var(--ink)]">
              {t("llm.systemPromptLabel", { defaultValue: "全局指令（System Prompt）" })}
            </label>
            <textarea
              className="bg-[var(--surface)] border border-[var(--line)] focus:border-[var(--celadon)] focus:ring-2 focus:ring-[var(--celadon)]/15 rounded-xl p-3 text-[12.5px] font-mono text-[var(--ink)] outline-none resize-y min-h-[90px] transition-all shadow-2xs leading-relaxed"
              rows={4}
              value={systemPrompt}
              onChange={(e) => setSystemPrompt(e.target.value)}
              placeholder={t("llm.systemPromptPlaceholder", { defaultValue: "输入全局系统提示词…" })}
            />
          </div>

          <div className="flex flex-col gap-1.5">
            <span className="text-[12px] font-semibold text-[var(--ink)]">{t("llm.thinkingLevelTitle", { defaultValue: "思考档位" })}</span>
            <em className="not-italic text-[11px] text-[var(--ink-3)] leading-tight">
              {t("llm.thinkingLevelDesc", { defaultValue: "思考阶段的输出送不进 TTS，语音链路等的是第一个正文 token。档位按链路记，不跟着模型走。" })}
            </em>
            <div className="flex flex-col gap-1 pt-1 text-[11.5px] text-[var(--ink-2)]">
              <span>
                {t("llm.realtimeRoute", { defaultValue: "实时链路：" })}<strong className="text-[var(--ink)]">{config.thinking.realtime}</strong>
              </span>
              <span>
                {t("llm.backgroundRoute", { defaultValue: "后台链路：" })}<strong className="text-[var(--ink)]">{config.thinking.background}</strong>
              </span>
            </div>
          </div>

          <div className="flex flex-col gap-1.5 pt-3 border-t border-[var(--line)]/50">
            <label className="text-[12px] font-semibold text-[var(--ink)] flex justify-between items-center">
              {t("llm.chatRoutingLabel", { defaultValue: "文字对话路由" })}
              <em className="not-italic font-normal text-[10.5px] text-[var(--ink-3)]">
                {t("llm.chatRoutingNote", { defaultValue: "仅影响侧边栏文字输入，语音固定走实时链路。" })}
              </em>
            </label>
            <select
              className="bg-[var(--surface)] hover:bg-[var(--surface-2)] border border-[var(--line)] rounded-xl px-3 py-1.5 outline-none text-[12.5px] text-[var(--ink)] focus:border-[var(--celadon)] transition-colors cursor-pointer shadow-2xs"
              value={tier}
              onChange={(e) => setTier(e.target.value as Tier)}
            >
              <option value="realtime">{t("llm.realtimeMode", { defaultValue: "实时对话（极速模式）" })}</option>
              <option value="background">{t("llm.backgroundMode", { defaultValue: "后台推理（高精推理）" })}</option>
            </select>
          </div>
        </div>
      </div>

      {message && (
        <div className="fixed bottom-6 right-6 z-50 text-[12.5px] text-[var(--ink)] bg-[var(--paper)] border border-[var(--celadon)] shadow-lg px-4 py-2.5 rounded-2xl flex items-center gap-2 animate-in fade-in slide-in-from-bottom-2">
          <span className="w-2 h-2 rounded-full bg-[var(--celadon)] animate-pulse" />
          <span>{message}</span>
        </div>
      )}
    </div>
  );
}

interface BackendCardProps {
  provider: ProviderStatus;
  usedBy: string[];
  models?: ModelList;
  busy: string | null;
  draftUrl: string;
  onDraftUrl: (v: string) => void;
  draftKey: string;
  onDraftKey: (v: string) => void;
  onSaveUrl: () => void;
  onSaveKey: () => void;
  onRefresh: () => void;
}

function BackendCard({
  provider: p,
  usedBy,
  models,
  busy,
  draftUrl,
  onDraftUrl,
  draftKey,
  onDraftKey,
  onSaveUrl,
  onSaveKey,
  onRefresh,
}: BackendCardProps) {
  const { t } = useTranslation(["settings", "common"]);
  const disabled = busy !== null;
  const list = models?.list ?? [];

  const displayProviderLabel = t("providers." + p.id, { defaultValue: p.label });

  return (
    <div className="flex flex-col gap-3.5 p-4 bg-[var(--paper)] border border-[var(--line)] rounded-2xl shadow-2xs transition-all hover:border-[var(--ink-3)]/40">
      <div className="flex justify-between items-start">
        <div className="flex items-center gap-2 flex-wrap">
          <span className="text-[13.5px] font-bold text-[var(--ink)]">{displayProviderLabel}</span>
          {usedBy.length > 0 && (
            <span className="text-[11px] text-[var(--celadon)] bg-[var(--celadon-2)] px-2 py-0.5 rounded-full font-medium">
              {usedBy.join(" · ")}
            </span>
          )}
        </div>

        <span
          className={`text-[10.5px] px-2 py-0.5 rounded-full font-medium border ${
            p.has_key
              ? "bg-emerald-50 text-emerald-600 border-emerald-200/60"
              : p.requires_key
                ? "bg-rose-50 text-rose-600 border-rose-200/60"
                : "bg-[var(--surface-2)] text-[var(--ink-3)] border-[var(--line)]"
          }`}
        >
          {p.has_key
            ? keySourceLabel(p.key_source) || "Key Configured"
            : p.requires_key
              ? "Missing Key"
              : t("llm.noKeyRequired", { defaultValue: "无需密钥" })}
        </span>
      </div>

      {/* 地址配置 */}
      {p.base_url !== "" && (
        <div className="flex flex-col gap-1">
          <label className="text-[11.5px] font-bold text-[var(--ink-2)]">{t("llm.apiAddress", { defaultValue: "API 地址" })}</label>
          <div className="flex gap-2">
            <input
              className="flex-1 bg-[var(--surface)] border border-[var(--line)] focus:border-[var(--celadon)] focus:ring-2 focus:ring-[var(--celadon)]/15 rounded-xl px-3 py-1.5 outline-none text-[12.5px] text-[var(--ink)] disabled:opacity-50 transition-all shadow-2xs font-mono"
              value={draftUrl}
              disabled={disabled}
              onChange={(e) => onDraftUrl(e.target.value)}
            />
            <button
              type="button"
              className="bg-[var(--surface-2)] hover:bg-[var(--surface-3)] text-[var(--ink)] border border-[var(--line)] rounded-xl px-3.5 py-1.5 text-[12px] font-medium transition-colors disabled:opacity-40 cursor-pointer shadow-2xs shrink-0"
              disabled={disabled || draftUrl === p.base_url}
              onClick={onSaveUrl}
            >
              {t("llm.saveAddress", { defaultValue: "保存" })}
            </button>
          </div>
        </div>
      )}

      {/* 密钥配置 */}
      {p.requires_key && (
        <div className="flex flex-col gap-1">
          <label className="text-[11.5px] font-bold text-[var(--ink-2)] flex flex-col">
            <span>{t("llm.apiKey", { defaultValue: "API 密钥" })}</span>
            <span className="font-normal text-[10.5px] text-[var(--ink-3)] mt-0.5 leading-tight">
              {keySourceHint(p.key_source)}
            </span>
          </label>
          <div className="flex gap-2 mt-0.5">
            <input
              className="flex-1 bg-[var(--surface)] border border-[var(--line)] focus:border-[var(--celadon)] focus:ring-2 focus:ring-[var(--celadon)]/15 rounded-xl px-3 py-1.5 outline-none text-[12.5px] text-[var(--ink)] disabled:opacity-50 transition-all shadow-2xs font-mono"
              type="password"
              placeholder={
                p.key_from_env
                  ? "From .env.local"
                  : p.has_key
                    ? "••••••••••••••••"
                    : "Paste API Key…"
              }
              value={draftKey}
              disabled={disabled || p.key_from_env}
              onChange={(e) => onDraftKey(e.target.value)}
            />
            <button
              type="button"
              className="bg-[var(--surface-2)] hover:bg-[var(--surface-3)] text-[var(--ink)] border border-[var(--line)] rounded-xl px-3.5 py-1.5 text-[12px] font-medium transition-colors disabled:opacity-40 cursor-pointer shadow-2xs shrink-0"
              disabled={disabled || p.key_from_env || draftKey.trim() === ""}
              onClick={onSaveKey}
            >
              {t("common:save", { defaultValue: "保存" })}
            </button>
          </div>
        </div>
      )}

      {/* 模型列表 */}
      <div className="flex flex-col gap-1 border-t border-[var(--line)]/50 pt-2.5 mt-0.5">
        <label className="text-[11.5px] font-bold text-[var(--ink-2)]">{t("llm.modelsList", { defaultValue: "可用模型列表" })}</label>
        <div className="flex items-center justify-between gap-2">
          <span className="text-[11.5px] text-[var(--ink-3)] flex-1 leading-tight">
            {models?.status === "loading"
              ? "Fetching models…"
              : models?.status === "error" && list.length > 0
                ? // 刷新失败但还有上次的列表：**错误要说出来**，不然看着像刷新成功了
                  `刷新失败（下面还是上次拉到的 ${list.length} 个）：${models.error}`
                : list.length > 0
                  ? t("llm.modelsFound", { count: list.length, defaultValue: `已发现 ${list.length} 个可用模型` })
                  : `No models found${models?.error ? ` (${models.error})` : ""}`}
          </span>
          <button
            type="button"
            className="shrink-0 bg-[var(--surface-2)] hover:bg-[var(--surface-3)] text-[var(--ink-2)] hover:text-[var(--ink)] border border-[var(--line)] rounded-lg px-2.5 py-1 text-[11.5px] font-medium transition-colors disabled:opacity-50 cursor-pointer shadow-2xs"
            disabled={disabled}
            onClick={onRefresh}
          >
            {t("llm.refreshModels", { defaultValue: "刷新列表" })}
          </button>
        </div>
      </div>

      {!p.supports_tools && (
        <div className="text-[11px] text-amber-700 bg-amber-50 border border-amber-200/60 px-3 py-2 rounded-xl mt-0.5 leading-snug">
          {t("llm.unsupportedToolsWarning", { defaultValue: "⚠️ 该后端不支持原生流式工具调用，遇到 Agent 工具任务将自动调度至备用链路。" })}
        </div>
      )}
    </div>
  );
}
