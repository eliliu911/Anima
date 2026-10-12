//! 项目知识库：把 `.anima/knowledge/` 下的文档切块、向量化、按项目隔离检索。
//!
//! ## 为什么单独一张表而不是塞进 facts
//!
//! 「事实」和「文档块」是两种东西，混在一张表里会同时坏掉两边：
//!
//! | | 事实 | 文档块 |
//! |---|---|---|
//! | 来源 | 模型从对话里提取 | 用户放进目录的文件 |
//! | 粒度 | 一句话 | 几百字符，带出处 |
//! | 更新 | 追加 + 查重 | **整个项目重建**（文件改了旧块必须失效） |
//! | 召回对象 | 陪伴链路的 System Prompt | agent 主动调工具查 |
//!
//! 最后一行是关键：重建索引要能「删掉这个项目的全部旧块再写新的」，
//! 而事实**绝不能**被这个动作波及。放同一张表就得靠 where 条件小心避让，
//! 而一次写错就是静默删掉用户的长期记忆。
//!
//! ## 这个模块存在之前发生了什么
//!
//! `workspace_reindex_memory` 曾经是这样的：
//!
//! ```ignore
//! for entry in entries.flatten() { if entry.path().is_file() { count += 1; } }
//! // 模拟建立知识库向量检索索引
//! tokio::time::sleep(std::time::Duration::from_millis(300)).await;
//! Ok(count)
//! ```
//!
//! 数文件、睡 300 毫秒装作在忙、返回个数。而前端拿到之后报
//! 「⚡ 向量知识库重构完成！已建立 N 篇文档语义索引」，init 生成的
//! `architecture.md` 里也写着「Anima 将在对话中自动索引」——
//! **一个假承诺有两个出口，其中一个还是模型会读到的文件。**

use std::sync::Arc;

use arrow_array::types::Float32Type;
use arrow_array::{
    FixedSizeListArray, Float32Array, Int64Array, RecordBatch, RecordBatchIterator, StringArray,
};
use arrow_schema::{DataType, Field, Schema};
use futures_util::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase};
use lancedb::{connect, Connection, Table};
use serde::{Deserialize, Serialize};

use super::embed::l2_distance_to_similarity;
use crate::error::{Error, Result};

const TABLE_NAME: &str = "knowledge";

/// 一块的字符上限。
///
/// 实测这个项目的中英混合文本约 2.35 字符/token，所以 800 字符 ≈ 340 token ——
/// 一次召回三块也就一千出头，塞进 agent 的上下文不心疼。
///
/// **上限不是目标值**：切块优先按标题和段落的自然边界来，只有一个段落本身
/// 超过上限时才硬切。把语义完整的一节切两半，比留一块 1200 字符更伤召回。
const MAX_CHUNK_CHARS: usize = 800;

/// 短于这个长度的块直接丢掉。
///
/// 「## 2. 核心模块划分」这种只有标题没有内容的块，向量化之后是纯噪声：
/// 它和任何提到「模块」的查询都有中等相似度，却不含任何信息。
const MIN_CHUNK_CHARS: usize = 24;

/// 索引里的一块文档
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chunk {
    pub id: String,
    /// 属于哪个项目。规范化后的工作区绝对路径，见 [`scope_of`]
    pub scope: String,
    /// 出处文件名（相对 `.anima/knowledge/`），召回时要能告诉模型这块来自哪
    pub doc: String,
    /// 在该文件里是第几块，从 0 开始。让模型能判断先后顺序
    pub index: i64,
    pub text: String,
}

/// 检索结果
#[derive(Debug, Clone, Serialize)]
pub struct KnowledgeHit {
    #[serde(flatten)]
    pub chunk: Chunk,
    pub similarity: f32,
}

