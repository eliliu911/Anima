# MiniMax H3 全参考模式的输出格式

> **2026-10-06：以官方原文为准** —— `h3-official/`（HF `MiniMaxAI/MiniMax-H3@42ed227`，编译时嵌入 `src-tauri/src/h3_spec.rs`，
> 合成时按生产方式把对应章节原文喂给写手）。本文是早先的中文摘要，和原文冲突时听原文。
> 项目现在的写法：纯文生 = T2VA、首帧 = I2VA、首尾帧 = FL2VA（三字段，**不用 `<Subject N>`**）；参考图 / 接上一镜 / 长镜头 = Ref2VA 六段。
> `[Shot 1]` 不带时间码，镜内时刻写 `Around the N.N-second mark,`，`At MM:SS.mmm` 只标切镜；画面文字用英文双引号。

参考图/参考视频/参考音频一起进来时，H3 的稿子从三段变成**六段**，
而且多了一套**引用标签**。先读 `h3-prompt-format.md`——镜头、运镜、说话人、
`<d>` 那些规则这里全部沿用，本文只写不一样的地方。

来源同上：`ComfyUI-MiniMaxH3-Prompt-Writer/guides/VIDEO_PROMPT_WRITING_GUIDE_ref_en.md`（MIT）。

## 1. 六段，顺序固定

| 段 | 干什么 |
|---|---|
| `subject_definitions` | 定义被引用的内容，给它们标签 |
| `summary` | 一段话交代任务类型、目标视频、主要引用关系 |
| `retention_analysis` | 每个标签**在成片里被保留到什么程度** |
| `detailed_description` | 按播放顺序逐镜写画面/动作/声音/台词（正文） |
| `overall_soundscape` | 全片环境音与动作音 |
| `non_diegetic_music` | 只有观众听得见的配乐 |

注意正文字段**改名了**：T2VA 叫 `integrated_multimodal_description`，
这里叫 `detailed_description`。写错名字不报错，只是那一段不被当正文。

## 2. 四种标签

| 标签 | 指什么 |
|---|---|
| `<Subject N>` | 从素材里**抽出来的可复用内容**：人、动物、物件、场景、服装、道具、特效、风格、动作、表情、姿态 |
| `<Picture N>` | 一张**当作具体帧或构图锚点**用的参考图 |
| `<Video N>` | 一段提供剪辑源 / 续接起点 / 整片时间结构的参考视频 |
| `<Audio N>` | 被复制或被参考的音频 |

**标签一旦定了，六段里含义必须一致。**

三条最容易搞混的：

- **`<Subject N>` 指内容，不指文件。** 一个主体可以由多份素材共同定义，
  一份素材也可以提供多个主体：

  ```text
  <Subject 1> is the woman whose appearance comes from <Picture 1> and whose walking motion comes from <Video 1>.
  ```
- **图只是用来定义角色/场景/服装/风格时，不单开 `<Picture N>`** ——
  把出处写进对应的 `<Subject N>` 定义里就行。只有当这张图本身是某镜的
  首帧、关键帧、末帧或构图锚点，才给它独立编号。
- **从参考视频里拿人/物/场景/动作，那仍然是 `<Subject N>`。**
  `<Video N>` 只标"整片关系"（剪辑源、续接、节奏结构），不替代主体标签。
  参考视频里带声音**不会自动**产生一个 `<Audio N>`。

`<Video N>` 和 `<Audio N>` 各自独立编号 —— 同一个源文件完全可能是
`<Video 1>` 和 `<Audio 2>`，序号不同不代表不同源。

## 3. `summary` 的任务类型前缀

方括号开头，多种关系用 ` + ` 连接、不重复：

| 类型 | 什么时候 |
|---|---|
| `keyframe completion` | 图当首帧/关键帧/末帧等**具体帧锚点** |
| `reference generation` | 图/视频/音频只提供角色、场景、风格、动作、运镜、分镜等**生成指导** |
| `video editing` | **直接改**一段已有视频（改图、或在静止关键帧之间生成，都不算） |
| `video continuation` | 从已有视频**接着往下**生成 |
| `audio reuse` | 音频信号被整段或部分**复制** |
| `audio reference` | 只参考风格、音色、台词内容、音效质感、节拍、连续性 |

