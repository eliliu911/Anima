/**
 * 3DGS 片场文件的读取。
 *
 * 片场一份五六十 MB（SHARP 每个像素一个高斯），`assets_read_media` 的 base64 通道上限 24 MB 读不了，
 * `anima-media://` 不带 CORS 头、加载器 fetch 会被拦 —— 所以单开一条按二进制回的命令
 * （Rust `world_sharp::world_read_splat`），这里包成 `blob:` 地址给 splat 加载器。
 * **用完要 `URL.revokeObjectURL`**，否则每换一次片场就漏一份。
 */
import { invoke } from "@tauri-apps/api/core";

export async function readSplatBlobUrl(path: string): Promise<string> {
  const bytes = await invoke<ArrayBuffer>("world_read_splat", { path });
  return URL.createObjectURL(new Blob([bytes], { type: "application/octet-stream" }));
}
