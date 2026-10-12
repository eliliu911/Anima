import { useTranslation } from "react-i18next";
/**
 * MCP server 的增删改与权限设置：现代 Antigravity 纯白卡片设计
 */
import { useEffect, useState } from "react";
import {
  defaultPermissions,
  mcpApi,
  type Access,
  type ApprovalMode,
  type McpServerConfig,
  type McpView,
  type Operation,
  type ServerStatus,
} from "../api/mcp";

const ACCESS_OPTIONS: { value: Access; label: string }[] = [
  { value: "allow", label: "放行" },
  { value: "ask", label: "需确认" },
  { value: "deny", label: "禁用" },
];

const OPERATIONS: { key: Operation; label: string; hint: string }[] = [
  { key: "read", label: "读取", hint: "查询、列举、获取" },
  { key: "write", label: "写入", hint: "创建、更新、发送" },
  { key: "delete", label: "删除", hint: "不可逆，默认禁用" },
  { key: "execute", label: "执行", hint: "跑命令、脚本" },
];

function blankServer(): McpServerConfig {
  return {
    id: "",
    label: "",
    enabled: true,
    transport: {
      kind: "stdio",
      command: "",
      args: [],
      env: {},
      env_from_secrets: {},
    },
    permissions: defaultPermissions(),
    approval_mode: "conservative",
    commands: { allow_prefixes: [], deny_substrings: [] },
    startup_timeout_secs: 30,
  };
}

