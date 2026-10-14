import { useEffect, useState } from "react";
import {
  cinemaSaveModelSettings,
  type CinemaProject,
  type ModelSettings,
  type PromptRoute,
} from "../../../api/cinema";
import { studioCapabilities, type CapabilityCatalog } from "../../../api/studio";
import { DEFAULT_RENDER } from "../storyboard/renderOptions";
import { CombatPresetEditor } from "./CombatPresetEditor";

/**
 * 生成设置。项目级，管「用哪个引擎、各类资产什么画幅」。
 *
 * 结构照 LumenX 的 `SeriesModelSettingsModal`（MIT，Copyright 2026 Alibaba）：
 * 按用途分段（资产 = 文生图、分镜 = 图生图、动效 = 出片），
 * 每段先选引擎，资产那段再给三类各自的画幅。
 *
 * # 画幅选项来自后端
 *
 * **前端一个比例都不许自己写。** 后端的 `aspect_options()` 是从实测过的
 * 尺寸表里推出来的，所以这里永远不会出现引擎产不出的比例。
 * LumenX 就是在前端写死了 9:16 / 3:4 / 1:1 / 4:3 / 16:9 五档（云端模型的
 * 习惯），接到本机之后出片直接报 `WT resolution must be 384p or 512p` ——
 * 界面上能选、送到引擎报错。
 */