/// 把工作区路径规范成 scope 值。
///
/// **必须只有这一个实现。** 写入时用规范化路径、检索时用原始路径的话，
/// 过滤条件永远不匹配，表现为「索引明明建好了但一条也搜不到」，而且不报错。
///
/// `canonicalize` 失败（目录不存在）时退回去掉尾斜杠的原字符串 ——
/// 那种情况下检索本来也没有意义，但至少写入和检索仍然一致。
pub fn scope_of(workspace_path: &str) -> String {
    let trimmed = workspace_path.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    match std::fs::canonicalize(trimmed) {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(_) => trimmed.trim_end_matches('/').to_string(),
    }
}

/// LanceDB 的 Connection/Table 内部是 Arc 共享的，Clone 很廉价且指向同一个库。
#[derive(Clone)]
pub struct KnowledgeStore {
    _conn: Connection,
    table: Table,
    dimensions: usize,
}

impl KnowledgeStore {
    pub async fn open(dir: &std::path::Path, dimensions: usize) -> Result<Self> {
        if dimensions == 0 {
            return Err(Error::config("向量维度不能为 0"));
        }
        std::fs::create_dir_all(dir)?;

        let uri = dir.to_string_lossy().to_string();
        let conn = connect(&uri)
            .execute()
            .await
            .map_err(|e| Error::Io(format!("打开 LanceDB 失败：{e}")))?;

        let existing = conn
            .table_names()
            .execute()
            .await
            .map_err(|e| Error::Io(format!("读取表列表失败：{e}")))?;

        let table = if existing.iter().any(|n| n == TABLE_NAME) {
            let table = conn
                .open_table(TABLE_NAME)
                .execute()
                .await
                .map_err(|e| Error::Io(format!("打开 knowledge 表失败：{e}")))?;
            // 和 facts 表同一条理由：维度不一致时显式报错，不静默写坏。
            // 差别是这张表可以整个重建（源文件都还在磁盘上），所以提示词是"重建索引"
            // 而不是"删库"。
            let actual = vector_dimensions(&table).await?;
            if actual != dimensions {
                return Err(Error::config(format!(
                    "知识库索引是 {actual} 维，当前嵌入模型输出 {dimensions} 维。\
                     换嵌入模型后需要重建索引（删除 {} 下的 {TABLE_NAME}.lance 再点一次重建）。",
                    dir.display()
                )));
            }
            table
        } else {
            conn.create_empty_table(TABLE_NAME, chunk_schema(dimensions))
                .execute()
                .await
                .map_err(|e| Error::Io(format!("创建 knowledge 表失败：{e}")))?
        };

        Ok(Self {
            _conn: conn,
            table,
            dimensions,
        })
    }

    pub fn dimensions(&self) -> usize {
        self.dimensions
    }

    /// 给迁移用，理由同 `VectorStore::table`
    pub(crate) fn table(&self) -> &Table {
        &self.table
    }

