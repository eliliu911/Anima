import { cinemaPolishPrompt, type PolishArgs } from "../../api/polish";
import { pickProse } from "../../components/studio/prosePolish";

export async function polishCanvasPrompt(args: PolishArgs, isCurrent: () => boolean, apply: (text: string) => void) {
  const result = await cinemaPolishPrompt(args);
  if (!isCurrent()) return "输入或项目已变化，未应用旧润色结果";
  const text = pickProse(result, args.engine ?? "").trim();
  if (!text) throw new Error("润色未返回有效提示词，原文已保留");
  apply(text);
  return result.warning ?? "";
}
