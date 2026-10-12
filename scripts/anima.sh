#!/usr/bin/env bash
#
# Anima 系统启动器 —— 一条命令拉起/关掉整套东西。
#
#   ./scripts/anima.sh start      启动依赖服务 + 应用（开发模式）
#   ./scripts/anima.sh start -r   跑打包好的 .app（没有就先构建）
#   ./scripts/anima.sh stop       只关应用
#   ./scripts/anima.sh stop -a    连 LM Studio / Ollama 一起关
#   ./scripts/anima.sh restart
#   ./scripts/anima.sh status     看一眼各部件状态
#   ./scripts/anima.sh doctor     体检：依赖、模型、密钥齐不齐
#   ./scripts/anima.sh logs -f    跟踪日志
#
# 设计上的两个取舍：
#   1. `stop` **默认不动** LM Studio 和 Ollama —— 它们是常驻服务，你很可能同时
#      在别处用着，顺手关掉是很讨厌的副作用。要关得显式加 -a。
#   2. 关应用时按「项目路径」匹配进程，而不是按进程名。否则会误杀你其它项目的
#      vite / node。

set -euo pipefail

ROOT="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

APP_SUPPORT="$HOME/Library/Application Support/com.anima.desktop"
MODEL_DIR="$APP_SUPPORT/models"
LOG_DIR="$HOME/Library/Logs/Anima"
# 应用自己会写 anima.log（见 lib.rs 的 init_logging）。
# 启动器只收 npm/cargo 的构建输出，分开放避免两边交错。
LOG_FILE="$LOG_DIR/launcher.log"
APP_LOG="$LOG_DIR/anima.log"

# Rust 安装时用了 --no-modify-path，脚本里自己补上；lms 也不在默认 PATH
export PATH="$HOME/.cargo/bin:$HOME/.lmstudio/bin:$PATH"

OLLAMA_PORT=11434
# Kokoro-FastAPI（本地 TTS，OpenAI 兼容接口）
KOKORO_PORT=8880
KOKORO_DIR="$ROOT/tools/kokoro-native"
# F5-TTS：零样本音色克隆
F5_PORT=8881
F5_DIR="$ROOT/tools/f5-tts"
# GPT-SoVITS：官方 api_v2.py 的默认端口
SOVITS_PORT=9880
SOVITS_DIR="$ROOT/tools/gpt-sovits/upstream"
# CosyVoice2 / IndexTTS2 本地语音引擎端口
COSY_PORT=50000
INDEX_PORT=8000
# ComfyUI 工坊服务端口（生图/分镜/视频）
COMFY_PORT=8188
# 4D 神经数字人推理引擎 (Metal/MPS)
DIGITAL_HUMAN_PORT=8765
DIGITAL_HUMAN_DIR="$ROOT/tools/digital-human"

# 共享音色库。两个克隆型引擎（F5-TTS / GPT-SoVITS）用同一套参考音频。
VOICES_DIR="$ROOT/voice-library"
LMSTUDIO_PORT=1234
VITE_PORT=1420

# 列出音色库里的参考音频。
#
# 后缀必须跟 Rust 侧的 AUDIO_SUFFIXES 一致（src-tauri/src/tts/voices.rs）——
# 之前这里只数 *.wav，于是一个全是 mp3 的人物会被 doctor 报成 0 条参考、
# 启动时还会警告"音色库还是空的"，而它其实完全能用。数字和实际能力不一致，
# 比没有数字更糟。
voice_refs() {
  find "$VOICES_DIR" -mindepth 2 -maxdepth 2 \
    \( -iname '*.wav' -o -iname '*.mp3' -o -iname '*.m4a' -o -iname '*.flac' \) \
    2>/dev/null
}

# ---------------------------------------------------------------- 输出helpers

if [ -t 1 ]; then
  BOLD=$'\033[1m'; DIM=$'\033[2m'; RED=$'\033[31m'; GREEN=$'\033[32m'
  YELLOW=$'\033[33m'; BLUE=$'\033[34m'; RESET=$'\033[0m'
else
  BOLD=''; DIM=''; RED=''; GREEN=''; YELLOW=''; BLUE=''; RESET=''
fi

info()  { printf '%s\n' "${BLUE}▸${RESET} $*"; }
ok()    { printf '%s\n' "${GREEN}✓${RESET} $*"; }
warn()  { printf '%s\n' "${YELLOW}!${RESET} $*"; }
fail()  { printf '%s\n' "${RED}✗${RESET} $*"; }
die()   { fail "$*"; exit 1; }

# ---------------------------------------------------------------- 构建产物回收

# 回收 target/ 里已经没用的编译产物。
#
# ## 为什么需要
#
# **cargo 从不回收旧产物。** 每次编译产出一个新 hash 的文件，旧的原样留着 ——
# 一个月下来实测堆到 154GB，其中光 `anima` 这一个可执行文件就有 48 份、
# 每份 650MB（debug 符号 + LanceDB/DataFusion 那一大坨依赖）。
#
# ## 为什么默认不是 cargo clean
#
# `cargo clean` 会连当前构建一起删，下次启动就是**全量重编** ——
# 这个项目十几分钟起步。所以默认只删两类**确定没人用**的东西，
# 删完当前构建原样能跑、不触发任何重编：
#
#   1. 同名可执行产物的旧版本（每组只留最新的一份）
#   2. 孤儿目标文件 —— 它归属的 rlib/可执行文件已经不存在了
#
# 实测这两类占了 154GB 里的 59GB。真要腾干净用 `clean -a`。

prune_target() {
  local target="$ROOT/src-tauri/target"
  [ -d "$target" ] || return 0
  command -v python3 >/dev/null 2>&1 || return 0

  # **有编译在跑就不要动。** stop_app 在应用尚未起来时会直接返回，
  # 那种情况下 tauri dev 很可能正在编译 —— 这时候删文件会把构建搞崩，
  # 而报出来的错会是一堆莫名其妙的 "file not found"，很难联想到是清理干的
  if pgrep -x rustc >/dev/null 2>&1; then
    warn "有编译正在进行，跳过回收（这时候删文件会把构建搞崩）"
    return 0
  fi

  local before
  before=$(du -sh "$target" 2>/dev/null | cut -f1)
  info "构建产物 ${before:-?}，回收旧版本中…"
  python3 - "$target" "${before:-}" <<'PRUNE_PY'
import os, re, sys, collections

target = sys.argv[1]
before = sys.argv[2] if len(sys.argv) > 2 else ""
freed = 0
removed = 0
groups_seen = 0

for sub in ("debug", "release"):
    for kind in ("deps", "examples"):
        d = os.path.join(target, sub, kind)
        if not os.path.isdir(d):
            continue

        # ① 同名可执行产物只留最新的一份。
        #    它们是 bin/test 产物，从不作为编译输入，旧的删掉零代价
        groups = collections.defaultdict(list)
        for e in os.scandir(d):
            if not e.is_file(follow_symlinks=False) or "." in e.name:
                continue
            m = re.match(r"^(.*)-[0-9a-f]{8,20}$", e.name)
            if not m:
                continue
            st = e.stat()
            groups[m.group(1)].append((st.st_mtime, st.st_size, e.path))
        groups_seen += len(groups)
        for items in groups.values():
            items.sort(reverse=True)
            for _, size, path in items[1:]:
                try:
                    os.remove(path); freed += size; removed += 1
                except OSError:
                    pass

        # ② 孤儿目标文件：归属的 rlib / 可执行文件已经不在了。
        #    **`.d` 不算产物** —— 它只是依赖清单，产物删了它还留着，
        #    拿它判断存活会把所有孤儿都放过（踩过一次）
        names = {e.name for e in os.scandir(d) if e.is_file(follow_symlinks=False)}
        alive = {}
        def has_owner(prefix):
            got = alive.get(prefix)
            if got is None:
                got = alive[prefix] = any(
                    c in names for c in (
                        f"lib{prefix}.rlib", f"{prefix}.rlib",
                        f"lib{prefix}.rmeta", f"{prefix}.rmeta",
                        f"lib{prefix}.dylib", f"{prefix}.dylib",
                        f"lib{prefix}.a", prefix,
                    )
                )
            return got

        for name in names:
            if not name.endswith(".o"):
                continue
            if has_owner(name.split(".", 1)[0]):
                continue
            path = os.path.join(d, name)
            try:
                freed += os.path.getsize(path); os.remove(path); removed += 1
            except OSError:
                pass

if removed:
    print(f"  回收 {removed} 个文件，约 {freed / 1e9:.1f} GB")
else:
    # **说清楚为什么是 0**，否则看起来像工具没生效。
    # 这里回收的是「同一个东西的多份历史版本」——刚全量清理过、
    # 或者编译次数还不多时，每个产物都只有一份，本来就没得删
    print(f"  没有旧产物可回收：{groups_seen} 个产物都只有当前这一版")
    print("  （编译几十次之后才会攒出旧版本；现在这 %s 是当前构建在用的）" % (before or "空间"))
PRUNE_PY

  ok "现在 $(du -sh "$target" 2>/dev/null | cut -f1)"
}