export function ModelSettingsDialog({
  project,
  onClose,
  onSaved,
}: {
  project: CinemaProject;
  onClose: () => void;
  onSaved: (p: CinemaProject) => void;
}) {
  const [cat, setCat] = useState<CapabilityCatalog | null>(null);
  const [draft, setDraft] = useState<ModelSettings>(
    () =>
      project.model_settings ?? {
        t2i_engine: "zimage",
        i2i_engine: "zimage",
        video_engine: "h3",
        character_aspect: "portrait",
        scene_aspect: "landscape",
        prop_aspect: "square",
        prompt_route: "b",
      },
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    studioCapabilities().then(setCat).catch((e) => setError(String(e)));
  }, []);

  const set = <K extends keyof ModelSettings>(k: K, v: ModelSettings[K]) =>
    setDraft((d) => ({ ...d, [k]: v }));

  /**
   * 出片设置里的「声音与字幕」。**和分镜页出片弹窗改的是同一份**（`model_settings.render`），
   * 两处都能改是故意的：出片前在弹窗里临时改，平时在这儿定。
   */
  const render = draft.render ?? DEFAULT_RENDER;
  const setRender = <K extends keyof typeof render>(k: K, v: (typeof render)[K]) =>
    setDraft((d) => ({ ...d, render: { ...(d.render ?? DEFAULT_RENDER), [k]: v } }));

  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      onSaved(await cinemaSaveModelSettings(project.id, draft));
      onClose();
    } catch (e) {
      // 后端会拒绝产不出的画幅 —— 原样显示，那句话里有是哪一类、哪个值
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  // **只列 `project_settings` 为真的。** 用户明确不要 Pony 进工作区。
  const images = cat?.engines.filter((e) => e.kind === "image" && e.project_settings) ?? [];
  // 资产形象图是从文字出的，那时还没有图可参考 —— 参考生图引擎只进首帧那一栏。
  const assetImages = images.filter((e) => e.reference_slots === 0);
  const videos = cat?.engines.filter((e) => e.kind === "video" && e.project_settings) ?? [];
  const frameSlots = images.find((e) => e.id === draft.i2i_engine)?.reference_slots ?? 0;

  return (
    <div className="studio-scrim fixed inset-0 z-50 flex items-center justify-center">
      <div
        className="studio-modal flex max-h-[86vh] w-[620px] max-w-[94vw] flex-col overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="px-6 pb-4 pt-5" style={{ borderBottom: "1px solid var(--studio-border-subtle)" }}>
          <h2 className="studio-display text-[17px]">生成设置</h2>
          <p className="mt-0.5 text-[12px]" style={{ color: "var(--studio-text-muted)" }}>
            这个项目用哪个引擎出图出片，各类资产用什么画幅
          </p>
        </header>

        <div className="studio-scroll min-h-0 flex-1 overflow-y-auto px-6 py-5">
          {!cat && !error && (
            <p className="text-[12px]" style={{ color: "var(--studio-text-muted)" }}>
              读取引擎能力…
            </p>
          )}

          {cat && (
            <>
              <Section title="资产" hint="角色 / 场景 / 道具的形象图（文生图）">
                <EnginePicker
                  engines={assetImages}
                  value={draft.t2i_engine}
                  onPick={(id) => set("t2i_engine", id)}
                />
                <div className="mt-4 grid grid-cols-3 gap-3">
                  <AspectColumn
                    label="角色"
                    options={cat.aspects}
                    value={draft.character_aspect}
                    onPick={(id) => set("character_aspect", id)}
                  />
                  <AspectColumn
                    label="场景"
                    options={cat.aspects}
                    value={draft.scene_aspect}
                    onPick={(id) => set("scene_aspect", id)}
                  />
                  <AspectColumn
                    label="道具"
                    options={cat.aspects}
                    value={draft.prop_aspect}
                    onPick={(id) => set("prop_aspect", id)}
                  />
                </div>
              </Section>

              <Section title="分镜" hint="镜头首帧生成模型">
                <EnginePicker
                  engines={images}
                  value={draft.i2i_engine}
                  onPick={(id) => set("i2i_engine", id)}
                />
                <p className="mt-2 text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
                  {frameSlots > 0
                    ? `当前模型用资产图出首帧：场景 + 人物最多 ${frameSlots} 张（场景在前，说话的人优先），比文生图慢 4~6 倍。`
                    : draft.i2i_engine !== "sdxl" && "当前模型按资产文字描述出图，不使用参考图锁定人物身份。"}
                  分镜画幅跟项目走（当前 {project.aspect_ratio}），不在这里单独设
                  —— 两处各存一份迟早对不上。
                </p>
              </Section>

              {/* 两条线路物理隔离，可以随时切，切换不会覆盖另一条已经合成好的提示词。
                  见 docs/plans/2026-09-13-分镜重构方案.md 的「两条提示词线路」。 */}
              <Section title="提示词线路" hint="镜头信息怎么写成发给视频模型的提示词">
                <div className="grid grid-cols-2 gap-2">
                  {ROUTES.map((r) => {
                    const on = (draft.prompt_route ?? "b") === r.id;
                    return (
                      <button
                        key={r.id}
                        type="button"
                        onClick={() => set("prompt_route", r.id)}
                        className="rounded-xl px-3 py-2.5 text-left"
                        style={{
                          background: on
                            ? "color-mix(in srgb, var(--studio-primary) 12%, transparent)"
                            : "var(--studio-bg-inset)",
                          boxShadow: `0 0 0 ${on ? 1.5 : 1}px ${
                            on ? "var(--studio-primary)" : "var(--studio-border-subtle)"
                          }`,
                        }}
                      >
                        <span className="block text-[12.5px] font-medium">{r.label}</span>
                        <span className="mt-0.5 block text-[10.5px]" style={{ color: "var(--studio-text-muted)" }}>
                          {r.hint}
                        </span>
                      </button>
                    );
                  })}
                </div>
                <p className="mt-2 text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
                  两条线路的提示词分开保存，切换不会覆盖另一条。出片用当前选中的线路。
                </p>
              </Section>

              <Section title="动效" hint="首帧 / 参考图 → 视频">
                <EnginePicker
                  engines={videos}
                  value={draft.video_engine}
                  onPick={(id) => set("video_engine", id)}
                />
              </Section>

              {/* 采样方法 + 战斗 LoRA 预设：只 H3。打斗镜（镜头详情里选了打斗类型）才挂预设 */}
              <Section title="H3 采样与打斗" hint="采样方法整个项目统一；战斗 LoRA 只挂在选了打斗类型的镜头上">
                <div className="mb-2 grid grid-cols-2 gap-2">
                  <Toggle
                    on={(render.sampling ?? "euler") === "euler"}
                    onClick={() => setRender("sampling", undefined)}
                    label="Euler"
                    hint="默认"
                  />
                  <Toggle
                    on={render.sampling === "res_multistep"}
                    onClick={() => setRender("sampling", "res_multistep")}
                    label="RES 多步"
                    hint="Combat 类 LoRA 作者推荐；精细档不生效"
                  />
                </div>
                <CombatPresetEditor value={render.combat} onChange={(next) => setRender("combat", next)} />
              </Section>

              {/* **硬字幕是出片时烧进画面的，事后改不了。**
                  关掉之后画面干净，字幕走合成时另出的那份 SRT（软字幕）：
                  播放器能挂、剪映 / 达芬奇里能改字体和时间。 */}
              <Section title="声音与字幕" hint="出片时的声音和字幕，出片弹窗里也能临时改">
                <div className="grid grid-cols-2 gap-2">
                  <Toggle
                    on={render.generate_audio}
                    onClick={() => setRender("generate_audio", !render.generate_audio)}
                    label={render.generate_audio ? "生成声音 · 开" : "生成声音 · 关"}
                    hint="模型按台词出中文配音，和口型对上"
                  />
                  <Toggle
                    on={render.burn_subtitles}
                    onClick={() => setRender("burn_subtitles", !render.burn_subtitles)}
                    label={render.burn_subtitles ? "烧入字幕 · 开" : "烧入字幕 · 关"}
                    hint={render.burn_subtitles ? "字幕烧进画面，后期改不了" : "画面干净；字幕在合成时另出一份 SRT"}
                  />
                </div>
                <p className="mt-2 text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
                  改了只影响之后出的镜头 —— 已经出好的片子里字幕已经烧死了，要去掉得重出那几镜。
                </p>
              </Section>
            </>
          )}

          {error && (
            <p className="mt-4 text-[12px]" style={{ color: "var(--studio-failed-fg)" }}>
              {error}
            </p>
          )}
        </div>

        <footer
          className="flex items-center justify-end gap-2 px-6 py-4"
          style={{ borderTop: "1px solid var(--studio-border-subtle)" }}
        >
          <button
            type="button"
            onClick={onClose}
            className="rounded-lg px-3.5 py-2 text-[12px]"
            style={{ color: "var(--studio-text-2)" }}
          >
            取消
          </button>
          <button
            type="button"
            disabled={busy || !cat}
            onClick={() => void save()}
            className="rounded-lg px-4 py-2 text-[12.5px] font-medium"
            style={{
              background: "var(--studio-primary)",
              color: "var(--studio-on-primary)",
              opacity: busy || !cat ? 0.6 : 1,
            }}
          >
            {busy ? "保存中…" : "保存"}
          </button>
        </footer>
      </div>
    </div>
  );
}

