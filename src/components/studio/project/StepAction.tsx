/**
 * 步骤页把**自己的主操作**放进共用页头右上角。
 *
 * # 为什么要绕一层 portal
 *
 * LumenX 的版式是「主操作在 banner 区、和页面标题同一行」——
 * 「生成分镜」在故事板页头右上，和 QUEUE 并排。这个位置是对的：
 * 它是这一页唯一那个「让事情发生」的按钮，不该混在内容区的工具条里。
 *
 * 但页头 `StepHeader` 是六个步骤共用的，而按钮的状态（正在生成、
 * 有没有剧本）只有步骤组件自己知道。把状态提到 ProjectPage 去，
 * 等于让父组件替六个孩子各存一份互不相干的状态。
 *
 * 所以走 portal：页头留一个挂载点，谁需要谁挂。
 * **挂载点还没就绪时渲染 null 而不是就地渲染** —— 就地渲染的话
 * 按钮会在内容区里闪一下再跳到页头，比不显示更糟。
 */
import { createContext, useContext, type ReactNode } from "react";
import { createPortal } from "react-dom";

export const StepActionSlot = createContext<HTMLElement | null>(null);

export function StepAction({ children }: { children: ReactNode }) {
  const el = useContext(StepActionSlot);
  // **挂载点还得**在文档里**。**
  //
  // 挂载点是个存在 state 里的 DOM 节点。页头被重建时（开发期的
  // Fast Refresh 最典型：state 保留、DOM 重建），state 里还是那个
  // 已经脱离文档的旧节点 —— 往它里面 portal 不报任何错，
  // 内容渲染出来了，只是渲染在一个看不见的地方。
  //
  // 表现就是「按钮突然没了」，而代码、CSS、布局全是对的。
  if (!el || !el.isConnected) return null;
  return createPortal(children, el);
}
