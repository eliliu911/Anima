import { useState, useEffect } from "react";
import { useTranslation } from "react-i18next";
import {
  createCharacter,
  deleteCharacter,
  selectCharacter,
  updateCharacter,
  characterBinding,
  setCharacterBinding,
  type Character,
  type CharacterBinding,
  type CharacterBindingView,
} from "../api/voice";
import { usePersona } from "./PersonaLock";
import { useFace } from "../store/face";

/**
 * 角色立绘与视觉元数据
 */
interface CharacterMetaVisual {
  portrait: string;
  tag: string;
  tagClass: string;
  voiceName: string;
  avatarName: string;
}

const PORTRAIT_MAP: Record<string, CharacterMetaVisual> = {
  hiyori: {
    portrait: "/portraits/hiyori.jpg",
    tag: "Live2D",
    tagClass: "bg-amber-500/80 text-white border-amber-300/40",
    voiceName: "三月七 (星铁原声)",
    avatarName: "Hiyori 日和",
  },
  mao: {
    portrait: "/portraits/mao.jpg",
    tag: "Live2D",
    tagClass: "bg-pink-500/80 text-white border-pink-300/40",
    voiceName: "绮良良 (原神原声)",
    avatarName: "Mao 真央",
  },
  shizuku: {
    portrait: "/portraits/shizuku.jpg",
    tag: "Live2D",
    tagClass: "bg-sky-600/80 text-white border-sky-300/40",
    voiceName: "诺艾尔 (原神原声)",
    avatarName: "Shizuku 雫",
  },
  haru: {
    portrait: "/portraits/haru.jpg",
    tag: "Live2D",
    tagClass: "bg-emerald-600/80 text-white border-emerald-300/40",
    voiceName: "砂糖 (原神原声)",
    avatarName: "Haru 春",
  },
  natori: {
    portrait: "/portraits/natori.jpg",
    tag: "Live2D",
    tagClass: "bg-indigo-600/80 text-white border-indigo-300/40",
    voiceName: "枫原万叶 (原神原声)",
    avatarName: "Natori 名取",
  },
  ad_xuejie: {
    portrait: "/portraits/ad_xuejie.jpg",
    tag: "3D VRM",
    tagClass: "bg-purple-600/80 text-white border-purple-300/40",
    voiceName: "台湾学姐 (专属原声)",
    avatarName: "Anaxa 具象",
  },
  furina: {
    portrait: "/portraits/furina.jpg",
    tag: "3D VRM",
    tagClass: "bg-blue-600/80 text-white border-blue-300/40",
    voiceName: "芙宁娜 (原神原声)",
    avatarName: "Clara 水神",
  },
  yae: {
    portrait: "/portraits/yae.jpg",
    tag: "3D GLB",
    tagClass: "bg-rose-600/80 text-white border-rose-300/40",
    voiceName: "八重神子 (原神原声)",
    avatarName: "Yae Miko 宫司",
  },
  hinata: {
    portrait: "/portraits/hinata.jpg",
    tag: "神经写实",
    tagClass: "bg-purple-600/80 text-white border-purple-300/40",
    voiceName: "诺艾尔 (原声配音)",
    avatarName: "日向雏田 (LivePortrait)",
  },
  seeu: {
    portrait: "/portraits/seeu.jpg",
    tag: "神经写实",
    tagClass: "bg-pink-600/80 text-white border-pink-300/40",
    voiceName: "三月七 (星铁原声)",
    avatarName: "SeeU 小柔 (MuseTalk)",
  },
  ziling: {
    portrait: "/portraits/ziling.jpg",
    tag: "神经写实",
    tagClass: "bg-teal-600/80 text-white border-teal-300/40",
    voiceName: "台湾学姐 (知性原声)",
    avatarName: "子苓学姐 (MuseTalk)",
  },
  rice: {
    portrait: "/portraits/rice.jpg",
    tag: "Live2D",
    tagClass: "bg-amber-600/80 text-white border-amber-300/40",
    voiceName: "绮良良 (原神原声)",
    avatarName: "Rice 小饭女仆",
  },
  wanko: {
    portrait: "/portraits/wanko.jpg",
    tag: "Live2D",
    tagClass: "bg-yellow-600/80 text-white border-yellow-300/40",
    voiceName: "砂糖 (原神原声)",
    avatarName: "Wanko 萌犬",
  },
  xiaoya: {
    portrait: "/portraits/xiaoya.jpg",
    tag: "4D 视频",
    tagClass: "bg-rose-600/80 text-white border-rose-300/40",
    voiceName: "小雅原声 (声音克隆)",
    avatarName: "小雅 (赛博女友)",
  },
  yumiko: {
    portrait: "/portraits/yumiko.jpg",
    tag: "4D 视频",
    tagClass: "bg-rose-600/80 text-white border-rose-300/40",
    voiceName: "Edge · 晓晓",
    avatarName: "由美子 (赛博恋人)",
  },
  hanyu: {
    portrait: "/portraits/hanyu.jpg",
    tag: "4D 视频",
    tagClass: "bg-teal-600/80 text-white border-teal-300/40",
    voiceName: "Edge · 晓晓",
    avatarName: "涵予 (知性学姐)",
  },
  ayami: {
    portrait: "/portraits/ayami.jpg",
    tag: "4D 视频",
    tagClass: "bg-pink-600/80 text-white border-pink-300/40",
    voiceName: "Edge · 晓晓",
    avatarName: "绫美 (虚拟偶像)",
  },
};

