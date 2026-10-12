#!/usr/bin/env bash
# 在**隔离副本**里做变异测试。
#
# 为什么必须隔离：变异测试要把改坏的代码写进文件才能验证测试抓不抓得住，
# 而 `npm run tauri:dev` 的 HMR 会立刻把那份坏代码推进正在跑的界面。
# 真出过事 —— 用户截图里出现了一个只存在于变异期间的 `<p>空</p>`，
# 以为是我们的 bug。
#
# 用法：
#   scripts/mut.sh sync                     # 把当前工作区同步进副本
#   scripts/mut.sh run <vitest 过滤词>       # 在副本里跑前端测试
#   scripts/mut.sh cargo <cargo 过滤词>      # 在副本里跑 Rust 测试
#   scripts/mut.sh path                     # 打印副本路径（用来改文件）
#   scripts/mut.sh clean                    # 把副本恢复干净（**做完必须跑**）
#
# 典型流程：sync → 改副本里的文件 → run → clean
#
# **做完一定要 clean。** 副本是个 git worktree，pre-commit 护栏会把
# 「另一个工作区里有未提交改动」当成两个 agent 在抢同一个文件而拦下提交 ——
# 它拦得对，只是这次那个"另一个 agent"是我自己。
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
W="${ANIMA_MUT_WORKTREE:-/tmp/anima-mut}"

ensure() {
  if [ ! -e "$W/.git" ]; then
    git -C "$ROOT" worktree add -f --detach "$W" HEAD >/dev/null
  fi
  # **副本建好之后主干还在往前走。** 副本一直停在建它那天的提交上，
  # 而 sync 只同步源码、不同步清单文件 —— 于是 `Cargo.toml` 会慢慢过期。
  #
  # 2026-09-18 撞上：副本停在 56aba3b，缺了后来加的 `libc` 依赖和
  # `build.rs` 里的两个环境变量，**整个 lib 编译不过**。报错是
  # 「找不到 crate libc」「环境变量未定义」—— 完全看不出是副本旧了，
  # 很容易当成自己刚写的变异把代码改坏了。
  local HEAD_SHA; HEAD_SHA=$(git -C "$ROOT" rev-parse HEAD)
  if [ "$(git -C "$W" rev-parse HEAD)" != "$HEAD_SHA" ]; then
    git -C "$W" checkout --detach -f "$HEAD_SHA" >/dev/null 2>&1
  fi
  # node_modules 用软链，不复制 —— 几百 MB 复制一次要很久
  [ -e "$W/node_modules" ] || ln -s "$ROOT/node_modules" "$W/node_modules"
}

sync_tree() {
  ensure
  # **只同步源码**。dist / target / node_modules 不动，
  # 否则一次同步比跑测试还慢。
  #
  # **不保留修改时间（没有 -t），按内容比对（--checksum）。** 用 -a 的话，
  # 变异后再同步会把原文件连同**更早的**修改时间拷回去，cargo 看时间戳比上次编译旧，
  # 不重新编译 —— 副本里跑的还是变异版。2026-09-13 真出过：实跑结果里场名少了一截，
  # 查了半天才发现跑的是上一个变异。按内容比对后只有真变了的文件会被写，写的时刻就是新时间。
  local RS=(rsync -rlpD --checksum --delete)
  "${RS[@]}" "$ROOT/src/" "$W/src/"
  "${RS[@]}" "$ROOT/src-tauri/src/" "$W/src-tauri/src/"
  # shared/ 里是两边共读的词表 —— 不同步它，改 JSON 的变异会静默不生效
  # （副本里还是 HEAD 那一份），测试照绿，而那是假绿。
  "${RS[@]}" "$ROOT/shared/" "$W/shared/"
  # skills/ 里有被 `include_str!` 编进二进制的编剧手艺
  # （`src-tauri/src/screenwriter.rs`）—— 不同步它副本根本编译不过，
  # 而报错是「找不到文件」，看不出是同步漏了。
  "${RS[@]}" "$ROOT/skills/" "$W/skills/"
  rsync -a "$ROOT/package.json" "$ROOT/vitest.config.ts" "$W/" 2>/dev/null || true
  # **清单和 build.rs 也要跟。** 上面 `ensure` 已经把副本挪到 HEAD，
  # 这里管的是主工作区里**还没提交**的清单改动 —— 加了依赖还没提交时，
  # 副本会缺那个 crate 而编译不过。
  rsync -a "$ROOT/src-tauri/Cargo.toml" "$ROOT/src-tauri/Cargo.lock" \
           "$ROOT/src-tauri/build.rs" "$ROOT/src-tauri/tauri.conf.json" \
           "$W/src-tauri/" 2>/dev/null || true
}

case "${1:-}" in
  sync) sync_tree ;;
  path) ensure; echo "$W" ;;
  clean)
    # 恢复到 HEAD。副本脏着的时候 pre-commit 护栏会拦下主工作区的提交。
    if [ -e "$W/.git" ]; then
      git -C "$W" checkout -- . 2>/dev/null || true
      git -C "$W" clean -fdq -e node_modules 2>/dev/null || true
    fi
    ;;
  run)  ensure; cd "$W" && npx vitest run "${2:-}" ;;
  # **编译产物放主工作区的 target，不在副本里另攒一份。** 2026-09-28：副本自己的 target 一天长到 29 GB
  # （每轮变异完整编一遍所有依赖），和一个任务副本的 26 GB 一起吃掉了 60 GB 硬盘。共用之后依赖只编一次，
  # 副本里被改的这一个 crate 路径不同、cargo 会单独编一份（一两个 GB），不会和主工作区的互相覆盖；
  # 它们都在主 target 里，构建后的自动清理（build_gc）扫得到。
  cargo) ensure; cd "$W/src-tauri" && CARGO_TARGET_DIR="$ROOT/src-tauri/target" cargo test --lib "${2:-}" ;;
  *) echo "用法: $0 {sync|path|run <过滤词>|cargo <过滤词>}" >&2; exit 2 ;;
esac
