import { useEffect, useRef, useState, type ReactNode } from "react";
import { OperationPanelShell } from "../../ui/OperationPanelShell";
import { ArrowUp, ChevronDown, Loader2, Maximize2, Music, SlidersHorizontal, Upload } from "../../ui/icons";
import { ComposingTextarea } from "../../../components/ComposingInput";
import type { TtsEngineConfig, TtsVoiceInfo } from "../../../api/voice";
import type { AudioNodeData } from "../../types";
import { canClone } from "../../utils/audioChain";

/**
 * 音频节点的**设置卡**（选中节点时挂在下方，和视频 / 图片节点的操作面板同一个外壳）。
 *
 * 用户 2026-10-13：「所有的设置都在设置卡里，不要在节点里放那么多按钮。包括模型和音频设置。」—— 照 LibTV：
 * 上面一行是参考（左边连进来的音频 = 声音复制），中间是要念的话，底栏是模型 / 音色 / 音频格式 / 高级设置 / 上传 / 生成。
 * 节点本身只放播放器。
 */
export const SAMPLE_RATES = [8000, 16000, 24000, 48000] as const;
export const FORMATS = ["wav", "mp3", "pcm", "ogg_opus"] as const;

export interface AudioRef {
  path: string;
  name: string;
}

export function AudioOperationPanel({
  data, upstreamText, reference, referenceEmpty, engines, voices, isGenerating, onChange, onGenerate, onUpload,
}: {
  data: AudioNodeData;
  upstreamText: string;
  reference: AudioRef | null;
  referenceEmpty: boolean;
  engines: { list: TtsEngineConfig[]; selected: string } | null;
  voices: TtsVoiceInfo[] | null;
  isGenerating: boolean;
  onChange: (patch: Partial<AudioNodeData>) => void;
  onGenerate: () => void;
  onUpload: (file: File) => void;
}) {
  const fileInput = useRef<HTMLInputElement>(null);
  const engineId = data.engineId || engines?.selected || "";
  const engine = engines?.list.find((e) => e.id === engineId);
  const cloneBlocked = !!reference && !!engine && !canClone(engine.kind);
  const clone = data.cloneUsed && reference && data.cloneUsed.ref_path === reference.path ? data.cloneUsed : undefined;
  const text = (upstreamText || data.text || "").trim();
  const canGenerate = !!text && !isGenerating && !cloneBlocked && !referenceEmpty;
  const rate = data.sampleRate ?? 24000;
  const format = data.format ?? "wav";
  const advancedChanged = (data.speed ?? 1) !== 1 || (data.pitch ?? 0) !== 0 || (data.volume ?? 1) !== 1;

  return (
    <OperationPanelShell widthClass="w-[560px]">
      {({ toggleExpand }) => (
        <div className="relative flex flex-col gap-2.5">
          {/* 参考：左边连进来的音频 = 声音复制（照 LibTV 的「参考」那一行） */}
          <div className="flex items-start justify-between gap-2">
            <div className="flex min-w-0 flex-1 flex-col gap-1.5">
              {reference ? (
                <div className="flex items-center gap-2">
                  <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-white/10" title={reference.path}>
                    <Music className="h-4 w-4 text-white/80" />
                  </span>
                  <div className="min-w-0 text-[11px] leading-snug">
                    <div className="truncate text-white/85">声音复制 · {reference.name}</div>
                    <div className="text-white/40">
                      {clone
                        ? <>参考用第 {clone.from_secs.toFixed(1)}~{clone.to_secs.toFixed(1)} 秒{clone.padded && <span className="text-amber-300"> · 不到 3 秒，补了静音，换段长的更像</span>}</>
                        : "生成时自动挑最清楚的 8 秒当参考，并转写它说的话"}
                    </div>
                  </div>
                </div>
              ) : referenceEmpty ? (
                <p className="text-[11px] text-amber-300/85">左边连着的音频节点还没有音频 —— 先在它上面上传一段或生成一遍</p>
              ) : (
                <p className="text-[11px] text-white/35">左边连一段音频进来 = 声音复制（用那段的嗓子念）</p>
              )}
              {clone && (
                // 参考说的话：Whisper 转的，可以改。错了会「音色像但吐字乱」
                <label className="flex items-start gap-2 text-[11px]">
                  <span className="shrink-0 pt-0.5 text-white/40">参考原话</span>
                  <ComposingTextarea
                    rows={1}
                    value={data.refText ?? clone.ref_text}
                    onCommit={(v) => onChange({ refText: v })}
                    onKeyDown={(e) => e.stopPropagation()}
                    title="参考那段说的话（自动转写）。认错了就改，改完再生成 —— 错字会让吐字变乱"
                    className="nodrag nopan nowheel w-full resize-none rounded-md bg-white/5 px-2 py-0.5 text-white/85 outline-none focus:bg-white/10"
                  />
                </label>
              )}
            </div>
            <button type="button" onClick={toggleExpand} title="最大化输入面板"
              className="h-7 w-7 shrink-0 flex items-center justify-center rounded-md text-white/40 hover:text-white hover:bg-white/10 transition-colors">
              <Maximize2 className="h-3.5 w-3.5" />
            </button>
          </div>

          {/* 要念的话 */}
          <div className="relative flex flex-col gap-1.5 py-1">
            {upstreamText ? (
              <div className="flex items-center gap-1.5 text-xs text-white/50">
                <span className="font-medium text-white/70">↳ 念上游的词：</span>
                <span className="truncate text-white/90">"{upstreamText}"</span>
              </div>
            ) : (
              <ComposingTextarea
                rows={3}
                value={data.text ?? ""}
                onCommit={(v) => onChange({ text: v })}
                onKeyDown={(e) => e.stopPropagation()}
                placeholder={reference ? "用这把嗓子念的话" : "要念的话（只放音频就不用填）"}
                className="nodrag nopan nowheel w-full resize-none border-0 bg-transparent p-0 text-sm leading-relaxed text-white outline-none"
              />
            )}
          </div>

          {cloneBlocked && <p className="text-[11px] text-amber-300">「{engine?.label}」不会复制声音 —— 换成 GPT-SoVITS、CosyVoice2 或 IndexTTS2</p>}

          {/* 底栏：模型 / 音色 / 音频格式 / 高级设置 / 上传 / 生成 */}
          <div className="flex items-center justify-between gap-1.5 pt-1 text-xs text-white/70">
            <div className="flex min-w-0 flex-nowrap items-center gap-2">
              {engines && (
                <Picker
                  label={engine?.label ?? engineId}
                  title="合成引擎（空 = 跟设置）。连了参考音频时只有能复制声音的三家可选"
                >
                  {(close) => (
                    <>
                      <PickItem active={!data.engineId} onClick={() => { onChange({ engineId: undefined }); close(); }}>
                        跟设置（{engines.list.find((e) => e.id === engines.selected)?.label ?? engines.selected}）
                      </PickItem>
                      {engines.list.map((e) => (
                        <PickItem key={e.id} active={data.engineId === e.id} disabled={!!reference && !canClone(e.kind)}
                          onClick={() => { onChange({ engineId: e.id }); close(); }}>
                          {e.label}{canClone(e.kind) ? <span className="ml-1 text-[10px] text-emerald-300/80">能复制声音</span> : null}
                        </PickItem>
                      ))}
                    </>
                  )}
                </Picker>
              )}

              {!reference && voices && voices.length > 0 && (
                <Picker label={data.voice ? (voices.find((v) => v.short_name === data.voice)?.name || data.voice) : "默认音色"} title="音色（空 = 这个引擎的默认音色）">
                  {(close) => (
                    <div className="max-h-64 overflow-y-auto">
                      <PickItem active={!data.voice} onClick={() => { onChange({ voice: undefined }); close(); }}>默认音色</PickItem>
                      {/* 值是 short_name —— name 是给人看的长名，填错会静默退回默认 */}
                      {voices.map((v) => (
                        <PickItem key={v.short_name} active={data.voice === v.short_name} onClick={() => { onChange({ voice: v.short_name }); close(); }}>
                          {v.name || v.short_name}{v.gender ? ` · ${v.gender}` : ""}
                        </PickItem>
                      ))}
                    </div>
                  )}
                </Picker>
              )}

              {/* 音频格式（照 LibTV「中文 · 24k · wav」）。语种不做：几家引擎都按文字自己判 */}
              <Picker label={`${rate / 1000}k · ${format}`} title="采样率和输出格式">
                {() => (
                  <div className="flex w-56 flex-col gap-3 p-1">
                    <Segment title="采样率" options={SAMPLE_RATES.map((r) => [r, `${r / 1000}k`] as const)} value={rate} onPick={(r) => onChange({ sampleRate: r })} />
                    <Segment title="输出格式" options={FORMATS.map((f) => [f, f] as const)} value={format} onPick={(f) => onChange({ format: f })} />
                    {format === "pcm" && <p className="text-[10.5px] leading-snug text-amber-300/85">pcm 是裸数据（16 位单声道、没有文件头）：节点里放不了，也不能当 H3 参考音频，只适合导出</p>}
                  </div>
                )}
              </Picker>

              {/* 高级设置：语速 / 声调 / 音量（照 LibTV 的滑条） */}
              <Picker icon={<SlidersHorizontal className={`h-3.5 w-3.5 ${advancedChanged ? "text-white" : "text-white/60"}`} />} title="高级设置：语速、声调、音量">
                {() => (
                  <div className="flex w-64 flex-col gap-2.5 p-1">
                    <div className="text-[11px] font-medium text-zinc-400">高级设置</div>
                    {([
                      ["语速", "speed", 0.5, 2, 0.05, 1, (v: number) => v.toFixed(2)],
                      ["声调", "pitch", -12, 12, 1, 0, (v: number) => (v > 0 ? `+${v}` : `${v}`)],
                      ["音量", "volume", 0, 2, 0.05, 1, (v: number) => v.toFixed(2)],
                    ] as const).map(([label, key, min, max, step, def, fmt]) => {
                      const v = (data[key] as number | undefined) ?? def;
                      return (
                        <label key={key} className="flex items-center gap-2 text-[11px]">
                          <span className="w-8 text-white/60">{label}</span>
                          <input type="range" min={min} max={max} step={step} value={v} title="双击恢复默认"
                            onChange={(e) => onChange({ [key]: Number(e.target.value) })}
                            onDoubleClick={() => onChange({ [key]: def })}
                            className="nodrag nopan flex-1" />
                          <span className="w-10 rounded bg-white/5 py-0.5 text-center tabular-nums text-white/80">{fmt(v)}</span>
                        </label>
                      );
                    })}
                    <p className="text-[10px] text-white/35">声调单位是半音。双击滑条恢复默认</p>
                  </div>
                )}
              </Picker>

              <button type="button" onClick={() => fileInput.current?.click()} disabled={!!data.referenceLoading}
                title={data.uploaded ? "换一段音频" : "上传一段音频（不生成，直接当素材 / 参考）"}
                className="h-7 w-7 shrink-0 flex items-center justify-center rounded-md text-white/60 hover:text-white hover:bg-white/10 transition-colors disabled:opacity-40">
                {data.referenceLoading ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Upload className="h-3.5 w-3.5" />}
              </button>
              <input ref={fileInput} type="file" accept="audio/*,.mp3,.wav,.m4a,.flac,.ogg" className="nodrag nopan hidden"
                onChange={(e) => { const f = e.target.files?.[0]; e.target.value = ""; if (f) onUpload(f); }} />
            </div>

            <button type="button" disabled={!canGenerate} onClick={onGenerate}
              title={isGenerating ? "生成中…" : canGenerate ? (reference ? "用参考的嗓子生成" : "生成语音") : "先写要念的话（或接上游提示词）"}
              className={`h-7 w-7 shrink-0 flex items-center justify-center rounded-full transition-all ${
                canGenerate ? "bg-white text-black shadow-lg shadow-white/10 hover:scale-105 hover:bg-zinc-200" : "bg-white/10 text-white/30 cursor-not-allowed"}`}>
              {isGenerating ? <Loader2 className="h-3.5 w-3.5 animate-spin text-black" /> : <ArrowUp className="h-3.5 w-3.5 stroke-[2.5]" />}
            </button>
          </div>
        </div>
      )}
    </OperationPanelShell>
  );
}

