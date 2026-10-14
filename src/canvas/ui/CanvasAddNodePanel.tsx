import { useState } from "react";
import {
  Type,
  ImageIcon,
  Video,
  Upload, Music,
  Orbit,
  Film,
  Layers,
  ChevronRight,
  User,
  Maximize2,
} from "./icons";
import type { CanvasNodeType } from "../types";

export interface NodeMenuItem {
  type: CanvasNodeType;
  label: string;
  /**
   * 悬停时的一句说明。
   *
   * **标签放不下的话说不清这个节点干什么。** 格子只有 76px 宽，
   * 「镜头上下文」这种名字要么截断要么换行，两种都不解释它是什么。
   */
  hint: string;
  /** 建节点时带上的初始数据（比如「上传资源」建的是素材节点，不是生图节点） */
  data?: Record<string, unknown>;
  icon: React.ComponentType<{ className?: string }>;
  color: string;
  bg: string;
}

const ALL_ITEMS: NodeMenuItem[] = [
  {
    type: "prompt",
    label: "文本",
    hint: "自由写画面、故事或灵感",
    icon: Type,
    color: "text-white/80",
    bg: "bg-white/5 border-white/10",
  },
  {
    // **画布和成片之间的那条线。** 没有它的话分镜只能对着自由文本框
    // 重打一遍，而重打的那一遍不带身份锚点、地理锁和风格定调 ——
    // 画布里出的图和成片里出的图长得不一样，两边都不报错。
    type: "shot",
    label: "分镜",
    hint: "接进分集里的某一镜，发的提示词和出片那一刻一模一样",
    icon: Film,
    color: "text-white/80",
    bg: "bg-white/5 border-white/10",
  },
  {
    // **以前这里是 `prompt`。** 菜单承诺了一个不存在的能力：点下去拿到的
    // 是个普通文本框，而且不报错。那是「导演契约不好用」最小的样本。
    type: "shotContext",
    label: "镜头上下文",
    hint: "挑一个场景，把它的空间契约（地理锁 + 站位 + 时段）发给下游",
    icon: Orbit,
    color: "text-white/80",
    bg: "bg-white/5 border-white/10",
  },
  {
    type: "imageGen",
    label: "图片",
    hint: "出图。文生图 / 图生图由连线决定",
    icon: ImageIcon,
    color: "text-white/80",
    bg: "bg-white/5 border-white/10",
  },
  {
    type: "videoGen",
    label: "视频",
    hint: "出片。t2v / i2v / r2v 由连线推导，不用自己选",
    icon: Video,
    color: "text-white/80",
    bg: "bg-white/5 border-white/10",
  },
  {
    // 用户 2026-09-20：「画布上还无法使用特定的 LoRA 作为节点」。
    // 生成节点工具条里那个弹层还在，两者共用同一份数据（`collectLoras`）。
    type: "lora",
    label: "LoRA",
    hint: "挂一个 LoRA 接到出图 / 出片节点上。一次配好可以接给多个节点；挂不上会当场标红",
    icon: Layers,
    color: "text-white/80",
    bg: "bg-white/5 border-white/10",
  },
  {
    // 照 libtv：视频 → 深度动作捕捉 → 视频（+ 人物图）= 把动作换到别人身上
    type: "depthCapture",
    label: "深度动作捕捉",
    hint: "接一段视频抽出动作（黑白深度），再接到视频节点 + 人物图，就能把这段动作换到那个人身上",
    icon: Layers,
    color: "text-white/80",
    bg: "bg-white/5 border-white/10",
  },
  {
    type: "upscale",
    label: "超分 ×2",
    hint: "接一张图或一段视频，宽高各放大一倍、细节更锐（RealESRGAN）。图几秒，视频每 1 秒约 25 秒",
    icon: Maximize2,
    color: "text-white/80",
    bg: "bg-white/5 border-white/10",
  },
  {
    // **一个音频节点干三件事**（用户 2026-10-13：「直接把两个合并起来，不要试音色了，就跟 libtv 一致」）：
    // 上传一段 = 素材；打字生成 = 文字转语音；左边连一个音频 = 声音复制。老画布上的「音频素材」（audioClip）照旧能用
    type: "audio",
    label: "音频",
    hint: "上传一段音频，或打字生成语音；左边连一段音频进来就是声音复制。连到视频节点上 = H3 参考音频 <Audio N>",
    icon: Music,
    color: "text-white/80",
    bg: "bg-white/5 border-white/10",
  },
  {
    type: "imageGen",
    label: "上传资源",
    hint: "放一张本机的图或一段音频当素材（自动认）。图要改就从它连一个图片节点出去；音频连视频节点当参考音频",
    // **素材节点，不是生图节点。** 用户 2026-09-14：图生图是「图片节点 → 生图节点」两个节点，
    // 不是一个节点自己既放图又生图
    data: { referenceOnly: true, label: "外部素材" },
    icon: Upload,
    color: "text-white/80",
    bg: "bg-white/5 border-white/10",
  },
  {
    // **以前这里是 `character`** —— 点「导演世界」建出来一个叫「新角色」
    // 的角色节点。
    //
    // 名字换了两次，第二次是被用户纠正的。**DramaClaw 里那是三件不同的
    // 东西，不能混成一个**：
    //
    // | | 是什么 | 我们有没有 |
    // |---|---|---|
    // | 全景**出图** | 从场景生成一张等距圆柱全景 | ✅ 就是这个节点 |
    // | 360° 全景**查看器** | 吃上游的宫格图/全景图，球面里转着看 | ❌ 还没做 |
    // | **导演世界** | 3DGS 片场：摆人物、道具、AI 占位 | ❌ 明确暂缓 |
    //
    // 我一度把这个节点叫成「360° 全景」，那是 DramaClaw 查看器的名字；
    // 更早还沿用着「导演世界」，那是 3DGS 的名字。两次都是**用一个已有
    // 概念的名字去指另一件事** —— 用户照着 DramaClaw 的经验来找，
    // 找到的是别的东西。
    //
    // 「360°」留给查看器，「导演世界」留给 3DGS，两个都别提前占用。
    type: "panorama",
    label: "全景出图",
    hint: "把场景的空间契约铺成一圈等距圆柱全景（360°），接到图片节点上出图，画幅固定 2:1",
    icon: Orbit,
    color: "text-white/80",
    bg: "bg-white/5 border-white/10",
  },
  {
    // **和「全景出图」是一前一后两件事**，不是一件。
    // 出图在上游造那张球面贴图，这个在下游站进去取景。
    type: "panoViewer",
    label: "全景查看",
    hint: "吃上游那张全景图，站进球心转着看，按画幅截取景。反打 = 相机转 180°，几何保证",
    icon: Orbit,
    color: "text-white/80",
    bg: "bg-white/5 border-white/10",
  },
  {
    // **终于名副其实了。** 前两次这个标签指着别的节点：
    // 一开始是 `character`（点下去建出一个叫「新角色」的角色节点），
    // 后来被我改名成「360° 全景」又指着全景出图。
    //
    // 现在它就是导演世界：站进全景球里摆人和道具，截控制帧。
    // 3DGS 是以后**换一种世界来源**（DramaClaw 的 `source_type`
    // 三选一：sog / pano360 / mesh），不是换一条路。
    type: "directorWorld",
    label: "导演世界",
    hint: "在全景场景里摆人偶：几个人、位置、朝向、姿势。输出合成图 + 纯背景到画布，站位能写进分镜",
    icon: Orbit,
    color: "text-white/80",
    bg: "bg-white/5 border-white/10",
  },
];

