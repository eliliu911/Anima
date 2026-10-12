# 编剧手艺参考 —— 来自 drama-skills

这一整个目录**是别人的东西，原样收进来的**，不是我们写的。

| | |
|---|---|
| 上游 | `drama-skills` · `skills/short-drama-write/references/` |
| 许可 | MIT — Copyright (c) 2026 drama-skills contributors |
| 取回时间 | 2026-09-11 |
| 本机路径 | `/Users/you/WorkSpace/drama-skills` |

## 为什么原样收，而不是压成提示词

压成提示词会分叉：上游改了我们跟不上，而我们改了也说不清改的是谁的。
镜头语言那次已经得出过同一个结论 —— **手艺要有单一来源**。

所以这里的文件**不要手改**。要补我们自己的实测结论，写进
`../SKILL.md` 或另起一份 `anima-*.md`，别混进上游的原文里。
重新同步就是把上游对应文件再拷一遍。

## 少收了哪几份，为什么

| 没收 | 理由 |
|---|---|
| `stage-contract.md` | 讲的是 drama-skills 自己的阶段边界（哪个 skill 管哪一段），对我们没有意义 |
| `production-format-dialect.md` | 项目特有的制作稿方言，我们的格式由 `script/format.ts` 定 |
| `scene-handoff-capsule.md` | 上下文续跑用的，不是手艺 |

## 谁在用

`src-tauri/src/screenwriter.rs` 把它们编译进二进制（`include_str!`），
按输入里出现的信号**按需**挑其中几节塞进 `ScreenplayFromSource`。

**按需是抄上游的设计**：它自己的 SKILL.md 就写着「默认只读本 SKILL 和
直接输入，出现下列明确信号时才读取对应知识」。1300 行全塞进提示词，
真正要紧的那几条会被稀释掉 —— 而提示词越长越靠后越容易被忽略，
这一点在出图那边已经实测过。
