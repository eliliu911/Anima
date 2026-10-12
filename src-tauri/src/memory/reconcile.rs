//! 写入时收敛：新事实和已有的近邻是同一件事、矛盾、还是细化。
//!
//! ## 补的是什么缺口
//!
//! 原先只有一道 0.93 的向量查重。它挡得住措辞几乎相同的重复，挡不住
//! **同一件事的两种说法**。本机真实库里同时存在：
//!
//! ```text
//! 用户：姓名是刘松公子
//! 用户：我叫刘松，是你的开发者，你可以叫我"主人"，"公子"或者"亲爱的"
//! 用户：最近在看《凡人修仙传》
//! 用户：正在阅读《凡人修仙传》
//! ```
//!
//! 而 `recall_limit = 5` —— 上面这两组重复已经吃掉 4 个位置，一次召回里
//! 真正的信息量只剩一条。这不是理论担忧，是当前库里就成立的算术。
//!
//! ## 为什么矛盾必须是一等结论
//!
//! 只做「重复就跳过」的话，**矛盾的两条会同时留在库里，然后同时被召回**，
//! 而模型没有任何依据去选。这是「表意不准」的最大来源 —— 不是缺记忆，
//! 是冲突的记忆。「用 pnpm」和「用 npm」同时回来，后果比重复严重得多。
//!
//! 所以判定有四种结论，`contradicts` 会**删掉旧的**。
//!
//! ## 这个删除是有风险的，而且我们接受它
//!
//! 如果新提取是一次误读，它会毁掉一条正确的旧事实。缓解只有两条：
//! 删除时把两条原文都记进 info 日志；以及设置面板里本来就能浏览和删除事实。
//!
//! 不这么做的代价更大 —— 一个只增不减的库，召回精度会随时间单调下降。

use serde::Deserialize;

use crate::config::Tier;
use crate::error::Result;
use crate::llm::gateway::Gateway;
use crate::llm::{ChatChunk, ChatRequest, Message, ThinkingMode};

/// 进入 reconcile 的相似度下限。
///
/// 低于它的不做判定，直接当新事实 —— 省一次模型调用，而且那个区间里
/// 「同一件事的两种说法」已经很罕见。
///
/// 上限是 `DEDUPE_SIMILARITY`（0.93）：那之上是措辞几乎相同的确定重复，
/// 直接跳过，也不必问模型。
pub const RECONCILE_SIMILARITY: f32 = 0.80;

/// 一次判定最多看几个近邻。3 个足够看清局部簇，再多只是拉长提示词。
pub const MAX_NEIGHBORS: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// 新的一件事，直接入库
    New,
    /// 和第 `index` 条（0 基）是同一件事，跳过新的
    Duplicate { index: usize },
    /// 和第 `index` 条矛盾。**删掉旧的，写入新的**
    Contradicts { index: usize },
    /// 是第 `index` 条的更完整版本。删掉旧的，写入 `text`
    Refines { index: usize, text: String },
}

const PROMPT: &str = r#"你在维护一个长期记忆库。判断【新事实】和【已有事实】的关系。

只输出这四种之一：
- "new"        —— 说的是不同的事
- "duplicate"  —— 和某一条是同一件事，措辞不同而已
- "contradicts" —— 和某一条说的是同一个属性，但**内容不相容**（不能同时为真）
- "refines"    —— 和某一条是同一件事，但新的更完整/更准确，应当合并

判断要点：
- "用 pnpm" 和 "用 npm" 是 contradicts（同一个属性，不能同时为真）
- "姓名是张三" 和 "叫张三，可以称他老张" 是 refines（后者更完整）
- "测试用 npm test" 和 "构建用 npm run build" 是 new（两个不同的属性）
- 不确定就输出 "new" —— 多存一条比错删一条好

严格只输出 JSON，不要解释、不要 markdown 围栏：
{"verdict":"refines","index":0,"text":"合并后的完整表述"}

index 是【已有事实】的编号（从 0 开始）。verdict 为 "new" 时不需要 index。
text 只在 "refines" 时需要，其余情况可省略。"#;

#[derive(Deserialize)]
struct RawVerdict {
    #[serde(default, alias = "Verdict", alias = "relation")]
    verdict: Option<String>,
    #[serde(default, alias = "Index", alias = "target")]
    index: Option<i64>,
    #[serde(default, alias = "Text", alias = "merged", alias = "fact")]
    text: Option<String>,
}

