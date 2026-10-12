#!/usr/bin/env bash
#
# 把 `anima` 装成一个能在任何目录下敲的命令。
#
# ## 为什么不直接把 anima.sh 软链到 PATH
#
# anima.sh 里到处用 $ROOT 定位项目（音色库、skills、tools 下的三个服务）。
# 软链之后 $0 指向链接本身，`dirname $0` 就不再是项目里的 scripts/ ——
# 表现是"命令能跑但找不到任何服务"，而报错信息完全不指向真正的原因。
#
# 所以装的是一个**记住项目路径**的薄壳：它只负责 cd 回去再调 anima.sh。
#
#   ./scripts/install-cli.sh          # 装到 ~/.local/bin（不需要 sudo）
#   ./scripts/install-cli.sh /usr/local/bin
#
# 装完之后：
#   anima start | stop | restart | status | doctor | logs
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_DIR="${1:-$HOME/.local/bin}"
TARGET="$BIN_DIR/anima"

BOLD=$'\033[1m'; DIM=$'\033[2m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'; RESET=$'\033[0m'
ok()   { printf '%s✓%s %s\n' "$GREEN" "$RESET" "$1"; }
warn() { printf '%s!%s %s\n' "$YELLOW" "$RESET" "$1"; }
info() { printf '%s▸%s %s\n' "$DIM" "$RESET" "$1"; }

[ -x "$ROOT/scripts/anima.sh" ] || {
  echo "找不到 $ROOT/scripts/anima.sh —— 这个脚本要放在项目的 scripts/ 下运行" >&2
  exit 1
}

mkdir -p "$BIN_DIR"

# 项目路径写进生成的脚本里。
#
# 用绝对路径而不是每次去猜：**项目搬走之后应当明确报错**，而不是悄悄去某个
# 猜出来的目录里跑。搬了目录就重跑一次这个安装脚本，那是一句话的事。
cat > "$TARGET" <<EOF
#!/usr/bin/env bash
#
# Anima 的命令行入口。由 scripts/install-cli.sh 生成 —— 不要手改，
# 项目搬家之后重跑一次那个脚本即可。
set -euo pipefail

ANIMA_ROOT="$ROOT"

if [ ! -x "\$ANIMA_ROOT/scripts/anima.sh" ]; then
  echo "Anima 项目不在 \$ANIMA_ROOT 了（可能是搬了目录）。" >&2
  echo "到新的项目目录里重跑一次：./scripts/install-cli.sh" >&2
  exit 1
fi

# **必须 cd 回项目**：anima.sh 靠相对位置定位音色库、skills 和 tools 下的
# 三个 python 服务。在别的目录里跑它会"能启动但找不到任何服务"
cd "\$ANIMA_ROOT"
exec ./scripts/anima.sh "\$@"
EOF
chmod +x "$TARGET"
ok "已安装：$TARGET"
info "指向项目：$ROOT"

# PATH 检查放在最后并且说清怎么办 —— 装完却敲不到命令是最常见的坑
case ":$PATH:" in
  *":$BIN_DIR:"*)
    ok "$BIN_DIR 已在 PATH 里，现在可以直接敲 ${BOLD}anima${RESET}"
    ;;
  *)
    warn "$BIN_DIR 不在 PATH 里。加上这一行再开一个新终端："
    # 按当前 shell 给出对应的配置文件，而不是笼统地说"改你的 rc 文件"
    case "$(basename "${SHELL:-}")" in
      zsh)  printf '\n    echo '"'"'export PATH="%s:$PATH"'"'"' >> ~/.zshrc\n\n' "$BIN_DIR" ;;
      bash) printf '\n    echo '"'"'export PATH="%s:$PATH"'"'"' >> ~/.bash_profile\n\n' "$BIN_DIR" ;;
      *)    printf '\n    export PATH="%s:$PATH"\n\n' "$BIN_DIR" ;;
    esac
    ;;
esac

printf '%s用法%s\n' "$BOLD" "$RESET"
printf '  %s\n' \
  "anima start      启动（会顺带拉起 LM Studio / Ollama / 当前选的 TTS 服务）" \
  "anima stop       只停应用；stop -a 连依赖服务一起停" \
  "anima restart    重启" \
  "anima status     看现在跑着什么" \
  "anima doctor     体检：工具链、模型、服务、音色库" \
  "anima logs -f    跟日志"