# 全量清空。**下次启动是全量重编**，这个项目十几分钟起步
wipe_target() {
  command -v cargo >/dev/null 2>&1 || die "找不到 cargo"
  if pgrep -x rustc >/dev/null 2>&1; then
    die "有编译正在进行，先等它结束"
  fi
  warn "全量清空 —— 下次启动要重编整个项目（十几分钟）"
  (cd "$ROOT/src-tauri" && cargo clean)
  ok "已清空"
}

# ---------------------------------------------------------------- 基础探测

port_open() {
  # 用 lsof 而不是 curl：这里只想知道「有没有人在听」，
  # 不关心它能不能正确响应（LM Studio 开了 token 校验时会返回 401，但服务是活的）
  lsof -nP -iTCP:"$1" -sTCP:LISTEN >/dev/null 2>&1
}

wait_for_port() {
  local port=$1 name=$2 timeout=${3:-30} elapsed=0
  while ! port_open "$port"; do
    if [ "$elapsed" -ge "$timeout" ]; then
      return 1
    fi
    sleep 1
    elapsed=$((elapsed + 1))
  done
  return 0
}

# 找出属于本项目的进程。
#
# node 那两个（tauri / vite）能用完整项目路径匹配，安全。
# 但 cargo 启动 Rust 二进制时用的是**相对路径**（命令行就是 `target/debug/anima`），
# 所以只能按这个相对模式匹配 —— 加不加 $ROOT 前缀都匹配不到。
# 这一点是实测发现的：一开始用完整路径，status 明明有实例在跑却报「未运行」。
app_pids() {
  {
    pgrep -f 'target/(debug|release)/anima$' 2>/dev/null || true
    pgrep -f "$ROOT/node_modules/.bin/tauri" 2>/dev/null || true
    pgrep -f "$ROOT/node_modules/.bin/vite" 2>/dev/null || true
    # 打包版的 .app
    pgrep -f '/Anima.app/Contents/MacOS/Anima' 2>/dev/null || true
  } | sort -u
}

app_running() { [ -n "$(app_pids)" ]; }

# ---------------------------------------------------------------- 依赖服务

start_ollama() {
  if port_open "$OLLAMA_PORT"; then
    ok "Ollama 已在运行（:${OLLAMA_PORT}）"
    return 0
  fi
  info "启动 Ollama…"
  if [ -d /Applications/Ollama.app ]; then
    # 用 App 启动而不是直接 `ollama serve`：这台机器上服务本来就由 App 托管，
    # 另起一个会和它抢端口
    open -a Ollama
  elif command -v ollama >/dev/null 2>&1; then
    nohup ollama serve >>"$LOG_DIR/ollama.log" 2>&1 &
  else
    warn "找不到 Ollama，记忆层与后台模型将不可用"
    return 1
  fi

  if wait_for_port "$OLLAMA_PORT" Ollama 30; then
    ok "Ollama 就绪（:${OLLAMA_PORT}）"
  else
    warn "Ollama 30 秒内没起来，继续启动其余部分"
    return 1
  fi
}

# 语音合成引擎起不来是**用户一定会撞上**的失败（每句话都报错），
# 所以要单独喊出来，而不是混在一行 warn 里。
# 当前选中的语音合成引擎到底能不能用。
# 云端引擎（Edge）没有本地端口可查，直接算通过。
tts_health() {
  local config_json="${APP_SUPPORT}/config.json"
  [ -f "${config_json}" ] || { warn "还没有 config.json，语音合成引擎未知"; return 0; }

  local engine
  engine="$(sed -n 's/.*"tts_engine"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
    "${config_json}" | head -1)"

  case "${engine}" in
    edge|"")   ok "语音合成：Edge（云端，无本地依赖）"; return 0 ;;
    openai_compatible) local port=$KOKORO_PORT label="Kokoro" ;;
    f5_tts)            local port=$F5_PORT     label="F5-TTS" ;;
    gpt_sovits)        local port=$SOVITS_PORT label="GPT-SoVITS" ;;
    cosy_voice2)       local port=$COSY_PORT   label="CosyVoice2" ;;
    index_tts2)        local port=$INDEX_PORT  label="IndexTTS2" ;;
    *) warn "语音合成引擎 ${engine} 未知，跳过检查"; return 0 ;;
  esac

  if port_open "$port"; then
    ok "当前语音合成引擎：${label}（:${port} 在线）"
    return 0
  fi
  fail "当前语音合成引擎：${label}（:${port} 不可用 —— 语音对话将报「TTS 请求失败」）"
  return 1
}

# 启动前/调试时的全量服务多路探测
check_all_services() {
  printf '\n%s\n' "${BOLD}启动前服务与生态探测${RESET}"

  # 1. LLM 核心后端
  port_open "$LMSTUDIO_PORT" && ok "LLM 核心：LM Studio（:${LMSTUDIO_PORT} 在线）" || warn "LLM 核心：LM Studio（:${LMSTUDIO_PORT} 未运行，若使用本地模型请启动）"
  port_open "$OLLAMA_PORT"   && ok "LLM 记忆/后台：Ollama（:${OLLAMA_PORT} 在线）" || warn "LLM 记忆/后台：Ollama（:${OLLAMA_PORT} 未运行，后台任务/向量记忆将降级）"

  # 2. ComfyUI 视觉/生图工坊
  if port_open "$COMFY_PORT"; then
    ok "视觉工坊：ComfyUI（:${COMFY_PORT} 在线，分镜/生图/视频就绪）"
  else
    warn "视觉工坊：ComfyUI（:${COMFY_PORT} 未运行，ComfyUI Studio 面板与生图工具暂不可用）"
  fi

  # 3. 4D 神经数字人 Sidecar
  if port_open "$DIGITAL_HUMAN_PORT"; then
    ok "4D 神经数字人：Sidecar（:${DIGITAL_HUMAN_PORT} 在线，Metal/MPS 实时驱动就绪）"
  else
    warn "4D 神经数字人：Sidecar（:${DIGITAL_HUMAN_PORT} 未运行，启动后由 Rust 自动拉起或回退到 3D/Live2D）"
  fi

  # 4. 全量本地语音引擎状态概览
  printf '%s\n' "${DIM}本地语音合成引擎概览：${RESET}"
  port_open "$KOKORO_PORT" && ok "  · Kokoro 本地 TTS（:${KOKORO_PORT} 在线）" || printf '  %s\n' "${DIM}· Kokoro 本地 TTS（:${KOKORO_PORT} 离线）${RESET}"
  port_open "$F5_PORT"     && ok "  · F5-TTS 零样本音色克隆（:${F5_PORT} 在线）" || printf '  %s\n' "${DIM}· F5-TTS 零样本音色克隆（:${F5_PORT} 离线）${RESET}"
  port_open "$SOVITS_PORT" && ok "  · GPT-SoVITS 语音克隆（:${SOVITS_PORT} 在线）" || printf '  %s\n' "${DIM}· GPT-SoVITS 语音克隆（:${SOVITS_PORT} 离线）${RESET}"
  port_open "$COSY_PORT"   && ok "  · CosyVoice2（:${COSY_PORT} 在线）" || printf '  %s\n' "${DIM}· CosyVoice2（:${COSY_PORT} 离线）${RESET}"
  port_open "$INDEX_PORT"  && ok "  · IndexTTS2（:${INDEX_PORT} 在线）" || printf '  %s\n' "${DIM}· IndexTTS2（:${INDEX_PORT} 离线）${RESET}"

  # 5. 当前选定引擎的健康判定
  tts_health || true
  printf '\n'
}

fail_tts() {
  echo ""
  warn "语音合成引擎不可用：$1"
  warn "  配置里选的是 Kokoro（tts_engine=openai_compatible），起不来的话每句话都会报"
  warn "  手工起：cd tools/kokoro-native && uv run uvicorn server:app --port 8880"
  warn "  想先用别的：设置 → 音频 → 换成 Edge（云端，不需要本地服务）"
  echo ""
}

# F5-TTS 零样本音色克隆引擎
start_f5() {
  if port_open "$F5_PORT"; then
    ok "F5-TTS 已在运行（:${F5_PORT}）"
    return 0
  fi
  if [ ! -f "$F5_DIR/server.py" ]; then
    return 0
  fi
  if ! command -v uv >/dev/null 2>&1; then
    return 0
  fi

  info "启动 F5-TTS 零样本音色克隆（原生 + MPS）…"
  (
    cd "$F5_DIR" || exit 1
    ANIMA_VOICES="$VOICES_DIR" ANIMA_F5_NFE="${ANIMA_F5_NFE:-8}" \
      nohup uv run --quiet uvicorn server:app \
      --host 127.0.0.1 --port "$F5_PORT" --log-level info \
      >>"$LOG_DIR/f5tts.log" 2>&1 &
  )
  if wait_for_port "$F5_PORT" F5-TTS 30; then
    ok "F5-TTS 就绪（:${F5_PORT}）"
  else
    warn "F5-TTS 正在后台加载模型权重（:${F5_PORT}，日志: $LOG_DIR/f5tts.log）"
  fi
}

