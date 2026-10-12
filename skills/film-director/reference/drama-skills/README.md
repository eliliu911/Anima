# 分镜与视频提示词手艺参考 —— 来自 drama-skills

这一整个目录**是别人的东西，原样收进来的**，不是我们写的。

| | |
|---|---|
| 上游 | `drama-skills` · `skills/short-drama-storyboard/` 与 `skills/short-drama-video-prompts/` |
| 版本 | `bc96c5e`（2026-09-10） |
| 许可 | MIT — Copyright (c) 2026 drama-skills contributors（见同目录 `LICENSE`） |
| 取回时间 | 2026-09-13 |
| 本机路径 | `/Users/you/WorkSpace/drama-skills` |

## 为什么收

分镜重构（`docs/plans/2026-09-13-分镜重构方案.md`）的镜头表字段、切镜规则、参考图槽位写法、
H3 模式选择和无对白镜写法，都照这两份 SKILL 定。我们自己写的只有「这台机器上实测出来的结论」。

**原样收，不压成提示词、不手改** —— 和编剧那份同一个理由：手艺要有单一来源，
上游改了重新拷一遍即可；我们的实测结论写进 `../../SKILL.md` 或另起 `anima-*.md`。

## 收了哪些

| 目录 | 文件 | 用来定什么 |
|---|---|---|
| `storyboard/` | `SKILL.md` | 每镜唯一职责、来源、起点 → 唯一动作 → 终点、参考图槽位（用途 / 控制 / 不得控制） |
| | `shot-craft.md` | 对白估时、切镜必须带来变化、承受者一镜、时长与动作量 |
| | `production-shot-grammar.md` | 实际制作需要的镜头语法 |
| | `blocking-playbooks.md` | 多人与复杂空间调度 |
| | `keyframe-craft.md` | 冻结关键帧（分镜提示词 / 首帧） |
| `video-prompts/` | `SKILL.md` | 文生 / 图生判定、参考图准备度、正文语言 |
| | `minimax-h3.md` | **H3 方言**：三段 / 六段结构、模式选择表、`<Subject N>` 编号、无对白镜、配乐必须 `N/A` |
| | `motion-recipe.md`、`performance-action-timing.md` | 动作与表演的时间安排 |
| | `camera-audio-continuity.md` | 运镜与声音连续性 |
| | `generability.md` | 什么能生成、什么生成不了 |
| | `delivery-profile.md`、`production-prompt-grammar.md` | 容器算术（一次生成装几镜）、提示词语法 |

## 少收了哪几份，为什么

| 没收 | 理由 |
|---|---|
| `stage-contract.md`（两份） | 讲 drama-skills 自己的阶段边界，对我们没有意义 |
| `seedance-2.0.md`、`seedance-2.5.md` | 我们只跑本机 H3 / LTX，不跑 Seedance |
| `target-model-profile.md` | 他们的项目档案格式，我们的线路和模型设置由 `ModelSettings` 定 |
| `comic-keyframe-lexicon.md`、`screenplay-to-keyframe-example.md`、`scene-visual-plan.md`、`coverage-audition.md`、`shot-revision-identity.md`、两份 `review-and-fixtures.md` | 分镜重构第一版用不到；需要时再按上面的办法补拷 |

## 另外一份：`../director-skills/`

`kangarooking/director-skills` 的 `travel-skill`（MIT，Copyright (c) 2026 kangarooking），只收两份：
`motivated-lighting-library.md`（光影字段照它的布光公式）和 `failure-catalog.md`（症状 → 修复对照）。
它针对 Seedance / 可灵；「一次生成塞多个景别会乱切」那条不适用于 H3（官方支持镜内切换）。