const DEFAULT_PORTRAIT = "/portraits/default.jpg";

function resolveCardVisual(
  c: { id: string; name: string; marker?: string; description: string; binding?: CharacterBinding },
  fallbackMeta: CharacterMetaVisual
) {
  const avatar = c.binding?.avatar;
  const voice = c.binding?.voice;
  const engine = c.binding?.engine;

  let tag = fallbackMeta.tag;
  let tagClass = fallbackMeta.tagClass;
  let avatarName = fallbackMeta.avatarName;
  let voiceName = fallbackMeta.voiceName;

  // 角色立绘优先级：
  // 1. 静态映射表中的非默认立绘
  // 2. /portraits/<id>.jpg (ComfyUI 生成或用户导入的专属立绘)
  // 3. 兜底 DEFAULT_PORTRAIT
  let portrait = fallbackMeta.portrait;
  if (!portrait || portrait === DEFAULT_PORTRAIT) {
    portrait = `/portraits/${c.id}.jpg`;
  }

  if (avatar) {
    const isLive2d =
      avatar.toLowerCase().includes("live2d") ||
      avatar.endsWith(".model3.json") ||
      avatar.endsWith(".model.json");
    const isVrm = avatar.endsWith(".vrm");
    const isGlb = avatar.endsWith(".glb") || avatar.endsWith(".gltf");
    const isNeural =
      avatar.toLowerCase().startsWith("neural/") ||
      avatar.toLowerCase().endsWith(".mp4") ||
      avatar.toLowerCase().endsWith(".mov") ||
      avatar.toLowerCase().endsWith(".jpg") ||
      avatar.toLowerCase().endsWith(".png");

    if (isNeural) {
      tag = avatar.toLowerCase().endsWith(".mp4") ? "4D 视频" : "神经写实";
      tagClass = "bg-purple-600/80 text-white border-purple-300/40";
    } else if (isLive2d) {
      tag = "Live2D";
      tagClass = "bg-amber-500/80 text-white border-amber-300/40";
    } else if (isVrm) {
      tag = "3D VRM";
      tagClass = "bg-blue-600/80 text-white border-blue-300/40";
    } else if (isGlb) {
      tag = "3D GLB";
      tagClass = "bg-rose-600/80 text-white border-rose-300/40";
    }

    const parts = avatar.split("/");
    const filename = parts[parts.length - 1];
    avatarName = filename
      .replace(/\.(model3|model)\.json$/, "")
      .replace(/\.(vrm|glb|gltf|jpg|jpeg|png|mp4|mov|webm)$/, "");
  }

  if (voice) {
    if (voice.includes("xiaoya")) voiceName = "小雅原声 (声音克隆)";
    else if (voice.includes("hiyori")) voiceName = "三月七 (星铁)";
    else if (voice.includes("mao")) voiceName = "绮良良 (原神)";
    else if (voice.includes("shizuku")) voiceName = "诺艾尔 (原神)";
    else if (voice.includes("haru")) voiceName = "砂糖 (原神)";
    else if (voice.includes("natori")) voiceName = "枫原万叶 (原神)";
    else if (voice.includes("furina")) voiceName = "芙宁娜 (原神)";
    else if (voice.includes("yaemiko")) voiceName = "八重神子 (原神)";
    else if (voice.includes("ADSister") || voice.includes("taiwan")) voiceName = "台湾学姐 (AD)";
    else if (voice.startsWith("zh-CN-"))
      voiceName = `Edge · ${voice.replace("zh-CN-", "").replace("Neural", "")}`;
    else voiceName = voice;
  } else if (engine) {
    voiceName = engine === "edge" ? "Edge 默认" : "GPT-SoVITS 默认";
  }

  return {
    portrait,
    tag,
    tagClass,
    avatarName,
    voiceName,
  };
}

export type CharacterCategoryTag = "all" | "live2d" | "3d" | "neural";

function getCharacterCategory(c: { binding?: CharacterBinding }): "live2d" | "3d" | "neural" {
  const avatar = c.binding?.avatar?.toLowerCase() || "";
  if (
    avatar.startsWith("neural/") ||
    avatar.endsWith(".mp4") ||
    avatar.endsWith(".mov") ||
    avatar.endsWith(".webm") ||
    avatar.endsWith(".jpg") ||
    avatar.endsWith(".jpeg") ||
    avatar.endsWith(".png")
  ) {
    return "neural";
  }
  if (
    avatar.includes("live2d") ||
    avatar.endsWith(".model3.json") ||
    avatar.endsWith(".model.json")
  ) {
    return "live2d";
  }
  return "3d";
}

