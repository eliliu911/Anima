import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { memo } from "react";
import ReactMarkdown, { defaultUrlTransform } from "react-markdown";
import remarkGfm from "remark-gfm";
import { DiffViewer, parseRawDiff } from "./DiffViewer";
import { MusicPlayerCard } from "./MusicPlayerCard";
import { useCodeViewer } from "../store/codeViewer";
import { FileIcon, FolderIcon, CopyIcon, CheckIcon } from "./Icons";
import { convertFileSrc } from "@tauri-apps/api/core";
import { useImageProgress, progressSlot } from "../store/imageProgress";
import { useVideoProgress, videoProgressSlot } from "../store/videoProgress";

interface MarkdownRendererProps {
  content: string;
  className?: string;
}

const LOCAL_IMAGE_RETRY_DELAY_MS = 400;
/// 本地图加载失败后的重试次数。
///
/// 原来是 **220 次 × 900ms ≈ 3.3 分钟**：那是在还没有进度事件的年代，
/// 靠不停重试去"等文件落盘"。代价是任何**真正的**加载失败都要静默转三分多钟
/// 才肯说话 —— 用户看到的是「图片已写入本地文件」的骨架屏一直转到对话结束。
///
/// 现在后端会在 `done` 那一刻带着 `output_path` 报上来，文件是**已经写好**的。
/// 所以这里只需要覆盖「写入和渲染之间那点时间差」，几秒足够；
/// 再失败就是真失败，该立刻说出来。
const LOCAL_IMAGE_MAX_RETRIES = 8;

function safeDecodeUri(value: string): string {
  try {
    return decodeURI(value);
  } catch {
    return value;
  }
}

export function encodeMarkdownLocalImageUrls(content: string): string {
  return content.replace(/!\[([^\]]*)\]\((file:\/\/[^)\n]+)\)/g, (_match, alt: string, url: string) => {
    const normalized = encodeURI(safeDecodeUri(url));
    return `![${alt}](${normalized})`;
  });
}

function markdownUrlTransform(url: string): string {
  if (url.startsWith("file://") || url.startsWith("data:image/")) {
    return url;
  }
  return defaultUrlTransform(url);
}

function isLocalMediaSrc(src: string): boolean {
  return src.startsWith("file://") || src.startsWith("/");
}

function addRetryNonce(src: string, retry: number): string {
  if (retry <= 0) return src;
  return `${src}${src.includes("?") ? "&" : "?"}anima_retry=${retry}`;
}

