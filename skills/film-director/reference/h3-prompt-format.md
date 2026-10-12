# MiniMax H3 的提示词格式（T2VA / I2VA / FL2VA / L2VA）

> **2026-10-06：以官方原文为准** —— `h3-official/`（HF `MiniMaxAI/MiniMax-H3@42ed227`，编译时嵌入 `src-tauri/src/h3_spec.rs`，
> 合成时按生产方式把对应章节原文喂给写手）。本文是早先的中文摘要，和原文冲突时听原文。
> 项目现在的写法：纯文生 = T2VA、首帧 = I2VA、首尾帧 = FL2VA（三字段，**不用 `<Subject N>`**）；参考图 / 接上一镜 / 长镜头 = Ref2VA 六段。
> `[Shot 1]` 不带时间码，镜内时刻写 `Around the N.N-second mark,`，`At MM:SS.mmm` 只标切镜；画面文字用英文双引号。

H3 不是「一段描述 → 一段视频」。它期待一份**有结构的稿子**：三个具名字段、
带编号的镜头、带说话人 ID 的台词。写成散文它也出片，只是**画面和声音对不上**，
而这件事不报错。

来源：`ComfyUI-MiniMaxH3-Prompt-Writer` 的官方指南（MIT）。全文在
`ComfyUI/custom_nodes/ComfyUI-MiniMaxH3-Prompt-Writer/guides/`，
这里只留会改变输出的那些规则。

## 1. 三个字段，顺序固定

```text
integrated_multimodal_description: [Shot 1] ...

overall_soundscape: ...

non_diegetic_music: ...
```

| 字段 | 装什么 | 不装什么 |
|---|---|---|
| `integrated_multimodal_description` | 沿时间轴的画面、动作、运镜、说话人、台词、**画面内**的声音 | — |
| `overall_soundscape` | 全片环境音、动作音、非语言人声（风雨、脚步、衣料、呼吸、笑） | 台词、唱词、画面内音乐 —— 那些已经在上面了，重复会打架 |
| `non_diegetic_music` | 只有观众听得见的配乐：乐器、速度、节奏、强弱变化 | 抽象情绪词（"忧伤的"）、配乐的作用说明 |

角色听得见的音乐（收音机、电视、有人在唱）是**画面内事件**，写进第一个字段，
不是 `non_diegetic_music`。这两个字段没有内容时写 `N/A` —— 留空不等于没有。

> **`non_diegetic_music: N/A` 这一行不能省，有实测支撑**（来自
> `zenstory-ai/drama-skills` 的产线数据）：正文写了这一行的 **14 次生成没有
> 一次自带配乐**；完全不提配乐的 **3 次里有 2 次返回了自造的背景音乐**
> （持续谐波音床，随镜头变化换和弦）。
>
> **语音通道没有观察到同样的现象** —— 会自己编的是配乐通道。
> 本项目 2026-09-10 的单组对照（均值 −40.2 vs −30.3 dB、说话间隙中位
> −69 vs −56 dB、谱平坦度 0.63 vs 0.41）方向一致。

### 1.1 `overall_soundscape` 的「人声」栏只写名词短语（2026-09-21 实跑翻车）

上表那句「不装台词」不只是"别把台词抄两遍"，**它连提都不能提**。

EP06 实跑：`人声：` 那一栏我写了

```text
人声：无 —— 这段静默是全片唯一一句台词之前的蓄力。
```

**H3 把这句解释当成要念的台词念了出来**，而真正的台词被提前说了（用户原话：
「把这段当成了台词，真正的台词被提前说了」）。

所以这一栏的硬规矩：

- 只写**声音本身的名词短语**：`短促的惊叫与抽气，无词` / `远处隐约的讲课声`
- 没有非语言人声就写 `无`，**句号都不加**
- **不写时刻、不写「台词 / 对白」这两个词、不写任何解释或指令**
- 台词的时刻、语气、说完的目标，全部只写在 `integrated_multimodal_description` 里

同样的话对另外三层也成立，只是人声这一栏后果最直接 —— 它离语音通道最近。

> **这条没有任何自动检查拦得住。** 画布上的提示词是手写的，不走
> `prompt_route_b::check_wording`。扫一遍现有工作流用这个判据：
> `人声：` 后面出现「台词 / 对白 / 秒 / 怒吼」或者超过三十来个字，就是踩了。

### 1.2 声音层里不要用「」包拟声词（2026-09-22 实跑翻车）

这是 1.1 的同一个通道在漏，只是换了个入口。EP07 的动作层写了

```text
动作层：11.5 秒单手猛烈拍在课桌木板上的巨响「砰！」
```

**H3 把那个「砰！」念成了老师说话。** `「…」` 这个写法和台词标记长得一模一样。

对照很干净：

| | 声音层里的引号拟声词 | 台词表现 |
|---|---|---|
| EP01（跑通过） | **0 处** | 老师在 11.80 秒正常说出那一句 |
| EP06 | 7 处 | 人声栏的解释句被念出来、真台词被提前 |
| EP07 | 6 处 | 拍桌的「砰！」变成老师说话 |