export function McpSettings() {
  const { t } = useTranslation(["settings", "common"]);
  const [view, setView] = useState<McpView | null>(null);
  const [draft, setDraft] = useState<McpServerConfig | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  useEffect(() => {
    void refresh();
  }, []);

  async function refresh() {
    try {
      setView(await mcpApi.view());
    } catch (err) {
      setMessage(`读取失败：${err}`);
    }
  }

  function statusOf(id: string): ServerStatus | undefined {
    return view?.statuses.find((s) => s.id === id);
  }

  async function run(action: () => Promise<ServerStatus[]>) {
    setBusy(true);
    setMessage(null);
    try {
      const statuses = await action();
      await refresh();
      const failed = statuses.filter((s) => !s.connected);
      setMessage(
        failed.length === 0
          ? `已成功连接 ${statuses.length} 个 server`
          : failed.map((s) => `${s.label}：${s.error}`).join("；"),
      );
      setTimeout(() => setMessage(null), 3000);
    } catch (err) {
      setMessage(`操作失败：${err}`);
    } finally {
      setBusy(false);
    }
  }

  async function save() {
    if (!draft) return;
    if (!draft.id.trim() || !draft.label.trim()) {
      setMessage("id 和名称都不能为空");
      return;
    }
    await run(() => mcpApi.upsertServer(draft));
    setDraft(null);
  }

  if (!view) {
    return (
      <div className="flex items-center justify-center py-12 text-[13px] text-[var(--ink-3)]">
        加载 MCP 配置中…
      </div>
    );
  }

  const servers = view.servers;

  return (
    <div className="flex flex-col gap-4 max-w-4xl pb-8">
      <div className="flex flex-col">
        <span className="text-[14px] font-bold text-[var(--ink)] tracking-tight">{t("mcp.title", { defaultValue: "MCP 工具接入" })}</span>
        <span className="text-[12px] text-[var(--ink-2)] mt-0.5 leading-relaxed">
          {t("mcp.desc", { defaultValue: "管理 Model Context Protocol (MCP) 服务与工具执行安全策略。" })}
        </span>
      </div>

      <div className="p-3.5 px-4 rounded-2xl bg-[var(--paper)] border border-[var(--line)] shadow-2xs text-[12px] leading-relaxed text-[var(--ink-2)]">
        {t("mcp.policyBanner", { defaultValue: "接入一个 server 等于授权它在本机运行并被模型调用。默认策略：读取放行、写入需确认、删除禁用、执行需确认；高危操作（抹盘、rm -rf / 等）系统均会严格拦截。" })}
      </div>

      {servers.length === 0 && !draft && (
        <div className="text-[12.5px] text-[var(--ink-3)] text-center py-6 border border-dashed border-[var(--line)] rounded-2xl bg-[var(--paper)] shadow-2xs">
          {t("mcp.noServers", { defaultValue: "尚未配置任何 MCP Server。" })}
        </div>
      )}

      {servers.length > 0 && (
        <div className="flex flex-col gap-3">
          {servers.map((server) => {
            const status = statusOf(server.id);
            const displayLabel =
              server.id === "filesystem"
                ? t("mcp.serverLabels.filesystem", { defaultValue: server.label })
                : server.id === "system"
                  ? t("mcp.serverLabels.system", { defaultValue: server.label })
                  : server.label;

            return (
              <div
                className="flex flex-col sm:flex-row sm:items-center justify-between gap-3 p-4 bg-[var(--paper)] border border-[var(--line)] rounded-2xl shadow-2xs transition-all hover:border-[var(--ink-3)]/40"
                key={server.id}
              >
                <div className="flex items-center flex-wrap gap-2.5">
                  <span className="text-[13.5px] font-bold text-[var(--ink)]">{displayLabel}</span>
                  <span className="text-[11px] font-mono text-[var(--ink-3)] bg-[var(--surface)] px-2 py-0.5 rounded-lg border border-[var(--line)]">
                    {server.id}
                  </span>
                  {status && (
                    <span
                      className={`text-[11px] px-2.5 py-0.5 rounded-full font-medium border ${
                        status.connected
                          ? "bg-sky-50 text-sky-600 border-sky-200/60"
                          : "bg-amber-50 text-amber-600 border-amber-200/60"
                      }`}
                    >
                      {status.connected ? `${status.tool_count} 个工具` : status.error ?? "不可用"}
                    </span>
                  )}
                  {!server.enabled && (
                    <span className="text-[10.5px] px-2 py-0.5 rounded-full border bg-[var(--surface-2)] text-[var(--ink-3)] border-[var(--line)]">
                      已停用
                    </span>
                  )}
                </div>

                <div className="flex gap-2 shrink-0">
                  <button
                    type="button"
                    className="px-3 py-1.5 text-[12px] font-medium bg-[var(--surface)] hover:bg-[var(--surface-2)] border border-[var(--line)] rounded-xl shadow-2xs text-[var(--ink)] transition-colors disabled:opacity-50 cursor-pointer"
                    onClick={() => setDraft({ ...server })}
                    disabled={busy}
                  >
                    {t("mcp.edit", { defaultValue: "编辑" })}
                  </button>
                  <button
                    type="button"
                    className="px-3 py-1.5 text-[12px] font-medium bg-rose-50 hover:bg-rose-100 border border-rose-200/60 rounded-xl shadow-2xs text-rose-600 transition-colors disabled:opacity-50 cursor-pointer"
                    onClick={() => void run(() => mcpApi.removeServer(server.id))}
                    disabled={busy}
                    title={`${t("mcp.remove", { defaultValue: "移除" })} server`}
                  >
                    {t("mcp.remove", { defaultValue: "移除" })}
                  </button>
                </div>
              </div>
            );
          })}
        </div>
      )}

      <div className="flex gap-2 mt-1">
        <button
          type="button"
          className="px-4 py-2 text-[12px] font-semibold bg-[var(--surface-2)] hover:bg-[var(--surface-3)] border border-[var(--line)] rounded-xl text-[var(--ink)] shadow-2xs transition-colors disabled:opacity-50 cursor-pointer"
          onClick={() => setDraft(blankServer())}
          disabled={busy || !!draft}
        >
          {t("mcp.addServer", { defaultValue: "+ 添加 server" })}
        </button>
        <button
          type="button"
          className="px-4 py-2 text-[12px] font-semibold bg-[var(--surface-2)] hover:bg-[var(--surface-3)] border border-[var(--line)] rounded-xl text-[var(--ink)] shadow-2xs transition-colors disabled:opacity-50 cursor-pointer"
          onClick={() => void run(() => mcpApi.reconnect())}
          disabled={busy}
        >
          {t("mcp.reconnectAll", { defaultValue: "全部重连" })}
        </button>
      </div>

      {draft && (
        <div className="flex flex-col gap-4 p-5 bg-[var(--paper)] border border-[var(--line)] rounded-2xl mt-3 shadow-md">
          <div className="text-[14px] font-bold text-[var(--ink)]">
            {servers.some((s) => s.id === draft.id) ? "编辑 Server" : "新增 Server"}
          </div>

          <label className="flex flex-col gap-1.5">
            <span className="text-[11.5px] font-bold text-[var(--ink-2)]">
              ID 标识（工具名前缀，英文字母与连字符）
            </span>
            <input
              className="bg-[var(--surface)] border border-[var(--line)] focus:border-[var(--celadon)] focus:ring-2 focus:ring-[var(--celadon)]/15 rounded-xl px-3 py-1.5 outline-none text-[12.5px] text-[var(--ink)] shadow-2xs font-mono"
              value={draft.id}
              onChange={(e) => setDraft({ ...draft, id: e.target.value })}
              placeholder="filesystem"
            />
          </label>

          <label className="flex flex-col gap-1.5">
            <span className="text-[11.5px] font-bold text-[var(--ink-2)]">显示名称</span>
            <input
              className="bg-[var(--surface)] border border-[var(--line)] focus:border-[var(--celadon)] focus:ring-2 focus:ring-[var(--celadon)]/15 rounded-xl px-3 py-1.5 outline-none text-[12.5px] text-[var(--ink)] shadow-2xs"
              value={draft.label}
              onChange={(e) => setDraft({ ...draft, label: e.target.value })}
              placeholder="本地文件"
            />
          </label>

          {draft.transport.kind === "stdio" && (
            <>
              <label className="flex flex-col gap-1.5">
                <span className="text-[11.5px] font-bold text-[var(--ink-2)]">启动命令</span>
                <input
                  className="bg-[var(--surface)] border border-[var(--line)] focus:border-[var(--celadon)] focus:ring-2 focus:ring-[var(--celadon)]/15 rounded-xl px-3 py-1.5 outline-none text-[12.5px] font-mono text-[var(--ink)] shadow-2xs"
                  value={draft.transport.command}
                  onChange={(e) =>
                    setDraft({
                      ...draft,
                      transport: { ...draft.transport, command: e.target.value } as never,
                    })
                  }
                  placeholder="npx"
                />
              </label>

              <label className="flex flex-col gap-1.5">
                <span className="text-[11.5px] font-bold text-[var(--ink-2)]">
                  命令参数（每行一个）
                </span>
                <textarea
                  className="bg-[var(--surface)] border border-[var(--line)] focus:border-[var(--celadon)] focus:ring-2 focus:ring-[var(--celadon)]/15 rounded-xl px-3 py-2 outline-none text-[12.5px] font-mono text-[var(--ink)] resize-y shadow-2xs"
                  rows={3}
                  value={draft.transport.args.join("\n")}
                  onChange={(e) =>
                    setDraft({
                      ...draft,
                      transport: {
                        ...draft.transport,
                        args: e.target.value.split("\n").filter((a) => a.length > 0),
                      } as never,
                    })
                  }
                  placeholder={"-y\n@modelcontextprotocol/server-filesystem\n/Users/me/Documents"}
                />
              </label>
            </>
          )}

          <div className="flex flex-col gap-2.5 border border-[var(--line)] rounded-2xl p-4 bg-[var(--surface)] shadow-2xs">
            <div className="text-[12px] font-bold text-[var(--ink)] mb-0.5">权限安全策略</div>
            {OPERATIONS.map((op) => (
              <div className="flex items-center justify-between gap-4" key={op.key}>
                <span className="flex flex-col text-[12.5px] text-[var(--ink)]">
                  <span className="font-semibold">{op.label}</span>
                  <em className="not-italic text-[11px] text-[var(--ink-3)] leading-tight">{op.hint}</em>
                </span>
                <select
                  className="bg-[var(--paper)] border border-[var(--line)] rounded-xl px-3 py-1.5 outline-none text-[12px] text-[var(--ink)] shadow-2xs cursor-pointer"
                  value={draft.permissions[op.key]}
                  onChange={(e) =>
                    setDraft({
                      ...draft,
                      permissions: {
                        ...draft.permissions,
                        [op.key]: e.target.value as Access,
                      },
                    })
                  }
                >
                  {ACCESS_OPTIONS.map((o) => (
                    <option key={o.value} value={o.value}>
                      {o.label}
                    </option>
                  ))}
                </select>
              </div>
            ))}
          </div>

          <label className="flex flex-col gap-1.5">
            <span className="text-[11.5px] font-bold text-[var(--ink-2)]">审批模式</span>
            <select
              className="bg-[var(--surface)] border border-[var(--line)] focus:border-[var(--celadon)] rounded-xl px-3 py-1.5 outline-none text-[12.5px] text-[var(--ink)] shadow-2xs cursor-pointer"
              value={draft.approval_mode}
              onChange={(e) =>
                setDraft({ ...draft, approval_mode: e.target.value as ApprovalMode })
              }
            >
              <option value="conservative">保守模式 —— 白名单之外均需用户确认</option>
              <option value="auto">自动放行 —— 读写免问（高危指令仍会拦截）</option>
            </select>
          </label>

          <label className="flex items-center gap-2 mt-1 cursor-pointer w-fit">
            <div className="relative inline-block w-9 h-5 shrink-0">
              <input
                type="checkbox"
                className="peer appearance-none w-9 h-5 bg-[var(--surface-3)] border border-[var(--line)] rounded-full checked:bg-[var(--celadon)] checked:border-[var(--celadon)] cursor-pointer transition-colors"
                checked={draft.enabled}
                onChange={(e) => setDraft({ ...draft, enabled: e.target.checked })}
              />
              <span className="absolute left-[3px] top-[3px] w-[14px] h-[14px] bg-white rounded-full pointer-events-none transition-transform peer-checked:translate-x-4 shadow-xs" />
            </div>
            <span className="text-[12.5px] text-[var(--ink)] font-bold select-none">启用此 Server</span>
          </label>

          <div className="flex gap-2 justify-end mt-2 pt-3 border-t border-[var(--line)]/50">
            <button
              type="button"
              className="px-4 py-2 text-[12px] font-medium bg-[var(--surface-2)] hover:bg-[var(--surface-3)] border border-[var(--line)] rounded-xl text-[var(--ink)] transition-colors disabled:opacity-50 cursor-pointer shadow-2xs"
              onClick={() => setDraft(null)}
              disabled={busy}
            >
              取消
            </button>
            <button
              type="button"
              className="px-4 py-2 text-[12px] font-semibold bg-[var(--celadon)] hover:opacity-90 text-white rounded-xl shadow-2xs disabled:opacity-50 cursor-pointer"
              onClick={() => void save()}
              disabled={busy}
            >
              保存并连接
            </button>
          </div>
        </div>
      )}

      {message && (
        <div className="fixed bottom-6 right-6 z-50 text-[12.5px] text-[var(--ink)] bg-[var(--paper)] border border-[var(--celadon)] shadow-lg px-4 py-2.5 rounded-2xl flex items-center gap-2 animate-in fade-in slide-in-from-bottom-2">
          <span className="w-2 h-2 rounded-full bg-[var(--celadon)] animate-pulse" />
          <span>{message}</span>
        </div>
      )}
    </div>
  );
}