function isDirectoryPath(str: string): boolean {
  const s = str.trim().replace(/^file:\/\//, "");
  if (s.endsWith("/")) return true;
  const base = s.split("/").filter(Boolean).pop() || s;
  return (
    !base.includes(".") &&
    (s.includes("src") ||
      s.includes("docs") ||
      s.includes("poses") ||
      s.includes("cinema") ||
      s.includes("store") ||
      s.includes("components") ||
      s.includes("agent_runtime") ||
      s.includes("public"))
  );
}

function isFilePath(str: string): boolean {
  const s = str.trim().replace(/^file:\/\//, "");
  if (s.includes(" ") || s.length < 3) return false;
  if (s.endsWith("/")) return false;
  // 仅对代码和文本文件启用内置代码查看器按钮，图片等由媒体渲染组件处理
  const extMatch = s.match(
    /\.(rs|ts|tsx|js|jsx|json|md|toml|yaml|yml|css|html|py|sh|sql|txt)$/i
  );
  return !!extMatch;
}

function CodeBlockView({ lang, code }: { lang: string; code: string }) {
  const { t } = useTranslation("common");
  const [copied, setCopied] = useState(false);

  const handleCopy = () => {
    void navigator.clipboard.writeText(code);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  return (
    <div className="relative group my-2.5 rounded-xl overflow-hidden border border-[var(--line)] bg-[var(--paper)] text-[var(--ink)] font-mono shadow-2xs">
      {/* Code Header Bar */}
      <div className="flex items-center justify-between px-3.5 py-1.5 bg-[var(--surface-2)] border-b border-[var(--line)] text-[11px] text-[var(--ink-2)] select-none">
        <div className="flex items-center gap-2">
          <span className="w-2 h-2 rounded-full bg-[var(--ink-3)]/60" />
          <span className="font-semibold uppercase tracking-wider text-[var(--ink)]">{lang || "text"}</span>
        </div>
        <button
          type="button"
          onClick={handleCopy}
          className="text-[var(--ink-2)] hover:text-[var(--ink)] bg-[var(--surface-3)] hover:bg-[var(--surface-3)]/80 px-2 py-0.5 rounded-md transition-colors cursor-pointer flex items-center gap-1.5 text-[11px] border border-[var(--line)]"
          title={t("copy", { defaultValue: "复制代码" })}
        >
          {copied ? <CheckIcon className="w-3.5 h-3.5 text-emerald-500" /> : <CopyIcon className="w-3.5 h-3.5 text-[var(--ink-3)]" />}
          <span>{copied ? t("copied", { defaultValue: "已复制" }) : t("copy", { defaultValue: "复制" })}</span>
        </button>
      </div>

      {/* Code Content */}
      <pre className="p-3.5 overflow-x-auto text-[12px] leading-relaxed select-text m-0 bg-transparent">
        <code>{code}</code>
      </pre>
    </div>
  );
}

export function ImageGenerationSkeleton({
  label = "正在构思草图",
  width = 1024,
  height = 1024,
  engine,
}: {
  label?: string;
  width?: number;
  height?: number;
  /** 底模名。三个引擎实测耗时 45s / 331s / 39s，差 8.5 倍 —— 不显示的话
   *  用户分不清是在正常等待还是卡死了。 */
  engine?: string;
}) {
  const aspectRatio = `${Math.max(1, width)} / ${Math.max(1, height)}`;

  return (
    <div
      className="relative w-[min(360px,72vw)] max-w-full overflow-hidden rounded-2xl border border-[var(--line)] bg-[var(--paper)] shadow-2xs"
      style={{ aspectRatio }}
    >
      <div className="absolute inset-0 bg-[radial-gradient(circle_at_30%_22%,color-mix(in_srgb,var(--celadon)_16%,transparent),transparent_36%),radial-gradient(circle_at_72%_74%,color-mix(in_srgb,var(--amber)_10%,transparent),transparent_34%),linear-gradient(135deg,var(--surface),var(--paper)_42%,var(--surface-2))]" />
      <div className="absolute inset-0 opacity-[0.36] [background-image:linear-gradient(var(--line)_1px,transparent_1px),linear-gradient(90deg,var(--line)_1px,transparent_1px)] [background-size:42px_42px]" />
      <div className="absolute inset-y-0 left-0 w-1/2 shimmer-sweep bg-gradient-to-r from-transparent via-white/45 to-transparent dark:via-white/10" />

      {/* 左上角只放状态，底模名挪到底部进度条正上方。
          两者变化频率不同：状态每几秒换一次、底模一整轮不变。叠在一起时
          状态一跳眼睛会跟着去看底模名，而那行根本没变。放到进度条上方之后，
          「这一张的元信息」集中在底部一处，扫一眼就够。 */}
      <div className="absolute inset-x-5 top-4 flex items-center gap-2">
        <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-[var(--celadon)] animate-pulse" />
        <span className="truncate text-[11.5px] font-medium text-[var(--ink-2)]">{label}</span>
      </div>

      {engine && (
        <span className="absolute right-5 bottom-7 max-w-[60%] truncate rounded-full border border-[var(--line)] bg-[var(--surface-2)]/85 px-2 py-[1px] text-[10px] font-medium text-[var(--ink-3)]">
          {engine}
        </span>
      )}

      <div className="absolute inset-x-10 top-[36%] h-[30%] rounded-[999px] bg-[var(--celadon)]/10 blur-2xl" />

      {Array.from({ length: 18 }).map((_, i) => (
        <span
          key={i}
          className="absolute h-1.5 w-1.5 rounded-full bg-[var(--ink-3)]/25"
          style={{
            left: `${12 + ((i * 37) % 76)}%`,
            top: `${24 + ((i * 29) % 52)}%`,
            transform: `scale(${0.55 + (i % 4) * 0.18})`,
            animation: `twinkle-${i % 3 === 0 ? "d" : i % 3 === 1 ? "e" : "f"} ${4.2 + (i % 5) * 0.28}s linear infinite`,
            animationDelay: `${(i * 137) % 1600}ms`,
          }}
        />
      ))}

      <div className="absolute inset-x-5 bottom-4 h-1 overflow-hidden rounded-full bg-[var(--surface-3)]/70">
        <div className="h-full w-1/2 shimmer-sweep rounded-full bg-gradient-to-r from-transparent via-[var(--celadon)] to-transparent" />
      </div>
    </div>
  );
}

/**
 * 进度事件到了 `done` 就把 `output_path` 当图源用 —— **别等工具返回**。
 *
 * `src` 来自工具结果里解析出的路径，而工具要等整轮结束才返回。进度在文件
 * 落盘那一刻就带着路径了，用户看到的却是骨架屏上写着「图片已写入本地文件」
 * 还在转 —— 那一步之差没有任何必要。
 */
export function srcFromProgress(
  progress?: { stage: string; outputPath?: string },
): string | undefined {
  if (progress?.stage !== "done" || !progress.outputPath) return undefined;
  return progress.outputPath.startsWith("file://")
    ? progress.outputPath
    : `file://${progress.outputPath}`;
}

/**
 * 点开看大图。
 *
 * **为什么要自己做而不是靠系统右键菜单**：webview 的原生菜单里那几项
 * （Open Image in New Window / Download Image）在 Tauri 里是死的 ——
 * 没有浏览器外壳可以开新窗口，下载也没有接线。用户右键之后点了没反应，
 * 只会以为程序坏了。
 *
 * Esc 关闭、点背景关闭。**图片本身要吞掉点击** —— 否则想看细节点一下就关了。
 */
function ImageLightbox({ src, alt, onClose }: { src: string; alt?: string; onClose: () => void }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    // 打开时锁住背景滚动，关掉时**恢复原值**而不是硬写 "auto" —— 后者会覆盖别人设的值
    const prev = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      window.removeEventListener("keydown", onKey);
      document.body.style.overflow = prev;
    };
  }, [onClose]);

  return (
    <div
      className="fixed inset-0 z-[9999] flex items-center justify-center bg-black/80 backdrop-blur-sm"
      onClick={onClose}
    >
      <img
        src={src}
        alt={alt || ""}
        className="max-h-[92vh] max-w-[92vw] object-contain rounded-lg shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      />
      <button
        onClick={onClose}
        className="absolute right-6 top-6 rounded-full bg-white/10 px-3 py-1.5 text-[13px] text-white/90 hover:bg-white/20 transition-colors"
      >
        关闭 (Esc)
      </button>
    </div>
  );
}

