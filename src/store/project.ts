import { create } from "zustand";
import * as api from "../api";
import { logUi } from "../api/voice";
import type { Project, SessionInfo } from "../api";

interface ProjectState {
  projects: Project[];
  sessions: SessionInfo[];
  
  activeProjectId: string | null;
  activeSessionId: string | null;

  // 主视图模式："chat" (对话) | "tasks" (任务安排)
  mainView: "chat" | "tasks";
  setMainView: (view: "chat" | "tasks") => void;

  // UI state for sidebar
  isSidebarOpen: boolean;

  // Actions
  toggleSidebar: () => void;
  
  loadProjects: () => Promise<void>;
  createProject: (name: string, description?: string, systemPrompt?: string, characterId?: string, workspacePath?: string) => Promise<void>;
  updateProject: (id: string, name: string, description?: string, systemPrompt?: string, characterId?: string, workspacePath?: string) => Promise<void>;
  deleteProject: (id: string) => Promise<void>;
  setActiveProject: (id: string | null) => void;

  createSession: (projectId: string | null) => Promise<void>;
  loadSessions: (projectId: string | null, includeArchived?: boolean) => Promise<void>;
  setActiveSession: (id: string | null) => void;
  updateSessionTitle: (id: string, title: string) => Promise<void>;
  archiveSession: (id: string) => Promise<void>;
  deleteSession: (id: string) => Promise<void>;
}

/// 写操作出错时**先记一笔再抛**。
///
/// 这些调用原先是裸 await：失败变成一个没人接的 Promise rejection，
/// 既不进日志也不上界面 —— 用户看到的就是"点了没反应"。抛还是要抛的，
/// 上层（比如语音启动前先建会话）得知道这一步没成。
async function tracked<T>(what: string, run: () => Promise<T>): Promise<T> {
  try {
    return await run();
  } catch (e) {
    logUi("project", `${what}失败：${e}`);
    throw e;
  }
}

export const useProject = create<ProjectState>((set, get) => ({
  projects: [],
  sessions: [],
  activeProjectId: null,
  activeSessionId: null,
  mainView: "chat",
  setMainView: (mainView) => set({ mainView }),
  isSidebarOpen: true,

  toggleSidebar: () => set((state) => ({ isSidebarOpen: !state.isSidebarOpen })),

  loadProjects: async () => {
    try {
      const projects = await api.getProjects();
      set({ projects });
    } catch (e) {
      // **不能只 console.error。** Tauri 的 webview 没有可读控制台，
      // 前端抛的错在这里等于消失，用户看到的是"点了没反应"
      logUi("project", `加载项目失败：${e}`);
    }
  },

  createProject: async (name, description, systemPrompt, characterId, workspacePath) => {
    // **id 必须用后端返回的那个。** 后端自己生成 uuid，前端这边编一个
    // `proj-<时间戳>` 的话，列表里显示的是真项目、而选中的是个不存在的 id ——
    // 于是会话列表永远空，新建的会话还会挂到一个没有对应行的 project_id 上
    const id = await tracked("新建项目", () =>
      api.createProject({
        name,
        description,
        system_prompt: systemPrompt,
        character_id: characterId,
        workspace_path: workspacePath,
      })
    );
    await get().loadProjects();
    set({ activeProjectId: id });
    await get().loadSessions(id);
  },

  updateProject: async (id, name, description, systemPrompt, characterId, workspacePath) => {
    await tracked("修改项目", () =>
      api.updateProject({
        id,
        name,
        description,
        system_prompt: systemPrompt,
        character_id: characterId,
        workspace_path: workspacePath,
      })
    );
    await get().loadProjects();
  },

  deleteProject: async (id) => {
    await tracked("删除项目", () => api.deleteProject(id));
    await get().loadProjects();
    if (get().activeProjectId === id) {
      set({ activeProjectId: null, activeSessionId: null });
      await get().loadSessions(null);
    }
  },

  setActiveProject: async (id) => {
    set({ activeProjectId: id, activeSessionId: null });
    await get().loadSessions(id);
  },

  createSession: async (projectId) => {
    const id = await tracked("新建对话", () => api.createSession(projectId));
    await get().loadSessions(projectId);
    set({ activeSessionId: id });
  },

  loadSessions: async (_projectId, includeArchived = false) => {
    try {
      const sessions = await api.getAllSessions(includeArchived);
      set({ sessions });
    } catch (e) {
      logUi("project", `加载会话失败：${e}`);
    }
  },

  setActiveSession: (id) => {
    set({ activeSessionId: id });
  },

  updateSessionTitle: async (id, title) => {
    await tracked("重命名对话", () => api.updateSessionTitle(id, title));
    await get().loadSessions(get().activeProjectId);
  },

  archiveSession: async (id) => {
    await tracked("归档对话", () => api.archiveSession(id));
    await get().loadSessions(get().activeProjectId);
    if (get().activeSessionId === id) {
      set({ activeSessionId: null });
    }
  },

  deleteSession: async (id) => {
    await tracked("删除对话", () => api.deleteSession(id));
    await get().loadSessions(get().activeProjectId);
    if (get().activeSessionId === id) {
      set({ activeSessionId: null });
    }
  }
}));
