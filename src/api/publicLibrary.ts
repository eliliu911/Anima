import { invoke } from "@tauri-apps/api/core";
import type { AssetView, Scope } from "./assets";

/**
 * 公共素材库（见 Rust `public_library`）：`~/AI/assets/public/` 下的现成素材，只读。
 * 挑一个「加入」→ 复制进全局库或某个项目，成为普通场景资产（每个时段一张变体）。
 */
export interface PublicCollection {
  id: string;
  name: string;
  author: string;
  url: string;
  license: string;
}

export interface PublicVariant {
  /** 「白天」「黎明 · 关灯」；空串 = 文件名里没写时段 */
  label: string;
  path: string;
  url: string;
}

export interface PublicScene {
  id: string;
  collection: string;
  category: string;
  name: string;
  /** 第一个是封面（白天优先） */
  variants: PublicVariant[];
}

export interface PublicLibrary {
  root: string;
  collections: PublicCollection[];
  scenes: PublicScene[];
}

export const publicLibraryList = () => invoke<PublicLibrary>("public_library_list");

export const publicLibraryImport = (sceneId: string, scope: Scope, name: string) =>
  invoke<AssetView>("public_library_import", { sceneId, scope, name });