/** 底栏的一个下拉：按钮 + 往上弹的卡片，点外面收起（和视频设置卡同一套样式） */
function Picker({ label, icon, title, children }: { label?: string; icon?: ReactNode; title: string; children: (close: () => void) => ReactNode }) {
  const [open, setOpen] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const out = (e: MouseEvent) => { if (box.current && !box.current.contains(e.target as Node)) setOpen(false); };
    document.addEventListener("mousedown", out);
    return () => document.removeEventListener("mousedown", out);
  }, [open]);
  return (
    <div ref={box} className="relative flex items-center">
      <button type="button" title={title} onClick={() => setOpen((v) => !v)}
        className={`nodrag nopan h-7 flex items-center gap-1 px-1 text-xs transition-colors ${icon ? "w-7 justify-center rounded-md hover:bg-white/10" : ""} ${open ? "text-white" : "text-white/80 hover:text-white"}`}>
        {icon ?? <>
          <span className="max-w-[140px] truncate">{label}</span>
          <ChevronDown className={`h-3 w-3 text-white/40 transition-transform ${open ? "rotate-180 text-white/70" : ""}`} />
        </>}
      </button>
      {open && (
        <div className="nodrag nopan absolute bottom-full left-0 z-50 mb-2 min-w-[180px] rounded-2xl border border-white/15 bg-[#18181b]/98 p-2 text-xs text-zinc-300 shadow-2xl backdrop-blur-2xl"
          onClick={(e) => e.stopPropagation()}>
          {children(() => setOpen(false))}
        </div>
      )}
    </div>
  );
}

