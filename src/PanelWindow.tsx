import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { AvatarSettings } from "./components/AvatarSettings";
import { CharacterSettings } from "./components/CharacterSettings";
import { Diagnostics } from "./components/Diagnostics";
import { LlmSettings } from "./components/LlmSettings";
import { McpSettings } from "./components/McpSettings";
import { MemorySettings } from "./components/MemorySettings";
import { SkillSettings } from "./components/SkillSettings";
import { TtsEngineSettings } from "./components/TtsEngineSettings";
import { ProjectSettings } from "./components/ProjectSettings";
import { ToolConfirmDialog } from "./components/ToolConfirmDialog";
import { Sidebar } from "./components/Sidebar";
import { CommandBar } from "./components/CommandBar";
import { McpMonitor } from "./components/McpMonitor";
import { ChatHistory } from "./components/ChatHistory";
import { rememberPanelGeometry } from "./api/voice";
import { useProject } from "./store/project";

import { StudioShell } from "./components/studio/StudioShell";

/**
 * 面板窗口的外壳。
 *
 * ## 为什么每个面板是独立窗口
 *
 * 桌宠形态要求「关掉所有面板她还在桌面上」。做成一个大窗口里的标签页的话，
 * 那个窗口本身就是应用 —— 关了她就没了。所以每一块独立开窗，
 * 托盘是唯一入口。
 *
 * 面板组件本身没变，只是换了个容器 —— 逻辑是好的，改的是外壳。
 */
export function PanelWindow({ label }: { label: string }) {
  useRememberGeometry(label);

  const isFull =
    label === "chat" || label === "anima_studio" || label === "settings";

  return (
    <div className="bg-[var(--surface)] overflow-hidden flex flex-col text-[13px] leading-[1.6] h-screen w-screen">
      <div className={`flex flex-col flex-1 min-h-0 ${isFull ? "p-0 overflow-hidden" : "pt-7 p-4 gap-3 overflow-y-auto"}`}>
        {render(label)}
      </div>
      {/* 工具确认可能在任何窗口里弹出来 —— 谁触发的就在谁那儿确认 */}
      <ToolConfirmDialog />
    </div>
  );
}


function render(label: string) {
  switch (label) {
    case "persona":
      return <CharacterSettings />;
    case "anima_studio":
      return <StudioShell />;
    case "memory":
      return <MemorySettings />;
    case "settings":
      return <SettingsPanes />;
    case "diag":
      // **不是面容设置。** 面容设置在「设置 → 面容」里，那是"改成什么样"；
      // 这一页是"现在实际是什么样" —— 排查用的，两件事
      return <Diagnostics />;
    case "chat":
      return <ChatPane />;
    default:
      return <div>没有叫「{label}」的面板。</div>;
  }
}

import { ErrorBoundary } from "./components/ErrorBoundary";

import { useTranslation } from "react-i18next";
import { GeneralSettings } from "./components/GeneralSettings";
import { StudioSettings } from "./components/StudioSettings";
import { ServiceSettings } from "./components/ServiceSettings";