```text
[video continuation + keyframe completion] ...
[video editing + audio reuse] ...
```

> 参考视频只提供了运镜、切点或节奏，那是 `reference generation`，**不是**
> `video editing`。后两个只在那段视频真的被改或被接时才用。

`summary` 里只能用已经定义过的标签，**不许在这一段引入新标签**。
剪辑任务的 summary 在前缀之后固定接一句
`The target video is an edited version of <Video 1>.`

## 4. 正文的四点不同

| 维度 | T2VA | 全参考 |
|---|---|---|
| 正文字段 | `integrated_multimodal_description` | `detailed_description` |
| 风格开头 | 写在 `[Shot 1]` 之后 | **写在 `[Shot 1]` 之前**，一两句英文 |
| 引用信息 | 不用标签 | 标签**首次出现时**说清它的特征、位置、当前动作；之后直接沿用，不重新定义 |
| 音频关系 | 只写成片自己的声音 | 在对应镜头里引用 `<Audio N>`，并说明是**复制**还是**参考** |

具体帧锚点用自然说法：`the shot begins from <Picture 1>` /
`the shot's keyframe corresponds to <Picture 2>` / `the shot ends on <Picture 3>`。

生成类任务的 `detailed_description` 一般 **350–500 个英文词**。台词密集时
以放得下完整台词时间线为准，不要为了凑字数注水。**单镜头不构成写短的理由** ——
按信息量把细节分配到各镜头。

## 5. 参考主体说话时

`<Subject N>` 标的是**被引用的主体**，`(Sx)` 标的是**实际发声者**，两个都要留：

```text
<Subject 2> (S1) turns toward the woman and says, <d>[English] Last summer, I went to my grandfather's house.</d>
```

- 同一主体画外说话，写法不变，另注 `off-screen`。
- 发声者不对应任何已定义主体时，用一个稳定的嗓音描述加 `(Sx)`。
- **人声只是被整段复用的 BGM 里的一个点缀**（没有具体的人、角色或旁白在发声）时，
  音源写 `<Audio N>`，**不要再编一个 `(Sx)` 出来**。
- `(Sx)` 按成片里**实际发声事件的先后**分配一次，之后处处复用。
  `subject_definitions` 里绑定到说话人的 `<Audio N>` 沿用同一个 ID，不另起编号。
  **`retention_analysis` 里不写 `(Sx)`。**

直接复用参考音频里的台词/唱词时，**原词原语言照抄**，听不清的地方写 `[unclear]`，
不要猜也不要转述；标点规整成 `,` `.` `?` `!`，去掉波浪线、表情、重复符号。
只参考音色/节奏/情绪/表达方式时，**不要把原台词带进来**。

## 6. 两个声音字段里怎么提参考音频

按**听感层次**归位，别两边都写：环境和音效进 `overall_soundscape`，
只有观众听得见的配乐进 `non_diegetic_music`。同一份音频两样都提供时，各写各的：

```text
overall_soundscape: The copied ambience layer from <Audio 1> continues throughout the target video.
non_diegetic_music: <Audio 2> is directly reused as the complete audience-only score.
```

完整台词和唱词**只写在 `detailed_description` 的 `<d>` 里**，这两段不重复。

## 7. 接到本项目的链路上

`animate_image` + `mode: "reference"` 走的就是这条（H3 的 `ref2va` 分区）。
**已实跑通过**（2026-09-04）：640×384 / 5 秒 231s，峰值 53.80 GB，
参考图的人物身份守住，而且模型自己把中文字幕烧进了画面。

它用的是 `FL2VA/text_encoder/`（66.73 GB，带视觉权重）。常驻那份 q8-paged
是纯文本子集，只够跑 T2VA —— 代码按模式自动切，不用手动指定。

参考图的像素预算见 `character-consistency.md` —— 整排设定卡里一张脸只有
40~46 像素，那种图当参考图，提示词写得再准也救不回来。