# GPT-SoVITS 语音克隆引擎
start_sovits() {
  if port_open "$SOVITS_PORT"; then
    ok "GPT-SoVITS 已在运行（:${SOVITS_PORT}）"
    return 0
  fi
  if [ ! -f "$SOVITS_DIR/api_v2.py" ]; then
    return 0
  fi
  if [ ! -x "$SOVITS_DIR/.venv/bin/python" ]; then
    return 0
  fi

  info "启动 GPT-SoVITS 语音克隆服务（MPS）…"
  (
    cd "$SOVITS_DIR" || exit 1
    PYTORCH_ENABLE_MPS_FALLBACK=1 nohup .venv/bin/python api_v2.py \
      -a 127.0.0.1 -p "$SOVITS_PORT" \
      >>"$LOG_DIR/gptsovits.log" 2>&1 &
  )
  if wait_for_port "$SOVITS_PORT" GPT-SoVITS 30; then
    ok "GPT-SoVITS 就绪（:${SOVITS_PORT}）"
  else
    warn "GPT-SoVITS 正在后台加载模型权重（:${SOVITS_PORT}，日志: $LOG_DIR/gptsovits.log）"
  fi
}

# Kokoro 本地轻量神经语音引擎
start_kokoro() {
  if port_open "$KOKORO_PORT"; then
    ok "Kokoro 已在运行（:${KOKORO_PORT}）"
    return 0
  fi
  if [ ! -f "$KOKORO_DIR/server.py" ]; then
    return 0
  fi
  if ! command -v uv >/dev/null 2>&1; then
    return 0
  fi

  info "启动 Kokoro 本地神经语音服务（原生 + MPS）…"
  (
    cd "$KOKORO_DIR" || exit 1
    nohup uv run --quiet uvicorn server:app \
      --host 127.0.0.1 --port "$KOKORO_PORT" --log-level info \
      >>"$LOG_DIR/kokoro.log" 2>&1 &
  )

  if wait_for_port "$KOKORO_PORT" Kokoro 20; then
    ok "Kokoro 就绪（:${KOKORO_PORT}）"
  else
    warn "Kokoro 正在后台预热 MPS（:${KOKORO_PORT}，日志: $LOG_DIR/kokoro.log）"
  fi
}

# ComfyUI 的启动参数。**必须和 src-tauri/src/comfyui.rs 的 COMFY_ARGS 保持一致** ——
# 两条启动路径给的参数不一样的话，从哪边起的决定了链路能不能用，而且不报错。
#
#   --fp32-unet / --fp32-vae      FLUX / Wan2.1 / LTX-2.5 在 MPS 上必须 fp32。
#                                 不带的话 ComfyUI 按内存自己挑 bf16，而 bf16 会让
#                                 latent 全变 NaN —— 出来是**纯黑视频，不报任何错**。
#   --use-pytorch-cross-attention MiniMax H3 的 SolAttn 加速节点要求它；
#                                 实测对 Wan/LTX/SDXL 的输出没有影响。
#
# 细节见 docs/MEDIA_PIPELINES.md
COMFY_PRECISION_ARGS=(--fp32-unet --fp32-vae --use-pytorch-cross-attention)

# 启动 ComfyUI 必须带的环境变量。**必须和 src-tauri/src/comfyui.rs 的 COMFY_ENV 一致**
# —— 有测试盯着这一行。
#
# 现在是空的。曾经有 ASFP8_INT8_EXT=1，2026-09-03 撤掉：那个 W8A8 Metal 内核
# 在 M4 Max 上构建得起来，但它自带的逐位自检**全错**（2048 个元素无一相同，
# 最大偏差 329125），每次都回落到 comfy 自己的 int8 通路。强制它没有任何好处，
# 只是让每个新进程白编译一次内核。详细过程见 comfyui.rs 的 COMFY_ENV 注释。
COMFY_ENV_VARS=()

# 已经跑着的那个实例带的参数对不对。
#
# 只看端口通不通是不够的：用户经常自己先 `python main.py --listen ...` 起好，
# 那时上面这些参数一个都没有。今天就是这么丢了大半天 —— LTX 每次出纯黑，
# 而 ComfyUI 一切正常、任务状态 success。
warn_if_comfy_args_wrong() {
  local argv
  argv="$(curl -s --max-time 3 "http://127.0.0.1:${COMFY_PORT}/system_stats" 2>/dev/null \
          | python3 -c "import sys,json;print(' '.join(json.load(sys.stdin)['system']['argv']))" 2>/dev/null)" || return 0
  [ -z "$argv" ] && return 0
  case "$argv" in
    *--fp32-unet*) ;;
    *)
      warn "正在跑的 ComfyUI 没带 --fp32-unet —— LTX-2.5 会生成纯黑视频（且不报错）"
      warn "  当前参数: $argv"
      warn "  要修：anima stop -a 之后重新 anima start，或手工加上 --fp32-unet --fp32-vae"
      ;;
  esac

  # 这里曾经有一条「没装上 int8 的 W8A8 加速层就告警」。2026-09-03 删掉：
  # 那个内核在 M4 Max 上装得上但算不对（逐位自检 2048 个元素全错），
  # 于是"装上了"反而是坏消息。判据本身失去意义，留着只会误导。
}

start_comfy() {
  if port_open "$COMFY_PORT"; then
    ok "ComfyUI 已在运行（:${COMFY_PORT}）"
    warn_if_comfy_args_wrong
    return 0
  fi
  local comfy_dir=""
  for candidate in "$HOME/AI/ComfyUI" "$HOME/ComfyUI" "$HOME/Documents/ComfyUI" "/Users/you/AI/ComfyUI"; do
    if [ -f "$candidate/main.py" ]; then
      comfy_dir="$candidate"
      break
    fi
  done

  if [ -z "$comfy_dir" ]; then
    warn "未找到 ComfyUI 根目录，跳过自动拉起"
    return 0
  fi

  local python_bin="python3"
  if [ -x "$comfy_dir/.venv/bin/python" ]; then
    python_bin="$comfy_dir/.venv/bin/python"
  fi

  info "启动 ComfyUI 视觉/生图工坊服务…"
  (
    cd "$comfy_dir" || exit 1
    nohup env "${COMFY_ENV_VARS[@]}" \
      "$python_bin" main.py --listen 127.0.0.1 --port "$COMFY_PORT" \
      "${COMFY_PRECISION_ARGS[@]}" \
      >>"$LOG_DIR/comfyui.log" 2>&1 &
  )

  if wait_for_port "$COMFY_PORT" "ComfyUI" 25; then
    ok "ComfyUI 就绪（:${COMFY_PORT}）"
  else
    warn "ComfyUI 25 秒内未完成监听，后台继续加载（查看日志: $LOG_DIR/comfyui.log）"
  fi
}

start_digital_human() {
  if port_open "$DIGITAL_HUMAN_PORT"; then
    ok "4D 神经数字人 Sidecar 已在运行（:${DIGITAL_HUMAN_PORT}）"
    return 0
  fi
  if [ ! -f "$DIGITAL_HUMAN_DIR/server.py" ]; then
    return 0
  fi
  if [ -x "$DIGITAL_HUMAN_DIR/.venv/bin/python" ]; then
    info "启动 4D 神经数字人 Sidecar 推理服务…"
    (
      cd "$DIGITAL_HUMAN_DIR" || exit 1
      nohup .venv/bin/python server.py >>"$LOG_DIR/digital_human.log" 2>&1 &
    )
    if wait_for_port "$DIGITAL_HUMAN_PORT" "DigitalHuman" 20; then
      ok "4D 神经数字人 Sidecar 就绪（:${DIGITAL_HUMAN_PORT}）"
    fi
  fi
}

# 依赖服务统一入口。
#
# 在 Anima 启动/重启前，主动尝试拉起所有本地可用生态服务：
# LLM (LM Studio / Ollama)、视觉工坊 (ComfyUI)、4D 神经数字人以及已配置的语音合成引擎。
start_deps() {
  start_ollama || true
  start_lmstudio || true
  start_comfy || true
  start_digital_human || true
  start_kokoro || true
  start_f5 || true
  start_sovits || true
}

start_lmstudio() {
  if port_open "$LMSTUDIO_PORT"; then
    ok "LM Studio 已在运行（:${LMSTUDIO_PORT}）"
    return 0
  fi
  if ! command -v lms >/dev/null 2>&1; then
    warn "找不到 lms CLI，跳过 LM Studio（实时链路会连不上）"
    return 1
  fi
  info "启动 LM Studio 服务…"
  lms server start >>"$LOG_DIR/lmstudio.log" 2>&1 || true

  if wait_for_port "$LMSTUDIO_PORT" "LM Studio" 30; then
    ok "LM Studio 就绪（:${LMSTUDIO_PORT}）"
  else
    warn "LM Studio 30 秒内没起来，继续启动其余部分"
    return 1
  fi
}

