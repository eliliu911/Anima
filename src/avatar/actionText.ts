/**
 * 把动作标签从要**显示**的文本里剥掉。
 *
 * 后端在送去 TTS 之前已经剥过一遍，但 `assistant_delta` 是**逐 token 发的
 * 原文** —— 界面上拿到的是没剥过的版本，于是气泡里会出现
 * `<action:heart>好啊…</action:heart>`（用户实测撞到过）。
 *
 * 为什么在渲染时剥、而不是在收到 delta 时剥：一个标签会被切在两个 delta
 * 之间（`<acti` + `on:heart>`），逐块剥的话两半都漏。渲染时看到的是**累计**
 * 的整段文本，标签一定是完整的。
 *
 * 开标签和闭标签都认 —— 模型很爱写成对的。
 */
const TAG = /<\/?action:[^>]*>/g;

/**
 * 结尾处**还没写完**的标签。流式输出时它是一个字一个字长出来的
 * （`<a` → `<ac` → `<action:hea`），不藏的话用户会看着符号一路蹦。
 *
 * 只匹配 `<action:` 的各级前缀 —— 单独一个 `<` 不管，
 * 那更可能是正文里的小于号（"a < b"）。
 */
const PARTIAL = /<\/?(?:a|ac|act|acti|actio|action|action:[^>]*)$/;

/**
 * 模型有时会顺手编出别的标记 —— 实测收到过
 * `<span></span> 好啊，伸个懒腰～<span></span>`。
 *
 * 判据是"尖括号 + 字母开头"：正文里的小于号不长这样
 * （`1 < 2` 后面是空格和数字，`a<b` 没有闭合的 `>`）。
 */
const MARKUP = /<\/?[a-zA-Z][^>]*>/g;

export function stripActionTags(text: string): string {
  // 连尖括号都没有就不用做了 —— 这个函数每帧都会被调用
  if (!text.includes("<")) return text;
  return text.replace(TAG, "").replace(MARKUP, "").replace(PARTIAL, "");
}