    /// 用新内容**整体替换**某个项目的索引。
    ///
    /// 先删后写，而且即使 `chunks` 为空也要删 —— 用户把文档全删了再点重建，
    /// 期望的是索引也空掉。少了这一步，删掉的文档会永远留在召回结果里。
    pub async fn replace_scope(
        &self,
        scope: &str,
        chunks: &[Chunk],
        vectors: &[Vec<f32>],
    ) -> Result<usize> {
        if chunks.len() != vectors.len() {
            return Err(Error::config(format!(
                "块与向量数量不符：{} vs {}",
                chunks.len(),
                vectors.len()
            )));
        }
        for (i, v) in vectors.iter().enumerate() {
            if v.len() != self.dimensions {
                return Err(Error::config(format!(
                    "第 {i} 块的向量维度为 {}，期望 {}",
                    v.len(),
                    self.dimensions
                )));
            }
        }

        self.table
            .delete(&format!("scope = '{}'", escape_sql(scope)))
            .await
            .map_err(|e| Error::Io(format!("清除旧索引失败：{e}")))?;

        if chunks.is_empty() {
            return Ok(0);
        }

        let schema = chunk_schema(self.dimensions);
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(StringArray::from(
                    chunks.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
                )),
                Arc::new(StringArray::from(
                    chunks.iter().map(|c| c.scope.as_str()).collect::<Vec<_>>(),
                )),
                Arc::new(StringArray::from(
                    chunks.iter().map(|c| c.doc.as_str()).collect::<Vec<_>>(),
                )),
                Arc::new(Int64Array::from(
                    chunks.iter().map(|c| c.index).collect::<Vec<_>>(),
                )),
                Arc::new(StringArray::from(
                    chunks.iter().map(|c| c.text.as_str()).collect::<Vec<_>>(),
                )),
                Arc::new(
                    FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
                        vectors
                            .iter()
                            .map(|v| Some(v.iter().map(|x| Some(*x)).collect::<Vec<_>>())),
                        self.dimensions as i32,
                    ),
                ),
            ],
        )
        .map_err(|e| Error::decode(format!("构造 Arrow 批次失败：{e}")))?;

        let reader = RecordBatchIterator::new(vec![Ok(batch)], schema);
        let reader: Box<dyn arrow_array::RecordBatchReader + Send> = Box::new(reader);
        self.table
            .add(reader)
            .execute()
            .await
            .map_err(|e| Error::Io(format!("写入知识库索引失败：{e}")))?;
        Ok(chunks.len())
    }

    /// 在某个项目的索引里语义检索。
    ///
    /// **scope 过滤下推给 LanceDB 而不是查完再筛。** 取回 `limit` 条再过滤的话，
    /// 别的项目的块会挤掉本项目的 —— 项目一多，返回的结果就越来越少，
    /// 而且完全没有迹象说明为什么。
    pub async fn search(
        &self,
        scope: &str,
        query_vector: &[f32],
        limit: usize,
        min_similarity: f32,
    ) -> Result<Vec<KnowledgeHit>> {
        if query_vector.len() != self.dimensions {
            return Err(Error::config(format!(
                "查询向量维度为 {}，期望 {}",
                query_vector.len(),
                self.dimensions
            )));
        }
        if limit == 0 {
            return Ok(Vec::new());
        }

        let batches: Vec<RecordBatch> = self
            .table
            .vector_search(query_vector.to_vec())
            .map_err(|e| Error::Io(format!("构造向量查询失败：{e}")))?
            .only_if(format!("scope = '{}'", escape_sql(scope)))
            .limit(limit)
            .execute()
            .await
            .map_err(|e| Error::Io(format!("知识库检索失败：{e}")))?
            .try_collect()
            .await
            .map_err(|e| Error::Io(format!("读取检索结果失败：{e}")))?;

        let mut out = Vec::new();
        for batch in batches {
            out.extend(decode_batch(&batch)?);
        }
        out.retain(|h| h.similarity >= min_similarity);
        out.sort_by(|a, b| b.similarity.total_cmp(&a.similarity));
        Ok(out)
    }

    /// 某个项目索引了多少块
    pub async fn count(&self, scope: &str) -> Result<usize> {
        self.table
            .count_rows(Some(format!("scope = '{}'", escape_sql(scope))))
            .await
            .map_err(|e| Error::Io(format!("统计索引块数失败：{e}")))
    }
}

/// 拼 SQL 字面量时单引号是唯一能逃出去的字符。路径里出现单引号不常见但完全合法。
fn escape_sql(s: &str) -> String {
    s.replace('\'', "''")
}

fn chunk_schema(dimensions: usize) -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("scope", DataType::Utf8, false),
        Field::new("doc", DataType::Utf8, false),
        Field::new("chunk_index", DataType::Int64, false),
        Field::new("text", DataType::Utf8, false),
        Field::new(
            "vector",
            DataType::FixedSizeList(
                Arc::new(Field::new("item", DataType::Float32, true)),
                dimensions as i32,
            ),
            true,
        ),
    ]))
}

async fn vector_dimensions(table: &Table) -> Result<usize> {
    let schema = table
        .schema()
        .await
        .map_err(|e| Error::Io(format!("读取表 schema 失败：{e}")))?;
    let field = schema
        .field_with_name("vector")
        .map_err(|e| Error::decode(format!("知识库索引缺少 vector 字段：{e}")))?;
    match field.data_type() {
        DataType::FixedSizeList(_, dim) => Ok(*dim as usize),
        other => Err(Error::decode(format!("vector 字段类型异常：{other:?}"))),
    }
}

