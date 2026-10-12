/**
 * 面容形态的选择（电子云 / Live2D / 3D 实体 / 神经写实）：现代 Antigravity 纯白卡片设计
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { PersonaLockBanner, usePersona } from "./PersonaLock";
import {
  avatarModels,
  avatarModelsDir,
  openAvatarModelsDir,
  type AvatarModel,
} from "../api/voice";
import { mouth, selfTest, VISEMES } from "../avatar/mouth";
import { useFace } from "../store/face";
import { DEFAULT_HEAD_ID } from "../avatar/defaultHead";
import { SCHEME_LABEL } from "../avatar/morphMap";
import { Live2DConfigEditor } from "./Live2DConfigEditor";
import { ErrorBoundary } from "./ErrorBoundary";

export function AvatarSettings() {
  const { t } = useTranslation(["settings", "common"]);
  const { view: persona } = usePersona();
  const locked = persona ? !persona.resolved.is_default_form : false;
  const [models, setModels] = useState<AvatarModel[]>([]);
  const [dir, setDir] = useState("");
  const [message, setMessage] = useState<string | null>(null);
  const {
    modelId,
    mode,
    hologramStyle,
    hologramColor,
    setModelId: pick,
    setMode,
    setHologramStyle,
    setHologramColor,
  } = useFace();
  const report = useFace((s) => s.report);

  const reload = useCallback(async () => {
    try {
      const [list, d] = await Promise.all([avatarModels(), avatarModelsDir()]);
      setModels(list);
      setDir(d);
      const current = useFace.getState().modelId;
      if (current && current !== DEFAULT_HEAD_ID && !list.some((m) => m.id === current)) {
        useFace.getState().setModelId(null);
        if (useFace.getState().mode !== "abstract") {
          useFace.getState().setMode("abstract");
        }
        setMessage("原先选的模型文件不见了，已切回电子云");
      }
    } catch (err) {
      setMessage(`${err}`);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  const activeTag: "abstract" | "live2d" | "3d" | "neural" =
    mode === "abstract"
      ? "abstract"
      : mode === "live2d"
      ? "live2d"
      : mode === "neural_stream"
      ? "neural"
      : "3d";

  const live2dModels = models.filter(
    (m) =>
      m.kind === "live2d" ||
      m.id.toLowerCase().endsWith(".model3.json") ||
      m.id.toLowerCase().endsWith(".model.json")
  );

  const neuralModels = models.filter(
    (m) =>
      m.kind === "neural" ||
      m.folder === "neural" ||
      m.id.toLowerCase().startsWith("neural/")
  );

  const models3D = models.filter(
    (m) =>
      m.kind !== "live2d" &&
      m.kind !== "neural" &&
      !m.folder.startsWith("neural") &&
      !m.id.toLowerCase().endsWith(".model3.json") &&
      !m.id.toLowerCase().endsWith(".model.json")
  );

  const folders3D = Object.entries(
    models3D
      .filter((m) => m.folder)
      .reduce<Record<string, AvatarModel[]>>((acc, m) => {
        (acc[m.folder] ??= []).push(m);
        return acc;
      }, {})
  );

  const switchTo3D = () => {
    if (mode === "abstract" || mode === "live2d" || mode === "neural_stream") {
      setMode("concrete");
      if (!modelId || modelId.endsWith(".json") || modelId.startsWith("neural/")) {
        pick(DEFAULT_HEAD_ID);
      }
    }
  };

  const switchToLive2D = () => {
    setMode("live2d");
    if (!modelId || !modelId.endsWith(".json")) {
      const first = live2dModels[0];
      if (first) pick(first.id);
    }
  };

  const switchToNeural = () => {
    setMode("neural_stream");
    if (!modelId || (!modelId.startsWith("neural/") && !neuralModels.some((m) => m.id === modelId))) {
      const first = neuralModels[0];
      if (first) pick(first.id);
    }
  };

  return (
    <div className="flex flex-col gap-4 max-w-4xl pb-8">
      <PersonaLockBanner view={persona} what="面容" />
      
      <div className="flex items-center justify-between">
        <div className="flex flex-col">
          <span className="text-[14px] font-bold text-[var(--ink)] tracking-tight">面容形态</span>
          <span className="text-[12px] text-[var(--ink-2)] mt-0.5 leading-relaxed">
            选择形象渲染形态与调试参数
          </span>
        </div>
      </div>

      {/* 顶部四大分类 Tag 导航 */}
      <div className="grid grid-cols-4 p-1 bg-[var(--paper)] border border-[var(--line)] rounded-2xl gap-1 shadow-2xs">
        <button
          type="button"
          disabled={locked}
          onClick={() => setMode("abstract")}
          className={`py-2 px-2 rounded-xl text-center text-[12px] font-semibold transition-all cursor-pointer truncate ${
            activeTag === "abstract"
              ? "bg-[var(--surface-2)] text-[var(--ink)] shadow-2xs border border-[var(--line)]"
              : "text-[var(--ink-2)] hover:text-[var(--ink)]"
          }`}
        >
          🌌 电子云
        </button>
        <button
          type="button"
          disabled={locked}
          onClick={switchToLive2D}
          className={`py-2 px-2 rounded-xl text-center text-[12px] font-semibold transition-all cursor-pointer truncate ${
            activeTag === "live2d"
              ? "bg-[var(--surface-2)] text-[var(--ink)] shadow-2xs border border-[var(--line)]"
              : "text-[var(--ink-2)] hover:text-[var(--ink)]"
          }`}
        >
          🎀 Live2D
        </button>
        <button
          type="button"
          disabled={locked}
          onClick={switchTo3D}
          className={`py-2 px-2 rounded-xl text-center text-[12px] font-semibold transition-all cursor-pointer truncate ${
            activeTag === "3d"
              ? "bg-[var(--surface-2)] text-[var(--ink)] shadow-2xs border border-[var(--line)]"
              : "text-[var(--ink-2)] hover:text-[var(--ink)]"
          }`}
        >
          🧊 3D 实体
        </button>
        <button
          type="button"
          disabled={locked}
          onClick={switchToNeural}
          className={`py-2 px-2 rounded-xl text-center text-[12px] font-semibold transition-all cursor-pointer truncate ${
            activeTag === "neural"
              ? "bg-[var(--surface-2)] text-[var(--ink)] shadow-2xs border border-[var(--line)]"
              : "text-[var(--ink-2)] hover:text-[var(--ink)]"
          }`}
        >
          🎥 神经写实
        </button>
      </div>

      {/* 1. 电子云 */}
      {activeTag === "abstract" && (
        <div className="flex flex-col gap-3">
          <div className="p-4 bg-[var(--paper)] border border-[var(--line)] rounded-2xl shadow-2xs">
            <div className="text-[13.5px] font-bold text-[var(--ink)] mb-1">
              {t("face.abstractTitle", { defaultValue: "动态情感电子云" })}
            </div>
            <p className="text-[12px] text-[var(--ink-2)] leading-relaxed m-0">
              {t("face.abstractDesc", { defaultValue: "纯粒子流体形态，无需加载任何模型素材。粒子矩阵将实时响应当前说话情感轴、声音能量与情绪起伏，展现优雅的呼吸与变色光效。" })}
            </p>
          </div>
        </div>
      )}

      {/* 2. Live2D */}
      {activeTag === "live2d" && (
        <div className="flex flex-col gap-4">
          <ErrorBoundary fallbackTitle="Live2D 参数面板加载异常">
            <Live2DConfigEditor disabled={locked} />
          </ErrorBoundary>

          <div className="flex flex-col gap-2.5">
            <div className="flex items-center justify-between pt-1">
              <span className="text-[13.5px] font-bold text-[var(--ink)]">
                {t("face.live2dTitle", { defaultValue: "Live2D 角色素材" })}
              </span>
              <span className="text-[11px] text-[var(--ink-3)]">
                {t("face.live2dCount", { count: live2dModels.length, defaultValue: `共 ${live2dModels.length} 款模型` })}
              </span>
            </div>

            <div className="grid grid-cols-1 sm:grid-cols-2 gap-2.5">
              {live2dModels.map((m) => {
                const isSelected = modelId === m.id;
                return (
                  <button
                    disabled={locked}
                    key={m.id}
                    type="button"
                    className={`flex items-center justify-between border rounded-2xl p-3.5 text-left transition-all cursor-pointer shadow-2xs ${
                      isSelected
                        ? "border-[var(--celadon)] ring-1 ring-[var(--celadon)]/25 bg-[var(--paper)]"
                        : "border-[var(--line)] bg-[var(--paper)] hover:border-[var(--ink-3)]/50"
                    } disabled:opacity-50`}
                    onClick={() => {
                      pick(m.id);
                    }}
                  >
                    <div className="flex flex-col gap-0.5">
                      <span className="text-[13px] font-bold text-[var(--ink)]">
                        {m.name}
                      </span>
                      <span className="text-[10.5px] text-[var(--ink-3)] font-mono">
                        {(m.bytes / 1048576).toFixed(1)} MB
                      </span>
                    </div>
                    {isSelected && (
                      <span className="text-[11px] font-semibold text-[var(--celadon)] bg-[var(--celadon-2)] px-2 py-0.5 rounded-full border border-[var(--celadon)]/30">
                        {t("face.currentUsed", { defaultValue: "当前使用" })}
                      </span>
                    )}
                  </button>
                );
              })}
            </div>

            {live2dModels.length === 0 && (
              <div className="text-[12px] text-[var(--ink-3)] text-center py-6 border border-dashed border-[var(--line)] rounded-2xl bg-[var(--paper)] shadow-2xs">
                <code className="bg-[var(--surface)] px-1.5 py-0.5 rounded border border-[var(--line)] mr-1">
                  {dir || "avatars/live2d/"}
                </code>
                目录下未检测到 Live2D 素材。
              </div>
            )}
          </div>
        </div>
      )}

      {/* 3. 3D 实体 */}
      {activeTag === "3d" && (
        <div className="flex flex-col gap-4">
          <div className="flex flex-col gap-3 p-4 bg-[var(--paper)] border border-[var(--line)] rounded-2xl shadow-2xs">
            <div className="flex items-center justify-between">
              <div>
                <div className="text-[13.5px] font-bold text-[var(--ink)]">
                  全息粒子渲染模式
                </div>
                <div className="text-[11.5px] text-[var(--ink-2)]">
                  用发光粒子点阵勾勒 3D 模型面部轮廓，随声音流动
                </div>
              </div>
              <div className="relative inline-block w-9 h-5 shrink-0">
                <input
                  type="checkbox"
                  disabled={locked}
                  checked={mode === "hologram"}
                  onChange={(e) => setMode(e.target.checked ? "hologram" : "concrete")}
                  className="peer appearance-none w-9 h-5 bg-[var(--surface-3)] border border-[var(--line)] rounded-full checked:bg-[var(--celadon)] checked:border-[var(--celadon)] cursor-pointer transition-colors"
                />
                <span className="absolute left-[3px] top-[3px] w-[14px] h-[14px] bg-white rounded-full pointer-events-none transition-transform peer-checked:translate-x-4 shadow-xs" />
              </div>
            </div>

            {mode === "hologram" && (
              <div className="flex flex-col gap-3 pt-3 border-t border-[var(--line)]/50">
                <div className="text-[11.5px] font-bold text-[var(--ink-2)]">
                  全息子风格
                </div>
                <div className="flex flex-wrap gap-2">
                  {(["dot", "wireframe", "glitch", "ripple"] as const).map((st) => (
                    <button
                      key={st}
                      disabled={locked}
                      type="button"
                      className={`px-3 py-1.5 text-[11.5px] border rounded-xl transition-all cursor-pointer shadow-2xs ${
                        hologramStyle === st
                          ? "bg-[var(--celadon-2)] border-[var(--celadon)] text-[var(--celadon)] font-bold"
                          : "bg-[var(--surface)] border-[var(--line)] text-[var(--ink-2)] hover:border-[var(--ink-3)]"
                      } disabled:opacity-50`}
                      onClick={() => setHologramStyle(st)}
                    >
                      {st === "dot" ? "点阵" : st === "wireframe" ? "等高地形" : st === "glitch" ? "赛博故障" : "声波涟漪"}
                    </button>
                  ))}
                </div>
                <div className="flex items-center gap-2 mt-1">
                  <span className="text-[11.5px] text-[var(--ink-2)] font-medium">全息主题色：</span>
                  <input
                    disabled={locked}
                    type="color"
                    value={hologramColor}
                    onChange={(e) => setHologramColor(e.target.value)}
                    className="w-16 h-7 p-0 border-none outline-none cursor-pointer bg-transparent rounded-lg shadow-2xs disabled:opacity-50"
                  />
                </div>
              </div>
            )}
          </div>

          {/* 3D 模型素材选择 */}
          <div className="flex flex-col gap-2.5">
            <div className="flex items-center justify-between pt-1">
              <span className="text-[13.5px] font-bold text-[var(--ink)]">
                3D 模型素材 (GLB / VRM)
              </span>
              <span className="text-[11px] text-[var(--ink-3)]">
                共 {models3D.length + 1} 款模型
              </span>
            </div>

            <button
              disabled={locked}
              type="button"
              className={`flex flex-col gap-1 border rounded-2xl p-4 text-left transition-all cursor-pointer shadow-2xs ${
                modelId === DEFAULT_HEAD_ID
                  ? "border-[var(--celadon)] ring-1 ring-[var(--celadon)]/25 bg-[var(--paper)]"
                  : "border-[var(--line)] bg-[var(--paper)] hover:border-[var(--ink-3)]/50"
              } disabled:opacity-50`}
              onClick={() => {
                pick(DEFAULT_HEAD_ID);
              }}
            >
              <div className="flex items-center justify-between">
                <span className="text-[13px] font-bold text-[var(--ink)]">内置头 (标准基准)</span>
                {modelId === DEFAULT_HEAD_ID && (
                  <span className="text-[11px] font-semibold text-[var(--celadon)] bg-[var(--celadon-2)] px-2 py-0.5 rounded-full border border-[var(--celadon)]/30">
                    当前使用
                  </span>
                )}
              </div>
              <span className="text-[12px] text-[var(--ink-2)]">
                代码生成，无需素材。15 个口型通道齐全，为标准口型校准基准。
              </span>
            </button>

            <div className="grid grid-cols-1 sm:grid-cols-2 gap-2.5">
              {models3D
                .filter((m) => !m.folder)
                .map((m) => {
                  const isSelected = modelId === m.id;
                  return (
                    <button
                      disabled={locked}
                      key={m.id}
                      type="button"
                      className={`flex items-center justify-between border rounded-2xl p-3.5 text-left transition-all cursor-pointer shadow-2xs ${
                        isSelected
                          ? "border-[var(--celadon)] ring-1 ring-[var(--celadon)]/25 bg-[var(--paper)]"
                          : "border-[var(--line)] bg-[var(--paper)] hover:border-[var(--ink-3)]/50"
                      } disabled:opacity-50`}
                      onClick={() => {
                        pick(m.id);
                      }}
                    >
                      <div className="flex flex-col gap-0.5">
                        <span className="text-[13px] font-bold text-[var(--ink)]">
                          {m.name}
                        </span>
                        <span className="text-[10.5px] text-[var(--ink-3)] font-mono">
                          {(m.bytes / 1048576).toFixed(1)} MB
                        </span>
                      </div>
                      {isSelected && (
                        <span className="text-[11px] font-semibold text-[var(--celadon)] bg-[var(--celadon-2)] px-2 py-0.5 rounded-full border border-[var(--celadon)]/30">
                          当前使用
                        </span>
                      )}
                    </button>
                  );
                })}
            </div>

            {folders3D.map(([folder, items]) => (
              <div className="flex flex-col gap-2 mt-2" key={folder}>
                <div className="text-[11.5px] font-bold text-[var(--ink-2)] px-1">
                  📁 {folder}
                </div>
                <div className="grid grid-cols-1 sm:grid-cols-2 gap-2.5">
                  {items.map((m) => {
                    const isSelected = modelId === m.id;
                    return (
                      <button
                        disabled={locked}
                        key={m.id}
                        type="button"
                        className={`flex items-center justify-between border rounded-2xl p-3.5 text-left transition-all cursor-pointer shadow-2xs ${
                          isSelected
                            ? "border-[var(--celadon)] ring-1 ring-[var(--celadon)]/25 bg-[var(--paper)]"
                            : "border-[var(--line)] bg-[var(--paper)] hover:border-[var(--ink-3)]/50"
                        } disabled:opacity-50`}
                        onClick={() => {
                          pick(m.id);
                        }}
                      >
                        <div className="flex flex-col gap-0.5">
                          <span className="text-[13px] font-bold text-[var(--ink)]">
                            {m.name}
                          </span>
                          <span className="text-[10.5px] text-[var(--ink-3)] font-mono">
                            {(m.bytes / 1048576).toFixed(1)} MB
                          </span>
                        </div>
                        {isSelected && (
                          <span className="text-[11px] font-semibold text-[var(--celadon)] bg-[var(--celadon-2)] px-2 py-0.5 rounded-full border border-[var(--celadon)]/30">
                            当前使用
                          </span>
                        )}
                      </button>
                    );
                  })}
                </div>
              </div>
            ))}
          </div>

          {report && (
            <p className="text-[12px] text-[var(--ink-2)] leading-relaxed bg-[var(--paper)] p-4 rounded-2xl m-0 border border-[var(--line)] shadow-2xs">
              识别到 <strong className="text-[var(--ink)]">{report.channels}</strong> / 15 个口型通道（
              {SCHEME_LABEL[report.scheme]}）
              {report.channels === 0 && report.found.length === 0 && (
                <> —— 该 3D 模型未检测到 morph target 口型绑定。</>
              )}
              {report.channels === 0 && report.found.length > 0 && (
                <>
                  {" "}
                  —— 检测到 {report.found.length} 个 morph target，头几个为：
                  <code className="bg-[var(--surface)] px-1.5 py-0.5 rounded border border-[var(--line)]">
                    {report.found.slice(0, 8).join("、")}
                  </code>
                </>
              )}
            </p>
          )}
        </div>
      )}

      {/* 4. 神经写实 */}
      {activeTag === "neural" && (
        <div className="flex flex-col gap-4">
          <div className="p-4 bg-[var(--paper)] border border-[var(--line)] rounded-2xl flex flex-col gap-2 shadow-2xs">
            <div className="flex items-center justify-between flex-wrap gap-2">
              <div className="text-[13.5px] font-bold text-[var(--ink)] flex items-center gap-2">
                <span>4D 神经流式超写实数字人</span>
                <span className="text-[10px] bg-emerald-50 text-emerald-600 font-bold px-2 py-0.5 rounded-full border border-emerald-200/60">
                  GPU 加速
                </span>
                <span className="text-[10px] bg-sky-50 text-sky-600 font-medium px-2 py-0.5 rounded-full border border-sky-200/60">
                  Rust Core 自动守护
                </span>
              </div>
              <span className="text-[11.5px] text-[var(--celadon)] font-semibold">
                30 FPS · 毫秒级唇形重绘
              </span>
            </div>
            <p className="text-[12px] text-[var(--ink-2)] leading-relaxed m-0">
              基于音频驱动的神经流式重绘技术（MuseTalk / LivePortrait）。Anima 启动时由 Rust 后端自动拉起 Python 推理 Sidecar。
            </p>
          </div>

          <div className="flex flex-col gap-2.5">
            <div className="flex items-center justify-between pt-1">
              <span className="text-[13.5px] font-bold text-[var(--ink)]">
                已检测到底模素材 (avatars/neural/)
              </span>
              <span className="text-[11px] text-[var(--ink-3)]">
                共 {neuralModels.length} 个素材底模
              </span>
            </div>

            {neuralModels.length === 0 ? (
              <div className="p-6 bg-[var(--paper)] border border-[var(--line)] rounded-2xl text-center flex flex-col items-center gap-2 shadow-2xs">
                <span className="text-3xl">📂</span>
                <div className="text-[13px] font-bold text-[var(--ink)]">
                  暂未检测到神经底模素材
                </div>
                <p className="text-[12px] text-[var(--ink-2)] max-w-sm m-0">
                  点击下方「📂 打开素材目录 (访达)」，在 <code className="bg-[var(--surface)] px-1.5 py-0.5 rounded border border-[var(--line)]">avatars/neural/</code> 放入任意肖像照片 (.jpg/.png) 或视频 (.mp4)，然后点击重新扫描即可！
                </p>
              </div>
            ) : (
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-2.5">
                {neuralModels.map((m) => {
                  const isSelected = modelId === m.id;
                  const isVideo =
                    m.id.toLowerCase().endsWith(".mp4") ||
                    m.id.toLowerCase().endsWith(".mov") ||
                    m.id.toLowerCase().endsWith(".webm");
                  return (
                    <button
                      disabled={locked}
                      key={m.id}
                      type="button"
                      className={`flex items-center justify-between border rounded-2xl p-3.5 text-left transition-all cursor-pointer shadow-2xs ${
                        isSelected
                          ? "border-[var(--celadon)] ring-1 ring-[var(--celadon)]/25 bg-[var(--paper)]"
                          : "border-[var(--line)] bg-[var(--paper)] hover:border-[var(--ink-3)]/50"
                      } disabled:opacity-50`}
                      onClick={() => {
                        pick(m.id);
                      }}
                    >
                      <div className="flex items-center gap-2.5 min-w-0">
                        <span className="text-2xl shrink-0">
                          {isVideo ? "🎬" : "🖼️"}
                        </span>
                        <div className="flex flex-col gap-0.5 min-w-0">
                          <span className="text-[13px] font-bold text-[var(--ink)] truncate">
                            {m.name}
                          </span>
                          <span className="text-[10.5px] text-[var(--ink-3)] font-mono">
                            {isVideo ? "视频底模" : "单图肖像"} · {(m.bytes / 1048576).toFixed(1)} MB
                          </span>
                        </div>
                      </div>
                      {isSelected && (
                        <span className="text-[11px] font-semibold text-[var(--celadon)] bg-[var(--celadon-2)] px-2 py-0.5 rounded-full border border-[var(--celadon)]/30 shrink-0">
                          当前使用
                        </span>
                      )}
                    </button>
                  );
                })}
              </div>
            )}
          </div>
        </div>
      )}

      {/* 底部工具条 */}
      <div className="flex flex-col gap-3 pt-4 border-t border-[var(--line)]/60">
        <div className="flex items-center justify-between gap-3 flex-wrap">
          <div className="flex items-center gap-2">
            <button
              type="button"
              className="px-3.5 py-1.5 text-[12px] bg-[var(--surface-2)] hover:bg-[var(--surface-3)] border border-[var(--line)] rounded-xl text-[var(--ink)] transition-colors shrink-0 cursor-pointer flex items-center gap-1.5 shadow-2xs font-semibold"
              onClick={() => void openAvatarModelsDir()}
            >
              📂 打开素材目录 (访达)
            </button>
            <button
              disabled={locked}
              type="button"
              className="px-3.5 py-1.5 text-[12px] bg-[var(--surface-2)] hover:bg-[var(--surface-3)] border border-[var(--line)] rounded-xl text-[var(--ink)] transition-colors disabled:opacity-50 shrink-0 cursor-pointer shadow-2xs font-semibold"
              onClick={() => void reload()}
            >
              🔄 重新扫描
            </button>
          </div>

          <button
            type="button"
            className="px-4 py-1.5 text-[12px] bg-[var(--celadon)] hover:opacity-90 text-white rounded-xl font-semibold transition-all disabled:opacity-50 shrink-0 cursor-pointer shadow-2xs"
            disabled={locked}
            onClick={() => selfTest()}
          >
            🎙️ 试一句口型
          </button>
        </div>

        <MouthMonitor />

        {message && (
          <div className="text-[12px] text-[var(--ink)] bg-[var(--paper)] border border-[var(--celadon)] shadow-lg px-4 py-2 rounded-xl">
            {message}
          </div>
        )}
      </div>
    </div>
  );
}