export function ImageArtifactCard({
  src,
  alt,
  width,
  height,
  failed,
}: {
  src?: string;
  alt?: string;
  width?: number;
  height?: number;
  /**
   * 工具本身报了失败。
   *
   * **不能只靠 ComfyUI 的进度事件判失败** —— 有些失败发生在提交 ComfyUI
   * 之前（LoRA 名字对不上、模型挑不出来），那时一条进度都不会发，
   * 卡片就永远转骨架屏。
   */
  failed?: boolean;
}) {
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState(false);
  const [zoomed, setZoomed] = useState(false);
  const [waitingForRetry, setWaitingForRetry] = useState(false);
  const [retryAttempt, setRetryAttempt] = useState(0);

  // 后端每个阶段和**每条失败路径**都在报进度，按 prompt+尺寸归档。
  // 没有它的话，超时/报错的那张只会永远转骨架屏 —— 用户等半天不知道已经废了。
  const progress = useImageProgress(
    (s) => s.bySlot[progressSlot(alt || "", width, height)],
  );
  const generationFailed = (failed || progress?.stage === "error") && !loaded;

  // **文件写完就出图，别等工具返回。**
  //
  // `src` 来自工具结果里解析出的路径，而工具要等整轮结束才返回。
  // 进度事件在 `done` 那一刻就带着 `output_path` —— 用户看到的是
  // 骨架屏上写着「图片已写入本地文件」却还在转，那一步之差没有任何必要。
  const doneSrc = srcFromProgress(progress);

  const rawSrc = src || doneSrc || "";
  const decodedSrc = safeDecodeUri(rawSrc);
  const isLocalSrc = isLocalMediaSrc(decodedSrc);
  const resolvedBaseSrc = useMemo(() => {
    if (decodedSrc.startsWith("file://")) {
      return convertFileSrc(decodedSrc.slice(7));
    }
    if (decodedSrc.startsWith("/")) {
      return convertFileSrc(decodedSrc);
    }
    return decodedSrc;
  }, [decodedSrc]);
  const resolvedSrc = useMemo(
    () => addRetryNonce(resolvedBaseSrc, isLocalSrc && !loaded ? retryAttempt : 0),
    [isLocalSrc, loaded, resolvedBaseSrc, retryAttempt],
  );

  const rawPath = decodedSrc.startsWith("file://") ? decodedSrc.slice(7) : decodedSrc;
  const fileName = rawPath.split("/").pop() || "image.png";
  const loadedSrcRef = useRef<string | null>(null);

  useEffect(() => {
    // 只有当目标图片路径真正发生改变时，才重置 loaded 状态。
    // 如果之前通过 doneSrc 已经成功加载了同一个本地文件，当 markdown 正文流式解析出 src 时，
    // decodedSrc 会保持指向同一个文件，此时绝对不能重置为 false，否则会导致图片闪退回骨架屏。
    if (decodedSrc && loadedSrcRef.current === decodedSrc) {
      return;
    }
    setLoaded(false);
    setError(false);
    setWaitingForRetry(false);
    setRetryAttempt(0);
    loadedSrcRef.current = null;
  }, [decodedSrc]);

  useEffect(() => {
    if (!waitingForRetry || loaded || error) return;
    const timer = window.setTimeout(() => {
      setWaitingForRetry(false);
      setRetryAttempt((attempt) => attempt + 1);
    }, LOCAL_IMAGE_RETRY_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [error, loaded, waitingForRetry]);


  return (
    <div className="relative group my-3 max-w-fit select-none">
      {generationFailed && (
        // 尺寸**照抄骨架屏**：同样的 w-[min(360px,72vw)] + aspectRatio。
        // 之前写死 420×380，和旁边的骨架屏一个大一个小，看着格格不入 ——
        // 同一批图的卡片必须是同一个尺寸语言，只有配色能变。
        <div
          className="relative flex w-[min(360px,72vw)] max-w-full flex-col items-center justify-center gap-1.5 overflow-hidden rounded-2xl border border-[color-mix(in_srgb,var(--clay)_32%,transparent)] bg-[color-mix(in_srgb,var(--clay)_8%,transparent)] px-4 text-center shadow-2xs"
          style={{ aspectRatio: `${Math.max(1, width || 1024)} / ${Math.max(1, height || 1024)}` }}
        >
          <span className="text-[12.5px] font-medium text-[var(--clay)]">这张没生成出来</span>
          <span className="text-[11px] text-[var(--ink-3)] max-w-[300px] break-words">
            {progress?.message || progress?.label || "生成失败"}
          </span>
        </div>
      )}

      {!loaded && !error && !generationFailed && (
        <ImageGenerationSkeleton
          label={progress?.label || (retryAttempt > 0 ? "正在等待图片写入" : "正在构思草图")}
          engine={progress?.engine}
          width={width}
          height={height}
        />
      )}

      {resolvedSrc && !generationFailed ? (
        <img
          src={resolvedSrc}
          alt={alt || "Generated Image"}
          onLoad={() => {
            setLoaded(true);
            setError(false);
            setWaitingForRetry(false);
            loadedSrcRef.current = decodedSrc;
          }}
          onError={() => {
            if (isLocalSrc && retryAttempt < LOCAL_IMAGE_MAX_RETRIES) {
              setWaitingForRetry(true);
              return;
            }
            setWaitingForRetry(false);
            setError(true);
            loadedSrcRef.current = null;
          }}
          onClick={() => setZoomed(true)}
          className={`cursor-zoom-in max-h-[380px] max-w-[min(420px,82vw)] w-auto object-contain rounded-2xl border border-[var(--line)] bg-[var(--paper)] shadow-2xs transition-all duration-500 group-hover:scale-[1.01] ${
            loaded ? "opacity-100 block" : "opacity-0 absolute inset-0 pointer-events-none"
          }`}
          loading="lazy"
        />
      ) : null}

      {zoomed && resolvedSrc && (
        <ImageLightbox src={resolvedSrc} alt={alt} onClose={() => setZoomed(false)} />
      )}

      {/* 加载失败回退 */}
      {error && (
        // 尺寸照抄骨架屏，别再自成一套（同一批图的卡片必须是同一个尺寸语言）
        <div
          className="flex w-[min(360px,72vw)] max-w-full flex-col items-center justify-center gap-2 overflow-hidden rounded-2xl border border-[var(--line)] bg-[var(--surface-2)] p-4 text-[var(--ink-2)]"
          style={{ aspectRatio: `${Math.max(1, width || 1024)} / ${Math.max(1, height || 1024)}` }}
        >
          <span className="text-[12px] text-red-400 font-medium">图片加载失败</span>
          <span className="text-[10px] font-mono text-[var(--ink-3)] truncate max-w-[240px]">{rawPath}</span>
        </div>
      )}

      {/* 悬浮操作胶囊 */}
      {loaded && (
        <div className="absolute top-2.5 right-2.5 opacity-0 group-hover:opacity-100 transition-opacity duration-200 flex items-center gap-1.5 bg-[var(--surface)]/90 backdrop-blur-md px-2.5 py-1 rounded-full border border-[var(--line)] shadow-xs">
          <span className="text-[10.5px] font-mono text-[var(--ink-2)] px-1 truncate max-w-[150px]">
            {alt || fileName}
          </span>
        </div>
      )}
    </div>
  );
}

export function VideoGenerationSkeleton({
  label = "正在时序扩散渲染",
}: {
  label?: string;
}) {
  return (
    <div
      className="relative w-[min(400px,80vw)] max-w-full overflow-hidden rounded-2xl border border-[var(--line)] bg-[var(--paper)] shadow-2xs"
      style={{ aspectRatio: "16 / 9" }}
    >
      <div className="absolute inset-0 bg-[radial-gradient(circle_at_30%_22%,color-mix(in_srgb,var(--celadon)_20%,transparent),transparent_40%),radial-gradient(circle_at_72%_74%,color-mix(in_srgb,var(--accent)_15%,transparent),transparent_35%),linear-gradient(135deg,var(--surface),var(--paper)_42%,var(--surface-2))]" />
      <div className="absolute inset-0 opacity-[0.36] [background-image:linear-gradient(var(--line)_1px,transparent_1px),linear-gradient(90deg,var(--line)_1px,transparent_1px)] [background-size:42px_42px]" />
      <div className="absolute inset-y-0 left-0 w-1/2 shimmer-sweep bg-gradient-to-r from-transparent via-white/45 to-transparent dark:via-white/10" />

      <div className="absolute left-5 top-4 flex items-center gap-2">
        <span className="h-1.5 w-1.5 rounded-full bg-[var(--celadon)] animate-pulse" />
        <span className="text-[11.5px] font-medium text-[var(--ink-2)]">{label}</span>
      </div>

      <div className="absolute inset-0 flex items-center justify-center pointer-events-none">
        <div className="w-10 h-10 rounded-full border-2 border-[var(--celadon)]/40 border-t-[var(--celadon)] animate-spin" />
      </div>

      <div className="absolute inset-x-5 bottom-4 h-1 overflow-hidden rounded-full bg-[var(--surface-3)]/70">
        <div className="h-full w-1/2 shimmer-sweep rounded-full bg-gradient-to-r from-transparent via-[var(--celadon)] to-transparent" />
      </div>
    </div>
  );
}

export function VideoArtifactCard({
  src,
  alt,
  durationSecs,
  callId,
  failed,
}: {
  src?: string;
  alt?: string;
  /** 后端进度事件按 `prompt + 时长` 建索引，只传 alt 的话只有时长恰好是缺省值才对得上 */
  durationSecs?: number;
  /** 有 call id 时优先按它对齐 —— 同一轮生成两段视频时 prompt 可能相同 */
  callId?: string;
  /** 工具本身报了失败 —— 见 ImageArtifactCard 里同名字段的注释 */
  failed?: boolean;
}) {
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState(false);

  // **两条索引都要查。**
  //
  // 写入侧（CommandBar 的进度监听）同时存了 `call:${id}` 和
  // `${prompt} ${duration}s` 两份，读取侧原来只查后者、而且时长写死缺省值 ——
  // 于是只要模型给的 duration_secs 不是缺省值，进度就落进一个没人读的格子：
  // 卡片永远转骨架屏、失败提示也不显示，**而且不报任何错**。
  const byCall = useVideoProgress((s) => (callId ? s.bySlot[`call:${callId}`] : undefined));
  const bySlot = useVideoProgress((s) => s.bySlot[videoProgressSlot(alt || "", durationSecs)]);
  const progress = byCall ?? bySlot;
  const generationFailed = (failed || progress?.stage === "error") && !loaded;

  const doneSrc = progress?.stage === "done" && progress.outputPath
    ? (progress.outputPath.startsWith("file://") ? progress.outputPath : `file://${progress.outputPath}`)
    : undefined;

  const rawSrc = src || doneSrc || "";
  const decodedSrc = safeDecodeUri(rawSrc);
  const resolvedSrc = useMemo(() => {
    if (decodedSrc.startsWith("file://")) {
      return convertFileSrc(decodedSrc.slice(7));
    }
    if (decodedSrc.startsWith("/")) {
      return convertFileSrc(decodedSrc);
    }
    return decodedSrc;
  }, [decodedSrc]);

  const rawPath = decodedSrc.startsWith("file://") ? decodedSrc.slice(7) : decodedSrc;
  const fileName = rawPath.split("/").pop() || "video.mp4";

  return (
    <div className="relative group my-3 max-w-fit select-none">
      {generationFailed && (
        <div
          className="relative flex w-[min(400px,80vw)] max-w-full flex-col items-center justify-center gap-1.5 overflow-hidden rounded-2xl border border-[color-mix(in_srgb,var(--clay)_32%,transparent)] bg-[color-mix(in_srgb,var(--clay)_8%,transparent)] px-4 text-center shadow-2xs"
          style={{ aspectRatio: "16 / 9" }}
        >
          <span className="text-[12.5px] font-medium text-[var(--clay)]">视频未能生成成功</span>
          <span className="text-[11px] text-[var(--ink-3)] max-w-[300px] break-words">
            {progress?.message || progress?.label || "生成超时或失败"}
          </span>
        </div>
      )}

      {!loaded && !error && !generationFailed && (
        <VideoGenerationSkeleton
          label={progress?.label || "正在渲染动态视频"}
        />
      )}

      {resolvedSrc && !generationFailed ? (
        <video
          src={resolvedSrc}
          controls
          autoPlay
          loop
          muted
          playsInline
          onLoadedData={() => {
            setLoaded(true);
            setError(false);
          }}
          onError={() => {
            setError(true);
          }}
          className={`max-h-[380px] max-w-[min(480px,85vw)] w-auto rounded-2xl border border-[var(--line)] bg-[var(--paper)] shadow-2xs transition-all duration-500 ${
            loaded ? "opacity-100 block" : "opacity-0 absolute inset-0 pointer-events-none"
          }`}
        />
      ) : null}

      {/* 底部文件名标示 */}
      {loaded && (
        <div className="mt-1 flex items-center justify-between px-1 text-[11px] text-[var(--ink-3)]">
          <span className="font-mono truncate max-w-[200px]">{fileName}</span>
        </div>
      )}
    </div>
  );
}


/**
 * **必须 memo。**
 *
 * 流式期间每个 delta 都会换掉整个 `turns` 数组，于是 `ChatHistory` 的
 * `turns.map` 里每一轮都重渲染。不 memo 的话，**整条对话每一轮的 markdown
 * 都会被 remark 重新解析一遍**，而这是每个 delta 付一遍的 ——
 * 实测 20 轮 × 7.7KB 的对话约 94ms 一次，一条 400 delta 的流累计 30~40 秒。
 *
 * 表现是「流到一半不动、计时器和 token 也停、加载动画还转，几十秒后一次性全出来」，
 * 以及窗口拖不动（`decorations: false`，拖拽由 webview 的 JS 处理）——
 * 事件从没停过，是主线程被渲染占死了。
 *
 * memo 之后只有**内容真的变了的那一轮**（正在流的那条）会重新解析。
 */
function MarkdownRendererImpl({ content, className = "" }: MarkdownRendererProps) {
  const renderedContent = useMemo(() => encodeMarkdownLocalImageUrls(content), [content]);
  const components = useMemo(
    () => ({
      h1: ({ children }: any) => (
        <h1 className="text-[16px] font-bold text-[var(--ink)] mt-3 mb-1">{children}</h1>
      ),
      h2: ({ children }: any) => (
        <h2 className="text-[14.5px] font-bold text-[var(--ink)] mt-2.5 mb-1">{children}</h2>
      ),
      h3: ({ children }: any) => (
        <h3 className="text-[13.5px] font-bold text-[var(--ink)] mt-2 mb-0.5">{children}</h3>
      ),
      h4: ({ children }: any) => (
        <h4 className="text-[13px] font-bold text-[var(--ink)] mt-1.5 mb-0.5">{children}</h4>
      ),

      hr: () => <hr className="my-2.5 border-[var(--line)]" />,

      blockquote: ({ children }: any) => (
        <blockquote className="border-l-2 border-[var(--celadon)] pl-3 my-1.5 text-[var(--ink-2)] italic text-[12.5px] bg-[var(--surface-3)]/20 py-1 rounded-r-lg">
          {children}
        </blockquote>
      ),

      ul: ({ children }: any) => (
        <ul className="list-disc pl-5 space-y-1 text-[13px] my-1">{children}</ul>
      ),
      ol: ({ children }: any) => (
        <ol className="list-decimal pl-5 space-y-1 text-[13px] my-1">{children}</ol>
      ),
      li: ({ children }: any) => <li className="leading-relaxed">{children}</li>,

      p: ({ children }: any) => <p className="m-0 leading-relaxed break-words">{children}</p>,

      table: ({ children }: any) => (
        <div className="overflow-x-auto my-2 rounded-xl border border-[var(--line)]">
          <table className="min-w-full divide-y divide-[var(--line)] text-[12px]">{children}</table>
        </div>
      ),
      thead: ({ children }: any) => <thead className="bg-[var(--surface-2)]">{children}</thead>,
      tbody: ({ children }: any) => <tbody className="divide-y divide-[var(--line)]">{children}</tbody>,
      tr: ({ children }: any) => <tr>{children}</tr>,
      th: ({ children }: any) => (
        <th className="px-3 py-2 text-left font-semibold text-[var(--ink)]">{children}</th>
      ),
      td: ({ children }: any) => <td className="px-3 py-1.5 text-[var(--ink-2)]">{children}</td>,

      img: ({ src, alt }: any) => {
        const rawSrc = src || "";
        const isVideo =
          rawSrc.endsWith(".mp4") ||
          rawSrc.endsWith(".webm") ||
          rawSrc.endsWith(".mov") ||
          rawSrc.includes(".mp4?") ||
          alt?.toLowerCase().includes("video") ||
          alt?.toLowerCase().includes("视频");
        if (isVideo) {
          return <VideoArtifactCard src={src} alt={alt} />;
        }
        return <ImageArtifactCard src={src} alt={alt} />;
      },

      a: ({ href, children }: any) => {
        const rawHref = href || "";
        const titleStr = typeof children === "string" ? children : "";

        if (isDirectoryPath(rawHref) || isDirectoryPath(titleStr)) {
          return (
            <span className="inline-flex items-center gap-1.5 text-[var(--ink-2)] font-mono text-[12px]">
              <FolderIcon className="w-3.5 h-3.5" />
              <span>{children}</span>
            </span>
          );
        }

        const rawPath = rawHref.startsWith("file://") ? rawHref.slice(7) : rawHref;
        const cleanPath = (rawPath || titleStr).split("#")[0];

        const isImage =
          /\.(png|jpg|jpeg|webp|gif|svg)$/i.test(cleanPath) ||
          /\.(png|jpg|jpeg|webp|gif|svg)$/i.test(rawHref);
        if (isImage) {
          return <ImageArtifactCard src={rawHref} alt={titleStr || cleanPath} />;
        }

        const isLocalFile =
          rawHref.startsWith("file://") ||
          rawHref.startsWith("/") ||
          isFilePath(rawHref) ||
          isFilePath(titleStr);

        if (isLocalFile) {
          const lineMatch = (rawPath + " " + titleStr).match(/#L(\d+)(?:-L?(\d+))?/i);
          const lineRange: [number, number] | null = lineMatch
            ? [
                parseInt(lineMatch[1], 10),
                lineMatch[2] ? parseInt(lineMatch[2], 10) : parseInt(lineMatch[1], 10),
              ]
            : null;
          const ext = (titleStr || cleanPath).split(".").pop()?.toLowerCase() || "";

          return (
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                void useCodeViewer.getState().openFile(cleanPath, { lineRange });
              }}
              className="inline-flex items-center gap-1.5 text-[var(--celadon)] hover:text-emerald-400 font-mono font-semibold text-[12px] hover:underline cursor-pointer"
              title={`在右侧代码查看器中打开 ${titleStr || cleanPath}`}
            >
              <FileIcon ext={ext} className="w-3.5 h-3.5" />
              <span>{children}</span>
            </button>
          );
        }

        return (
          <a
            href={rawHref}
            target="_blank"
            rel="noreferrer"
            className="text-[var(--celadon)] hover:underline font-medium"
          >
            {children}
          </a>
        );
      },

      code: ({ className: codeClassName, children, ...props }: any) => {
        const match = /language-(\w+)/.exec(codeClassName || "");
        const codeString = String(children).replace(/\n$/, "");

        if (match && match[1].toLowerCase() === "diff") {
          return <DiffViewer diff={parseRawDiff(codeString)} />;
        }

        if (match && (match[1].toLowerCase() === "music" || match[1].toLowerCase() === "song" || match[1].toLowerCase() === "audio")) {
          try {
            const data = JSON.parse(codeString);
            if (data.audioUrl || data.audio_url || data.url || data.local_path) {
              return (
                <MusicPlayerCard
                  title={data.title || data.prompt || "AI 生成音乐"}
                  genre={data.genre}
                  bpm={data.bpm}
                  duration={data.duration || data.duration_secs}
                  audioUrl={data.audioUrl || data.audio_url || data.url || `file://${data.local_path}`}
                  autoPlay={data.auto_play ?? data.autoPlay ?? true}
                />
              );
            }
          } catch {}
        }

        if (match || String(children).includes("\n")) {
          return <CodeBlockView lang={match ? match[1] : "text"} code={codeString} />;
        }

        const rawCode = codeString.trim();
        if (isDirectoryPath(rawCode)) {
          return (
            <span className="inline-flex items-center gap-1.5 px-1.5 py-0.5 rounded bg-[var(--surface-3)]/60 text-[var(--ink-2)] font-mono text-[11.5px] border border-[var(--line)]/40 mx-0.5 select-text">
              <FolderIcon className="w-3.5 h-3.5" />
              <span>{rawCode}</span>
            </span>
          );
        }

        if (isFilePath(rawCode)) {
          const ext = rawCode.split(".").pop()?.toLowerCase() || "";
          return (
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                void useCodeViewer.getState().openFile(rawCode);
              }}
              className="inline-flex items-center gap-1.5 px-1.5 py-0.5 rounded bg-[var(--surface-3)] hover:bg-[var(--surface-2)] text-[var(--ink)] hover:text-emerald-400 font-mono text-[11.5px] border border-[var(--line)]/70 mx-0.5 cursor-pointer transition-colors shadow-2xs group"
              title={`在右侧代码查看器中打开 ${rawCode}`}
            >
              <FileIcon ext={ext} className="w-3.5 h-3.5" />
              <span className="underline decoration-dotted group-hover:decoration-solid">
                {rawCode}
              </span>
            </button>
          );
        }

        return (
          <code
            className="px-1.5 py-0.5 rounded bg-[var(--surface-3)]/80 text-[var(--ink)] font-mono text-[11.5px] border border-[var(--line)]/50 mx-0.5"
            {...props}
          >
            {children}
          </code>
        );
      },
    }),
    [],
  );

  if (!content) return null;

  return (
    <div
      className={`markdown-content flex flex-col gap-2 text-[13px] leading-[1.7] select-text text-[var(--ink)] ${className}`}
    >
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        urlTransform={markdownUrlTransform}
        components={components}
      >
        {renderedContent}
      </ReactMarkdown>
    </div>
  );
}

export const MarkdownRenderer = memo(
  MarkdownRendererImpl,
  (prev, next) => prev.content === next.content && prev.className === next.className,
);
