//! LanceDB 长期记忆：事实的向量存储与语义召回。
//!
//! 选 LanceDB 而不是「SQLite 存 blob 自己算余弦」：它是无 Server 的嵌入式库，
//! 能直接打包进桌面应用，事实量长到几万条以后还有 ANN 索引兜底
//! （见 docs/DECISIONS.md 的技术栈决策）。代价是构建时需要 `protoc`。
//!
//! ## 维度一旦定下就不能随便换嵌入模型
//!
//! 向量维度由嵌入模型决定（`mxbai-embed-large` 是 1024），而表 schema 把维度写死
//! 在 `FixedSizeList` 里。换一个维度不同的模型会与已建的表冲突，这里**显式检测并
//! 报错**，而不是静默写坏 —— 后者会让召回结果退化成噪声，且极难联想到是换模型
//! 引起的。

use std::sync::Arc;

use arrow_array::types::Float32Type;
use arrow_array::{
    Array, FixedSizeListArray, Float32Array, Int64Array, RecordBatch, RecordBatchIterator,
    StringArray,
};
use arrow_schema::{DataType, Field, Schema};
use futures_util::TryStreamExt;
use lancedb::query::{ExecutableQuery, QueryBase};
use lancedb::table::NewColumnTransform;
use lancedb::{connect, Connection};
pub(crate) use lancedb::Table;
use serde::{Deserialize, Serialize};

use super::embed::l2_distance_to_similarity;
use crate::error::{Error, Result};

const TABLE_NAME: &str = "facts";

/// 一条长期记忆事实
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fact {
    pub id: String,
    /// 事实主体，如「用户」
    pub subject: String,
    /// 事实内容，如「养了一只叫小黑的猫」
    pub fact: String,
    /// 来自哪个会话，便于溯源
    pub session_id: String,
    /// Unix 秒
    pub created_at: i64,
    /// 属于哪个项目。**空串 = 全局**（关于用户本人的事，在任何项目里都相关）。
    ///
    /// 分项目的理由是实测出来的：这个库里累积到的三条事实是「我们来看一看
    /// Anima 项目的具体内容吧」、「我后面会慢慢跟你一起研究这个项目」、
    /// 以及「我們有西瓜汁，很好喝哦」。它们和一次 coding 会话在同一个池子里
    /// 竞争召回额度 —— 而召回额度是从上下文预算里扣的。
    ///
    /// 召回规则见 [`ScopeFilter`]。
    #[serde(default)]
    pub scope: String,
}

/// 召回时的项目过滤。
///
/// **不是简单的"只看本项目"**：关于用户本人的事（职业、偏好、称呼）在任何
/// 项目里都该被召回，而某个项目的工程约定跑到另一个项目里就是噪声。
/// 所以全局事实永远可见，项目事实只在本项目可见。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeFilter {
    /// 不在任何项目里（陪伴对话）：只看全局事实。
    ///
    /// 项目事实**不**在这里出现 —— 否则在闲聊时被告知"订单服务参数不合法回 422"。
    GlobalOnly,
    /// 在某个项目里：全局事实 + 这个项目的事实
    Project(String),
    /// 不过滤。给"浏览它记住了我什么"的界面用 —— 那个界面的意义正是
    /// 看见**全部**，包括本来不该被记住的那些。
    Any,
}

impl ScopeFilter {
    /// 编译成 LanceDB 的 SQL 过滤串。`None` 表示不加过滤条件。
    fn to_sql(&self) -> Option<String> {
        match self {
            Self::Any => None,
            Self::GlobalOnly => Some("scope = ''".to_string()),
            Self::Project(p) => Some(format!(
                "(scope = '' OR scope = '{}')",
                p.replace('\'', "''")
            )),
        }
    }
}

/// 召回结果：事实 + 相似度
#[derive(Debug, Clone, Serialize)]
pub struct Recalled {
    #[serde(flatten)]
    pub fact: Fact,
    /// 余弦相似度，1.0 为完全一致
    pub similarity: f32,
    /// 排序用的最终分（相似度 + 时近性加分）。
    ///
    /// **和 similarity 分开保留**，因为界面要能解释「为什么这条排在前面」——
    /// 只有一个混合分的话，看不出是相关性高还是仅仅比较新。
    #[serde(default)]
    pub score: f32,
}

