# Anima

跨平台 AI 语音数字人助手，主要运行于 macOS。集成多大模型（OpenAI / Claude / 本地 LM Studio、Ollama、MLX）、低延迟实时语音对话、性格养成与本地记忆系统、MCP 工具调用，以及双模数字人面容（电子云虚拟面部 + 3D 模拟真人形态）。

当前阶段：个人使用 / 原型验证。**阶段一（基础框架 + 多模型适配网关）已完成并可运行**，下一步是阶段二的语音与记忆系统。

## 快速开始

```bash
npm install
./scripts/anima.sh doctor    # 先体检：工具链、密钥、模型、服务
./scripts/anima.sh start     # 拉起依赖服务 + 应用
./scripts/anima.sh stop      # 关掉应用（依赖服务保持运行）
```

`anima.sh` 会自动按需拉起 Ollama 与 LM Studio、注入 `.env.local` 的密钥、
并把日志写到 `~/Library/Logs/Anima/anima.log`。完整用法 `./scripts/anima.sh -h`。
也可以照旧直接用 `npm run tauri:dev`。

需要 Node 与 Rust 工具链。Rust 装在 `~/.cargo/bin`，新 shell 里可能要先 `export PATH="$HOME/.cargo/bin:$PATH"`。

构建还需要 `protoc`（LanceDB 依赖）与两个本地模型：

```bash
brew install protobuf
ollama pull bge-m3          # 嵌入模型，记忆层用（必须多语言，见 DECISIONS）
# Whisper 语音识别模型放到 ~/Library/Application Support/com.anima.desktop/models/
```

已接入的后端：**LM Studio / Ollama / MLX server**（本地）+ **Azure OpenAI / OpenAI / Claude**（云端）。各后端的地址在设置面板里改，模型列表**从后端实时拉取**，不写死。

API Key 开发期放在项目根目录的 `.env.local`（已 gitignore），格式：

```
ANIMA_API_KEY_AZURE='...'
ANIMA_API_KEY_LMSTUDIO='...'
```

变量名规则是 `ANIMA_API_KEY_` + 后端 id 大写。为什么开发期不用 Keychain（会反复弹授权框）见 [`docs/DECISIONS.md`](./docs/DECISIONS.md)。

## 文档索引

- [`docs/ARCHITECTURE.md`](./docs/ARCHITECTURE.md) —— 完整架构设计方案（技术选型、系统架构图、各功能模块设计、性能与并发注意事项、开发路线图）
- [`docs/DECISIONS.md`](./docs/DECISIONS.md) —— 关键决策记录（项目命名、技术栈、模型路由策略等）
- [`CLAUDE.md`](./CLAUDE.md) —— 给 Claude Code 的项目上下文（开发时优先阅读）

## 技术栈速览

| 层 | 技术 |
| :--- | :--- |
| 客户端外壳 | Tauri (Rust) |
| 前端/3D | Vite + React + Three.js / React Three Fiber |
| 本地存储 | SQLite + LanceDB |
| 音频 | Rust cpal/rodio + Web Audio API |
| 模型接入 | OpenAI、Claude（云端）+ LM Studio / Ollama / MLX server（本地，可插拔） |

## 目标硬件

Mac Studio (M4 Max, 128GB 统一内存)，本机同时运行 LM Studio、Ollama、MLX server 三套本地推理服务。

## 开发路线图（概览）

1. ✅ 基础框架搭建：Tauri + Vite/React 工程配置、多模型适配网关
2. ✅ 语音与记忆系统：音频采集播放、VAD、STT/TTS、可打断语音管线、SQLite + LanceDB 记忆层
3. ⬜ 面容与动画开发：电子云粒子形态 → 3D Avatar + Viseme 嘴型同步
4. ⬜ 功能集成与优化：MCP 工具链、性格配置面板、性能优化与打包

详见 [`docs/ARCHITECTURE.md`](./docs/ARCHITECTURE.md) 第 7 节。
