/**
 * 角色工作台 —— 一个角色的设定卡生成。
 *
 * 对应 LumenX 的 `CastWorkbenchModal`（全身图 / 三视图 / 头像三档）。
 * **我们这一块比它厚，别削平**：除了三视图，我们还有表情 ×6、姿势 ×4、
 * 面部机位 ×5，以及切片、视觉基因、固定种子、QC 告警。
 * 这一页是把这些**接进流程**，不是重做一个。
 *
 * # 为什么默认只做三视图
 *
 * 全套 18 张在 MPS 上约 10 分钟。默认不全做，让人自己决定什么时候
 * 花这个时间 —— 点一下等十分钟而事先不知道，是最让人恼火的一种交互。
 */
import { useState } from "react";
import {
  cinemaGenerateCharacterSheetViaComfyui,
  type CharacterAssetSheet,
  type CinemaCharacter,
} from "../../../api/cinema";
import { runTask } from "../toastStore";
import { MediaThumb } from "../MediaThumb";

type Tier = "turnaround" | "expressions" | "poses" | "angles";

const TIERS: { id: Tier; label: string; count: string; mins: string }[] = [
  { id: "turnaround", label: "三视图", count: "3 张", mins: "~2 分钟" },
  { id: "expressions", label: "表情", count: "6 张", mins: "~3 分钟" },
  { id: "poses", label: "姿势", count: "4 张", mins: "~2 分钟" },
  { id: "angles", label: "面部机位", count: "5 张", mins: "~3 分钟" },
];

export function CharacterWorkbench({
  projectId,
  character,
  onClose,
  onDone,
}: {
  projectId: string;
  character: CinemaCharacter;
  onClose: () => void;
  onDone: () => void;
}) {
  const [picked, setPicked] = useState<Set<Tier>>(() => new Set<Tier>(["turnaround"]));
  const [busy, setBusy] = useState(false);
  const [sheet, setSheet] = useState<CharacterAssetSheet | null>(
    character.asset_sheet ?? null,
  );
  const [error, setError] = useState<string | null>(null);
  const [useRef, setUseRef] = useState(false);

  const toggle = (t: Tier) =>
    setPicked((s) => {
      const n = new Set(s);
      if (n.has(t)) n.delete(t);
      else n.add(t);
      return n;
    });

  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      const out = await runTask(
        {
          title: `正在给「${character.name}」出设定图…`,
          body: "在 ComfyUI 里一张张出，要几分钟，可以关掉这个窗口去做别的。",
          success: () => ({ title: `「${character.name}」的设定图好了` }),
          failure: `「${character.name}」出设定图失败`,
        },
        () => cinemaGenerateCharacterSheetViaComfyui({
          project_id: projectId,
          character_id: character.id,
          name: character.name,
          trigger_words: character.trigger_words,
          clothing_prompt: character.clothing_prompt,
          front_image_url: character.avatar_url || null,
          lora: character.lora_model,
          lora_weight: character.lora_weight,
          consistency_mode: character.consistency_mode,
          seed: character.seed,
          visual_dna: character.visual_dna,
          tiers: [...picked],
          use_reference_for_turnaround: useRef,
        }),
      );
      setSheet(out);
      onDone();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const est = TIERS.filter((t) => picked.has(t.id));

  return (
    <div
      className="studio-scrim fixed inset-0 z-50 flex items-center justify-center"
    >
      <div
        className="studio-modal flex h-[80vh] w-[760px] max-w-[94vw] flex-col overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        <header
          className="flex shrink-0 items-start justify-between px-6 pb-4 pt-5"
          style={{ borderBottom: "1px solid var(--studio-border-subtle)" }}
        >
          <div className="min-w-0">
            <h2 className="truncate text-[17px] font-semibold">{character.name} · 角色工作台</h2>
            <p className="mt-0.5 text-[12px]" style={{ color: "var(--studio-text-muted)" }}>
              设定卡是跨镜头一致性的底子 —— 正面那格会成为后面分镜的参考图
            </p>
          </div>
          <button type="button" onClick={onClose} className="px-1 text-[15px] opacity-60">
            ✕
          </button>
        </header>

        <div className="min-h-0 flex-1 overflow-y-auto px-6 py-4">
          <section className="mb-5">
            <p className="studio-eyebrow mb-2">这次生成哪几档</p>
            <div className="flex flex-wrap gap-2">
              {TIERS.map((t) => {
                const on = picked.has(t.id);
                return (
                  <button
                    key={t.id}
                    type="button"
                    onClick={() => toggle(t.id)}
                    className="flex flex-col rounded-lg px-3 py-2 text-left"
                    style={{
                      background: on ? "var(--studio-primary)" : "var(--studio-bg-inset)",
                      color: on ? "var(--studio-on-primary)" : "var(--studio-text-2)",
                    }}
                  >
                    <span className="text-[12.5px]">{t.label}</span>
                    <span className="font-mono text-[0.5rem] opacity-70">
                      {t.count} · {t.mins}
                    </span>
                  </button>
                );
              })}
            </div>
            <label
              className="mt-3 flex items-center gap-1.5 text-[11.5px]"
              style={{ color: "var(--studio-text-2)" }}
              title="参考图多半是半身肖像，会把构图一起搬过来 —— 实测整排会塌成四个半身正面"
            >
              <input type="checkbox" checked={useRef} onChange={(e) => setUseRef(e.target.checked)} />
              三视图也挂参考图（默认关：实测会让整排塌成半身正面）
            </label>
            {est.length > 0 && (
              <p className="mt-2 text-[11.5px]" style={{ color: "var(--studio-text-muted)" }}>
                预计 {est.map((t) => t.mins.replace("~", "")).join(" + ")}，
                这台机器上是串行跑的
              </p>
            )}
          </section>

          {error && (
            <p className="mb-4 text-[12px]" style={{ color: "var(--studio-danger)" }}>
              {error}
            </p>
          )}

          {/* QC 告警必须回到界面上。整档失败是这条流程里最贵的一种失败，
              它绝对不能是安静的 —— 否则表现为「表情模组 (0/6)」，
              用户分不清是没做、做失败了、还是做完了没显示。 */}
          {sheet?.warnings?.length ? (
            <section className="mb-5">
              <p className="studio-eyebrow mb-2" style={{ color: "var(--studio-running-fg)" }}>
                这一次的问题 · {sheet.warnings.length}
              </p>
              <div className="flex flex-col gap-1">
                {sheet.warnings.map((w, i) => (
                  <p
                    key={i}
                    className="rounded-lg px-2.5 py-1.5 text-[11.5px]"
                    style={{ background: "var(--studio-bg-inset)", color: "var(--studio-text-2)" }}
                  >
                    {w}
                  </p>
                ))}
              </div>
            </section>
          ) : null}

          <Slots title="三视图" items={sheet?.turnarounds ?? []} total={3} />
          <Slots title="表情" items={sheet?.expressions ?? []} total={6} />
          <Slots title="姿势" items={sheet?.poses ?? []} total={4} />
          <Angles items={sheet?.angles ?? []} />
        </div>

        <footer
          className="flex shrink-0 items-center justify-between px-6 py-4"
          style={{ borderTop: "1px solid var(--studio-border-subtle)" }}
        >
          <span className="text-[11.5px]" style={{ color: "var(--studio-text-muted)" }}>
            {character.trigger_words || "还没有触发词 —— 去资产库详情里补"}
          </span>
          <button
            type="button"
            disabled={busy || picked.size === 0}
            onClick={() => void run()}
            className="rounded-lg px-4 py-2 text-[13px] font-medium"
            style={{
              background: picked.size ? "var(--studio-primary)" : "var(--studio-bg-inset)",
              color: picked.size ? "var(--studio-on-primary)" : "var(--studio-text-muted)",
            }}
          >
            {busy ? "生成中…（别关窗口）" : "生成"}
          </button>
        </footer>
      </div>
    </div>
  );
}

