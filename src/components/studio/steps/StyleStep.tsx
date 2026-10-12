/**
 * Step 2 · 风格定调。
 *
 * 选一个预设（或自己写），之后这个项目**每一次出图和出视频**都会带上它。
 * 这是 LumenX 那条流程里最值得借的一块 —— 定一次，后面全省。
 *
 * # 两处按我们的实情改
 *
 * 1. **缩略图是我们自己的引擎出的。** 参考项目那 16 张是 wanx/gpt-image
 *    出的，我们的编码器不同，同一段词出来不是同一张画 ——
 *    拿它们当选择器，等于让用户按一张我们复现不出来的画面做选择。
 *    还没出的显示占位，不显示别人的图。
 * 2. **负向词只在一半引擎上有效。** Z-Image 的 cfg=1，
 *    负向被清零。这里明说，而不是给一个看着能用的框。
 */
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  styleAnalyze,
  styleCatalog,
  styleGet,
  styleSave,
  type ArtDirection,
  type StyleCatalog,
  type StylePreset,
  type StyleSuggestion,
} from "../../../api/style";
import { styleCompareList, styleCompareStart } from "../../../api/styleCompare";
import { runTask } from "../toastStore";
import { MediaThumb } from "../MediaThumb";
import type { StepProps } from "./types";

