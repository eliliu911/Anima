import { useTranslation } from "react-i18next";
/**
 * Live2D 模型参数与渲染配置编辑器：现代 Antigravity 纯白卡片设计
 */
import { useState } from "react";
import { useFace, type Live2DConfig, DEFAULT_LIVE2D_CONFIG } from "../store/face";
import { selfTest } from "../avatar/mouth";

interface Props {
  disabled?: boolean;
}

export function Live2DConfigEditor({ disabled = false }: Props) {
  const { t } = useTranslation(["settings", "common"]);
  const { live2dConfig, updateLive2dConfig, resetLive2dConfig, modelId } = useFace();
  const [activeTab, setActiveTab] = useState<"transform" | "animation" | "render">("transform");

  const cfg: Live2DConfig = {
    ...DEFAULT_LIVE2D_CONFIG,
    ...(live2dConfig || {}),
  };

  const update = (partial: Partial<Live2DConfig>) => {
    updateLive2dConfig(partial);
  };

  const modelName = modelId
    ? modelId.split("/").filter(Boolean).pop()?.replace(/\.model3?\.json$/i, "") || "Live2D"
    : "Hiyori (默认)";

  return (
    <div className="flex flex-col gap-4 bg-[var(--paper)] border border-[var(--line)] rounded-2xl p-4 shadow-2xs text-[13px]">
      {/* 头部标题与当前模型 */}
      <div className="flex items-center justify-between pb-3 border-b border-[var(--line)]/60">
        <div className="flex items-center gap-2 flex-wrap">
          <span className="font-bold text-[var(--ink)]">{t("live2d.debugTitle", { defaultValue: "Live2D 画面与参数调试" })}</span>
          <span className="text-[11px] px-2.5 py-0.5 rounded-full bg-sky-50 text-sky-600 border border-sky-200/60 font-mono font-medium">
            {modelName}
          </span>
        </div>
        <button
          type="button"
          disabled={disabled}
          onClick={resetLive2dConfig}
          className="text-[11.5px] text-[var(--ink-2)] hover:text-[var(--ink)] hover:underline cursor-pointer disabled:opacity-50"
        >
          {t("live2d.restoreDefault", { defaultValue: "恢复默认" })}
        </button>
      </div>

      {/* 选项卡导航 */}
      <div className="flex items-center gap-1 bg-[var(--surface)] p-1 rounded-xl border border-[var(--line)] shadow-2xs">
        <button
          type="button"
          disabled={disabled}
          onClick={() => setActiveTab("transform")}
          className={`flex-1 py-1.5 px-2.5 rounded-lg text-center text-[12px] font-semibold transition-all cursor-pointer ${
            activeTab === "transform"
              ? "bg-[var(--paper)] text-[var(--ink)] shadow-2xs border border-[var(--line)]"
              : "text-[var(--ink-2)] hover:text-[var(--ink)]"
          }`}
        >
          {t("live2d.tabs.transform", { defaultValue: "缩放与位置" })}
        </button>
        <button
          type="button"
          disabled={disabled}
          onClick={() => setActiveTab("animation")}
          className={`flex-1 py-1.5 px-2.5 rounded-lg text-center text-[12px] font-semibold transition-all cursor-pointer ${
            activeTab === "animation"
              ? "bg-[var(--paper)] text-[var(--ink)] shadow-2xs border border-[var(--line)]"
              : "text-[var(--ink-2)] hover:text-[var(--ink)]"
          }`}
        >
          {t("live2d.tabs.animation", { defaultValue: "动画与视线" })}
        </button>
        <button
          type="button"
          disabled={disabled}
          onClick={() => setActiveTab("render")}
          className={`flex-1 py-1.5 px-2.5 rounded-lg text-center text-[12px] font-semibold transition-all cursor-pointer ${
            activeTab === "render"
              ? "bg-[var(--paper)] text-[var(--ink)] shadow-2xs border border-[var(--line)]"
              : "text-[var(--ink-2)] hover:text-[var(--ink)]"
          }`}
        >
          {t("live2d.tabs.render", { defaultValue: "渲染与特效" })}
        </button>
      </div>

      {/* 1. 缩放与位置 */}
      {activeTab === "transform" && (
        <div className="flex flex-col gap-3.5 pt-1">
          {/* {t("live2d.presetsTitle", { defaultValue: "构图预设" })} */}
          <div className="flex flex-col gap-1.5">
            <span className="text-[12px] text-[var(--ink)] font-bold">构图预设</span>
            <div className="grid grid-cols-3 gap-2">
              <button
                type="button"
                disabled={disabled}
                onClick={() => update({ focusPreset: "half", scale: 1.0, offsetX: 0, offsetY: 0 })}
                className={`py-2 px-3 rounded-xl border text-center transition-all cursor-pointer shadow-2xs ${
                  cfg.focusPreset === "half"
                    ? "border-[var(--celadon)] bg-[var(--celadon-2)] text-[var(--celadon)] font-bold"
                    : "border-[var(--line)] bg-[var(--surface)] text-[var(--ink-2)] hover:border-[var(--ink-3)]"
                }`}
              >
                <div className="text-[12.5px]">{t("live2d.presets.portrait", { defaultValue: "半身特写" })}</div>
                <div className="text-[10px] text-[var(--ink-3)] mt-0.5">(Portrait)</div>
              </button>
              <button
                type="button"
                disabled={disabled}
                onClick={() => update({ focusPreset: "full", scale: 1.0, offsetX: 0, offsetY: 0 })}
                className={`py-2 px-3 rounded-xl border text-center transition-all cursor-pointer shadow-2xs ${
                  cfg.focusPreset === "full"
                    ? "border-[var(--celadon)] bg-[var(--celadon-2)] text-[var(--celadon)] font-bold"
                    : "border-[var(--line)] bg-[var(--surface)] text-[var(--ink-2)] hover:border-[var(--ink-3)]"
                }`}
              >
                <div className="text-[12.5px]">{t("live2d.presets.fullBody", { defaultValue: "全身展示" })}</div>
                <div className="text-[10px] text-[var(--ink-3)] mt-0.5">(Full Model)</div>
              </button>
              <button
                type="button"
                disabled={disabled}
                onClick={() => update({ focusPreset: "custom" })}
                className={`py-2 px-3 rounded-xl border text-center transition-all cursor-pointer shadow-2xs ${
                  cfg.focusPreset === "custom"
                    ? "border-[var(--celadon)] bg-[var(--celadon-2)] text-[var(--celadon)] font-bold"
                    : "border-[var(--line)] bg-[var(--surface)] text-[var(--ink-2)] hover:border-[var(--ink-3)]"
                }`}
              >
                <div className="text-[12.5px]">{t("live2d.presets.custom", { defaultValue: "自由微调" })}</div>
                <div className="text-[10px] text-[var(--ink-3)] mt-0.5">(Custom Offset)</div>
              </button>
            </div>
          </div>

          {/* 缩放 Scale */}
          <div className="flex flex-col gap-1">
            <div className="flex items-center justify-between text-[12px]">
              <span className="text-[var(--ink-2)] font-medium">{t("live2d.sliders.scale", { defaultValue: "模型大小 (缩放)" })}</span>
              <div className="flex items-center gap-2">
                <span className="font-mono text-[var(--ink)] font-semibold">
                  {(cfg.scale ?? 1.0).toFixed(2)}x
                </span>
                <button
                  type="button"
                  disabled={disabled}
                  onClick={() => update({ scale: 1.0 })}
                  className="text-[11px] text-[var(--ink-3)] hover:text-[var(--ink)] cursor-pointer"
                  title="重置缩放"
                >
                  ↺
                </button>
              </div>
            </div>
            <input
              type="range"
              disabled={disabled}
              min={0.2}
              max={3.5}
              step={0.05}
              value={cfg.scale ?? 1.0}
              onChange={(e) => update({ scale: parseFloat(e.target.value) })}
              className="w-full accent-[var(--celadon)] h-1.5 bg-[var(--line)] rounded-lg cursor-pointer"
            />
          </div>

          {/* 水平偏移 X */}
          <div className="flex flex-col gap-1">
            <div className="flex items-center justify-between text-[12px]">
              <span className="text-[var(--ink-2)] font-medium">{t("live2d.sliders.posX", { defaultValue: "水平位置 (X 轴)" })}</span>
              <div className="flex items-center gap-2">
                <span className="font-mono text-[var(--ink)] font-semibold">
                  {(cfg.offsetX ?? 0) > 0 ? `+${cfg.offsetX}` : cfg.offsetX ?? 0} %
                </span>
                <button
                  type="button"
                  disabled={disabled}
                  onClick={() => update({ offsetX: 0 })}
                  className="text-[11px] text-[var(--ink-3)] hover:text-[var(--ink)] cursor-pointer"
                  title="重置水平位置"
                >
                  ↺
                </button>
              </div>
            </div>
            <input
              type="range"
              disabled={disabled}
              min={-150}
              max={150}
              step={1}
              value={cfg.offsetX ?? 0}
              onChange={(e) => update({ offsetX: parseInt(e.target.value, 10) })}
              className="w-full accent-[var(--celadon)] h-1.5 bg-[var(--line)] rounded-lg cursor-pointer"
            />
          </div>

          {/* 垂直偏移 Y */}
          <div className="flex flex-col gap-1">
            <div className="flex items-center justify-between text-[12px]">
              <span className="text-[var(--ink-2)] font-medium">{t("live2d.sliders.posY", { defaultValue: "垂直位置 (Y 轴)" })}</span>
              <div className="flex items-center gap-2">
                <span className="font-mono text-[var(--ink)] font-semibold">
                  {(cfg.offsetY ?? 0) > 0 ? `+${cfg.offsetY}` : cfg.offsetY ?? 0} %
                </span>
                <button
                  type="button"
                  disabled={disabled}
                  onClick={() => update({ offsetY: 0 })}
                  className="text-[11px] text-[var(--ink-3)] hover:text-[var(--ink)] cursor-pointer"
                  title="重置垂直位置"
                >
                  ↺
                </button>
              </div>
            </div>
            <input
              type="range"
              disabled={disabled}
              min={-150}
              max={150}
              step={1}
              value={cfg.offsetY ?? 0}
              onChange={(e) => update({ offsetY: parseInt(e.target.value, 10) })}
              className="w-full accent-[var(--celadon)] h-1.5 bg-[var(--line)] rounded-lg cursor-pointer"
            />
          </div>
        </div>
      )}

      {/* 2. 动画与视线 */}
      {activeTab === "animation" && (
        <div className="flex flex-col gap-3 pt-1">
          {/* 光标跟随 */}
          <div className="flex items-center justify-between py-1">
            <div>
              <div className="text-[12.5px] text-[var(--ink)] font-semibold">视线光标跟随</div>
              <div className="text-[11px] text-[var(--ink-3)]">头部与眼球跟随鼠标光标移动</div>
            </div>
            <div className="relative inline-block w-9 h-5 shrink-0">
              <input
                type="checkbox"
                disabled={disabled}
                checked={cfg.mouseTracking ?? true}
                onChange={(e) => update({ mouseTracking: e.target.checked })}
                className="peer appearance-none w-9 h-5 bg-[var(--surface-3)] border border-[var(--line)] rounded-full checked:bg-[var(--celadon)] checked:border-[var(--celadon)] cursor-pointer transition-colors"
              />
              <span className="absolute left-[3px] top-[3px] w-[14px] h-[14px] bg-white rounded-full pointer-events-none transition-transform peer-checked:translate-x-4 shadow-xs" />
            </div>
          </div>

          {/* 视线灵敏度 */}
          {cfg.mouseTracking && (
            <div className="flex flex-col gap-1 pl-3 border-l-2 border-[var(--celadon)]">
              <div className="flex items-center justify-between text-[12px]">
                <span className="text-[var(--ink-2)]">视线跟随灵敏度</span>
                <span className="font-mono text-[var(--ink)] font-semibold">
                  {(cfg.gazeMultiplier ?? 1.0).toFixed(1)}x
                </span>
              </div>
              <input
                type="range"
                disabled={disabled}
                min={0.2}
                max={2.0}
                step={0.1}
                value={cfg.gazeMultiplier ?? 1.0}
                onChange={(e) => update({ gazeMultiplier: parseFloat(e.target.value) })}
                className="w-full accent-[var(--celadon)] h-1.5 bg-[var(--line)] rounded-lg cursor-pointer"
              />
            </div>
          )}

          {/* 待机微跳视 */}
          <div className="flex items-center justify-between py-1">
            <div>
              <div className="text-[12.5px] text-[var(--ink)] font-semibold">待机眼神微动</div>
              <div className="text-[11px] text-[var(--ink-3)]">无鼠标移动时模拟自然眨动与呼吸</div>
            </div>
            <div className="relative inline-block w-9 h-5 shrink-0">
              <input
                type="checkbox"
                disabled={disabled}
                checked={cfg.idleMovement ?? true}
                onChange={(e) => update({ idleMovement: e.target.checked })}
                className="peer appearance-none w-9 h-5 bg-[var(--surface-3)] border border-[var(--line)] rounded-full checked:bg-[var(--celadon)] checked:border-[var(--celadon)] cursor-pointer transition-colors"
              />
              <span className="absolute left-[3px] top-[3px] w-[14px] h-[14px] bg-white rounded-full pointer-events-none transition-transform peer-checked:translate-x-4 shadow-xs" />
            </div>
          </div>

          {/* 自动眨眼 */}
          <div className="flex items-center justify-between py-1">
            <div>
              <div className="text-[12.5px] text-[var(--ink)] font-semibold">自动眨眼</div>
              <div className="text-[11px] text-[var(--ink-3)]">保持鲜活的眼部微表情</div>
            </div>
            <div className="relative inline-block w-9 h-5 shrink-0">
              <input
                type="checkbox"
                disabled={disabled}
                checked={cfg.blink ?? true}
                onChange={(e) => update({ blink: e.target.checked })}
                className="peer appearance-none w-9 h-5 bg-[var(--surface-3)] border border-[var(--line)] rounded-full checked:bg-[var(--celadon)] checked:border-[var(--celadon)] cursor-pointer transition-colors"
              />
              <span className="absolute left-[3px] top-[3px] w-[14px] h-[14px] bg-white rounded-full pointer-events-none transition-transform peer-checked:translate-x-4 shadow-xs" />
            </div>
          </div>

          {/* 动作速度 */}
          <div className="flex flex-col gap-1">
            <div className="flex items-center justify-between text-[12px]">
              <span className="text-[var(--ink-2)] font-medium">动作播放速度</span>
              <span className="font-mono text-[var(--ink)] font-semibold">
                {(cfg.motionSpeed ?? 1.0).toFixed(1)}x
              </span>
            </div>
            <input
              type="range"
              disabled={disabled}
              min={0.5}
              max={2.0}
              step={0.1}
              value={cfg.motionSpeed ?? 1.0}
              onChange={(e) => update({ motionSpeed: parseFloat(e.target.value) })}
              className="w-full accent-[var(--celadon)] h-1.5 bg-[var(--line)] rounded-lg cursor-pointer"
            />
          </div>

          {/* 口型同步与测试 */}
          <div className="flex items-center justify-between pt-2 border-t border-[var(--line)]/50">
            <div>
              <div className="text-[12.5px] text-[var(--ink)] font-semibold">口型与声音同步</div>
              <div className="text-[11px] text-[var(--ink-3)]">根据当前说话语音实时对口型</div>
            </div>
            <button
              type="button"
              disabled={disabled}
              onClick={() => selfTest()}
              className="px-3.5 py-1.5 bg-[var(--surface-2)] hover:bg-[var(--surface-3)] text-[var(--ink)] border border-[var(--line)] rounded-xl text-[11.5px] font-semibold transition-all cursor-pointer shadow-2xs"
            >
              🎙️ 测试说话口型
            </button>
          </div>
        </div>
      )}

      {/* 3. 渲染与特效 */}
      {activeTab === "render" && (
        <div className="flex flex-col gap-3.5 pt-1">
          {/* 帧率上限 */}
          <div className="flex flex-col gap-1.5">
            <span className="text-[12px] text-[var(--ink)] font-bold">FPS 帧率上限</span>
            <div className="grid grid-cols-3 gap-2">
              <button
                type="button"
                disabled={disabled}
                onClick={() => update({ fpsLimit: 30 })}
                className={`py-1.5 px-3 rounded-xl border text-center transition-all cursor-pointer shadow-2xs ${
                  cfg.fpsLimit === 30
                    ? "border-[var(--celadon)] bg-[var(--celadon-2)] text-[var(--celadon)] font-bold"
                    : "border-[var(--line)] bg-[var(--surface)] text-[var(--ink-2)]"
                }`}
              >
                30 帧 (省电)
              </button>
              <button
                type="button"
                disabled={disabled}
                onClick={() => update({ fpsLimit: 60 })}
                className={`py-1.5 px-3 rounded-xl border text-center transition-all cursor-pointer shadow-2xs ${
                  cfg.fpsLimit === 60
                    ? "border-[var(--celadon)] bg-[var(--celadon-2)] text-[var(--celadon)] font-bold"
                    : "border-[var(--line)] bg-[var(--surface)] text-[var(--ink-2)]"
                }`}
              >
                60 帧 (流畅)
              </button>
              <button
                type="button"
                disabled={disabled}
                onClick={() => update({ fpsLimit: 0 })}
                className={`py-1.5 px-3 rounded-xl border text-center transition-all cursor-pointer shadow-2xs ${
                  cfg.fpsLimit === 0
                    ? "border-[var(--celadon)] bg-[var(--celadon-2)] text-[var(--celadon)] font-bold"
                    : "border-[var(--line)] bg-[var(--surface)] text-[var(--ink-2)]"
                }`}
              >
                ∞ 不限
              </button>
            </div>
          </div>

          {/* 桌面柔和投影 */}
          <div className="flex items-center justify-between py-1">
            <div>
              <div className="text-[12.5px] text-[var(--ink)] font-semibold">桌面柔和投影</div>
              <div className="text-[11px] text-[var(--ink-3)]">在透明桌面上呈现柔和立体阴影</div>
            </div>
            <div className="relative inline-block w-9 h-5 shrink-0">
              <input
                type="checkbox"
                disabled={disabled}
                checked={cfg.dropShadow ?? true}
                onChange={(e) => update({ dropShadow: e.target.checked })}
                className="peer appearance-none w-9 h-5 bg-[var(--surface-3)] border border-[var(--line)] rounded-full checked:bg-[var(--celadon)] checked:border-[var(--celadon)] cursor-pointer transition-colors"
              />
              <span className="absolute left-[3px] top-[3px] w-[14px] h-[14px] bg-white rounded-full pointer-events-none transition-transform peer-checked:translate-x-4 shadow-xs" />
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