fn decode_batch(batch: &RecordBatch) -> Result<Vec<KnowledgeHit>> {
    let strings = |name: &str| -> Result<StringArray> {
        batch
            .column_by_name(name)
            .and_then(|c| c.as_any().downcast_ref::<StringArray>().cloned())
            .ok_or_else(|| Error::decode(format!("检索结果缺少字符串字段 {name}")))
    };
    let ids = strings("id")?;
    let scopes = strings("scope")?;
    let docs = strings("doc")?;
    let texts = strings("text")?;
    let indices = batch
        .column_by_name("chunk_index")
        .and_then(|c| c.as_any().downcast_ref::<Int64Array>().cloned())
        .ok_or_else(|| Error::decode("chunk_index 不是 int64"))?;

    // 和 facts 表同样的约定：拿不到 _distance 说明这不是向量检索（例如全表扫描），
    // 距离没有意义，当作完全匹配。
    let distances = batch
        .column_by_name("_distance")
        .and_then(|c| c.as_any().downcast_ref::<Float32Array>().cloned());

    let mut out = Vec::with_capacity(batch.num_rows());
    for i in 0..batch.num_rows() {
        out.push(KnowledgeHit {
            chunk: Chunk {
                id: ids.value(i).to_string(),
                scope: scopes.value(i).to_string(),
                doc: docs.value(i).to_string(),
                index: indices.value(i),
                text: texts.value(i).to_string(),
            },
            similarity: distances
                .as_ref()
                .map(|d| l2_distance_to_similarity(d.value(i)))
                .unwrap_or(1.0),
        });
    }
    Ok(out)
}

/// 把一份 Markdown 切成块。
///
/// ## 为什么要把标题带进正文
///
/// 一块「可以将产品需求说明、API 接口定义放入本目录」单独看不知道在说什么目录。
/// 每块前面拼上它所处的标题路径（`# 架构 > ## 快速接入指南`），召回出来的内容
/// 才是自解释的 —— 模型拿到的是一段话，不是一段话加一个它看不见的上下文。
///
/// ## 为什么不按固定长度滑窗
///
/// 滑窗会把「## 3. 安全约束」的第一条和「## 4. 产物规范」的最后一条切进同一块。
/// 检索时那一块对两个主题都是中等相似度，对哪个都不是好答案。
/// Markdown 的标题本来就是作者标出来的语义边界，用它比猜一个窗口大小可靠。
pub fn chunk_markdown(doc_name: &str, content: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    // 标题栈：(层级, 文本)。遇到同级或更高级标题时弹出更深的层
    let mut heading_stack: Vec<(usize, String)> = Vec::new();
    let mut body: Vec<&str> = Vec::new();

    let flush = |stack: &[(usize, String)], body: &mut Vec<&str>, out: &mut Vec<(String, String)>| {
        let text = body.join("\n").trim().to_string();
        body.clear();
        if text.is_empty() {
            return;
        }
        let prefix = if stack.is_empty() {
            String::new()
        } else {
            format!(
                "{}\n\n",
                stack
                    .iter()
                    .map(|(_, t)| t.as_str())
                    .collect::<Vec<_>>()
                    .join(" > ")
            )
        };
        for piece in split_long(&text) {
            out.push((doc_name.to_string(), format!("{prefix}{piece}")));
        }
    };

    for line in content.lines() {
        if let Some(level) = heading_level(line) {
            flush(&heading_stack, &mut body, &mut out);
            while heading_stack.last().is_some_and(|(l, _)| *l >= level) {
                heading_stack.pop();
            }
            heading_stack.push((level, line.trim().to_string()));
        } else {
            body.push(line);
        }
    }
    flush(&heading_stack, &mut body, &mut out);

    out.retain(|(_, t)| t.chars().count() >= MIN_CHUNK_CHARS);
    out
}