export function CharacterSettings() {
  const { t } = useTranslation(["settings", "common"]);
  const { view, reload } = usePersona();
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [draft, setDraft] = useState({ id: "", name: "", description: "" });
  const [selectedTag, setSelectedTag] = useState<CharacterCategoryTag>("all");
  const [searchQuery, setSearchQuery] = useState("");
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  async function applyCharacterSwitch(charId: string | null, charObj?: Character) {
    await selectCharacter(charId);
    if (charId === null) {
      useFace.getState().setMode("abstract");
      useFace.getState().setModelId(null);
    } else if (charObj?.binding?.avatar) {
      const avatar = charObj.binding.avatar;
      const lower = avatar.toLowerCase();
      const isNeural =
        lower.startsWith("neural/") ||
        lower.endsWith(".mp4") ||
        lower.endsWith(".mov") ||
        lower.endsWith(".webm") ||
        lower.endsWith(".jpg") ||
        lower.endsWith(".png") ||
        lower.endsWith(".jpeg");
      const isLive2d =
        lower.includes("live2d") ||
        lower.endsWith(".model3.json") ||
        lower.endsWith(".model.json");

      if (isNeural) {
        useFace.getState().setMode("neural_stream");
      } else if (isLive2d) {
        useFace.getState().setMode("live2d");
      } else {
        useFace.getState().setMode("concrete");
      }
      useFace.getState().setModelId(avatar);
    }
  }

  async function run(what: string, fn: () => Promise<unknown>) {
    if (busy) return;
    setBusy(true);
    setMessage(null);
    try {
      await fn();
      await reload();
      setMessage(what);
    } catch (err) {
      setMessage(`${err}`);
    } finally {
      setBusy(false);
    }
  }

  const active = view?.active ?? null;
  const editingChar = view?.characters.find((c) => c.id === editingId) ?? null;

  return (
    <div className="flex flex-col gap-4 pb-8">
      {/* 头部标题与新建角色入口 */}
      <div className="flex items-center justify-between pb-1 border-b border-[var(--line)]">
        <div>
          <div className="text-[15px] font-bold text-[var(--ink)]">{t("characters.title", { defaultValue: "角色形态 · Souls" })}</div>
          <div className="text-[11.5px] text-[var(--ink-3)]">
            {t("characters.subtitle", { defaultValue: "点击立绘一键切换桌面角色，已为每张角色卡适配专属声音与高精度面容" })}
          </div>
        </div>
        <button
          type="button"
          disabled={busy}
          onClick={() => setCreating(!creating)}
          className="px-3.5 py-1.5 bg-[var(--celadon-2)] text-[var(--celadon)] border border-[var(--celadon)] hover:bg-[var(--celadon)] hover:text-white rounded-xl text-[12px] font-semibold transition-all cursor-pointer shadow-xs disabled:opacity-50"
        >
          {creating ? t("common:cancel", { defaultValue: "取消创建" }) : "＋ " + t("characters.newCharacter", { defaultValue: "新建角色" })}
        </button>
      </div>

      {/* 新建角色表单 */}
      {creating && (
        <div className="flex flex-col gap-3 p-4 bg-[var(--surface-2)] border border-[var(--celadon)]/40 rounded-2xl shadow-sm">
          <div className="text-[13px] font-semibold text-[var(--ink)]">创建全新角色灵魂</div>
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
            <label className="flex flex-col gap-1 text-[12px]">
              <span className="text-[var(--ink-3)]">唯一标识 (ID)</span>
              <input
                className="bg-[var(--surface)] border border-[var(--line)] rounded-lg px-3 py-1.5 outline-none text-[var(--ink)] font-mono text-[12px]"
                value={draft.id}
                placeholder="例如: koharu, chitose"
                onChange={(e) => setDraft({ ...draft, id: e.target.value.toLowerCase().replace(/[^a-z0-9_]/g, "") })}
              />
            </label>
            <label className="flex flex-col gap-1 text-[12px]">
              <span className="text-[var(--ink-3)]">显示名称</span>
              <input
                className="bg-[var(--surface)] border border-[var(--line)] rounded-lg px-3 py-1.5 outline-none text-[var(--ink)] text-[12px]"
                value={draft.name}
                placeholder="例如: 小春"
                onChange={(e) => setDraft({ ...draft, name: e.target.value })}
              />
            </label>
          </div>
          <label className="flex flex-col gap-1 text-[12px]">
            <span className="text-[var(--ink-3)]">人设简述</span>
            <input
              className="bg-[var(--surface)] border border-[var(--line)] rounded-lg px-3 py-1.5 outline-none text-[var(--ink)] text-[12px]"
              value={draft.description}
              placeholder="一句话介绍她的性格与陪伴定位"
              onChange={(e) => setDraft({ ...draft, description: e.target.value })}
            />
          </label>
          <div className="flex justify-end gap-2">
            <button
              type="button"
              className="px-3 py-1.5 text-[12px] bg-[var(--surface-3)] text-[var(--ink)] rounded-lg hover:bg-[var(--surface-4)] cursor-pointer"
              onClick={() => setCreating(false)}
            >
              取消
            </button>
            <button
              type="button"
              disabled={!draft.id.trim() || !draft.name.trim()}
              className="px-3.5 py-1.5 text-[12px] bg-[var(--celadon)] text-white font-medium rounded-lg hover:opacity-90 disabled:opacity-40 cursor-pointer"
              onClick={() =>
                void run(`已创建角色「${draft.name}」`, async () => {
                  await createCharacter(draft.id.trim(), draft.name.trim(), draft.description.trim());
                  setDraft({ id: "", name: "", description: "" });
                  setCreating(false);
                })
              }
            >
              确认创建
            </button>
          </div>
        </div>
      )}

      {/* 自动 Tag 分类与快捷搜索栏 */}
      <div className="flex flex-wrap items-center justify-between gap-2.5 py-1">
        <div className="flex flex-wrap items-center gap-1.5 bg-[var(--surface-2)] p-1 rounded-xl border border-[var(--line)]">
          <button
            type="button"
            onClick={() => setSelectedTag("all")}
            className={`px-3 py-1.5 rounded-lg text-[12px] font-semibold transition-all cursor-pointer flex items-center gap-1.5 ${
              selectedTag === "all"
                ? "bg-[var(--celadon)] text-white shadow-xs"
                : "text-[var(--ink-2)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)]"
            }`}
          >
            <span>✨ {t("characters.tabs.all", { defaultValue: "全部形态" })}</span>
            <span className="text-[10px] opacity-75 px-1.5 py-0.2 rounded-full bg-black/20">
              {(view?.characters.length || 0) + 1}
            </span>
          </button>

          <button
            type="button"
            onClick={() => setSelectedTag("live2d")}
            className={`px-3 py-1.5 rounded-lg text-[12px] font-semibold transition-all cursor-pointer flex items-center gap-1.5 ${
              selectedTag === "live2d"
                ? "bg-amber-600 text-white shadow-xs"
                : "text-[var(--ink-2)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)]"
            }`}
          >
            <span>🎀 {t("characters.tabs.live2d", { defaultValue: "Live2D 二次元" })}</span>
            <span className="text-[10px] opacity-75 px-1.5 py-0.2 rounded-full bg-black/20">
              {view?.characters.filter((c) => getCharacterCategory(c) === "live2d").length || 0}
            </span>
          </button>

          <button
            type="button"
            onClick={() => setSelectedTag("3d")}
            className={`px-3 py-1.5 rounded-lg text-[12px] font-semibold transition-all cursor-pointer flex items-center gap-1.5 ${
              selectedTag === "3d"
                ? "bg-blue-600 text-white shadow-xs"
                : "text-[var(--ink-2)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)]"
            }`}
          >
            <span>🧊 {t("characters.tabs.vrm3d", { defaultValue: "3D 具象实体" })}</span>
            <span className="text-[10px] opacity-75 px-1.5 py-0.2 rounded-full bg-black/20">
              {view?.characters.filter((c) => getCharacterCategory(c) === "3d").length || 0}
            </span>
          </button>

          <button
            type="button"
            onClick={() => setSelectedTag("neural")}
            className={`px-3 py-1.5 rounded-lg text-[12px] font-semibold transition-all cursor-pointer flex items-center gap-1.5 ${
              selectedTag === "neural"
                ? "bg-purple-600 text-white shadow-xs"
                : "text-[var(--ink-2)] hover:text-[var(--ink)] hover:bg-[var(--surface-3)]"
            }`}
          >
            <span>🎥 {t("characters.tabs.neural", { defaultValue: "神经超写实 / 视频" })}</span>
            <span className="text-[10px] opacity-75 px-1.5 py-0.2 rounded-full bg-black/20">
              {view?.characters.filter((c) => getCharacterCategory(c) === "neural").length || 0}
            </span>
          </button>
        </div>

        {/* 快捷搜索栏 */}
        <div className="relative flex items-center">
          <span className="absolute left-3 text-[12px] text-[var(--ink-3)] pointer-events-none">🔍</span>
          <input
            type="text"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder={t("characters.searchPlaceholder", { defaultValue: "搜索角色 / 声线 / 标签..." })}
            className="bg-[var(--surface-2)] border border-[var(--line)] rounded-xl pl-8 pr-7 py-1.5 text-[12px] text-[var(--ink)] placeholder:text-[var(--ink-3)] outline-none focus:border-[var(--celadon)] transition-all w-48 sm:w-56"
          />
          {searchQuery && (
            <button
              type="button"
              onClick={() => setSearchQuery("")}
              className="absolute right-2.5 text-[11px] text-[var(--ink-3)] hover:text-[var(--ink)] cursor-pointer"
            >
              ✕
            </button>
          )}
        </div>
      </div>

      {/* 选妃立绘大卡片网格选择器 (Character Roster Gallery) */}
      <div className="grid grid-cols-2 sm:grid-cols-3 md:grid-cols-4 gap-3.5">
        {/* 1. 默认形态 (系统初始/全局粒子云) */}
        {(selectedTag === "all" || selectedTag === "3d") &&
          (!searchQuery.trim() ||
            "默认形态 全局兜底 电子云 粒子云"
              .toLowerCase()
              .includes(searchQuery.trim().toLowerCase())) && (
            <div
              className={`group relative rounded-2xl overflow-hidden aspect-[3/4] border-2 transition-all duration-200 cursor-pointer flex flex-col justify-between ${
                active === null
                  ? "border-[var(--celadon)] ring-4 ring-[var(--celadon)]/35 shadow-xl scale-[1.02]"
                  : "border-[var(--line)] hover:border-[var(--celadon)]/70 hover:shadow-md hover:-translate-y-0.5"
              }`}
              onClick={() => void run("已切回默认形态", () => applyCharacterSwitch(null))}
            >
              {/* 背景立绘图 */}
              <img
                src={DEFAULT_PORTRAIT}
                alt="默认形态"
                className="absolute inset-0 w-full h-full object-cover group-hover:scale-105 transition-transform duration-300 pointer-events-none"
              />

              {/* 顶部标签 */}
              <div className="relative z-10 flex items-center justify-between p-2.5">
                <span className="text-[10px] font-bold px-2.5 py-0.5 rounded-full bg-black/60 backdrop-blur-md text-white border border-white/20 uppercase shadow-xs whitespace-nowrap">
                  全局兜底
                </span>
              </div>

              {/* 底部渐变信息条 */}
              <div className="relative z-10 bg-gradient-to-t from-black/95 via-black/75 to-transparent pt-12 pb-3 px-3 flex flex-col gap-1">
                <div className="flex items-center justify-between gap-1.5">
                  <div className="text-white font-bold text-[14.5px] drop-shadow-sm truncate">
                    默认形态
                  </div>
                  {active === null && (
                    <span className="text-[10.5px] font-bold px-2 py-0.5 rounded-full bg-[var(--celadon)] text-white shadow-sm flex items-center gap-1 shrink-0 whitespace-nowrap border border-white/20">
                      <span className="w-1.5 h-1.5 rounded-full bg-white animate-pulse" />
                      {t("characters.active", { defaultValue: "出战中" })}
                    </span>
                  )}
                </div>

                <div className="flex items-center gap-1.5 text-[10px] text-white/90">
                  <span className="bg-white/20 backdrop-blur-xs px-1.5 py-0.5 rounded border border-white/15">
                    🎙️ 全局音色
                  </span>
                  <span className="bg-white/20 backdrop-blur-xs px-1.5 py-0.5 rounded border border-white/15">
                    🌌 电子云
                  </span>
                </div>

                <p className="text-white/75 text-[11px] line-clamp-1 m-0">
                  粒子云流体 · 全局通用设置
                </p>
              </div>
            </div>
          )}

        {/* 2. 筛选后的角色立绘卡片 */}
        {view?.characters
          .filter((c) => {
            if (selectedTag !== "all" && getCharacterCategory(c) !== selectedTag) {
              return false;
            }
            if (searchQuery.trim()) {
              const q = searchQuery.trim().toLowerCase();
              const fallbackMeta = PORTRAIT_MAP[c.id] || {
                portrait: DEFAULT_PORTRAIT,
                tag: "自定义",
                tagClass: "",
                voiceName: "",
                avatarName: "",
              };
              const meta = resolveCardVisual(c, fallbackMeta);
              const matchName = c.name.toLowerCase().includes(q);
              const matchId = c.id.toLowerCase().includes(q);
              const matchDesc = (c.description || "").toLowerCase().includes(q);
              const matchVoice = (meta.voiceName || "").toLowerCase().includes(q);
              const matchTag = (meta.tag || "").toLowerCase().includes(q);
              if (!matchName && !matchId && !matchDesc && !matchVoice && !matchTag) {
                return false;
              }
            }
            return true;
          })
          .map((c) => {
            const isActive = active === c.id;
            const fallbackMeta = PORTRAIT_MAP[c.id] || {
              portrait: DEFAULT_PORTRAIT,
              tag: "自定义",
              tagClass: "bg-black/60 text-white border-white/20",
              voiceName: "专属配置",
              avatarName: "专属面容",
            };
            const meta = resolveCardVisual(c, fallbackMeta);

            return (
              <div
                key={c.id}
                className={`group relative rounded-2xl overflow-hidden aspect-[3/4] border-2 transition-all duration-200 cursor-pointer flex flex-col justify-between ${
                  isActive
                    ? "border-[var(--celadon)] ring-4 ring-[var(--celadon)]/35 shadow-xl scale-[1.02]"
                    : "border-[var(--line)] hover:border-[var(--celadon)]/70 hover:shadow-md hover:-translate-y-0.5"
                }`}
                onClick={() => void run(`已切到「${c.name}」`, () => applyCharacterSwitch(c.id, c))}
              >
                {/* 背景立绘图 */}
                <img
                  src={meta.portrait}
                  alt={c.name}
                  onError={(e) => {
                    (e.currentTarget as HTMLImageElement).src = DEFAULT_PORTRAIT;
                  }}
                  className="absolute inset-0 w-full h-full object-cover group-hover:scale-105 transition-transform duration-300 pointer-events-none"
                />

                {/* 顶部标签与快捷操作 */}
                <div className="relative z-10 flex items-center justify-between p-2.5">
                  <span
                    className={`text-[10px] font-bold px-2.5 py-0.5 rounded-full backdrop-blur-md border shadow-xs whitespace-nowrap ${meta.tagClass}`}
                  >
                    {meta.tag}
                  </span>

                  <div className="flex items-center gap-1.5">
                    <button
                      type="button"
                      title="配置与编辑角色"
                      className="w-7 h-7 rounded-full flex items-center justify-center text-[12px] bg-black/40 hover:bg-[var(--celadon)] text-white backdrop-blur-md hover:scale-110 border border-white/20 transition-all cursor-pointer shadow-xs"
                      onClick={(e) => {
                        e.stopPropagation();
                        setEditingId(c.id);
                      }}
                    >
                      ✏️
                    </button>
                    {confirmDeleteId === c.id ? (
                      <div
                        className="flex items-center gap-1 bg-red-950/90 backdrop-blur-md px-2 py-0.5 rounded-full border border-red-500/50 shadow-lg"
                        onClick={(e) => e.stopPropagation()}
                      >
                        <span className="text-[10px] text-red-200 font-bold whitespace-nowrap">删除?</span>
                        <button
                          type="button"
                          title="确认删除"
                          className="px-1.5 py-0.5 rounded bg-red-600 hover:bg-red-500 text-white text-[10px] font-bold cursor-pointer transition-colors"
                          onClick={(e) => {
                            e.stopPropagation();
                            setConfirmDeleteId(null);
                            void run(`已删除角色「${c.name}」`, async () => {
                              await deleteCharacter(c.id);
                            });
                          }}
                        >
                          确定
                        </button>
                        <button
                          type="button"
                          title="取消"
                          className="px-1.5 py-0.5 rounded bg-white/20 hover:bg-white/30 text-white text-[10px] cursor-pointer transition-colors"
                          onClick={(e) => {
                            e.stopPropagation();
                            setConfirmDeleteId(null);
                          }}
                        >
                          取消
                        </button>
                      </div>
                    ) : (
                      <button
                        type="button"
                        title="删除角色"
                        className="w-7 h-7 rounded-full flex items-center justify-center text-[11px] bg-black/40 text-white/80 hover:bg-red-600 hover:text-white transition-all cursor-pointer opacity-0 group-hover:opacity-100 hover:scale-110 border border-white/20 shadow-xs"
                        onClick={(e) => {
                          e.stopPropagation();
                          setConfirmDeleteId(c.id);
                        }}
                      >
                        🗑️
                      </button>
                    )}
                  </div>
                </div>

                {/* 底部渐变信息条 */}
                <div className="relative z-10 bg-gradient-to-t from-black/95 via-black/75 to-transparent pt-12 pb-3 px-3 flex flex-col gap-1">
                  <div className="flex items-center justify-between gap-1.5">
                    <div className="text-white font-bold text-[14.5px] drop-shadow-sm truncate">
                      {c.name}
                    </div>
                    {isActive ? (
                      <span className="text-[10.5px] font-bold px-2 py-0.5 rounded-full bg-[var(--celadon)] text-white shadow-sm flex items-center gap-1 shrink-0 whitespace-nowrap border border-white/20">
                        <span className="w-1.5 h-1.5 rounded-full bg-white animate-pulse" />
                        出战中
                      </span>
                    ) : (
                      c.marker && (
                        <span className="text-[10px] font-medium px-2 py-0.5 rounded-full bg-white/20 text-white/90 shrink-0 whitespace-nowrap backdrop-blur-xs border border-white/10">
                          {c.marker}
                        </span>
                      )
                    )}
                  </div>

                  <div className="flex items-center gap-1.5 text-[10px] text-white/90">
                    <span className="bg-white/20 backdrop-blur-xs px-1.5 py-0.5 rounded border border-white/15 truncate max-w-[110px]" title={meta.voiceName}>
                      🎙️ {meta.voiceName}
                    </span>
                    <span className="bg-white/20 backdrop-blur-xs px-1.5 py-0.5 rounded border border-white/15 truncate max-w-[110px]" title={meta.avatarName}>
                      🎭 {meta.avatarName}
                    </span>
                  </div>

                  <p className="text-white/75 text-[11px] line-clamp-1 m-0" title={c.description}>
                    {c.description || "点击切换形态与灵魂"}
                  </p>
                </div>
              </div>
            );
          })}
      </div>

      {/* 提示消息 */}
      {message && (
        <div className="text-[12px] text-[var(--celadon)] bg-[color-mix(in_srgb,var(--celadon)_12%,transparent)] px-3.5 py-2.5 rounded-xl border border-[var(--celadon)]/30 font-medium">
          {message}
        </div>
      )}

      {/* 模态弹窗：角色全量设定编辑器 (Dedicated Modal Window) */}
      {editingChar && (
        <CharacterEditModal
          character={editingChar}
          metaVisual={resolveCardVisual(
            editingChar,
            PORTRAIT_MAP[editingChar.id] || {
              portrait: `/portraits/${editingChar.id}.jpg`,
              tag: "自定义",
              tagClass: "bg-black/60 text-white border-white/20",
              voiceName: "专属配置",
              avatarName: "专属面容",
            }
          )}
          onClose={() => setEditingId(null)}
          onSaved={() => {
            void reload();
          }}
        />
      )}
    </div>
  );
}

