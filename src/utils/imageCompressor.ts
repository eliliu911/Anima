/**
 * 客户端图片等比缩放与降采样工具（对标 Codex / Claude Code 多模态图片预处理）
 * 
 * 核心目标：
 * 1. 限制最长边 ≤ 1920px（Retina / 4K 截图等比缩小，保留代码文字可读性，同时减少 70%~90% Base64 体积）；
 * 2. 转换为轻量 JPEG (quality = 0.85)；
 * 3. 避免大图撑爆 IPC 传输带宽、LLM 上下文 Token 预算与本地模型显存。
 */

const MAX_DIMENSION = 1920;
const COMPRESS_QUALITY = 0.85;

export async function downsampleImage(fileOrDataUrl: File | string): Promise<{
  dataUrl: string;
  size: number;
  width: number;
  height: number;
}> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.crossOrigin = "anonymous";

    img.onload = () => {
      let { width, height } = img;

      // 如果最长边超过 MAX_DIMENSION，进行等比缩放
      if (width > MAX_DIMENSION || height > MAX_DIMENSION) {
        if (width >= height) {
          height = Math.round((height * MAX_DIMENSION) / width);
          width = MAX_DIMENSION;
        } else {
          width = Math.round((width * MAX_DIMENSION) / height);
          height = MAX_DIMENSION;
        }
      }

      const canvas = document.createElement("canvas");
      canvas.width = width;
      canvas.height = height;

      const ctx = canvas.getContext("2d");
      if (!ctx) {
        // 降级：Canvas 不可用时返回原图
        if (typeof fileOrDataUrl === "string") {
          resolve({ dataUrl: fileOrDataUrl, size: fileOrDataUrl.length, width, height });
        } else {
          const reader = new FileReader();
          reader.onload = () => {
            const res = reader.result as string;
            resolve({ dataUrl: res, size: fileOrDataUrl.size, width, height });
          };
          reader.onerror = reject;
          reader.readAsDataURL(fileOrDataUrl);
        }
        return;
      }

      ctx.imageSmoothingEnabled = true;
      ctx.imageSmoothingQuality = "high";
      ctx.drawImage(img, 0, 0, width, height);

      const mimeType = "image/jpeg";
      const compressedDataUrl = canvas.toDataURL(mimeType, COMPRESS_QUALITY);
      const approxBytes = Math.round((compressedDataUrl.length * 3) / 4);

      resolve({
        dataUrl: compressedDataUrl,
        size: approxBytes,
        width,
        height,
      });
    };

    img.onerror = (err) => {
      reject(err);
    };

    if (typeof fileOrDataUrl === "string") {
      img.src = fileOrDataUrl;
    } else {
      const reader = new FileReader();
      reader.onload = (e) => {
        if (e.target?.result) {
          img.src = e.target.result as string;
        } else {
          reject(new Error("读取文件失败"));
        }
      };
      reader.onerror = reject;
      reader.readAsDataURL(fileOrDataUrl);
    }
  });
}