function PickItem({ active, disabled, onClick, children }: { active: boolean; disabled?: boolean; onClick: () => void; children: ReactNode }) {
  return (
    <button type="button" disabled={disabled} onClick={onClick}
      className={`flex w-full items-center whitespace-nowrap rounded-lg px-2.5 py-1.5 text-left text-[12px] transition-colors disabled:cursor-not-allowed disabled:opacity-35 ${
        active ? "bg-white/10 text-white" : "text-zinc-300 hover:bg-white/5 hover:text-white"}`}>
      {children}
    </button>
  );
}

function Segment<T extends string | number>({ title, options, value, onPick }: { title: string; options: readonly (readonly [T, string])[]; value: T; onPick: (v: T) => void }) {
  return (
    <div>
      <div className="mb-1.5 text-[11px] font-medium text-zinc-400">{title}</div>
      <div className="grid grid-cols-2 gap-1.5">
        {options.map(([v, label]) => (
          <button key={String(v)} type="button" onClick={() => onPick(v)}
            className={`rounded-xl py-1.5 text-[11.5px] transition-all ${value === v
              ? "border border-white/90 bg-white/10 text-white"
              : "border border-white/5 bg-zinc-800/40 text-zinc-400 hover:border-white/20 hover:text-zinc-200"}`}>
            {label}
          </button>
        ))}
      </div>
    </div>
  );
}
