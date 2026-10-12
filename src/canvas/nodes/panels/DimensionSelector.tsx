import { useState, useRef, useEffect } from "react";
import { ChevronDown } from "../../ui/icons";
import { studioCapabilities } from "../../../api/studio";
import {
  ASPECT_RATIOS,
  VIDEO_RESOLUTIONS,
  calculateVideoDimensions,
  imageSizesByRatio,
  inferRatio,
  inferVideoResolution,
  pickSizeForRatio,
  sizeId,
  videoRatioIds,
  type AspectRatioPreset,
  type CatalogSize,
  type ResolutionPreset,
} from "../../utils/dimensionUtils";

interface DimensionSelectorProps {
  mode: "image" | "video";
  width?: number;
  height?: number;
  ratio?: string;
  resolution?: string;
  /** 出图引擎 id。**图片尺寸按引擎取**，没给就用第一个出图引擎的。 */
  engine?: string;
  onChange: (width: number, height: number, ratio: string, resolution: string) => void;
}

/**
 * 画幅 + 尺寸选择。
 *
 * **图片尺寸只从后端来**（`studio::image_sizes()`）。这里曾经写死一份表，
 * 和后端对不上，而且提供 4K —— 后端明写着 4K 在这台机器上跑不了。
 * 视频仍用 `dimensionUtils` 里那份（视频的尺寸由出片链路自己对齐，不在这次范围里）。
 */
