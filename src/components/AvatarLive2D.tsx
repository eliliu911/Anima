/**
 * Live2D 面容渲染组件。
 *
 * ## 架构设计
 *
 * 与 Avatar3D 平级的具象 2D 纸片人形态组件：
 * - 采用 PIXI.js + pixi-live2d-display (Cubism 2/3/4/5 支持)；
 * - 内存 File 数组直读，0 网络请求，秒开加载；
 * - 基于原始网格尺寸 (originalWidth/Height) 实现精确无损的实时缩放与平移变换；
 * - 实时对接 Anima 的全局 Gaze 视线解算器（ParamAngleX/Y/Z、ParamEyeBallX/Y）；
 * - 实时对接 Anima 的 Mouth 时间线与 Viseme 权重（ParamMouthOpenY、ParamMouthForm），并在 beforeModelUpdate 钩子中精准注入，杜绝被动作覆盖；
 * - 响应式动作（Motion）与情绪（Expression）触发。
 */
import { useEffect, useRef, useState } from "react";
import * as PIXI from "pixi.js";
import { Live2DModel } from "pixi-live2d-display";
import { avatarModelBytes, logUi } from "../api/voice";
import { mouth } from "../avatar/mouth";
import { Gaze, targetFromPointer } from "../avatar/gaze";
import { desktopCursor, hasDesktopCursor } from "../avatar/desktopCursor";
import type { EmotionState } from "../store/voice";
import { useFace, type Live2DConfig } from "../store/face";

// 确保使用 WebGL2 优先
PIXI.settings.PREFER_ENV = PIXI.ENV.WEBGL2;

// 注册全局 PIXI 供 Live2D 插件识别
if (typeof window !== "undefined") {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  (window as any).PIXI = PIXI;
  try {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    Live2DModel.registerTicker(PIXI.Ticker as any);
  } catch {}
}

interface Props {
  modelId: string | null;
  emotion?: EmotionState;
  action?: string;
  actionSeq?: number;
  className?: string;
}

function getMimeType(path: string): string {
  const lower = path.toLowerCase();
  if (lower.endsWith(".png")) return "image/png";
  if (lower.endsWith(".jpg") || lower.endsWith(".jpeg")) return "image/jpeg";
  if (lower.endsWith(".json")) return "application/json";
  if (lower.endsWith(".moc3") || lower.endsWith(".moc")) return "application/octet-stream";
  return "application/octet-stream";
}