/**
 * **菜单分层**（用户 2026-10-08：「最顶层，文本，图片，音频，视频。第二层是深度动作，超分，等等，上传资源在最下面」）。
 *
 * - `main`：四种基本东西，一排四个，一打开就看到；
 * - `tool`：在它们上面加工 / 辅助的（深度动作、超分、试音色、LoRA、分镜、全景……）；
 * - `upload`：上传资源，单独一行放在整个面板最底下（技能节点之后）。
 */
export type MenuTier = "main" | "tool" | "upload";
export const MENU_ORDER: { label: string; tier: MenuTier }[] = [
  { label: "文本", tier: "main" },
  { label: "图片", tier: "main" },
  { label: "音频", tier: "main" },
  { label: "视频", tier: "main" },
  { label: "深度动作捕捉", tier: "tool" },
  { label: "超分 ×2", tier: "tool" },
  { label: "LoRA", tier: "tool" },
  { label: "分镜", tier: "tool" },
  { label: "镜头上下文", tier: "tool" },
  { label: "全景出图", tier: "tool" },
  { label: "全景查看", tier: "tool" },
  { label: "导演世界", tier: "tool" },
  { label: "上传资源", tier: "upload" },
];

/** 按 [`MENU_ORDER`] 排好、带上层级的菜单项。**新加节点要在 MENU_ORDER 里写一行**，没写的不显示（有测试盯着） */
export const ADD_NODE_ITEMS: (NodeMenuItem & { tier: MenuTier })[] = MENU_ORDER.flatMap(({ label, tier }) => {
  const item = ALL_ITEMS.find((i) => i.label === label);
  return item ? [{ ...item, tier }] : [];
});
/** 所有定义了的菜单项（测试用：确认每一项都排进了 MENU_ORDER） */
export const DEFINED_NODE_ITEMS = ALL_ITEMS;

