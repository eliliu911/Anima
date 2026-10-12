/**
 * 「把关键词改写成句子」这个动作**什么时候该给、什么时候别给**。
 *
 * # 方向是单向的
 *
 * 关键词串（`1girl, solo, masterpiece` 这种逗号堆）是 **CLIP 的语言**：
 * SDXL 的编码器把 77 token 分块取平均，短关键词正合适。
 * 而 Z-Image 的 Qwen3-4B、Chroma 的 T5-XXL、H3 的 Qwen3-VL 是长句原生的 ——
 * 同一串标签发过去不报错，只是大部分不起作用。
 *
 * 所以：**在句子引擎上把标签改成句子有用；在 SDXL 上反着来会更糟。**
 * 这个按钮必须知道当前是哪种引擎，否则就成了一个「点了让结果变差」的按钮。
 *
 * 判断中英语域的那份名单在词表里（`chineseEngines`），这里判的是另一件事：
 * 编码器吃不吃长句。两者恰好高度重合，但不是同一个概念 ——
 * Chroma 的 T5-XXL 吃长句，却不该发中文；Krea 2 的 Qwen3-VL 两样都吃。
 */
import { prefersChinese } from "../../shared/shotLanguage";

/** 编码器是长句原生的。写死是因为这是模型架构的事实，不是配置。 */
const SENTENCE_ENGINES = ["zimage", "z_image", "krea2", "qwen21", "h3", "minimax_h3", "ltx", "ltx25"];

export function isSentenceEngine(engineId: string): boolean {
  const e = engineId.trim().toLowerCase();
  return SENTENCE_ENGINES.includes(e);
}

/**
 * 一段文本像不像**一个标签**（而不是一个分句）。
 *
 * **不能只看字符数。** 第一版写的是「整段 ≤ 24 字就算标签」，
 * 而中文分句天然就是十来个字 ——「一个少女独自站在窗边」被判成了标签。
 * 中英要分开看：中文标签通常 ≤ 6 个字，英文标签通常 ≤ 3 个词。
 */
function tagLike(seg: string): boolean {
  const s = seg.replace(/[。！？.!?]+$/u, "").trim();
  if (!s) return false;
  const cjk = (s.match(/[\u4e00-\u9fff]/gu) ?? []).length;
  if (cjk > 0) return cjk <= 6;
  return s.split(/\s+/).filter(Boolean).length <= 3;
}

/**
 * 这串文本看着像不像关键词堆。
 *
 * 判据：逗号分出来至少 4 段，且**绝大多数段都是标签形态**。
 * 留出两成余量是因为用户常在标签串后面手写一句自己的描述。
 */
export function looksLikeTags(text: string): boolean {
  const t = text.trim();
  if (!t) return false;
  const parts = t.split(/[,，]/).map((s) => s.trim()).filter(Boolean);
  if (parts.length < 4) return false;
  const tags = parts.filter(tagLike).length;
  return tags / parts.length >= 0.8;
}

export interface ProseAdvice {
  /** 按钮能不能点。 */
  enabled: boolean;
  /** 鼠标停上去说什么。**不能点的时候要说清楚为什么。** */
  title: string;
  /** true = 值得点一下（当前是句子引擎，而提示词还是关键词堆）。 */
  suggested: boolean;
}

export function proseAdvice(engineId: string, prompt: string): ProseAdvice {
  const text = prompt.trim();
  if (!text) {
    return { enabled: false, title: "先写点什么，或者用上面的选择器填充", suggested: false };
  }
  if (!isSentenceEngine(engineId)) {
    return {
      enabled: false,
      title:
        `${engineId || "当前引擎"} 的编码器是 CLIP —— 77 token 分块取平均，` +
        "要的就是短关键词。改写成句子会更差，所以这里不提供。",
      suggested: false,
    };
  }
  const tags = looksLikeTags(text);
  return {
    enabled: true,
    title: tags
      ? "这串是关键词，而当前引擎的编码器是长句原生的 —— 改写成句子它才吃得进去"
      : "把提示词改写成更连贯的句子。内容不变，只改说法",
    suggested: tags,
  };
}

/** 改写结果按引擎取中文还是英文那一份。和词表用同一个判断。 */
export function pickProse(
  result: { prompt_cn: string; prompt_en: string },
  engineId: string,
): string {
  return prefersChinese(engineId) ? result.prompt_cn : result.prompt_en;
}