function MouthMonitor() {
  const barsRef = useRef<Array<HTMLElement | null>>([]);
  const [open, setOpen] = useState(false);

  useEffect(() => {
    if (!open) return;
    let raf = 0;
    const tick = () => {
      const w = mouth.sample();
      VISEMES.forEach((v, i) => {
        const el = barsRef.current[i];
        if (el) el.style.height = `${Math.max(2, w[v] * 100)}%`;
      });
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [open]);

  return (
    <div className="flex flex-col gap-2">
      <button
        type="button"
        className="px-3.5 py-1.5 text-[12px] bg-[var(--paper)] border border-[var(--line)] rounded-xl text-[var(--ink-2)] hover:text-[var(--ink)] hover:border-[var(--ink-3)] transition-colors self-start cursor-pointer shadow-2xs"
        onClick={() => setOpen((o) => !o)}
      >
        {open ? "收起实时口型监视器" : "📊 展开实时口型监视器 (15 通道)"}
      </button>
      {open && (
        <div className="flex flex-col gap-2 p-4 bg-[var(--paper)] border border-[var(--line)] rounded-2xl shadow-2xs mt-1">
          <p className="text-[11.5px] text-[var(--ink-3)] leading-tight m-0">
            说话或测试时柱状图跳动说明音频/文本到 Viseme 链路正常，模型若不动则需检查口型命名。
          </p>
          <div className="flex flex-col bg-[var(--surface)] border border-[var(--line)] rounded-xl p-3 shadow-2xs">
            <div className="flex items-end justify-between h-8 gap-[2px]" aria-hidden="true">
              {VISEMES.map((v, i) => (
                <i
                  key={v}
                  ref={(el) => {
                    barsRef.current[i] = el;
                  }}
                  className="flex-1 bg-[var(--celadon)] rounded-t-[1px] transition-all duration-75 min-w-[2px]"
                  style={{ height: "2%" }}
                />
              ))}
            </div>
            <div className="flex justify-between mt-1 border-t border-dashed border-[var(--line)] pt-1" aria-hidden="true">
              {VISEMES.map((v) => (
                <span key={v} className="text-[9px] font-mono text-[var(--ink-3)] flex-1 text-center scale-90 origin-top">
                  {v}
                </span>
              ))}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
