# Anima

**本地优先的 AI 创作工作室 + 语音数字人，跑在 Apple Silicon 的 Mac 上。**
从一份中文剧本出发，在本机完成「资产 → 分镜 → 带中文配音和口型的视频 → 剪辑成片」整条短剧生产线；
同时也是一个可以实时语音对话、能被打断、有记忆和面容的 AI 助手。

> A local-first AI studio and voice companion for Apple Silicon Macs. It turns a Chinese
> screenplay into a finished, voiced and lip-synced short drama — assets, storyboard,
> video and assembly — entirely on your own machine, and doubles as a real-time voice
> assistant with memory and an animated face. Built with Tauri (Rust) + React.

当前状态：**个人原型，持续开发中**（版本见 `package.json`）。不提供安装包，需要从源码构建。

---

## 能做什么

### 短剧工作台（Studio）

六步流水线，每一步都在界面上，也都能通过 MCP 交给 AI 代理来驱动：

1. **剧本**：导入或改写剧本，按场标题自动分场。
2. **风格**：整部剧统一画风（预设风格库 + 自定义）。
3. **资产**：从剧本里抽出角色 / 场景 / 道具，出设定图；角色可以有多套造型。
4. **分镜**：后台模型生成镜头表（景别、机位、运镜、站位、台词、时间段），逐镜检查时间线、口型、遮挡等问题。
5. **出片**：每镜合成 MiniMax H3 官方格式的提示词，按镜选择「首帧出片」或「参考出片」；
   台词直接由 H3 生成同步的中文语音和口型。超过 15 秒的镜头自动分段串成一条（窗口链）。
6. **组装**：按镜头顺序拼成一集，可导出到剪映 / Final Cut Pro 继续剪。

批量出片有队列（可拖动改顺序、插队），应用重启后会接着去 ComfyUI 收结果。

### 无限画布

节点式工作流（参考 LibTV 的交互）：出图、图生图、多视图设定卡、首帧 / 首尾帧 / 参考生视频、
动作迁移、超分补帧、全景 / 3DGS 导演世界……节点之间连线就是数据流。

- **超长视频**：H3 节点时长可拉到 75 秒，还是点「生成」，后台按提示词里的时间码分段（没写时间码就交给后台模型拆），一次串成一整条。
- **风格 / LoRA**：风格广场里挑 LoRA 或提示词风格，按引擎自动核对 LoRA 架构（读 safetensors 的张量键名）。
- 工作流可以存成模板、复用。

### 语音助手

- 多模型网关：OpenAI、Azure OpenAI、Claude（云端）+ LM Studio、Ollama、MLX server（本地），模型列表从后端实时拉取。
- 实时语音：麦克风 → VAD 断句 → Whisper（Metal）→ 大模型流式输出 → 逐句 TTS → 播放，**全程可被开口打断**。
- TTS 引擎可选：Edge TTS（免费、无需 key）以及若干本地引擎。
- 记忆：SQLite + LanceDB，对话落库、后台提取事实、语义召回（嵌入模型需多语言，默认 `bge-m3`）。
- 面容：电子云粒子形态 / 内置头 / 自己放进 `avatars/` 的 `.glb` / VRM；文字 → 拼音 → viseme 时间线驱动口型。
- MCP：既是 MCP 客户端（接本地 / 远程工具），也自带 `anima-mcp` 服务端，把 Studio 的能力暴露给 Claude Code 等代理。

---

## 环境要求

- **macOS + Apple Silicon**。开发机是 M4 Max / 128 GB 统一内存；视频生成很吃内存，小内存机器只能跑轻量部分。
- Node.js、Rust 工具链（`rustup`）、`brew install protobuf`（LanceDB 需要）。
- [Ollama](https://ollama.com)：`ollama pull bge-m3`（记忆层的嵌入模型）。
- 可选：[LM Studio](https://lmstudio.ai) 跑本地大模型。
- **出图 / 出片要 ComfyUI**，以及 WeeTodd 的 MLX 节点和对应的模型权重（MiniMax H3、LTX-2.5、Z-Image、Qwen-Image 等）。
  **本仓库不分发任何模型权重**，需要自己下载，各模型遵循各自的许可（有些禁止商用）。
  ComfyUI 在 Apple Silicon 上需要带 `--fp32-unet` 启动，否则视频会是纯黑（`scripts/anima.sh` 会处理）。

## 快速开始

```bash
npm install
cp .env.example .env.local && chmod 600 .env.local   # 只在要用云端大模型时填 key
./scripts/anima.sh doctor     # 体检：工具链、模型、服务、缺什么怎么装
./scripts/anima.sh start      # 拉起依赖服务 + 应用
```

也可以只起应用本体：

```bash
npm run tauri:dev
```

一个 key 都不填也能跑：本地 LM Studio / Ollama 不需要 key，语音默认走 Edge TTS。
**开发期不要把 key 存进 macOS Keychain**（每次编译都会反复弹授权框），原因见 `.env.example` 和 `src-tauri/src/config/secrets.rs`。

### 测试

```bash
npm test                                   # 前端（vitest）
cd src-tauri && cargo test --lib           # Rust
npm run build                              # 类型检查 + 前端构建
```

打真实服务的联调测试默认 `#[ignore]`，需要本机服务在跑。

---

## 目录

```
src/              React 前端（Studio、画布、设置、语音界面）
src-tauri/        Rust 后端（Tauri 命令、模型网关、语音管线、记忆、出图出片、任务队列、MCP）
  src/bin/        anima-mcp 等独立工具
skills/           给后台模型 / 代理用的技能（各自目录里有许可）
shared/           前后端共用的提示词模板和词表
comfyui-nodes/    给 ComfyUI 补的小节点
workflows/        ComfyUI 工作流示例
scripts/          启动器、体检、辅助脚本
docs/ARCHITECTURE.md  架构设计
```

## 技术栈

| 层 | 技术 |
| :--- | :--- |
| 客户端外壳 | Tauri 2 (Rust) |
| 前端 / 3D | Vite + React + TypeScript + Three.js / React Three Fiber，状态管理 Zustand |
| 本地存储 | SQLite + LanceDB |
| 音频 | cpal / rodio + Web Audio API；Whisper（whisper.cpp + Metal） |
| 出图出片 | ComfyUI + WeeTodd MLX 节点（MiniMax H3 / LTX-2.5 / Z-Image / Qwen-Image …） |
| 模型接入 | OpenAI、Azure OpenAI、Claude + LM Studio / Ollama / MLX server |

---

## 许可

**双许可**：

- **非商业使用**免费：[PolyForm Noncommercial License 1.0.0](./LICENSES/PolyForm-Noncommercial-1.0.0.txt)（个人、学习、研究、非营利机构）。
- **商业使用**需要另外取得授权：请在 [Issues](https://github.com/eliliu911/Anima/issues) 里联系。

例外：改写自 DramaClaw（ClaymoreLab）的文件仍按 [Elastic License 2.0](./LICENSES/ELv2.txt)，清单见 [`NOTICE`](./NOTICE)；
`skills/` 下的第三方技能按各自目录里的许可。完整说明见 [`LICENSE`](./LICENSE)。

Copyright (c) 2026 SONG LIU
