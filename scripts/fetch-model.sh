#!/usr/bin/env bash
# 带鉴权下载模型 —— token 从 .env.local 读，**绝不出现在命令行或日志里**。
#
# 用法:
#   scripts/fetch-model.sh hf  <repo> <path-in-repo> <目标目录>
#   scripts/fetch-model.sh civitai <modelVersionId?fileId=...> <目标文件名> <目标目录>
#
# 需要在 .env.local 里配（已 gitignore、权限 600）:
#   HF_TOKEN=hf_xxx           # https://huggingface.co/settings/tokens （read 权限即可）
#   CIVITAI_TOKEN=xxx         # https://civitai.com/user/account 最下面 API Keys
#
# **HuggingFace 的 gated 仓库还要先在网页上点同意**，光有 token 不够 —— 会一直 401。
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
[ -f "$ROOT/.env.local" ] && set -a && . "$ROOT/.env.local" && set +a

KIND="${1:?用法见脚本注释}"; shift

case "$KIND" in
  hf)
    REPO="${1:?repo}"; FILE="${2:?repo 内路径}"; DEST="${3:?目标目录}"
    [ -n "${HF_TOKEN:-}" ] || { echo "✗ .env.local 里没有 HF_TOKEN"; exit 1; }
    URL="https://huggingface.co/$REPO/resolve/main/$FILE"
    OUT="$DEST/$(basename "$FILE")"
    AUTH="Authorization: Bearer $HF_TOKEN"
    ;;
  civitai)
    ID="${1:?下载 id}"; NAME="${2:?文件名}"; DEST="${3:?目标目录}"
    [ -n "${CIVITAI_TOKEN:-}" ] || { echo "✗ .env.local 里没有 CIVITAI_TOKEN"; exit 1; }
    URL="https://civitai.com/api/download/models/$ID"
    OUT="$DEST/$NAME"
    AUTH="Authorization: Bearer $CIVITAI_TOKEN"
    ;;
  *) echo "✗ 第一个参数只能是 hf 或 civitai"; exit 1 ;;
esac

mkdir -p "$DEST"
# 先探一次：401/403 立刻停，别下半个文件占空间
CODE=$(curl -sL --max-time 30 -r 0-99 -H "$AUTH" "$URL" -o /dev/null -w '%{http_code}')
case "$CODE" in
  200|206) ;;
  401) echo "✗ 401 —— token 无效或过期"; exit 1 ;;
  403) echo "✗ 403 —— token 有效但没权限。HuggingFace 的 gated 仓库要**先在网页上点同意**"; exit 1 ;;
  *) echo "✗ HTTP $CODE"; exit 1 ;;
esac

echo "→ $(basename "$OUT")"
# `-C -` 断点续传：几十 GB 的下载被打断是常态（父命令超时、网络抖动），
# 不续传就要从头再来。已下的部分留在 .part 里。
# `-C -` 断点续传 + 低速自断。HuggingFace **按连接**限速：一条连接开久了
# 会从 47 MB/s 掉到 2 MB/s，而同时另开一条仍跑满。所以低于 3 MB/s 持续
# 60 秒就主动断开，让调用方重连 —— 干等只会一直被限着。
curl -L --max-time 7200 -C - --speed-limit 3000000 --speed-time 60 \
     -H "$AUTH" "$URL" -o "$OUT.part" --progress-bar
mv "$OUT.part" "$OUT"
ls -lh "$OUT" | awk '{print "  完成:",$9,$5}'