export function DimensionSelector({
  mode,
  width,
  height,
  ratio: explicitRatio,
  resolution: explicitRes,
  engine,
  onChange,
}: DimensionSelectorProps) {
  const [isOpen, setIsOpen] = useState(false);
  const [sizes, setSizes] = useState<CatalogSize[]>([]);
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (mode !== "image") return;
    let alive = true;
    void studioCapabilities()
      .then((cat) => {
        const images = cat.engines.filter((e) => e.kind === "image");
        const picked = images.find((e) => e.id === engine) ?? images[0];
        if (alive) setSizes(picked?.sizes ?? []);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [mode, engine]);

  const currentRatio = explicitRatio || inferRatio(width, height);
  const currentVideoRes = explicitRes || inferVideoResolution(width, height);

  const imageGroups = imageSizesByRatio(sizes);
  const ratioChoices: AspectRatioPreset[] =
    mode === "image"
      ? imageGroups.map((g) => g.ratio)
      : ASPECT_RATIOS.filter((a) => videoRatioIds().includes(a.id));
  const currentSizes = imageGroups.find((g) => g.ratio.id === currentRatio)?.sizes ?? [];

  useEffect(() => {
    if (!isOpen) return;
    const handleClickOutside = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setIsOpen(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, [isOpen]);

  const handleSelectRatio = (newRatio: string) => {
    if (mode === "image") {
      const picked = pickSizeForRatio(sizes, newRatio, width && height ? { width, height } : undefined);
      if (!picked) return;
      onChange(picked.width, picked.height, newRatio, sizeId(picked.width, picked.height));
      return;
    }
    const { width: newW, height: newH } = calculateVideoDimensions(newRatio, currentVideoRes);
    onChange(newW, newH, newRatio, currentVideoRes);
  };

  const summary =
    mode === "image" ? `${currentRatio} · ${width ?? "?"}×${height ?? "?"}` : `${currentRatio} · ${currentVideoRes.toUpperCase()}`;

  return (
    <div ref={containerRef} className="relative nodrag nopan nowheel">
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          setIsOpen(!isOpen);
        }}
        onMouseDown={(e) => e.stopPropagation()}
        className={`h-7 flex items-center gap-1.5 rounded-md px-2 text-xs transition-colors select-none ${
          isOpen ? "bg-white/15 text-white" : "text-white/80 hover:bg-white/10 hover:text-white"
        }`}
        title="调整画幅比例与生成尺寸"
      >
        <span className="font-medium tracking-tight">{summary}</span>
        <ChevronDown className={`h-3 w-3 opacity-60 transition-transform ${isOpen ? "rotate-180" : ""}`} />
      </button>

      {isOpen && (
        <div
          onMouseDown={(e) => e.stopPropagation()}
          className="absolute bottom-full left-0 mb-2 z-50 w-72 rounded-2xl border border-white/15 bg-[#1e1e20] p-3 shadow-2xl backdrop-blur-2xl flex flex-col gap-3 animate-in fade-in-0 zoom-in-95 text-xs select-none"
        >
          <div className="flex flex-col gap-1.5">
            <div className="flex items-center justify-between px-0.5">
              <span className="text-[11px] font-semibold text-white/50">画幅比例</span>
              <span className="text-[10px] text-white/40">
                当前: {width} × {height}
              </span>
            </div>
            <div className="grid grid-cols-4 gap-1.5">
              {ratioChoices.map((item) => {
                const isSelected = currentRatio === item.id;
                return (
                  <button
                    key={item.id}
                    type="button"
                    onClick={() => handleSelectRatio(item.id)}
                    className={`flex flex-col items-center justify-center gap-1 rounded-xl p-2 border transition-all ${
                      isSelected
                        ? "border-white/30 bg-white/20 text-white font-medium shadow-sm"
                        : "border-white/5 bg-white/5 text-white/70 hover:bg-white/10 hover:text-white"
                    }`}
                  >
                    <div className="h-5 flex items-center justify-center">
                      <div
                        className={`rounded-xs border ${isSelected ? "border-white bg-white/30" : "border-white/50"}`}
                        style={{
                          width: `${Math.round((item.wRatio / Math.max(item.wRatio, item.hRatio)) * 18)}px`,
                          height: `${Math.round((item.hRatio / Math.max(item.wRatio, item.hRatio)) * 18)}px`,
                        }}
                      />
                    </div>
                    <span className="text-[11px] leading-none">{item.id}</span>
                  </button>
                );
              })}
            </div>
          </div>

          <div className="h-px bg-white/10" />

          {mode === "image" ? (
            <div className="flex flex-col gap-1.5">
              <span className="text-[11px] font-semibold text-white/50 px-0.5">尺寸</span>
              <div className="flex flex-col gap-1">
                {currentSizes.map((s) => {
                  const isSelected = s.width === width && s.height === height;
                  return (
                    <button
                      key={sizeId(s.width, s.height)}
                      type="button"
                      onClick={() => onChange(s.width, s.height, currentRatio, sizeId(s.width, s.height))}
                      className={`flex items-center justify-between rounded-lg py-1.5 px-2 border transition-all ${
                        isSelected
                          ? "border-white/30 bg-white/20 text-white font-semibold"
                          : "border-white/5 bg-white/5 text-white/70 hover:bg-white/10 hover:text-white"
                      }`}
                    >
                      <span>
                        {s.width}×{s.height}
                      </span>
                      {/* 没实测过就直说。借别的尺寸的秒数等于骗人。 */}
                      <span className="text-[10px] text-white/40">
                        {s.verified && s.secs ? `约 ${s.secs} 秒` : "未实测"}
                      </span>
                    </button>
                  );
                })}
                {!currentSizes.length && (
                  <span className="text-[10px] text-white/40 px-0.5">这个画幅没有可用尺寸</span>
                )}
              </div>
              <span className="text-[10px] text-white/35 px-0.5">要 4K：出 2K 再放大 —— 这台机器直接出 4K 内存不够</span>
            </div>
          ) : (
            <div className="flex flex-col gap-1.5">
              <span className="text-[11px] font-semibold text-white/50 px-0.5">分辨率规格</span>
              <div className="grid grid-cols-3 gap-1.5">
                {VIDEO_RESOLUTIONS.map((res: ResolutionPreset) => {
                  const isSelected = currentVideoRes.toLowerCase() === res.id.toLowerCase();
                  return (
                    <button
                      key={res.id}
                      type="button"
                      onClick={() => {
                        const { width: w, height: h } = calculateVideoDimensions(currentRatio, res.id);
                        onChange(w, h, currentRatio, res.id);
                      }}
                      className={`flex flex-col items-center justify-center rounded-lg py-1.5 px-1 border transition-all ${
                        isSelected
                          ? "border-white/30 bg-white/20 text-white font-semibold"
                          : "border-white/5 bg-white/5 text-white/70 hover:bg-white/10 hover:text-white"
                      }`}
                    >
                      <span className="text-xs">{res.label}</span>
                      <span className="text-[9px] text-white/40 scale-90">{res.desc.split(" · ")[0]}</span>
                    </button>
                  );
                })}
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
