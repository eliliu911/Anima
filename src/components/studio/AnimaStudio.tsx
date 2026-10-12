/**
 * Anima Studio —— 创作台。
 *
 * 布局与交互**照搬自 LumenX**（阿里，MIT，https://github.com/alibaba/lumenx）
 * 的 Playground：左侧 420px 输入栏（模式 → 提示词 → 媒体 → 模型与参数），
 * 右侧结果画廊。照搬是为了先跳过设计环节、把交互跑通。
 *
 * **业务逻辑完全是我们自己的**：本机 ComfyUI 的四个出图引擎 + 两个视频引擎，
 * 八种模式（比 LumenX 多设定卡、姿势控制、首尾帧，少一个 v2v）。
 *
 * # 这一页存在的理由
 *
 * 后端有的能力，旧的 `ComfyStudio.tsx`（2026-09-16 已删）一个都够不着 —— 那一页只能按写死的
 * 人设预设出人像。所有真能力（LoRA、ControlNet、设定卡、首尾帧、参考图生视频）
 * 以前只能在对话里让 agent 调。**能力全在，UI 里没有**，这就是"太难用"的根源。
 *
 * # 引擎知识只有一个来源
 *
 * 哪个引擎吃负向提示词、哪个不该暴露步数 —— 全部来自 `studio_capabilities()`。
 * 前端**不硬编码任何引擎脾气**，否则会变成第二份真相并和 Rust 侧漂移。
 */
import { requiredImages, slotCount, slotLabels } from "./referenceSlots";
import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  enginesForMode,
  pickEngine,
  studioCapabilities,
  type CapabilityCatalog,
  type EngineSpec,
} from "../../api/studio";
import { ModeSelector } from "./ModeSelector";
import { PromptPanel } from "./PromptPanel";
import { PromptDrawer, type DrawerKind } from "./PromptDrawer";
import { AssetPicker } from "./AssetPicker";
import { MediaInput } from "./MediaInput";
import { EnginePanel } from "./EnginePanel";
import { ResultGallery } from "./ResultGallery";
import { useResultHistory } from "./useResultHistory";
import { reconcileParams, useStudioStore } from "./useStudioStore";
import { useResultPersistence } from "./useResultPersistence";
import { canSubmit } from "./canSubmit";
import { usesDenoise } from "./denoise";
import { runTask } from "./toastStore";
import {
  mergeTriggers,
  studioAddHistory,
  studioGenerate,
  studioLoras,
} from "../../api/studio";

/** 各模式对输入图的要求，一句话。显示在拖拽区里。 */
const MEDIA_HINT: Record<string, string> = {
  i2i: "上传后自动切换为图像编辑模式",
  pose: "只取姿势骨架，不取长相和画风",
  i2v: "这张就是视频的第一帧",
  fl2v: "两张分别是首帧和末帧，别塞反",
  r2v: "只锁人物身份，首帧不会是这张图",
};

