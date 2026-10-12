#!/usr/bin/env bash
# 给每个 agent 一份自己的 checkout，避免多个 agent 在同一个工作区里互相提交对方的代码。
#
# 起因是真实事故：两个 agent 共用 /Users/you/AI/workspace/Anima，
# 其中一个跑了 `git add -A`，把另一个正在写、还没提交的代码和评审文档一起
# 提交进了自己的 commit。后果是报告归属失真、重复劳动，
# 而且随时可能在对方代码不编译的瞬间把主干搞断。
#
# 用法：
#   ./scripts/agent-worktree.sh new anti      # 建 ../Anima-anti，分支 agent/anti
#   ./scripts/agent-worktree.sh list
#   ./scripts/agent-worktree.sh drop anti     # 合并/丢弃后清理
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

usage() { sed -n '2,14p' "${BASH_SOURCE[0]}" | sed 's/^# \?//'; exit 1; }
[ $# -ge 1 ] || usage

case "$1" in
  new)
    [ $# -eq 2 ] || usage
    name="$2"; dir="$(dirname "$ROOT")/$(basename "$ROOT")-$name"; branch="agent/$name"
    if [ -e "$dir" ]; then echo "已存在：$dir"; exit 1; fi
    if git show-ref --verify --quiet "refs/heads/$branch"; then
      git worktree add "$dir" "$branch"
    else
      git worktree add "$dir" -b "$branch"
    fi
    echo
    echo "✓ $name 的独立工作区：$dir"
    echo "  分支：${branch}（与 main 共享同一个对象库，不占额外的 .git 体积）"
    echo
    echo "  让那个 agent 改用这个目录，然后："
    echo "    cd \"$dir\" && git rebase main   # 随时同步主干"
    echo "    git push -u origin $branch      # 推自己的分支，不直接推 main"
    ;;
  list)
    git worktree list
    ;;
  drop)
    [ $# -eq 2 ] || usage
    name="$2"; dir="$(dirname "$ROOT")/$(basename "$ROOT")-$name"
    git worktree remove "$dir" && git branch -d "agent/$name" 2>/dev/null || true
    echo "✓ 已清理 $dir"
    ;;
  *) usage ;;
esac
