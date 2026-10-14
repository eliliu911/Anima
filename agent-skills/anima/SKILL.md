---
name: anima
description: 驱动 Anima Studio 做漫剧 —— 建项目、导剧本、抽实体建资产、定风格、生成分镜、合成提示词、出片、组装成集。也包括查服务状态和排查出图出片失败。当用户提到 Anima、漫剧、短剧、分镜、出片、资产、风格、或者要「做一集」时使用。
---

# 驱动 Anima Studio

Anima 是一台本机的漫剧生产线：**中文剧本 → 一集带配音带字幕的成片**，
全程离线跑在本机 Mac 上。
仓库：https://github.com/eliliu911/Anima

工具通过 `anima-mcp` 暴露（24 个，白名单见 `src-tauri/src/mcp_server/expose.rs`：
不暴露 shell / 文件读写 / 私人记忆库）。**没有这些工具时**，先照下面「安装」装上。

## 安装（给拿到这个链接的 Agent：照下面一步步做）

> English: this is an agent skill for **Anima**, a local (macOS / Apple Silicon) AI short-drama studio.
> Steps: clone the repo → build the `anima-mcp` stdio MCP server → register it with your agent (Claude Code / Codex / any MCP client)
> → install this file as a skill → verify with `studio_list_projects` and `comfyui_status`. The rest of this file explains how to drive Anima well.

### 0. 前提

- macOS、Apple Silicon（这是第一目标平台）
- Rust 工具链（`rustup`；装在 `~/.cargo/bin`，新 shell 里可能要先 `export PATH="$HOME/.cargo/bin:$PATH"`）、`brew install protobuf`
- 出图出片要本机 ComfyUI + 模型；只建项目、写剧本、排分镜不需要。缺什么用体检看：`./scripts/install-cli.sh` 装一次命令行，之后 `anima doctor`

### 1. 拿到代码

已经在 Anima 仓库里就跳过。否则：

```bash
git clone https://github.com/eliliu911/Anima.git && cd Anima
```

### 2. 编译 MCP 服务端

```bash
cd src-tauri && cargo build --release --bin anima-mcp && cd ..
echo "$PWD/src-tauri/target/release/anima-mcp"   # 记下这个绝对路径，下面叫 <ANIMA_MCP>
```

### 3. 接到 Agent 上（stdio，不带参数）

- **Claude Code**：`claude mcp add anima -- <ANIMA_MCP>`
- **Codex CLI**：`codex mcp add anima -- <ANIMA_MCP>`；老版本没有这个子命令就写进 `~/.codex/config.toml`：

  ```toml
  [mcp_servers.anima]
  command = "<ANIMA_MCP>"
  ```

- **其他 MCP 客户端**：stdio 传输，命令就是 `<ANIMA_MCP>`

### 4. 把这份文件装成技能

- **Claude Code**：放到 `~/.claude/skills/anima/SKILL.md`（全局），或者仓库里已经有 `.claude/skills/anima/` 就不用动
- **Codex**：支持技能的版本放 `~/.codex/skills/anima/SKILL.md`；不支持的话，把这份内容追加进项目的 `AGENTS.md`

### 5. 验证

重启客户端后调 `studio_list_projects`（能回列表 = MCP 通了）和 `comfyui_status`（看出图出片能不能跑）。
没有这两个工具 = 第 3 步没生效；`comfyui_status` 报连不上 = ComfyUI 没开（`anima start` 会一起拉起）。

### 要知道的

- `anima-mcp` **不需要桌面应用开着**，和桌面应用读同一个数据目录（`~/Library/Application Support/com.anima.desktop`）；两边同时跑没关系，任务登记用跨进程锁
- 云端模型的密钥在桌面应用的设置里配，`anima-mcp` 启动时自己加载
- **改了 Rust 代码要重新编译 `anima-mcp`** —— 客户端连着旧二进制时不会有新工具，也不报错

---

## 最重要的三条

**① 动手之前先 `studio_get_project`。**
它直接告诉你流程走到哪一步、下一步该做什么。不要凭对话记忆判断 ——
用户可能在界面上改过东西。

**② 耗时差两个数量级，贵的那几步要先说清楚再动手。**

| 操作 | 耗时 |
|---|---|
| 建项目 / 导剧本 / 列资产 | 秒级 |
| 抽实体、生成分镜、合成提示词 | 几十秒 ~ 几分钟 |
| 单张出图 | 默认 Qwen-Image 2.1 约 3.5 分钟（1024²）；Z-Image 45 秒 |
| 资产批量出图 | 每张一两分钟 × N |
| **出片（`studio_render_episode`）** | **每镜 15~40 分钟，整集几小时** |
| 合成一集（不超分 / 超分） | 几分钟 / 每镜一两分钟 |

发起出片或批量出图之前，**把镜数和预计总时长说出来**，然后再调。
用户明确说了「出片」「跑起来」「开始」就可以直接发起，不用再问一遍。

**③ 所有出图出片排在同一个 ComfyUI 队列里。**
不要同时发起多个批量任务 —— 2026-09-16 曾因 GPU 叠满导致整机重启。
一次只跑一个，用 `studio_job_status` 看完了没。

---

## 标准流程：六步

```
① 剧本 → ② 风格 → ③ 资产 → ④ 分镜 → ⑤ 出片 → ⑥ 组装
```

**顺序是硬的。** 跳步的后果不报错，只是结果废掉：
没有资产就出片 = 每一镜的角色长相都不一样。