function createFile(bytes: ArrayBuffer | Uint8Array, relPath: string): File {
  const name = relPath.includes("/") ? relPath.slice(relPath.lastIndexOf("/") + 1) : relPath;
  const file = new File([bytes], name, { type: getMimeType(relPath) });
  Object.defineProperty(file, "webkitRelativePath", {
    value: relPath,
    writable: false,
  });
  return file;
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function collectRelativeFiles(json: any): string[] {
  const paths = new Set<string>();

  // Cubism 3/4/5 标准
  const fr = json.FileReferences || {};
  if (fr.Moc && typeof fr.Moc === "string") paths.add(fr.Moc);
  if (Array.isArray(fr.Textures)) {
    for (const t of fr.Textures) if (typeof t === "string") paths.add(t);
  }
  if (fr.Physics && typeof fr.Physics === "string") paths.add(fr.Physics);
  if (fr.Pose && typeof fr.Pose === "string") paths.add(fr.Pose);
  if (fr.UserData && typeof fr.UserData === "string") paths.add(fr.UserData);
  if (fr.DisplayInfo && typeof fr.DisplayInfo === "string") paths.add(fr.DisplayInfo);
  if (fr.Motions && typeof fr.Motions === "object") {
    for (const list of Object.values(fr.Motions)) {
      if (Array.isArray(list)) {
        for (const m of list) {
          // eslint-disable-next-line @typescript-eslint/no-explicit-any
          if (m && typeof (m as any).File === "string") paths.add((m as any).File);
        }
      }
    }
  }
  if (Array.isArray(fr.Expressions)) {
    for (const exp of fr.Expressions) {
      if (exp && typeof exp.File === "string") paths.add(exp.File);
    }
  }

  // Cubism 2.1 兼容
  if (json.model && typeof json.model === "string") paths.add(json.model);
  if (Array.isArray(json.textures)) {
    for (const t of json.textures) if (typeof t === "string") paths.add(t);
  }
  if (json.physics && typeof json.physics === "string") paths.add(json.physics);
  if (json.pose && typeof json.pose === "string") paths.add(json.pose);
  if (json.motions && typeof json.motions === "object") {
    for (const list of Object.values(json.motions)) {
      if (Array.isArray(list)) {
        for (const m of list) {
          // eslint-disable-next-line @typescript-eslint/no-explicit-any
          if (m && typeof (m as any).file === "string") paths.add((m as any).file);
        }
      }
    }
  }
  if (Array.isArray(json.expressions)) {
    for (const exp of json.expressions) {
      if (exp && typeof exp.file === "string") paths.add(exp.file);
    }
  }

  return Array.from(paths);
}

async function ensureCubismCore(): Promise<void> {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const w = window as any;
  const loadScript = (src: string): Promise<void> => {
    return new Promise((resolve) => {
      const existing = document.querySelector(`script[src="${src}"]`);
      if (existing) {
        resolve();
        return;
      }
      const script = document.createElement("script");
      script.src = src;
      script.onload = () => resolve();
      script.onerror = () => resolve();
      document.head.appendChild(script);
    });
  };

  const tasks: Promise<void>[] = [];
  if (!w.Live2D) tasks.push(loadScript("/live2d/live2d.min.js"));
  if (!w.Live2DCubismCore) tasks.push(loadScript("/live2d/live2dcubismcore.min.js"));
  if (tasks.length > 0) await Promise.all(tasks);
}

export function AvatarLive2D({
  modelId,
  emotion,
  action,
  actionSeq,
  className = "w-full h-full",
}: Props) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [error, setError] = useState<string | null>(null);
  const modelRef = useRef<Live2DModel | null>(null);
  const appRef = useRef<PIXI.Application | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const modelSizeRef = useRef<{ w: number; h: number }>({ w: 0, h: 0 });
  const live2dConfig = useFace((s) => s.live2dConfig);

  // 核心构图与自适应缩放（基于不变的原始宽高计算，保证无论 scale 如何变动都能精确缩放）
  const applyTransform = (cfg: Live2DConfig) => {
    const model = modelRef.current;
    const app = appRef.current;
    const container = containerRef.current;
    if (!model || !app || !container) return;

    const cw = container.clientWidth || 360;
    const ch = container.clientHeight || 420;
    app.renderer.resize(cw, ch);

    // 取原始不变尺寸，杜绝使用 model.width 导致缩放递归抵消的 bug
    const origW = modelSizeRef.current.w || (model as any).internalModel?.originalWidth || (model as any).internalModel?.width || 1000;
    const origH = modelSizeRef.current.h || (model as any).internalModel?.originalHeight || (model as any).internalModel?.height || 1000;

    const scaleX = cw / origW;
    const scaleY = ch / origH;

    let baseScale = Math.min(scaleX, scaleY);
    let baseAnchorY = 0.5;
    let basePosY = ch * 0.5;

    if (cfg.focusPreset === "half") {
      // 半身特写：聚焦面部与上半身
      baseScale = Math.min(scaleX, scaleY) * 1.6;
      baseAnchorY = 0.22;
      basePosY = ch * 0.22;
    } else if (cfg.focusPreset === "full") {
      // 全身展示
      baseScale = Math.min(scaleX, scaleY) * 0.92;
      baseAnchorY = 0.5;
      basePosY = ch * 0.5;
    } else {
      // 自由微调
      baseScale = Math.min(scaleX, scaleY) * 1.25;
      baseAnchorY = 0.3;
      basePosY = ch * 0.35;
    }

    const finalScale = baseScale * (cfg.scale || 1.0);
    model.scale.set(finalScale);
    model.anchor.set(0.5, baseAnchorY);
    model.x = cw / 2 + ((cfg.offsetX || 0) / 100) * cw;
    model.y = basePosY + ((cfg.offsetY || 0) / 100) * ch;

    // 桌面柔和立体阴影
    if (canvasRef.current) {
      canvasRef.current.style.filter = cfg.dropShadow
        ? "drop-shadow(0 12px 28px rgba(0, 0, 0, 0.45))"
        : "none";
    }

    // 动画速度与帧率
    if (app.ticker) {
      app.ticker.speed = cfg.motionSpeed || 1.0;
      app.ticker.maxFPS = cfg.fpsLimit || 0;
    }
  };

  // 1. 初始化 PIXI 应用与加载 Live2D 模型
  useEffect(() => {
    let canceled = false;

    let gazeTarget: { x: number; y: number } | null = null;
    const gazeSolver = new Gaze();
    let lastTime = performance.now();

    const onPointerMove = (e: PointerEvent) => {
      if (hasDesktopCursor() || !containerRef.current) return;
      const r = containerRef.current.getBoundingClientRect();
      gazeTarget = targetFromPointer(e.clientX, e.clientY, r);
    };
    const onPointerLeave = () => {
      if (hasDesktopCursor()) return;
      gazeTarget = null;
    };
    window.addEventListener("pointermove", onPointerMove);
    window.addEventListener("pointerleave", onPointerLeave);

    const readGaze = () => {
      if (!hasDesktopCursor()) return gazeTarget;
      const c = desktopCursor();
      if (!c || !containerRef.current) return null;
      const r = containerRef.current.getBoundingClientRect();
      return targetFromPointer(c.x + r.left, c.y + r.top, r);
    };

    let canvasElement: HTMLCanvasElement | null = null;

    async function init() {
      if (!containerRef.current) return;
      setError(null);

      // 确保 Live2D 运行库已载入
      await ensureCubismCore();
      if (canceled) return;

      // 如果未指定模型或当前选中的是 3D 模型，默认选择 Hiyori
      const isLive2d = modelId && (modelId.toLowerCase().endsWith(".model3.json") || modelId.toLowerCase().endsWith(".model.json"));
      const targetModelId = isLive2d ? modelId : "live2d/Hiyori/Hiyori.model3.json";

      try {
        const width = containerRef.current.clientWidth || 360;
        const height = containerRef.current.clientHeight || 420;

        // 创建专属全新的 PIXI Application（让 PIXI 自动生成 canvas）
        const app = new PIXI.Application({
          width,
          height,
          backgroundAlpha: 0,
          autoDensity: true,
          resolution: window.devicePixelRatio || 1,
          antialias: true,
          powerPreference: "high-performance",
        });

        if (canceled) {
          app.destroy(true, { children: true, texture: true, baseTexture: true });
          return;
        }

        appRef.current = app;
        canvasElement = app.view as HTMLCanvasElement;
        canvasRef.current = canvasElement;
        canvasElement.className = "w-full h-full object-contain pointer-events-none transition-all duration-150";
        containerRef.current.appendChild(canvasElement);

        // 读出 model3.json 及其关联所有资产，构建内存 File 数组
        const manifestBytes = await avatarModelBytes(targetModelId);
        const manifestText = new TextDecoder().decode(manifestBytes);
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const json: any = JSON.parse(manifestText);

        const dir = targetModelId.includes("/")
          ? targetModelId.slice(0, targetModelId.lastIndexOf("/") + 1)
          : "";

        const manifestFilename = targetModelId.includes("/")
          ? targetModelId.slice(targetModelId.lastIndexOf("/") + 1)
          : targetModelId;

        const files: File[] = [createFile(manifestBytes, manifestFilename)];
        const relPaths = collectRelativeFiles(json);

        for (const rel of relPaths) {
          try {
            const bytes = await avatarModelBytes(dir + rel);
            files.push(createFile(bytes, rel));
          } catch (e) {
            logUi("warn", `[Live2D] 附属文件跳过 ${dir + rel}: ${e}`);
          }
        }

        if (canceled) return;

        // 利用 FileLoader 直接从内存 File 数组解析模型（0 网络请求）
        const model = await Live2DModel.from(files, {
          autoInteract: false,
        });

        if (canceled) {
          model.destroy({ children: true });
          return;
        }

        modelRef.current = model;

        // 记录原始不变尺寸
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const origW = (model as any).internalModel?.originalWidth || (model as any).internalModel?.width || model.width || 1000;
        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const origH = (model as any).internalModel?.originalHeight || (model as any).internalModel?.height || model.height || 1000;
        modelSizeRef.current = { w: origW, h: origH };

        // 应用初次构图与缩放
        applyTransform(useFace.getState().live2dConfig);
        app.stage.addChild(model as unknown as PIXI.DisplayObject);

        // 窗口尺寸自适应监听
        const resizeObserver = new ResizeObserver(() => {
          applyTransform(useFace.getState().live2dConfig);
        });
        resizeObserver.observe(containerRef.current);

        // 核心参数注入循环（支持 Cubism 2 / 3 / 4 / 5）
        const updateParams = () => {
          if (!model || canceled) return;

          const cfg = useFace.getState().live2dConfig;
          const now = performance.now();
          const dt = Math.min((now - lastTime) / 1000, 0.1);
          lastTime = now;

          // eslint-disable-next-line @typescript-eslint/no-explicit-any
          const internal = (model as any).internalModel;
          const core = internal?.coreModel;
          if (!core) return;

          const setParam = (id4: string, id2: string, val: number) => {
            try {
              if (typeof core.setParameterValueById === "function") {
                core.setParameterValueById(id4, val);
              }
            } catch {}
            try {
              if (typeof core.setParamFloat === "function") {
                core.setParamFloat(id2, val);
              }
            } catch {}
          };

          // 1. 视线解算与头部旋转
          if (cfg.mouseTracking) {
            const target = readGaze();
            const g = gazeSolver.update(target, dt);
            const mult = cfg.gazeMultiplier || 1.0;

            // Live2D 头部旋转 (-30 ~ 30)
            setParam("ParamAngleX", "PARAM_ANGLE_X", g.headYaw * 30 * mult);
            setParam("ParamAngleY", "PARAM_ANGLE_Y", g.headPitch * 30 * mult);
            setParam("ParamAngleZ", "PARAM_ANGLE_Z", -g.headYaw * 10 * mult);
            // 眼球注视 (-1 ~ 1)
            setParam("ParamEyeBallX", "PARAM_EYE_BALL_X", g.eyeYaw * mult);
            setParam("ParamEyeBallY", "PARAM_EYE_BALL_Y", g.eyePitch * mult);
            // 身体倾斜 (-10 ~ 10)
            setParam("ParamBodyAngleX", "PARAM_BODY_ANGLE_X", g.headYaw * 10 * mult);
          }

          // 2. 嘴型与能量
          const weights = mouth.sample();
          // 张嘴幅度：五元音 + 辅音的开合权重之和
          const openAmount =
            weights.aa * 1.0 +
            weights.O * 0.85 +
            weights.U * 0.65 +
            weights.E * 0.7 +
            weights.I * 0.45 +
            (weights.CH + weights.SS + weights.TH + weights.DD + weights.kk) * 0.35 +
            (1 - weights.sil) * 0.25;

          const mouthOpen = Math.min(1.0, Math.max(0.0, openAmount));
          const mouthForm = Math.min(
            1.0,
            Math.max(-1.0, weights.I * 0.8 + weights.E * 0.5 - (weights.U * 0.6 + weights.O * 0.7))
          );

          setParam("ParamMouthOpenY", "PARAM_MOUTH_OPEN_Y", mouthOpen);
          setParam("ParamMouthForm", "PARAM_MOUTH_FORM", mouthForm);
        };

        // eslint-disable-next-line @typescript-eslint/no-explicit-any
        const internal = (model as any).internalModel;
        if (internal && typeof internal.on === "function") {
          // beforeModelUpdate 是在动作/待机计算之后、顶点绘制之前触发，保证嘴型不被 motion 冲掉
          internal.on("beforeModelUpdate", updateParams);
        }
        app.ticker.add(updateParams);

        // 汇报模型状态
        useFace.getState().setReport({
          channels: 15,
          scheme: "vowel",
          found: ["ParamAngleX", "ParamAngleY", "ParamMouthOpenY", "ParamEyeBallX"],
        });

        logUi("info", `[Live2D] 模型加载成功: ${targetModelId}`);
      } catch (err: unknown) {
        if (!canceled) {
          let msg = err instanceof Error ? err.message : String(err);
          if (msg.toLowerCase().includes("unknown error") || msg.includes("Cannot read properties of null")) {
            msg = "模型解析失败：该模型格式可能为 Cubism 5.0 (MOC3 v6)，当前运行库支持 Cubism 2 / 3 / 4 标准模型。";
          }
          setError(msg);
          logUi("error", `[Live2D] 加载失败: ${msg}`);
          useFace.getState().setReport({
            channels: 0,
            scheme: "none",
            found: [],
            error: msg,
          });
        }
      }
    }

    void init();

    return () => {
      canceled = true;
      try {
        window.removeEventListener("pointermove", onPointerMove);
        window.removeEventListener("pointerleave", onPointerLeave);
      } catch {}
      try {
        if (modelRef.current) {
          modelRef.current.destroy({ children: true });
          modelRef.current = null;
        }
      } catch {}
      try {
        if (appRef.current) {
          appRef.current.destroy(true);
          appRef.current = null;
        }
      } catch {}
      try {
        if (canvasElement && canvasElement.parentNode) {
          canvasElement.parentNode.removeChild(canvasElement);
        }
      } catch {}
    };
  }, [modelId]);

  // 2. 响应 Live2DConfig 动态实时更新（无需重载模型）
  useEffect(() => {
    applyTransform(live2dConfig);
  }, [live2dConfig]);

  // 3. 动作 (Action) 调度
  useEffect(() => {
    if (!action || !modelRef.current) return;
    const model = modelRef.current;
    try {
      void model.motion(action);
    } catch {
      try {
        void model.motion("TapBody");
      } catch {}
    }
  }, [action, actionSeq]);

  // 4. 情绪 (Emotion) 调度
  useEffect(() => {
    if (!emotion || !modelRef.current) return;
  }, [emotion]);

  return (
    <div ref={containerRef} className={`relative flex items-center justify-center ${className}`}>
      {error && (
        <div className="absolute inset-x-4 top-4 bg-red-500/20 text-red-300 text-xs px-3 py-2 rounded-lg border border-red-500/40 backdrop-blur-md">
          {error}
        </div>
      )}
    </div>
  );
}
