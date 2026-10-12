import { useEffect, useMemo, useRef, useState } from "react";
import { Handle, NodeToolbar, Position, type NodeProps } from "@xyflow/react";
import {
  ArrowDown,
  ArrowLeft,
  ArrowRight,
  ArrowUp,
  Camera,
  Grid2x2,
  Grid3x3,
  Loader2,
  Lock,
  Maximize2,
  Globe,
  RotateCcw,
  ZoomIn,
  ZoomOut,
} from "lucide-react";
import { BaseNodeWrapper } from "./BaseNodeWrapper";
import { useCanvasStore } from "../store/useCanvasStore";
import { selectContentNodes } from "../store/selectContentNodes";
import { NodeHeader } from "../ui/NodeHeader";
import { Orbit } from "../ui/icons";
import { createPanoStage, captureShots, type PanoStage } from "../pano/panoStage";
import {
  CAPTURE_ASPECTS,
  FOV_DEFAULT,
  FOV_MAX,
  FOV_MIN,
  applyFrontOffset,
  displayDeg,
  fovToFocal,
  frontToCurrent,
  normalizeDeg,
  shotFileName,
  shotReverseOf,
  viewGrid,
  type CaptureAspect,
  type Shot,
} from "../pano/panoMath";
import {
  PLANET_FOV_DEFAULT,
  PLANET_FOV_MAX,
  PLANET_FOV_MIN,
  clampPlanetFov,
  lockViewCorrection,
  seamView,
} from "../pano/panoOrient";
import { saveChatImageAttachment } from "../../api";
import { mediaDataUrl } from "../../components/studio/MediaThumb";
import {
  Chip,
  PanelSection,
  PanoSplitLayout,
  ResetButton,
  SliderRow,
  nodeWidth,
  useStageAutoResize,
} from "../ui/PanoSidePanel";
import type { PanoViewerNode } from "../types";

/**
 * 360° 全景查看器。
 *
 * **产出是取景截图，不是浏览。** 它转的是相机而不是模型 ——
 * 见 `types.ts` 里 `PanoViewerNodeData` 的注释。
 *
 * 节点上方的截图工具栏、视口里的缩放 / 方向 / 全屏按钮、多视角截图落成画布上的组，
 * 照 DramaClaw（ClaymoreLab, Elastic License 2.0）
 * frontend/src/features/canvas/nodes/Pano360ViewerNode.tsx 改写。
 * 出处规则见 DECISIONS.md 2026-09-16。
 */
