import { useEffect, useRef, useState, type InputHTMLAttributes, type TextareaHTMLAttributes } from "react";

/**
 * 一个**中文能正常打**的输入框。
 *
 * # 不用它会怎样
 *
 * 直接 `value={x} onChange={e => 存起来(e.target.value)}` 的输入框，
 * 用拼音打中文时会打出这种东西：
 *
 * ```text
 * yyuyu'jyu'jiyu'jinyu'jiyu'jiyu'jyu'jyuyuyy
 * ```
 *
 * 拼音串原样进去了。因为每敲一个字母都会往上写一次，父组件把**存下来的
 * 值**回灌进 `value`，而那次重渲染把输入法的**组合缓冲**冲掉了 ——
 * 输入法还在拼字，字却已经被换过一轮。
 *
 * 英文察觉不到（一个字母就是一个字符），所以这类 bug **只有真的用中文
 * 打字才会发现**，而这是个中文工具。
 *
 * # 为什么单独抽出来
 *
 * 这个项目里 `onCompositionStart` 已经手写了五处以上（`ShotCard`、
 * `PromptNode`、`CharacterNode`、`PromptMentionEditor`…），每处一份。
 * 我又加了两个没写的，用户当场撞上。**一个每次都要记得的约定等于
 * 没有约定** —— 所以把它变成一个组件。
 *
 * 新加输入框时：能用这个就用这个。
 *
 * # 多行的用 {@link ComposingTextarea}
 *
 * 2026-09-20 又撞了一次：画布的「试音色」节点里写了个裸 `<textarea>`，
 * 打拼音出来是 `qqiqinqinaqin'aiqin'ai'd`。**当时这个组件只包 `<input>`** ——
 * 于是「能用这个就用这个」这句话对多行输入框不成立，等于对一半的场景
 * 没有约定。所以补上了 textarea 版，两边共用同一套组合态处理。
 */
export interface ComposingInputProps
  extends Omit<InputHTMLAttributes<HTMLInputElement>, "value" | "onChange"> {
  value: string;
  /** 值真正定下来时调一次。**拼字过程中不会调。** */
  onCommit: (value: string) => void;
}

/** 组合态处理本身。`<input>` 和 `<textarea>` 共用 —— 写两份必然漂移。 */
function useComposing(value: string, onCommit: (v: string) => void) {
  const [draft, setDraft] = useState(value);
  /** 输入法正在拼字。**用 ref 不用 state** —— 它要在同一次事件里就生效。 */
  const composing = useRef(false);
  /** 上一次发出去的值。用来分清「外面改的」和「我们自己刚发的」。 */
  const emitted = useRef(value);

  useEffect(() => {
    // **正在拼字时绝不动 `draft`** —— 动了就是把输入法的缓冲冲掉。
    // 外面真的改了（不是我们自己刚发出去那次）才跟上。
    if (!composing.current && value !== emitted.current) {
      emitted.current = value;
      setDraft(value);
    }
  }, [value]);

  const emit = (next: string) => {
    emitted.current = next;
    onCommit(next);
  };

  return {
    draft,
    handlers: {
      onChange: (e: { target: { value: string } }) => {
        const next = e.target.value;
        setDraft(next);
        // 拼字过程中只更新本地，不往上写
        if (!composing.current) emit(next);
      },
      onCompositionStart: () => { composing.current = true; },
      onCompositionEnd: (e: { currentTarget: { value: string } }) => {
        composing.current = false;
        const next = e.currentTarget.value;
        setDraft(next);
        emit(next);
      },
      // 失焦时补一次 —— 拼字拼到一半就点走的情况下，
      // `compositionend` 可能已经发过但值又变了
      commitOnBlur: (v: string) => { if (v !== emitted.current) emit(v); },
    },
  };
}

/** 多行版。和 {@link ComposingInput} 一样，只是渲染 `<textarea>`。 */
export interface ComposingTextareaProps
  extends Omit<TextareaHTMLAttributes<HTMLTextAreaElement>, "value" | "onChange"> {
  value: string;
  onCommit: (value: string) => void;
}

export function ComposingTextarea({ value, onCommit, onBlur, ...rest }: ComposingTextareaProps) {
  const { draft, handlers } = useComposing(value, onCommit);
  return (
    <textarea
      {...rest}
      value={draft}
      onChange={handlers.onChange}
      onCompositionStart={handlers.onCompositionStart}
      onCompositionEnd={handlers.onCompositionEnd}
      onBlur={(e) => { handlers.commitOnBlur(e.currentTarget.value); onBlur?.(e); }}
    />
  );
}

export function ComposingInput({ value, onCommit, onBlur, ...rest }: ComposingInputProps) {
  const { draft, handlers } = useComposing(value, onCommit);

  return (
    <input
      {...rest}
      value={draft}
      onChange={handlers.onChange}
      onCompositionStart={handlers.onCompositionStart}
      onCompositionEnd={handlers.onCompositionEnd}
      onBlur={(e) => { handlers.commitOnBlur(e.currentTarget.value); onBlur?.(e); }}
    />
  );
}
