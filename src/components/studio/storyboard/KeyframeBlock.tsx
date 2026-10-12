/**
 * 分镜详情里的「关键帧」一栏：首帧、尾帧各一格，可以出、可以导入（点选或拖进来）、可以摘掉。
 *
 * 出片方式按挂了哪几张自动定（见 `keyframes.ts`），这里只显示认出来的是哪一种。
 */
import { useRef, useState } from "react";
import type { MediaInput } from "../../../api/assets";
import {
  cinemaKeyframePreview,
  cinemaRenderEndFrame,
  cinemaRenderShotFrame,
  cinemaSetShotKeyframe,
  type KeyframePreview,
  type PromptRoute,
  type StoryboardShot,
} from "../../../api/cinema";
import { MediaThumb } from "../MediaThumb";
import { isAcceptedImagePath, NATIVE_DROP_ATTR, useNativeFileDrop } from "../nativeDrop";
import { ACCEPTED_TYPES, uploadProblem } from "../assets/uploadImage";
import { KEYFRAME_MODE_HINT, KEYFRAME_MODE_LABEL, KEYFRAME_RESYNTH_NOTE, keyframeMode } from "./keyframes";

type Which = "first" | "last";

export function KeyframeBlock({
  projectId,
  episodeId,
  shot,
  route,
  frameRunning,
  onChanged,
  framePrompt,
  onFramePrompt,
  endFrame,
  onEndFrame,
  fallbackEnding,
  save,
}: {
  projectId: string;
  episodeId: string;
  shot: StoryboardShot;
  route: PromptRoute;
  /** 这一镜有首帧 / 尾帧任务在跑 */
  frameRunning: boolean;
  /** 挂上 / 摘掉 / 出完之后重读分集 */
  onChanged: () => void;
  /**
   * 首帧提示词（当前线路合成出的分镜提示词，草稿里的）和尾帧「结束那一刻的画面」。
   * 用户 2026-09-28：「没有首尾帧提示词吗？直接就出来了，不对都没法修改。」—— 以前首帧那段藏在最下面、
   * 尾帧那段根本看不到。现在就在首帧 / 尾帧下面，改完再点重出
   */
  framePrompt: string;
  onFramePrompt: (v: string) => void;
  endFrame: string;
  onEndFrame: (v: string) => void;
  /** 尾帧没写时用的：最后一段画面 */
  fallbackEnding: string;
  /** 把草稿存下来（出图、看实际发送都读盘上的，先存再动）。回存没存上 */
  save: () => Promise<boolean>;
}) {
  const [busy, setBusy] = useState<Which | null>(null);
  const [error, setError] = useState<string | null>(null);
  const mode = keyframeMode(shot.keyframes);
  const has = { first: !!shot.keyframes?.first, last: !!shot.keyframes?.last };

  const [preview, setPreview] = useState<Partial<Record<Which, KeyframePreview | string>>>({});
  const run = async (which: Which, work: () => Promise<unknown>) => {
    setBusy(which);
    setError(null);
    try {
      await work();
      onChanged();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };
  const put = (which: Which, media: MediaInput | null) =>
    run(which, () => cinemaSetShotKeyframe(projectId, episodeId, shot.id, which, media));
  /** 出图 / 看实际发送之前先存草稿：后端读的是盘上的提示词，不存就还是改之前那份 */
  const saved = async () => {
    if (!(await save())) throw new Error("提示词没存上 —— 先看上面的保存报错");
  };
  const togglePreview = async (which: Which) => {
    if (preview[which]) {
      setPreview((p) => ({ ...p, [which]: undefined }));
      return;
    }
    try {
      await saved();
      const got = await cinemaKeyframePreview(projectId, episodeId, shot.id, which);
      setPreview((p) => ({ ...p, [which]: got }));
    } catch (e) {
      setPreview((p) => ({ ...p, [which]: String(e) }));
    }
  };

  const locked = busy !== null || frameRunning;
  return (
    <div className="flex flex-col gap-2">
      <p className="text-[12px]">
        出片方式：<b>{KEYFRAME_MODE_LABEL[mode]}</b>
        <span className="ml-1.5 text-[11px]" style={{ color: mode === "last_only" ? "var(--studio-failed-fg)" : "var(--studio-text-muted)" }}>
          {KEYFRAME_MODE_HINT[mode]}
        </span>
      </p>
      <div className="grid gap-2" style={{ gridTemplateColumns: "repeat(2, minmax(0,1fr))" }}>
        <Slot
          which="first"
          title="首帧"
          url={shot.image_url}
          version={shot.image_rev}
          present={has.first}
          locked={locked}
          working={busy === "first" || (frameRunning && busy === null)}
          generate={{
            label: has.first ? "重出首帧" : "出首帧",
            disabled: false,
            why: "",
            onClick: () => void run("first", async () => {
              await saved();
              await cinemaRenderShotFrame(projectId, episodeId, shot.id);
            }),
          }}
          onPut={(m) => void put("first", m)}
          onError={setError}
        >
          <PromptBox
            label={`首帧提示词（线路 ${route.toUpperCase()}；实际发出去的以「查看实际发送」为准）`}
            value={framePrompt}
            placeholder="留空 = 按镜头表自动拼一份（不调模型、不花 token）；也可以直接写这一镜开头那一刻的画面"
            onChange={onFramePrompt}
            onBlur={() => void save()}
            preview={preview.first}
            onPreview={() => void togglePreview("first")}
            note="出首帧会挂这一镜的场景、人物、道具图（这一步是资产进片子的地方：挂了首帧之后，出片只发首帧，不再单独发资产图）"
          />
        </Slot>
        <Slot
          which="last"
          title="尾帧"
          url={shot.end_image_url}
          present={has.last}
          locked={locked}
          working={busy === "last"}
          generate={{
            label: has.last ? "重出尾帧" : "出尾帧",
            disabled: !has.first,
            why: "尾帧照着首帧出（地方、光线、衣服都跟首帧一致），先出或导入首帧",
            onClick: () => void run("last", async () => {
              await saved();
              await cinemaRenderEndFrame(projectId, episodeId, shot.id);
            }),
          }}
          onPut={(m) => void put("last", m)}
          onError={setError}
        >
          <PromptBox
            label="尾帧：这一镜结束那一刻的画面"
            value={endFrame}
            placeholder={fallbackEnding ? `留空 = 用最后一段画面：${fallbackEnding}` : "写这一镜结束那一刻的画面"}
            onChange={onEndFrame}
            onBlur={() => void save()}
            preview={has.first ? preview.last : undefined}
            onPreview={has.first ? () => void togglePreview("last") : undefined}
            note="出尾帧把首帧当第 1 张参考图（地方、光线、衣服照它），再加人物图保脸；这里只写结束时的动作和构图"
          />
        </Slot>
      </div>
      <p className="text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
        导入的图会裁成项目画幅。{KEYFRAME_RESYNTH_NOTE}。出首帧会挂这一镜的场景、人物、道具图（要项目设置里的首帧模型是参考生图）。
      </p>
      {error && <p className="text-[11px]" style={{ color: "var(--studio-failed-fg)" }}>{error}</p>}
    </div>
  );
}

/** 首帧 / 尾帧下面的提示词框 + 「查看实际发送」 */
function PromptBox({
  label,
  value,
  placeholder,
  onChange,
  onBlur,
  preview,
  onPreview,
  note,
}: {
  label: string;
  value: string;
  placeholder: string;
  onChange: (v: string) => void;
  onBlur: () => void;
  preview?: KeyframePreview | string;
  onPreview?: () => void;
  note: string;
}) {
  const name = (p: string) => p.split("/").pop() ?? p;
  return (
    <div className="mt-1 flex flex-col gap-1">
      <span className="text-[11px]" style={{ color: "var(--studio-text-muted)" }}>{label}</span>
      <textarea
        className="w-full rounded-md px-2 py-1.5 text-[12px] leading-5 outline-none"
        style={{ background: "var(--studio-bg-elevated)", boxShadow: "0 0 0 1px var(--studio-border)", color: "var(--studio-text)" }}
        rows={4}
        value={value}
        placeholder={placeholder}
        onChange={(e) => onChange(e.target.value)}
        onBlur={onBlur}
      />
      <div className="flex flex-wrap items-center gap-2 text-[10.5px]" style={{ color: "var(--studio-text-muted)" }}>
        {onPreview && (
          <button type="button" className="rounded px-1.5 py-0.5" style={{ boxShadow: "0 0 0 1px var(--studio-border)", color: "var(--studio-text-2)" }} onClick={onPreview}>
            {preview ? "收起" : "查看实际发送"}
          </button>
        )}
        <span>{note}</span>
      </div>
      {preview && (
        typeof preview === "string" ? (
          <p className="text-[11px]" style={{ color: "var(--studio-failed-fg)" }}>{preview}</p>
        ) : (
          <div className="flex flex-col gap-1 rounded-md px-2 py-1.5 text-[11px]" style={{ background: "var(--studio-bg-inset)", boxShadow: "0 0 0 1px var(--studio-border-subtle)" }}>
            <span>
              引擎 <b>{preview.engine}</b> · 挂 {preview.images.length} 张参考图
              {preview.images.length > 0 && `：${preview.images.map((p, i) => `图 ${i + 1} ${name(p)}`).join("、")}`}
            </span>
            <pre className="max-h-64 overflow-auto whitespace-pre-wrap font-mono text-[10.5px] leading-4" style={{ color: "var(--studio-text-2)" }}>{preview.prompt}</pre>
            {preview.negative_prompt && <span style={{ color: "var(--studio-text-muted)" }}>负向：{preview.negative_prompt}</span>}
          </div>
        )
      )}
    </div>
  );
}

function Slot({
  which,
  title,
  url,
  version,
  present,
  locked,
  working,
  generate,
  onPut,
  onError,
  children,
}: {
  which: Which;
  title: string;
  url?: string | null;
  version?: number;
  present: boolean;
  locked: boolean;
  working: boolean;
  generate: { label: string; disabled: boolean; why: string; onClick: () => void };
  onPut: (m: MediaInput | null) => void;
  onError: (e: string) => void;
  children?: React.ReactNode;
}) {
  const zoneRef = useRef<HTMLLabelElement | null>(null);
  const [over, setOver] = useState(false);

  // 从访达拖进来的是本地路径（见 nativeDrop.ts）
  useNativeFileDrop((zone) => zone === zoneRef.current, {
    onOver: (zone) => setOver(!!zone),
    onDrop: (_zone, paths) => {
      const p = paths[0];
      if (!p || locked) return;
      if (!isAcceptedImagePath(p)) onError("只收 PNG / JPG / WebP 图片");
      else onPut({ kind: "path", path: p });
    },
  });

  const pick = async (file: File | undefined) => {
    if (!file) return;
    const problem = uploadProblem(file);
    if (problem) return onError(problem);
    const dataUrl = await new Promise<string>((ok, no) => {
      const r = new FileReader();
      r.onload = () => ok(String(r.result));
      r.onerror = () => no(new Error(`读不出「${file.name}」`));
      r.readAsDataURL(file);
    }).catch((e) => (onError(String(e)), null));
    if (dataUrl) onPut({ kind: "data_url", data_url: dataUrl });
  };

  const btn = "rounded-md px-2 py-0.5 text-[11px] disabled:opacity-40";
  const btnStyle = { background: "var(--studio-bg-elevated)", boxShadow: "0 0 0 1px var(--studio-border)", color: "var(--studio-text)" } as const;
  return (
    <div className="flex min-w-0 flex-col gap-1" data-keyframe-slot={which}>
      <span className="text-[11.5px]" style={{ color: "var(--studio-text-2)" }}>{title}</span>
      <label
        ref={zoneRef}
        {...{ [NATIVE_DROP_ATTR]: "" }}
        className="relative flex aspect-video cursor-pointer items-center justify-center overflow-hidden rounded-md border border-dashed text-[11px]"
        style={{ borderColor: over ? "var(--studio-primary)" : "var(--studio-border)", color: "var(--studio-text-muted)", opacity: locked ? 0.7 : 1 }}
        onDragOver={(e) => {
          e.preventDefault();
          setOver(true);
        }}
        onDragLeave={() => setOver(false)}
        onDrop={(e) => {
          e.preventDefault();
          setOver(false);
          if (!locked) void pick(e.dataTransfer.files[0]);
        }}
      >
        {present && url ? (
          <MediaThumb path={url} version={version ?? url} alt={title} className="h-full w-full object-contain" />
        ) : (
          <span>{working ? "正在出…" : "拖图进来或点这里导入"}</span>
        )}
        {working && present && (
          <span className="absolute inset-x-0 bottom-0 py-0.5 text-center text-[10.5px]" style={{ background: "var(--studio-bg-elevated)", color: "var(--studio-text)" }}>正在出…</span>
        )}
        <input
          type="file"
          accept={ACCEPTED_TYPES.join(",")}
          disabled={locked}
          className="hidden"
          onChange={(e) => {
            const f = e.target.files?.[0];
            e.target.value = "";
            void pick(f);
          }}
        />
      </label>
      <div className="flex flex-wrap gap-1">
        <button type="button" className={btn} style={btnStyle} disabled={locked || generate.disabled} title={generate.disabled ? generate.why : undefined} onClick={generate.onClick}>
          {generate.label}
        </button>
        {present && (
          <button type="button" className={btn} style={{ ...btnStyle, color: "var(--studio-failed-fg)" }} disabled={locked} onClick={() => onPut(null)}>
            摘掉
          </button>
        )}
      </div>
      {generate.disabled && <span className="text-[10.5px]" style={{ color: "var(--studio-text-muted)" }}>{generate.why}</span>}
      {children}
    </div>
  );
}
