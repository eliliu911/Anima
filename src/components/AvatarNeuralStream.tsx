import { useEffect, useRef, useState } from "react";
import { useVoice, type EmotionState } from "../store/voice";
import { useFace } from "../store/face";
import { activeLayer, neuralStreamWanted } from "../avatar/neuralLayer";
import { avatarModelBytes } from "../api/voice";
import { mouth } from "../avatar/mouth";

interface Props {
  modelId?: string | null;
  emotion?: EmotionState;
  className?: string;
}

const WS_STREAM_URL = "ws://127.0.0.1:8765/ws/stream";

type VideoState = "idle" | "speaking" | "listening";

/**
 * 🎥 4D 神经流式写实数字人渲染器 (Neural Stream Avatar)
 *
 * ## 双模态多动作流架构设计
 * 1. **多状态动态视频流 (Multi-Clip Video State Machine)**：
 *    - 待机 (Idle)：播放待机循环（如 xiaoya_idle.mp4），微动、呼吸与自然眨眼；
 *    - 说话 (Speaking)：当 TTS 发声或口型激活时，无缝交叉淡入说话动作（如 xiaoya_speaking.mp4）；
 *    - 倾听 (Listening)：当用户在麦克风说话时，无缝切换倾听动作（如 xiaoya_listening.mp4）；
 *    - 双 <video> 缓冲池与透明度淡入淡出，消除黑屏与卡顿。
 * 2. **实时神经流 (Neural WebSocket Canvas Stream)**：
 *    - 当 Python 后端实时推理推送帧时，高优先级绘制在画布上，实现毫秒级音频换嘴；
 * 3. **单图写真肖像 (LivePortrait / Static Portrait)**：
 *    - 自适应比例居中呈现，配合说话时的光环与动态渲染。
 */
