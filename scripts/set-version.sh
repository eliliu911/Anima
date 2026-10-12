#!/usr/bin/env bash
# 改版本号。**四处必须一起改** —— package.json / Cargo.toml / Cargo.lock / tauri.conf.json。
#
# 为什么要这个脚本：四个文件各写一份，手工同步迟早漏一处，而漏了之后
# 界面显示的版本、更新器比较的版本、bundle 里的版本会互相打架，**且不报错**。
# 这个仓库栽在「一份清单抄在两处」上已经很多次了。
#
# 漏了也不会悄悄过去 —— `src/version.test.ts` 会读这四份文件比对。
#
# # 什么时候改
#
# 用户要求（2026-09-23）：每次完成改动都递增版本号，小修复递增 patch。
# 只有单纯重新构建未修改代码时不改版本号。
#
# 档位的判据和完整规划写在 `docs/DECISIONS.md` 的「版本号怎么走」。
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

CUR="$(node -p "require('${ROOT}/package.json').version")"

# 按档位算下一个版本号。**纯计算，不写文件** —— 方便 usage 里预览，
# 也方便测试（`--next <档位> [起始版本]` 只打印结果、什么都不改）。
bump() {
  python3 - "${2:-$CUR}" "$1" <<'BUMP_PY'
import re, sys

cur, kind = sys.argv[1], sys.argv[2]
m = re.match(r"^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?$", cur)
if not m:
    sys.exit(f"当前版本号读不懂：{cur}")
major, minor, patch = (int(m.group(i)) for i in (1, 2, 3))
pre = m.group(4)

if kind == "patch":
    # **预发布转正不是再进一位**：0.9.1-beta.3 → 0.9.1，不是 0.9.2
    print(f"{major}.{minor}.{patch}" if pre else f"{major}.{minor}.{patch + 1}")
elif kind == "minor":
    print(f"{major}.{minor + 1}.0")
elif kind == "major":
    print(f"{major + 1}.0.0")
elif kind == "release":
    if not pre:
        sys.exit(f"{cur} 本来就不是预发布版")
    print(f"{major}.{minor}.{patch}")
elif kind == "beta":
    b = re.match(r"^beta\.(\d+)$", pre) if pre else None
    if b:
        print(f"{major}.{minor}.{patch}-beta.{int(b.group(1)) + 1}")
    elif pre:
        sys.exit(f"当前预发布后缀是 {pre}，不是 beta.N —— 手工写完整版本号")
    else:
        print(f"{major}.{minor}.{patch + 1}-beta.1")
else:
    sys.exit(f"不认识的档位：{kind}")
BUMP_PY
}

usage() {
  printf '用法: %s <版本号 | patch | minor | major | beta | release>\n\n' "$0"
  printf '  patch    改了但用法不变（修 bug、文档、内部重构、打包修复）  → %s\n' "$(bump patch 2>/dev/null || echo '?')"
  printf '  minor    多了能做的事，或者原来的做法要调整                  → %s\n' "$(bump minor 2>/dev/null || echo '?')"
  printf '  major    0→1 只发生一次：能给别人用了                       → %s\n' "$(bump major 2>/dev/null || echo '?')"
  printf '  beta     想让人试，但不想让 updater 推给所有人               → %s\n' "$(bump beta 2>/dev/null || echo '?')"
  printf '  release  预发布转正，去掉 -beta.N\n'
  printf '\n  每次完成改动都递增版本号；小修复用 patch，重建未修改代码不用改。\n'
  printf '  判据和规划：docs/DECISIONS.md「版本号怎么走」\n'
  printf '\n当前：\n'
  grep -m1 '"version"' "$ROOT/package.json"      | sed 's/^/  package.json      /'
  grep -m1 '^version'  "$ROOT/src-tauri/Cargo.toml" | sed 's/^/  Cargo.toml        /'
  grep -m1 '"version"' "$ROOT/src-tauri/tauri.conf.json" | sed 's/^/  tauri.conf.json   /'
}

