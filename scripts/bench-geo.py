#!/usr/bin/env python3
"""eBayManager 的 GEO 生成能力对比。

## 为什么另开一个脚本

GEO 和 Anima 要的东西**几乎相反**：Anima 要中文创作、要模型自由发挥；
GEO 要美式英文、要**绝对不许发挥**（源数据没写的一律省略）。
两套判据混在一起会互相污染，所以分开。

判据全部照 `TSP.EbayManager/backend/.../EbayContentRules.cs` 的红线来，
包括那条它自己写在注释里的死循环：

    「幻觉/无源类 → 明确要求【删除】，禁止换同义词/替代值
     (治 "aluminum→durable metal" 死循环)」

用法：

    ./scripts/with-env.sh python3 scripts/bench-geo.py --models "ornith-1.0-35b,ornith-1.5-35b-a3b"
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request

HOST = os.environ.get("LMSTUDIO_HOST", "http://127.0.0.1:1234")
TOKEN = os.environ.get("ANIMA_API_KEY_LMSTUDIO", "")
LMS = os.path.expanduser("~/.lmstudio/bin/lms")

TITLE_MAX = 80  # EbayContentRules.TitleMax

# 照抄生产的 system prompt 骨架（红线部分节选，保留会被判据检到的那几条）
SYSTEM = (
    "You are an expert eBay Motors listing copywriter for automotive performance parts.\n"
    "Use ONLY facts supported by the provided listing context. If data is missing, omit it instead of guessing.\n"
    "Prefer structured, concrete, buyer-useful wording over hype.\n"
    "eBay LISTING RED LINES (hard constraints — never violate):\n"
    "- Title MUST be <= 80 characters.\n"
    "- MUST NOT contain contact info, external links, price, or payment terms.\n"
    "- MUST NOT use promo spam (ALL-CAPS words, \"best\", \"cheap\"), asterisks, or emojis.\n"
    "- MUST NOT fabricate specs, measurements, fitment, certifications, or horsepower claims.\n"
    "- If the source data does not support a value, OMIT it — never guess or default "
    "(do not assume a color, material, or dimension).\n"
    "- Brand MUST be the PART maker, NEVER a vehicle make.\n"
    "- Audience: US English, automotive performance-parts buyers.\n\n"
    "Return ONLY the requested content — no explanations, no markdown code fences."
)

# 源数据刻意留了几个坑：
#   1. 材质没写      → 看它会不会编 "aluminum" / "billet"
#   2. 年份是 2022-2024 → 看它会不会按平台常识扩成 2021-2025
#   3. 交叉推荐段落里有别的年份和别的品牌 → 看它会不会串进本商品
SOURCE = """SKU: TSP-8054
Title: Top Street Performance Valve Cover Set Super Cap 2022-2024 6.6L Duramax
Brand: Top Street Performance
Category: eBay Motors > Parts & Accessories > Car & Truck Parts > Engines & Components
Spec table:
  Part Type: Valve Cover Set
  Quantity: 2
  Finish: Black
  Baffled: Yes
  Grommets Included: Yes
  Fitment: 2022-2024 6.6L Duramax
  Feature: "Super Cap" design — wider terminal spacing reduces spark crossfire and adds
           plug-wire-boot clearance
Notes: Gaskets are NOT included. Requires removal of factory heat shield.
--- Customers also viewed (DO NOT use for this product) ---
  DNJ Cylinder Head 2015-2021 5.3L Vortec, cast aluminum, Chevrolet Silverado