/** 提示词线路。文字说给普通用户听：差别是「怎么写」和「要不要多一次 AI 整理」。 */
const ROUTES: { id: PromptRoute; label: string; hint: string }[] = [
  { id: "a", label: "线路 A · 中文结构", hint: "镜头信息直接填进中文模板，快，可直接改" },
  { id: "b", label: "线路 B · H3 官方格式", hint: "按视频模型官方格式整理，多一次 AI 整理" },
];

function Toggle({ on, onClick, label, hint }: { on: boolean; onClick: () => void; label: string; hint: string }) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={on}
      className="rounded-xl px-3 py-2.5 text-left"
      style={{
        background: on ? "color-mix(in srgb, var(--studio-primary) 12%, transparent)" : "var(--studio-bg-inset)",
        boxShadow: `0 0 0 ${on ? 1.5 : 1}px ${on ? "var(--studio-primary)" : "var(--studio-border-subtle)"}`,
      }}
    >
      <span className="block text-[12.5px] font-medium">{label}</span>
      <span className="mt-0.5 block text-[10.5px]" style={{ color: "var(--studio-text-muted)" }}>
        {hint}
      </span>
    </button>
  );
}

function Section({
  title,
  hint,
  children,
}: {
  title: string;
  hint: string;
  children: React.ReactNode;
}) {
  return (
    <section className="mb-6">
      <div className="mb-2.5 flex items-baseline gap-2">
        <h3 className="text-[13px] font-medium">{title}</h3>
        <span className="text-[11.5px]" style={{ color: "var(--studio-text-muted)" }}>
          {hint}
        </span>
      </div>
      {children}
    </section>
  );
}

/**
 * 引擎卡片。**耗时和限制写在卡片上** —— 选引擎时最该知道的就是
 * 「这个多久出一张」和「它支不支持负向提示词」，事后才发现代价太大。
 */
function EnginePicker({
  engines,
  value,
  onPick,
}: {
  engines: CapabilityCatalog["engines"];
  value: string;
  onPick: (id: string) => void;
}) {
  return (
    <div className="grid grid-cols-2 gap-2">
      {engines.map((e) => {
        const on = e.id === value;
        return (
          <button
            key={e.id}
            type="button"
            onClick={() => onPick(e.id)}
            className="rounded-xl px-3 py-2.5 text-left"
            style={{
              background: on
                ? "color-mix(in srgb, var(--studio-primary) 12%, transparent)"
                : "var(--studio-bg-inset)",
              boxShadow: `0 0 0 ${on ? 1.5 : 1}px ${
                on ? "var(--studio-primary)" : "var(--studio-border-subtle)"
              }`,
            }}
          >
            <span className="block text-[12.5px] font-medium">{e.label}</span>
            <span
              className="mt-0.5 block font-mono text-[10px]"
              style={{ color: "var(--studio-text-muted)" }}
            >
              约 {e.secs}s / 张{e.negative_prompt ? "" : " · 无负向"}
            </span>
          </button>
        );
      })}
    </div>
  );
}

function AspectColumn({
  label,
  options,
  value,
  onPick,
}: {
  label: string;
  options: CapabilityCatalog["aspects"];
  value: string;
  onPick: (id: string) => void;
}) {
  return (
    <div>
      <p className="studio-eyebrow mb-1.5">{label}</p>
      <div className="flex flex-col gap-1">
        {options.map((a) => {
          const on = a.id === value;
          return (
            <button
              key={a.id}
              type="button"
              onClick={() => onPick(a.id)}
              className="rounded-lg px-2.5 py-1.5 text-left font-mono text-[11px]"
              style={{
                background: on
                  ? "color-mix(in srgb, var(--studio-primary) 12%, transparent)"
                  : "var(--studio-bg-inset)",
                color: on ? "var(--studio-text)" : "var(--studio-text-2)",
                boxShadow: on
                  ? "0 0 0 1px color-mix(in srgb, var(--studio-primary) 45%, transparent)"
                  : "none",
              }}
            >
              {a.label}
            </button>
          );
        })}
      </div>
    </div>
  );
}
