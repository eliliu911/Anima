#!/usr/bin/env python3
"""代码编辑基准：针对 Anima 的 apply_patch 契约。

判定不靠肉眼——模型产出的 patch 真的去应用一遍，对不上就是失败。
量化损伤在代码上的典型表现是「上下文行几乎对但差一个字符」，
这种错误读起来完全正常，只有真的 apply 才会暴露。
"""
import json, os, re, sys, time, urllib.request

BASE = "http://127.0.0.1:1234/v1"
KEY = os.environ.get("ANIMA_API_KEY_LMSTUDIO", "")
ROOT = "/Users/you/AI/workspace/Anima"


def post(path, payload, timeout=1800):
    req = urllib.request.Request(
        BASE + path, data=json.dumps(payload).encode(),
        headers={"Content-Type": "application/json",
                 "Authorization": f"Bearer {KEY}"})
    return json.loads(urllib.request.urlopen(req, timeout=timeout).read())


def unload_all():
    os.system("lms unload --all >/dev/null 2>&1")
    time.sleep(2)


# ─────────────────────────── 素材：真实源文件片段 ───────────────────────────
def excerpt(relpath, start, end):
    lines = open(os.path.join(ROOT, relpath), encoding="utf-8").read().splitlines()
    return "\n".join(lines[start - 1:end]), lines


SRC_REL = "src-tauri/src/llm/sse.rs"
if not os.path.exists(os.path.join(ROOT, SRC_REL)):
    SRC_REL = "src-tauri/src/error.rs"
SNIPPET, FULL_LINES = excerpt(SRC_REL, 1, 70)

APPLY_PATCH_TOOL = {
    "type": "function",
    "function": {
        "name": "apply_patch",
        "description": ("应用 Codex 风格 patch。优先传 patch 字段，格式以 *** Begin Patch 开始、"
                        "*** End Patch 结束；也兼容旧的 path/search/replace。"),
        "parameters": {
            "type": "object",
            "properties": {
                "patch": {"type": "string", "description": "Codex apply_patch 文本"},
                "path": {"type": "string"},
                "search": {"type": "string", "description": "唯一待替换片段"},
                "replace": {"type": "string"},
            },
        },
    },
}


def call(model, messages, tools=None, budget=4096):
    body = {"model": model, "messages": messages, "max_tokens": budget, "temperature": 0.2}
    if tools:
        body["tools"] = tools
        body["tool_choice"] = "auto"
    t0 = time.time()
    d = post("/chat/completions", body)
    dt = time.time() - t0
    ch = d["choices"][0]["message"]
    u = d.get("usage", {})
    ct = u.get("completion_tokens", 0)
    return {
        "text": ch.get("content") or "",
        "tool_calls": ch.get("tool_calls") or [],
        "tps": ct / dt if dt > 0 else 0,
        "completion_tokens": ct,
        "elapsed": dt,
    }


# ───────────────────────────── 校验器 ─────────────────────────────
def _rtrim_lines(t):
    return [l.rstrip() for l in t.split("\n")]


def _rtrim_unique(content, old):
    """复刻 find_rtrim_line_matches：按行尾空白容错的整行序列匹配。"""
    cl = _rtrim_lines(content)
    ol = _rtrim_lines(old)
    while ol and ol[-1] == "":
        ol.pop()
    if not ol or len(ol) > len(cl):
        return 0
    n = 0
    for i in range(len(cl) - len(ol) + 1):
        if cl[i:i + len(ol)] == ol:
            n += 1
    return n



def _normalize_dialect(lines):
    """与 Rust 的 normalize_patch_dialect 一一对应。"""
    out, i = [], 0
    while i < len(lines):
        line = lines[i]
        bare = line.rstrip("* ")
        if bare in ("*** Begin Patch", "*** End Patch"):
            out.append(bare); i += 1; continue
        if line.startswith("--- ") and i + 1 < len(lines) and lines[i+1].startswith("+++ "):
            o = line[4:]; n = lines[i+1][4:]
            o = o[2:] if o.startswith("a/") else o
            n = n[2:] if n.startswith("b/") else n
            hunk = i + 2 < len(lines) and lines[i+2].startswith("@@")
            if o == n and n and hunk:
                out.append(f"*** Update File: {n}"); i += 2; continue
        out.append(line); i += 1
    return out