# 只算不改。**给测试和「先看看会变成什么」用** —— 版本号算错的后果
# （退版本 / 预发布转正多进一位）从现象上很难查回来，必须能被测试驱动。
if [ "${1:-}" = "--next" ]; then
  [ -n "${2:-}" ] || { printf '用法: %s --next <patch|minor|major|beta|release> [起始版本]\n' "$0" >&2; exit 2; }
  bump "$2" "${3:-}"
  exit $?
fi

VER="${1:-}"
case "$VER" in
  ""|-h|--help|help) usage; exit 1 ;;
  patch|minor|major|beta|release)
    NEXT="$(bump "$VER")" || exit 1
    printf '%s：%s → %s\n\n' "$VER" "$CUR" "$NEXT"
    VER="$NEXT"
    ;;
esac

# 语义化版本，三段数字。updater 要靠它比大小，写错格式会导致「永远没有新版本」
if ! printf '%s' "$VER" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$'; then
  printf '✗ 版本号格式不对：%s（要 x.y.z，可带 -beta.1 这类后缀）\n' "$VER" >&2
  exit 1
fi

# **不能往回退。** updater 按 semver 比大小，退版本号会让所有人卡在旧版
# 且不报错。真要退（发错了）就先手工删掉 tag，再显式写目标版本并加 --force
if [ "${2:-}" != "--force" ]; then
  python3 - "$CUR" "$VER" <<'CMP_PY' || exit 1
import re, sys

def key(v):
    m = re.match(r"^(\d+)\.(\d+)\.(\d+)(?:-(.+))?$", v)
    nums = tuple(int(m.group(i)) for i in (1, 2, 3))
    pre = m.group(4)
    # 预发布排在同版本的正式版之前：0.9.1-beta.1 < 0.9.1
    return (nums, 0 if pre else 1, pre or "")

cur, new = sys.argv[1], sys.argv[2]
if key(new) < key(cur):
    sys.exit(f"✗ {new} 比当前的 {cur} 小 —— updater 按大小比，退版本会让所有人卡在旧版且不报错。\n"
             f"  真要退就加 --force，并记得先删掉已经推出去的 tag。")
if key(new) == key(cur):
    sys.exit(f"✗ {new} 和当前一样，没改")
CMP_PY
fi

python3 - "$ROOT" "$VER" <<'PY'
import json, re, sys, pathlib
root, ver = pathlib.Path(sys.argv[1]), sys.argv[2]

# package.json / tauri.conf.json：改 JSON 但**不重排其余内容**，用正则只动第一处 version
for rel in ("package.json", "src-tauri/tauri.conf.json"):
    p = root / rel
    s = p.read_text()
    s2, n = re.subn(r'("version"\s*:\s*)"[^"]*"', lambda m: m.group(1) + f'"{ver}"', s, count=1)
    assert n == 1, f"{rel} 里没找到 version 字段"
    p.write_text(s2)
    print(f"  {rel} → {ver}")

# Cargo.toml：只动 [package] 段第一处 version，别碰依赖的版本号
p = root / "src-tauri/Cargo.toml"
s = p.read_text()
s2, n = re.subn(r'(?m)^(version\s*=\s*)"[^"]*"', lambda m: m.group(1) + f'"{ver}"', s, count=1)
assert n == 1, "Cargo.toml 里没找到 version"
p.write_text(s2)
print(f"  src-tauri/Cargo.toml → {ver}")

# Cargo.lock 里也有一份。
#
# **不改它的话下次 cargo build 会自己改**，于是工作区凭空多一个已跟踪文件的
# 改动 —— 而 `anima update` 和 `release.sh` 都因为「工作区不干净」拒绝工作，
# 界面上还会标一个 `-dirty`。查起来完全想不到是改版本号留下的。
#
# 只动 `name = "anima"` 紧跟着的那一行，别碰几百个依赖的版本号。
p = root / "src-tauri/Cargo.lock"
s = p.read_text()
s2, n = re.subn(r'(?m)^(name = "anima"\nversion = )"[^"]*"', lambda m: m.group(1) + f'"{ver}"', s, count=1)
assert n == 1, "Cargo.lock 里没找到 anima 这个包"
p.write_text(s2)
print(f"  src-tauri/Cargo.lock → {ver}")
PY

printf '\n改完了。记得跑一次 `npx vitest run version` 确认四处一致。\n'
