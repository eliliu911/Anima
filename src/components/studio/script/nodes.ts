/**
 * 剧本的六种排版节点。
 *
 * 结构照 LumenX（MIT，Copyright 2026 Alibaba）的 ScriptEditor，
 * 但**只取最小完备集**：场景标题 / 动作 / 角色名 / 对白 / 括号提示 / 转场。
 * 它那边还有 Section、DualDialogue、ShotBlock、Note ——
 * 双人对白是英文剧本的排版惯例，中文短剧用不上；
 * 内嵌分镜我们在第 4 步有专门的工作台，两处能编就是两个所有者。
 */
import { Node } from "@tiptap/core";

/** 每种节点渲染成一个带 `data-line` 的 div，样式全在 CSS 里按属性选。 */
function line(name: string, tag = "p") {
  return Node.create({
    name,
    group: "block",
    content: "inline*",
    defining: true,
    parseHTML() {
      return [{ tag: `${tag}[data-line="${name}"]` }];
    },
    renderHTML() {
      return [tag, { "data-line": name, class: `script-${name}` }, 0];
    },
  });
}

/**
 * **`Action` 必须第一个注册。**
 *
 * ProseMirror 用 block 组里的**第一个**成员去填充 `block+` 的空文档。
 * 它的内容是 `inline*`（可以为空），填充能立刻终止；
 * 换成一个自身要求 `block+` 的节点排在前面，`createAndFill` 会无限递归 ——
 * 空文档初始化时直接爆栈。这条是 LumenX 踩过的，注释原样留下。
 */
export const Action = line("action");
export const SceneHeading = line("sceneHeading", "h2");
export const CharacterCue = line("characterCue").extend({
  addAttributes() {
    return { name: { default: "" }, delivery: { default: "" }, audioType: { default: "dialogue" } };
  },
});
export const Dialogue = line("dialogue");
export const Parenthetical = line("parenthetical");
export const Transition = line("transition");
export const Metadata = line("metadata");
export const Caption = line("caption");

export const scriptNodes = [
  Action,
  SceneHeading,
  CharacterCue,
  Dialogue,
  Parenthetical,
  Transition,
  Metadata,
  Caption,
];