def verify_codex_patch(patch_text, source_text):
    """忠实复刻 Anima 的 parse_codex_patch + replace_unique_chunk。"""
    if not patch_text:
        return False, "没有 patch 字段"
    lines = [l.rstrip() for l in patch_text.replace("\r\n", "\n").split("\n")]
    while lines and lines[-1] == "":
        lines.pop()
    lines = _normalize_dialect(lines)
    if not lines or lines[0] != "*** Begin Patch":
        got = lines[0][:46] if lines else "(空)"
        return False, f"首行必须严格是 '*** Begin Patch'，实际是 {got!r}"
    if "*** End Patch" not in lines:
        return False, "缺少 *** End Patch"

    i, ops = 1, 0
    while i < len(lines):
        line = lines[i]
        if line == "*** End Patch":
            break
        if line.startswith("*** Add File: ") or line.startswith("*** Delete File: "):
            ops += 1
            i += 1
            while i < len(lines) and lines[i] != "*** End Patch" and not lines[i].startswith("*** "):
                i += 1
            continue
        if not line.startswith("*** Update File: "):
            return False, f"无法识别的指令行：{line[:46]!r}"
        i += 1
        if i < len(lines) and lines[i].startswith("*** Move to: "):
            i += 1
        chunks = 0
        while i < len(lines) and lines[i] != "*** End Patch" and not lines[i].startswith("*** "):
            if not lines[i].startswith("@@"):
                return False, f"Update File 需要 @@ chunk 头，实际：{lines[i][:42]!r}"
            i += 1
            old = ""
            while (i < len(lines) and not lines[i].startswith("@@")
                   and lines[i] != "*** End Patch" and not lines[i].startswith("*** ")):
                ln = lines[i]
                if ln == "*** End of File":
                    i += 1
                    continue
                if ln == "":
                    old += "\n"
                    i += 1
                    continue
                pfx, text = ln[0], ln[1:]
                if pfx == " ":
                    old += text + "\n"
                elif pfx == "-":
                    old += text + "\n"
                elif pfx == "+":
                    pass
                else:
                    return False, f"chunk 行必须以 空格/+/- 开头，实际：{ln[:42]!r}"
                i += 1
            if not old:
                return False, "chunk 原文为空"
            exact = source_text.count(old)
            if exact > 1:
                return False, f"chunk 匹配到 {exact} 处，Anima 会拒绝模糊修改"
            if exact == 0:
                n = _rtrim_unique(source_text, old)
                if n == 0:
                    first = old.split("\n")[0][:46]
                    return False, f"chunk 未匹配到原文，首行 {first!r}"
                if n > 1:
                    return False, f"行尾空白容错后仍匹配 {n} 处，拒绝"
            chunks += 1
        if chunks == 0:
            return False, "Update File 没有任何 chunk"
        ops += 1
    if ops == 0:
        return False, "patch 里没有任何操作"
    return True, f"解析通过，{ops} 个操作，chunk 全部唯一命中"


def verify_search_replace(args, source_text):
    s = args.get("search")
    if not s:
        return False, "没给 search"
    n = source_text.count(s)
    if n == 0:
        return False, f"search 片段在源码里不存在：{s.strip()[:52]!r}"
    if n > 1:
        return False, f"search 片段出现 {n} 次（有歧义，Anima 会拒绝）"
    return True, "search 片段唯一"


# ───────────────────────────── 任务 ─────────────────────────────
def t_codex_patch(model):
    msg = [
        {"role": "system", "content": "你是代码编辑助手。用 apply_patch 工具修改文件，必须使用 patch 字段（Codex 格式）。"},
        {"role": "user", "content":
            f"文件 `{SRC_REL}` 的前 70 行如下：\n\n```rust\n{SNIPPET}\n```\n\n"
            "请在这个文件里，给第一个 `pub` 项上方加一行注释 `// benchmark marker`。"
            "调用 apply_patch，用 patch 字段给出 Codex 格式补丁。"},
    ]
    r = call(model, msg, [APPLY_PATCH_TOOL])
    if not r["tool_calls"]:
        return r, False, "没有发起工具调用"
    try:
        args = json.loads(r["tool_calls"][0]["function"]["arguments"])
    except Exception as e:
        return r, False, f"参数不是合法 JSON：{e}"
    if "patch" not in args:
        return r, False, "没用 patch 字段（用了旧兼容模式）"
    ok, note = verify_codex_patch(args["patch"], SNIPPET)
    return r, ok, note