/// `## foo` → Some(2)。要求 `#` 后面有空格，避免把代码块里的 `#!/bin/sh`
/// 和 Rust 的 `#[derive]` 当成标题。
fn heading_level(line: &str) -> Option<usize> {
    let t = line.trim_start();
    let hashes = t.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    if t.chars().nth(hashes) == Some(' ') {
        Some(hashes)
    } else {
        None
    }
}

/// 一节超过上限时按段落边界切开；单个段落还超就按行；单行还超就硬切。
///
/// 逐级降级而不是直接按字符数切：能保住段落就保住段落。
fn split_long(text: &str) -> Vec<String> {
    if text.chars().count() <= MAX_CHUNK_CHARS {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut current = String::new();
    for para in text.split("\n\n") {
        let para = para.trim_end();
        if para.is_empty() {
            continue;
        }
        if current.chars().count() + para.chars().count() + 2 > MAX_CHUNK_CHARS
            && !current.is_empty()
        {
            out.push(std::mem::take(&mut current));
        }
        if para.chars().count() > MAX_CHUNK_CHARS {
            if !current.is_empty() {
                out.push(std::mem::take(&mut current));
            }
            out.extend(hard_split(para));
        } else {
            if !current.is_empty() {
                current.push_str("\n\n");
            }
            current.push_str(para);
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn hard_split(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        if current.chars().count() + line.chars().count() + 1 > MAX_CHUNK_CHARS
            && !current.is_empty()
        {
            out.push(std::mem::take(&mut current));
        }
        if line.chars().count() > MAX_CHUNK_CHARS {
            // 单行超长（压缩过的 JSON、没有换行的长段）：按字符边界切，
            // 用 chars() 而不是字节切片 —— 后者会在多字节字符中间 panic
            let chars: Vec<char> = line.chars().collect();
            for piece in chars.chunks(MAX_CHUNK_CHARS) {
                out.push(piece.iter().collect());
            }
            continue;
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// 读取 `.anima/knowledge/` 下的所有文本文档并切块。
///
/// 只认文本扩展名：目录里放了 PDF 或图片时，把二进制当 UTF-8 读会得到乱码，
/// 向量化之后是纯污染，而且用户完全看不出发生了什么。
pub fn collect_chunks(knowledge_dir: &std::path::Path, scope: &str) -> Vec<Chunk> {
    const TEXT_EXT: &[&str] = &["md", "markdown", "txt", "json", "yaml", "yml", "toml", "csv"];

    let Ok(entries) = std::fs::read_dir(knowledge_dir) else {
        return Vec::new();
    };
    let mut files: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .map(|e| TEXT_EXT.contains(&e.to_ascii_lowercase().as_str()))
                .unwrap_or(false)
        })
        .collect();
    // 排序让同一批文档的 id 稳定可复现，方便比对两次重建的差异
    files.sort();

    let mut out = Vec::new();
    for path in files {
        let Ok(content) = std::fs::read_to_string(&path) else {
            tracing::warn!(path = %path.display(), "知识库文档读取失败，跳过");
            continue;
        };
        let doc_name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        for (i, (doc, text)) in chunk_markdown(&doc_name, &content).into_iter().enumerate() {
            out.push(Chunk {
                id: uuid::Uuid::new_v4().to_string(),
                scope: scope.to_string(),
                doc,
                index: i as i64,
                text,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 标题被拼进正文所以每块自解释() {
        let md = "# 架构\n\n## 快速接入指南\n\n可以把产品需求说明放入本目录，Anima 会在重建索引后收录。\n";
        let chunks = chunk_markdown("architecture.md", md);
        assert_eq!(chunks.len(), 1, "{chunks:?}");
        let (doc, text) = &chunks[0];
        assert_eq!(doc, "architecture.md");
        // 「本目录」指的是哪个目录，只有带上标题路径才说得清
        assert!(text.starts_with("# 架构 > ## 快速接入指南"), "{text}");
        assert!(text.contains("产品需求说明"));
    }

    #[test]
    fn 同级标题会弹出而不是层层累积() {
        let md = "# 顶\n\n## 一\n\n第一节的内容要足够长才不会被最小长度过滤掉。\n\n## 二\n\n第二节的内容也要足够长才不会被最小长度过滤掉。\n";
        let chunks = chunk_markdown("d.md", md);
        assert_eq!(chunks.len(), 2, "{chunks:?}");
        assert!(chunks[0].1.starts_with("# 顶 > ## 一"), "{}", chunks[0].1);
        // 曾经的 bug 形状：标题栈只 push 不 pop，第二节会变成 "# 顶 > ## 一 > ## 二"
        assert!(chunks[1].1.starts_with("# 顶 > ## 二"), "{}", chunks[1].1);
    }

    #[test]
    fn 只有标题没有内容的块被丢掉() {
        // 这种块和任何提到「模块划分」的查询都有中等相似度，却不含信息
        let chunks = chunk_markdown("d.md", "# 标题\n\n## 2. 核心模块划分\n\n## 3. 下一节\n");
        assert!(chunks.is_empty(), "{chunks:?}");
    }

    #[test]
    fn 井号后面没空格的不算标题() {
        // Rust 的 #[derive] 和 shebang 都长得像标题
        assert_eq!(heading_level("## 标题"), Some(2));
        assert_eq!(heading_level("#[derive(Debug)]"), None);
        assert_eq!(heading_level("#!/bin/sh"), None);
        assert_eq!(heading_level("####### 七个井号"), None);
        assert_eq!(heading_level("普通正文"), None);
    }

    #[test]
    fn 超长段落被切开且每块都不超上限() {
        let long: String = std::iter::repeat_n("这是一个很长的段落。", 300).collect();
        let chunks = chunk_markdown("d.md", &format!("# 标题\n\n{long}\n"));
        assert!(chunks.len() > 1, "应该被切成多块，实际 {}", chunks.len());
        for (_, t) in &chunks {
            // 上限只约束正文，标题前缀是额外附加的
            assert!(
                t.chars().count() <= MAX_CHUNK_CHARS + 32,
                "有一块 {} 字符",
                t.chars().count()
            );
        }
    }

    #[test]
    fn 单行超长时按字符切不会在多字节中间断开() {
        // 按字节切会 panic；这里全是三字节字符，能可靠地暴露那个错误
        let one_line: String = std::iter::repeat_n('中', MAX_CHUNK_CHARS * 2 + 5).collect();
        let pieces = hard_split(&one_line);
        assert!(pieces.len() >= 3, "{}", pieces.len());
        assert_eq!(
            pieces.iter().map(|p| p.chars().count()).sum::<usize>(),
            MAX_CHUNK_CHARS * 2 + 5
        );
    }

    #[test]
    fn scope_对不存在的路径也要写读一致() {
        // canonicalize 失败时的退路：至少写入和检索用的是同一个值
        let a = scope_of("/nope/does/not/exist/");
        let b = scope_of("/nope/does/not/exist");
        assert_eq!(a, b, "尾斜杠不能造成 scope 不一致");
        assert_eq!(scope_of("   "), "");
    }

    #[test]
    fn sql_字面量里的单引号被转义() {
        assert_eq!(escape_sql("/Users/o'brien/p"), "/Users/o''brien/p");
    }

    #[test]
    fn 只收文本扩展名() {
        let dir = std::env::temp_dir().join(format!("anima-k-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("a.md"),
            "# 甲\n\n这一段内容足够长，能通过最小长度过滤。\n",
        )
        .unwrap();
        // 把二进制当 UTF-8 读会得到乱码，向量化之后是纯污染
        std::fs::write(dir.join("b.png"), [0x89, 0x50, 0x4e, 0x47]).unwrap();
        let chunks = collect_chunks(&dir, "S");
        assert_eq!(chunks.len(), 1, "{chunks:?}");
        assert_eq!(chunks[0].doc, "a.md");
        assert_eq!(chunks[0].scope, "S");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
