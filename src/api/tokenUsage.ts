import { invoke } from "@tauri-apps/api/core";

/**
 * 模型用量。见 Rust `token_usage`：网关每次调用结束时按「后端 + 模型 + 分集 + 步骤」记账。
 *
 * `usd` 是**估算**：只对官方公布过价格的模型算（gpt-5.6 系列），本地模型不花钱。
 * `priced_all: false` 说明里面有认不出价的云端模型，`usd` 只是认得出的那部分 —— 界面要说出来。
 */
export interface UsageTotals {
  calls: number;
  prompt_tokens: number;
  completion_tokens: number;
  reasoning_tokens: number;
  usd: number;
  priced_all: boolean;
}

export interface ModelUsage {
  provider: string;
  provider_label: string;
  model: string;
  cloud: boolean;
  /** 每百万 token 的价（输入, 输出）；认不出是 null */
  price: [number, number] | null;
  totals: UsageTotals;
}

export interface UsageSummary {
  models: ModelUsage[];
  steps: { step: string; totals: UsageTotals }[];
  /** 键是分集 id；`""` 是项目里没归到某一集的 */
  episodes: Record<string, UsageTotals>;
  project: UsageTotals;
  /** 所有项目 + 对话等不归项目的 */
  all_models: ModelUsage[];
  all: UsageTotals;
}

export const tokenUsageSummary = (projectId: string) => invoke<UsageSummary>("token_usage_summary", { projectId });
