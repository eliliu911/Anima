/**
 * 从剧本文档**直接**派生场景、角色、分镜草案 —— 不问模型。
 *
 * # 这是整块剧本编辑器唯一的兑现点
 *
 * 之前是「一整块纯文本 → 让模型抽」。模型抽漏一个人、把旁白当台词、
 * 把两场戏并成一场，**全都不报错**。而台词那条后果最重：
 * H3 会把提示词里的台词变成人物**嘴里说出来的话** ——
 * 旁白被当成台词，视频里的人就会开口念旁白。
 *
 * 文档里 `characterCue` / `dialogue` / `sceneHeading` 本来就是不同的节点，
 * 遍历一遍就是确定的答案，不存在挑错。
 *
 * 模型退到只做一件事：**给这些确定的东西补描述**（外观、氛围），
 * 以及找结构里表达不了的道具。
 */
import type { ScriptDoc } from "./docConvert";
import { numberedHeading, speakerInfo } from "./format";

export interface DerivedScene {
  /** 从 1 开始 */
  index: number;
  /** 场景标题原文 */
  heading: string;
  /** 从标题里拆出来的地点。拆不出来就是整个标题 */
  location: string;
  /** 时间/氛围，拆不出来是空 */
  timeOfDay: string;
  /** 这一场里出场的角色（按出现顺序） */
  characters: string[];
  /** 这一场有几句台词 */
  dialogueCount: number;
  /** 这一场在 `doc.content` 里的起止下标 —— 点大纲跳转用 */
  from: number;
  to: number;
}

export interface DerivedCharacter {
  name: string;
  /** 台词条数 */
  lines: number;
  /** 第一次出场在第几场 */
  firstScene: number;
}

export interface Derived {
  scenes: DerivedScene[];
  characters: DerivedCharacter[];
}

/* 场景标题里拆地点和时间：
     INT. 办公室 - 日     西文
     内景. 办公室 - 日     中文直译
     【场景：老茶馆】       中文剧本常见
   拆不出来不算错 —— 地点就是整个标题，时间留空。 */
const HEAD_WESTERN = /^\s*(?:INT|EXT|INT\.?\/EXT|I\/E)[.．\s]+(.+?)\s*[-–—]\s*(.+)$/i;
const HEAD_CN = /^\s*(?:内景|外景|内外景|内\/外)[.．\s]*(.+?)\s*[-–—·]\s*(.+)$/;
const HEAD_BRACKET = /^\s*【\s*(?:场景|地点|场)\s*[:：]\s*(.+?)\s*】\s*$/;
/** 去掉前缀但没有时间分隔的：`INT. 办公室` */
const HEAD_PREFIX_ONLY = /^\s*(?:INT|EXT|INT\.?\/EXT|I\/E|内景|外景|内外景|内\/外)[.．\s]+(.+)$/i;

export function splitHeading(heading: string): { location: string; timeOfDay: string } {
  const numbered = numberedHeading(heading);
  if (numbered) return { location: numbered[1], timeOfDay: numbered[2] || numbered[5] };
  for (const re of [HEAD_WESTERN, HEAD_CN]) {
    const m = re.exec(heading);
    if (m) return { location: m[1].trim(), timeOfDay: m[2].trim() };
  }
  const b = HEAD_BRACKET.exec(heading);
  if (b) return { location: b[1].trim(), timeOfDay: "" };
  const p = HEAD_PREFIX_ONLY.exec(heading);
  if (p) return { location: p[1].trim(), timeOfDay: "" };
  return { location: heading.trim(), timeOfDay: "" };
}

function textOf(node: { content?: { text?: string }[] }): string {
  return (node.content ?? []).map((c) => c.text ?? "").join("").trim();
}

/**
 * 遍历文档。
 *
 * **场景标题之前的内容也算一场**（第 0 场，标题为空）——
 * 很多人开头先写一段引子再写第一个场景标题，丢掉的话那段就没了。
 */