/// LanceDB 的 Connection/Table 内部都是 Arc 共享的，所以 Clone 很廉价，
/// 且指向同一个库。
#[derive(Clone)]
pub struct VectorStore {
    _conn: Connection,
    table: Table,
    dimensions: usize,
}

impl VectorStore {
    /// 打开（或创建）向量库。`dimensions` 必须与嵌入模型一致。
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
                .map_err(|e| Error::Io(format!("打开 facts 表失败：{e}")))?;

            // 已有表的维度必须与当前嵌入模型一致，否则召回会退化成噪声
            let actual = table_dimensions(&table).await?;
            if actual != dimensions {
                return Err(Error::config(format!(
                    "记忆库的向量维度是 {actual}，当前嵌入模型输出 {dimensions} 维。\
                     换嵌入模型需要重建记忆库（删除 {} 目录），或改回原来的模型。",
                    dir.display()
                )));
            }
            migrate_add_scope(&table).await?;
            table
        } else {
            let schema = fact_schema(dimensions);
            conn.create_empty_table(TABLE_NAME, schema)
                .execute()
                .await
                .map_err(|e| Error::Io(format!("创建 facts 表失败：{e}")))?
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

    /// 给迁移用。**只给迁移** —— 别处要什么就在这里加方法，
    /// 把 `Table` 漏出去会让存储细节散到上层。
    pub(crate) fn table(&self) -> &Table {
        &self.table
    }

    /// 批量写入事实。`facts` 与 `vectors` 必须一一对应。
    pub async fn add(&self, facts: &[Fact], vectors: &[Vec<f32>]) -> Result<()> {
        if facts.is_empty() {
            return Ok(());
        }
        if facts.len() != vectors.len() {
            return Err(Error::config(format!(
                "事实与向量数量不符：{} vs {}",
                facts.len(),
                vectors.len()
            )));
        }
        for (i, v) in vectors.iter().enumerate() {
            if v.len() != self.dimensions {
                return Err(Error::config(format!(
                    "第 {i} 条向量维度为 {}，期望 {}",
                    v.len(),
                    self.dimensions
                )));
            }
        }

        let schema = fact_schema(self.dimensions);
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(StringArray::from(
                    facts.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(),
                )),
                Arc::new(StringArray::from(
                    facts.iter().map(|f| f.subject.as_str()).collect::<Vec<_>>(),
                )),
                Arc::new(StringArray::from(
                    facts.iter().map(|f| f.fact.as_str()).collect::<Vec<_>>(),
                )),
                Arc::new(StringArray::from(
                    facts
                        .iter()
                        .map(|f| f.session_id.as_str())
                        .collect::<Vec<_>>(),
                )),
                Arc::new(Int64Array::from(
                    facts.iter().map(|f| f.created_at).collect::<Vec<_>>(),
                )),
                Arc::new(StringArray::from(
                    facts.iter().map(|f| f.scope.as_str()).collect::<Vec<_>>(),
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
        // 0.33 的 add() 要的是 trait object，显式标注目标类型
        let reader: Box<dyn arrow_array::RecordBatchReader + Send> = Box::new(reader);
        self.table
            .add(reader)
            .execute()
            .await
            .map_err(|e| Error::Io(format!("写入记忆失败：{e}")))?;
        Ok(())
    }

    /// 语义召回。返回相似度不低于 `min_similarity` 的前 `limit` 条。
    pub async fn search(
        &self,
        query_vector: &[f32],
        limit: usize,
        min_similarity: f32,
        scope: &ScopeFilter,
    ) -> Result<Vec<Recalled>> {
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

        // **过滤下推给 LanceDB，不是查完再筛。** 取回 limit 条再筛的话，
        // 别的项目的事实会挤掉本项目的 —— 项目一多召回就越来越少，
        // 而且完全没有迹象说明为什么。
        let mut query = self
            .table
            .vector_search(query_vector.to_vec())
            .map_err(|e| Error::Io(format!("构造向量查询失败：{e}")))?;
        if let Some(filter) = scope.to_sql() {
            query = query.only_if(filter);
        }
        let batches: Vec<RecordBatch> = query
            .limit(limit)
            .execute()
            .await
            .map_err(|e| Error::Io(format!("向量检索失败：{e}")))?
            .try_collect()
            .await
            .map_err(|e| Error::Io(format!("读取检索结果失败：{e}")))?;

        let mut out = Vec::new();
        for batch in batches {
            out.extend(decode_batch(&batch)?);
        }

        // LanceDB 给的是 L2 距离，先转成相似度再按阈值过滤，
        // 这样配置里的阈值有直观含义
        out.retain(|r| r.similarity >= min_similarity);
        out.sort_by(|a, b| b.similarity.total_cmp(&a.similarity));
        Ok(out)
    }

    /// 事实总数
    pub async fn count(&self) -> Result<usize> {
        self.table
            .count_rows(None)
            .await
            .map_err(|e| Error::Io(format!("统计记忆条数失败：{e}")))
    }

    /// 按时间倒序浏览事实。
    ///
    /// ## 为什么"能浏览"和"能搜索"是两件事
    ///
    /// 原先只有 `search` —— 要知道它记住了什么，必须先猜一个查询词。
    /// 而**记忆系统不可见时用户无法建立信任**（见 DECISIONS 2.2）：
    /// "它是不是记错了我什么"这个问题，没有列表就永远回答不了。
    ///
    /// 倒序是因为新记的最可能是错的 —— 刚说过的话最容易被提取歪。
    pub async fn list(&self, limit: usize, offset: usize) -> Result<Vec<Fact>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        // **必须取全量再排序，不能先截断。**
        //
        // 早先这里是 `.limit(limit + offset)`。那样 `list(2, 0)` 只取 2 行就排序、
        // `list(2, 2)` 取 4 行才排序 —— **两页排的是不同的子集**，于是同一条会
        // 出现在两页里，或者某一条谁都看不到。
        //
        // LanceDB 的 `limit` 不保证返回哪几行（多个 fragment 时尤其明显），
        // 所以「先截断再排序」这个组合从根上不自洽。加次级排序键也治不了它 ——
        // 我试过，还是间歇失败。
        //
        // 取全量的成本可接受：这张表是几百到几千条的量级。但仍然封一个顶，
        // 免得将来长到十万条时这里静默变慢。
        const MAX_SCAN: usize = 10_000;
        let batches: Vec<RecordBatch> = self
            .table
            .query()
            .limit(MAX_SCAN)
            .execute()
            .await
            .map_err(|e| Error::Io(format!("读取记忆列表失败：{e}")))?
            .try_collect()
            .await
            .map_err(|e| Error::Io(format!("读取记忆列表失败：{e}")))?;

        let mut out: Vec<Fact> = Vec::new();
        for batch in batches {
            out.extend(decode_batch(&batch)?.into_iter().map(|r| r.fact));
        }
        // **在内存里排序而不是交给查询。** 这张表是几百到几千条的量级，
        // 排序成本可以忽略；而 LanceDB 的排序表达式在不同版本间不稳定，
        // 为它写一段容易在升级时静默失效的代码不值得。
        //
        // **必须是全序，所以 id 做次级键。** `created_at` 是秒级的，同一秒里
        // 写进去的几条事实 key 完全相同 —— 那时排序虽然稳定，但输入顺序来自
        // LanceDB 的扫描，两次查询之间不保证一致。后果是翻页会重叠或漏掉：
        // 第一页和第二页出现同一条，或者某一条谁都看不到。
        //
        // 实测的样子是间歇的（连跑四次挂一次），所以很容易被当成偶发而放过。
        if out.len() >= MAX_SCAN {
            tracing::warn!(
                MAX_SCAN,
                "事实数达到浏览扫描上限，翻页会看不到更早的内容"
            );
        }
        // 次级键让顺序成为全序：`created_at` 是秒级的，同一秒写进去的几条
        // key 完全相同，没有次级键时顺序取决于扫描顺序。
        out.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(out.into_iter().skip(offset).take(limit).collect())
    }

    /// 删掉一条事实。
    ///
    /// 存在的意义：**记错了要能删**。原先只有"清空全部"，粒度太粗 ——
    /// 为了删掉一条错的记忆而丢掉全部正确的，没人会真的去用。
    pub async fn delete(&self, id: &str) -> Result<()> {
        // id 是我们自己生成的 uuid，但仍然要转义 —— 拼 SQL 字符串时
        // 单引号是唯一能逃出字面量的字符，挡住它就够
        let escaped = id.replace('\'', "''");
        self.table
            .delete(&format!("id = '{escaped}'"))
            .await
            .map_err(|e| Error::Io(format!("删除记忆失败：{e}")))?;
        Ok(())
    }

    /// 清空所有长期记忆
    pub async fn clear(&self) -> Result<()> {
        self.table
            .delete("true")
            .await
            .map_err(|e| Error::Io(format!("清空记忆失败：{e}")))?;
        Ok(())
    }
}

fn fact_schema(dimensions: usize) -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::Utf8, false),
        Field::new("subject", DataType::Utf8, false),
        Field::new("fact", DataType::Utf8, false),
        Field::new("session_id", DataType::Utf8, false),
        Field::new("created_at", DataType::Int64, false),
        Field::new("scope", DataType::Utf8, false),
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

