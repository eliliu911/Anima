import { create } from "zustand";
import * as api from "../api";
import { useProject } from "./project";
import type { DiffData } from "../components/DiffViewer";

export interface CodeTab {
  id: string;
  title: string;
  filePath: string;
  content: string;
  editedContent?: string;
  isDirty?: boolean;
  isEditing?: boolean;
  isBinary?: boolean;
  mediaType?: "image" | "audio" | "video" | "binary" | null;
  dataUrl?: string | null;
  diffData: DiffData | null;
  mode: "code" | "diff";
  lineRange: [number, number] | null;
  badge?: string;
  loading?: boolean;
  error?: string | null;
}

interface CodeViewerState {
  isOpen: boolean;
  mainView: "overview" | "files" | "code";
  tabs: CodeTab[];
  activeTabId: string | null;

  // Active tab shortcuts
  filePath: string | null;
  content: string;
  editedContent: string;
  isDirty: boolean;
  isEditing: boolean;
  isBinary: boolean;
  mediaType: "image" | "audio" | "video" | "binary" | null;
  dataUrl: string | null;
  diffData: DiffData | null;
  mode: "code" | "diff";
  lineRange: [number, number] | null;
  loading: boolean;
  error: string | null;
  badge?: string;

  setMainView: (view: "overview" | "files" | "code") => void;
  openFile: (
    filePath: string,
    opts?: {
      lineRange?: [number, number] | null;
      diffData?: DiffData | null;
      mode?: "code" | "diff";
      badge?: string;
    }
  ) => Promise<void>;
  openDiff: (diffData: DiffData, badge?: string) => void;
  switchTab: (tabId: string) => void;
  closeTab: (tabId: string) => void;
  setMode: (mode: "code" | "diff") => void;
  setIsEditing: (isEditing: boolean) => void;
  updateTabContent: (tabId: string, newContent: string) => void;
  saveCurrentFile: () => Promise<boolean>;
  commitCurrentProject: (message: string, files?: string[]) => Promise<api.GitCommitResult>;
  close: () => void;
}

function syncActiveState(tabs: CodeTab[], activeId: string | null) {
  const activeTab = tabs.find((t) => t.id === activeId);
  if (!activeTab) {
    return {
      filePath: null,
      content: "",
      editedContent: "",
      isDirty: false,
      isEditing: false,
      isBinary: false,
      mediaType: null,
      dataUrl: null,
      diffData: null,
      mode: "code" as const,
      lineRange: null,
      loading: false,
      error: null,
      badge: undefined,
    };
  }
  return {
    filePath: activeTab.filePath,
    content: activeTab.content,
    editedContent: activeTab.editedContent ?? activeTab.content,
    isDirty: !!activeTab.isDirty,
    isEditing: !!activeTab.isEditing,
    isBinary: !!activeTab.isBinary,
    mediaType: activeTab.mediaType ?? null,
    dataUrl: activeTab.dataUrl ?? null,
    diffData: activeTab.diffData,
    mode: activeTab.mode,
    lineRange: activeTab.lineRange,
    loading: !!activeTab.loading,
    error: activeTab.error || null,
    badge: activeTab.badge,
  };
}