/**
 * 下面那一段「技能节点」。
 *
 * **和上面的格子分工不一样**：格子是「往画布里放什么」（文本、图片、
 * 视频这些实体），这里是「给这条链加什么本事」—— 镜头语言和角色人设
 * 都属于后者。技能节点原来混在格子里，和「图片」「视频」并排，
 * 看不出它是另一类东西。
 *
 * 这一排是一行一个（图标 + 名字 + 一句说明），因为这类东西**名字说不清
 * 自己**：「技能」两个字不告诉你它能挑景别和光位。
 */
export interface SkillNodeItem {
  type: CanvasNodeType;
  label: string;
  hint: string;
  icon: React.ComponentType<{ className?: string }>;
  initialData?: Record<string, unknown>;
}

export const SKILL_NODE_ITEMS: SkillNodeItem[] = [
  {
    type: "character",
    label: "主线技能与角色",
    hint: "Anima 角色人设 · 声音与外貌",
    icon: User,
    initialData: { name: "主线角色", promptTrigger: "main_character" },
  },
  {
    type: "skill",
    label: "镜头语言",
    hint: "景别 · 角度 · 运镜 · 光位 · 镜头 · 景深",
    icon: Layers,
  },
];

interface CanvasAddNodePanelProps {
  /**
   * 选了一项。**关闭菜单也由它负责** —— 面板自己不关自己。
   *
   * 以前面板还收一个 `onClose` 并在选完之后也调一次。上层给的是个
   * toggle，而 `onSelectNode` 里已经关过了，于是第二次调用又翻开 ——
   * 净效果是**点了不关**。那个 prop 删掉了：留着一个没人用的关闭回调，
   * 下一个人还会再调一次。
   */
  onSelectNode: (type: CanvasNodeType, initialData?: Record<string, unknown>) => void;
}

/**
 * **竖排紧凑列表，照 LibTV**（用户 2026-10-08：「技能节点……只有两条，占用两行就太浪费空间了。对比 libtv 里的，就简洁很多」）。
 *
 * 一行一项：图标 + 名字，说明放悬停。主菜单五项（文本 / 图片 / 音频 / 视频 / 上传资源），
 * 分隔线下面「工具 ›」「技能 ›」各一行，悬停或点击在右边展开二级菜单。以前是 4 列大图标格子 + 两大行技能，面板高到要滚。
 */
