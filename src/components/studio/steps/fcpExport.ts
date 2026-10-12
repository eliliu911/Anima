/**
 * 导出到 Final Cut Pro：默认名字 + 记住上次填的。
 *
 * **名字照用户自己的习惯起，而且不用他每集再改**（用户 2026-10-05：「需要你做命名规则预设」「不要我后面每次都修改」）：
 * - 项目名 `S01E02_本机无武器_初剪`；
 * - 事件名照 FCP 资源库里已有的对齐（`01_PV01_先行预告片`、`02_S01E01_第三次` → 下一集 `03_S01E02_本机无武器`），
 *   资源库里已经有这一集的事件就用它的原名。这一步要读磁盘，在 Rust（`cinema_fcp_names`），见 [`alignFcpNames`]。
 *
 * 导出位置按**项目**记（一个剧的素材放同一块盘），事件名 / 项目名按**集**记。
 * 存在 localStorage：只是这台机器上的便利，丢了也就是重填一次。
 */

export interface FcpPrefs {
  dest: string;
  event: string;
  project: string;
}

interface Stored {
  dest?: string;
  episodes?: Record<string, { event?: string; project?: string }>;
}

type StorageLike = Pick<Storage, "getItem" | "setItem">;

const key = (projectId: string) => `anima.fcpExport.${projectId}`;

/** `E01-《第三次》` → `S01E01_第三次`。集名里已经带的 `E01-` 和书名号去掉，不然会变成 `S01E01_E01-《第三次》` */
export function defaultFcpName(episodeNum: number, title: string): string {
  const base = title
    .replace(/^\s*E\d+\s*[-_·:：\s]\s*/i, "")
    .replace(/[《》]/g, "")
    .trim();
  return `S01E${String(episodeNum).padStart(2, "0")}_${base || `第 ${episodeNum} 集`}`;
}

function storage(): StorageLike | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    return null;
  }
}

function read(projectId: string, store: StorageLike | null): Stored {
  try {
    return JSON.parse(store?.getItem(key(projectId)) || "{}") as Stored;
  } catch {
    return {};
  }
}

export function loadFcpPrefs(
  projectId: string,
  episodeId: string,
  episodeNum: number,
  title: string,
  store: StorageLike | null = storage(),
): FcpPrefs {
  const s = read(projectId, store);
  const ep = s.episodes?.[episodeId] ?? {};
  const name = defaultFcpName(episodeNum, title);
  return { dest: s.dest ?? "", event: ep.event || name, project: ep.project || defaultFcpProject(episodeNum, title) };
}

/** 项目名的预设：`S01E02_本机无武器_初剪` */
export function defaultFcpProject(episodeNum: number, title: string): string {
  return `${defaultFcpName(episodeNum, title)}_初剪`;
}

/**
 * 按资源库对齐出来的名字（`cinema_fcp_names`）换进去 —— **只换没手改过的**：
 * 空的、或者还是老的默认名（`S01E02_本机无武器`，事件名没序号、项目名没「_初剪」）的才换；
 * 用户自己填过别的就留着。
 */
export function alignFcpNames(prefs: FcpPrefs, suggested: { event: string; project: string }, episodeNum: number, title: string): FcpPrefs {
  const old = defaultFcpName(episodeNum, title);
  const untouched = (v: string) => !v.trim() || v.trim() === old;
  return {
    ...prefs,
    event: untouched(prefs.event) ? suggested.event : prefs.event,
    project: untouched(prefs.project) || prefs.project.trim() === defaultFcpProject(episodeNum, title) ? suggested.project : prefs.project,
  };
}

export function saveFcpPrefs(projectId: string, episodeId: string, prefs: FcpPrefs, store: StorageLike | null = storage()) {
  const s = read(projectId, store);
  const next: Stored = {
    dest: prefs.dest,
    episodes: { ...s.episodes, [episodeId]: { event: prefs.event, project: prefs.project } },
  };
  try {
    store?.setItem(key(projectId), JSON.stringify(next));
  } catch {
    // 存不下（隐私模式、配额满）就算了，下次重填
  }
}
