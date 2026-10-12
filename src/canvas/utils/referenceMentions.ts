import { useEffect, useRef } from "react";

export interface MentionFamily {
  prefix: string;
  prevIds: string[];
  nextIds: string[];
}

export interface NameMappingFamily {
  prevNames: string[];
  nextNames: string[];
}

export interface ReferenceMentionFamilyInput {
  prefix: string;
  ids: string[];
  names?: string[];
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

export function sameOrder(a: string[], b: string[]): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i += 1) {
    if (a[i] !== b[i]) return false;
  }
  return true;
}

/**
 * 保持提示词中的 `@图片N` 或 `@角色名` 与参考图连线/顺序一一对应。
 * 当重命名角色、删除中间某张图或重排连线时，自动重新对号，并移除已失效的 mention。
 */
export function remapReferenceMentions(
  prompt: string,
  families: MentionFamily[],
  nameFamilies?: NameMappingFamily[],
): string {
  let out = prompt;

  // 1. 处理名称重命名 (如 筱筱 -> 小雅)
  if (nameFamilies) {
    for (const { prevNames, nextNames } of nameFamilies) {
      if (sameOrder(prevNames, nextNames)) continue;
      prevNames.forEach((oldName, idx) => {
        const newName = nextNames[idx];
        if (oldName && newName && oldName !== newName) {
          const escaped = escapeRegExp(oldName);
          out = out.replace(new RegExp(`@${escaped}(?=[\\s，,。!！?\\s]|$)`, "g"), `@${newName}`);
        }
      });
    }
  }

  // 2. 处理 @图片N 序号重映射
  for (const { prefix, prevIds, nextIds } of families) {
    if (sameOrder(prevIds, nextIds)) {
      continue;
    }
    const pattern = new RegExp(`@${escapeRegExp(prefix)}\\s*(\\d+)(\\s?)`, "g");
    out = out.replace(pattern, (full, numStr: string, trailing: string) => {
      const oldIndex = Number(numStr);
      const node = prevIds[oldIndex - 1];
      if (node === undefined) {
        return full;
      }
      const nextIndex = nextIds.indexOf(node);
      if (nextIndex === -1) {
        // 引用被删了 -> 连同其后随空格一起移除
        return "";
      }
      return `@${prefix}${nextIndex + 1}${trailing}`;
    });
  }
  return out;
}

const FIELD_SEP = "\u241F";
const FAMILY_SEP = "\u241E";

export function useReferenceMentionSync(
  prompt: string,
  families: ReferenceMentionFamilyInput[],
  applyPrompt: (next: string) => void,
): void {
  const prevRef = useRef<Map<string, { ids: string[]; names: string[] }> | null>(null);
  const signature = families
    .map((family) =>
      family.prefix +
      FIELD_SEP +
      family.ids.join(FIELD_SEP) +
      FIELD_SEP +
      (family.names ?? []).join(FIELD_SEP)
    )
    .join(FAMILY_SEP);

  useEffect(() => {
    const prev = prevRef.current;
    prevRef.current = new Map(
      families.map((family) => [
        family.prefix,
        { ids: family.ids, names: family.names ?? [] },
      ] as const),
    );
    if (!prev) return;

    let changed = false;
    const remapFamilies: MentionFamily[] = [];
    const nameFamilies: NameMappingFamily[] = [];

    for (const family of families) {
      const prevData = prev.get(family.prefix) ?? { ids: [], names: [] };
      if (!sameOrder(prevData.ids, family.ids)) changed = true;
      if (!sameOrder(prevData.names, family.names ?? [])) changed = true;

      remapFamilies.push({
        prefix: family.prefix,
        prevIds: prevData.ids,
        nextIds: family.ids,
      });

      nameFamilies.push({
        prevNames: prevData.names,
        nextNames: family.names ?? [],
      });
    }

    if (!changed) return;

    const next = remapReferenceMentions(prompt, remapFamilies, nameFamilies);
    if (next !== prompt) applyPrompt(next);
  }, [signature, prompt, applyPrompt]);
}