/// 有多少行的 scope 还是**路径形态**（老格式）。
///
/// 迁移前的便宜守卫：为 0 就整个跳过。不查就迁的话，每次启动都会给 LanceDB
/// 加一个数据集版本 —— 那些版本永远不会被清理。
pub async fn count_path_scoped(table: &Table) -> Result<usize> {
    // 路径一定以 / 开头；项目 id 形态是 `p:xxx`；全局是空串
    table
        .count_rows(Some("scope LIKE '/%'".to_string()))
        .await
        .map_err(|e| Error::Io(format!("统计老格式 scope 失败：{e}")))
}

/// 把某个路径 scope 原地改成项目 id scope。
///
/// **原地改列，不读写向量。** 读出来再写回是这类迁移最容易丢数据的一步 ——
/// 而 `list()` 压根不返回向量，那条路要先加一个只为迁移存在的读取接口。
pub async fn rescope(table: &Table, from: &str, to: &str) -> Result<u64> {
    let r = table
        .update()
        .only_if(format!("scope = '{}'", escape_sql(from)))
        .column("scope", format!("'{}'", escape_sql(to)))
        .execute()
        .await
        .map_err(|e| Error::Io(format!("改写 scope 失败：{e}")))?;
    Ok(r.rows_updated)
}

/// 拼 SQL 字面量时单引号是唯一能逃出去的字符。
fn escape_sql(s: &str) -> String {
    s.replace('\'', "''")
}