"""


def task_title() -> dict:
    return {
        "key": "title",
        "title": "标题（<=80 字符）",
        "prompt": SOURCE + "\n=== TASK ===\nGenerate the title for this listing. "
        f"Output ONE title only, MUST be {TITLE_MAX} characters or fewer. No quotes/markdown. "
        "Front-load the most searched terms: [Brand] [Part type] [key spec] [fitment cue]. "
        "Keep the engine platform code if the source names one. "
        "MODEL YEARS: use EXACTLY the year range the source states — never widen or infer a broader range.",
        "max_tokens": 2000,
    }


def task_selling() -> dict:
    return {
        "key": "sellingpoints",
        "title": "卖点（JSON 5 条，不许编材质）",
        "prompt": SOURCE + "\n=== TASK ===\nGenerate the sellingPoints. "
        "Output a JSON array of exactly 5 concise selling-point strings. Each string should contain one "
        "concrete spec, buyer benefit, authority signal, or use-case. No promo fluff, no duplicates, "
        "no unsupported claims. Do NOT invent material/finish/dimension facts not in the source data "
        "(e.g. do not claim \"6061 aluminum\" or an aluminum housing unless the source states it).",
        "max_tokens": 2500,
    }


def task_schema() -> dict:
    return {
        "key": "schema",
        "title": "JSON-LD（禁 offers/price）",
        "prompt": SOURCE + "\n=== TASK ===\nGenerate the schema. "
        "Output valid JSON-LD only (no markdown). Use schema.org Product as @type. "
        "Required: @context, @type, name, description. Include a brand object when brand is known, and "
        "additionalProperty entries only for concrete specs/fitment facts supported by the source data. "
        "Do NOT include offers, price, availability, shipping, returns, or seller contact data. "
        # 实测补丁：1.5 会把 Notes 当散文丢掉，1.0 会转成属性。
        # 这一句就是为了让「Gaskets are NOT included」变成可被 AI 搜索引用的结构化事实
        "Also convert Notes and exclusions into additionalProperty entries whenever they express a "
        "concrete yes/no or required/not-required fact (e.g. \"Gaskets are NOT included\" -> "
        "{\"name\":\"Gaskets Included\",\"value\":\"No\"}; \"Requires removal of factory heat shield\" -> "
        "{\"name\":\"Heat Shield Removal Required\",\"value\":\"Yes\"}).",
        "max_tokens": 3000,
    }


def task_delete_loop() -> dict:
    """生产代码里明确写过的死循环：要求删掉无源词时，模型换个模糊同义词而不是删。"""
    return {
        "key": "deletion",
        "title": "按要求删除（不许换同义词）",
        "prompt": SOURCE
        + "\n=== PREVIOUS QA FOUND (must fix, do not repeat) ===\n"
        "- [sellingPoints]: value \"aluminum construction\" — the source/spec table does not state any "
        "material → FIX BY DELETION: remove this exact word/phrase from the content; do NOT swap it for a "
        "synonym, a vaguer term (e.g. \"durable metal\"), or another value — if the source does not state "
        "it, it must not appear at all.\n"
        "\n=== TASK ===\nRegenerate the sellingPoints (JSON array of 5 strings) with that defect fixed.",
        "max_tokens": 2500,
    }


TASKS = [task_title(), task_selling(), task_schema(), task_delete_loop()]

# 生产代码 ContentGenerationService.cs:904 用来检测「思考泄漏进输出」的正则
LEAK_RX = re.compile(
    r"\b(the user wants|user wants|i need to|i will|constraints?:|task:|generate the|"
    r"requested content|ebay listing title for)\b",
    re.I,
)
# 红线里明令禁止的推广词/符号
SPAM_RX = re.compile(r"\b(best|cheap|wow|l@@k)\b|[*✨🔥]|\bBUY NOW\b", re.I)
# 源里没有的材质词 —— 出现即幻觉
INVENTED = ["aluminum", "aluminium", "billet", "steel", "6061", "cast iron", "durable metal", "alloy"]


def call(model: str, task: dict, timeout: int = 900) -> dict:
    body = {
        "model": model,
        "messages": [
            {"role": "system", "content": SYSTEM},
            {"role": "user", "content": task["prompt"]},
        ],
        "max_tokens": task["max_tokens"],
        "temperature": 0.3,   # GEO 要稳定不要创意
        "stream": False,
    }
    req = urllib.request.Request(
        f"{HOST}/api/v0/chat/completions",
        data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json", "Authorization": f"Bearer {TOKEN}"},
    )
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return json.load(r)
    except urllib.error.HTTPError as e:
        return {"error": f"HTTP {e.code}: {e.read().decode()[:160]}"}
    except Exception as e:  # noqa: BLE001
        return {"error": f"{type(e).__name__}: {e}"}


def strip_fence(t: str) -> str:
    return re.sub(r"^```(?:json|html)?|```$", "", t.strip(), flags=re.M).strip()


def grade(task: dict, out: dict) -> tuple[str, list[str]]:
    """返回 (判定, 问题列表)。判据全部照生产的红线。"""
    if "error" in out:
        return "❌", [out["error"][:70]]
    text = (out.get("choices", [{}])[0].get("message", {}).get("content") or "").strip()
    if not text:
        rt = (out.get("usage", {}).get("completion_tokens_details") or {}).get("reasoning_tokens", 0)
        return "❌", [f"预算耗尽于思考（{rt} tok），无输出"]

    bad: list[str] = []
    raw = strip_fence(text)

    # 所有类型共有的红线
    if LEAK_RX.search(text):
        bad.append(f"思考泄漏（命中生产正则：{LEAK_RX.search(text).group(0)!r}）")
    if SPAM_RX.search(text):
        bad.append(f"推广词/符号：{SPAM_RX.search(text).group(0)!r}")

    k = task["key"]
    if k == "title":
        one = raw.splitlines()[0].strip().strip('"')
        if len(one) > TITLE_MAX:
            bad.append(f"超长 {len(one)}/{TITLE_MAX} 字符")
        if len(raw.splitlines()) > 1:
            bad.append(f"输出了 {len(raw.splitlines())} 行，要求只有一行")
        if "duramax" not in one.lower():
            bad.append("丢了发动机平台码 Duramax")
        if re.search(r"20(1[0-9]|2[01])|202[5-9]", one):
            bad.append(f"年份被扩了：{one}")
        elif "2022" not in one and "22-24" not in one.replace(" ", ""):
            bad.append("没带源里的年份 2022-2024")
        if re.search(r"\b(vortec|silverado|chevrolet|chevy|dnj)\b", one, re.I):
            bad.append("串进了交叉推荐里别的商品/品牌")

    if k in ("sellingpoints", "deletion"):
        try:
            arr = json.loads(raw)
            if not isinstance(arr, list):
                bad.append("不是 JSON 数组")
            elif len(arr) != 5:
                bad.append(f"{len(arr)} 条，要求 5 条")
        except json.JSONDecodeError:
            bad.append("不是合法 JSON")
        hit = [w for w in INVENTED if re.search(rf"\b{re.escape(w)}\b", raw, re.I)]
        if hit:
            bad.append(f"编造材质：{hit}")

    if k == "schema":
        try:
            obj = json.loads(raw)
            for f in ("@context", "@type", "name", "description"):
                if f not in obj:
                    bad.append(f"缺必填字段 {f}")
            for f in ("offers", "price", "availability", "aggregateRating", "review"):
                if f in obj:
                    bad.append(f"含禁用字段 {f}")
            props = json.dumps(obj.get("additionalProperty", []), ensure_ascii=False)
            hit = [w for w in INVENTED if re.search(rf"\b{re.escape(w)}\b", props, re.I)]
            if hit:
                bad.append(f"additionalProperty 里编造材质：{hit}")
        except json.JSONDecodeError:
            bad.append("不是合法 JSON-LD")

    return ("✅" if not bad else ("⚠️" if len(bad) == 1 else "❌")), bad


def bench(model: str) -> dict:
    print(f"\n{'━' * 76}\n  {model}\n{'━' * 76}", flush=True)
    subprocess.run([LMS, "unload", "--all"], capture_output=True, timeout=180)
    t0 = time.time()
    warm = call(model, {"prompt": "hi", "max_tokens": 8}, timeout=1800)
    if "error" in warm:
        print(f"  ✗ 加载失败：{warm['error'][:90]}", flush=True)
        return {"model": model, "error": warm["error"]}
    print(f"  加载耗时 {time.time() - t0:.0f}s", flush=True)

    res: dict = {"model": model, "tasks": {}}
    for task in TASKS:
        out = call(model, task)
        mark, bad = grade(task, out)
        st, u = out.get("stats", {}) or {}, out.get("usage", {}) or {}
        rt = (u.get("completion_tokens_details") or {}).get("reasoning_tokens", 0)
        txt = (out.get("choices", [{}])[0].get("message", {}).get("content") or "")
        res["tasks"][task["key"]] = {
            "mark": mark, "issues": bad, "reasoning_tokens": rt,
            "tps": st.get("tokens_per_second"), "text": txt[:900],
        }
        tps = f"{st.get('tokens_per_second', 0):5.1f}" if st.get("tokens_per_second") else "  —  "
        print(f"  {mark} {task['title']:26s} {tps} tok/s  思考{rt:>5d}  "
              + ("；".join(bad) if bad else "全部合规"), flush=True)
    ok = sum(1 for t in res["tasks"].values() if t["mark"] == "✅")
    res["pass"] = ok
    return res


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--models", required=True)
    ap.add_argument("--out", default="/tmp/bench-geo.json")
    args = ap.parse_args()
    if not TOKEN:
        print("缺 ANIMA_API_KEY_LMSTUDIO —— 用 ./scripts/with-env.sh 运行", file=sys.stderr)
        return 2

    results = [bench(m) for m in args.models.split(",")]
    print(f"\n{'═' * 76}\n  汇总（GEO 合规）\n{'═' * 76}")
    print(f"  {'模型':34s} {'合规':>5s}  {'判定':10s} 思考tok合计")
    for r in results:
        if r.get("error"):
            print(f"  {r['model']:34s} {'—':>5s}  ❌ 加载失败")
            continue
        marks = "".join(t["mark"] for t in r["tasks"].values())
        rt = sum(t["reasoning_tokens"] or 0 for t in r["tasks"].values())
        print(f"  {r['model']:34s} {r['pass']}/{len(TASKS):<3d} {marks:10s} {rt:>8d}")
    print(f"\n  判定顺序：{' '.join(t['title'] for t in TASKS)}")
    with open(args.out, "w") as f:
        json.dump(results, f, ensure_ascii=False, indent=2)
    print(f"  完整输出 → {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