function Slots({
  title,
  items,
  total,
}: {
  title: string;
  items: { key: string; title: string; image_url: string }[];
  total: number;
}) {
  return (
    <section className="mb-4">
      <p className="studio-eyebrow mb-2">
        {title} ({items.length}/{total})
      </p>
      <div className="flex flex-wrap gap-2">
        {items.length === 0 && (
          <p className="text-[11.5px]" style={{ color: "var(--studio-text-muted)" }}>
            还没生成
          </p>
        )}
        {items.map((it) => (
          <div key={it.key} className="w-[96px]">
            <div
              className="flex aspect-[3/4] items-center justify-center overflow-hidden rounded-lg"
              style={{ background: "var(--studio-bg-inset)" }}
            >
              <MediaThumb path={it.image_url} alt={it.title} className="h-full w-full object-contain" />
            </div>
            <p className="mt-1 truncate text-[10px]" style={{ color: "var(--studio-text-muted)" }}>
              {it.title || it.key}
            </p>
          </div>
        ))}
      </div>
    </section>
  );
}

function Angles({ items }: { items: { angle_name: string; image_url: string }[] }) {
  return (
    <section className="mb-4">
      <p className="studio-eyebrow mb-2">面部机位 ({items.length}/5)</p>
      <div className="flex flex-wrap gap-2">
        {items.length === 0 && (
          <p className="text-[11.5px]" style={{ color: "var(--studio-text-muted)" }}>
            还没生成
          </p>
        )}
        {items.map((a) => (
          <div key={a.angle_name} className="w-[96px]">
            <div
              className="flex aspect-square items-center justify-center overflow-hidden rounded-lg"
              style={{ background: "var(--studio-bg-inset)" }}
            >
              <MediaThumb path={a.image_url} alt={a.angle_name} className="h-full w-full object-contain" />
            </div>
            <p className="mt-1 truncate text-[10px]" style={{ color: "var(--studio-text-muted)" }}>
              {a.angle_name}
            </p>
          </div>
        ))}
      </div>
    </section>
  );
}