/// 给旧表补上 `scope` 列。
///
/// 这一列是后加的（项目隔离之前只有一个全局池），已有的库里没有它。
/// 不补的话 `add()` 会因为批次多一列而失败 —— 表现为**从此再也记不住任何事**，
/// 而提取是后台异步跑的，失败只进日志，用户看不到任何提示。
///
/// 用 SQL 常量表达式而不是重建表：重建要先把向量读出来再写回去，
/// 而那正是最容易丢数据的一步；何况已有的行本来就都该是全局（`''`）。
async fn migrate_add_scope(table: &Table) -> Result<()> {
    let schema = table
        .schema()
        .await
        .map_err(|e| Error::Io(format!("读取表 schema 失败：{e}")))?;
    if schema.field_with_name("scope").is_ok() {
        return Ok(());
    }
    table
        .add_columns(
            NewColumnTransform::SqlExpressions(vec![("scope".to_string(), "''".to_string())]),
            None,
        )
        .await
        .map_err(|e| Error::Io(format!("为记忆库补 scope 列失败：{e}")))?;
    tracing::info!("记忆库已补上 scope 列，已有事实归为全局");
    Ok(())
}

/// 读出已有表的向量维度
async fn table_dimensions(table: &Table) -> Result<usize> {
    let schema = table
        .schema()
        .await
        .map_err(|e| Error::Io(format!("读取表 schema 失败：{e}")))?;
    let field = schema
        .field_with_name("vector")
        .map_err(|e| Error::decode(format!("记忆库缺少 vector 字段：{e}")))?;

    match field.data_type() {
        DataType::FixedSizeList(_, dim) => Ok(*dim as usize),
        other => Err(Error::decode(format!("vector 字段类型异常：{other:?}"))),
    }
}