export function PanoViewerNodeComponent({ id, data, selected }: NodeProps<PanoViewerNode>) {
  const updateNodeData = useCanvasStore((s) => s.updateNodeData);
  const removeNode = useCanvasStore((s) => s.removeNode);
  const addImageGroup = useCanvasStore((s) => s.addImageGroup);
  const nodes = useCanvasStore(selectContentNodes);
  const edges = useCanvasStore((s) => s.edges);

  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const stageRef = useRef<PanoStage | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  /** 截完之后告诉用户图去哪了 —— 图落在画布上而不是节点里，不说的话会以为没截上 */
  const [status, setStatus] = useState<string | null>(null);
  const viewportRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (busy || !status) return;
    const t = setTimeout(() => setStatus(null), 4000);
    return () => clearTimeout(t);
  }, [busy, status]);
  /** 右侧参数栏展开与否。默认展开 —— 第一次用得先看到有哪些参数。 */
  const [panelOpen, setPanelOpen] = useState(true);
  /** 舞台建好的次数。异步建台后用它踢依赖舞台的 effect 重跑一次。 */
  const [ready, setReady] = useState(0);

  /** 上游图片节点出的那张图。**没有就什么都不显示** —— 空球比空白更误导。 */
  const source = useMemo(() => {
    for (const e of edges.filter((x) => x.target === id)) {
      const n = nodes.find((x) => x.id === e.source);
      const url = (n?.data as { outputImageUrl?: string } | undefined)?.outputImageUrl;
      if (url) return url;
    }
    return null;
  }, [id, nodes, edges]);

  const aspect = (data.aspect || "16:9") as CaptureAspect;
  const fov = data.fovDeg || FOV_DEFAULT;
  /**
   * 小行星模式。视场角另存一份（`planetFovDeg`）：透视的 70° 和小行星的 270°
   * 不是一回事，共用一个值的话进出一次模式视场角就乱了。
   */
  const planet = data.projection === "planet";
  const planetFov = clampPlanetFov(data.planetFovDeg ?? PLANET_FOV_DEFAULT);
  const viewFov = planet ? planetFov : fov;
  /** 进小行星之前的机位，退出时放回去。只在这次打开期间有效 */
  const [planetBackup, setPlanetBackup] = useState<{ yawDeg: number; pitchDeg: number } | null>(null);

  // 建舞台。**换图要重建** —— 贴图换了球也得换。
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !source) return;
    // **`source` 是本机路径，不是可加载的 URL。** 原来直接丢给 TextureLoader，
    // 必然加载失败，界面上只剩一句 `[object Event]`。先读成 data URL ——
    // 不走 `anima-media://` 的理由（没 CORS 头、会污染 canvas 让截图失败）
    // 写在 `mediaDataUrl` 上。
    //
    // 因为解析是异步的，舞台建好之后要 `setReady` 一次：下面那几个依赖舞台的
    // effect（相机、标记）在舞台还不存在时已经跑过一轮并提前返回了，不再踢一下
    // 的话初始视角和已摆的标记都不会上屏 —— 直到用户碰一下参数才出现。
    let cancelled = false;
    let stage: PanoStage | null = null;
    let raf = 0;
    setErr(null);
    mediaDataUrl(source)
      .then((textureUrl) => {
        if (cancelled) return;
        const s = createPanoStage({ canvas, textureUrl, onError: setErr });
        stage = s;
        stageRef.current = s;
        s.resize(canvas.clientWidth, canvas.clientHeight);
        const loop = () => {
          if (cancelled) return;
          s.render();
          raf = requestAnimationFrame(loop);
        };
        loop();
        setReady((n) => n + 1);
      })
      .catch((e) => {
        if (!cancelled) setErr(`取不到全景图：${String(e)}`);
      });
    return () => {
      cancelled = true;
      cancelAnimationFrame(raf);
      stage?.dispose();
      stageRef.current = null;
    };
  }, [source]);

  // 相机参数变了就同步进去
  useEffect(() => {
    const stage = stageRef.current;
    if (!stage) return;
    stage.setCorrection(data.correction ?? { roll: 0, pitch: 0, yaw: 0 });
    stage.setFrontYaw(data.frontYawDeg ?? 0);
    stage.setProjection(planet ? "planet" : "rect");
    stage.setView({ yawDeg: data.yawDeg ?? 0, pitchDeg: data.pitchDeg ?? 0, fovDeg: viewFov });
  }, [data.correction, data.frontYawDeg, data.yawDeg, data.pitchDeg, viewFov, planet, ready]);

  useStageAutoResize(canvasRef, stageRef, ready);

  /** 拖着转。**左右是 yaw、上下是 pitch**，和所有全景查看器一致。 */
  const onDrag = (e: React.PointerEvent) => {
    if (e.buttons !== 1) return;
    // 视角越窄，拖同样距离转得越少 —— 否则窄视角下没法微调。小行星视场角大，按 170° 封顶算
    const k = Math.min(viewFov, 170) / 600;
    const limit = planet ? 90 : 89;
    updateNodeData(id, {
      // 归一化到 ±180：不归的话转几圈之后参数栏里显示 396°，滑块也顶到头
      yawDeg: normalizeDeg((data.yawDeg ?? 0) - e.movementX * k),
      pitchDeg: Math.max(-limit, Math.min(limit, (data.pitchDeg ?? 0) + e.movementY * k)),
    });
  };

  /**
   * 截图并落到画布上：一张就单放一个图片节点，多张排网格打成组。
   * 节点自己的 `outputImages` 照旧追加 —— 已经从这个节点连出去的下游还读它。
   */
  const captureToCanvas = async (
    shots: Shot[],
    group: { cols: number; label: string },
    /** 多视角组永远是普通透视 —— 小行星模式下截 12 视角，截出来的也该是能用的镜头 */
    projection: "rect" | "current" = "current",
  ) => {
    const stage = stageRef.current;
    if (!stage || busy) return;
    setBusy(true);
    setErr(null);
    setStatus(`截图中… 共 ${shots.length} 张`);
    if (projection === "rect") stage.setProjection("rect");
    try {
      const grabbed = await captureShots(stage, shots, aspect);
      const saved: { path: string; label: string }[] = [];
      for (const g of grabbed) {
        const r = await saveChatImageAttachment({
          dataUrl: g.dataUrl,
          filename: shotFileName("pano", g.shot),
        });
        if (r?.path) saved.push({ path: r.path, label: g.shot.name });
      }
      if (!saved.length) throw new Error("截图没保存下来");
      const all = [...(data.outputImages ?? []), ...saved.map((x) => x.path)];
      updateNodeData(id, { outputImages: all, outputImageUrl: all[all.length - 1] });
      addImageGroup(id, saved, { cols: group.cols, label: `全景截图 · ${group.label}`, sourceHandle: "pano-view-out" });
      setStatus(saved.length > 1 ? `已放到右边：${group.label}，${saved.length} 张` : "已放到右边");
    } catch (e) {
      setErr(String(e));
      setStatus(null);
    } finally {
      // 截完把相机放回原位 —— 不放的话界面上停在最后一个机位，
      // 用户以为自己的取景被改掉了
      stage.setProjection(planet ? "planet" : "rect");
      stage.setView({ yawDeg: data.yawDeg ?? 0, pitchDeg: data.pitchDeg ?? 0, fovDeg: viewFov });
      setBusy(false);
    }
  };

  const here: Shot = {
    name: planet ? "小行星" : `取景 ${displayDeg(data.yawDeg ?? 0)}°`,
    yawDeg: data.yawDeg ?? 0,
    pitchDeg: data.pitchDeg ?? 0,
    fovDeg: viewFov,
  };
  const snapCurrent = () => void captureToCanvas([here], { cols: 1, label: planet ? "小行星" : "当前视角" });
  const snapGrid = (kind: "four" | "twelve") => {
    const g = viewGrid(kind);
    void captureToCanvas(g.shots, { cols: g.cols, label: g.label }, "rect");
  };

  /** 进小行星：朝正下方看、视场角拉到 270° —— 脚下的地面卷成一颗星球 */
  const enterPlanet = () => {
    setPlanetBackup({ yawDeg: data.yawDeg ?? 0, pitchDeg: data.pitchDeg ?? 0 });
    updateNodeData(id, { projection: "planet", pitchDeg: -90 });
  };
  const exitPlanet = () => {
    updateNodeData(id, {
      projection: undefined,
      yawDeg: planetBackup?.yawDeg ?? data.yawDeg ?? 0,
      pitchDeg: planetBackup?.pitchDeg ?? 0,
    });
    setPlanetBackup(null);
  };

  const correction = data.correction ?? { roll: 0, pitch: 0, yaw: 0 };
  const setCorrection = (patch: Partial<typeof correction>) =>
    updateNodeData(id, { correction: { ...correction, ...patch } });
  const yaw = data.yawDeg ?? 0;
  const pitch = data.pitchDeg ?? 0;
  const front = data.frontYawDeg ?? 0;

  const setFov = (v: number) =>
    planet
      ? updateNodeData(id, { planetFovDeg: clampPlanetFov(v) })
      : updateNodeData(id, { fovDeg: Math.max(FOV_MIN, Math.min(FOV_MAX, v)) });
  const rotateBy = (dYaw: number, dPitch: number) =>
    updateNodeData(id, { yawDeg: normalizeDeg(yaw + dYaw), pitchDeg: Math.max(-89, Math.min(89, pitch + dPitch)) });

  /** 把现在看的方向烘进球面校正，之后回正看到的就是这个画面。数学见 `lockViewCorrection` */
  const lockView = () =>
    updateNodeData(id, {
      correction: lockViewCorrection(correction, yaw + front, pitch),
      yawDeg: 0,
      pitchDeg: 0,
      frontYawDeg: 0,
    });

  /**
   * 全屏。WKWebView 没开元素全屏时 `requestFullscreen` 不存在 ——
   * 那种环境下按钮直接不显示，而不是点了没反应。
   */
  const canFullscreen =
    typeof document !== "undefined" &&
    Boolean(document.fullscreenEnabled || (document as { webkitFullscreenEnabled?: boolean }).webkitFullscreenEnabled);
  const toggleFullscreen = () => {
    const el = viewportRef.current as (HTMLDivElement & { webkitRequestFullscreen?: () => void }) | null;
    if (!el) return;
    if (document.fullscreenElement) void document.exitFullscreen();
    else if (el.requestFullscreen) void el.requestFullscreen();
    else el.webkitRequestFullscreen?.();
  };

  /** 视场角预设。和 DramaClaw 同一组，覆盖超长焦到超广角。 */
  const FOV_PRESETS = [20, 35, 50, 70, 90, 120, 150];
  const PLANET_FOV_PRESETS = [180, 240, 270, 300, 330];
  /** 转到四个正方位。相对「正前方」算，所以校过正前方之后依然是对的。 */
  const HEADINGS = [
    { label: "正前", yaw: 0 },
    { label: "右", yaw: 90 },
    { label: "正后", yaw: 180 },
    { label: "左", yaw: -90 },
  ];

  const viewport = !source ? (
    // **说清楚缺什么。** 空球比空白更误导 —— 用户会以为节点坏了
    <div className="flex h-full flex-col items-center justify-center gap-2 text-center">
      <Orbit className="h-7 w-7 text-white/25" />
      <p className="px-6 text-[11.5px] leading-relaxed text-white/40">
        连一个出了图的「图片」节点进来。
        <br />
        典型接法：全景出图 → 图片 → 这里。
      </p>
    </div>
  ) : (
    <div
      ref={viewportRef}
      className="nodrag nopan relative h-full w-full cursor-grab bg-black active:cursor-grabbing"
      onPointerMove={onDrag}
      onWheel={(e) => {
        e.stopPropagation();
        setFov(viewFov + Math.sign(e.deltaY) * (planet ? 6 : 3));
      }}
    >
      <canvas ref={canvasRef} className="h-full w-full" />
      {/* 读数常驻。导演看的是「朝哪、多广」，不是滑块位置 */}
      <div className="pointer-events-none absolute left-2 top-2 rounded bg-black/55 px-2 py-1 font-mono text-[10px] text-white/70">
        {planet
          ? `小行星 · yaw ${displayDeg(yaw)}° · pitch ${Math.round(pitch)}° · ${Math.round(viewFov)}°`
          : `yaw ${displayDeg(yaw)}° · pitch ${Math.round(pitch)}° · ${Math.round(fov)}° ≈ ${fovToFocal(fov)}mm`}
      </div>
      {/* 视口按钮：没有触控板、不想拖的时候也能微调。和 DC 同一组 */}
      <div
        className="absolute bottom-2 left-2 flex items-center gap-0.5 rounded-full border border-white/10 bg-black/40 px-1 py-0.5 backdrop-blur-sm"
        onPointerDown={(e) => e.stopPropagation()}
        onPointerMove={(e) => e.stopPropagation()}
      >
        <ViewportButton title="缩小（视角变广）" onClick={() => setFov(viewFov + (planet ? 20 : 10))}><ZoomOut /></ViewportButton>
        <ViewportButton title="放大（视角变窄）" onClick={() => setFov(viewFov - (planet ? 20 : 10))}><ZoomIn /></ViewportButton>
        <ViewportButton title="向左转 12°" onClick={() => rotateBy(-12, 0)}><ArrowLeft /></ViewportButton>
        <ViewportButton title="向右转 12°" onClick={() => rotateBy(12, 0)}><ArrowRight /></ViewportButton>
        <ViewportButton title="抬头 8°" onClick={() => rotateBy(0, 8)}><ArrowUp /></ViewportButton>
        <ViewportButton title="低头 8°" onClick={() => rotateBy(0, -8)}><ArrowDown /></ViewportButton>
        {canFullscreen && (
          <ViewportButton title="全屏" onClick={toggleFullscreen}><Maximize2 /></ViewportButton>
        )}
      </div>
      {(busy || status) && (
        <div className="pointer-events-none absolute bottom-2 right-10 rounded bg-black/55 px-2 py-1 text-[11px] text-white/70">
          {status}
        </div>
      )}
    </div>
  );

  const panelBody = (
    <>
      <PanelSection
        title={planet ? "视场角 · 小行星" : "视场角"}
        aside={planet ? `${Math.round(planetFov)}°` : `${Math.round(fov)}° · ${fovToFocal(fov)}mm`}
      >
        <SliderRow
          label="fov"
          value={viewFov}
          min={planet ? PLANET_FOV_MIN : FOV_MIN}
          max={planet ? PLANET_FOV_MAX : FOV_MAX}
          onChange={setFov}
        />
        <div className="flex flex-wrap gap-1.5">
          {(planet ? PLANET_FOV_PRESETS : FOV_PRESETS).map((f) => (
            <Chip key={f} active={Math.round(viewFov) === f} onClick={() => setFov(f)}>
              {f}°
            </Chip>
          ))}
        </div>
      </PanelSection>

      <PanelSection title="小行星">
        <p className="text-[11px] leading-relaxed text-white/35">
          球极投影：朝下看地面卷成一颗星球，朝上看是隧道。视场角越大星球越小。
        </p>
        <div className="flex flex-wrap gap-1.5">
          {planet ? (
            <>
              <Chip active onClick={exitPlanet}>退出小行星</Chip>
              <Chip onClick={() => updateNodeData(id, { pitchDeg: -90 })} title="朝正下方：小行星">星球</Chip>
              <Chip onClick={() => updateNodeData(id, { pitchDeg: 90 })} title="朝正上方：隧道 / 兔子洞">隧道</Chip>
            </>
          ) : (
            <Chip disabled={!source} onClick={enterPlanet}>小行星模式</Chip>
          )}
        </div>
      </PanelSection>

      <PanelSection
        title="机位"
        aside={<ResetButton onClick={() => updateNodeData(id, { yawDeg: 0, pitchDeg: 0 })} />}
      >
        <SliderRow label="yaw" value={yaw} min={-180} max={180} onChange={(v) => updateNodeData(id, { yawDeg: v })} />
        <SliderRow label="pitch" value={pitch} min={-89} max={89} onChange={(v) => updateNodeData(id, { pitchDeg: v })} />
        <div className="flex flex-wrap gap-1.5">
          {HEADINGS.map((h) => (
            <Chip
              key={h.label}
              active={Math.round(yaw) === h.yaw && Math.round(pitch) === 0}
              onClick={() => updateNodeData(id, { yawDeg: h.yaw, pitchDeg: 0 })}
            >
              {h.label}
            </Chip>
          ))}
          <Chip
            onClick={() => updateNodeData(id, seamView(correction, front))}
            title="转到贴图左右边缘拼起来的那条缝。生成的全景这里最容易断层"
          >
            接缝
          </Chip>
        </div>
        <div className="flex flex-wrap gap-1.5">
          <Chip onClick={lockView} title="把现在看的方向烘进球面校正：之后回正（yaw 0 / pitch 0）看到的就是现在这个画面">
            <span className="inline-flex items-center gap-1"><Lock className="h-3 w-3" />锁定当前视角</span>
          </Chip>
        </div>
      </PanelSection>

      <PanelSection
        title="球面校正"
        aside={<ResetButton onClick={() => updateNodeData(id, { correction: { roll: 0, pitch: 0, yaw: 0 } })} />}
      >
        {/* **生成的全景不可能正好水平。** 歪的地平线转到侧面会
            变成明显的倾斜 —— 校一次，所有机位都跟着正。 */}
        <p className="text-[11px] leading-relaxed text-white/35">
          生成的全景不会正好水平。校一次，所有机位都跟着正。
        </p>
        <SliderRow label="roll" value={correction.roll} min={-30} max={30} step={0.5} onChange={(v) => setCorrection({ roll: v })} />
        <SliderRow label="pitch" value={correction.pitch} min={-90} max={90} step={0.5} onChange={(v) => setCorrection({ pitch: v })} />
        <SliderRow label="yaw" value={correction.yaw} min={-180} max={180} onChange={(v) => setCorrection({ yaw: v })} />
      </PanelSection>

      <PanelSection title="正前方" aside={`偏移 ${displayDeg(data.frontYawDeg ?? 0)}°`}>
        <div className="flex flex-wrap items-center gap-1.5">
          <Chip
            onClick={() => updateNodeData(id, frontToCurrent(yaw, data.frontYawDeg ?? 0))}
            title="把现在看的方向定为 0°，之后所有机位都相对它算"
          >
            设为当前视角
          </Chip>
          <Chip
            onClick={() => updateNodeData(id, { frontYawDeg: 0, yawDeg: applyFrontOffset(yaw, data.frontYawDeg ?? 0) })}
            title="偏移归零，读数改回以全景图正中为 0°（画面不动）"
          >
            清除
          </Chip>
        </div>
      </PanelSection>

      <PanelSection title="截图画幅">
        <div className="flex flex-wrap gap-1.5">
          {CAPTURE_ASPECTS.map((a) => (
            <Chip key={a.id} active={aspect === a.id} onClick={() => updateNodeData(id, { aspect: a.id })}>
              {a.label}
            </Chip>
          ))}
        </div>
      </PanelSection>

      <PanelSection title="截图到画布">
        <p className="text-[11px] leading-relaxed text-white/35">
          截下来的图放在节点右边；多张会排好并打成一个组，能直接连去出图。
        </p>
        <div className="flex flex-wrap gap-1.5">
          <Chip disabled={busy || !source} onClick={snapCurrent}>
            截当前
          </Chip>
          {/* **反打是加 180°，几何保证。** 模型转不过去，相机一定转得过去 */}
          <Chip
            disabled={busy || !source}
            onClick={() => void captureToCanvas([here, { ...shotReverseOf(here), name: "反打" }], { cols: 2, label: "正反打" })}
            title="当前机位 + 它正对面的机位。差整 180°，不可能不转过去"
          >
            正反打
          </Chip>
          <Chip disabled={busy || !source} onClick={() => snapGrid("four")} title="前 / 右 / 后 / 左，平视，90°">
            4 视角
          </Chip>
          <Chip
            disabled={busy || !source}
            onClick={() => snapGrid("twelve")}
            title="前 / 右 / 后 / 左 × 仰 40° / 平视 / 俯 40°，75°"
          >
            12 视角
          </Chip>
        </div>
      </PanelSection>
    </>
  );

  return (
    <div className="flex flex-col gap-2 select-none" style={{ width: nodeWidth(panelOpen) + 24 }}>
      <NodeToolbar isVisible={selected && Boolean(source)} position={Position.Top} offset={12}>
        <div className="flex items-center gap-0.5 rounded-full border border-white/12 bg-[#282828]/95 px-1.5 py-1 shadow-[0_10px_24px_rgba(0,0,0,0.32)] backdrop-blur-md">
          <ToolbarButton title="截当前视角" disabled={busy} onClick={snapCurrent}>
            {busy ? <Loader2 className="animate-spin" /> : <Camera />}
          </ToolbarButton>
          <ToolbarButton title="4 视角截图组（前 / 右 / 后 / 左）" disabled={busy} onClick={() => snapGrid("four")}>
            <Grid2x2 />
          </ToolbarButton>
          <ToolbarButton title="12 视角截图组（四个方向 × 仰 / 平 / 俯）" disabled={busy} onClick={() => snapGrid("twelve")}>
            <Grid3x3 />
          </ToolbarButton>
          <span className="mx-1 h-5 w-px bg-white/15" aria-hidden />
          <ToolbarButton title={planet ? "退出小行星" : "小行星模式"} active={planet} onClick={planet ? exitPlanet : enterPlanet}>
            <Globe />
          </ToolbarButton>
          <ToolbarButton title="视角回正" onClick={() => updateNodeData(id, { yawDeg: 0, pitchDeg: 0 })}>
            <RotateCcw />
          </ToolbarButton>
        </div>
      </NodeToolbar>
      <NodeHeader
        icon={<Orbit className="h-3.5 w-3.5 text-white/60" />}
        title={source ? "360° 全景查看器" : "360° 全景查看器 · 等待上游连接全景图"}
        onDelete={() => removeNode(id)}
      />
      <BaseNodeWrapper id={id} selected={selected} sourceHandleId="pano-view-out">
        <Handle
          type="target"
          position={Position.Left}
          id="pano-view-in"
          className="!w-2.5 !h-5 !rounded-full !bg-zinc-300 hover:!bg-blue-400 hover:!border-white/50 !border !border-zinc-800 shadow-md transition-transform hover:scale-125 cursor-crosshair z-10"
        />
        <div className="flex w-full flex-col gap-2 p-3">
          <PanoSplitLayout
            height={360}
            panelOpen={panelOpen}
            onTogglePanel={() => setPanelOpen((v) => !v)}
            viewport={viewport}
            panel={panelBody}
          />
          {err && (
            <p className="rounded-md bg-rose-400/10 px-2 py-1.5 text-[11px] leading-relaxed text-rose-200/80">
              {err}
            </p>
          )}
        </div>
      </BaseNodeWrapper>
    </div>
  );
}

function ToolbarButton({
  title,
  onClick,
  disabled,
  active,
  children,
}: {
  title: string;
  onClick: () => void;
  disabled?: boolean;
  active?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      title={title}
      disabled={disabled}
      onClick={(e) => {
        e.stopPropagation();
        onClick();
      }}
      className={`inline-flex h-8 w-8 items-center justify-center rounded-full transition-colors hover:bg-white/10 hover:text-white disabled:opacity-40 [&>svg]:h-4 [&>svg]:w-4 ${active ? "bg-white/15 text-white" : "text-white/75"}`}
    >
      {children}
    </button>
  );
}

function ViewportButton({ title, onClick, children }: { title: string; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      type="button"
      title={title}
      onClick={(e) => {
        e.stopPropagation();
        onClick();
      }}
      className="inline-flex h-7 w-7 items-center justify-center rounded-full text-white/70 transition-colors hover:bg-white/10 hover:text-white [&>svg]:h-4 [&>svg]:w-4 [&>svg]:stroke-[1.8]"
    >
      {children}
    </button>
  );
}