**拟声词写成描述，不要写成引号里的字**：

```text
✗  巨响「砰！」          ✓  一记沉闷的巨响
✗  「咔嗒！」金属别死声   ✓  压到卡死时那一下清脆的金属别死声
✗  「嗒！嗒！嗒！」踏步声  ✓  踏步声，一声比一声近
✗  闷炸「轰隆隆隆！！」    ✓  毁天灭地的连绵闷炸
```

连同 1.1 一起是同一条规矩：**`overall_soundscape` 里除了描述声音的名词短语，
什么都不要放** —— 不放解释、不放时刻、不放「台词/对白」这两个字、不放引号里的拟声词。
能进语音通道的只有 `integrated_multimodal_description` 里 `<d>` 标签包着的那一句。

## 2. 镜头与切点

第一个镜头**不带时间戳**。之后每个镜头以严格递增的切点开头，且必须落在时长内：

```text
[Shot 1] Live-action, cinematic, a medium-wide shot frames...
[Shot 2] At 00:03.500, the camera cuts to...
```

风格和初始构图写在 `[Shot 1]` 的开头：`Cinematic` / `live-action` /
`2D-animated` / `3D CG` / `claymation` / `watercolor` / `vintage film`。
有参考图时风格**从图里读**，不要另起一套。

切点的措辞：`the camera cuts to` / `the shot cuts to` / `the shot transitions to` /
`the shot changes to` / `the shot switches to`。用户明确要求时才用叠化、淡入淡出、划像。

> **一刀要带来新信息**（新的主体、空间、状态、视角或时间）。
> 只是想换个景别或微调角度，用运镜，不要切。

## 3. 运镜 = 类型 + 幅度 + 速度

写成镜头里的自然英文句子，**不是句尾堆标签**。中等幅度、正常速度就省略不写。

| 维度 | 词 |
|---|---|
| 类型 | `Zoom In/Out`（焦段变、机身不动）· `Push In / Pull Out`（机身进退） |
| | `Pan Left/Right`（原地横摇）· `Truck Left/Right`（机身平移） |
| | `Tilt Up/Down`（原地纵摇）· `Pedestal Up/Down`（整机升降） |
| | `Arc Shot`（绕拍）· `Tracking Shot`（跟拍）· `Static Shot`（不动） |
| | `Shake Slightly/Strongly` · `POV` · `Roll Clockwise/Counterclockwise` |
| 幅度 | `with small amplitude` / `with large amplitude` |
| 速度 | `at slow speed` / `at fast speed` |

```text
The camera pushes in with small amplitude at slow speed toward the folded letter in her hands.
The camera pans right with large amplitude at fast speed, revealing the open doorway.
```

## 4. 说话人和台词 —— 这块最容易静默出错

发声的主体给稳定 ID：`(S1)` `(S2)`；一起说的写 `(S1,S2)`。**同一个人跨镜头 ID 不变。**
从不出声的角色**不给 ID**。

```text
The young woman with a quiet, breathy voice (S1) says: <d>[English] I get off at the next station.</d>
```

- `<d>` **里面只放语言标签和台词原文**。说话人的身份、动作、语气全部放在 `<d>` 外面。
- 台词**逐字保留，连标点一起**，不翻译、不改写。
- 说话人首次出现时把身份交代够：类型、年龄、性别、在不在画面里、音高、音色、语速、口音。
- **那个称呼必须在画面描写里出现过 —— 这条官方指南没写，是本机实测出来的。**

  2026-09-10 T2VA 对照，每组同 seed 同内容，只改说话人的写法：

  | 画面描写 | 说话人写成 | 结果 |
  |---|---|---|
  | `A young woman and an old villager…` | `沈星晴` / `老村民` | **两句话全归女角色，男角色没开口** |
  | `A young woman in a denim jacket…` | `The young woman in the denim jacket` | 对 |
  | `Shen Xingqing, a young woman…` | `Shen Xingqing` | 对 |
  | 同上，但**完全不写说话人** | —— | **也对** |

  用描述短语还是用名字都行，**但那个词得在画面描写里指得到人**。
  指向画面里不存在的东西时，模型会把它并进唯一沾边的那个人，
  于是双人对话塌成一个人在说 —— 而且**不报错**。

  最后一行是关键：**不标记是安全的**。H3 自己能从台词内容和场景推断出
  两个人在对话。所以对不上时宁可不发 ID —— **标错比不标更差**。
  本项目的 `h3_prompt::speakers_anchored` 就是这道护栏。
- 旁白用固定说法 `says in an off-screen voiceover`，**紧接着**要写出画面里那个人嘴唇不动：

  ```text
  The man (S1) says in an off-screen voiceover: <d>[English] I still remember that road.</d> while his lips remain completely closed.
  ```
