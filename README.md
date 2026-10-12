<p align="right"><b>English</b> | <a href="./README.zh-CN.md">简体中文</a></p>

# Anima

**A local-first AI short-drama studio and voice companion for Apple Silicon Macs.**

Starting from a Chinese screenplay, Anima runs the whole short-drama pipeline on your own machine —
script → assets → storyboard → video with synchronized Chinese speech and lip-sync → final cut.
It is also a real-time voice assistant that can be interrupted, remembers, has an animated face, and calls tools.

Status: **personal prototype, under active development** (version in `package.json`). No binary releases — build from source.

- [Features](#features)
  - [Short-drama studio](#1-short-drama-studio)
  - [Infinite canvas](#2-infinite-canvas)
  - [Voice assistant & digital human](#3-voice-assistant--digital-human)
  - [AI agent & tools](#4-ai-agent--tools)
  - [System & operations](#5-system--operations)
- [Tech stack](#tech-stack)
- [Requirements](#requirements)
- [Getting started](#getting-started)
- [Layout](#layout)
- [License](#license)

> The app UI, prompts and most code comments are in Chinese; the UI also ships an English locale.

---

## Features

### 1. Short-drama studio

Work is organized as **series → episodes → shots**, in a six-step pipeline: script → style → assets → storyboard → render → assemble.
Every step works in the UI and can also be driven by an AI agent over MCP.

**Script**
- Rich-text screenplay editor (TipTap) and a script library for source texts and setting documents.
- Adapt prose into a screenplay; generate an adaptation plan (which chapters become an episode and what each must pay off).
- Automatic scene split from scene headings; line classification (dialogue / action / on-screen text / heading).
- A screenwriting-craft library used on demand during adaptation.

**Style**
- One visual style for the whole series: built-in presets plus custom style text.
- Style plaza: the same shot rendered in several styles side by side.
- Style suggestions from reading the script.

**Assets**
- Extract characters / scenes / props from the script, review, then create; series-level asset pool.
- Slot-based character specs (face / body / outfit kept separate), **multiple looks** per character with per-shot overrides; merge a character into another as one of its looks.
- Sheet pipelines: turnaround, expressions, actions, multi-view character sheets, a seven-zone reference sheet; visual-DNA anchors; persona templates (no base image needed).
- Image QC after generation; a text "spatial contract" that pins scene geography.
- Auto-generated project and episode covers; a public asset library.

**Storyboard**
- A background model writes the shot table: size, angle, movement, purpose, blocking, lighting, dialogue (per-line emotion and voice), timed segments, cast.
- One-take mode: a scene as one shot with in-shot cuts on a timeline.
- Timeline detailing: every moment of a shot written from 0 s to the end.
- Timeline lint: warns before rendering about wording the video model would draw wrong (occlusion, actions hidden behind a character, lines too long for their window, speech inside a hand-off zone).
- A single shot-language vocabulary; programmatic ControlNet control images.
- **Director world**: place cameras, mannequins, poses and blocking inside a panorama / 3D Gaussian-splat set, carried across shots.

**Prompt synthesis & rendering**
- Two prompt routes: a Chinese fill-in template (route A) and the official MiniMax H3 six-section format (route B; the official spec is compiled in).
- Voice direction per line: emotion, timbre, stress and pauses — only where a line needs them.
- Per-shot render route chosen automatically or by hand: **first-frame** (FL2VA) or **reference** (Ref2VA); optional continuity from the previous shot.
- Engines: **MiniMax H3** (generates synchronized Chinese speech and lip-sync from the dialogue, including dialects) and **LTX-2.5** (about 10× faster).
- Long takes (a whole scene in one generation) and long shots (over 15 s, split automatically and chained through H3 windows).
- Quality tiers (LightX2V / Larry 4-step / 6-step / dense) × resolution (480P / 576P / 720P), with time estimates from measured runs.

**Jobs & queue**
- Batch rendering, batch first frames and asset image batches run as background jobs; the queue can be reordered by drag and drop, with hold slots; chained shots move as a group.
- **Resume after restart**: the in-flight shot is collected from ComfyUI and the rest of the batch is requeued.
- Progress follows ComfyUI's real sampling steps instead of fixed timeouts; the Mac is kept awake while rendering; local LLMs are unloaded to free memory before a render.

**Assembly & export**
- Episodes assembled in shot order; long takes occupy one slot.
- Per-character TTS dubbing (an alternative to H3's native voice) and burned-in subtitles.
- Export to **Final Cut Pro** (FCPXML 1.11) and **CapCut / JianYing** drafts.
- A token-usage ledger per backend and model.

### 2. Infinite canvas

A node-based workflow editor (interaction modeled on LibTV) where edges are data flow.

**Nodes**: prompt, image, video, audio / audio clip, character, LoRA, skill, shot / shot context,
depth motion capture, upscale, panorama, panorama viewer, director world, group.

**Capabilities**
- Text-to-image, image-to-image, multi-view sheets, pose control; first-frame / first-last-frame / reference video, reference audio.
- **Long video**: an H3 node can go up to 75 s and still just "Generate" — the backend splits by the timecodes in your prompt (or lets a background model split it if there are none) and chains it into one clip.
- Long-take window table: one node, one table of windows, one MP4.
- Motion transfer (depth / Canny control) and source-video remakes.
- Background removal, upscaling (RealESRGAN), frame interpolation (RIFE), video upscaling.
- 360° panorama to cube faces; single-image 3DGS set reconstruction (Apple SHARP).
- Style plaza for LoRAs or prompt styles; LoRA architecture checked per engine from safetensors tensor keys, not file names.
- Drag a connection to empty space for a "new node" menu; "+" handles on both sides.
- Save workflows as templates, linted for silently broken wiring before saving; export results to Downloads.
- Each node shows its own job's progress; jobs survive page reloads and app restarts.

### 3. Voice assistant & digital human

**Multi-model gateway**
- Five adapters: OpenAI, Azure OpenAI, Claude, OpenAI-compatible (LM Studio / MLX), Ollama native `/api/chat`.
- Model lists fetched live; real-time / background two-tier routing; automatic fallback for tool calls on backends that can't do them.
- Streams normalized into text / reasoning / tool-call events; adjustable thinking effort; every stream cancellable.

**Real-time voice**
- Mic capture (cpal) → Silero VAD → Whisper (whisper.cpp + Metal) → streaming LLM → sentence-by-sentence TTS → playback (rodio).
- **Barge-in**: speak at any time to interrupt (a cancellable state machine).
- Proactive speech, timers, multiple conversation modes.
- TTS engines: Edge TTS (free, no key), CosyVoice, IndexTTS, GPT-SoVITS, OpenAI-compatible servers (e.g. Kokoro, F5-TTS); emotion-matched reference voices and a voice library.

**Memory & personality**
- Dual-track memory: SQLite (sessions, metadata) + LanceDB (vectors) — conversations stored, facts extracted in the background, deduplicated, summarized, recalled into context.
- Local knowledge base and a task-scoped ledger.
- Personality growth: emotional axes (affinity / mood / energy) drive the system prompt and facial micro-expressions.
- Skills (role-play): one skill switches identity, method and available tools together.

**Digital human**
- Particle-cloud face (shaders), a built-in head, your own `.glb` / VRM models, Live2D.
- Text → pinyin → Oculus viseme timeline drives the mouth with any TTS engine; morph targets auto-detected across three naming schemes (Oculus / ARKit / five vowels).

### 4. AI agent & tools

- Agentic tool loop with tiered permissions, confirmation before execution, and Tauri capability allow-lists.
- **MCP client** for local / remote tool servers, with a permission guard, resources and prompts.
- **MCP server `anima-mcp`** exposing the studio (create projects, import scripts, create assets, shot tables, synthesis, rendering, stitching…) to agents such as Claude Code.
- Image / video tools: Z-Image, Qwen-Image, Krea 2, SDXL (LoRA / ControlNet), MiniMax H3, LTX-2.5 — through ComfyUI + WeeTodd MLX nodes.
- Research and web fetch, code outline, LSP, interactive terminal (PTY), git worktrees, sub-agents.
- Scheduled tasks; music (internet radio + controlling the local Music.app).

### 5. System & operations

- `anima` launcher: one command starts / stops Ollama, LM Studio, ComfyUI and TTS services with keys injected; `anima doctor` health check.
- Diagnostics that surface silent failures (e.g. ComfyUI started without `--fp32-unet`); a services page.
- Self-update (pull and rebuild locally) and build garbage collection.
- Multiple windows, single instance, Chinese / English UI (i18next), light / dark theme.
- Keys in `.env.local` during development, macOS Keychain in signed builds.

---

## Tech stack

| Layer | Technology |
| :--- | :--- |
| Shell | Tauri 2 (Rust), single-instance plugin |
| Frontend | React 19 · TypeScript · Vite · Tailwind CSS · Zustand · i18next |
| Canvas | React Flow (@xyflow/react) |
| 3D / face | Three.js / React Three Fiber · Gaussian Splats 3D · Pixi.js + Live2D |
| Editors | TipTap · react-markdown |
| Async & network | Tokio · reqwest (rustls) · tokio-tungstenite · SSE / NDJSON decoding |
| LLMs | OpenAI · Azure OpenAI · Anthropic Claude · LM Studio · Ollama · MLX server |
| Voice | cpal / rodio · Silero VAD (ONNX Runtime) · whisper-rs (whisper.cpp + Metal) · Edge TTS · CosyVoice · IndexTTS · GPT-SoVITS |
| Storage | SQLite (rusqlite, WAL) · LanceDB (Arrow) · macOS Keychain (keyring) |
| Image & video | ComfyUI · WeeTodd MLX nodes · MiniMax H3 · LTX-2.5 · Z-Image · Qwen-Image · Krea 2 · SDXL |
| Media | ffmpeg · RealESRGAN · RIFE · Apple SHARP (3DGS) |
| Protocols | MCP (rmcp) · LSP (lsp-types) · portable-pty |
| Tests | Vitest · cargo test (incl. protocol-level integration tests against mock HTTP servers) |

## Requirements

- **macOS on Apple Silicon.** Developed on an M4 Max with 128 GB unified memory; video generation is memory-hungry, so smaller machines can only run the lighter parts.
- Node.js, the Rust toolchain (`rustup`), `brew install protobuf` (needed by LanceDB), ffmpeg.
- [Ollama](https://ollama.com): `ollama pull bge-m3` (the memory embedding model — it must be multilingual).
- Optional: [LM Studio](https://lmstudio.ai) for local LLMs.
- **Image / video generation needs ComfyUI** with the WeeTodd MLX nodes and the model weights.
  **This repository ships no model weights** — download them yourself; each model follows its own license (some are noncommercial only).
  On Apple Silicon, ComfyUI must run with `--fp32-unet` or videos come out black (`scripts/anima.sh` handles it).

## Getting started

```bash
npm install
cp .env.example .env.local && chmod 600 .env.local   # only needed for cloud LLM keys
./scripts/anima.sh doctor     # health check: toolchain, models, services, what's missing
./scripts/anima.sh start      # start dependent services + the app
```

App only:

```bash
npm run tauri:dev
```

It runs with no keys at all: local LM Studio / Ollama need none, and voice defaults to Edge TTS.
**Don't store keys in the macOS Keychain during development** — every rebuild produces a new unsigned binary and the system keeps asking for permission (see `.env.example`).

**Tests**

```bash
npm test                                   # frontend (Vitest)
cd src-tauri && cargo test --lib           # Rust
npm run build                              # type check + frontend build
```

Live tests against real services are `#[ignore]` by default and need the services running locally.

## Layout

```
src/                  React frontend (studio, canvas, settings, voice UI)
src-tauri/            Rust backend
  src/llm/            multi-model gateway
  src/voice/ audio/ stt/ tts/   voice pipeline
  src/memory/         SQLite + LanceDB memory
  src/avatar/         lip-sync timeline
  src/agent_runtime/  agent runtime and tools (image, video, ComfyUI, MCP bridge…)
  src/mcp/ mcp_server/   MCP client / server
  src/bin/            anima-mcp and other standalone tools
skills/               skills for background models / agents (third-party skills carry their own licenses)
shared/               prompt templates and vocabularies shared by frontend and backend
comfyui-nodes/        small ComfyUI nodes
workflows/            example ComfyUI workflows
scripts/              launcher, health check, helper scripts
docs/ARCHITECTURE.md  architecture
```

## License

**Dual-licensed:**

- **Noncommercial use** is free under the [PolyForm Noncommercial License 1.0.0](./LICENSES/PolyForm-Noncommercial-1.0.0.txt) (personal use, study, research, noncommercial organizations).
- **Commercial use** requires a separate license — please ask via [Issues](https://github.com/eliliu911/Anima/issues).

Exceptions: files adapted from DramaClaw (ClaymoreLab) remain under the [Elastic License 2.0](./LICENSES/ELv2.txt), listed in [`NOTICE`](./NOTICE);
third-party skills under `skills/` keep the license in their own directory. Full terms in [`LICENSE`](./LICENSE).

Copyright (c) 2026 SONG LIU