/**
 * 将提示词中的 `@角色名` 或 `@图片N` 编译为底模协议。
 *
 * **`references` 必须是实际下发给模型的那个数组**（`videoInputReferences`
 * 的返回值），不能是节点上的 `data.references` —— 后者不含沿图追加的参考、
 * 也没做按路径去重，用它的下标算 `<Picture N>` 会静默错位到别人脸上。
 *
 * 只有支持 Picture 槽位的链路（MiniMax H3 的 Ref2V）才编译成
 * `<Picture 1>` / `<Picture 2>`。其余模式（i2v / t2v / fl2v）**没有槽位**，
 * 这时把 `@` 记号摘掉、保留名字本身 —— 原来这些模式压根不编译，
 * 字面量 `@筱筱` 直接发给模型，不报错，只是画面里没有那个角色。
 */
export function compilePromptMentions(
  prompt: string,
  references?: Array<{ name?: string; path?: string }>,
  engine?: string,
): string {
  if (!prompt) return prompt;

  // Picture 槽位是 H3 Ref2V 的协议，别的引擎不认这个记号。
  // 注意这里**只看引擎**：`@图片2` 是用户显式给的槽位序号，
  // 即使这次没解析出带名字的参考，它照样该编译成 <Picture 2>。
  const supportsPictureTokens = engine === "h3";
  let compiled = prompt;

  // 1. 已知名称的参考图：`@角色名` 精准映射到它在**下发顺序**里的位置
  if (references && references.length > 0) {
    const sorted = references
      .map((ref, idx) => ({ ref, idx, name: ref.name?.trim() }))
      .filter((item): item is { ref: { name?: string; path?: string }; idx: number; name: string } =>
        Boolean(item.name && item.name !== "画布图片" && item.name !== "参考图")
      )
      .sort((a, b) => b.name.length - a.name.length);

    for (const { idx, name } of sorted) {
      const escaped = escapeRegExp(name);
      compiled = compiled.replace(
        new RegExp(`@${escaped}`, "g"),
        supportsPictureTokens ? `<Picture ${idx + 1}>` : name,
      );
    }
  }

  // 2. `@图片N` / `@图片 N`
  compiled = compiled.replace(/@图片\s*(\d+)/g, (_match, numStr: string) =>
    supportsPictureTokens ? `<Picture ${numStr}>` : "",
  );

  // 3. 没能对上任何参考的 `@名字`：摘掉 @ 记号，别把它当字面量发出去
  compiled = compiled.replace(/@([^\s，,。!！?？、]+)/g, "$1");

  return compiled.replace(/\s{2,}/g, " ").trim();
}

/**
 * `@音频N` / `@音频名` 编译成 `<Audio N>`（画布音频素材，2026-10-08）。
 *
 * **要在 `compilePromptMentions` 之前跑** —— 它最后一步会把对不上图片的 `@名字` 摘成纯文字，
 * 音频的名字就被吃掉了。编号按 `videoInputAudios` 的顺序（和发出去的一致），和 `<Picture N>` 各自独立。
 */
export function compileAudioMentions(prompt: string, audios: Array<{ name?: string }>, engine?: string): string {
  if (!prompt || engine !== "h3") return prompt;
  let out = prompt;
  const named = audios
    .map((a, i) => ({ i, name: a.name?.trim() }))
    .filter((x): x is { i: number; name: string } => Boolean(x.name && x.name !== "音频" && x.name !== "音频素材"))
    .sort((a, b) => b.name.length - a.name.length);
  for (const { i, name } of named) out = out.replace(new RegExp(`@${escapeRegExp(name)}`, "g"), `<Audio ${i + 1}>`);
  return out.replace(/@音频\s*(\d+)/g, (_m, n: string) => `<Audio ${n}>`);
}
