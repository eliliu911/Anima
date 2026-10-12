/**
 * 本地图片 / 视频的显示组件。
 *
 * # 为什么不用 `convertFileSrc`
 *
 * Tauri 自带的 `asset://` 协议在这个应用里**显示不出来**（用户实测：
 * 资产库和选择器里全是碎图）。这不是小事 —— 「路径显示不了」正是当初
 * 有人改成把 base64 存进 JSON 的原因，结果是 86 MB 的 `characters.json`。
 *
 * # 两条路，先快后稳
 *
 * 1. `anima-media://` —— 我们自己注册的协议。走浏览器缓存、不占 IPC、
 *    大图也不用编码成 base64。
 * 2. 它要是也加载失败，退回 `assets_read_media` 取 data URL。慢、占内存，
 *    但一定能显示。
 *
 * **退化过一次就记住**（模块级变量），后面的缩略图直接走兜底 ——
 * 否则一屏几十张图会各自先失败一次，白闪一遍。
 * 同时在控制台留一条，让「协议没工作」这件事是**可见的**，而不是
 * 悄悄退化成慢路径。
 */
import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

/**
 * 本机绝对路径 → 自定义协议 URL。**必须和 Rust 侧 `media_url` 一致。**
 *
 * `version` 只是给浏览器看的缓存击穿参数，协议那边只读 `p`，会忽略它。
 *
 * # 为什么光有 ETag 不够
 *
 * 分镜首帧固定写 `shots/<镜头 id>.png`。重出一次内容换了、路径没变 ——
 * `<img src>` 那个字符串一模一样，**React 不会重设 src，浏览器也就不会
 * 发请求**，ETag 那层根本轮不到。表现是要切一次页面才看到新图。
 * URL 本身必须变。
 */
export function mediaSrc(absPath: string, version?: string | number): string {
  const base = `anima-media://localhost/?p=${encodeURIComponent(absPath)}`;
  return version ? `${base}&v=${encodeURIComponent(String(version))}` : base;
}

/**
 * 反过来：`anima-media://` URL → 本机绝对路径。
 *
 * 后端下发的是 URL（`cover_url` / `image_urls`），兜底通道要的是路径 ——
 * 两种输入都得能接，否则调用方要自己分情况，迟早漏一处。
 */
export function pathFromMediaUrl(input: string): string {
  if (!input.startsWith("anima-media:")) return input;
  const q = input.split("?")[1];
  if (!q) return input;
  for (const pair of q.split("&")) {
    if (pair.startsWith("p=")) return decodeURIComponent(pair.slice(2));
  }
  return input;
}

let protocolBroken = false;

async function readAsDataUrl(path: string): Promise<string> {
  return invoke<string>("assets_read_media", { path });
}

/**
 * 已经是可以直接喂给 `<img>` 的东西吗（`data:` / `http(s):` / `blob:`）。
 *
 * **老分集里的 `image_url` 存的是 base64 data URL。** 不认出来的话，
 * 它会被当成路径包进 `anima-media://?p=data:image/...`，协议去按路径读，
 * 必然失败 —— 表现是老镜头一律显示不出来。
 */
function isInlineSrc(v: string): boolean {
  // `tauri:`：打包后应用自带的静态文件（public/ 下的，如运镜示例）—— 页面源是 tauri://localhost，不是 http
  return v.startsWith("data:") || v.startsWith("blob:") || v.startsWith("http:") || v.startsWith("https:") || v.startsWith("tauri:");
}

/**
 * 本机路径 → **data URL**，给 WebGL 贴图用（three.js 的 `TextureLoader`）。
 *
 * # 为什么贴图不走 `anima-media://`
 *
 * 那个协议的响应**不带 `Access-Control-Allow-Origin`**。`<img>` 显示无所谓，
 * 但贴图不一样：
 *
 * 1. three 的加载器默认带 `crossOrigin`，跨域图要么加载失败，要么**污染 canvas**，
 *    之后节点里「截这一帧」调 `toDataURL` 直接抛 `SecurityError`。
 * 2. 第一版这里先 `fetch` 探一下协议通不通，探失败就把模块级的
 *    `protocolBroken` 置真 —— 而那个标记是**全会话共用的**，一个全景节点探失败，
 *    资产库里所有缩略图都跟着降级到慢路。
 *
 * data URL 是同源的，两件事都没有。代价是占内存：`assets_read_media` 上限 24 MB，
 * 4096×2048 的全景 PNG 大约 8–12 MB，放得下。
 *
 * **不要在这里用 `convertFileSrc`** —— Tauri 的 `asset://` 在这个应用里
 * 显示不出来，理由见本文件开头。
 */
export async function mediaDataUrl(input: string): Promise<string> {
  if (input.startsWith("data:") || input.startsWith("blob:")) return input;
  return readAsDataUrl(pathFromMediaUrl(input));
}

function useResolvedSrc(input: string | null | undefined, version?: string | number) {
  const inline = !!input && isInlineSrc(input);
  const path = input && !inline ? pathFromMediaUrl(input) : input;
  const [src, setSrc] = useState<string | null>(null);
  const [failed, setFailed] = useState<string | null>(null);

  useEffect(() => {
    if (!path) {
      setSrc(null);
      return;
    }
    setFailed(null);
    if (inline) {
      setSrc(path);
      return;
    }
    if (!protocolBroken) {
      setSrc(mediaSrc(path, version));
      return;
    }
    let alive = true;
    readAsDataUrl(path)
      .then((d) => alive && setSrc(d))
      .catch((e) => alive && setFailed(String(e)));
    return () => {
      alive = false;
    };
  }, [path, inline, version]);

  /** 协议路加载失败 → 记下来并改走兜底。 */
  const onError = () => {
    if (!path || inline) return;
    if (!protocolBroken) {
      protocolBroken = true;
      console.warn(
        "[media] anima-media:// 加载失败，本次会话改走 assets_read_media 兜底（慢，占内存）",
      );
    }
    readAsDataUrl(path)
      .then(setSrc)
      .catch((e) => setFailed(String(e)));
  };

  return { src, failed, onError };
}

