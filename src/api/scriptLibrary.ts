import { invoke } from "@tauri-apps/api/core";

/** 一份文档是原文还是剧本。 */
/**
 * 三种文档，**下游只认 `screenplay`**。
 *
 * `plan`（改编方案）是原文和剧本之间缺的那一层：哪几章并成哪一集、
 * 每集兑现什么、退出状态是什么。没有它的话模型每次自己现分集 ——
 * 同一本小说跑两次得到两套完全不同的分法，而且都不报错。
 */
export type ScriptDocKind = "source" | "plan" | "screenplay";

/**
 * 这份剧本从哪份原文生成的。存下来是为了**可追溯、可重生成** ——
 * 改了原文之后能找回来重跑一次，而不是从头再来。
 */
export interface DerivedFrom {
  source_doc_id: string;
  range: string;
  recipe: string;
  /** 生成时间（毫秒）。原文后来改过的话，这个时间就是「过期」的判据。 */
  at: number;
}

/**
 * 一个剧本项目。
 *
 * **和视频项目是两回事，而且它不知道视频项目的存在。**
 * 可以只写剧本、完全不建视频项目；等剧本成了再送去建。
 */
export interface ScriptProject {
  id: string;
  title: string;
  note: string;
  created_at: number;
  updated_at: number;
}

export interface ScriptDocument {
  id: string;
  title: string;
  text: string;
  doc: string | null;
  revision: number;
  /** 属于哪个剧本项目。`null` = 没归档的散落文档（所有存量数据都是这个状态）。 */
  project_id?: string | null;
  /** 原文还是剧本。**下游只认 screenplay。** */
  kind?: ScriptDocKind;
  derived_from?: DerivedFrom | null;
  /**
   * 上次保存的时刻（毫秒）。**存量数据是 0**，下次保存时自己就有了。
   *
   * 有它才排得了「最近改动」。`revision` 顶不了这个用：它是**改过几次**，
   * 不是**什么时候改的** —— 一份改过 30 次的旧稿会排在昨天刚碰过一次的
   * 新稿前面。
   */
  updated_at?: number;
}

export const listScriptDocuments = () => invoke<ScriptDocument[]>("studio_script_list");
export const saveScriptDocument = (document: ScriptDocument) =>
  invoke<ScriptDocument>("studio_script_save", { document });

export const listScriptProjects = () => invoke<ScriptProject[]>("studio_script_projects");
export const saveScriptProject = (project: ScriptProject) =>
  invoke<ScriptProject>("studio_script_project_save", { project });
/** 删项目**不删里面的文档** —— 它们回到散落状态。返回解除归属的文档数。 */
export const deleteScriptProject = (projectId: string) =>
  invoke<number>("studio_script_project_delete", { projectId });

/**
 * 删掉一份文档。返回**因此失去源头**的那几份的标题。
 *
 * 派生出去的稿子不跟着删 —— 它们可能已经被手改过很多，用户删的是素材，
 * 不是成果。但它们的「回到源头」会跳不动，所以受影响的标题要报出来说给人听。
 */
export const deleteScriptDocument = (id: string) =>
  invoke<string[]>("studio_script_delete", { id });

/**
 * 生成下一步产物。**产出什么由 `sourceDocId` 那份文档是什么决定**：
 * 原文 → 改编方案，改编方案 → 剧本。
 *
 * `also` 是「还要一起规划的章节」。**章节和集数不是一对一** ——
 * 几章讲同一件事就该并成一集，而十个字的一章撑不起任何一集。
 *
 * **重新生成会盖掉上一份**（同项目、同类型、同一组来源的那一份），
 * 但**手改过的那份不会被盖** —— 那种情况下另存一份新的。
 */
export const generateScreenplay = (sourceDocId: string, note: string, also?: string[]) =>
  invoke<ScriptDocument>("studio_script_generate", { sourceDocId, note, also });
