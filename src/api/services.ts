/**
 * 「服务」页的数据。清单在 Rust 侧（`src-tauri/src/services.rs`），
 * **这里不重复定义任何服务或端口** —— 抄一份就会漂。
 */
import { invoke } from "@tauri-apps/api/core";

export interface ServiceStatus {
  id: string;
  name: string;
  group: "llm" | "vision" | "avatar" | "voice";
  group_label: string;
  port: number;
  online: boolean;
  /** 它是干什么的 */
  what: string;
  /** **离线了会怎么样** —— 这一页最有用的一列 */
  degraded: string;
  /** 起不来怎么办。空串 = 没有一句话能说清的办法 */
  how: string;
  /** 没开是不是正常状态 */
  optional: boolean;
}

export async function serviceStatus(): Promise<ServiceStatus[]> {
  return invoke<ServiceStatus[]>("service_status");
}