export function StyleStep({ project, episode, onError }: StepProps) {
  const [cat, setCat] = useState<StyleCatalog | null>(null);
  const [art, setArt] = useState<ArtDirection | null>(null);
  const [tab, setTab] = useState<string>("all");
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const [analyzing, setAnalyzing] = useState(false);
  const [suggestions, setSuggestions] = useState<StyleSuggestion[] | null>(null);
  const [customOpen, setCustomOpen] = useState(false);
  // 风格广场：拿本片的镜头出的对照图，preset_id → 绝对路径
  const [mine, setMine] = useState<Record<string, string>>({});
  const [plazaOpen, setPlazaOpen] = useState(false);
  const [plazaPrompt, setPlazaPrompt] = useState("");
  const [customPos, setCustomPos] = useState("");
  const [customNeg, setCustomNeg] = useState("");
  const [shotNotes, setShotNotes] = useState("");
  const [notesNote, setNotesNote] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!project) return;
    try {
      const [c, a] = await Promise.all([styleCatalog(), styleGet(project.id)]);
      void styleCompareList(project.id)
        .then((rows) => setMine(Object.fromEntries(rows)))
        .catch(() => setMine({}));
      setCat(c);
      setArt(a);
      setCustomPos(a.positive_prompt);
      setCustomNeg(a.negative_prompt);
      setCustomOpen(!!a.positive_prompt.trim());
      setShotNotes(a.shot_notes ?? "");
    } catch (e) {
      onError(String(e));
    }
  }, [project, onError]);

  useEffect(() => {
    void load();
  }, [load]);

  const shown = useMemo(
    () => (cat?.presets ?? []).filter((p) => tab === "all" || p.category === tab),
    [cat, tab],
  );

  if (!project) return null;

  const pick = async (next: Partial<ArtDirection>) => {
    if (!art) return;
    setBusy(true);
    setNote(null);
    try {
      const saved = await styleSave(project.id, { ...art, ...next });
      setArt(saved);
      setNote("已定调 —— 之后每次出图/出视频都会带上它");
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  };

  // 镜头与表演偏好单独存、单独提示：它管的是镜头表和提示词怎么写，不是每次出图带的风格词
  const saveNotes = async () => {
    if (!art) return;
    setBusy(true);
    setNotesNote(null);
    try {
      const saved = await styleSave(project.id, { ...art, shot_notes: shotNotes });
      setArt(saved);
      setNotesNote(
        shotNotes.trim()
          ? "已保存 —— 之后生成镜头表、细化时间线、合成提示词都会带上；已经生成过的要重新生成才变"
          : "已清掉 —— 之后生成不再带额外的镜头偏好",
      );
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const analyze = async () => {
    const script = episode?.script_text?.trim();
    if (!script) {
      onError("先去第 1 步写剧本 —— 推荐是读剧本推出来的");
      return;
    }
    setAnalyzing(true);
    onError(null);
    try {
      setSuggestions(
        await runTask(
          {
            title: "正在读剧本推荐风格…",
            body: "通常几十秒，可以先去别的地方。",
            success: (list) => ({ title: "风格推荐好了", body: `推荐了 ${list.length} 种，在「风格」这一步看。` }),
            failure: "推荐风格失败",
          },
          () => styleAnalyze(script),
        ),
      );
    } catch (e) {
      onError(String(e));
    } finally {
      setAnalyzing(false);
    }
  };

  return (
    <div className="flex flex-col gap-4 px-7 py-5">
      {/* 当前定调 */}
      <CurrentBanner cat={cat} art={art} />

      <div className="flex flex-wrap items-center gap-2">
        <div className="flex items-center gap-1 rounded-full p-0.5" style={chip}>
          <TabBtn id="all" label="全部" active={tab} onClick={setTab} />
          {cat?.categories.map((c) => (
            <TabBtn key={c.id} id={c.id} label={c.name_zh} active={tab} onClick={setTab} />
          ))}
        </div>
        <button
          type="button"
          disabled={analyzing}
          onClick={() => void analyze()}
          title="读这一集的剧本，推荐 3 种合适的风格"
          className="rounded-lg px-3.5 py-2 text-[12px]"
          style={{ background: "var(--studio-bg-inset)", color: "var(--studio-text-2)" }}
        >
          {analyzing ? "分析中…" : "分析剧本"}
        </button>
        <button
          type="button"
          onClick={() => setPlazaOpen((v) => !v)}
          title="拿本片自己的一个镜头，把所有风格各出一张 —— 内置缩略图是通用样例，不是你的画面"
          className="rounded-lg px-3.5 py-2 text-[12px]"
          style={{ background: "var(--studio-bg-inset)", color: "var(--studio-text-2)" }}
        >
          用本片试
        </button>
        <button
          type="button"
          onClick={() => setCustomOpen((v) => !v)}
          className="rounded-lg px-3.5 py-2 text-[12px]"
          style={{ background: "var(--studio-bg-inset)", color: "var(--studio-text-2)" }}
        >
          自定义
        </button>
      </div>

      {cat && cat.thumbs_ready < cat.presets.length && (
        <p className="text-[11.5px]" style={{ color: "var(--studio-text-muted)" }}>
          缩略图 {cat.thumbs_ready}/{cat.presets.length} 张已出。缺的那些还没在本机跑过 ——
          我们不显示参考项目的样例图，因为编码器不同，同一段词出来不是同一张画。
          跑 <code>cargo run --bin render-style-thumbs</code> 补齐。
        </p>
      )}

      {plazaOpen && (
        <section
          className="flex flex-col gap-2 rounded-xl p-3.5"
          style={{ background: "var(--studio-bg-elevated)" }}
        >
          <p className="text-[12px]" style={{ color: "var(--studio-text-2)" }}>
            风格广场：填一句**本片的画面**（不用写风格词），每个风格各出一张。
            同一个 seed，所以差别只来自画风。
          </p>
          <textarea
            value={plazaPrompt}
            onChange={(e) => setPlazaPrompt(e.target.value)}
            rows={3}
            placeholder="例：雨夜天台，男主背对镜头握着伞，远处城市灯火"
            className="w-full rounded-lg px-3 py-2 text-[12.5px]"
            style={{ background: "var(--studio-bg-inset)", color: "var(--studio-text)" }}
          />
          <div className="flex items-center gap-2">
            <button
              type="button"
              disabled={!plazaPrompt.trim() || busy}
              onClick={() =>
                void runTask(
                  {
                    title: "风格广场开跑了…",
                    body: "每个风格串行出一张，出一张刷新一张。可以先去别的地方。",
                    success: () => ({
                      title: "风格广场已登记",
                      body: "进度在右下角队列里。",
                    }),
                    failure: "风格广场没跑起来",
                  },
                  async () => {
                  setBusy(true);
                  try {
                    await styleCompareStart(project.id, {
                      shot_prompt: plazaPrompt.trim(),
                      preset_ids: [],
                      width: 1024,
                      height: 1024,
                      // 固定 seed：重跑时能和上一轮逐格对比
                      seed: 424242,
                    });
                    setNote("已开始 —— 进度在右下角，出一张刷新一张");
                  } finally {
                    setBusy(false);
                  }
                  },
                )
              }
              className="rounded-lg px-3.5 py-2 text-[12px]"
              style={{ background: "var(--studio-primary)", color: "var(--studio-on-primary)" }}
            >
              全部风格各出一张
            </button>
            <span className="text-[11.5px]" style={{ color: "var(--studio-text-muted)" }}>
              {cat ? `${cat.presets.length} 张，串行出，约 ${Math.round((cat.presets.length * 110) / 60)} 分钟` : ""}
            </span>
          </div>
        </section>
      )}

      {suggestions && (
        <SuggestionRow
          items={suggestions}
          onUse={(s) => {
            setCustomOpen(true);
            setCustomPos(s.positive_prompt);
            setCustomNeg(s.negative_prompt);
            void pick({ preset_id: "", positive_prompt: s.positive_prompt, negative_prompt: s.negative_prompt });
          }}
          onClose={() => setSuggestions(null)}
        />
      )}

      {customOpen && (
        <section
          className="flex flex-col gap-2 rounded-xl p-3.5"
          style={{ background: "var(--studio-bg-inset)" }}
        >
          <p className="studio-eyebrow">自定义风格词（留空 = 用选中的预设）</p>
          <textarea
            value={customPos}
            onChange={(e) => setCustomPos(e.target.value)}
            rows={2}
            placeholder="正向：cinematic photorealism, natural light, 35mm…"
            className="w-full resize-y rounded-lg px-3 py-2 text-[12px] outline-none"
            style={field}
          />
          <textarea
            value={customNeg}
            onChange={(e) => setCustomNeg(e.target.value)}
            rows={2}
            placeholder="负向：cartoon, plastic skin…"
            className="w-full resize-y rounded-lg px-3 py-2 text-[12px] outline-none"
            style={field}
          />
          <p className="text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
            负向词只在 SDXL 上真起作用。Z-Image、Krea 2、Qwen 2.1 的 cfg=1，
            负向会被清零 —— 传了不报错，只是完全没效果。
          </p>
          <button
            type="button"
            disabled={busy}
            onClick={() =>
              void pick({ positive_prompt: customPos, negative_prompt: customNeg })
            }
            className="self-start rounded-lg px-3.5 py-2 text-[12px] font-medium"
            style={{ background: "var(--studio-primary)", color: "var(--studio-on-primary)" }}
          >
            用这个
          </button>
        </section>
      )}

      {note && (
        <p className="text-[11.5px]" style={{ color: "var(--studio-text-2)" }}>
          {note}
        </p>
      )}

      <section
        className="flex flex-col gap-2 rounded-xl p-3.5"
        style={{ background: "var(--studio-bg-inset)" }}
      >
        <p className="studio-eyebrow">镜头与表演偏好（本项目）</p>
        <textarea
          value={shotNotes}
          onChange={(e) => setShotNotes(e.target.value)}
          rows={4}
          placeholder="这部剧自己的拍法口味，比如：女性角色多用低机位仰拍、坐姿翘腿斜坐、走路猫步；动作要有真实的物理质感……留空 = 没有额外偏好"
          className="w-full resize-y rounded-lg px-3 py-2 text-[12px] outline-none"
          style={field}
        />
        <p className="text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
          只管这个项目。生成镜头表、细化时间线、合成提示词时当成「用户定下的」规则带进去，别的项目不受影响。
        </p>
        <button
          type="button"
          disabled={busy || shotNotes === (art?.shot_notes ?? "")}
          onClick={() => void saveNotes()}
          className="self-start rounded-lg px-3.5 py-2 text-[12px] font-medium disabled:opacity-50"
          style={{ background: "var(--studio-primary)", color: "var(--studio-on-primary)" }}
        >
          保存偏好
        </button>
        {notesNote && (
          <p className="text-[11.5px]" style={{ color: "var(--studio-text-2)" }}>
            {notesNote}
          </p>
        )}
      </section>

      <div className="grid grid-cols-[repeat(auto-fill,minmax(224px,1fr))] gap-3">
        {shown.map((p) => (
          <PresetCard
            key={p.id}
            preset={p}
            mineUrl={mine[p.id]}
            active={art?.preset_id === p.id}
            onPick={() => void pick({ preset_id: p.id, positive_prompt: "", negative_prompt: "" })}
          />
        ))}
      </div>
    </div>
  );
}

function CurrentBanner({
  cat,
  art,
}: {
  cat: StyleCatalog | null;
  art: ArtDirection | null;
}) {
  const preset = cat?.presets.find((p) => p.id === art?.preset_id);
  const custom = !!art?.positive_prompt.trim();
  const set = !!preset || custom;
  return (
    <div
      className="flex items-center gap-3 rounded-xl px-4 py-3"
      style={{
        background: "var(--studio-bg-inset)",
        border: `1px solid ${set ? "var(--studio-primary)" : "var(--studio-border-subtle)"}`,
      }}
    >
      <span className="studio-eyebrow shrink-0">当前定调</span>
      <span className="min-w-0 flex-1 truncate text-[13px]">
        {custom ? "自定义" : preset ? `${preset.name_zh} · ${preset.name}` : "未定调"}
      </span>
      {!set && (
        <span className="text-[11.5px]" style={{ color: "var(--studio-text-muted)" }}>
          不定也能生成，只是每一镜的画风会各走各的
        </span>
      )}
    </div>
  );
}

function PresetCard({
  preset,
  active,
  mineUrl,
  onPick,
}: {
  preset: StylePreset;
  active: boolean;
  mineUrl?: string;
  onPick: () => void;
}) {
  // 本片出的优先 —— 内置缩略图是通用样例，看本片的画面才判得准
  const thumb = mineUrl || preset.thumb_url;
  return (
    <button
      type="button"
      onClick={onPick}
      className="studio-tile flex flex-col text-left"
      data-active={active}
    >
      <div
        className="studio-tile-media flex aspect-[4/3] w-full items-center justify-center"
        style={{ background: "var(--studio-bg-inset)" }}
      >
        {thumb ? (
          <MediaThumb
            path={thumb}
            alt={preset.name_zh}
            /* **风格缩略图用 `cover`，是这一栏的例外。**
               它示范的是画风 —— 色彩、光线、质感，裁掉边缘不损失任何信息。
               缩略图是 768 方图、卡位是横的，用 `contain` 会在两边留黑边，
               看着很不专业（用户原话）。
               「资产图不许裁」那条针对的是设定卡：那种图裁一刀就少半个版面。 */
            className="h-full w-full object-cover"
          />
        ) : (
          <span
            className="px-3 text-center text-[10.5px] leading-relaxed"
            style={{ color: "var(--studio-text-muted)" }}
          >
            还没在本机出过样例
          </span>
        )}
      </div>
      <div className="flex flex-col gap-1 px-3 py-2.5">
        <p className="studio-display truncate text-[15px]">{preset.name_zh}</p>
        <p
          className="truncate font-mono text-[0.5rem] uppercase tracking-[0.1em]"
          style={{ color: "var(--studio-text-muted)" }}
        >
          {preset.name}
        </p>
        {preset.subtitle_zh && (
          <p className="text-[11px]" style={{ color: "var(--studio-text-2)" }}>
            {preset.subtitle_zh}
          </p>
        )}
        {preset.best_for.length > 0 && (
          <p className="text-[10.5px]" style={{ color: "var(--studio-text-muted)" }}>
            适合：{preset.best_for.join(" · ")}
          </p>
        )}
        {preset.avoid_for.length > 0 && (
          <p className="text-[10.5px]" style={{ color: "var(--studio-text-muted)" }}>
            不适合：{preset.avoid_for.join(" · ")}
          </p>
        )}
      </div>
    </button>
  );
}

function SuggestionRow({
  items,
  onUse,
  onClose,
}: {
  items: StyleSuggestion[];
  onUse: (s: StyleSuggestion) => void;
  onClose: () => void;
}) {
  return (
    <section className="flex flex-col gap-2 rounded-xl p-3.5" style={{ background: "var(--studio-bg-inset)" }}>
      <div className="flex items-center justify-between">
        <p className="studio-eyebrow">读剧本推荐 · {items.length} 种</p>
        <button type="button" onClick={onClose} className="text-[13px] opacity-60">
          ✕
        </button>
      </div>
      <div className="grid grid-cols-[repeat(auto-fit,minmax(200px,1fr))] gap-2">
        {items.map((s, i) => (
          <div
            key={`${s.name}-${i}`}
            className="flex flex-col gap-1 rounded-lg p-3"
            style={{ background: "var(--studio-bg-elevated)" }}
          >
            <p className="text-[13px]">{s.name}</p>
            {s.description && (
              <p className="text-[11.5px]" style={{ color: "var(--studio-text-2)" }}>
                {s.description}
              </p>
            )}
            {s.reason && (
              <p className="text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
                {s.reason}
              </p>
            )}
            <button
              type="button"
              onClick={() => onUse(s)}
              className="mt-1 self-start rounded-lg px-2.5 py-1 text-[11.5px]"
              style={{ background: "var(--studio-bg-inset)", color: "var(--studio-text-2)" }}
            >
              用这个
            </button>
          </div>
        ))}
      </div>
    </section>
  );
}

function TabBtn({
  id,
  label,
  active,
  onClick,
}: {
  id: string;
  label: string;
  active: string;
  onClick: (id: string) => void;
}) {
  return (
    <button
      type="button"
      onClick={() => onClick(id)}
      className="rounded-full px-3 py-1.5 text-[12px]"
      style={{
        background: active === id ? "var(--studio-bg-elevated)" : "transparent",
        color: active === id ? "var(--studio-text)" : "var(--studio-text-muted)",
      }}
    >
      {label}
    </button>
  );
}

const chip: React.CSSProperties = {
  background: "var(--studio-bg-inset)",
  border: "1px solid var(--studio-border-subtle)",
};
const field: React.CSSProperties = {
  background: "var(--studio-bg-elevated)",
  border: "1px solid var(--studio-border-subtle)",
  color: "var(--studio-text)",
};