/// 把 Arrow 批次解回 `Recalled`
fn decode_batch(batch: &RecordBatch) -> Result<Vec<Recalled>> {
    let strings = |name: &str| -> Result<StringArray> {
        batch
            .column_by_name(name)
            .and_then(|c| c.as_any().downcast_ref::<StringArray>().cloned())
            .ok_or_else(|| Error::decode(format!("检索结果缺少字符串字段 {name}")))
    };

    let ids = strings("id")?;
    let subjects = strings("subject")?;
    let facts = strings("fact")?;
    let sessions = strings("session_id")?;
    // scope 用 optional 取：迁移是在 open() 里做的，但只读路径
    // （例如别处直接打开同一个库）不该因为缺一列就整个解不出来
    let scopes = batch
        .column_by_name("scope")
        .and_then(|c| c.as_any().downcast_ref::<StringArray>().cloned());

    let created = batch
        .column_by_name("created_at")
        .and_then(|c| c.as_any().downcast_ref::<Int64Array>().cloned())
        .ok_or_else(|| Error::decode("created_at 不是 int64"))?;

    // LanceDB 把距离放在保留列 _distance。取不到就当作完全匹配 ——
    // 那说明这次不是向量检索（例如全表扫描），距离没有意义。
    let distances = batch
        .column_by_name("_distance")
        .and_then(|c| c.as_any().downcast_ref::<Float32Array>().cloned());

    let mut out = Vec::with_capacity(batch.num_rows());
    for i in 0..batch.num_rows() {
        let similarity = distances
            .as_ref()
            .map(|d| l2_distance_to_similarity(d.value(i)))
            .unwrap_or(1.0);

        out.push(Recalled {
            fact: Fact {
                id: ids.value(i).to_string(),
                subject: subjects.value(i).to_string(),
                fact: facts.value(i).to_string(),
                session_id: sessions.value(i).to_string(),
                created_at: created.value(i),
                // 取不到这一列说明是迁移之前写的行 —— 当成全局，
                // 那也正是它当时的语义（那时还没有项目这个概念）
                scope: scopes
                    .as_ref()
                    .map(|s| s.value(i).to_string())
                    .unwrap_or_default(),
            },
            similarity,
            // 排序分由策略层（`Memory::recall`）算，存储层不掺和
            score: similarity,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("anima-vec-{}", uuid::Uuid::new_v4()))
    }

    fn fact(id: &str, text: &str) -> Fact {
        Fact {
            id: id.into(),
            subject: "用户".into(),
            fact: text.into(),
            session_id: "s1".into(),
            created_at: 1_700_000_000,
            scope: String::new(),
        }
    }

    /// 造一个指定方向的单位向量，方便构造「相近」与「无关」
    fn unit(dim: usize, hot: usize) -> Vec<f32> {
        let mut v = vec![0.0; dim];
        v[hot % dim] = 1.0;
        v
    }

    #[tokio::test]
    async fn stores_and_recalls_by_similarity() {
        let dir = temp_dir();
        let store = VectorStore::open(&dir, 8).await.expect("建库失败");

        store
            .add(
                &[
                    fact("1", "养了一只叫小黑的猫"),
                    fact("2", "在做一个语音助手"),
                ],
                &[unit(8, 0), unit(8, 3)],
            )
            .await
            .expect("写入失败");

        assert_eq!(store.count().await.unwrap(), 2);

        // 用与第一条同方向的向量查询，第一条应当排最前
        let hits = store.search(&unit(8, 0), 2, -1.0, &ScopeFilter::Any).await.expect("检索失败");
        assert!(!hits.is_empty());
        assert_eq!(hits[0].fact.id, "1");
        assert!(
            hits[0].similarity > 0.9,
            "同方向向量相似度应当接近 1，实际 {}",
            hits[0].similarity
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn similarity_threshold_filters_unrelated_facts() {
        let dir = temp_dir();
        let store = VectorStore::open(&dir, 8).await.unwrap();
        store
            .add(&[fact("1", "无关的事实")], &[unit(8, 0)])
            .await
            .unwrap();

        // 正交向量查询：相似度约 0，0.5 的阈值应当挡掉
        let hits = store.search(&unit(8, 4), 5, 0.5, &ScopeFilter::Any).await.unwrap();
        assert!(hits.is_empty(), "正交结果不该通过 0.5 阈值：{hits:?}");

        // 阈值放开就能取到
        let hits = store.search(&unit(8, 4), 5, -1.0, &ScopeFilter::Any).await.unwrap();
        assert_eq!(hits.len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn rejects_dimension_mismatch_on_write_and_query() {
        let dir = temp_dir();
        let store = VectorStore::open(&dir, 8).await.unwrap();

        let err = store
            .add(&[fact("1", "x")], &[vec![0.0; 4]])
            .await
            .expect_err("维度不符应当报错");
        assert!(err.to_string().contains("维度"), "实际：{err}");

        let err = store
            .search(&[0.0; 4], 1, 0.0, &ScopeFilter::Any)
            .await
            .expect_err("查询维度不符应当报错");
        assert!(err.to_string().contains("维度"), "实际：{err}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn reopening_with_different_dimensions_fails_loudly() {
        let dir = temp_dir();
        {
            let store = VectorStore::open(&dir, 8).await.unwrap();
            store.add(&[fact("1", "x")], &[unit(8, 0)]).await.unwrap();
        }

        // 换了嵌入模型（维度变了）—— 必须明确报错而不是静默写坏
        // 不用 expect_err：VectorStore 内含不可 Debug 的 LanceDB 句柄
        let msg = match VectorStore::open(&dir, 16).await {
            Ok(_) => panic!("维度变化应当报错，实际打开成功了"),
            Err(err) => err.to_string(),
        };
        assert!(msg.contains("维度"), "实际：{msg}");
        assert!(msg.contains("重建"), "报错应当告诉用户怎么办：{msg}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn data_survives_reopen() {
        let dir = temp_dir();
        {
            let store = VectorStore::open(&dir, 8).await.unwrap();
            store
                .add(&[fact("1", "重开后还在")], &[unit(8, 0)])
                .await
                .unwrap();
        }

        let store = VectorStore::open(&dir, 8).await.unwrap();
        assert_eq!(store.count().await.unwrap(), 1);
        let hits = store.search(&unit(8, 0), 1, -1.0, &ScopeFilter::Any).await.unwrap();
        assert_eq!(hits[0].fact.fact, "重开后还在");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn clear_removes_everything() {
        let dir = temp_dir();
        let store = VectorStore::open(&dir, 8).await.unwrap();
        store
            .add(&[fact("1", "a"), fact("2", "b")], &[unit(8, 0), unit(8, 1)])
            .await
            .unwrap();

        store.clear().await.unwrap();
        assert_eq!(store.count().await.unwrap(), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn empty_add_and_zero_limit_are_no_ops() {
        let dir = temp_dir();
        let store = VectorStore::open(&dir, 8).await.unwrap();
        store.add(&[], &[]).await.expect("空写入应当无害");
        assert_eq!(store.count().await.unwrap(), 0);
        assert!(store.search(&unit(8, 0), 0, 0.0, &ScopeFilter::Any).await.unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scope_过滤的sql是全局加本项目而不是只有本项目() {
        // 关于用户本人的事（职业、称呼、偏好）在任何项目里都该被召回；
        // 只留本项目会让"它怎么突然不认识我了"发生在每个新项目里
        assert_eq!(ScopeFilter::Any.to_sql(), None);
        assert_eq!(
            ScopeFilter::GlobalOnly.to_sql().unwrap(),
            "scope = ''"
        );
        assert_eq!(
            ScopeFilter::Project("/a/b".into()).to_sql().unwrap(),
            "(scope = '' OR scope = '/a/b')"
        );
        // 路径里的单引号会逃出 SQL 字面量
        assert_eq!(
            ScopeFilter::Project("/a/o'b".into()).to_sql().unwrap(),
            "(scope = '' OR scope = '/a/o''b')"
        );
    }

    #[tokio::test]
    async fn 项目事实不会漏到别的项目也不会漏到闲聊() {
        let dir = temp_dir();
        let store = VectorStore::open(&dir, 8).await.unwrap();

        let mk = |id: &str, text: &str, scope: &str| Fact {
            id: id.into(),
            subject: "用户".into(),
            fact: text.into(),
            session_id: "s".into(),
            created_at: 1,
            scope: scope.into(),
        };
        // 三条事实用同一个方向的向量，所以命中与否**只由 scope 决定**
        let v = unit(8, 1);
        store
            .add(
                &[
                    mk("g", "是软件工程师", ""),
                    mk("a", "A 项目用 pnpm", "/proj/a"),
                    mk("b", "B 项目用 yarn", "/proj/b"),
                ],
                &[v.clone(), v.clone(), v.clone()],
            )
            .await
            .unwrap();

        let ids = |hits: Vec<Recalled>| {
            let mut v: Vec<String> = hits.into_iter().map(|h| h.fact.id).collect();
            v.sort();
            v
        };

        // 在 A 项目里：全局 + A，**没有 B**
        let in_a = store
            .search(&v, 10, -1.0, &ScopeFilter::Project("/proj/a".into()))
            .await
            .unwrap();
        assert_eq!(ids(in_a), vec!["a".to_string(), "g".to_string()]);

        // 闲聊（不在项目里）：只有全局。否则会在聊天里被告知"A 项目用 pnpm"
        let chat = store.search(&v, 10, -1.0, &ScopeFilter::GlobalOnly).await.unwrap();
        assert_eq!(ids(chat), vec!["g".to_string()]);

        // 浏览界面：全部可见
        let all = store.search(&v, 10, -1.0, &ScopeFilter::Any).await.unwrap();
        assert_eq!(all.len(), 3);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn 没有scope列的旧库会被迁移而不是写入失败() {
        // 这是最危险的一条路径：不迁移的话 add() 因为批次多一列而失败，
        // 而提取是后台异步跑的、失败只进日志 —— 表现为**从此再也记不住任何事**，
        // 且没有任何用户可见的迹象。
        let dir = temp_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let uri = dir.to_string_lossy().to_string();
        let conn = connect(&uri).execute().await.unwrap();

        // 手工建一张 scope 之前的表（照抄旧 schema）
        let legacy = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Utf8, false),
            Field::new("subject", DataType::Utf8, false),
            Field::new("fact", DataType::Utf8, false),
            Field::new("session_id", DataType::Utf8, false),
            Field::new("created_at", DataType::Int64, false),
            Field::new(
                "vector",
                DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), 8),
                true,
            ),
        ]));
        let batch = RecordBatch::try_new(
            Arc::clone(&legacy),
            vec![
                Arc::new(StringArray::from(vec!["old"])),
                Arc::new(StringArray::from(vec!["用户"])),
                Arc::new(StringArray::from(vec!["我們有西瓜汁，很好喝哦"])),
                Arc::new(StringArray::from(vec!["s0"])),
                Arc::new(Int64Array::from(vec![100i64])),
                Arc::new(FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
                    vec![Some(unit(8, 2).into_iter().map(Some).collect::<Vec<_>>())],
                    8,
                )),
            ],
        )
        .unwrap();
        let reader = RecordBatchIterator::new(vec![Ok(batch)], legacy);
        let reader: Box<dyn arrow_array::RecordBatchReader + Send> = Box::new(reader);
        conn.create_table(TABLE_NAME, reader).execute().await.unwrap();
        drop(conn);

        // open() 要把 scope 补上
        let store = VectorStore::open(&dir, 8).await.unwrap();
        assert_eq!(store.count().await.unwrap(), 1, "旧数据不能在迁移中丢掉");

        // 旧行归为全局 —— 那正是它当时的语义
        let hits = store
            .search(&unit(8, 2), 5, -1.0, &ScopeFilter::GlobalOnly)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1, "迁移后的旧行应当作为全局事实可见");
        assert_eq!(hits[0].fact.scope, "");

        // 而且**现在能写入了**：这才是迁移真正要保住的东西
        store
            .add(
                &[Fact {
                    id: "new".into(),
                    subject: "用户".into(),
                    fact: "A 项目用 pnpm".into(),
                    session_id: "s1".into(),
                    created_at: 200,
                    scope: "/proj/a".into(),
                }],
                &[unit(8, 3)],
            )
            .await
            .expect("迁移之后必须能写入带 scope 的新事实");
        assert_eq!(store.count().await.unwrap(), 2);

        // 重复 open 不该重复迁移（add_columns 对已存在的列会报错）
        let again = VectorStore::open(&dir, 8).await.expect("二次打开不该失败");
        assert_eq!(again.count().await.unwrap(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn 同一秒写入的事实翻页不重叠也不漏() {
        // `created_at` 是秒级的：同一秒写进去的几条 key 完全相同，
        // 而输入顺序来自 LanceDB 的扫描、两次查询之间不保证一致。
        // 没有次级键的话翻页会重叠或漏掉，而且是**间歇性**的
        // （实测连跑四次挂一次），很容易被当成偶发放过。
        let dir = temp_dir();
        let store = VectorStore::open(&dir, 8).await.unwrap();

        let same_second = 1_700_000_000;
        let facts: Vec<Fact> = (0..5)
            .map(|i| Fact {
                id: format!("id-{i}"),
                subject: "用户".into(),
                fact: format!("第 {i} 条"),
                session_id: "s".into(),
                created_at: same_second,
                scope: String::new(),
            })
            .collect();
        // **一条一条写。** 一次批量 add 只产生一个 fragment，扫描顺序恰好稳定，
        // 于是这个 bug 复现不出来 —— 我第一版测试就是这么写的，它通过了，
        // 而联调测试仍然间歇失败。多次 add 才是真实形状。
        for (f, v) in facts.iter().zip((0..5).map(|i| unit(8, i))) {
            store.add(std::slice::from_ref(f), &[v]).await.unwrap();
        }

        // 分三页取完，每页之间不该有重复，合起来要正好是五条
        let mut seen: Vec<String> = Vec::new();
        for offset in [0, 2, 4] {
            let page = store.list(2, offset).await.unwrap();
            for f in page {
                assert!(!seen.contains(&f.id), "第 {offset} 页出现了重复：{}", f.id);
                seen.push(f.id);
            }
        }
        seen.sort();
        assert_eq!(
            seen,
            vec!["id-0", "id-1", "id-2", "id-3", "id-4"],
            "翻页漏掉了或多出了条目"
        );

        // 同一次查询重复调也要给出同一个顺序
        let a = store.list(5, 0).await.unwrap();
        let b = store.list(5, 0).await.unwrap();
        assert_eq!(
            a.iter().map(|f| &f.id).collect::<Vec<_>>(),
            b.iter().map(|f| &f.id).collect::<Vec<_>>(),
            "同样的查询给出了不同顺序"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn 路径_scope_原地改成项目id且向量不受损() {
        let dir = temp_dir();
        let store = VectorStore::open(&dir, 8).await.unwrap();

        let mk = |id: &str, scope: &str| Fact {
            id: id.into(),
            subject: "用户".into(),
            fact: format!("事实-{id}"),
            session_id: "s".into(),
            created_at: 1,
            scope: scope.into(),
        };
        // 老格式（路径）两条、全局一条、已经是新格式的一条
        let v_old = unit(8, 1);
        store
            .add(
                &[
                    mk("a", "/Users/x/proj"),
                    mk("b", "/Users/x/proj"),
                    mk("g", ""),
                    mk("n", "p:already"),
                ],
                &[v_old.clone(), v_old.clone(), unit(8, 2), unit(8, 3)],
            )
            .await
            .unwrap();

        // 只数路径形态的：全局和 p: 都不该算进来
        assert_eq!(count_path_scoped(store.table()).await.unwrap(), 2);

        let n = rescope(store.table(), "/Users/x/proj", "p:123").await.unwrap();
        assert_eq!(n, 2, "两行都该被改写");
        assert_eq!(
            count_path_scoped(store.table()).await.unwrap(),
            0,
            "改写之后不该再有路径形态的"
        );

        // **向量必须完好。** 原地改列如果碰坏了向量，那两条会从此召不回来，
        // 而这不会报任何错 —— 只是搜不到。
        let hits = store
            .search(&v_old, 10, -1.0, &ScopeFilter::Project("p:123".into()))
            .await
            .unwrap();
        let mut ids: Vec<String> = hits.iter().map(|h| h.fact.id.clone()).collect();
        ids.sort();
        // 全局那条也在（Project 过滤 = 全局 + 本项目）
        assert_eq!(ids, vec!["a", "b", "g"], "{hits:#?}");
        // 相似度还在最高档，说明向量没被动过
        let a = hits.iter().find(|h| h.fact.id == "a").unwrap();
        assert!(a.similarity > 0.99, "向量被改坏了：{}", a.similarity);
        assert_eq!(a.fact.scope, "p:123");
        // 别的项目的没被误伤
        assert_eq!(store.count().await.unwrap(), 4);

        // 幂等：再跑一次改 0 行
        assert_eq!(
            rescope(store.table(), "/Users/x/proj", "p:123").await.unwrap(),
            0
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn 迁移不会误伤同前缀的别的项目() {
        // `/Users/x/proj` 和 `/Users/x/proj-old` 是两个项目，
        // 用 `LIKE` 或前缀匹配就会连带改掉后者
        let dir = temp_dir();
        let store = VectorStore::open(&dir, 8).await.unwrap();
        let mk = |id: &str, scope: &str| Fact {
            id: id.into(),
            subject: "u".into(),
            fact: id.into(),
            session_id: "s".into(),
            created_at: 1,
            scope: scope.into(),
        };
        store
            .add(
                &[mk("in", "/Users/x/proj"), mk("out", "/Users/x/proj-old")],
                &[unit(8, 1), unit(8, 2)],
            )
            .await
            .unwrap();
        assert_eq!(rescope(store.table(), "/Users/x/proj", "p:1").await.unwrap(), 1);
        let all = store.list(10, 0).await.unwrap();
        let out = all.iter().find(|f| f.id == "out").unwrap();
        assert_eq!(out.scope, "/Users/x/proj-old", "同前缀的项目被误改了");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