/// 从模型输出里解析判定。
///
/// **解析不出来一律当 `New`。** 记忆收敛是「有则更好」的增强 ——
/// 让一次解析失败去删掉一条已有事实是不可接受的，而多存一条只是噪声。
pub fn parse(raw: &str, neighbor_count: usize) -> Verdict {
    let Some(json) = extract_json_object(raw) else {
        return Verdict::New;
    };
    let Ok(v) = serde_json::from_str::<RawVerdict>(&json) else {
        return Verdict::New;
    };
    let kind = v.verdict.unwrap_or_default().to_ascii_lowercase();
    let kind = kind.trim();

    // ── index 的处理 ──
    //
    // 越界当 New：模型指了一条不存在的邻居，说明它没在认真判定。
    //
    // **只有一个近邻时，缺失的 index 补成 0。** 这一条是实测逼出来的：
    // `openai/gpt-oss-120b`（本项目的后台档）在单近邻时稳定输出
    // `{"verdict":"contradicts"}` —— 判断完全正确，只是省掉了那个没有歧义的
    // 编号。原先一律退回 New，于是**正确的矛盾判定被当成解析失败丢掉**，
    // 而这只在后台档上发生（qwen3.8 / ornith 都会带 index）——
    // 我最初就是用那两个验的，所以没看见。
    //
    // 多个近邻时缺 index 仍然保守：那时候「指哪一条」是真的有歧义，
    // 而猜错会删掉一条正确的事实。
    let idx = match v.index {
        Some(i) if i >= 0 && (i as usize) < neighbor_count => i as usize,
        None if neighbor_count == 1 => 0,
        _ => {
            return Verdict::New;
        }
    };

    match kind {
        "duplicate" | "dup" | "same" => Verdict::Duplicate { index: idx },
        "contradicts" | "contradict" | "conflict" => Verdict::Contradicts { index: idx },
        "refines" | "refine" | "merge" => {
            let text = v.text.unwrap_or_default().trim().to_string();
            // refines 但没给合并文本 —— 退化成 Duplicate（保留旧的），
            // 而不是删掉旧的写入一个空字符串
            if text.is_empty() {
                Verdict::Duplicate { index: idx }
            } else {
                Verdict::Refines { index: idx, text }
            }
        }
        _ => Verdict::New,
    }
}