export function derive(doc: ScriptDoc | null | undefined): Derived {
  const content = doc?.content;
  if (!Array.isArray(content)) return { scenes: [], characters: [] };

  const scenes: DerivedScene[] = [];
  const charMap = new Map<string, DerivedCharacter>();
  let cur: DerivedScene | null = null;
  /** 最近一个角色名 —— 台词算在它头上 */
  let speaker: string | null = null;

  const ensureScene = (i: number) => {
    if (cur) return cur;
    cur = {
      index: scenes.length + 1,
      heading: "",
      location: "",
      timeOfDay: "",
      characters: [],
      dialogueCount: 0,
      from: i,
      to: i,
    };
    scenes.push(cur);
    return cur;
  };

  for (let i = 0; i < content.length; i++) {
    const node = content[i];
    const text = textOf(node);
    if (node.type === "metadata" || node.type === "caption") continue;

    if (node.type === "sceneHeading") {
      if (cur) cur.to = i - 1;
      const { location, timeOfDay } = splitHeading(text);
      cur = {
        index: scenes.length + 1,
        heading: text,
        location,
        timeOfDay,
        characters: [],
        dialogueCount: 0,
        from: i,
        to: i,
      };
      scenes.push(cur);
      speaker = null;
      continue;
    }
    if (!text) continue;

    const scene = ensureScene(i);
    const roster = /^人物\s*[:：]\s*(.+)$/.exec(text);
    if (roster && node.type === "action") {
      for (const rawName of roster[1].split(/[、，,]/)) {
        const name = speakerInfo(rawName).name;
        if (!name) continue;
        if (!scene.characters.includes(name)) scene.characters.push(name);
        if (!charMap.has(name)) charMap.set(name, { name, lines: 0, firstScene: scene.index });
      }
    }
    if (node.type === "characterCue") {
      const { name } = speakerInfo(text);
      speaker = name === "旁白" ? null : name;
      if (!speaker) continue;
      if (!scene.characters.includes(speaker)) scene.characters.push(speaker);
      const existing = charMap.get(speaker);
      if (existing) {
        // 已经见过：只更新首次出场（更小的场号）
        existing.firstScene = Math.min(existing.firstScene, scene.index);
      } else {
        charMap.set(speaker, { name: speaker, lines: 0, firstScene: scene.index });
      }
    } else if (node.type === "dialogue") {
      scene.dialogueCount += 1;
      if (speaker) {
        const c = charMap.get(speaker);
        if (c) c.lines += 1;
      }
    }
  }
  // 范围由两处决定就够了：换场时收上一场的尾（循环里 `cur.to = i - 1`），
  // 和这里收最后一场。**早先每个分支还各写一遍 `scene.to = i`** ——
  // 三处互相兜底，拆掉任何一处测试都不变红，等于都没被测到。
  if (cur && content.length > 0) cur.to = Math.max(cur.to, content.length - 1);

  return {
    scenes,
    // 台词多的排前面 —— 主角先出现，符合「先做主要角色」的顺序
    characters: [...charMap.values()].sort(
      (a, b) => b.lines - a.lines || a.firstScene - b.firstScene,
    ),
  };
}

/* ── 分镜草案 ───────────────────────────────────────────────────────────── */

export interface ShotDraft {
  audioType?: string;
  delivery?: string;
  /** 画面描述 */
  prompt: string;
  /** 这一镜的台词。空 = 没有台词 */
  dialogue: string;
  /** 说这句话的角色。空 = 旁白/无 */
  speaker: string;
  scale: string;
  cameraMovement: string;
  duration: number;
  /** 属于第几场 */
  scene: number;
  sceneLocation: string;
}

/**
 * 按结构切分镜。
 *
 * 规则简单到可以复述：**一句台词一镜，连续的动作合成一镜**。
 * 复杂的运镜设计交给模型或人 —— 这里要的是一个**不会错**的底稿，
 * 不是一个聪明的底稿。
 *
 * 台词的镜头默认给特写（说话的人要看得见嘴），动作给中景。
 */
export function deriveShots(doc: ScriptDoc | null | undefined): ShotDraft[] {
  const content = doc?.content;
  if (!Array.isArray(content)) return [];

  const shots: ShotDraft[] = [];
  let sceneIdx = 0;
  let sceneLoc = "";
  let speaker = "";
  /** 攒着的动作描写，遇到台词或换场时结成一镜 */
  let pending: string[] = [];

  const flushAction = () => {
    if (pending.length === 0) return;
    shots.push({
      prompt: pending.join(" "),
      dialogue: "",
      speaker: "",
      scale: "中景 (Medium)",
      cameraMovement: "固定 (Static)",
      duration: 4,
      scene: sceneIdx,
      sceneLocation: sceneLoc,
    });
    pending = [];
  };

  for (const node of content) {
    const text = textOf(node);
    if (node.type === "sceneHeading") {
      flushAction();
      sceneIdx += 1;
      sceneLoc = splitHeading(text).location;
      speaker = "";
      continue;
    }
    if (!text) continue;
    if (node.type === "characterCue") {
      speaker = text;
      continue;
    }
    if (node.type === "dialogue") {
      flushAction();
      if (sceneIdx === 0) sceneIdx = 1;
      const voice = speakerInfo(speaker);
      shots.push({
        // 画面描述：谁在哪。**台词不写进 prompt** ——
        // 它单独进 dialogue 字段，由出片那一步决定怎么用
        prompt: [voice.audioType === "dialogue" ? voice.name : "", sceneLoc].filter(Boolean).join("，"),
        dialogue: text,
        speaker: voice.name,
        audioType: voice.audioType,
        delivery: voice.delivery,
        scale: "特写 (Close-Up)",
        cameraMovement: "固定 (Static)",
        // 中文约每秒 5 字，最短 2 秒
        duration: Math.max(2, Math.min(12, Math.round(text.length / 5) + 1)),
        scene: sceneIdx,
        sceneLocation: sceneLoc,
      });
      continue;
    }
    if (node.type === "action") {
      if (sceneIdx === 0) sceneIdx = 1;
      pending.push(text);
    }
    // 括号提示和转场不单独成镜
  }
  flushAction();
  return shots;
}