/**
 * 循环预览视频。`hoverPlay`：鼠标停在上面才播、移开停下回到开头。
 *
 * **用原生 mouseenter / mouseleave，不用 React 的 onMouseEnter。** 这些卡片在 portal（挂在 body 下的窗口）里，
 * React 合成的 enter / leave 在那里收不到（2026-09-25 在调试台里查实：处理函数绑上了、直接 play() 能播，
 * 鼠标移上去就是不触发）。
 */
function AmbientVideo({ src, className, hoverPlay, onError }: { src: string; className: string; hoverPlay: boolean; onError: () => void }) {
  const ref = useRef<HTMLVideoElement>(null);
  useEffect(() => {
    const v = ref.current;
    if (!v || !hoverPlay) return;
    // Chromium 内核里没播过、也没跳过的视频**第 0 帧不画**，卡片是一块黑（readyState 已经是 4）。
    // 往后跳一丁点就画出来了；移开时也回到这里，不回 0
    const FIRST = 0.05;
    const paint = () => { if (v.paused) v.currentTime = FIRST; };
    const enter = () => { void v.play().catch(() => {}); };
    const leave = () => { v.pause(); v.currentTime = FIRST; };
    if (v.readyState >= 2) paint();
    v.addEventListener("loadeddata", paint);
    v.addEventListener("mouseenter", enter);
    v.addEventListener("mouseleave", leave);
    return () => {
      v.removeEventListener("loadeddata", paint);
      v.removeEventListener("mouseenter", enter);
      v.removeEventListener("mouseleave", leave);
    };
  }, [hoverPlay]);
  return (
    <video ref={ref} src={src} className={className} muted loop playsInline onError={onError}
      autoPlay={!hoverPlay}
      // 停在第一帧给人看：hoverPlay 时要把画面先解出来（metadata 在有的内核里只给黑帧）
      preload={hoverPlay ? "auto" : "metadata"} />
  );
}

export function MediaThumb({
  path,
  alt,
  className,
  kind = "image",
  version,
  ambient = false,
  hoverPlay = false,
}: {
  /** 本机绝对路径，或后端下发的 `anima-media://` URL */
  path: string | null | undefined;
  alt?: string;
  className?: string;
  kind?: "image" | "video";
  /**
   * 这个文件当前这一版（一般是 mtime）。**路径会被覆写的地方必须传** ——
   * 不传的话内容换了 URL 没变，界面会一直显示旧的。
   */
  version?: string | number;
  /**
   * 视频当**循环预览**放：静音、自动播放、循环、不带控件（运镜窗口的示例短片）。
   * 防拖拽的类名照样加 —— 这个组件存在的意义就是那一条不靠调用方记得。
   */
  ambient?: boolean;
  /**
   * 配合 `ambient`：**鼠标停在上面才播**，移开停下并回到开头（用户 2026-09-25：「运镜需要用鼠标停留激活播放」）。
   * 一屏 21 段一起自动播又吵又费电，而且看不出哪一段是自己在看的。
   */
  hoverPlay?: boolean;
}) {
  const { src, failed, onError } = useResolvedSrc(path, version);

  if (failed) {
    // **失败原因要看得见。** 碎图图标会被当成「文件没了」，
    // 而真实原因常常是路径不在允许目录内 —— 那是可操作的信息。
    return (
      <span
        className={`flex items-center justify-center px-2 text-center text-[10px] ${className ?? ""}`}
        style={{ color: "var(--studio-text-muted)" }}
        title={failed}
      >
        读不出来
      </span>
    );
  }
  if (!src) {
    return <span className={className} style={{ background: "var(--studio-bg-inset)" }} />;
  }
  if (kind === "video") {
    // **`nodrag nopan` 无条件加在这里，不靠调用方记得。**
    //
    // 画布节点里的 `<video controls>` 一旦被 ReactFlow 的拖拽处理器抢走指针，
    // 拖进度条就会变成「滑块永远跟着鼠标、节点切不了、菜单点不动，只能强杀进程」
    // —— 用户 2026-09-22 在深度动作捕捉节点上撞到。
    //
    // 整条画布里 `<video>` 只有这一处。`CanvasMediaGallery` 当时是自己在
    // className 里补了 `nodrag` 才没事，而 `DepthCaptureNode` / `UpscaleNode`
    // 没补 —— 这就是「一个每次都要记得的约定等于没有约定」。
    // 类名在 ReactFlow 之外是惰性的，工作区里用同一个组件不受影响。
    return ambient ? (
      <AmbientVideo src={src} className={`nodrag nopan ${className ?? ""}`} hoverPlay={hoverPlay} onError={onError} />
    ) : (
      <video
        src={src}
        className={`nodrag nopan ${className ?? ""}`}
        controls
        playsInline
        preload="metadata"
        onError={onError}
      />
    );
  }
  return <img src={src} alt={alt ?? ""} className={className} loading="lazy" onError={onError} />;
}