export function AvatarNeuralStream({ modelId, emotion: _emotion, className }: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const videoIdleRef = useRef<HTMLVideoElement>(null);
  const videoSpeakingRef = useRef<HTMLVideoElement>(null);
  const videoListeningRef = useRef<HTMLVideoElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const imgRef = useRef<HTMLImageElement | null>(null);
  const wsRef = useRef<WebSocket | null>(null);

  const voiceState = useVoice((s) => s.state);
  const micLevel = useVoice((s) => s.level);
  const outputLevel = useVoice((s) => s.outputLevel);
  const neuralConfig = useFace((s) => s.neuralConfig);

  const [loading, setLoading] = useState(true);
  const [error] = useState<string | null>(null);
  const [isVideo, setIsVideo] = useState(false);
  const [isMultiClip, setIsMultiClip] = useState(false);

  // 视频 Blob URLs
  const [idleBlobUrl, setIdleBlobUrl] = useState<string | null>(null);
  const [speakingBlobUrl, setSpeakingBlobUrl] = useState<string | null>(null);
  const [listeningBlobUrl, setListeningBlobUrl] = useState<string | null>(null);

  const [currentClipState, setCurrentClipState] = useState<VideoState>("idle");
  const [hasWsFrames, setHasWsFrames] = useState(false);
  // sidecar 自报的能力：能不能做**真口型**（wav2lip 重绘嘴部）。
  // 这一位决定画布该不该盖在三段体态视频上面 —— 见 avatar/neuralLayer.ts
  const [hasLipsync, setHasLipsync] = useState(false);
  const [isWsLive, setIsWsLive] = useState(false);

  const targetId = modelId || "neural/seeu.mp4";
  const cleanTarget = targetId.replace(/^neural\//, "");
  const baseName = cleanTarget.replace(/(_idle|_speaking|_listening)?\.(mp4|mov|webm|jpg|png|jpeg)$/i, "");
  const charDisplayName =
    baseName === "yumiko"
      ? "由美子"
      : baseName === "xiaoya"
      ? "小雅"
      : baseName === "ayami"
      ? "绚美"
      : baseName === "hanyu"
      ? "韩羽"
      : baseName === "ziling"
      ? "子苓"
      : baseName === "seeu"
      ? "小柔 SeeU"
      : baseName;

  // 谁在画面上、神经流要不要起 —— 规则和理由都在 avatar/neuralLayer.ts，
  // 那里有测试。**别把它改回内联三元** —— 上一版就是那么写错的，而且不报错
  const layerInput = { isMultiClip, isVideo, hasWsFrames, hasLipsync };
  const showCanvas = activeLayer(layerInput) === "canvas";
  const neuralOwnsScreen = neuralStreamWanted(layerInput);

  // 1. 自动检测并加载全量素材（单视频 / 多片段视频 / 单张肖像）
  useEffect(() => {
    let active = true;
    const blobUrlsToRevoke: string[] = [];

    async function loadAssets() {
      setLoading(true);
      setHasWsFrames(false);

      const isVideoAsset =
        targetId.toLowerCase().endsWith(".mp4") ||
        targetId.toLowerCase().endsWith(".mov") ||
        targetId.toLowerCase().endsWith(".webm");

      setIsVideo(isVideoAsset);

      // 提取角色基础标识（例如 neural/xiaoya_idle.mp4 -> xiaoya, neural/yumiko_idle.mp4 -> yumiko, neural/yumiko.jpg -> yumiko）
      const cleanTarget = targetId.replace(/^neural\//, "");
      const baseName = cleanTarget.replace(/(_idle|_speaking|_listening)?\.(mp4|mov|webm|jpg|png|jpeg)$/i, "");

      try {
        // 并行尝试探测并加载该角色的 3 段动作视频：待机、说话、倾听
        const [idleBytes, speakingBytes, listeningBytes] = await Promise.all([
          avatarModelBytes(`neural/${baseName}_idle.mp4`).catch(() => isVideoAsset ? avatarModelBytes(targetId).catch(() => null) : null),
          avatarModelBytes(`neural/${baseName}_speaking.mp4`).catch(() => null),
          avatarModelBytes(`neural/${baseName}_listening.mp4`).catch(() => null),
        ]);

        if (!active) return;

        const hasMulti = Boolean(idleBytes && (speakingBytes || listeningBytes));
        setIsMultiClip(hasMulti);

        if (idleBytes) {
          setIsVideo(true);
          const uIdle = URL.createObjectURL(new Blob([idleBytes], { type: "video/mp4" }));
          blobUrlsToRevoke.push(uIdle);
          setIdleBlobUrl(uIdle);

          if (speakingBytes) {
            const uSpeak = URL.createObjectURL(new Blob([speakingBytes], { type: "video/mp4" }));
            blobUrlsToRevoke.push(uSpeak);
            setSpeakingBlobUrl(uSpeak);
          } else {
            setSpeakingBlobUrl(uIdle);
          }

          if (listeningBytes) {
            const uListen = URL.createObjectURL(new Blob([listeningBytes], { type: "video/mp4" }));
            blobUrlsToRevoke.push(uListen);
            setListeningBlobUrl(uListen);
          } else {
            setListeningBlobUrl(uIdle);
          }
        } else {
          // 单视频或单图片素材
          const bytes = await avatarModelBytes(targetId);
          if (!active) return;

          const mime = isVideoAsset ? "video/mp4" : "image/jpeg";
          const u = URL.createObjectURL(new Blob([bytes], { type: mime }));
          blobUrlsToRevoke.push(u);
          setIdleBlobUrl(u);
          setSpeakingBlobUrl(null);
          setListeningBlobUrl(null);

          if (!isVideoAsset) {
            const img = new Image();
            img.src = u;
            img.onload = () => {
              if (!active) return;
              imgRef.current = img;
              setLoading(false);
            };
          }
        }

        if (active) setLoading(false);
      } catch (err) {
        if (active) setLoading(false);
      }
    }

    void loadAssets();

    return () => {
      active = false;
      blobUrlsToRevoke.forEach((u) => URL.revokeObjectURL(u));
    };
  }, [targetId]);

  // 2. 实时语音状态机驱动：根据对话状态（说话/倾听/待机）无缝流转
  useEffect(() => {
    if (!isMultiClip) {
      setCurrentClipState("idle");
      return;
    }

    const v = mouth.sample();
    const isSpeaking =
      voiceState === "speaking" ||
      outputLevel > 0.015 ||
      v.aa > 0.04 ||
      v.O > 0.04 ||
      v.E > 0.04;

    const isListening = voiceState === "listening" || (micLevel > 0.06 && !isSpeaking);

    if (isSpeaking && speakingBlobUrl) {
      setCurrentClipState("speaking");
    } else if (isListening && listeningBlobUrl) {
      setCurrentClipState("listening");
    } else {
      setCurrentClipState("idle");
    }
  }, [voiceState, outputLevel, micLevel, isMultiClip, speakingBlobUrl, listeningBlobUrl]);

  // 3. 视频播放器自动装载与静音循环
  useEffect(() => {
    const playSafe = (v: HTMLVideoElement | null, url: string | null) => {
      if (!v || !url) return;
      if (v.src !== url) {
        v.src = url;
        v.load();
      }
      const p = v.play();
      if (p) p.catch(() => {});
    };

    if (idleBlobUrl) playSafe(videoIdleRef.current, idleBlobUrl);
    if (speakingBlobUrl) playSafe(videoSpeakingRef.current, speakingBlobUrl);
    if (listeningBlobUrl) playSafe(videoListeningRef.current, listeningBlobUrl);
  }, [idleBlobUrl, speakingBlobUrl, listeningBlobUrl]);

  // 4. WebSocket 神经流连接与双向帧通信
  useEffect(() => {
    // **每换一个素材都要连一次去问。**
    //
    // 能不能做真口型是 sidecar 在 hello 里报的，不连上就不知道。
    // 早先这里按 neuralOwnsScreen 决定连不连，结果是：一旦判定「没有口型」
    // 就再也不连，于是也永远学不到它后来有了 —— 换素材、换权重都救不回来。
    //
    // 所以改成先连，问到没有口型、而本地又有三段体态素材时再主动断开
    // （见下面 hello 的处理）。断开仍然是必要的，不只是省资源：
    // 连着的话每秒 30 帧 JPEG 要解成 ImageBitmap 再画进一个 opacity-0 的
    // 画布，实测白烧 1.1MB/s 带宽和一条 GPU 解码路径。
    let unmounted = false;
    let ws: WebSocket | null = null;
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
    // 我们主动断的（问到没口型），区别于连接掉线
    let declined = false;

    function connectWs() {
      if (unmounted) return;
      try {
        ws = new WebSocket(WS_STREAM_URL);
        ws.binaryType = "blob";
        wsRef.current = ws;

        ws.onopen = () => {
          if (unmounted) return;
          setIsWsLive(true);
          ws?.send(
            JSON.stringify({
              type: "load",
              model_id: targetId,
            })
          );
        };

        ws.onmessage = async (event) => {
          if (unmounted) return;
          if (typeof event.data === "string") {
            try {
              const msg = JSON.parse(event.data);
              if (msg.type === "hello") {
                const lip = !!msg.lipsync;
                setHasLipsync(lip);
                // 问到了：没有真口型、而本地有三段体态素材 —— 让位并断开。
                // 盖上去只会让 speaking / listening 永远不出现
                if (!lip && isMultiClip) {
                  declined = true;
                  ws?.close();
                  return;
                }
              }
            } catch {
              // sidecar 发来的不是 JSON 就忽略 —— 帧走二进制那条路
            }
            return;
          }
          if (event.data instanceof Blob) {
            try {
              const bitmap = await createImageBitmap(event.data);
              const canvas = canvasRef.current;
              if (canvas) {
                const ctx = canvas.getContext("2d");
                if (ctx) {
                  if (canvas.width !== bitmap.width || canvas.height !== bitmap.height) {
                    canvas.width = bitmap.width;
                    canvas.height = bitmap.height;
                  }
                  ctx.clearRect(0, 0, canvas.width, canvas.height);
                  ctx.drawImage(bitmap, 0, 0);
                  setHasWsFrames(true);
                }
              }
              bitmap.close();
            } catch {}
          }
        };

        ws.onclose = () => {
          if (unmounted) return;
          setIsWsLive(false);
          setHasWsFrames(false);
          // 是我们自己因为「没口型」断开的，就别再连回去问同一个答案
          if (declined) return;
          // **能力要作废。** 留着的话换到没有口型的素材后，
          // 画布仍然会盖住三段体态，又变回「永远只播待机」
          setHasLipsync(false);
          reconnectTimer = setTimeout(connectWs, 3000);
        };

        ws.onerror = () => {
          if (unmounted) return;
          setIsWsLive(false);
          setHasWsFrames(false);
        };
      } catch {
        setIsWsLive(false);
        setHasWsFrames(false);
        reconnectTimer = setTimeout(connectWs, 3000);
      }
    }

    connectWs();

    return () => {
      unmounted = true;
      if (reconnectTimer) clearTimeout(reconnectTimer);
      if (ws) {
        try {
          ws.close();
        } catch {}
      }
    };
  }, [targetId, isMultiClip]);

  // 输出电平要在 rAF 里读到最新值，但**不能当依赖** ——
  // 它每来一帧音频就变一次，当依赖等于每秒重建几十次 rAF 循环
  const outputLevelRef = useRef(outputLevel);
  outputLevelRef.current = outputLevel;

  // 5. 口型高频采样与流式推送 (60FPS)
  //
  // **只在退回形变引擎时才发。** 有 wav2lip 时驱动数据是 Rust 直送的整句音频
  // （梅尔频谱），前端这几个 viseme 标量对它毫无用处 —— 白发 60 次/秒
  useEffect(() => {
    if (!neuralOwnsScreen || hasLipsync) return;

    let animId: number;
    const sendVisemes = () => {
      const ws = wsRef.current;
      if (ws && ws.readyState === WebSocket.OPEN) {
        const v = mouth.sample();
        const maxVowel = Math.max(v.aa, v.O, v.E, v.I, v.U, outputLevelRef.current * 1.5);
        const form = v.I * 0.8 + v.E * 0.5 - (v.U * 0.6 + v.O * 0.7);
        ws.send(
          JSON.stringify({
            type: "viseme",
            is_speaking: maxVowel > 0.02,
            mouth_open: maxVowel,
            mouth_form: form,
            aa: v.aa,
            O: v.O,
            E: v.E,
            I: v.I,
            U: v.U,
          })
        );
      }
      animId = requestAnimationFrame(sendVisemes);
    };

    animId = requestAnimationFrame(sendVisemes);
    return () => cancelAnimationFrame(animId);
  }, [neuralOwnsScreen, hasLipsync]);

  // 6. 单图写真模式平滑渲染
  useEffect(() => {
    if (isVideo || hasWsFrames) return;

    let animId: number;
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const render = () => {
      const container = containerRef.current;
      if (container && canvas) {
        const rect = container.getBoundingClientRect();
        const dpr = Math.min(2, window.devicePixelRatio || 1);
        const tw = Math.max(360, Math.round(rect.width * dpr));
        const th = Math.max(480, Math.round(rect.height * dpr));
        if (canvas.width !== tw || canvas.height !== th) {
          canvas.width = tw;
          canvas.height = th;
        }
      }

      const img = imgRef.current;
      if (img && img.naturalWidth > 0) {
        const cw = canvas.width;
        const ch = canvas.height;
        const iw = img.naturalWidth;
        const ih = img.naturalHeight;

        const scale = Math.max(cw / iw, ch / ih);
        const dw = iw * scale;
        const dh = ih * scale;
        const dx = (cw - dw) / 2;
        const dy = (ch - dh) / 2;

        ctx.clearRect(0, 0, cw, ch);
        ctx.drawImage(img, dx, dy, dw, dh);
      }

      animId = requestAnimationFrame(render);
    };

    animId = requestAnimationFrame(render);
    return () => cancelAnimationFrame(animId);
  }, [isVideo, hasWsFrames]);

  const fitMode = neuralConfig?.fitMode || "cover";

  return (
    <div
      ref={containerRef}
      className={`relative w-full h-full flex items-center justify-center overflow-hidden bg-transparent ${
        className || ""
      }`}
    >
      {/* 1. 神经推理画布 (高优先级：实时换嘴与单图驱动) */}
      <canvas
        ref={canvasRef}
        className={`absolute inset-0 w-full h-full pointer-events-none drop-shadow-2xl transition-opacity duration-200 ${
          showCanvas ? "opacity-100 z-10" : "opacity-0 pointer-events-none"
        } ${fitMode === "cover" ? "object-cover" : "object-contain"}`}
      />

      {/* 2. 视频底模渲染池 (支持待机/说话/倾听三态平滑交叉淡入) */}
      {isVideo && (
        <div className="absolute inset-0 w-full h-full overflow-hidden pointer-events-none">
          {/* 待机循环视频 */}
          {idleBlobUrl && (
            <video
              ref={videoIdleRef}
              src={idleBlobUrl}
              autoPlay
              playsInline
              muted
              loop
              onLoadedData={(e) => {
                const p = e.currentTarget.play();
                if (p) p.catch(() => {});
              }}
              className={`absolute inset-0 w-full h-full transition-opacity duration-300 ${
                currentClipState === "idle" ? "opacity-100" : "opacity-0"
              } ${fitMode === "cover" ? "object-cover" : "object-contain"}`}
            />
          )}

          {/* 说话动作视频 */}
          {speakingBlobUrl && (
            <video
              ref={videoSpeakingRef}
              src={speakingBlobUrl}
              autoPlay
              playsInline
              muted
              loop
              onLoadedData={(e) => {
                const p = e.currentTarget.play();
                if (p) p.catch(() => {});
              }}
              className={`absolute inset-0 w-full h-full transition-opacity duration-300 ${
                currentClipState === "speaking" ? "opacity-100" : "opacity-0"
              } ${fitMode === "cover" ? "object-cover" : "object-contain"}`}
            />
          )}

          {/* 倾听动作视频 */}
          {listeningBlobUrl && (
            <video
              ref={videoListeningRef}
              src={listeningBlobUrl}
              autoPlay
              playsInline
              muted
              loop
              onLoadedData={(e) => {
                const p = e.currentTarget.play();
                if (p) p.catch(() => {});
              }}
              className={`absolute inset-0 w-full h-full transition-opacity duration-300 ${
                currentClipState === "listening" ? "opacity-100" : "opacity-0"
              } ${fitMode === "cover" ? "object-cover" : "object-contain"}`}
            />
          )}
        </div>
      )}

      {/* 加载中状态 */}
      {loading && (
        <div className="absolute inset-0 flex flex-col items-center justify-center bg-black/40 backdrop-blur-xs text-white gap-2 z-30">
          <div className="w-8 h-8 rounded-full border-2 border-white/20 border-t-[var(--celadon)] animate-spin" />
          <span className="text-[12px] font-medium tracking-wide">
            正在载入高清动态底模...
          </span>
        </div>
      )}

      {/* 错误状态 */}
      {error && !loading && (
        <div className="absolute bottom-10 px-4 py-2 bg-red-900/80 backdrop-blur-md rounded-xl text-white text-[12px] border border-red-500/40 z-30">
          ⚠️ {error}
        </div>
      )}

      {/* 神经流实时状态胶囊 (居中置顶) */}
      <div className="absolute top-4 left-1/2 -translate-x-1/2 z-20 flex items-center gap-2 bg-black/80 backdrop-blur-md px-3.5 py-1.5 rounded-full border border-white/20 text-white text-[11px] shadow-xl whitespace-nowrap pointer-events-none">
        <span
          className={`w-2 h-2 rounded-full ${
            hasWsFrames || isWsLive ? "bg-emerald-400 animate-pulse" : "bg-emerald-400"
          }`}
        />
        <span className="font-semibold tracking-wider">
          {isMultiClip
            ? `${charDisplayName} (${currentClipState === "speaking" ? "🗣️ 说话动作" : currentClipState === "listening" ? "👂 倾听动作" : "✨ 待机动作"})`
            : hasWsFrames
            ? "LiveTalking 实时神经换嘴"
            : isVideo
            ? `${charDisplayName} · 4D 动态视频`
            : `${charDisplayName} · 神经写真肖像`}
        </span>
        <span className="text-white/60 font-mono text-[10px]">
          | {isVideo ? (isMultiClip ? "4D 动作流" : "60 FPS") : "30 FPS"}
        </span>
      </div>
    </div>
  );
}