stop_deps() {
  if command -v lms >/dev/null 2>&1 && port_open "$LMSTUDIO_PORT"; then
    info "停止 LM Studio 服务…"
    lms server stop >/dev/null 2>&1 && ok "LM Studio 已停止" || warn "LM Studio 停止失败"
  fi

  if port_open "$OLLAMA_PORT"; then
    info "停止 Ollama…"
    osascript -e 'quit app "Ollama"' >/dev/null 2>&1 || true
    sleep 2
    if port_open "$OLLAMA_PORT"; then
      pkill -f 'ollama serve' >/dev/null 2>&1 || true
      sleep 1
    fi
    port_open "$OLLAMA_PORT" && warn "Ollama 仍在监听 :$OLLAMA_PORT" || ok "Ollama 已停止"
  fi

  if port_open "$SOVITS_PORT"; then
    info "停止 GPT-SoVITS…"
    pkill -f "api_v2.py.*--port ${SOVITS_PORT}" >/dev/null 2>&1 || true
    pkill -f "api_v2.py.*-p ${SOVITS_PORT}" >/dev/null 2>&1 || true
    sleep 1
    port_open "$SOVITS_PORT" && warn "GPT-SoVITS 仍在监听 :$SOVITS_PORT" || ok "GPT-SoVITS 已停止"
  fi

  if port_open "$F5_PORT"; then
    info "停止 F5-TTS…"
    pkill -f "uvicorn server:app.*--port ${F5_PORT}" >/dev/null 2>&1 || true
    sleep 1
    port_open "$F5_PORT" && warn "F5-TTS 仍在监听 :$F5_PORT" || ok "F5-TTS 已停止"
  fi

  if port_open "$KOKORO_PORT"; then
    info "停止 Kokoro…"
    # 按 uvicorn 的完整启动命令匹配，不要按端口杀 —— 8880 上万一是别人的服务，
    # 按端口杀会误伤
    pkill -f "uvicorn server:app.*--port ${KOKORO_PORT}" >/dev/null 2>&1 || true
    sleep 1
    port_open "$KOKORO_PORT" && warn "Kokoro 仍在监听 :$KOKORO_PORT" || ok "Kokoro 已停止"
  fi

  if port_open "$COMFY_PORT"; then
    info "停止 ComfyUI…"
    pkill -f "main.py.*--port ${COMFY_PORT}" >/dev/null 2>&1 || true
    sleep 1
    port_open "$COMFY_PORT" && warn "ComfyUI 仍在监听 :$COMFY_PORT" || ok "ComfyUI 已停止"
  fi

  if port_open "$DIGITAL_HUMAN_PORT"; then
    info "停止 4D 神经数字人 Sidecar…"
    pkill -f "tools/digital-human/server.py" >/dev/null 2>&1 || true
    sleep 1
    port_open "$DIGITAL_HUMAN_PORT" && warn "Sidecar 仍在监听 :$DIGITAL_HUMAN_PORT" || ok "Sidecar 已停止"
  fi
}

# ---------------------------------------------------------------- 应用

stop_app() {
  if ! app_running; then
    ok "应用未在运行"
    return 0
  fi

  info "停止 Anima…"
  # 先礼后兵：TERM 让 Tauri 有机会释放麦克风/音频设备，
  # 直接 KILL 有时会把音频设备留在占用状态
  local pids
  pids="$(app_pids)"
  # shellcheck disable=SC2086
  kill $pids 2>/dev/null || true

  local waited=0
  while app_running && [ "$waited" -lt 10 ]; do
    sleep 1
    waited=$((waited + 1))
  done

  if app_running; then
    warn "优雅退出超时，强制结束"
    # shellcheck disable=SC2086
    kill -9 $(app_pids) 2>/dev/null || true
    sleep 1
  fi

  app_running && fail "仍有残留进程：$(app_pids | tr '\n' ' ')" || ok "Anima 已停止"
}

start_app_dev() {
  # 启动前执行全量服务检测（LM Studio, Ollama, ComfyUI, 4D 数字人, 各路语音引擎）
  check_all_services

  info "以开发模式启动 Anima（首次编译较慢）…"
  mkdir -p "$LOG_DIR"
  # **记下起始偏移。** 日志是追加写的，扫全文会把历史上的旧错误也算进来 ——
  # 那样一旦失败过一次，之后每次启动都会被报成失败。只看本次新增的部分
  local log_offset=1
  [ -f "$LOG_FILE" ] && log_offset=$(( $(wc -c <"$LOG_FILE") + 1 ))
  # 通过 with-env.sh 注入 .env.local 里的 API Key
  nohup "$ROOT/scripts/with-env.sh" npm run tauri:dev >>"$LOG_FILE" 2>&1 &
  local launcher=$!
  info "日志：$LOG_FILE"

  # Vite 起来只说明前端就绪，真正要等的是 Rust 那个二进制
  info "等待应用窗口（编译中，可能要几十秒）…"
  local waited=0
  while [ "$waited" -lt 600 ]; do
    if pgrep -f 'target/debug/anima$' >/dev/null 2>&1; then
      ok "Anima 已启动"
      return 0
    fi

    # **编译失败时 tauri dev 不会退出** —— 它继续监听文件变化等着重编。
    # 所以光靠 `kill -0 $launcher` 判断"还活着"会一直为真，
    # 而二进制永远不出现，这个循环就空转十分钟、一个字都不打。
    # 实测撞到过：用户看着"等待应用窗口…"卡住，而日志里早就有 19 个编译错误。
    if tail -c "+$log_offset" "$LOG_FILE" 2>/dev/null \
      | tr '\r' '\n' \
      | grep -qaE 'error\[E[0-9]+\]|^error:|could not compile'; then
      fail "编译失败："
      # **先把回车换成换行。** cargo 的进度条是用 \r 原地刷新的，
      # 不拆开的话「Building [===]」会和错误挤在同一行，把错误淹掉。
      # 然后去掉 ANSI 转义，再挑出错误行
      tail -c "+$log_offset" "$LOG_FILE" \
        | tr '\r' '\n' \
        | sed 's/\x1b\[[0-9;]*[A-Za-z]//g' \
        | grep -aE '^error(\[E[0-9]+\])?:|^\s+--> ' \
        | tail -25 | sed 's/^/    /'
      echo
      info "dev server 仍在监听，改完会自动重编 —— 不想等就 $0 stop"
      return 1
    fi

    if ! kill -0 "$launcher" 2>/dev/null; then
      fail "启动进程已退出，最后 20 行日志："
      tail -20 "$LOG_FILE" | sed 's/^/    /'
      return 1
    fi

    # 每 30 秒吱一声。全程无输出的话，"在编译"和"卡死了"看起来一模一样
    if [ "$waited" -gt 0 ] && [ $((waited % 30)) -eq 0 ]; then
      info "还在编译…（已等 ${waited}s）"
    fi
    sleep 2
    waited=$((waited + 2))
  done
  warn "等待超时（600s）。看日志：$0 logs -f"
  return 1
}

start_app_release() {
  local bundle="$ROOT/src-tauri/target/release/bundle/macos/Anima.app"
  if [ ! -d "$bundle" ]; then
    warn "还没有打包版本，先构建（会花几分钟）…"
    build_app 2>&1 | tail -5
    [ -d "$bundle" ] || die "构建后仍找不到 $bundle"
  fi
  info "启动打包版 Anima…"
  open "$bundle"
  sleep 2
  ok "已启动（打包版的 key 读 ${SECRET_FILE}，不读 .env.local —— 缺了跑 $0 key import）"
}

# ---------------------------------------------------------------- 体检