function SettingsPanes() {
  const { t } = useTranslation(["settings", "common"]);
  const { projects, loadProjects, createProject } = useProject();
  const [activeTab, setActiveTab] = useState<string>("general");
  const [isCreatingProject, setIsCreatingProject] = useState(false);
  const [newProjectName, setNewProjectName] = useState("");

  useEffect(() => {
    void loadProjects();
  }, [loadProjects]);

  const handleCreateProject = async () => {
    if (newProjectName.trim()) {
      const pName = newProjectName.trim();
      setNewProjectName("");
      setIsCreatingProject(false);
      await createProject(pName);
      const latest = useProject.getState().projects;
      const created = latest.find((p) => p.name === pName);
      if (created) {
        setActiveTab(`project:${created.id}`);
      }
    }
  };

  const renderContent = () => {
    if (activeTab.startsWith("project:")) {
      const pId = activeTab.replace("project:", "");
      return <ProjectSettings projectId={pId} />;
    }
    switch (activeTab) {
      case "general":
        return <GeneralSettings />;
      case "application":
        return <TtsEngineSettings />;
      case "appearance":
        return <AvatarSettings />;
      case "models":
        return <LlmSettings />;
      case "customizations":
        return <SkillSettings />;
      case "browser":
        return <McpSettings />;
      case "services":
        return <ServiceSettings />;
      case "studio":
        return <StudioSettings />;
      default:
        return <GeneralSettings />;
    }
  };

  return (
    <div className="flex flex-1 min-h-0 h-full">
      {/* 左侧导航栏（对标 Antigravity 纯净极简系统设计） */}
      <nav data-tauri-drag-region className="w-[200px] border-r border-[var(--line)] pt-7 pb-4 px-2.5 flex flex-col gap-4 bg-[var(--surface-2)] shrink-0 overflow-y-auto select-none">
        {/* 分组 1: Settings 核心导航 */}
        <div className="flex flex-col gap-0.5">
          <div className="px-2.5 py-1 text-[11px] font-bold text-[var(--ink-3)] uppercase tracking-wider">
            {t("nav.settings", { defaultValue: "Settings" })}
          </div>

          {/* General */}
          <button
            type="button"
            className={`text-[12.5px] px-2.5 py-1.5 rounded-lg text-left flex items-center gap-2.5 transition-colors cursor-pointer ${
              activeTab === "general"
                ? "bg-[var(--surface-3)] text-[var(--ink)] font-semibold shadow-2xs"
                : "text-[var(--ink-2)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)]/60"
            }`}
            onClick={() => setActiveTab("general")}
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="shrink-0 opacity-75">
              <path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" />
              <circle cx="12" cy="12" r="3" />
            </svg>
            <span>{t("nav.general", { defaultValue: "General" })}</span>
          </button>

          {/* Application */}
          <button
            type="button"
            className={`text-[12.5px] px-2.5 py-1.5 rounded-lg text-left flex items-center gap-2.5 transition-colors cursor-pointer ${
              activeTab === "application"
                ? "bg-[var(--surface-3)] text-[var(--ink)] font-semibold shadow-2xs"
                : "text-[var(--ink-2)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)]/60"
            }`}
            onClick={() => setActiveTab("application")}
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="shrink-0 opacity-75">
              <rect x="2" y="3" width="20" height="14" rx="2" ry="2" />
              <line x1="8" y1="21" x2="16" y2="21" />
              <line x1="12" y1="17" x2="12" y2="21" />
            </svg>
            <span>{t("nav.application", { defaultValue: "Application" })}</span>
          </button>

          {/* Studio —— 漫剧的 AI 环节。
              用户 2026-09-19：「我需要在设置里单独分出一个 Studio 区域，
              这个区域里单独有一个 Prompt 设置」。提示词配置本来就存在，
              只是入口在项目页上、名字叫「提示词」，他一直忽略了。 */}
          <button
            type="button"
            className={`text-[12.5px] px-2.5 py-1.5 rounded-lg text-left flex items-center gap-2.5 transition-colors cursor-pointer ${
              activeTab === "studio"
                ? "bg-[var(--surface-3)] text-[var(--ink)] font-semibold shadow-2xs"
                : "text-[var(--ink-2)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)]/60"
            }`}
            onClick={() => setActiveTab("studio")}
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="shrink-0 opacity-75">
              <path d="m22 8-6 4 6 4V8Z" />
              <rect x="2" y="6" width="14" height="12" rx="2" ry="2" />
            </svg>
            <span>Studio</span>
          </button>

          {/* Appearance */}
          <button
            type="button"
            className={`text-[12.5px] px-2.5 py-1.5 rounded-lg text-left flex items-center gap-2.5 transition-colors cursor-pointer ${
              activeTab === "appearance"
                ? "bg-[var(--surface-3)] text-[var(--ink)] font-semibold shadow-2xs"
                : "text-[var(--ink-2)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)]/60"
            }`}
            onClick={() => setActiveTab("appearance")}
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="shrink-0 opacity-75">
              <circle cx="12" cy="12" r="5" />
              <path d="M12 1v2M12 21v2M4.22 4.22l1.42 1.42M18.36 18.36l1.42 1.42M1 12h2M21 12h2M4.22 19.78l1.42-1.42M18.36 5.64l1.42-1.42" />
            </svg>
            <span>{t("nav.appearance", { defaultValue: "Appearance" })}</span>
          </button>

          {/* Models */}
          <button
            type="button"
            className={`text-[12.5px] px-2.5 py-1.5 rounded-lg text-left flex items-center gap-2.5 transition-colors cursor-pointer ${
              activeTab === "models"
                ? "bg-[var(--surface-3)] text-[var(--ink)] font-semibold shadow-2xs"
                : "text-[var(--ink-2)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)]/60"
            }`}
            onClick={() => setActiveTab("models")}
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="shrink-0 opacity-75">
              <path d="M12 2a8 8 0 0 0-8 8c0 3.37 2.08 6.25 5 7.42V20a2 2 0 0 0 2 2h2a2 2 0 0 0 2-2v-2.58c2.92-1.17 5-4.05 5-7.42a8 8 0 0 0-8-8z" />
              <line x1="9" y1="9" x2="9.01" y2="9" />
              <line x1="15" y1="9" x2="15.01" y2="9" />
            </svg>
            <span>{t("nav.models", { defaultValue: "Models" })}</span>
          </button>

          {/* Customizations */}
          <button
            type="button"
            className={`text-[12.5px] px-2.5 py-1.5 rounded-lg text-left flex items-center gap-2.5 transition-colors cursor-pointer ${
              activeTab === "customizations"
                ? "bg-[var(--surface-3)] text-[var(--ink)] font-semibold shadow-2xs"
                : "text-[var(--ink-2)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)]/60"
            }`}
            onClick={() => setActiveTab("customizations")}
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="shrink-0 opacity-75">
              <polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2" />
            </svg>
            <span>{t("nav.customizations", { defaultValue: "Customizations" })}</span>
          </button>

          {/* Browser / MCP */}
          <button
            type="button"
            className={`text-[12.5px] px-2.5 py-1.5 rounded-lg text-left flex items-center gap-2.5 transition-colors cursor-pointer ${
              activeTab === "browser"
                ? "bg-[var(--surface-3)] text-[var(--ink)] font-semibold shadow-2xs"
                : "text-[var(--ink-2)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)]/60"
            }`}
            onClick={() => setActiveTab("browser")}
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="shrink-0 opacity-75">
              <circle cx="12" cy="12" r="10" />
              <line x1="2" y1="12" x2="22" y2="12" />
              <path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z" />
            </svg>
            <span>{t("nav.browser", { defaultValue: "Browser" })}</span>
          </button>
          {/* 服务：这台机器上 Anima 依赖的东西现在谁在跑。
              原来这份信息只有 `anima status` 打得出来 —— 只有开着终端的人看得到，
              而它恰恰是排查的第一步（出图失败多半是 ComfyUI 没开） */}
          <button
            type="button"
            className={`text-[12.5px] px-2.5 py-1.5 rounded-lg text-left flex items-center gap-2.5 transition-colors cursor-pointer ${
              activeTab === "services"
                ? "bg-[var(--surface-3)] text-[var(--ink)] font-semibold shadow-2xs"
                : "text-[var(--ink-2)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)]/60"
            }`}
            onClick={() => setActiveTab("services")}
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="shrink-0 opacity-75">
              <rect x="2" y="2" width="20" height="8" rx="2" />
              <rect x="2" y="14" width="20" height="8" rx="2" />
              <line x1="6" y1="6" x2="6.01" y2="6" />
              <line x1="6" y1="18" x2="6.01" y2="18" />
            </svg>
            <span>{t("nav.services", { defaultValue: "服务" })}</span>
          </button>
        </div>

        {/* 分组 2: Projects 项目列表 */}
        <div className="flex flex-col gap-0.5 pt-2 border-t border-[var(--line)]">
          <div className="flex items-center justify-between px-2.5 py-1">
            <span className="text-[11px] font-bold text-[var(--ink-3)] uppercase tracking-wider">
              {t("nav.projects", { defaultValue: "Projects" })}
            </span>
            <button
              type="button"
              title={t("nav.newProject", { defaultValue: "新建项目" })}
              onClick={() => setIsCreatingProject(true)}
              className="text-[var(--ink-3)] hover:text-[var(--ink)] p-0.5 rounded cursor-pointer"
            >
              <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5">
                <path d="M12 5v14M5 12h14" />
              </svg>
            </button>
          </div>

          {isCreatingProject && (
            <input
              autoFocus
              value={newProjectName}
              placeholder="项目名，回车确认"
              onChange={(e) => setNewProjectName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void handleCreateProject();
                if (e.key === "Escape") {
                  setIsCreatingProject(false);
                  setNewProjectName("");
                }
              }}
              onBlur={() => {
                setIsCreatingProject(false);
                setNewProjectName("");
              }}
              className="w-[calc(100%-8px)] mx-1 my-1 p-1 px-2 text-[12px] bg-[var(--surface)] border border-[var(--celadon)] rounded-lg outline-none text-[var(--ink)] shadow-xs"
            />
          )}

          {projects.length === 0 && !isCreatingProject && (
            <div className="px-2.5 py-1 text-[11px] text-[var(--ink-3)]">
              {t("chat:noProjects", { defaultValue: "暂无项目" })}
            </div>
          )}

          {projects.map((p) => {
            const isSelected = activeTab === `project:${p.id}`;
            return (
              <button
                key={p.id}
                type="button"
                className={`text-[12.5px] px-2.5 py-1.5 rounded-lg text-left flex items-center gap-2 transition-colors cursor-pointer truncate ${
                  isSelected
                    ? "bg-[var(--surface-3)] text-[var(--ink)] font-semibold shadow-2xs"
                    : "text-[var(--ink-2)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)]/60"
                }`}
                onClick={() => setActiveTab(`project:${p.id}`)}
              >
                <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" className="shrink-0 opacity-70">
                  <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
                </svg>
                <span className="truncate">{p.name}</span>
              </button>
            );
          })}
        </div>
      </nav>

      {/* 右侧主内容区 */}
      <div className="flex-1 pt-7 p-5 flex flex-col gap-3 min-w-0 overflow-y-auto bg-[var(--surface)]">
        <ErrorBoundary fallbackTitle="设置面板加载异常">
          {renderContent()}
        </ErrorBoundary>
      </div>
    </div>
  );
}

