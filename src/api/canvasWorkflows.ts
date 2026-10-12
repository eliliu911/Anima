import { invoke } from "@tauri-apps/api/core";

/**
 * 工作流目录。**存文件，不存 localStorage** ——
 * localStorage 有 5MB 上限、清缓存就没了，而工作流是要长期攒、
 * 要备份、要能拷给别的机器的东西。理由见 Rust `canvas_workflows`。
 */
export interface WorkflowEntry {
  name: string;
  /** 属于哪个工程。空 = 没挂工程（存的时候画布还没选工程） */
  project_id: string;
  saved_at: number;
  bytes: number;
}

export const canvasWorkflowSave = (projectId: string, name: string, json: string) =>
  invoke<void>("canvas_workflow_save", { projectId, name, json });

/**
 * **所有**工程的工作流，各自带着 `project_id`。
 *
 * 后端不按工程筛 —— 筛了的话界面就没法说「别的工程里还有 N 套」，
 * 而模板存在的全部理由就是换个项目还能跑。筛选是界面的事。
 */
export const canvasWorkflowList = () => invoke<WorkflowEntry[]>("canvas_workflow_list");

export const canvasWorkflowLoad = (projectId: string, name: string) =>
  invoke<string>("canvas_workflow_load", { projectId, name });

export const canvasWorkflowDelete = (projectId: string, name: string) =>
  invoke<void>("canvas_workflow_delete", { projectId, name });

export const canvasWorkflowRename = (projectId: string, from: string, to: string) =>
  invoke<void>("canvas_workflow_rename", { projectId, from, to });