/// 问一次后台模型。
pub async fn judge(
    gateway: &Gateway,
    candidate: &str,
    neighbors: &[String],
) -> Result<Verdict> {
    if neighbors.is_empty() {
        return Ok(Verdict::New);
    }
    let listed = neighbors
        .iter()
        .enumerate()
        .map(|(i, n)| format!("{i}. {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    let body = format!("【新事实】\n{candidate}\n\n【已有事实】\n{listed}");

    let mut req = ChatRequest::new(vec![Message::system(PROMPT), Message::user(body)]);
    req.temperature = Some(0.0);
    req.max_tokens = Some(256);
    // 判定不需要思考，只会拖慢并吃掉 token 预算
    req.thinking = ThinkingMode::Off;

    let stream_id = format!("memory-reconcile-{}", uuid::Uuid::new_v4());
    let (_, mut stream) = gateway
        .chat_stream(stream_id, Tier::Background, req)
        .await?;

    use futures_util::StreamExt;
    let mut text = String::new();
    while let Some(chunk) = stream.next().await {
        match chunk? {
            ChatChunk::Text { delta } => text.push_str(&delta),
            ChatChunk::Done { .. } => break,
            _ => {}
        }
    }
    Ok(parse(&text, neighbors.len()))
}

/// 从可能带围栏和解释的输出里抠出第一个完整 JSON 对象。
///
/// 和 `extract::extract_json_object` 是同一个需求，但**刻意不共用** ——
/// 那边是提取事实数组的解析器，共用之后任何一边改坏都会静默影响另一边，
/// 而这两处的输入形状不同（一个是数组、一个是单对象）。二十行代码不值得
/// 冒那个风险。
fn extract_json_object(raw: &str) -> Option<String> {
    let bytes = raw.as_bytes();
    let start = raw.find('{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for i in start..bytes.len() {
        let c = bytes[i];
        if in_string {
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == b'"' {
                in_string = false;
            }
            continue;
        }
        match c {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(raw[start..=i].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 四种结论都能解析() {
        assert_eq!(parse(r#"{"verdict":"new"}"#, 2), Verdict::New);
        assert_eq!(
            parse(r#"{"verdict":"duplicate","index":1}"#, 2),
            Verdict::Duplicate { index: 1 }
        );
        assert_eq!(
            parse(r#"{"verdict":"contradicts","index":0}"#, 2),
            Verdict::Contradicts { index: 0 }
        );
        assert_eq!(
            parse(r#"{"verdict":"refines","index":0,"text":"合并后"}"#, 2),
            Verdict::Refines {
                index: 0,
                text: "合并后".into()
            }
        );
    }

    #[test]
    fn 解析失败一律当新事实() {
        // **让一次解析失败去删掉一条已有事实是不可接受的。**
        // 多存一条只是噪声，错删是数据丢失。
        for bad in [
            "完全不是 JSON",
            "",
            "{}",
            r#"{"verdict":"什么鬼","index":0}"#,
            r#"{"index":0}"#,
        ] {
            assert_eq!(parse(bad, 2), Verdict::New, "{bad}");
        }
    }

    #[test]
    fn index_越界当新事实() {
        // 模型指了一条不存在的邻居，说明它没在认真判定
        assert_eq!(parse(r#"{"verdict":"duplicate","index":5}"#, 2), Verdict::New);
        assert_eq!(parse(r#"{"verdict":"duplicate","index":-1}"#, 2), Verdict::New);
        // 多个近邻时缺 index 是真的有歧义，破坏性结论不能生效
        assert_eq!(parse(r#"{"verdict":"contradicts"}"#, 2), Verdict::New);
        assert_eq!(parse(r#"{"verdict":"refines","text":"x"}"#, 3), Verdict::New);
    }

    #[test]
    fn 单个近邻时缺index要补成0() {
        // **这条是实测逼出来的。** gpt-oss-120b（后台档）在单近邻时稳定输出
        // `{"verdict":"contradicts"}` —— 判断对，只是省掉了没有歧义的编号。
        // 原先一律退回 New，正确的矛盾判定被当成解析失败丢掉，
        // 而这只在后台档上发生（qwen3.8 / ornith 都会带 index）。
        assert_eq!(
            parse(r#"{"verdict":"contradicts"}"#, 1),
            Verdict::Contradicts { index: 0 }
        );
        assert_eq!(
            parse(r#"{"verdict":"duplicate"}"#, 1),
            Verdict::Duplicate { index: 0 }
        );
        assert_eq!(
            parse(r#"{"verdict":"refines","text":"合并"}"#, 1),
            Verdict::Refines { index: 0, text: "合并".into() }
        );
        // 但 new 仍然是 new —— 补 index 不该把它变成破坏性结论
        assert_eq!(parse(r#"{"verdict":"new"}"#, 1), Verdict::New);
    }

    #[test]
    fn refines_没给文本时退化成保留旧的() {
        // 而不是删掉旧的写入一个空字符串
        assert_eq!(
            parse(r#"{"verdict":"refines","index":1}"#, 2),
            Verdict::Duplicate { index: 1 }
        );
        assert_eq!(
            parse(r#"{"verdict":"refines","index":1,"text":"   "}"#, 2),
            Verdict::Duplicate { index: 1 }
        );
    }

    #[test]
    fn 容忍围栏解释和字段别名() {
        assert_eq!(
            parse("这是我的判断：\n```json\n{\"verdict\":\"duplicate\",\"index\":0}\n```\n以上", 1),
            Verdict::Duplicate { index: 0 }
        );
        // 本地小模型会把字段名写成 relation / target / merged
        assert_eq!(
            parse(r#"{"relation":"refines","target":0,"merged":"m"}"#, 1),
            Verdict::Refines { index: 0, text: "m".into() }
        );
        // 大小写不敏感
        assert_eq!(
            parse(r#"{"verdict":"CONTRADICTS","index":0}"#, 1),
            Verdict::Contradicts { index: 0 }
        );
    }

    #[test]
    fn 事实文本里的花括号不会提前收尾() {
        let raw = r#"{"verdict":"refines","index":0,"text":"配置写 {\"a\":1} 就行"}"#;
        assert_eq!(
            parse(raw, 1),
            Verdict::Refines {
                index: 0,
                text: "配置写 {\"a\":1} 就行".into()
            }
        );
    }

    #[test]
    fn 没有近邻时不问模型() {
        // judge 是 async，这里只验它的前置判断不需要网络
        assert_eq!(parse(r#"{"verdict":"duplicate","index":0}"#, 0), Verdict::New);
    }

    #[test]
    fn 提示词给了四种关系各自的例子() {
        // 本地小模型判不出抽象定义，靠例子。少了任何一个例子都会让那一类失效
        assert!(PROMPT.contains("pnpm"), "缺 contradicts 的例子");
        assert!(PROMPT.contains("老张"), "缺 refines 的例子");
        assert!(PROMPT.contains("npm run build"), "缺 new 的例子");
        // 兜底方向必须写明：宁可多存
        assert!(PROMPT.contains("不确定就输出"), "没给兜底方向");
    }
}
