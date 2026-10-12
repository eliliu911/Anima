import { useEffect, useRef, useState } from "react";
import { Check, ChevronDown, Clapperboard, Image, Images, SquareSplitHorizontal, Type, Video } from "lucide-react";
import type { ModeInference, VideoMode } from "../utils/videoMode";
import { MODE_LABEL } from "../utils/videoMode";

type PickableMode = Exclude<VideoMode, "motion">;
/** 菜单里能选的：四种普通模式 + 全能参考（= 原片复刻：图 + 一段参考视频） */
export type MenuMode = PickableMode | "omni";

/**
 * 视频节点底栏的「视频生成模式」—— 照 LibTV 的五项（用户 2026-09-25：「把它分清楚，不然不清不楚的」）。
 *
 * - 按钮上直接写**现在是什么模式**（按连线推出来的；手动改过的后面标「手动」）
 * - 每一项下面一行说明它拿图做什么 —— 光看「图生视频 / 图片参考」这几个字分不清
 * - 跟连线对不上的置灰并说为什么（连着图就不能文生，少于两张图就不能首尾帧，没连视频就不能全能参考）
 * - 选了和推导一样的那项 = 回到「跟着连线走」，不留一个多余的手动标记
 * - **全能参考** = 原片复刻（H3 全参考：<Picture N> 人物 + <Video 1> 参考视频）。以前要在节点上另点「设成原片复刻」
 * - 接了深度节点的动作迁移不给切：模式由连线定死，只显示名字
 */
export const MODE_OPTIONS: { id: MenuMode; label: string; hint: string; icon: typeof Type }[] = [
  { id: "t2v", label: "文生视频", hint: "只写提示词，不用图", icon: Type },
  { id: "omni", label: "全能参考", hint: "人物图 + 一段参考视频：照视频的动作和镜头出片，人换成图里的（H3）", icon: Clapperboard },
  { id: "i2v", label: "图生视频", hint: "1 张图当第一帧，从它开始动", icon: Image },
  { id: "fl2v", label: "首尾帧", hint: "2 张图：第 1 张是开头、第 2 张是结尾，中间补出来", icon: SquareSplitHorizontal },
  { id: "r2v", label: "图片参考", hint: "角色 / 场景图只当外观参考，不当画面帧；多张时可以 @ 点名谁是谁（H3）", icon: Images },
];

/**
 * **菜单按三组摆**（用户 2026-10-08：「图生视频和首尾帧是一套，可选首帧或首尾帧……图片参考和全能参考是一样套」）。
 *
 * 两组内部各两个子项，用组里的小按钮切：首帧 / 首尾帧（都是 FL2VA，拿图当画面帧）；图片参考 / 全能参考（都是 H3 全参考，
 * 图只当外观，全能参考再加一段参考视频）。五种模式本身不变（`MODE_OPTIONS`、`modeBlocked` 照旧），只是菜单的摆法。
 */
export const MODE_GROUPS: { id: "t2v" | "frames" | "ref"; label: string; hint: string; icon: typeof Type; subs: MenuMode[] }[] = [
  { id: "t2v", label: "文生视频", hint: "只写提示词，不用图", icon: Type, subs: ["t2v"] },
  { id: "frames", label: "首帧 / 首尾帧", hint: "图当画面帧：首帧 = 从这张开始动；首尾帧 = 第 1 张开头、第 2 张结尾", icon: Image, subs: ["i2v", "fl2v"] },
  { id: "ref", label: "参考", hint: "图只当角色 / 场景的外观参考，不当画面帧（H3）；再连一段参考视频 = 全能参考，照视频的动作和镜头出", icon: Images, subs: ["r2v", "omni"] },
];
/** 组里子项的短名 */
const SUB_LABEL: Record<MenuMode, string> = { t2v: "文生视频", i2v: "首帧", fl2v: "首尾帧", r2v: "图片参考", omni: "全能参考" };

/** 点组那一行选哪个子项：当前就在这组就不动；否则按连线推的那个在这组就用它；再否则取第一个能选的 */
export function pickInGroup(subs: MenuMode[], current: MenuMode, auto: MenuMode | null, blocked: (m: MenuMode) => string | null): MenuMode | null {
  if (subs.includes(current)) return current;
  if (auto && subs.includes(auto) && !blocked(auto)) return auto;
  return subs.find((m) => !blocked(m)) ?? null;
}

/** 这一项现在能不能选；不能选时给一句理由。`n` = 连着的图（含还没出的），`video` = 上游有没有连视频 */
export function modeBlocked(id: MenuMode, n: number, video = false): string | null {
  if (id === "t2v" && n > 0) return "连着图片 —— 要文生视频先断开图片连线";
  if (id === "i2v" && n === 0) return "先连一张图当首帧";
  if (id === "fl2v" && n < 2) return "要两张图：第 1 张当开头、第 2 张当结尾";
  if (id === "r2v" && n === 0) return "先连至少一张角色或场景图";
  if (id === "omni" && !video) return "先连一段参考视频（上传视频的节点连到这里）";
  return null;
}