doctor() {
  local problems=0

  printf '%s\n' "${BOLD}构建工具链${RESET}"
  for tool in node npm cargo rustc protoc; do
    if command -v "$tool" >/dev/null 2>&1; then
      ok "$tool  ${DIM}$(command -v "$tool")${RESET}"
    else
      case "$tool" in
        protoc) fail "$tool 缺失 —— LanceDB 构建需要它：brew install protobuf" ;;
        cargo|rustc) fail "$tool 缺失 —— 装 Rust：https://rustup.rs" ;;
        *) fail "$tool 缺失" ;;
      esac
      problems=$((problems + 1))
    fi
  done

  printf '\n%s\n' "${BOLD}API Key${RESET}"
  if [ -f "$ROOT/.env.local" ]; then
    local perms
    perms="$(stat -f '%Lp' "$ROOT/.env.local")"
    ok ".env.local 存在（权限 ${perms}）"
    [ "$perms" = "600" ] || warn "建议 chmod 600 .env.local"
    grep -qE '^ANIMA_API_KEY_[A-Z_]+=.+' "$ROOT/.env.local" \
      && ok "至少配了一个 ANIMA_API_KEY_*" \
      || warn ".env.local 里没找到有效的 ANIMA_API_KEY_*"
  else
    warn ".env.local 不存在 —— 开发模式下需要 Key 的后端（Azure/OpenAI/Claude）会不可用"
  fi

  # **打包版读的是这个文件，不是 .env.local。** 双击启动没有任何环境变量，
  # 所以这条缺了的话「开发模式一切正常、打包版模型列表空白」
  if [ -f "$SECRET_FILE" ]; then
    local sp; sp="$(stat -f '%Lp' "$SECRET_FILE")"
    if [ "$sp" = "600" ]; then
      ok "secrets.env 存在（权限 600，打包版读这个）"
    else
      warn "secrets.env 权限是 $sp —— 里面是明文 key，应当是 600（应用启动时会自动收紧）"
    fi
    grep -qE '^ANIMA_API_KEY_[A-Z_]+=.+' "$SECRET_FILE" \
      && ok "secrets.env 里至少配了一个 key" \
      || warn "secrets.env 里没有有效的 ANIMA_API_KEY_*"
  else
    warn "secrets.env 不存在 —— **打包版会连不上任何云端后端**（跑 \`anima key import\` 从 .env.local 搬过去）"
  fi

  printf '\n%s\n' "${BOLD}模型${RESET}"
  # Whisper
  local whisper
  whisper="$(ls "$MODEL_DIR"/ggml-*.bin 2>/dev/null | head -1 || true)"
  if [ -n "$whisper" ]; then
    ok "语音识别：$(basename "$whisper") ${DIM}($(du -h "$whisper" | cut -f1))${RESET}"
  else
    fail "找不到 Whisper 模型。把 ggml-*.bin 放进：$MODEL_DIR"
    problems=$((problems + 1))
  fi
  # 嵌入模型。名字优先从 config.json 读，用户换了模型体检也跟着变
  local want_embed="bge-m3"
  local config_json="${APP_SUPPORT}/config.json"
  if [ -f "${config_json}" ]; then
    local configured
    configured="$(sed -n 's/.*"embedding_model"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
      "${config_json}" | head -1)"
    [ -n "${configured}" ] && want_embed="${configured}"
  fi

  # Silero VAD。没有它会退回能量 VAD —— 能跑，但"话没说完就被切断"会回来
  if [ -f "$MODEL_DIR/silero_vad.onnx" ]; then
    ok "Silero VAD 模型就位（$(du -h "$MODEL_DIR/silero_vad.onnx" | cut -f1)）"
  else
    warn "没有 Silero VAD 模型，会退回能量 VAD（话没说完容易被切断）。装它："
    printf '      curl -sL -o "%s/silero_vad.onnx" \\\n        https://github.com/snakers4/silero-vad/raw/master/src/silero_vad/data/silero_vad.onnx\n' "$MODEL_DIR"
  fi

  if port_open "$OLLAMA_PORT"; then
    # 先把输出收进变量再判断，**不要** `ollama list | grep -q`：
    # grep -q 命中即退出，ollama 随即收到 SIGPIPE（退出码 141），
    # 在 pipefail 下整条管道算失败 —— 结果模型明明装了却报「缺失」。
    # 这个假阴性实测踩过，排查起来很费劲。
    local installed
    installed="$(ollama list 2>/dev/null || true)"
    case "${installed}" in
      *"${want_embed}"*) ok "嵌入模型：${want_embed}" ;;
      *)
        fail "缺 ${want_embed}（记忆层召回会排错序）：ollama pull ${want_embed}"
        problems=$((problems + 1))
        ;;
    esac
  else
    warn "Ollama 未运行，无法检查嵌入模型（期望 ${want_embed}）"
  fi

  check_all_services
  # 共享音色库：两个克隆型引擎都靠它，空库等于它们都说不出话
  if [ -d "$VOICES_DIR" ]; then
    refs=$(voice_refs | wc -l | tr -d ' ')
    people=$(find "$VOICES_DIR" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | wc -l | tr -d ' ')
    if [ "$refs" -gt 0 ]; then
      ok "音色库：$people 个人物，$refs 条参考"
    else
      warn "音色库还是空的（${VOICES_DIR}）—— 克隆型引擎（F5-TTS / GPT-SoVITS）会说不出话"
    fi
  fi

  # 唱歌走 WORLD 声码器，跑在 Kokoro 那个 python 环境里
  if [ -d "$KOKORO_DIR/.venv" ]; then
    if "$KOKORO_DIR/.venv/bin/python" -c 'import pyworld' >/dev/null 2>&1; then
      ok "唱歌：WORLD 声码器就绪"
    else
      warn "缺 pyworld，唱歌会退回旧的切片管线（音质明显更差）：cd $KOKORO_DIR && uv sync"
    fi
  fi

  printf '\n'
  if [ "$problems" -eq 0 ]; then
    ok "${BOLD}没有阻塞性问题${RESET}"
  else
    fail "${BOLD}$problems 个问题需要处理${RESET}"
    return 1
  fi
}

status() {
  printf '%s\n' "${BOLD}Anima${RESET}"
  if app_running; then
    ok "运行中  ${DIM}PID: $(app_pids | tr '\n' ' ')${RESET}"
    port_open "$VITE_PORT" && printf '  %s\n' "${DIM}开发模式（Vite :${VITE_PORT}）${RESET}"
  else
    printf '  %s\n' "${DIM}未运行${RESET}"
  fi

  check_all_services

  printf '%s\n' "${BOLD}数据${RESET}"
  local db="$APP_SUPPORT/memory.db"
  if [ -f "$db" ]; then
    printf '  记忆库  %s\n' "${DIM}$(du -h "$db" | cut -f1)${RESET}"
  else
    printf '  记忆库  %s\n' "${DIM}尚未创建${RESET}"
  fi
  if [ -d "$APP_SUPPORT/vectors" ]; then
    printf '  向量库  %s\n' "${DIM}$(du -sh "$APP_SUPPORT/vectors" | cut -f1)${RESET}"
  else
    printf '  向量库  %s\n' "${DIM}尚未创建${RESET}"
  fi

  if [ -f "$LOG_FILE" ]; then
    printf '\n%s\n' "${DIM}日志：$LOG_FILE${RESET}"
  fi
  # status 是纯查询，永远成功返回 —— 否则「没有日志文件」这种
  # 完全正常的情况也会让调用方以为出错了
  return 0
}

# 用 heredoc 而不是从文件头注释里 sed 抽行：按行号抽取一改文件就错位，
# 实测就漏出过下面的源码。

# ---------------------------------------------------------------------------
# API Key：应用数据目录里的安全文件
#
# **为什么不是 Keychain。** 用户是免费 Apple 账号，拿不到 Developer ID 证书，
# 只能 ad-hoc 签名 —— 它的身份基于 cdhash，每次重编都变，于是 Keychain 每次
# 都当「另一个程序」，每个 provider 弹一次授权框。
#
# **为什么不是环境变量。** 打包版是双击启动的，不经过 shell，**没有任何环境变量**。
# 开发期靠 with-env.sh 注入的那条路在打包版里直接断掉，表现是「模型列表拉不出来」。
#
# 所以：<应用数据目录>/secrets.env，权限 600，格式和 .env.local 一样。
# Rust 侧的读取顺序是 环境变量 → 这个文件 → Keychain（见 config/secrets.rs）。
# ---------------------------------------------------------------------------
SECRET_FILE="$APP_SUPPORT/secrets.env"

# 建一个权限 600 的空文件。**先 chmod 再写** —— 反过来的话中间那一瞬是 644 的
secret_touch() {
  mkdir -p "$APP_SUPPORT"
  [ -f "$SECRET_FILE" ] || : > "$SECRET_FILE"
  chmod 600 "$SECRET_FILE"
}

# provider id → 变量名。和 Rust 的 env_var_name 同一套规则
secret_var() {
  printf 'ANIMA_API_KEY_%s' "$(printf '%s' "$1" | tr 'a-z.-' 'A-Z__')"
}

# **只显示有没有、从哪儿来，绝不打印值。**
secret_list() {
  printf '%s\n' "${BOLD}API Key${RESET}"
  if [ ! -f "$SECRET_FILE" ]; then
    warn "还没有 $SECRET_FILE"
  else
    local perms; perms="$(stat -f '%Lp' "$SECRET_FILE")"
    [ "$perms" = "600" ] && ok "${SECRET_FILE}（权限 600）" || { warn "$SECRET_FILE 权限是 ${perms}，正在收紧"; chmod 600 "$SECRET_FILE"; }
    local n=0
    while IFS= read -r line; do
      case "$line" in
        ANIMA_API_KEY_*=*)
          n=$((n+1))
          printf '  %s%s%s  %s已配置%s\n' "$BOLD" "${line%%=*}" "$RESET" "$GREEN" "$RESET"
          ;;
      esac
    done < "$SECRET_FILE"
    [ "$n" -gt 0 ] || warn "文件里一条都没有"
  fi
  # 环境变量会压过文件 —— 不说的话「改了文件却不生效」会查很久
  local envn=0
  while IFS= read -r v; do
    [ -n "$v" ] || continue
    envn=$((envn+1))
    printf '  %s%s%s  %s当前 shell 的环境变量，会压过文件%s\n' "$BOLD" "$v" "$RESET" "$YELLOW" "$RESET"
  done <<EOF
$(env | grep -oE '^ANIMA_API_KEY_[A-Z0-9_]+' || true)
EOF
  [ "$envn" -eq 0 ] || printf '%s\n' "${DIM}（应用是双击启动的话读不到这些环境变量，只有从终端起才有）${RESET}"
}

# 写一条。值从 stdin 读，**不回显、不进 shell 历史、不出现在 ps 的参数里**
secret_set() {
  local id="${1:-}"
  [ -n "$id" ] || die "用法：anima key set <provider-id>（比如 openai / azure / anthropic）"
  local var; var="$(secret_var "$id")"
  local val
  printf '粘贴 %s 的 key（不回显，回车结束）： ' "$id" >&2
  IFS= read -rs val || true
  printf '\n' >&2
  [ -n "$val" ] || die "什么都没输入，没改"
  secret_touch
  # 先去掉旧的同名行，再追加
  local tmp; tmp="$(mktemp)"; chmod 600 "$tmp"
  grep -v "^${var}=" "$SECRET_FILE" > "$tmp" 2>/dev/null || true
  printf '%s=%s\n' "$var" "$val" >> "$tmp"
  mv "$tmp" "$SECRET_FILE"
  chmod 600 "$SECRET_FILE"
  ok "已写入 $var → ${SECRET_FILE}（重启应用生效）"
}

