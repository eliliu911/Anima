/**
 * 系列通用资产：把分集里的资产收进系列池（Rust `series_assets.rs`）。
 *
 * 系列里没有同类同名的 → 原样搬过去，**id 不变**（镜头按 id 引用，一镜都不用改）；
 * 已经有 → 并进那一个，镜头引用改指过去。每次调用先备份到 `backups/series-<时间>/`。
 */
import { invoke } from "@tauri-apps/api/core";
import type { AssetView } from "./assets";
import type { ExtractedEntity } from "./cinema";

export interface InboxItem {
  episode_id: string;
  episode_num: number;
  episode_title: string;
  asset: AssetView;
  /** 系列里同类同名的那一个 —— 收进去时会并进它 */
  match_id: string | null;
}

export interface SeriesInbox {
  /** 待归属：在分集里、没标「只留本集」 */
  pending: InboxItem[];
  /** 标了「只留本集」的 */
  kept: InboxItem[];
}

export type AdoptOutcome =
  | { kind: "moved"; id: string; name: string }
  | { kind: "absorbed"; from_id: string; into_id: string; name: string; shots: number };

export interface AdoptSummary {
  moved: number;
  absorbed: number;
  kept: number;
}

export const seriesAssetsInbox = (projectId: string) =>
  invoke<SeriesInbox>("series_assets_inbox", { projectId });

export const seriesAssetsAdopt = (projectId: string, episodeId: string, id: string) =>
  invoke<AdoptOutcome>("series_assets_adopt", { projectId, episodeId, id });

export const seriesAssetsAdoptAll = (projectId: string) =>
  invoke<AdoptSummary>("series_assets_adopt_all", { projectId });

export const seriesAssetsKeepEpisode = (
  projectId: string,
  episodeId: string,
  id: string,
  on: boolean,
) => invoke<void>("series_assets_keep_episode", { projectId, episodeId, id, on });

/**
 * 从设定文档（人物小传 / 场景设计 / 道具设计）里认出角色、场景、道具。**不落盘**。
 * 一份文档调一次模型，**只收标题对应的那一类**（人物小传只出角色…），结果按「类型/名字」去重；系列里已有的标 `exists`。
 */
export const seriesExtractFromTexts = (projectId: string, docs: { title: string; text: string }[]) =>
  invoke<ExtractedEntity[]>("series_extract_from_texts", { projectId, docs });