export function VideoModeMenu({ inference, videoUpstream = false, disabled, onChange }: {
  inference: ModeInference;
  /** 上游直连了一段视频（全能参考要它） */
  videoUpstream?: boolean;
  disabled?: boolean;
  /** 传 "auto" = 跟着连线走 */
  onChange: (mode: "auto" | MenuMode) => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const close = (e: PointerEvent) => { if (!ref.current?.contains(e.target as Node)) setOpen(false); };
    window.addEventListener("pointerdown", close);
    return () => window.removeEventListener("pointerdown", close);
  }, [open]);

  if (inference.mode === "motion" && inference.via !== "video") {
    return (
      <span className="flex h-7 items-center gap-1.5 px-1.5 text-emerald-200/80" title={`${inference.reason}（模式由连线定死：要换别的模式，先断开深度节点的连线）`}>
        <Video className="h-3.5 w-3.5" />
        {MODE_LABEL.motion}
      </span>
    );
  }
  const omni = inference.mode === "motion";
  const current: MenuMode = omni ? "omni" : (inference.mode as PickableMode);
  // 「按连线」那一项：全能参考是手动开的开关，不算连线推出来的
  const auto: MenuMode | null = omni ? null : ((inference.overridden ?? inference.mode) as PickableMode);
  const n = inference.imageCount ?? inference.images.length;
  const option = MODE_OPTIONS.find((m) => m.id === current);
  const Icon = option?.icon ?? Video;

  return (
    <div className="relative" ref={ref}>
      <button
        type="button" disabled={disabled} aria-haspopup="menu" aria-expanded={open} title={inference.reason}
        onClick={() => setOpen((v) => !v)}
        className="nodrag nopan flex h-7 items-center gap-1.5 rounded-md px-1.5 text-white/80 hover:bg-white/10 hover:text-white disabled:opacity-50"
      >
        <Icon className="h-3.5 w-3.5" />
        {option ? SUB_LABEL[option.id] : MODE_LABEL[inference.mode]}
        {/* 不用黄色（用户 2026-09-25）：完整理由在按钮的悬停提示里 */}
        {inference.overridden && <span className="text-white/40">· 手动</span>}
        <ChevronDown className="h-3 w-3 text-white/40" />
      </button>
      {open && (
        <div role="menu" aria-label="视频生成模式"
          className="nodrag nopan absolute bottom-9 left-0 z-50 flex w-72 flex-col gap-0.5 rounded-xl border border-white/10 bg-zinc-900/95 p-1.5 shadow-xl backdrop-blur">
          <div className="px-2 pb-1 pt-0.5 text-[11px] text-white/40">视频生成模式</div>
          {MODE_GROUPS.map(({ id: gid, label, hint, icon: I, subs }) => {
            const blocked = (m: MenuMode) => modeBlocked(m, n, videoUpstream);
            const target = pickInGroup(subs, current, auto, blocked);
            const on = subs.includes(current);
            const pick = (m: MenuMode) => { onChange(m === auto ? "auto" : m); setOpen(false); };
            const why = target ? null : blocked(subs[0]);
            return (
              <div key={gid} className={`rounded-lg ${on ? "bg-white/10" : ""}`}>
                <button
                  type="button" role="menuitemradio" aria-checked={on} disabled={!target} title={why ?? undefined}
                  onClick={() => target && pick(target)}
                  className={`flex w-full items-start gap-2 rounded-lg px-2 py-1.5 text-left ${on ? "text-white" : "text-white/80 hover:bg-white/10"} disabled:cursor-not-allowed disabled:opacity-35 disabled:hover:bg-transparent`}
                >
                  <I className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                  <span className="flex-1">
                    <span className="block text-[12px]">{label}{auto && subs.includes(auto) && <span className="ml-1.5 text-[10px] text-white/40">按连线</span>}</span>
                    <span className="block text-[10px] leading-snug text-white/40">{why ?? hint}</span>
                  </span>
                  {on && <Check className="mt-0.5 h-3.5 w-3.5 shrink-0 text-white/70" />}
                </button>
                {/* 组里两个子项：首帧 / 首尾帧、图片参考 / 全能参考。不能选的置灰，悬停说为什么 */}
                {subs.length > 1 && (
                  <div className="flex gap-1 px-2 pb-1.5 pl-7">
                    {subs.map((m) => {
                      const b = blocked(m);
                      const active = m === current;
                      return (
                        <button
                          key={m} type="button" disabled={Boolean(b)} title={b ?? MODE_OPTIONS.find((o) => o.id === m)?.hint}
                          onClick={() => pick(m)}
                          className={`rounded-md border px-2 py-0.5 text-[11px] ${active ? "border-white/70 bg-white/10 text-white" : "border-white/10 text-white/60 hover:border-white/30 hover:text-white"} disabled:cursor-not-allowed disabled:opacity-35`}
                        >
                          {SUB_LABEL[m]}
                        </button>
                      );
                    })}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