- 一句话跨越切点：两边都插 `<scenetrans>`，并明写声音连过去
  （`continues seamlessly across the cut` / `carries over from the previous shot`…）。
  被片尾截断用 `<cutoff>`。

> **本项目实测**：H3 的编码器是 Qwen3-VL，`<d>[Chinese] …</d>` 里的中文台词会
> 变成同步的中文语音 —— 用项目自己的 Whisper 转写核对过，逐字一致。
> 所以中文剧本不需要先翻成英文，**只有正文描写用英文**。

## 4.5 无对白镜 / 台词容量

### 无对白镜要把「不说话」写成可见状态

绑到本镜真正在场的人身上，它是表演描述：

```text
Everyone on screen keeps their lips closed and their mouths still for the full shot.
```

时间线闭合声明**只列本镜真有的声音层**：

```text
无对白：The audio timeline is complete after the listed ambience and effects.
有对白：The audio timeline is complete after the listed timed dialogue, ambience and effects.
```

无对白镜里枚举 `timed dialogue` 是一句和本镜事实不符的话。

### 台词容量 —— **超了会静默截断**

实测（`drama-skills`，同一镜 20 次生成，7~108 字 × 5/10/13 秒）：

| 送入可发声字数 ÷ 镜长 | 表现 |
|---|---|
| ≤ 约 **4.1 字/秒** | 自然语速，完整 |
| 约 4.1 ~ 9 | 完整但**明显赶**（最快到自然语速的 2.5 倍） |
| > 约 **9 字/秒** | **静默截断**：超出的部分根本不发声 |

> **接口不会报错。** 截断时任务照样返回成功、画面正常、时长也对，只是台词
> 后半段没有了 —— 实测**送 78 字只说了 46 字，送 108 字同样只说了 46 字**。
> 文字审查看不出来，秒数加总也仍然「正确」。
> **算术通过从来不是台词说得完的证据。**

**延长镜头确实管用**（也是实测）：37 字从 5 秒改到 10 秒，语速 7.0 → 4.3；
50 字从 5 秒改到 13 秒，9.6 → 4.2，都恢复自然且完整。

容量 ≈ `4.1 × 镜长`：5 秒镜约 20 字，10 秒镜约 40 字。
**「字」是可发声字，不含标点** —— 把标点算进去会高估约两成。

本项目在 `h3_prompt::check_dialogue_fit` 里做了这道核算，超容量时写进镜头状态。

## 5. 画面里的字

任何真的出现在画面上的招牌、字幕、标签、霓虹，放进**英文双引号**里，原文原标点：

```text
A red neon sign reading "营业中" glows above the doorway.
```

## 6. 四种任务的开头一行

正文之前的第一行是**对齐指令**，然后空一行再接三个字段。

| 任务 | 有什么图 | 第一行 |
|---|---|---|
| T2VA | 无 | **没有这一行**，直接从三个字段开始 |
| I2VA | 首帧 | `For the target video, at 0.00 seconds into the target video, <Picture 1> (from [Shot 1]) is fully referenced.` |
| FL2VA | 首+尾 | `How the reference pictures align with the target video — Picture 1 (from Shot 1) aligns with the 0.00-second mark of the target video; Picture 2 (from Shot N) aligns with the S.SS-second mark of the target video.` |
| L2VA | 尾帧 | `How the reference pictures align with the target video — <Picture 1> (from [Shot N]) aligns with the S.SS-second mark of the target video.` |

`S.SS` 是有效时长，**必须两位小数**。

正文怎么接：

- **I2VA**：首帧锚点 → 动作起势 → 连续发展 → 结果/反应。
  身份、衣着、颜色、关键物件、空间关系保持一致。
- **FL2VA**：**不要把两张图各描写一遍** —— 要写的是连接它们的那条**运动路径**：
  主体怎么移动、姿态怎么变、物件怎么被操作、构图怎么演化、光怎么转。
- **L2VA**：先立一个说得通的前一状态，再让动作和构图**逐渐落到**那张图上。

## 7. 接到本项目的链路上

| 任务 | Anima 里怎么调 | 状态 |
|---|---|---|
| T2VA | `generate_video`，`engine: "h3"` | ✅ 可用 |
| I2VA / FL2VA / L2VA | `animate_image`（首帧 / 末帧 / 首尾帧都支持） | ✅ 可用 |
| 全参考 | `animate_image` + `mode: "reference"` | ✅ 可用，另见 `h3-reference-mode.md` |

LTX 2.5 那条路**不吃这套格式** —— 它没有说话人 ID、没有 `<d>`、
也没有负向提示词入口，写自然语言描述即可。首帧走 `animate_image`，
首尾帧同理（两头都走关键帧，工具已经处理）。

> **未验证**：上面那一行「对齐指令」是官方 API 的写法。走 WeeTodd 时首帧是
> 由 `KeyframeEncode` 节点接进去的，这一行**还需不需要写进提示词，本机没验过**
> —— 等视觉编码器到位后再测。三个字段和台词格式这部分是模型侧的，不受影响。