export const useCodeViewer = create<CodeViewerState>((set, get) => ({
  isOpen: false,
  mainView: "code",
  tabs: [],
  activeTabId: null,

  filePath: null,
  content: "",
  editedContent: "",
  isDirty: false,
  isEditing: false,
  isBinary: false,
  mediaType: null,
  dataUrl: null,
  diffData: null,
  mode: "code",
  lineRange: null,
  loading: false,
  error: null,
  badge: undefined,

  setMainView: (view) => set({ mainView: view }),

  openFile: async (filePath: string, opts = {}) => {
    let cleanPath = filePath.replace(/^file:\/\//, "");
    if (cleanPath.includes("asset.localhost/") || cleanPath.includes("localhost/")) {
      const idx = cleanPath.indexOf("localhost/");
      if (idx >= 0) {
        cleanPath = cleanPath.slice(idx + "localhost/".length);
      }
    }
    cleanPath = cleanPath.replace(/^(asset|http|https|file):\/\//, "");
    try {
      cleanPath = decodeURIComponent(cleanPath);
    } catch {}
    if (filePath.startsWith("/") || cleanPath.startsWith("Users/") || cleanPath.startsWith("var/") || cleanPath.startsWith("tmp/")) {
      if (!cleanPath.startsWith("/")) cleanPath = `/${cleanPath}`;
    }

    const rawName = cleanPath.split("/").filter(Boolean).pop() || cleanPath;
    const title = rawName.replace(/_[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}(\.[a-zA-Z0-9]+)$/i, "$1");
    const tabId = cleanPath;

    const existingIndex = get().tabs.findIndex((t) => t.id === tabId || t.filePath === cleanPath);
    let newTabs = [...get().tabs];

    if (existingIndex >= 0) {
      newTabs[existingIndex] = {
        ...newTabs[existingIndex],
        lineRange: opts.lineRange ?? newTabs[existingIndex].lineRange,
        diffData: opts.diffData ?? newTabs[existingIndex].diffData,
        mode: opts.mode ?? (opts.diffData ? "diff" : newTabs[existingIndex].mode),
        badge: opts.badge ?? newTabs[existingIndex].badge,
      };
    } else {
      const newTab: CodeTab = {
        id: tabId,
        title,
        filePath: cleanPath,
        content: "",
        diffData: opts.diffData ?? null,
        mode: opts.mode ?? (opts.diffData ? "diff" : "code"),
        lineRange: opts.lineRange ?? null,
        badge: opts.badge,
        loading: true,
        error: null,
      };
      newTabs.push(newTab);
    }

    set({
      isOpen: true,
      mainView: "code",
      tabs: newTabs,
      activeTabId: tabId,
      ...syncActiveState(newTabs, tabId),
    });

    try {
      const projectState = useProject.getState();
      const activeProject = projectState.projects.find(
        (p) => p.id === projectState.activeProjectId
      );
      const wsRoot = activeProject?.workspace_path || null;

      const res = await api.workspaceReadFile(cleanPath, wsRoot);

      const updatedTabs = get().tabs.map((t) =>
        t.id === tabId
          ? {
              ...t,
              filePath: res.relative_path || cleanPath,
              title: (res.relative_path || cleanPath).split("/").filter(Boolean).pop() || t.title,
              content: res.content,
              isBinary: res.is_binary,
              mediaType: res.media_type,
              dataUrl: res.data_url,
              loading: false,
              error: null,
            }
          : t
      );

      set({
        tabs: updatedTabs,
        ...syncActiveState(updatedTabs, get().activeTabId),
      });
    } catch (err) {
      const updatedTabs = get().tabs.map((t) =>
        t.id === tabId
          ? {
              ...t,
              content: "",
              loading: false,
              error: String(err),
            }
          : t
      );

      set({
        tabs: updatedTabs,
        ...syncActiveState(updatedTabs, get().activeTabId),
      });
    }
  },

  openDiff: (diffData: DiffData, badge?: string) => {
    const tabId = `diff:${diffData.filePath || diffData.fileName}`;
    const title = diffData.fileName;
    const cleanPath = diffData.filePath || diffData.fileName;

    const existingIndex = get().tabs.findIndex((t) => t.id === tabId);
    let newTabs = [...get().tabs];

    const tabData: CodeTab = {
      id: tabId,
      title,
      filePath: cleanPath,
      content: "",
      diffData,
      mode: "diff",
      lineRange: null,
      badge: badge || `+${diffData.additions} -${diffData.deletions}`,
      loading: false,
      error: null,
    };

    if (existingIndex >= 0) {
      newTabs[existingIndex] = tabData;
    } else {
      newTabs.push(tabData);
    }

    set({
      isOpen: true,
      tabs: newTabs,
      activeTabId: tabId,
      ...syncActiveState(newTabs, tabId),
    });
  },

  switchTab: (tabId: string) => {
    set({
      isOpen: true,
      activeTabId: tabId,
      ...syncActiveState(get().tabs, tabId),
    });
  },

  closeTab: (tabId: string) => {
    const currentTabs = get().tabs;
    const tabIndex = currentTabs.findIndex((t) => t.id === tabId);
    if (tabIndex === -1) return;

    const newTabs = currentTabs.filter((t) => t.id !== tabId);

    // 如果关闭的是最后一个标签，保留面板打开，清空当前活跃文件（UI 切换到文件列表/概览）
    if (newTabs.length === 0) {
      set({
        isOpen: true,
        tabs: [],
        activeTabId: null,
        filePath: null,
        content: "",
        diffData: null,
        mode: "code",
        lineRange: null,
        loading: false,
        error: null,
        badge: undefined,
      });
      return;
    }

    // 如果关闭的是当前激活的标签，切换到前一个或第一个标签
    let nextActiveId = get().activeTabId;
    if (nextActiveId === tabId) {
      const nextIndex = Math.max(0, tabIndex - 1);
      nextActiveId = newTabs[nextIndex]?.id || null;
    }

    set({
      tabs: newTabs,
      activeTabId: nextActiveId,
      ...syncActiveState(newTabs, nextActiveId),
    });
  },

  setMode: (mode) => {
    const activeId = get().activeTabId;
    if (!activeId) return;
    const newTabs = get().tabs.map((t) => (t.id === activeId ? { ...t, mode } : t));
    set({
      tabs: newTabs,
      mode,
    });
  },

  setIsEditing: (isEditing: boolean) => {
    const activeId = get().activeTabId;
    if (!activeId) return;
    const newTabs = get().tabs.map((t) => (t.id === activeId ? { ...t, isEditing } : t));
    set({
      tabs: newTabs,
      isEditing,
    });
  },

  updateTabContent: (tabId: string, newContent: string) => {
    const newTabs = get().tabs.map((t) => {
      if (t.id === tabId) {
        const isDirty = newContent !== t.content;
        return {
          ...t,
          editedContent: newContent,
          isDirty,
        };
      }
      return t;
    });

    const activeId = get().activeTabId;
    set({
      tabs: newTabs,
      ...syncActiveState(newTabs, activeId),
    });
  },

  saveCurrentFile: async () => {
    const activeTab = get().tabs.find((t) => t.id === get().activeTabId);
    if (!activeTab || !activeTab.filePath) return false;

    const projectState = useProject.getState();
    const activeProject = projectState.projects.find(
      (p) => p.id === projectState.activeProjectId
    );
    const wsRoot = activeProject?.workspace_path;
    if (!wsRoot) {
      console.error("未找到当前工作区路径");
      return false;
    }

    const contentToSave = activeTab.editedContent ?? activeTab.content;
    try {
      await api.workspaceWriteFile({
        workspacePath: wsRoot,
        relativePath: activeTab.filePath,
        content: contentToSave,
      });

      // 保存成功：更新 content 为最新内容，清除 dirty 状态
      const newTabs = get().tabs.map((t) => {
        if (t.id === activeTab.id) {
          return {
            ...t,
            content: contentToSave,
            editedContent: contentToSave,
            isDirty: false,
          };
        }
        return t;
      });

      set({
        tabs: newTabs,
        ...syncActiveState(newTabs, activeTab.id),
      });
      return true;
    } catch (err) {
      console.error("保存文件失败:", err);
      return false;
    }
  },

  commitCurrentProject: async (message: string, files?: string[]) => {
    const projectState = useProject.getState();
    const activeProject = projectState.projects.find(
      (p) => p.id === projectState.activeProjectId
    );
    const wsRoot = activeProject?.workspace_path;
    if (!wsRoot) {
      throw new Error("未绑定有效工作区");
    }

    // 先保存当前激活的有未保存更改的文件
    await get().saveCurrentFile();

    const res = await api.workspaceCommit({
      workspacePath: wsRoot,
      message,
      files,
    });
    return res;
  },

  close: () => {
    set({
      isOpen: false,
    });
  },
}));