def t_unique_anchor(model):
    msg = [
        {"role": "system", "content": "你是代码编辑助手。用 apply_patch 的 path/search/replace 模式。search 片段必须在文件里唯一，否则会被拒绝。"},
        {"role": "user", "content":
            f"文件 `{SRC_REL}` 的前 70 行：\n\n```rust\n{SNIPPET}\n```\n\n"
            "把文件里第一个 `use` 语句所在行原样保留，但在它下面插入 `use std::fmt;`。"
            "调用 apply_patch，用 path/search/replace，search 必须唯一。"},
    ]
    r = call(model, msg, [APPLY_PATCH_TOOL])
    if not r["tool_calls"]:
        return r, False, "没有发起工具调用"
    try:
        args = json.loads(r["tool_calls"][0]["function"]["arguments"])
    except Exception as e:
        return r, False, f"参数不是合法 JSON：{e}"
    ok, note = verify_search_replace(args, SNIPPET)
    return r, ok, note


def t_longctx(model):
    big, _ = excerpt("src-tauri/src/agent_runtime/local_tools.rs", 1, 900)
    # 只考真实出现在这 900 行里的工具名（apply_patch 在 902 行，故意排除）
    truth = {"view_file", "fs_write_text", "shell_command", "web_search",
             "exec_command", "workspace_search", "fs_list_directory"}
    assert all(f'"{t}"' in big for t in truth), "题面自检失败"
    msg = [
        {"role": "user", "content":
            f"下面是 Rust 源码：\n\n```rust\n{big}\n```\n\n"
            "只回答：这段代码里注册了哪些工具名（工具的 name 字符串）？每行一个，不要解释。"},
    ]
    r = call(model, msg, budget=1024)
    got = set(re.findall(r"[a-z_]{4,24}", r["text"]))
    hit = len(got & truth)
    return r, hit >= 5, f"命中 {hit}/{len(truth)} 个真实工具名"


def t_speed(model):
    msg = [{"role": "user", "content": "写一个 Rust 函数，把 snake_case 转成 camelCase，带 3 个单元测试。只给代码。"}]
    r = call(model, msg, budget=1200)
    ok = "fn " in r["text"] and "#[test]" in r["text"]
    return r, ok, f"{r['completion_tokens']} tok"


TASKS = [("Codex 补丁可应用", t_codex_patch),
         ("search 锚点唯一", t_unique_anchor),
         ("长上下文读码", t_longctx),
         ("代码生成/测速", t_speed)]


def main():
    models = sys.argv[1].split(",")
    out = []
    for m in models:
        unload_all()
        print(f"\n{'━'*74}\n  {m}\n{'━'*74}")
        t0 = time.time()
        try:
            call(m, [{"role": "user", "content": "hi"}], budget=4)
        except Exception as e:
            print(f"  ✗ 加载失败: {str(e)[:160]}")
            out.append({"model": m, "load_failed": str(e)[:300]})
            continue
        print(f"  加载耗时 {time.time()-t0:.0f}s")
        rec = {"model": m, "tasks": {}}
        for name, fn in TASKS:
            try:
                r, ok, note = fn(m)
            except Exception as e:
                print(f"  ✗ {name:16s} 异常 {type(e).__name__}: {str(e)[:70]}")
                rec["tasks"][name] = {"ok": False, "note": f"异常 {e}"}
                continue
            print(f"  {'✅' if ok else '❌'} {name:16s} {r['tps']:6.1f} tok/s  {note}")
            rec["tasks"][name] = {"ok": ok, "note": note, "tps": r["tps"],
                                  "tokens": r["completion_tokens"], "text": r["text"][:1500]}
        out.append(rec)
    print(f"\n{'═'*74}\n  汇总\n{'═'*74}")
    print(f"  {'模型':38s} {'tok/s':>7s}  判定")
    for r in out:
        if "load_failed" in r:
            print(f"  {r['model']:38s} {'—':>7s}  加载失败")
            continue
        ts = list(r["tasks"].values())
        tps = sum(t.get("tps", 0) for t in ts) / max(len(ts), 1)
        print(f"  {r['model']:38s} {tps:7.1f}  {''.join('✅' if t['ok'] else '❌' for t in ts)}")
    print(f"\n  判定顺序：{' '.join(n for n, _ in TASKS)}")
    json.dump(out, open("/tmp/bench-code.json", "w"), ensure_ascii=False, indent=1)
    print("  完整输出 → /tmp/bench-code.json")


main()
