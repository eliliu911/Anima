import { invoke } from "@tauri-apps/api/core";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { mediaSrc, pathFromMediaUrl } from "../../components/studio/MediaThumb";

function base64ToBlob(dataUrl: string): Blob {
  const parts = dataUrl.split(",");
  const mime = parts[0]?.match(/:(.*?);/)?.[1] || "image/png";
  const bstr = atob(parts[1] || "");
  let n = bstr.length;
  const u8arr = new Uint8Array(n);
  while (n--) {
    u8arr[n] = bstr.charCodeAt(n);
  }
  return new Blob([u8arr], { type: mime });
}

function triggerAnchorDownload(url: string, filename: string): void {
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
}

export async function downloadMedia(path: string, customFilename?: string): Promise<void> {
  if (!path) return;
  const rawPath = pathFromMediaUrl(path);
  const cleanPath = rawPath.split("?")[0];
  const ext = cleanPath.split(".").pop() || (cleanPath.endsWith(".mp4") ? "mp4" : "png");
  const filename = customFilename || `anima-${Date.now()}.${ext}`;

  // 1. Data URL
  if (rawPath.startsWith("data:")) {
    try {
      const blob = base64ToBlob(rawPath);
      const objUrl = URL.createObjectURL(blob);
      triggerAnchorDownload(objUrl, filename);
      setTimeout(() => URL.revokeObjectURL(objUrl), 15000);
      return;
    } catch {
      triggerAnchorDownload(rawPath, filename);
      return;
    }
  }

  // 2. Blob URL
  if (rawPath.startsWith("blob:")) {
    triggerAnchorDownload(rawPath, filename);
    return;
  }

  // 3. Remote HTTP(S) URL
  if (rawPath.startsWith("http:") || rawPath.startsWith("https:")) {
    try {
      const res = await fetch(rawPath);
      if (res.ok) {
        const blob = await res.blob();
        const objUrl = URL.createObjectURL(blob);
        triggerAnchorDownload(objUrl, filename);
        setTimeout(() => URL.revokeObjectURL(objUrl), 15000);
        return;
      }
    } catch (e) {
      console.warn("Direct http download fetch failed:", e);
    }
    triggerAnchorDownload(rawPath, filename);
    return;
  }

  // 4. 本机文件：直接复制到「下载」文件夹，再在访达里选中它。
  //    原来走 base64 + <a download>，在桌面 WebView 里不一定生效而且不报错；视频几十 MB 读成 base64 也慢。
  try {
    const dest = await invoke<string>("media_save_to_downloads", { path: rawPath });
    try {
      await revealItemInDir(dest);
    } catch {
      // 选不中也没关系，文件已经在「下载」里了
    }
    return;
  } catch {
    // 不是本机媒体文件（或者不在桌面端）：走下面的老路
  }

  // 5. Local filesystem path in Tauri: try assets_read_media
  try {
    const dataUrl = await invoke<string>("assets_read_media", { path: rawPath });
    if (dataUrl && dataUrl.startsWith("data:")) {
      const blob = base64ToBlob(dataUrl);
      const objUrl = URL.createObjectURL(blob);
      triggerAnchorDownload(objUrl, filename);
      setTimeout(() => URL.revokeObjectURL(objUrl), 15000);
      return;
    }
  } catch (ipcErr) {
    // assets_read_media failed or not in Tauri environment
  }

  // 6. Try fetch on mediaSrc (works in browser dev mode or mocked test environments)
  const url = mediaSrc(rawPath);
  try {
    const res = await fetch(url);
    if (res.ok) {
      const blob = await res.blob();
      const objUrl = URL.createObjectURL(blob);
      triggerAnchorDownload(objUrl, filename);
      setTimeout(() => URL.revokeObjectURL(objUrl), 15000);
      return;
    }
  } catch (fetchErr) {
    // Direct fetch failed
  }

  // 7. If downloading file directly in Tauri fails, reveal in Finder / Explorer
  try {
    await revealItemInDir(rawPath);
    return;
  } catch (revealErr) {
    // reveal failed
  }

  // 8. Fallback anchor trigger
  triggerAnchorDownload(url, filename);
}
