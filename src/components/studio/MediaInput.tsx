import type * as React from "react";
import { MediaThumb } from "./MediaThumb";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { loadDroppedAttachment, saveChatImageAttachment } from "../../api";
import { isAcceptedImagePath, NATIVE_DROP_ATTR, useNativeFileDrop } from "./nativeDrop";
import { useStudioStore } from "./useStudioStore";

/**
 * 媒体输入。**槽位数由模式决定** —— 首尾帧要两张，其余要一张。
 *
 * 槽位标题分开写（首帧 / 末帧），因为塞反了不会报错：出来的片子首尾颠倒，
 * 而这一眼看不出是塞反了还是模型没听话。
 *
 * # 为什么要落盘再传路径
 *
 * Tauri 的 webview 里 `<input type="file">` 拿不到真实路径（安全限制），
 * 而后端的 `generate_image` / `animate_image` 吃的是**本地路径**。
 * 所以走 `save_chat_image_attachment`（已有的命令，正是为这条鸿沟写的）：
 * 把 base64 落成文件、返回可以直接交给工具的路径。
 */
export function MediaInput({
  count,
  hint,
  slotLabels,
  onPickFromLibrary,
}: {
  count: number;
  /** 每个槽上写什么。给了就用它（参考生图：场景 / 人物），不给按原规则 */
  slotLabels?: string[];
  /** 这个模式对图的要求，一句话 */
  hint: string;
  onPickFromLibrary: (slot: number) => void;
}) {
  const { t } = useTranslation(["studio", "common"]);
  const images = useStudioStore((s) => s.images);
  const setImage = useStudioStore((s) => s.setImage);
  const inputs = useRef<(HTMLInputElement | null)[]>([]);
  const [busy, setBusy] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [drag, setDrag] = useState<number | null>(null);

  const slotLabel = (i: number) =>
    slotLabels?.[i] ??
    (count === 2
      ? i === 0
        ? t("studio:firstFrame", { defaultValue: "首帧" })
        : t("studio:lastFrame", { defaultValue: "末帧" })
      : t("studio:sourceImage", { defaultValue: "图片" }));

  async function onPick(i: number, file: File) {
    setBusy(i);
    setError(null);
    try {
      const dataUrl = await new Promise<string>((resolve, reject) => {
        const r = new FileReader();
        r.onload = () => resolve(r.result as string);
        r.onerror = () => reject(r.error);
        r.readAsDataURL(file);
      });
      const saved = await saveChatImageAttachment({
        dataUrl,
        filename: file.name || `studio_${Date.now()}.png`,
      });
      setImage(i, saved.path);
    } catch (e) {
      // 落盘失败要说出来 —— 静默失败的话用户会以为图选上了，
      // 然后拿到一段和参考图无关的片子
      setError(String(e));
    } finally {
      setBusy(null);
    }
  }

  // 从访达拖进来的：Tauri 给本地路径，HTML5 的 onDrop 拿不到（见 `nativeDrop.ts`）。
  // `loadDroppedAttachment` 会把图拷进上传目录 —— 和点选那条一样，不引用访达里的原件。
  const root = useRef<HTMLDivElement | null>(null);
  async function onDropPath(i: number, path: string) {
    if (!isAcceptedImagePath(path)) {
      setError(`「${path.split("/").pop()}」不是 PNG / JPG / WebP 图片`);
      return;
    }
    setBusy(i);
    setError(null);
    try {
      const info = await loadDroppedAttachment({ path });
      setImage(i, info.local_path ?? path);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  }
  useNativeFileDrop((zone) => !!root.current?.contains(zone), {
    onOver: (zone) => setDrag(zone ? Number(zone.getAttribute("data-slot")) : null),
    onDrop: (zone, paths) => void onDropPath(Number(zone.getAttribute("data-slot")), paths[0]),
  });

  async function onDrop(i: number, e: React.DragEvent) {
    e.preventDefault();
    setDrag(null);
    const f = e.dataTransfer.files?.[0];
    if (f) await onPick(i, f);
  }

  return (
    <div ref={root} className="flex flex-col gap-3">
      {Array.from({ length: count }, (_, i) => (
        <div key={i} className="flex flex-col gap-2">
          {count > 1 && (
            <span
              className="font-mono text-[0.5625rem] uppercase tracking-[0.14em]"
              style={{ color: "var(--studio-text-muted)" }}
            >
              {slotLabel(i)}
            </span>
          )}
          <input
            type="file"
            accept="image/*"
            className="hidden"
            ref={(el) => {
              inputs.current[i] = el;
            }}
            onChange={(e) => {
              const f = e.target.files?.[0];
              if (f) void onPick(i, f);
              e.target.value = "";
            }}
          />

          {/* 拖拽区。**已选图就显示图本身** —— 显示路径字符串没用，
              人是靠看图确认自己选对了没有。 */}
          <div
            {...{ [NATIVE_DROP_ATTR]: "", "data-slot": i }}
            onClick={() => inputs.current[i]?.click()}
            onDragOver={(e) => {
              e.preventDefault();
              setDrag(i);
            }}
            onDragLeave={() => setDrag(null)}
            onDrop={(e) => void onDrop(i, e)}
            className="flex min-h-[128px] cursor-pointer flex-col items-center justify-center gap-2 rounded-xl px-4 py-5 text-center transition-colors"
            style={{
              background: "var(--studio-bg-inset)",
              border: `1px dashed ${
                drag === i ? "var(--studio-primary)" : "var(--studio-border)"
              }`,
            }}
          >
            {images[i] ? (
              <>
                <MediaThumb
                  path={images[i]}
                  className="max-h-[120px] rounded-lg object-contain"
                />
                <span
                  className="line-clamp-1 break-all font-mono text-[0.5625rem]"
                  style={{ color: "var(--studio-text-muted)" }}
                >
                  {images[i].split("/").pop()}
                </span>
              </>
            ) : busy === i ? (
              <span className="text-[12px]" style={{ color: "var(--studio-text-muted)" }}>
                {t("studio:saving", { defaultValue: "保存中…" })}
              </span>
            ) : (
              <>
                <IconUpload />
                <span className="text-[12px]" style={{ color: "var(--studio-text-2)" }}>
                  {t("studio:dropHere", { defaultValue: "拖拽或点击上传" })}
                </span>
                <span className="text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
                  {hint}
                </span>
              </>
            )}
          </div>

          <div className="flex gap-2">
            <button
              type="button"
              onClick={() => inputs.current[i]?.click()}
              className="studio-input flex-1 py-2 text-[12px]"
            >
              {t("studio:uploadLocal", { defaultValue: "本地上传" })}
            </button>
            <button
              type="button"
              onClick={() => onPickFromLibrary(i)}
              className="studio-input flex-1 py-2 text-[12px]"
            >
              {t("studio:fromLibrary", { defaultValue: "从资产库选取" })}
            </button>
          </div>
        </div>
      ))}
      {error && (
        <p className="text-[11px]" style={{ color: "var(--studio-failed-fg)" }}>
          {error}
        </p>
      )}
    </div>
  );
}

function IconUpload() {
  return (
    <svg width="26" height="26" viewBox="0 0 24 24" fill="none" stroke="currentColor"
         strokeWidth="1.4" style={{ color: "var(--studio-text-muted)" }}>
      <rect x="3" y="4.5" width="18" height="15" rx="2.5" />
      <circle cx="8.5" cy="9.5" r="1.6" />
      <path d="m3.5 16.5 4.7-4.2 4 3.4 3-2.5 5.3 4.3" strokeLinejoin="round" />
      <path d="M19 3v5M16.5 5.5h5" strokeLinecap="round" />
    </svg>
  );
}