export function AnimaStudio() {
  const { t } = useTranslation(["studio", "common"]);
  // 结果画廊装回上次的 + 变了就存。挂在这里而不是画廊里 ——
  // 画廊在没有结果时会早退，挂在它里面就永远装不回来。
  useResultHistory();
  const [cat, setCat] = useState<CapabilityCatalog | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  const mode = useStudioStore((s) => s.mode);
  const engineId = useStudioStore((s) => s.engineId);
  const setMode = useStudioStore((s) => s.setMode);
  const setEngineRaw = useStudioStore((s) => s.setEngine);
  const results = useStudioStore((s) => s.results);
  // **结果画廊接上磁盘。** 在这之前 Rust 侧的存读命令一行都没被调过 ——
  // 每次重开应用结果都是空的。
  useResultPersistence();

  useEffect(() => {
    studioCapabilities()
      .then((c) => {
        setCat(c);
        // 首次进来把引擎定下来，别让参数区处于"没有引擎"的空档
        if (!engineId) setEngine(pickEngine(c, mode, null));
      })
      .catch((e) => setLoadError(String(e)));
    // 只在挂载时取一次；目录是编译期常量，不会中途变
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /**
   * 换引擎时收敛参数。
   *
   * **越界的换掉、不越界的留住** —— 无脑重置会让人每次切引擎都重设一遍；
   * 无脑保留会把 1344×768 带到只支持 640×384 的引擎上，而那个错误
   * 要等到提交才出现。
   */
  const setEngine = useCallback(
    (id: string | null) => {
      setEngineRaw(id);
      const e = cat?.engines.find((x) => x.id === id);
      if (!e) return;
      const st = useStudioStore.getState();
      const next = reconcileParams(e, {
        size: st.size,
        duration: st.duration,
        batch: st.batch,
      });
      st.setSize(next.size);
      st.setDuration(next.duration);
      st.setBatch(next.batch);
    },
    [cat, setEngineRaw],
  );

  const promptText = useStudioStore((s) => s.prompt);
  const inputImages = useStudioStore((s) => s.images);

  const modeSpec = useMemo(
    () => cat?.modes.find((m) => m.id === mode) ?? null,
    [cat, mode],
  );
  const engine: EngineSpec | null = useMemo(
    () => cat?.engines.find((e) => e.id === engineId) ?? null,
    [cat, engineId],
  );
  // 选中的 LoRA 带的激活词
  const [drawer, setDrawer] = useState<DrawerKind>(null);
  /** 打开资产库时记住是给哪个槽位选图；null = 纯浏览 */
  const [pickSlot, setPickSlot] = useState<number | null>(null);
  const [libOpen, setLibOpen] = useState(false);
  const [loraTriggers, setLoraTriggers] = useState<string[]>([]);
  const picked = useStudioStore((s) => s.loras);
  useEffect(() => {
    if (picked.length === 0) {
      setLoraTriggers([]);
      return;
    }
    studioLoras()
      .then((all) =>
        setLoraTriggers(
          picked.flatMap((p) => all.find((l) => l.name === p.name)?.triggers ?? []),
        ),
      )
      .catch(() => setLoraTriggers([]));
  }, [picked]);

  const usable = useMemo(
    () => (cat ? enginesForMode(cat, mode) : []),
    [cat, mode],
  );

  /**
   * 提交生成。**激活词在这里并进提示词，并把加了什么显示出来** ——
   * 静默改用户的提示词是我们自己造的静默失败。
   */
  // 能不能点生成 —— 三种原因分开说，见 `canSubmit`
  const check = canSubmit({
    engineId: engine?.id ?? null,
    prompt: promptText,
    images: inputImages,
    // 参考生图最少一张就能出 —— 模式上的 images 是槽位上限，不是必填数
    needImages: requiredImages(modeSpec ?? null),
    mediaLabel: modeSpec?.media_label ?? null,
  });

  const submit = useCallback(async () => {
    const st = useStudioStore.getState();
    if (!engine || !modeSpec) return;

    let prompt = st.prompt;
    let added: string[] = [];
    if (st.injectTriggers && loraTriggers.length > 0) {
      const merged = mergeTriggers(prompt, loraTriggers);
      prompt = merged.prompt;
      added = merged.added;
    }

    for (let i = 0; i < st.batch; i += 1) {
      const id = `${Date.now()}_${i}`;
      // seed 给了就每张递增，否则各自随机 —— 同一个 seed 出四张一样的没意义
      const seed = st.seed !== null ? st.seed + i : undefined;
      st.addResult({
        id, mode: st.mode, engineId: engine.id, prompt,
        status: "pending", startedAt: Date.now(), seed,
        triggersAdded: added.length ? added : undefined,
      });
      const which = st.batch > 1 ? ` · 第 ${i + 1}/${st.batch} 张` : "";
      const what = `${modeSpec.label} · ${engine.label}${which}`;
      try {
        useStudioStore.getState().patchResult(id, { status: "running" });
        // 右下角通知条和画廊并存：画廊看结果，通知条保证切走了也知道跑完没有
        await runTask(
          {
            title: `${what} 生成中`,
            body: modeSpec.kind === "video" ? "视频要跑几分钟，可以先去别的地方。" : "结果在右边画廊，可以先去别的地方。",
            success: (out) => ({ title: `${what} 好了`, body: out.notes?.length ? out.notes.join("；") : undefined }),
            failure: `${what} 失败`,
          },
          async () => {
            const out = await studioGenerate({
              mode: st.mode, engine: engine.id, prompt,
              negative_prompt: st.negativePrompt,
              width: st.size?.width ?? 1024, height: st.size?.height ?? 1024,
              seed, images: st.images.filter(Boolean),
              duration: st.duration ?? undefined,
              // 只在图生图带上 —— 别的模式传了也不生效，传了反而让人以为它管用
              denoise: usesDenoise(st.mode) ? (st.denoise ?? undefined) : undefined,
              loras: st.loras,
            });
            useStudioStore.getState().patchResult(id, {
              status: out.ok ? "done" : "failed",
              path: out.path ?? undefined,
              error: out.ok ? undefined : out.message,
              // **成功也可能有话要说** —— 引擎回落、LoRA 名字对不上、
              // 提示词被摊薄。以前这段只在失败时保留，成功时整个丢掉，
              // 于是「选了 Z-Image 实际出的是 SDXL」界面一声不吭。
              notes: out.notes?.length ? out.notes : undefined,
              endedAt: Date.now(),
            });
            // 引擎说没成：画廊已经记下失败原文和备注，这里抛出去只为让通知条显示失败
            if (!out.ok) throw out.message || "引擎没有返回结果";
            return out;
          },
        );
      } catch (e) {
        const cur = useStudioStore.getState().results.find((r) => r.id === id);
        if (cur?.status !== "failed") {
          useStudioStore.getState().patchResult(id, {
            status: "failed", error: String(e), endedAt: Date.now(),
          });
        }
      }
    }
    // 记历史。**只在真的提交过之后记** —— 记下没跑过的提示词，
    // 历史就变成了草稿箱，失去「这段我跑出来过」的意义
    void studioAddHistory({
      prompt, mode: st.mode, engine: engine.id, at: Date.now(),
    }).catch(() => {});
  }, [engine, modeSpec, loraTriggers]);

  if (loadError) {
    return (
      <div className="anima-studio flex h-full items-center justify-center p-8">
        <div className="studio-card max-w-md px-6 py-5">
          <div className="studio-eyebrow mb-2">
            {t("studio:catalogFailed", { defaultValue: "能力目录加载失败" })}
          </div>
          <p className="text-[13px]" style={{ color: "var(--studio-text-2)" }}>
            {loadError}
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
      {/* ── 页头 ── */}
      <header
        className="flex shrink-0 items-center justify-between px-7 py-5"
        style={{ borderBottom: "1px solid var(--studio-border-subtle)" }}
        data-tauri-drag-region
      >
        <div className="flex flex-col gap-1">
          <span className="studio-eyebrow" style={{ letterSpacing: "0.2em" }}>
            ANIMA STUDIO
            <span style={{ color: "var(--studio-primary)", fontWeight: 600 }}>
              {" · "}
              {t("studio:freeform", { defaultValue: "自由创作" })}
            </span>
          </span>
          <div className="flex items-baseline gap-[10px]">
            <h1 className="text-[1.75rem] font-semibold tracking-tight">
              {t("studio:workbench", { defaultValue: "创作台" })}
            </h1>
            <span
              className="font-mono text-[0.6875rem] uppercase tracking-[0.1em]"
              style={{ color: "var(--studio-text-muted)" }}
            >
              {t("studio:resultCount", {
                count: results.length,
                defaultValue: "{{count}} 个结果",
              })}
            </span>
          </div>
          <p
            className="font-mono text-[0.6875rem] tracking-[0.06em]"
            style={{ color: "var(--studio-text-muted)" }}
          >
            {t("studio:subtitle", {
              defaultValue: "本机 ComfyUI · 不绑定项目",
            })}
          </p>
        </div>
        {modeSpec && <span className="studio-badge">{modeSpec.id.toUpperCase()}</span>}
      </header>

      {/* ── 左输入 / 右画廊 ── */}
      <div className="flex min-h-0 flex-1 overflow-hidden">
        {/* 左栏 = 可滚的参数区 + **常驻的生成按钮**。
            按钮以前是滚动区的最后一个子元素，引擎参数一多就被推到屏幕外 ——
            而它是这一页唯一那个"让事情发生"的控件，
            每次都要先滚到底才点得到。LumenX 是钉在底部的，照它来。 */}
        <aside
          className="flex w-[420px] shrink-0 flex-col"
          style={{ borderRight: "1px solid var(--studio-border)" }}
        >
          <div className="studio-scroll flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-4 py-4">
          {cat && (
            <>
              <section className="studio-card px-5 py-5">
                <div className="studio-eyebrow mb-3">
                  {t("studio:modeLabel", { defaultValue: "生成模式" })}
                </div>
                <ModeSelector
                  catalog={cat}
                  value={mode}
                  onChange={(m) => {
                    setMode(m);
                    setEngine(pickEngine(cat, m, engineId));
                  }}
                />
              </section>

              <section className="studio-card px-5 py-5">
                <div className="studio-eyebrow mb-3">PROMPT</div>
                <PromptPanel engine={engine} onOpenDrawer={setDrawer} />
              </section>


              {/* 需要图的模式才显示媒体区，标签跟着模式走 */}
              {modeSpec && modeSpec.images > 0 && (
                <section className="studio-card px-5 py-5">
                  <div className="studio-eyebrow mb-3">{modeSpec.media_label}</div>
                  <MediaInput
                    count={slotCount(modeSpec, engine)}
                    slotLabels={slotLabels(modeSpec, slotCount(modeSpec, engine))}
                    hint={MEDIA_HINT[modeSpec.id] ?? ""}
                    onPickFromLibrary={(slot) => {
                      setPickSlot(slot);
                      setLibOpen(true);
                    }}
                  />
                </section>
              )}

              <section className="studio-card px-5 py-5">
                <div className="studio-eyebrow mb-3">
                  {t("studio:engineLabel", { defaultValue: "引擎与参数" })}
                </div>
                <EnginePanel
                  engines={usable}
                  engine={engine}
                  onChange={setEngine}
                  mode={modeSpec}
                />
              </section>
            </>
          )}
          </div>

          {cat && (
            <div
              className="shrink-0 px-4 pb-4 pt-3"
              style={{ borderTop: "1px solid var(--studio-border-subtle)" }}
            >
              {/* **点不动要说为什么。** 这条链路上"点不动"有三种原因
                  （没引擎 / 没提示词 / 少输入图），解法完全不同。
                  灰着不解释，比不灰更让人卡住。 */}
              <button
                type="button"
                className="studio-primary-btn flex w-full items-center justify-center gap-2 px-5 py-3 text-[13px]"
                disabled={!check.ok}
                title={check.reason}
                onClick={() => void submit()}
              >
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" aria-hidden="true">
                  <path
                    d="M12 3l1.9 5.1L19 10l-5.1 1.9L12 17l-1.9-5.1L5 10l5.1-1.9L12 3z"
                    fill="currentColor"
                  />
                  <path
                    d="M18.5 15l.8 2.2 2.2.8-2.2.8-.8 2.2-.8-2.2-2.2-.8 2.2-.8.8-2.2z"
                    fill="currentColor"
                  />
                </svg>
                {check.ok ? t("studio:generate", { defaultValue: "生成" }) : check.reason}
              </button>
            </div>
          )}
        </aside>

        <main className="flex min-w-0 flex-1 flex-col overflow-hidden">
          <ResultGallery />
        </main>

        <AssetPicker
          open={libOpen}
          // 给槽位选图时只列图；侧边栏点开是浏览，图和视频都列
          only={pickSlot === null ? undefined : "image"}
          onClose={() => {
            setLibOpen(false);
            setPickSlot(null);
          }}
          onPick={
            pickSlot === null
              ? undefined
              : (p) => useStudioStore.getState().setImage(pickSlot, p)
          }
        />

        <PromptDrawer
          kind={drawer}
          onClose={() => setDrawer(null)}
          onPick={(text) => useStudioStore.getState().setPrompt(text)}
          currentPrompt={useStudioStore.getState().prompt}
          currentKind={modeSpec?.kind ?? "general"}
        />
        </div>
    </div>
  );
}

export default AnimaStudio;
