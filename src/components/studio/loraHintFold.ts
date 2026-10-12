/**
 * LoRA 说明的折叠逻辑。
 *
 * 说明从「一句话」变成「实测 + 出处 + 推荐权重 + 坑」之后就长了 ——
 * 20 条 LoRA 全部铺开，列表会长到没法扫。所以默认折成一行，点开看全文。
 *
 * **但不能一刀切全折。** 短说明本来就一行，折起来等于多一次点击换零收益，
 * 而且「展开之后和折叠时一模一样」会让人以为没反应。所以只折真的超长的。
 *
 * 抽成纯函数是为了能测 —— 这类阈值逻辑写在 JSX 里就只能靠眼睛验。
 */

/** 超过这个字数才值得折。中文一行大约 24~28 字，两行以内不折。 */
export const FOLD_OVER = 56;

export function shouldFold(text: string): boolean {
  return text.trim().length > FOLD_OVER;
}

/**
 * 折叠时显示的摘要。
 *
 * **按边界截，不按字数硬切** —— 硬切会把「高跟鞋 + 手提包」斩成「高跟鞋 + 手」，
 * 读起来像渲染出错了。三级回退：
 *
 * 1. 句子结束（。！？；和英文的 `. `）—— 最干净，摘要自成一句
 * 2. 词组边界（，、+ 空格）—— 次好，至少不断在词中间
 * 3. 都没有才硬切，**并加省略号明说被截断了**（不加的话会被当成全文）
 *
 * 两级都要求切点在上限一半之后，否则摘要只剩两三个字，等于没有。
 */
const SENTENCE_MARKS = ["。", "！", "？", "；", ". ", "! ", "? ", "; "];
const PHRASE_MARKS = ["，", "、", ", ", " + ", " —— "];

export function foldSummary(text: string, limit = FOLD_OVER): string {
  const t = text.trim();
  if (t.length <= limit) return t;
  const head = t.slice(0, limit);
  const lastOf = (marks: string[]) =>
    marks.reduce((best, m) => Math.max(best, head.lastIndexOf(m)), -1);

  const sentence = lastOf(SENTENCE_MARKS);
  if (sentence >= limit / 2) return t.slice(0, sentence + 1);

  const phrase = lastOf(PHRASE_MARKS);
  if (phrase >= limit / 2) return `${t.slice(0, phrase).trimEnd()}…`;

  return `${head.trimEnd()}…`;
}