export function CanvasAddNodePanel({ onSelectNode }: CanvasAddNodePanelProps) {
  const [sub, setSub] = useState<"tool" | "skill" | null>(null);
  const main = ADD_NODE_ITEMS.filter((i) => i.tier === "main");
  const tools = ADD_NODE_ITEMS.filter((i) => i.tier === "tool");
  const uploads = ADD_NODE_ITEMS.filter((i) => i.tier === "upload");
  const pick = (item: NodeMenuItem) => onSelectNode(item.type, { label: item.label, ...item.data });
  return (
    <div
      className="relative w-[232px] rounded-[16px] border border-white/10 bg-[#232428] p-1.5 shadow-2xl animate-in fade-in-0 duration-150 [-webkit-font-smoothing:antialiased]"
      onClick={(e) => e.stopPropagation()}
      onMouseLeave={() => setSub(null)}
    >
      <SectionTitle>添加节点</SectionTitle>
      {/* 主菜单五项（用户 2026-10-08：「主菜单就保留五项。文本，图片，音频，视频，上传资源」） */}
      {[...main, ...uploads].map((item) => <Row key={item.label} item={item} onClick={() => pick(item)} onHover={() => setSub(null)} />)}
      {/* 工具、技能各是一个二级菜单 */}
      <div className="mx-2 my-1 border-t border-white/10" />
      <SubRow label="工具" icon={Layers} open={sub === "tool"} onOpen={() => setSub("tool")} />
      <SubRow label="技能" icon={User} open={sub === "skill"} onOpen={() => setSub("skill")} />

      {/* 二级菜单：贴着面板右边展开 */}
      {sub && (
        <div className="absolute left-full top-0 ml-1.5 w-[232px] rounded-[16px] border border-white/10 bg-[#232428] p-1.5 shadow-2xl [-webkit-font-smoothing:antialiased]">
          <SectionTitle>{sub === "tool" ? "工具" : "技能节点"}</SectionTitle>
          {sub === "tool"
            ? tools.map((item) => <Row key={item.label} item={item} onClick={() => pick(item)} />)
            : SKILL_NODE_ITEMS.map((item) => (
                <Row key={item.label} item={{ ...item, label: item.label }} onClick={() => onSelectNode(item.type, item.initialData)} />
              ))}
        </div>
      )}
    </div>
  );
}

function SectionTitle({ children }: { children: React.ReactNode }) {
  return <div className="px-2.5 pb-1 pt-2 text-[12px] text-white/45 first:pt-1">{children}</div>;
}

/** 一行：图标 + 名字，说明在悬停里（标签说不清自己的节点，悬停是唯一的出路 —— 见 NodeMenuItem.hint） */
function Row({ item, label, onClick, onHover }: {
  item: { label: string; hint: string; icon: React.ComponentType<{ className?: string }> };
  label?: string;
  onClick: () => void;
  onHover?: () => void;
}) {
  const Icon = item.icon;
  return (
    <button
      type="button"
      // **只调 onSelectNode，不要再调 onClose。** 上层给的 `onClose` 是个 toggle，它的 `onSelectNode` 里已经关过一次 ——
      // 这里再调一次等于又翻开，净效果是**菜单点了不关**。用户报过这个。关不关是上层的事。
      onClick={onClick}
      onMouseEnter={onHover}
      title={`${item.label} —— ${item.hint}`}
      className="flex h-9 w-full items-center gap-3 rounded-lg px-2.5 text-left text-[14.5px] text-white/95 transition-colors hover:bg-white/[0.08] hover:text-white"
    >
      <Icon className="h-[18px] w-[18px] shrink-0 text-white/90" />
      <span className="truncate">{label ?? item.label}</span>
    </button>
  );
}

/** 有二级菜单的一行（工具 › / 技能 ›）：悬停或点击展开 */
function SubRow({ label, icon: Icon, open, onOpen }: { label: string; icon: React.ComponentType<{ className?: string }>; open: boolean; onOpen: () => void }) {
  return (
    <button
      type="button"
      onClick={onOpen}
      onMouseEnter={onOpen}
      aria-expanded={open}
      className={`flex h-9 w-full items-center gap-3 rounded-lg px-2.5 text-left text-[14.5px] transition-colors ${open ? "bg-white/[0.08] text-white" : "text-white/95 hover:bg-white/[0.08] hover:text-white"}`}
    >
      <Icon className="h-[18px] w-[18px] shrink-0 text-white/90" />
      <span className="flex-1">{label}</span>
      <ChevronRight className="h-4 w-4 text-white/50" />
    </button>
  );
}