secret_rm() {
  local id="${1:-}"
  [ -n "$id" ] || die "用法：anima key rm <provider-id>"
  [ -f "$SECRET_FILE" ] || die "还没有 $SECRET_FILE"
  local var; var="$(secret_var "$id")"
  local tmp; tmp="$(mktemp)"; chmod 600 "$tmp"
  grep -v "^${var}=" "$SECRET_FILE" > "$tmp" 2>/dev/null || true
  mv "$tmp" "$SECRET_FILE"
  chmod 600 "$SECRET_FILE"
  ok "已删掉 ${var}（重启应用生效）"
}

# 把 .env.local 里的 key 搬进安全文件。**打包自用前就是要做这一步。**
secret_import() {
  local src="${1:-$ROOT/.env.local}"
  [ -f "$src" ] || die "找不到 $src"
  secret_touch
  local tmp; tmp="$(mktemp)"; chmod 600 "$tmp"
  cp "$SECRET_FILE" "$tmp"
  local n=0
  while IFS= read -r line; do
    case "$line" in
      ANIMA_API_KEY_*=*)
        local var="${line%%=*}"
        # 值可能带引号，原样搬 —— Rust 侧会剥引号
        local t2; t2="$(mktemp)"; chmod 600 "$t2"
        grep -v "^${var}=" "$tmp" > "$t2" 2>/dev/null || true
        printf '%s\n' "$line" >> "$t2"
        mv "$t2" "$tmp"
        n=$((n+1))
        printf '  %s\n' "$var"
        ;;
    esac
  done < "$src"
  mv "$tmp" "$SECRET_FILE"
  chmod 600 "$SECRET_FILE"
  [ "$n" -gt 0 ] && ok "搬了 $n 条到 ${SECRET_FILE}（重启应用生效）" || warn "$src 里没有 ANIMA_API_KEY_*"
}


# ---------------------------------------------------------------------------
# MCP 服务端：让 Claude 直接调用 Anima
# ---------------------------------------------------------------------------
MCP_BIN_DEBUG="${ROOT}/src-tauri/target/debug/anima-mcp"
MCP_BIN_RELEASE="${ROOT}/src-tauri/target/release/anima-mcp"

mcp_bin() {
  if [ -x "${MCP_BIN_RELEASE}" ]; then printf '%s' "${MCP_BIN_RELEASE}"
  elif [ -x "${MCP_BIN_DEBUG}" ]; then printf '%s' "${MCP_BIN_DEBUG}"
  else printf ''
  fi
}

mcp_info() {
  local bin; bin="$(mcp_bin)"
  if [ -z "${bin}" ]; then
    warn "还没构建 anima-mcp"
    info "先构建：  cd ${ROOT}/src-tauri && cargo build --release --bin anima-mcp"
    return 1
  fi
  ok "anima-mcp：${bin}"
  # **只打印命令，不替用户改他的 Claude 配置。**
  printf '\n%s\n' "${BOLD}接进 Claude Code：${RESET}"
  printf '  claude mcp add anima -- %s\n' "${bin}"
  printf '\n%s\n' "${DIM}仓库里还有一份 skill（.claude/skills/anima/），告诉 Claude 六步的顺序、"
  printf '%s\n' "各步耗时、以及哪些操作不可逆 —— 只有工具没有它，它会自己摸索且大概率摸错。${RESET}"
  printf '\n%s\n' "${BOLD}自检（不需要 Claude，直接看它回什么）：${RESET}"
  printf '  %s mcp test\n' "$0"
  printf '\n%s\n' "${DIM}暴露出去的只有 Studio 和出图出片那一组，shell / 文件类一律不暴露。"
  printf '%s\n' "要界面才能跑的（出片、合成、分镜、合成提示词、资产批量出图）在这里调不了，"
  printf '%s\n' "因为它们要么需要界面派事件，要么会往共享的 jobs.json 里写。${RESET}"
}

# 自检：握一次手、列一次工具。**不调用任何会改东西的工具**
mcp_test() {
  local bin; bin="$(mcp_bin)"
  [ -n "${bin}" ] || { mcp_info; return 1; }
  printf '%s\n' "${BOLD}握手 + 列工具${RESET}"
  printf '%s\n%s\n' \
    '{"jsonrpc":"2.0","id":1,"method":"initialize"}' \
    '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
  | "${bin}" 2>/dev/null | python3 -c '
import sys, json
for line in sys.stdin:
    d = json.loads(line)
    r = d.get("result", {})
    if "serverInfo" in r:
        si = r["serverInfo"]
        print("  服务名 %s v%s，协议 %s" % (si["name"], si["version"], r["protocolVersion"]))
    elif "tools" in r:
        ts = r["tools"]
        print("  暴露 %d 个工具：" % len(ts))
        for t in ts:
            print("    " + t["name"])
'
}



# ---------------------------------------------------------------------------
# 安装 / 更新
#
# **走「重新构建 + 整包覆盖」，不是 Tauri 的增量 updater。** 理由：
#
# 1. 自用 + 开源路线下，装的人本来就是自己 build 的 —— 没有分发端点可谈
# 2. Tauri updater 要一对 minisign 密钥和一个放 latest.json 的地方，
#    两样都得用户先定；而覆盖更新现在就能用
# 3. 全量覆盖是原子的（先构建好、再一次换掉），delta 的失败模式更难收拾
#
# 以后真要对外分发，再加 updater 那条路，两者不冲突。
# ---------------------------------------------------------------------------
APP_BUNDLE="${ROOT}/src-tauri/target/release/bundle/macos/Anima.app"
APP_INSTALLED="/Applications/Anima.app"

app_version_of() {
  local plist="$1/Contents/Info.plist"
  [ -f "${plist}" ] || { printf '(没装)'; return; }
  /usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "${plist}" 2>/dev/null || printf '(读不出)'
}

# 产物是不是比源码旧。**这个检查是补出来的** ——
# 2026-09-18 真出过事：改完代码直接 `anima install`，它一声不吭地把上一版
# 装了进去，然后对着「修好的」应用发现问题照旧，白查了一轮。
#
# 装一个旧包和装不上不一样：装不上你立刻知道，装了旧包你以为装好了。
bundle_is_stale() {
  local bin="${APP_BUNDLE}/Contents/MacOS/anima"
  [ -f "${bin}" ] || return 0
  # 任何一个源文件比产物新就算旧。前端也算 —— 界面改动同样要重新打包
  local newer
  newer="$(find "${ROOT}/src-tauri/src" "${ROOT}/src" \
             "${ROOT}/src-tauri/Cargo.toml" "${ROOT}/src-tauri/tauri.conf.json" \
             "${ROOT}/package.json" \
             -newer "${bin}" 2>/dev/null | head -1)"
  [ -n "${newer}" ]
}

# 旧 .app 备份保留几个。
#
# **它们是可再生的** —— 随时能重新构建出来，唯一用途是「刚换的包坏了，
# 立刻退回上一个」。而每次 install 涨 102 MB，不清的话一天就上 G。
#
# 保留 3 个：够覆盖「今天换了两次都不对，退回昨天那个」。
APP_BACKUP_KEEP=3

# 只清 .app 备份。**不碰数据备份**（那是唯一的回头路，删了就没了）、
# 也不碰 target（删了下次要全量重编十几分钟，那是 anima clean 的事）。
prune_app_backups() {
  local dir="${HOME}/AnimaBackups/apps"
  [ -d "${dir}" ] || return 0
  # 按修改时间倒序，跳过最新的 N 个，其余删掉
  local old
  old="$(ls -dt "${dir}"/Anima-*.app 2>/dev/null | tail -n +$((APP_BACKUP_KEEP + 1)))"
  [ -n "${old}" ] || return 0
  local n=0 freed
  freed="$(du -sh "${dir}" 2>/dev/null | cut -f1)"
  while IFS= read -r a; do
    [ -n "${a}" ] || continue
    rm -rf "${a}"
    n=$((n + 1))
  done <<< "${old}"
  info "清掉 ${n} 个旧的 .app 备份（原 ${freed}，现 $(du -sh "${dir}" 2>/dev/null | cut -f1)，保留最近 ${APP_BACKUP_KEEP} 个）"
}