/**
 * 角色全量模态配置窗口 (Modal Dialog)
 */
function CharacterEditModal({
  character,
  metaVisual,
  onClose,
  onSaved,
}: {
  character: { id: string; name: string; description: string; marker?: string };
  metaVisual: CharacterMetaVisual;
  onClose: () => void;
  onSaved: () => void;
}) {
  const [name, setName] = useState(character.name);
  const [description, setDescription] = useState(character.description);
  const [view, setView] = useState<CharacterBindingView | null>(null);
  const [draft, setDraft] = useState<CharacterBinding>({});
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  // 加载绑定数据
  async function loadBinding(engine?: string | null) {
    try {
      const v = await characterBinding(character.id, engine);
      setView(v);
      setDraft((d) => (engine === undefined ? v.binding : d));
    } catch (err) {
      setMessage(`${err}`);
    }
  }

  useEffect(() => {
    void loadBinding();
    // 监听 ESC 键快速关闭
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [character.id]);

  const set = (patch: Partial<CharacterBinding>) => setDraft((d) => ({ ...d, ...patch }));

  const dirtyMeta = name !== character.name || description !== character.description;
  const dirtyBinding = view ? JSON.stringify(draft) !== JSON.stringify(view.binding) : false;
  const isDirty = dirtyMeta || dirtyBinding;

  const skillsBound = draft.skills ?? null;
  const toggleSkill = (sid: string) => {
    const cur = skillsBound ?? [];
    set({ skills: cur.includes(sid) ? cur.filter((x) => x !== sid) : [...cur, sid] });
  };

  async function handleSaveAll() {
    if (busy) return;
    setBusy(true);
    setMessage(null);
    try {
      if (dirtyMeta) {
        await updateCharacter(character.id, name.trim(), description.trim());
      }
      if (dirtyBinding) {
        await setCharacterBinding(character.id, draft);
      }
      setMessage("已保存全部修改并生效！");
      onSaved();
      setTimeout(() => {
        onClose();
      }, 600);
    } catch (err) {
      setMessage(`${err}`);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/70 backdrop-blur-sm animate-in fade-in duration-150"
      onClick={onClose}
    >
      <div
        className="relative w-full max-w-xl max-h-[88vh] bg-[var(--surface)] text-[var(--ink)] border border-[var(--line)] rounded-2xl shadow-2xl flex flex-col overflow-hidden animate-in zoom-in-95 duration-150"
        onClick={(e) => e.stopPropagation()}
      >
        {/* 模态顶部栏 (Sticky Header) */}
        <div className="flex items-center justify-between px-5 py-3.5 border-b border-[var(--line)] bg-[var(--surface-2)] shrink-0">
          <div className="flex items-center gap-3">
            <img
              src={metaVisual.portrait}
              alt={name}
              onError={(e) => {
                (e.currentTarget as HTMLImageElement).src = DEFAULT_PORTRAIT;
              }}
              className="w-10 h-10 rounded-xl object-cover border border-[var(--line)] shadow-xs"
            />
            <div>
              <div className="flex items-center gap-2">
                <span className="font-bold text-[14.5px] text-[var(--ink)]">
                  配置角色「{name}」
                </span>
                {character.marker && (
                  <span className="text-[10.5px] font-semibold bg-[var(--celadon-2)] text-[var(--celadon)] border border-[var(--celadon)]/30 px-1.5 py-0.2 rounded">
                    {character.marker}
                  </span>
                )}
              </div>
              <div className="text-[11px] font-mono text-[var(--ink-3)]">
                唯一标识: {character.id}
              </div>
            </div>
          </div>

          <button
            type="button"
            onClick={onClose}
            className="w-8 h-8 rounded-full bg-[var(--surface)] hover:bg-[var(--surface-3)] text-[var(--ink-2)] hover:text-[var(--ink)] border border-[var(--line)] flex items-center justify-center text-[14px] transition-colors cursor-pointer"
          >
            ✕
          </button>
        </div>

        {/* 模态滚动主体 (Scrollable Body) */}
        <div className="overflow-y-auto p-5 flex flex-col gap-5 flex-1">
          {/* 第 1 块：基本人设信息 */}
          <div className="flex flex-col gap-3">
            <div className="text-[12px] font-bold text-[var(--ink)] uppercase tracking-wider flex items-center gap-1.5">
              <span className="w-1.5 h-3.5 bg-[var(--celadon)] rounded-full" />
              基本设定与性格简介
            </div>

            <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
              <label className="flex flex-col gap-1 text-[12px]">
                <span className="text-[var(--ink-3)] font-medium">显示名称</span>
                <input
                  className="bg-[var(--surface-2)] border border-[var(--line)] rounded-xl px-3 py-2 outline-none text-[var(--ink)] text-[12px] focus:border-[var(--celadon)] transition-colors"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                />
              </label>

              <label className="flex flex-col gap-1 text-[12px]">
                <span className="text-[var(--ink-3)] font-medium">当前适配声线</span>
                <div className="bg-[var(--surface-2)] border border-[var(--line)] rounded-xl px-3 py-2 text-[var(--ink-2)] text-[12px] font-medium truncate">
                  🎙️ {metaVisual.voiceName}
                </div>
              </label>
            </div>

            <label className="flex flex-col gap-1 text-[12px]">
              <span className="text-[var(--ink-3)] font-medium">人设性格与口吻</span>
              <textarea
                rows={2}
                className="bg-[var(--surface-2)] border border-[var(--line)] rounded-xl p-3 outline-none text-[var(--ink)] text-[12px] focus:border-[var(--celadon)] transition-colors resize-none"
                value={description}
                onChange={(e) => setDescription(e.target.value)}
              />
            </label>
          </div>

          {/* 第 2 块：声音与音色绑定 */}
          <div className="flex flex-col gap-3 pt-3 border-t border-[var(--line)]">
            <div className="text-[12px] font-bold text-[var(--ink)] uppercase tracking-wider flex items-center gap-1.5">
              <span className="w-1.5 h-3.5 bg-[var(--celadon)] rounded-full" />
              声音引擎与专属音色
            </div>

            {view ? (
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
                <label className="flex flex-col gap-1 text-[12px]">
                  <span className="text-[var(--ink-3)] font-medium">语音引擎</span>
                  <select
                    className="bg-[var(--surface-2)] border border-[var(--line)] rounded-xl px-3 py-2 outline-none text-[var(--ink)] text-[12px] focus:border-[var(--celadon)] transition-colors cursor-pointer"
                    value={draft.engine ?? ""}
                    onChange={(e) => {
                      const next = e.target.value || null;
                      set({ engine: next, voice: null });
                      void loadBinding(next);
                    }}
                  >
                    <option value="">跟随全局设置</option>
                    {view.engines.map((e) => (
                      <option key={e.id} value={e.id}>
                        {e.label}
                      </option>
                    ))}
                  </select>
                </label>

                <label className="flex flex-col gap-1 text-[12px]">
                  <span className="text-[var(--ink-3)] font-medium">专属音色</span>
                  <select
                    className="bg-[var(--surface-2)] border border-[var(--line)] rounded-xl px-3 py-2 outline-none text-[var(--ink)] text-[12px] focus:border-[var(--celadon)] transition-colors cursor-pointer"
                    value={draft.voice ?? ""}
                    onChange={(e) => set({ voice: e.target.value || null })}
                  >
                    <option value="">跟随全局设置</option>
                    {view.voices.map((v) => (
                      <option key={v.short_name} value={v.short_name}>
                        {v.short_name}
                        {v.gender ? ` · ${v.gender}` : ""}
                      </option>
                    ))}
                    {draft.voice && !view.voices.some((v) => v.short_name === draft.voice) && (
                      <option value={draft.voice}>{draft.voice}（当前值）</option>
                    )}
                  </select>
                </label>
              </div>
            ) : (
              <div className="text-[12px] text-[var(--ink-3)]">正在加载语音引擎…</div>
            )}
          </div>

          {/* 第 3 块：面容形态绑定 */}
          <div className="flex flex-col gap-3 pt-3 border-t border-[var(--line)]">
            <div className="text-[12px] font-bold text-[var(--ink)] uppercase tracking-wider flex items-center gap-1.5">
              <span className="w-1.5 h-3.5 bg-[var(--celadon)] rounded-full" />
              面容形象模型
            </div>

            {view && (
              <label className="flex flex-col gap-1 text-[12px]">
                <span className="text-[var(--ink-3)] font-medium">绑定模型 (Live2D / 3D)</span>
                <select
                  className="bg-[var(--surface-2)] border border-[var(--line)] rounded-xl px-3 py-2 outline-none text-[var(--ink)] text-[12px] focus:border-[var(--celadon)] transition-colors cursor-pointer"
                  value={draft.avatar ?? ""}
                  onChange={(e) => set({ avatar: e.target.value || null })}
                >
                  <option value="">跟随全局设置</option>
                  {view.avatars.map((m) => (
                    <option key={m.id} value={m.id}>
                      {m.kind === "live2d" || m.id.endsWith(".model3.json") ? "[Live2D] " : "[3D] "}
                      {m.folder ? `${m.folder}/` : ""}
                      {m.name}
                    </option>
                  ))}
                  {draft.avatar && !view.avatars.some((m) => m.id === draft.avatar) && (
                    <option value={draft.avatar}>{draft.avatar}（当前值）</option>
                  )}
                </select>
              </label>
            )}
          </div>

          {/* 第 4 块：专属技能挂载 */}
          <div className="flex flex-col gap-3 pt-3 border-t border-[var(--line)]">
            <div className="flex items-center justify-between">
              <div className="text-[12px] font-bold text-[var(--ink)] uppercase tracking-wider flex items-center gap-1.5">
                <span className="w-1.5 h-3.5 bg-[var(--celadon)] rounded-full" />
                专属技能挂载
              </div>
              <span className="text-[11px] text-[var(--ink-3)]">
                {skillsBound === null ? "跟随全局设置" : `已独立启用 ${skillsBound.length} 项`}
              </span>
            </div>

            {view && (
              <div className="flex flex-wrap gap-1.5">
                <button
                  type="button"
                  className={`text-[11.5px] px-3 py-1.5 border rounded-xl transition-all cursor-pointer ${
                    skillsBound === null
                      ? "bg-[var(--celadon-2)] border-[var(--celadon)] text-[var(--celadon)] font-semibold shadow-xs"
                      : "border-[var(--line)] bg-[var(--surface-2)] text-[var(--ink-2)] hover:border-[var(--ink-3)]"
                  }`}
                  onClick={() => set({ skills: skillsBound === null ? [] : null })}
                >
                  跟随全局
                </button>
                {view.skills.map((s) => (
                  <button
                    key={s.id}
                    type="button"
                    className={`text-[11.5px] px-3 py-1.5 border rounded-xl transition-all cursor-pointer ${
                      skillsBound?.includes(s.id)
                        ? "bg-[var(--ink)] text-[var(--surface)] border-[var(--ink)] font-semibold shadow-xs"
                        : "border-[var(--line)] bg-[var(--surface-2)] text-[var(--ink-2)] hover:border-[var(--ink-3)]"
                    } disabled:opacity-40`}
                    disabled={skillsBound === null}
                    onClick={() => toggleSkill(s.id)}
                  >
                    {s.name}
                  </button>
                ))}
              </div>
            )}
          </div>
        </div>

        {/* 模态底部操作栏 (Sticky Footer) */}
        <div className="flex items-center justify-between px-5 py-3.5 border-t border-[var(--line)] bg-[var(--surface-2)] shrink-0">
          <div className="text-[12px] font-medium text-[var(--celadon)] truncate max-w-[260px]">
            {message}
          </div>

          <div className="flex items-center gap-2">
            <button
              type="button"
              className="px-4 py-2 text-[12px] bg-[var(--surface)] border border-[var(--line)] rounded-xl text-[var(--ink-2)] hover:text-[var(--ink)] cursor-pointer transition-colors"
              disabled={busy}
              onClick={onClose}
            >
              取消
            </button>
            <button
              type="button"
              className="px-5 py-2 text-[12px] bg-[var(--celadon)] hover:brightness-105 text-white rounded-xl font-semibold disabled:opacity-50 cursor-pointer shadow-md transition-all"
              disabled={busy || !name.trim()}
              onClick={() => void handleSaveAll()}
            >
              {busy ? "保存中…" : isDirty ? "保存修改" : "完成"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