import { CodeViewerPanel } from "./components/CodeViewerPanel";
import { useCodeViewer } from "./store/codeViewer";

/**
 * 对话窗：左侧会话列表 + 中间主对话区 + 右侧代码查看分栏。
 * 顶部统一单行紧凑 Header（左侧会话/项目信息 + 右侧代码面板 Tab 与折叠按钮同一行）。
 */
import { ScheduledTasksPanel } from "./components/ScheduledTasksPanel";

function ChatPane() {
  const { t } = useTranslation(["chat", "common"]);
  const isSidebarOpen = useProject((s) => s.isSidebarOpen);
  const toggleSidebar = useProject((s) => s.toggleSidebar);
  const mainView = useProject((s) => s.mainView);
  const currentProject = useProject((s) =>
    s.projects.find((p) => p.id === s.activeProjectId)
  );
  const currentSession = useProject((s) =>
    s.sessions.find((sess) => sess.id === s.activeSessionId)
  );

  return (
    <div className="flex h-full w-full overflow-hidden bg-[var(--surface)]">
      <Sidebar open={isSidebarOpen} onToggle={toggleSidebar} />

      {/* 主体与代码面板统一容器 */}
      <div className="flex-1 flex flex-col min-w-0 min-h-0 h-full overflow-hidden">
        {mainView === "tasks" ? (
          <ScheduledTasksPanel />
        ) : (
          /* 内容分栏：左侧消息与输入 + 右侧延伸展开的代码面板 */
          <div className="flex-1 flex min-h-0 min-w-0 overflow-hidden">
            <div className="flex-1 flex flex-col min-w-0 min-h-0 bg-[var(--surface)] h-full overflow-hidden">
              {/* 左侧对话区顶部导航条 (如果右侧面板未展开，右侧显示展开按钮；已展开则无需额外空白) */}
              <div data-tauri-drag-region className="pt-3 pb-2 px-4 flex items-center justify-between shrink-0 select-none bg-[var(--surface)]">
                <div className="flex items-center gap-2 text-[12px] font-medium text-[var(--ink)] min-w-0">
                  {!isSidebarOpen && toggleSidebar && (
                    <button
                      type="button"
                      onClick={toggleSidebar}
                      className="p-1 -ml-1 text-[var(--ink-3)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)]/60 rounded-lg transition-colors cursor-pointer mr-1"
                      title={t("chat:expandSidebar", { defaultValue: "展开侧边栏" })}
                      aria-label={t("chat:expandSidebar", { defaultValue: "展开侧边栏" })}
                    >
                      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                        <rect x="3" y="3" width="18" height="18" rx="2" ry="2" />
                        <line x1="9" y1="3" x2="9" y2="21" />
                      </svg>
                    </button>
                  )}
                  <span className="w-2 h-2 rounded-full bg-[var(--celadon)] shrink-0" />
                  {currentProject ? (
                    <div className="flex items-center gap-1.5 min-w-0 text-[12px] truncate">
                      <span className="text-[var(--ink-2)] truncate font-normal">
                        {currentProject.name}
                      </span>
                      <span className="text-[var(--ink-3)] select-none text-[11px]">/</span>
                      <span className="font-semibold text-[var(--ink)] truncate">
                        {currentSession?.title || t("chat:newChat", { defaultValue: "新对话" })}
                      </span>
                    </div>
                  ) : (
                    <span className="font-semibold text-[var(--ink)] truncate">
                      {currentSession?.title || t("chat:newChat", { defaultValue: "新对话" })}
                    </span>
                  )}
                </div>

                {/* 当代码面板未开启时，在右上角展示开启按钮 */}
                {!useCodeViewer((s) => s.isOpen) && (
                  <div className="flex items-center gap-1 shrink-0">
                    <button
                      type="button"
                      onClick={() => {
                        const codeViewer = useCodeViewer.getState();
                        if (codeViewer.filePath || codeViewer.diffData) {
                          useCodeViewer.setState({ isOpen: true });
                        } else {
                          void codeViewer.openFile("README.md");
                        }
                      }}
                      className="p-1.5 rounded-lg text-[var(--ink-3)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)]/60 transition-colors cursor-pointer"
                      title={t("chat:expandRightPanel", { defaultValue: "展开右侧面板" })}
                      aria-label={t("chat:expandRightPanel", { defaultValue: "展开右侧面板" })}
                    >
                      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                        <rect x="3" y="3" width="18" height="18" rx="2" ry="2" />
                        <line x1="15" y1="3" x2="15" y2="21" />
                      </svg>
                    </button>
                  </div>
                )}
              </div>

              <ChatHistory />
              <CommandBar />
            </div>
            <CodeViewerPanel />
          </div>
        )}
      </div>
      <McpMonitor />
    </div>
  );
}

/**
 * 关窗前把位置和尺寸记下来。
 *
 * **必须在关闭事件里同步记，不能等销毁之后** —— 窗口销毁后
 * 系统给出的几何信息是 0 或者屏幕外的坐标，存下去下次会开到看不见的地方。
 */
function useRememberGeometry(label: string) {
  useEffect(() => {
    const w = getCurrentWindow();
    const remember = () => {
      void rememberPanelGeometry(label).catch(() => {});
    };

    // 移动/缩放之后也记一次，防止应用退出时丢掉。**要节流**
    let timer: number | undefined;
    const throttled = () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(remember, 600);
    };
    const unMoved = w.onMoved(throttled);
    const unResized = w.onResized(throttled);

    const onUnload = () => {
      remember();
    };
    window.addEventListener("beforeunload", onUnload);

    return () => {
      window.removeEventListener("beforeunload", onUnload);
      window.clearTimeout(timer);
      void unMoved.then((f) => f());
      void unResized.then((f) => f());
    };
  }, [label]);
}