# 把构建产物装到 /Applications。**旧的先挪走，不是直接删** —— 留条回头路
install_app() {
  if [ ! -d "${APP_BUNDLE}" ]; then
    warn "还没有构建产物，先构建（十几分钟）…"
    build_app || die "构建失败"
  elif bundle_is_stale; then
    warn "产物比源码旧 —— 直接装会把上一版装进去，先重新构建"
    printf '%s\n' "${DIM}（比产物新的源文件：$(find "${ROOT}/src-tauri/src" "${ROOT}/src" -newer "${APP_BUNDLE}/Contents/MacOS/anima" 2>/dev/null | wc -l | tr -d " ") 个）${RESET}"
    build_app || die "构建失败"
  fi
  if app_running; then
    die "Anima 正在运行，先 $0 stop 再装（覆盖正在跑的 .app 会让它下次启动起不来）"
  fi
  local old_ver; old_ver="$(app_version_of "${APP_INSTALLED}")"
  local new_ver; new_ver="$(app_version_of "${APP_BUNDLE}")"
  if [ -d "${APP_INSTALLED}" ]; then
    local keep="${HOME}/AnimaBackups/apps"
    mkdir -p "${keep}"
    local stamp; stamp="$(date +%Y%m%d-%H%M%S)"
    rm -rf "${keep}/Anima-${old_ver}-${stamp}.app"
    mv "${APP_INSTALLED}" "${keep}/Anima-${old_ver}-${stamp}.app"
    info "旧版本挪到 ${keep}/Anima-${old_ver}-${stamp}.app"
  fi
  cp -R "${APP_BUNDLE}" "${APP_INSTALLED}"
  ok "已装到 ${APP_INSTALLED}（${old_ver} → ${new_ver}）"

  # **告诉打包版 tools/ 在哪。**
  #
  # `tools/` 是五个各自带 .venv 的 Python 服务，一共 12 GB，torch 那类轮子
  # 不可搬运，所以**不进包**。而打包版自己找不到它：双击启动没有
  # CARGO_MANIFEST_DIR、cwd 是 `/`、Resources 里没有 tools/。
  #
  # 找不到的后果是**静默的** —— 本地语音引擎一个都起不来，只在 debug 日志里
  # 留一行。所以装的时候留个指针文件，Rust 侧优先读它。
  mkdir -p "${APP_SUPPORT}"
  printf '%s\n' "${ROOT}" > "${APP_SUPPORT}/app_root"
  info "本机仓库路径写进 ${APP_SUPPORT}/app_root（打包版靠它找 tools/）"
  prune_app_backups
  # **打包版没有环境变量**，key 必须在安全文件里
  if [ ! -f "${SECRET_FILE}" ]; then
    warn "还没有 ${SECRET_FILE} —— 双击启动会连不上任何云端后端"
    info "跑一句：$0 key import"
  fi
}

# 只出 .app，不出 .dmg。
#
# dmg 那步要跑 AppleScript 摆 Finder 窗口，而那需要 macOS 的「自动化」权限 ——
# 第一次会弹框问，没人点就等到超时（`AppleEvent 已超时 -1712`），
# 然后**整个 tauri build 返回 1**。自用根本不需要 dmg，却会因为它装不上。
#
# 真要出 dmg：`npm run tauri:build`（配置里 targets 有 dmg），
# 第一次记得点掉那个「想要控制「访达」」的框。
build_app() {
  "$ROOT/scripts/with-env.sh" npm run tauri:build -- --bundles app
}

# 拉代码 + 重新构建 + 覆盖安装
update_app() {
  local dry="${1:-0}"
  cd "${ROOT}"

  # **共用工作区，动 git 之前先看干净不干净。** Antigravity 也在这个仓库里干活，
  # 拉代码撞上未提交的改动会很难收拾。
  #
  # **只看已跟踪的文件**（`-uno`）：工作区里长期躺着一堆未跟踪的设计稿，
  # 算上它们的话这条护栏永远是红的、等于没有。而 `git pull --ff-only`
  # 真要覆盖某个未跟踪文件时 git 自己会拒绝，那一层不用我们操心。
  if [ -n "$(git status --porcelain -uno)" ]; then
    warn "有已跟踪文件被改过，不动 git："
    git status --short -uno | head -20
    die "先提交或收好再更新"
  fi

  local branch; branch="$(git rev-parse --abbrev-ref HEAD)"
  [ "${branch}" = "main" ] || warn "当前在 ${branch} 分支，不是 main"

  info "看有没有新提交…"
  git fetch --quiet origin "${branch}" || die "拉不到远端"
  local behind; behind="$(git rev-list --count "HEAD..origin/${branch}")"
  if [ "${behind}" -eq 0 ]; then
    ok "已经是最新的（${branch} 和 origin 一致）"
    printf '%s\n' "${DIM}装着的版本：$(app_version_of "${APP_INSTALLED}")${RESET}"
    [ "${dry}" -eq 1 ] && return 0
    printf '%s\n' "${DIM}要强制重装当前代码：$0 install${RESET}"
    return 0
  fi

  printf '\n%s\n' "${BOLD}有 ${behind} 个新提交：${RESET}"
  git log --oneline "HEAD..origin/${branch}" | head -20
  if [ "${dry}" -eq 1 ]; then
    printf '\n%s\n' "${DIM}这是预演（-n），什么都没改。真更新去掉 -n${RESET}"
    return 0
  fi

  local lock_before; lock_before="$(shasum package-lock.json 2>/dev/null | cut -d' ' -f1)"
  # **--ff-only**：不产生合并提交。真分叉了就停下来让人看，别自动合
  git pull --ff-only origin "${branch}" || die "不是快进 —— 本地和远端分叉了，自己看一眼"
  local lock_after; lock_after="$(shasum package-lock.json 2>/dev/null | cut -d' ' -f1)"
  if [ "${lock_before}" != "${lock_after}" ]; then
    info "依赖锁文件变了，重装 node 依赖…"
    npm ci || die "npm ci 失败"
  fi

  info "构建（十几分钟，release 全量编译）…"
  build_app || die "构建失败 —— 代码已经拉下来了，修好之后跑 $0 install"
  install_app
}

