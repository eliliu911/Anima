import { useRef, useState } from "react";
import { assetsAddVariant, type AssetView } from "../../../api/assets";
import { ACCEPTED_TYPES, uploadNote, uploadProblem } from "./uploadImage";
import { isAcceptedImagePath, NATIVE_DROP_ATTR, useNativeFileDrop } from "../nativeDrop";

/**
 * 「上传图片」入口：点选或把图拖进来，加成这个资产的一版图并设为封面。
 *
 * 可以一次选多张，逐张加；最后一张成为封面。
 * 传完之后已经合成过提示词的镜头会提示「要重新合成」—— 重新合成就会把这张挂成参考图。
 */
export function UploadImageButton({ asset, onDone, lookId, label }: {
  asset: AssetView;
  onDone: () => void;
  /** 传到哪个形象。不传 = 主形象 */
  lookId?: string;
  label?: string;
}) {
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);
  const [over, setOver] = useState(false);
  const zoneRef = useRef<HTMLLabelElement | null>(null);

  /**
   * 从访达拖进来的走这条：Tauri 给的是本地路径，不是 `File`（见 `nativeDrop.ts`）。
   * 后端的 `{kind: "path"}` 会自己把文件拷进资产库。
   */
  const uploadPaths = async (paths: string[]) => {
    setBusy(true);
    setMessage(null);
    const failed: string[] = [];
    let added = 0;
    for (const path of paths) {
      const name = path.split("/").pop() || path;
      if (!isAcceptedImagePath(path)) {
        failed.push(`「${name}」不是 PNG / JPG / WebP 图片`);
        continue;
      }
      try {
        await assetsAddVariant({ scope: asset.scope, id: asset.id, media: { kind: "path", path }, prompt_used: uploadNote(name), look_id: lookId ?? null });
        added += 1;
      } catch (e) {
        failed.push(`「${name}」：${String(e)}`);
      }
    }
    setBusy(false);
    if (added > 0) onDone();
    setMessage(
      failed.length
        ? { ok: false, text: `${added ? `传上了 ${added} 张；` : ""}${failed.join("；")}` }
        : { ok: true, text: `传上了 ${added} 张，已设为封面。用到它的镜头重新合成提示词就会挂上。` },
    );
  };

  useNativeFileDrop(
    (zone) => zone === zoneRef.current,
    {
      onOver: (zone) => setOver(!!zone),
      onDrop: (_zone, paths) => {
        if (!busy) void uploadPaths(paths);
      },
    },
  );

  const upload = async (files: File[]) => {
    if (files.length === 0) return;
    setBusy(true);
    setMessage(null);
    const failed: string[] = [];
    let added = 0;
    for (const file of files) {
      const problem = uploadProblem(file);
      if (problem) {
        failed.push(problem);
        continue;
      }
      try {
        const dataUrl = await new Promise<string>((ok, no) => {
          const r = new FileReader();
          r.onload = () => ok(String(r.result));
          r.onerror = () => no(new Error(`读不出「${file.name}」`));
          r.readAsDataURL(file);
        });
        await assetsAddVariant({ scope: asset.scope, id: asset.id, media: { kind: "data_url", data_url: dataUrl }, prompt_used: uploadNote(file.name), look_id: lookId ?? null });
        added += 1;
      } catch (e) {
        failed.push(`「${file.name}」：${String(e)}`);
      }
    }
    setBusy(false);
    if (added > 0) onDone();
    setMessage(
      failed.length
        ? { ok: false, text: `${added ? `传上了 ${added} 张；` : ""}${failed.join("；")}` }
        : { ok: true, text: `传上了 ${added} 张，已设为封面。用到它的镜头重新合成提示词就会挂上。` },
    );
  };

  return (
    <div>
      <label
        ref={zoneRef}
        {...{ [NATIVE_DROP_ATTR]: "" }}
        className="flex cursor-pointer items-center justify-center rounded-lg border border-dashed px-3 py-2.5 text-[12px]"
        style={{
          borderColor: over ? "var(--studio-primary)" : "var(--studio-border)",
          color: over ? "var(--studio-primary)" : "var(--studio-text-2)",
          opacity: busy ? 0.6 : 1,
        }}
        onDragOver={(e) => {
          e.preventDefault();
          setOver(true);
        }}
        onDragLeave={() => setOver(false)}
        onDrop={(e) => {
          e.preventDefault();
          setOver(false);
          if (!busy) void upload([...e.dataTransfer.files]);
        }}
      >
        {busy ? "上传中…" : label ?? "上传图片（点选或拖进来）"}
        <input
          type="file"
          accept={ACCEPTED_TYPES.join(",")}
          multiple
          disabled={busy}
          className="hidden"
          onChange={(e) => {
            const files = [...(e.target.files ?? [])];
            e.target.value = "";
            void upload(files);
          }}
        />
      </label>
      {message && (
        <p className="mt-1 text-[11px] leading-relaxed" style={{ color: message.ok ? "var(--studio-done-fg)" : "var(--studio-failed-fg)" }}>
          {message.text}
        </p>
      )}
    </div>
  );
}
