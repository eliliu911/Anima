/**
 * 提示词框上方的「出镜」头像行。
 *
 * # 为什么必须有
 *
 * `@` 引用的绑定关系藏在提示词文本里（`[角色:老茶师]`）。
 * 不把它渲染出来的话，用户看不出「这次生成到底挂了哪几张参考图」——
 * 而挂没挂上是这条链路最容易出问题也最难发现的地方。
 *
 * **没对上库的引用要标红。** 悄悄当普通文字处理的话，
 * 用户以为挂了参考图，实际一张都没挂。
 */
import { useEffect, useState } from "react";
import { assetsResolveRefs, KIND_LABEL, type ResolvedRef } from "../../api/assets";
import { cinemaFrameReferencePick, type FrameRefPick, type StoryboardShot } from "../../api/cinema";
import { castUsage, castUsageNote } from "./castUsage";
import { MediaThumb } from "./MediaThumb";

export function CastRow({
  prompt,
  shot,
  frameEngine,
  frameSlots = 0,
}: {
  prompt: string;
  /** 少了它就只能说「引用了谁」，说不了「这一镜到底会不会用上它的图」。 */
  shot?: Pick<StoryboardShot, "mode" | "character_id" | "dialogue_lines" | "blocking">;
  frameEngine?: string;
  /** 首帧引擎的参考图槽数。0 = 文生图，首帧不吃资产图。 */
  frameSlots?: number;
}) {
  const [refs, setRefs] = useState<ResolvedRef[]>([]);
  const [pick, setPick] = useState<FrameRefPick | null>(null);
  const wantsPick = !!shot && shot.mode !== "reference" && frameSlots > 0 && !!frameEngine;
  const dialogue = shot?.dialogue_lines;
  const blocking = shot?.blocking;

  useEffect(() => {
    if (!prompt.includes("[")) {
      setRefs([]);
      return;
    }
    // 打字时别每个字符都打一次 IPC。
    let alive = true;
    const timer = setTimeout(() => {
      assetsResolveRefs(prompt)
        .then((r) => alive && setRefs(r))
        .catch(() => alive && setRefs([]));
    }, 250);
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, [prompt]);

  // 参考生图首帧挑哪几张：**问后端**，排序规则只在 Rust 那边写一份。
  useEffect(() => {
    if (!wantsPick || !prompt.includes("[")) {
      setPick(null);
      return;
    }
    let alive = true;
    const timer = setTimeout(() => {
      cinemaFrameReferencePick(prompt, dialogue ?? [], blocking ?? [], frameEngine ?? "")
        .then((p) => alive && setPick(p))
        .catch(() => alive && setPick(null));
    }, 250);
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, [wantsPick, prompt, dialogue, blocking, frameEngine]);

  if (refs.length === 0) return null;

  const missing = refs.filter((r) => !r.asset_id);
  const noImage = refs.filter((r) => r.asset_id && !r.image_path);
  // **「有图」不等于「这一镜会用它」。** 三条链路用图的方式完全不同，
  // 而界面上长得一模一样 —— 见 `castUsage` 的注释。
  const unused = shot ? castUsageNote(castUsage(refs, shot, frameEngine ?? "", wantsPick ? pick : null), shot.mode) : null;

  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex flex-wrap items-center gap-1.5">
        <span className="studio-eyebrow">出镜</span>
        {refs.map((r) => (
          <span
            key={`${r.kind}/${r.name}`}
            title={
              r.asset_id
                ? r.description || `${KIND_LABEL[r.kind]}：${r.name}`
                : `资产库里没有叫「${r.name}」的${KIND_LABEL[r.kind]}`
            }
            className="flex items-center gap-1.5 rounded-full py-0.5 pl-0.5 pr-2.5 text-[11px]"
            style={{
              background: "var(--studio-bg-inset)",
              border: `1px solid ${r.asset_id ? "transparent" : "var(--studio-danger)"}`,
              color: r.asset_id ? "var(--studio-text-2)" : "var(--studio-danger)",
            }}
          >
            <span
              className="flex h-[18px] w-[18px] shrink-0 items-center justify-center overflow-hidden rounded-full"
              style={{ background: "var(--studio-bg-elevated)" }}
            >
              {r.image_url ? (
                <MediaThumb path={r.image_url} className="h-full w-full object-contain" />
              ) : (
                <span className="text-[8px] opacity-50">?</span>
              )}
            </span>
            {r.name}
          </span>
        ))}
      </div>
      {missing.length > 0 && (
        <p className="text-[11px]" style={{ color: "var(--studio-danger)" }}>
          资产库里没有：{missing.map((r) => r.name).join("、")} —— 会按普通文字处理，不挂参考图
        </p>
      )}
      {noImage.length > 0 && (
        <p className="text-[11px]" style={{ color: "var(--studio-text-muted)" }}>
          {noImage.map((r) => r.name).join("、")} 还没有图，只有描述会进提示词
        </p>
      )}
      {/* **用警告色，不用次要文字色。** 这条说的是「你以为挂上了、其实没有」，
          和上面那条「还没有图」不是一个量级 —— 后者用户自己就知道。 */}
      {unused && (
        <p
          className="rounded-md px-2 py-1.5 text-[11px] leading-relaxed"
          style={{
            background: "color-mix(in srgb, var(--studio-accent) 12%, transparent)",
            color: "var(--studio-accent)",
          }}
        >
          {unused}
        </p>
      )}
    </div>
  );
}
