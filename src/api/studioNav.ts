import { emit, listen } from "@tauri-apps/api/event";
import { openPanel } from "./voice";

/**
 * 从别的窗口（Anima Chat）跳到 Studio 的某个项目 / 分集 / 步骤。
 *
 * Studio 是单独的窗口（`anima_studio`）。**窗口刚被打开时事件会丢**（它的页面还没挂上监听），
 * 所以目标同时记在 localStorage（同一个应用的窗口共用），Studio 挂载时取一次、取完就删。
 */
export interface StudioLink {
  project_id: string;
  episode_id?: string;
  /** 项目里的哪一步：script / style / assets / storyboard / assembly */
  page?: string;
}

export const STUDIO_NAVIGATE = "studio://navigate";
const PENDING_KEY = "anima.studio.pendingNav";

/** Studio 工具的返回里带的跳转目标。解析不出来回 null */
export function studioLinkFromToolResult(content: string): StudioLink | null {
  try {
    const v = JSON.parse(content) as { studio_link?: StudioLink };
    const l = v?.studio_link;
    return l && typeof l.project_id === "string" && l.project_id ? l : null;
  } catch {
    return null;
  }
}

export async function openInStudio(link: StudioLink): Promise<void> {
  try {
    localStorage.setItem(PENDING_KEY, JSON.stringify({ ...link, at: Date.now() }));
  } catch {
    // 记不住也没关系：窗口已经开着时事件照样到
  }
  await openPanel("anima_studio");
  await emit(STUDIO_NAVIGATE, link);
}

/** Studio 挂载时取一次（1 分钟内的才算，过期的丢掉） */
export function takePendingStudioLink(now = Date.now()): StudioLink | null {
  try {
    const raw = localStorage.getItem(PENDING_KEY);
    if (!raw) return null;
    localStorage.removeItem(PENDING_KEY);
    const v = JSON.parse(raw) as StudioLink & { at?: number };
    return v.project_id && (!v.at || now - v.at < 60_000) ? v : null;
  } catch {
    return null;
  }
}

export function onStudioNavigate(cb: (l: StudioLink) => void): Promise<() => void> {
  return listen<StudioLink>(STUDIO_NAVIGATE, (e) => {
    // 事件到了就把备份删掉，免得下次打开 Studio 又跳一次
    try {
      localStorage.removeItem(PENDING_KEY);
    } catch {
      // ignore
    }
    cb(e.payload);
  });
}
