/**
 * 新建资产。
 *
 * **图可以留空** —— 先有身份，再补图。写剧本时先把「老茶师」这个名字
 * 建出来，提示词里就能 `@老茶师`，图什么时候生成是另一回事。
 */
import { useState } from "react";
import { MediaThumb } from "../MediaThumb";
import {
  assetsCreate,
  KIND_LABEL,
  type AssetKind,
  type MediaInput,
  type Scope,
} from "../../../api/assets";

export function NewAssetDialog({
  onClose,
  onCreated,
  /** 默认建到哪一级。资产库页面默认全局。 */
  scope = { kind: "global" },
  /** 从创作台「录入」进来时预填的那张图（本机路径）。 */
  initialMedia = null,
  /** 预填描述。创作台带的是生成用的提示词。 */
  initialDescription = "",
  initialName = "",
}: {
  onClose: () => void;
  onCreated: () => void;
  scope?: Scope;
  initialMedia?: string | null;
  initialDescription?: string;
  initialName?: string;
}) {
  const [kind, setKind] = useState<AssetKind>("character");
  const [name, setName] = useState(initialName);
  const [description, setDescription] = useState(initialDescription);
  const [media, setMedia] = useState<MediaInput | null>(
    initialMedia ? { kind: "path", path: initialMedia } : null,
  );
  const [preview, setPreview] = useState<string | null>(initialMedia);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const pickFile = (file: File) => {
    const reader = new FileReader();
    reader.onload = () => {
      const url = String(reader.result);
      // data URL 只活到入库那一刻 —— 后端落盘后存的是路径。
      setMedia({ kind: "data_url", data_url: url });
      setPreview(url);
    };
    reader.readAsDataURL(file);
  };

  const submit = async () => {
    if (!name.trim()) {
      setError("先给它起个名字");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await assetsCreate({ scope, kind, name: name.trim(), description, media });
      onCreated();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      className="studio-scrim fixed inset-0 z-50 flex items-center justify-center"
    >
      <div
        className="studio-modal max-h-[86vh] w-[440px] max-w-[92vw] overflow-y-auto"
        onClick={(e) => e.stopPropagation()}
      >
        <header
          className="flex items-start justify-between px-6 pb-4 pt-5"
          style={{ borderBottom: "1px solid var(--studio-border-subtle)" }}
        >
          <div>
            <h2 className="studio-display text-[17px]">新建资产</h2>
            <p className="mt-0.5 text-[12px]" style={{ color: "var(--studio-text-muted)" }}>
              {scope.kind === "global" ? "录入全局共享池" : "建在当前项目里"}
            </p>
          </div>
          <button type="button" onClick={onClose} className="px-1 text-[15px] opacity-60">
            ✕
          </button>
        </header>

        <div className="flex flex-col gap-4 px-6 py-5">
          <div className="flex items-center gap-2">
            <span className="studio-eyebrow w-[56px] shrink-0">类型</span>
            <div className="flex gap-1">
              {(["character", "scene", "prop"] as AssetKind[]).map((k) => (
                <button
                  key={k}
                  type="button"
                  onClick={() => setKind(k)}
                  className="rounded-full px-3 py-1.5 text-[12px]"
                  style={{
                    background:
                      kind === k ? "var(--studio-primary)" : "var(--studio-bg-inset)",
                    color: kind === k ? "var(--studio-on-primary)" : "var(--studio-text-2)",
                  }}
                >
                  {KIND_LABEL[k]}
                </button>
              ))}
            </div>
          </div>

          <div>
            <label className="studio-eyebrow mb-1 block">名称</label>
            <input
              autoFocus
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="提示词里 @ 的就是这个名字"
              className="w-full rounded-lg px-3 py-2 text-[13px] outline-none"
              style={inputStyle}
            />
          </div>

          <div>
            <label className="studio-eyebrow mb-1 block">描述（可选）</label>
            <textarea
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              rows={3}
              placeholder="外观、性格或用途 —— @ 引用时会作为提示词上下文"
              className="w-full resize-y rounded-lg px-3 py-2 text-[13px] outline-none"
              style={inputStyle}
            />
          </div>

          <div>
            <label className="studio-eyebrow mb-1 block">图片（可选）</label>
            <div className="flex items-center gap-3">
              <label
                className="flex h-[64px] w-[64px] cursor-pointer items-center justify-center overflow-hidden rounded-lg text-[11px]"
                style={{ background: "var(--studio-bg-inset)", border: "1px dashed var(--studio-border)" }}
              >
                {preview ? (
                  preview.startsWith("data:") ? (
                    <img src={preview} alt="" className="h-full w-full object-contain" />
                  ) : (
                    <MediaThumb path={preview} className="h-full w-full object-contain" />
                  )
                ) : (
                  <span style={{ color: "var(--studio-text-muted)" }}>+</span>
                )}
                <input
                  type="file"
                  accept="image/*,video/*"
                  className="hidden"
                  onChange={(e) => {
                    const f = e.target.files?.[0];
                    if (f) pickFile(f);
                  }}
                />
              </label>
              <p className="text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
                留空也能建。先有身份，图什么时候生成是另一回事。
              </p>
            </div>
          </div>

          {error && (
            <p className="text-[12px]" style={{ color: "var(--studio-danger)" }}>
              {error}
            </p>
          )}
        </div>

        <footer
          className="flex justify-end gap-2 px-6 py-4"
          style={{ borderTop: "1px solid var(--studio-border-subtle)" }}
        >
          <button
            type="button"
            onClick={onClose}
            className="rounded-lg px-4 py-2 text-[13px]"
            style={{ background: "var(--studio-bg-inset)", color: "var(--studio-text-2)" }}
          >
            取消
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={() => void submit()}
            className="rounded-lg px-4 py-2 text-[13px] font-medium"
            style={{ background: "var(--studio-primary)", color: "var(--studio-on-primary)", opacity: busy ? 0.6 : 1 }}
          >
            {busy ? "创建中…" : "创建"}
          </button>
        </footer>
      </div>
    </div>
  );
}

const inputStyle: React.CSSProperties = {
  background: "var(--studio-bg-inset)",
  border: "1px solid var(--studio-border-subtle)",
  color: "var(--studio-text)",
};
