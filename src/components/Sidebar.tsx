import { useEffect, useState, useRef } from "react";
import { useTranslation } from "react-i18next";
import { isInsideMenu } from "./sessionMenu";
import { useChat } from "../store/chat";
import { useProject } from "../store/project";
import { openPanel, setAvatarVisible, getAvatarVisible } from "../api/voice";
import { listen } from "@tauri-apps/api/event";
import { ProjectSettings } from "./ProjectSettings";

function formatRelativeTime(timestamp?: number | null): string {
  if (!timestamp) return "";
  const ms = timestamp < 1e11 ? timestamp * 1000 : timestamp;
  const diffSec = Math.max(0, Math.floor((Date.now() - ms) / 1000));

  if (diffSec < 60) return "now";
  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 60) return `${diffMin}m`;
  const diffHours = Math.floor(diffMin / 60);
  if (diffHours < 24) return `${diffHours}h`;
  const diffDays = Math.floor(diffHours / 24);
  if (diffDays < 30) return `${diffDays}d`;
  const diffMonths = Math.floor(diffDays / 30);
  if (diffMonths < 12) return `${diffMonths}mo`;
  const diffYears = Math.floor(diffDays / 365);
  return `${diffYears}y`;
}

export function Sidebar({ open, onToggle }: { open: boolean; onToggle: () => void }) {
  const { t } = useTranslation(["settings", "chat", "common"]);
  const { 
    projects, sessions, activeProjectId, activeSessionId,
    loadProjects, createProject, setActiveProject,
    createSession, setActiveSession, deleteSession, updateSessionTitle,
    loadSessions
  } = useProject();

  const loadSession = useChat((s) => s.loadSession);
  const clearChat = useChat((s) => s.clear);
  const turns = useChat((s) => s.turns);
  const bottomRef = useRef<HTMLDivElement>(null);

  const [settingsModalProjectId, setSettingsModalProjectId] = useState<string | null>(null);
  const [isAvatarVisible, setIsAvatarVisible] = useState(true);
  const [isEditingProject, setIsEditingProject] = useState(false);
  const [newProjectName, setNewProjectName] = useState("");
  const [viewMode, setViewMode] = useState<"list" | "chat">("list");
  // 「⋯」菜单开在哪一条上；重命名中的是哪一条
  const [menuFor, setMenuFor] = useState<string | null>(null);
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [draftTitle, setDraftTitle] = useState("");

  // 点别处要关掉菜单。挂在 document 上而不是给每行加遮罩
  useEffect(() => {
    if (!menuFor) return;
    const close = (e: MouseEvent) => {
      if (isInsideMenu(e.target)) return;
      setMenuFor(null);
    };
    document.addEventListener("mousedown", close);
    return () => document.removeEventListener("mousedown", close);
  }, [menuFor]);

  // 实时同步虚拟人/桌宠的显隐状态（初次加载从后端读，之后监听托盘及多端联动事件）
  useEffect(() => {
    void getAvatarVisible().then((vis) => setIsAvatarVisible(vis)).catch(() => {});
    const unlistenPromise = listen<boolean>("avatar-visibility-changed", (event) => {
      setIsAvatarVisible(event.payload);
    });
    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);

  useEffect(() => {
    loadProjects();
    loadSessions(activeProjectId);
  }, [loadProjects, loadSessions, activeProjectId]);

  useEffect(() => {
    if (activeSessionId) {
      loadSession(activeSessionId);
    }
  }, [activeSessionId, loadSession]);

  // Scroll to bottom in chat mode
  useEffect(() => {
    if (viewMode === "chat") {
      bottomRef.current?.scrollIntoView({ block: "end" });
    }
  }, [turns.length, turns[turns.length - 1]?.content, viewMode]);

  const handleCreateProject = () => {
    if (newProjectName.trim()) {
      createProject(newProjectName.trim());
      setNewProjectName("");
      setIsEditingProject(false);
    }
  };

  const mainView = useProject((s) => s.mainView);
  const setMainView = useProject((s) => s.setMainView);

  const handleNewSession = async (projectId: string | null = null) => {
    clearChat();
    if (projectId !== activeProjectId) {
      setActiveProject(projectId);
    }
    await createSession(projectId);
    setMainView("chat");
    setViewMode("chat");
  };

  // 空标题就当没改（回到自动标题），不要写一个空名字进去
  const commitRename = (id: string) => {
    const next = draftTitle.trim();
    setRenamingId(null);
    if (next) updateSessionTitle(id, next);
  };

  const handleSelectSession = (id: string) => {
    const target = sessions.find((s) => s.id === id);
    if (target) {
      if (target.project_id) {
        if (activeProjectId !== target.project_id) {
          setActiveProject(target.project_id);
        }
      } else {
        if (activeProjectId !== null) {
          setActiveProject(null);
        }
      }
    }
    setActiveSession(id);
    loadSession(id);
    setMainView("chat");
    setViewMode("chat");
  };

  return (
    <div className={`flex flex-col w-[240px] shrink-0 h-full bg-[var(--surface-2)] border-r border-[var(--line)] ${!open ? "hidden" : ""}`}>
      {/* Header (支持 macOS 顶栏拖拽并避让红绿灯，移除文字标题避免遮挡系统交通灯) */}
      <div data-tauri-drag-region className="pt-3 pb-2 px-3 flex justify-end items-center shrink-0 select-none min-h-[34px]">
        {onToggle && (
          <button
            type="button"
            onClick={onToggle}
            className="p-1 rounded-lg text-[var(--ink-3)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)]/60 transition-colors cursor-pointer"
            title={t("chat:hideSidebar", { defaultValue: "隐藏侧边栏" })}
            aria-label={t("chat:hideSidebar", { defaultValue: "隐藏侧边栏" })}
          >
            <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M15 18l-6-6 6-6" /></svg>
          </button>
        )}
      </div>

      <div className="flex-1 overflow-y-auto p-2 flex flex-col gap-3">
        {/* 顶部主快捷操作区：新对话 + 任务安排 (严格符合左侧列表设计规范) */}
        <div className="flex flex-col gap-0.5 select-none">
          {/* New Conversation 按钮 */}
          <button
            type="button"
            onClick={() => handleNewSession(activeProjectId)}
            className="w-full flex items-center gap-2 px-2.5 py-1.5 rounded-lg text-[12.5px] font-medium text-[var(--ink-2)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)]/60 transition-colors cursor-pointer text-left"
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="opacity-70 shrink-0">
              <path d="M12 5v14M5 12h14" />
            </svg>
            <span className="truncate">{t("chat:newChat", { defaultValue: "新对话" })}</span>
          </button>

          {/* 任务安排 (Scheduled Tasks) 标签页按钮 */}
          <button
            type="button"
            onClick={() => setMainView("tasks")}
            className={`w-full flex items-center gap-2 px-2.5 py-1.5 rounded-lg text-[12.5px] text-left transition-colors cursor-pointer ${
              mainView === "tasks"
                ? "bg-[var(--surface-3)] text-[var(--ink)] font-semibold shadow-2xs"
                : "text-[var(--ink-2)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)]/60"
            }`}
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="opacity-70 shrink-0">
              <circle cx="12" cy="12" r="10" />
              <polyline points="12 6 12 12 16 14" />
            </svg>
            <span className="truncate">{t("nav.scheduledTasks", { defaultValue: "任务安排" })}</span>
          </button>
        </div>

        {/* PROJECTS SECTION */}
        <div className="flex flex-col gap-1">
          <div className="flex justify-between items-center px-2 py-1 text-[var(--ink-3)] text-[11px] font-bold uppercase tracking-wider">
            <span>{t("nav.projects", { defaultValue: "Projects" })}</span>
            <button title={t("nav.newProject", { defaultValue: "新建项目" })} onClick={() => setIsEditingProject(true)} className="hover:text-[var(--ink)] cursor-pointer">
              <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M12 5v14M5 12h14" /></svg>
            </button>
          </div>

          {isEditingProject && (
            <input
              className="w-[calc(100%-8px)] mx-1 my-1 bg-[var(--surface)] border border-[var(--celadon)] text-[var(--ink)] rounded-lg p-1 px-2 text-[12px] outline-none shadow-xs"
              autoFocus
              value={newProjectName}
              placeholder={t("project.newPlaceholder", { defaultValue: "项目名称，回车确认" })}
              onChange={(e) => setNewProjectName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") handleCreateProject();
                if (e.key === "Escape") { setIsEditingProject(false); setNewProjectName(""); }
              }}
              onBlur={() => { setIsEditingProject(false); setNewProjectName(""); }}
            />
          )}

          {projects.length === 0 && !isEditingProject && (
            <div className="px-3 py-1 text-[var(--ink-3)] text-[11.5px]">{t("project.empty", { defaultValue: "还没有项目" })}</div>
          )}

          <ul className="m-0 p-0 list-none flex flex-col gap-0.5">
            {projects.map((p) => {
              const isActive = activeProjectId === p.id;
              return (
                <li key={p.id}>
                  <div
                    className="flex items-center px-2.5 py-1.5 rounded-lg cursor-pointer group text-[12.5px] text-[var(--ink-2)] hover:bg-[var(--surface-3)]/50 hover:text-[var(--ink)] transition-colors select-none"
                    onClick={() => setActiveProject(isActive ? null : p.id)}
                  >
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="mr-2 opacity-70 shrink-0">
                      <path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z" />
                    </svg>
                    <span className="flex-1 overflow-hidden text-ellipsis whitespace-nowrap">{p.name}</span>
                    
                    <div className="flex gap-1 opacity-0 group-hover:opacity-100 transition-opacity">
                      <button title={t("chat:projectMenuTooltip", { defaultValue: "项目菜单" })} onClick={(e) => { e.stopPropagation(); setMenuFor(menuFor === p.id ? null : p.id); }} className="p-0.5 hover:bg-[var(--surface)] rounded relative text-[var(--ink-3)] hover:text-[var(--ink)] cursor-pointer">
                        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><circle cx="12" cy="12" r="1"/><circle cx="12" cy="5" r="1"/><circle cx="12" cy="19" r="1"/></svg>
                        {menuFor === p.id && (
                          <div className="sessionmenu absolute right-0 top-full mt-1 bg-[var(--surface)] border border-[var(--line)] shadow-lg rounded-xl py-1 z-10 w-36 flex flex-col">
                            <button type="button" className="flex items-center gap-2 px-3 py-1.5 text-[12px] text-[var(--ink)] hover:bg-[var(--surface-2)] cursor-pointer" onClick={(e) => { e.stopPropagation(); setMenuFor(null); setSettingsModalProjectId(p.id); }}>
                              {t("chat:projectSettings", { defaultValue: "⚙️ 项目设置与目录" })}
                            </button>
                          </div>
                        )}
                      </button>
                      <button title={t("chat:newSessionTooltip", { defaultValue: "新建会话" })} onClick={(e) => { e.stopPropagation(); handleNewSession(p.id); }} className="p-0.5 hover:bg-[var(--surface)] rounded text-[var(--ink-3)] hover:text-[var(--ink)] cursor-pointer">
                        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M12 5v14M5 12h14" /></svg>
                      </button>
                    </div>
                  </div>

                  {/* Render Project's Sessions if expanded */}
                  {isActive && (
                    <ul className="m-0 pl-4 py-1 flex flex-col gap-0.5 list-none">
                      {sessions.filter((s) => s.project_id === p.id).map((s) => {
                        const isSessionActive = activeSessionId === s.id;
                        return (
                          <li key={s.id}>
                            <div className={`flex items-center justify-between px-2.5 py-1.5 rounded-lg cursor-pointer group text-[12.5px] transition-colors ${isSessionActive ? "bg-[var(--surface-3)] text-[var(--ink)] font-semibold shadow-2xs" : "text-[var(--ink-2)] hover:bg-[var(--surface-3)]/60 hover:text-[var(--ink)]"}`} onClick={() => handleSelectSession(s.id)}>
                              {renamingId === s.id ? (
                                <input
                                  className="flex-1 bg-transparent border-b border-[var(--celadon)] outline-none text-[var(--ink)] min-w-0"
                                  value={draftTitle}
                                  autoFocus
                                  onChange={(e) => setDraftTitle(e.target.value)}
                                  onKeyDown={(e) => {
                                    if (e.key === "Enter") commitRename(s.id);
                                    if (e.key === "Escape") setRenamingId(null);
                                  }}
                                  onBlur={() => commitRename(s.id)}
                                />
                              ) : (
                                <span className="flex-1 overflow-hidden text-ellipsis whitespace-nowrap mr-1.5">{s.title || t("chat:newChat", { defaultValue: "新对话" })}</span>
                              )}

                              <div className="flex items-center gap-1 shrink-0">
                                {isSessionActive && !renamingId && (
                                  <span className="w-1.5 h-1.5 rounded-full bg-[var(--celadon)] shrink-0 group-hover:hidden" />
                                )}

                                {!isSessionActive && (
                                  <span className="text-[11px] text-[var(--ink-3)] font-mono group-hover:hidden select-none transition-opacity">
                                    {formatRelativeTime(s.started_at)}
                                  </span>
                                )}

                                <div className={`relative ${menuFor === s.id ? "flex" : "hidden group-hover:flex"}`}>
                                  <button onClick={(e) => { e.stopPropagation(); setMenuFor(menuFor === s.id ? null : s.id); }} className="p-0.5 hover:bg-[var(--surface)] rounded text-[var(--ink-3)] hover:text-[var(--ink)] cursor-pointer">
                                    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><circle cx="12" cy="12" r="1"/><circle cx="12" cy="5" r="1"/><circle cx="12" cy="19" r="1"/></svg>
                                  </button>

                                  {menuFor === s.id && (
                                    <div className="sessionmenu absolute right-0 top-full mt-1 bg-[var(--surface)] border border-[var(--line)] shadow-lg rounded-xl py-1 z-10 w-32 flex flex-col">
                                      <button type="button" className="flex items-center gap-2 px-3 py-1.5 text-[12px] text-[var(--ink)] hover:bg-[var(--surface-2)] cursor-pointer" onClick={(e) => { e.stopPropagation(); setDraftTitle(s.title || ""); setRenamingId(s.id); setMenuFor(null); }}>
                                        {t("chat:rename", { defaultValue: "重命名" })}
                                      </button>
                                      <button type="button" className="flex items-center gap-2 px-3 py-1.5 text-[12px] text-[var(--clay)] hover:bg-[var(--surface-2)] cursor-pointer" onClick={(e) => { e.stopPropagation(); setMenuFor(null); deleteSession(s.id); }}>
                                        {t("common:delete", { defaultValue: "删除" })}
                                      </button>
                                    </div>
                                  )}
                                </div>
                              </div>
                            </div>
                          </li>
                        );
                      })}
                      {sessions.filter((s) => s.project_id === p.id).length === 0 && (
                        <li className="px-2.5 py-1 text-[11px] text-[var(--ink-3)]">{t("chat:noProjectSessions", { defaultValue: "暂无项目对话" })}</li>
                      )}
                    </ul>
                  )}
                </li>
              );
            })}
          </ul>
        </div>

        {/* CONVERSATIONS SECTION (仅展示不属于任何项目的独立对话) */}
        <div className="mb-4">
          <div className="flex justify-between items-center px-2 py-1 text-[var(--ink-3)] text-[11px] font-bold uppercase tracking-wider">
            <span>Conversations</span>
            <button title={t("chat:newSessionTooltip", { defaultValue: "新建对话" })} onClick={() => handleNewSession(null)} className="hover:text-[var(--ink)] cursor-pointer">
              <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M12 5v14M5 12h14" /></svg>
            </button>
          </div>

          {sessions.filter((s) => !s.project_id).length === 0 ? (
            <div className="px-3 py-1 text-[var(--ink-3)] text-[11.5px]">{t("chat:noIndependentSessions", { defaultValue: "还没有独立对话" })}</div>
          ) : (
            <ul className="m-0 p-0 list-none flex flex-col gap-0.5">
              {sessions.filter((s) => !s.project_id).map((s) => {
                const isSessionActive = activeSessionId === s.id && activeProjectId === null;
                return (
                  <li key={s.id}>
                    <div className={`flex items-center justify-between px-2.5 py-1.5 rounded-lg cursor-pointer group text-[12.5px] transition-colors ${isSessionActive ? "bg-[var(--surface-3)] text-[var(--ink)] font-semibold shadow-2xs" : "text-[var(--ink-2)] hover:bg-[var(--surface-3)]/60 hover:text-[var(--ink)]"}`} onClick={() => handleSelectSession(s.id)}>
                      {renamingId === s.id ? (
                        <input
                          className="flex-1 bg-transparent border-b border-[var(--celadon)] outline-none text-[var(--ink)] min-w-0"
                          value={draftTitle}
                          autoFocus
                          onChange={(e) => setDraftTitle(e.target.value)}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") commitRename(s.id);
                            if (e.key === "Escape") setRenamingId(null);
                          }}
                          onBlur={() => commitRename(s.id)}
                        />
                      ) : (
                        <span className="flex-1 overflow-hidden text-ellipsis whitespace-nowrap mr-1.5">{s.title || t("chat:newChat", { defaultValue: "新对话" })}</span>
                      )}

                      <div className="flex items-center gap-1 shrink-0">
                        {isSessionActive && !renamingId && (
                          <span className="w-1.5 h-1.5 rounded-full bg-[var(--celadon)] shrink-0 group-hover:hidden" />
                        )}

                        {!isSessionActive && (
                          <span className="text-[11px] text-[var(--ink-3)] font-mono group-hover:hidden select-none transition-opacity">
                            {formatRelativeTime(s.started_at)}
                          </span>
                        )}

                        <div className={`relative ${menuFor === s.id ? "flex" : "hidden group-hover:flex"}`}>
                          <button onClick={(e) => { e.stopPropagation(); setMenuFor(menuFor === s.id ? null : s.id); }} className="p-0.5 hover:bg-[var(--surface)] rounded text-[var(--ink-3)] hover:text-[var(--ink)] cursor-pointer">
                            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><circle cx="12" cy="12" r="1"/><circle cx="12" cy="5" r="1"/><circle cx="12" cy="19" r="1"/></svg>
                          </button>

                          {menuFor === s.id && (
                            <div className="sessionmenu absolute right-0 top-full mt-1 bg-[var(--surface)] border border-[var(--line)] shadow-lg rounded-xl py-1 z-10 w-32 flex flex-col">
                              <button type="button" className="flex items-center gap-2 px-3 py-1.5 text-[12px] text-[var(--ink)] hover:bg-[var(--surface-2)] cursor-pointer" onClick={(e) => { e.stopPropagation(); setDraftTitle(s.title || ""); setRenamingId(s.id); setMenuFor(null); }}>
                                重命名
                              </button>
                              <button type="button" className="flex items-center gap-2 px-3 py-1.5 text-[12px] text-[var(--clay)] hover:bg-[var(--surface-2)] cursor-pointer" onClick={(e) => { e.stopPropagation(); setMenuFor(null); deleteSession(s.id); }}>
                                删除
                              </button>
                            </div>
                          )}
                        </div>
                      </div>
                    </div>
                  </li>
                );
              })}
            </ul>
          )}
        </div>
      </div>

      {/* 底部设置入口 + 虚拟人显隐控制 (对齐 Antigravity 沉浸设计) */}
      <div className="p-2 shrink-0 select-none flex items-center gap-1.5">
        <button
          type="button"
          onClick={() => void openPanel("settings")}
          className="flex-1 flex items-center gap-2 px-2.5 py-1.5 rounded-lg text-[12.5px] font-medium text-[var(--ink-2)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)]/60 transition-colors cursor-pointer"
        >
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="opacity-70 shrink-0">
            <path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" />
            <circle cx="12" cy="12" r="3" />
          </svg>
          <span>Settings</span>
        </button>

        {/* 虚拟人/桌宠 显隐开关按钮 (无边框无背景色，纯净深浅切换) */}
        <button
          type="button"
          onClick={() => {
            const next = !isAvatarVisible;
            setIsAvatarVisible(next);
            void setAvatarVisible(next);
          }}
          className="p-1.5 px-2 rounded-lg flex items-center justify-center transition-colors cursor-pointer hover:bg-[var(--surface-3)]/60"
          title={t("chat:toggleAvatar", { defaultValue: "隐藏 / 显示虚拟人" })}
        >
          {/* 精美小人/桌宠 SVG 图标，通过透明度与文字深浅表达状态 */}
          <svg
            width="15"
            height="15"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            strokeLinecap="round"
            strokeLinejoin="round"
            className={`transition-opacity transition-colors ${
              isAvatarVisible
                ? "text-[var(--ink)] opacity-80 hover:opacity-100"
                : "text-[var(--ink-3)] opacity-35 hover:opacity-60"
            }`}
          >
            <path d="M12 2a4 4 0 1 0 0 8 4 4 0 0 0 0-8z" />
            <path d="M6 21v-2a4 4 0 0 1 4-4h4a4 4 0 0 1 4 4v2" />
            {/* 灵动猫耳/宠物装饰角 */}
            <path d="M9 3L7.5 1.5" />
            <path d="M15 3L16.5 1.5" />
          </svg>
        </button>
      </div>

      {/* 项目设置与执行目录模态窗口 */}
      {settingsModalProjectId && (
        <div className="fixed inset-0 z-50 flex items-center justify-center p-6 bg-black/60 backdrop-blur-sm animate-in fade-in duration-200">
          <div className="w-full max-w-3xl max-h-[90vh] overflow-y-auto bg-[var(--surface)] border border-[var(--line)] rounded-3xl shadow-2xl p-6 relative">
            <ProjectSettings
              projectId={settingsModalProjectId}
              onClose={() => setSettingsModalProjectId(null)}
            />
          </div>
        </div>
      )}
    </div>
  );
}
