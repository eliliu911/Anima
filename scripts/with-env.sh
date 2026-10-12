#!/bin/sh
# 加载 .env.local 后再执行后面的命令。
#
# 用途：把 API Key 以环境变量形式喂给 tauri dev / cargo test，避免开发期反复弹
# Keychain 授权框（原因见 src-tauri/src/config/secrets.rs 的模块注释）。
#
# 用法：scripts/with-env.sh <命令> [参数…]
#   scripts/with-env.sh tauri dev
#   scripts/with-env.sh cargo test --test live -- --ignored --nocapture
set -e

# Rust 安装时用了 --no-modify-path，cargo 不在交互 shell 的 PATH 里。
# 这里补上，否则从终端直接 `npm run tauri:dev` 会报
# "failed to run 'cargo metadata' ... No such file or directory"。
# anima.sh 里也 export 了，但直接走 npm 脚本时不经过它。
case ":${PATH}:" in
  *":${HOME}/.cargo/bin:"*) ;;
  *) PATH="${HOME}/.cargo/bin:${PATH}"; export PATH ;;
esac

# 相对脚本自身定位项目根目录，这样从任何子目录调用都能工作
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
ENV_FILE="$ROOT/.env.local"

if [ -f "$ENV_FILE" ]; then
  # set -a 让后续所有赋值自动 export；用完立刻关掉，避免影响被调用的命令
  set -a
  # shellcheck disable=SC1090
  . "$ENV_FILE"
  set +a
else
  echo "提示: 未找到 ${ENV_FILE}，需要 API Key 的后端将不可用" >&2
fi

exec "$@"