usage() {
  cat <<'HELP'
Anima 系统启动器 —— 一条命令拉起/关掉整套东西。

  anima.sh start        启动依赖服务 + 应用（开发模式）
  anima.sh start -r     跑打包好的 .app（没有就先构建）
  anima.sh stop         只关应用
  anima.sh stop -a      连 LM Studio / Ollama 一起关
  anima.sh restart      重启应用（依赖服务保持不动）
  anima.sh restart -r   同上，但用打包版
  anima.sh clean        回收没用的编译产物（不影响当前构建，不触发重编）
  anima.sh clean -a     全量清空 target（下次启动要全量重编，十几分钟）
  anima.sh backup       备份用户数据（项目/记忆/资产/配置），默认存 ~/AnimaBackups
                        默认目录只保留最近 2 份；备到别处的不自动清
  anima.sh backup <目录> 备份到指定目录（比如外置盘）
  anima.sh backup -a    列出已有备份
  anima.sh key          看有哪些 key、存在哪儿（不显示值）
  anima.sh key set <id> 写一条 key 进本机安全目录（不回显、不进 shell 历史）
  anima.sh key rm <id>  删一条
  anima.sh key import   把 .env.local 里的 key 搬进本机安全目录
  anima.sh install      装进 /Applications（产物比源码旧会先自动重新构建）
  anima.sh update       拉代码 + 重新构建 + 覆盖安装
  anima.sh update -n    只看有没有新提交，什么都不改
  anima.sh mcp          让 Claude 直接调用 Anima：给出接入命令
  anima.sh mcp test     自检：握手 + 列出暴露了哪些工具
  anima.sh status       看一眼各部件状态
  anima.sh doctor       体检：工具链、密钥、模型、服务齐不齐
  anima.sh logs         看最近 100 行日志
  anima.sh logs -f      跟踪日志

两个刻意的行为：

  · stop 默认**不动** LM Studio 和 Ollama。它们是常驻服务，你很可能同时在别处
    用着，顺手关掉是很讨厌的副作用。要一起关就加 -a。

  · 关应用时按项目路径匹配进程，不按进程名，避免误杀其它项目的 vite / node。

  · backup 刻意**不备份 models/** —— 那里是 Whisper 和 Silero VAD，可重新下载的
    权重，不是用户数据。memory.db 走 SQLite 的 .backup，所以应用开着也能备。

· key 存在 ~/Library/Application Support/com.anima.desktop/secrets.env（权限 600），
  **不是 Keychain**：ad-hoc 签名的身份每次重编都变，Keychain 会每个后端弹一次框。
  也**不能只靠环境变量**：打包版是双击启动的，不经过 shell，一个环境变量都没有。

日志：~/Library/Logs/Anima/anima.log
模型：~/Library/Application Support/com.anima.desktop/models/
备份：~/AnimaBackups/
HELP
}


# ---------------------------------------------------------------- 备份

# 用户数据的备份目录。默认放 ~/AnimaBackups，可用第一个参数覆盖。
BACKUP_ROOT_DEFAULT="$HOME/AnimaBackups"

# 备份用户数据。
#
# **为什么不是 tar 整个目录完事：**
#
# 1. `memory.db` 是带 WAL 的 SQLite，而**应用通常正跑着**。直接拷文件会拿到
#    撕裂的状态（主库和 -wal 不在同一个时间点），恢复出来可能少最近的写入、
#    甚至打不开。所以走 SQLite 自己的 `.backup`，它对活库是安全的。
# 2. `models/` 里是 Whisper（465 MB）和 Silero VAD —— **可重新下载的权重**，
#    不是用户数据。进备份只会让每次备份多花几分钟、多占半个 G。
#    真丢了 `anima doctor` 会告诉你怎么装回来。
# 数据备份保留几份。
#
# **和 .app 备份不是一回事**：那些是可再生的（随时能重新构建），
# 这些不是 —— 删掉就永远没了。所以这里有两条边界：
#
# 1. **只清默认目录。** 备到外置盘（`anima backup /Volumes/xxx`）的一律不碰 ——
#    那多半是有意存档，替人删掉是很讨厌的副作用
# 2. **只在这次备份成功之后才清。** 失败时一个都不动，否则会出现
#    「新的没成、旧的被删了」这种最坏情况
#
# 2 份 = 能退回上一次和上上次。要更多就改这个数。
BACKUP_KEEP=2

# 只清 `anima-backup-*.tar.gz`。同目录下的 apps/ 和别的东西一律不碰。
prune_backups() {
  local root="$1"
  # 非默认目录不碰 —— 见上面第 1 条
  [ "${root}" = "${BACKUP_ROOT_DEFAULT}" ] || {
    printf '%s\n' "${DIM}备份在非默认目录，不自动回收${RESET}"
    return 0
  }
  local old
  old="$(ls -t "${root}"/anima-backup-*.tar.gz 2>/dev/null | tail -n +$((BACKUP_KEEP + 1)))"
  [ -n "${old}" ] || return 0
  local n=0
  while IFS= read -r f; do
    [ -n "${f}" ] || continue
    # 删之前把体积和名字打出来 —— 悄悄删用户数据，哪怕是他要求的，也不该没有痕迹
    printf '%s\n' "${DIM}  回收 $(du -h "${f}" | cut -f1)  $(basename "${f}")${RESET}"
    rm -f "${f}"
    n=$((n + 1))
  done <<< "${old}"
  info "回收了 ${n} 份旧备份（保留最近 ${BACKUP_KEEP} 份）"
}

backup_data() {
  local data_dir="$HOME/Library/Application Support/com.anima.desktop"
  [ -d "$data_dir" ] || die "找不到数据目录：$data_dir"

  local root="${1:-$BACKUP_ROOT_DEFAULT}"
  mkdir -p "$root" || die "建不了备份目录：$root"

  local stamp; stamp="$(date +%Y%m%d-%H%M%S)"
  local out="$root/anima-backup-$stamp.tar.gz"
  local stage; stage="$(mktemp -d)"
  trap 'rm -rf "$stage"' RETURN

  info "备份 Anima 用户数据 → $out"

  # ① SQLite 走 .backup，**活库也安全**
  local db
  for db in "$data_dir"/*.db; do
    [ -f "$db" ] || continue
    local name; name="$(basename "$db")"
    if sqlite3 "$db" ".backup '$stage/$name'" 2>/dev/null; then
      ok "  ${name}（SQLite 一致性快照）"
    else
      warn "  $name 的 .backup 失败，退回直接复制（可能不一致）"
      cp "$db" "$stage/$name" 2>/dev/null || true
    fi
  done

  # ② 其余目录/文件直接进包。**models/ 排除** —— 可重新下载
  local keep=(projects assets library studio vectors public runtime anima config.json)
  local item
  for item in "${keep[@]}"; do
    [ -e "$data_dir/$item" ] || continue
    cp -R "$data_dir/$item" "$stage/" 2>/dev/null && ok "  $item"
  done

  # ③ 记一份清单，恢复时知道这包是什么时候、从哪个版本出来的
  {
    printf 'Anima 数据备份\n'
    printf '时间: %s\n' "$(date '+%Y-%m-%d %H:%M:%S')"
    printf '来源: %s\n' "$data_dir"
    printf 'git:  %s\n' "$(git -C "$ROOT" rev-parse --short HEAD 2>/dev/null || echo 未知)"
    printf '\n已排除（可重新下载，不是用户数据）:\n  models/  —— Whisper + Silero VAD\n'
    printf '\n恢复方法:\n'
    printf '  1. 关掉 Anima（anima stop）\n'
    printf '  2. tar -xzf <这个包> -C <临时目录>\n'
    printf '  3. 把里面的内容覆盖回 %s\n' "$data_dir"
    printf '  4. 模型用 anima doctor 看缺什么、按提示装回来\n'
  } > "$stage/BACKUP_INFO.txt"

  tar -czf "$out" -C "$stage" . || die "打包失败"
  ok "备份完成：${out}（$(du -h "$out" | cut -f1)）"
  printf '%s\n' "${DIM}恢复方法写在包里的 BACKUP_INFO.txt${RESET}"
  # **排在这里，不排在前面** —— 新备份确实成了才清旧的
  prune_backups "${root}"
}

# 列出已有备份
backup_list() {
  local root="${1:-$BACKUP_ROOT_DEFAULT}"
  if [ ! -d "$root" ]; then
    warn "还没有备份（目录不存在：${root}）"
    return 0
  fi
  printf '\n%s\n' "${BOLD}已有备份（${root}）${RESET}"
  local found=0
  local f
  for f in "$root"/anima-backup-*.tar.gz; do
    [ -f "$f" ] || continue
    found=1
    printf '  %s  %s\n' "$(du -h "$f" | cut -f1)" "$(basename "$f")"
  done
  [ "$found" -eq 1 ] || warn "  还没有备份"
  if [ "$found" -eq 1 ]; then
    printf '%s\n' "${DIM}  合计 $(du -sh "$root" 2>/dev/null | cut -f1)，备份时自动保留最近 ${BACKUP_KEEP} 份${RESET}"
  fi
}

# ---------------------------------------------------------------- 入口

mkdir -p "$LOG_DIR"

CMD="${1:-status}"
shift || true

RELEASE=0
ALL=0
FOLLOW=0
DRY=0
# 位置参数。**只有 backup 和 key 收**（backup 收目标目录，key 收子命令 + provider id）。
# 其余命令仍然对未知参数报错 —— 那个守卫是有意的，别为了一个可选参数放宽全局。
TARGET=""
KEY_ARG=""
for arg in "$@"; do
  case "$arg" in
    -r|--release) RELEASE=1 ;;
    -a|--all)     ALL=1 ;;
    -f|--follow)  FOLLOW=1 ;;
    -n|--dry-run) DRY=1 ;;
    -h|--help)    usage; exit 0 ;;
    *)
      if [ "$CMD" = "backup" ] && [ -z "$TARGET" ]; then
        TARGET="$arg"
      elif [ "$CMD" = "key" ] && [ -z "$TARGET" ]; then
        TARGET="$arg"
      elif [ "$CMD" = "mcp" ] && [ -z "$TARGET" ]; then
        TARGET="$arg"
      elif [ "$CMD" = "key" ] && [ -z "$KEY_ARG" ]; then
        KEY_ARG="$arg"
      else
        die "未知参数：${arg}（用 -h 看用法）"
      fi
      ;;
  esac
done

case "$CMD" in
  start)
    if app_running; then
      warn "Anima 已在运行（PID $(app_pids | tr '\n' ' ')）"
      info "要重启用：$0 restart"
      exit 0
    fi
    start_deps
    if [ "$RELEASE" -eq 1 ]; then start_app_release; else start_app_dev; fi
    printf '\n'
    status
    ;;

  stop)
    stop_app
    if [ "$ALL" -eq 1 ]; then
      stop_deps
    else
      printf '%s\n' "${DIM}LM Studio / Ollama 保持运行（要一起关：$0 stop -a）${RESET}"
    fi
    ;;

  restart)
    stop_app
    sleep 1
    start_deps
    if [ "$RELEASE" -eq 1 ]; then start_app_release; else start_app_dev; fi
    ;;

  clean)
    if [ "$ALL" -eq 1 ]; then wipe_target; else prune_target; fi
    ;;

  backup)
    if [ "$ALL" -eq 1 ]; then backup_list "$TARGET"; else backup_data "$TARGET"; fi
    ;;

  key)
    case "${TARGET:-}" in
      ""|list)  secret_list ;;
      set)      secret_set "${KEY_ARG:-}" ;;
      rm)       secret_rm "${KEY_ARG:-}" ;;
      import)   secret_import "${KEY_ARG:-}" ;;
      *)        die "未知的 key 子命令：${TARGET}（list / set / rm / import）" ;;
    esac
    ;;

  mcp)
    case "${TARGET:-}" in
      "")    mcp_info ;;
      test)  mcp_test ;;
      *)     die "未知的 mcp 子命令：${TARGET}（留空 / test）" ;;
    esac
    ;;

  install) install_app ;;
  update)
    # -n 借用 FOLLOW 那个位置不合适，单独认一个 dry 标志
    if [ "${TARGET:-}" = "-n" ] || [ "${DRY:-0}" -eq 1 ]; then update_app 1; else update_app 0; fi
    ;;

  status)  status ;;
  doctor)  doctor ;;

  logs)
    [ -f "$LOG_FILE" ] || die "还没有日志：$LOG_FILE"
    if [ "$FOLLOW" -eq 1 ]; then tail -f "$LOG_FILE"; else tail -100 "$LOG_FILE"; fi
    ;;

  -h|--help|help) usage ;;
  *) die "未知命令：${CMD}（用 -h 看用法）" ;;
esac