### ① 剧本

```
studio_create_project(title, kind)     # kind: series(多集) / standalone(单集)
studio_import_script(project, text)    # 已有剧本要 replace=true
```

**剧本格式**（`studio_import_script` 会检查）：

```
【场景：咖啡馆 · 日 · 内】
林晚：这杯我请。
（旁白）她没有再回头。
```

- 每场开头一行 `【场景：地点 · 时间】`
- 台词一人一行 `角色名：台词`
- **有场标题时分场是按标题直接切的，不问模型** —— 所以标题写清楚能显著提高分镜质量

### ② 风格

```
studio_list_styles()                   # 看内置预设
studio_set_style(project, preset_id)   # 或 custom_prompt
```

风格定的是**整部剧**的视觉基调，之后每次生成都会带上。早定，别等到出图才定。

### ③ 资产

```
studio_extract_entities(project, episode)   # 读剧本认出角色/场景/道具，**不落盘**
  ↓ 把结果给用户看，点头之后
studio_create_assets(project, assets)       # 建进库
studio_generate_asset_images(project)       # 批量出图（每张一两分钟）
studio_list_assets(project)                 # 看还差几个没图
```

**`studio_extract_entities` 刻意不落盘** —— 它的结果要人看过再建。
不要抽完直接建。

建角色时：**脸、身材、年龄写 `identity`（人物本身），服装发型写 `description`**。
分开是因为换装时只改后者。

### ④ 分镜

```
studio_generate_shot_table(project, episode)    # 一场一次 LM 调用，几分钟
studio_get_project(project)                     # 看镜头表有没有问题
studio_synthesize_prompts(project, episode)     # 每镜合成提示词
```

**已有分镜时不带 `replace=true` 会被拒绝，这是故意的** ——
重新生成会连合成好的提示词一起冲掉。要覆盖得用户明确同意。

镜头表生成完**先让用户看一眼再合成提示词**。分镜错了，后面几小时全白跑。

### ⑤ 出片

```
studio_render_episode(project, episode)   # 不指定 shots = 整集
studio_job_status(project)                # 查进度
```

- 只出没出过的镜头；`shots: [3, 5]` 可以点名重出
- 某一镜失败会**接着出下一镜**，不会整批停
- 没合成提示词的镜头会被跳过并在返回里说明

### ⑥ 组装

这一步**界面上做更合适**（有预览、有镜头顺序调整、有封面生成）。
用 `studio_stitch_episode` 也可以，但拼完没法直接看。

---

## 出问题时先查服务

出图出片失败，八成是本机服务没起。**先查，不要猜**：

```bash
anima status     # 一眼看全：LM Studio / Ollama / ComfyUI / 数字人 / 四个语音引擎
anima doctor     # 体检：工具链、密钥、模型、音色库
anima logs -f    # 跟踪日志
```

也可以用 `comfyui_status` 工具看 ComfyUI 的模型清单和显存。

常见对应关系：

| 症状 | 多半是 |
|---|---|
| 模型列表拉不出来 / 对话失败 | LM Studio 没开（:1234） |
| 记忆召回不工作 | Ollama 没开（:11434） |
| 出图出片全部失败 | ComfyUI 没开（:8188） |
| **视频全黑但任务成功** | ComfyUI 没带 `--fp32-unet` |
| 语音每句都报错 | 选的那个 TTS 引擎没起 |

---

## 出图工具（不走 Studio 流程时用）

```
generate_image(prompt, ...)      # 默认 Qwen-Image 2.1（约 211 秒一张）；带 image_path 改图走 Z-Image
generate_video(prompt, ...)      # 文生视频
animate_image(image_path, ...)   # 图生视频
```

**三件容易写错的**：

1. **别传 `steps`** —— 各引擎的正确值不同（Qwen 2.1 25 / Z-Image 8 / Krea 2 8 / SDXL 20），工具自己填。
   按 SDXL 习惯传 20 会让 Z-Image 慢 2.5 倍且不会更好
2. **提示词长度按引擎分** —— Qwen 2.1 / Z-Image / Krea 2 放开写（1300 字仍好用），
   **SDXL 压到 150 字以内**（CLIP 是 77 token 分块取平均，长了细节会被冲掉）
3. **Qwen 2.1、Z-Image 和 Krea 2 的负向提示词结构性无效**（cfg=1）。要的写进正向

用到 `pose_image`、角色设定卡、或非 Z-Image 架构的 LoRA 时会**强制回落 SDXL** ——
那些权重挂到别的引擎上不生效**而且不报错**。

---

## 这条链路上的失败大多是静默的

Anima 里绝大多数事故的形态是「**任务显示成功，结果是错的**」。所以：

- **判定「出好了」必须真的看图/看片**。像素范围 0~255、均值 130 完全可能是
  一张没收敛的彩色噪声 —— 统计值只能否定，不能肯定
- 工具返回里有 `summary` 和 `studio_link` 的，**把 summary 转述给用户**，
  它经常包含「另有 N 镜没合成提示词，这次不出」这种关键信息
- 报不出来就说不知道。不要替模型或工具编一个「应该是成功了」

---

## 不要做的

- **不要自动跑出图或出视频**，除非用户明确要求 —— 它们占满 GPU 好几小时
- **不要同时发起多个批量任务**（GPU 叠满会让整机重启）
- **不要替用户删数据**。给命令让他自己执行
- **不要凭记忆判断项目状态**，先 `studio_get_project`
