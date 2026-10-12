//! 文本向量化。
//!
//! 复用已经配好的本地后端，不再引一套嵌入运行时：
//!   - OpenAI 兼容（LM Studio / OpenAI / Azure）：`POST /v1/embeddings`
//!   - Ollama 原生：`POST /api/embed`
//!
//! 两个端点在本机实测都返回 1024 维（`mxbai-embed-large`），且同一文本的向量一致。

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;

use crate::config::{secrets, ProviderConfig, ProviderKind};
use crate::error::{Error, Result};

#[async_trait]
pub trait Embedder: Send + Sync {
    /// 批量向量化。返回顺序与输入一致。
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;

    /// 向量维度。建库时要用，所以必须能在写入前拿到。
    async fn dimensions(&self) -> Result<usize>;
}

pub struct HttpEmbedder {
    provider: ProviderConfig,
    model: String,
    http: reqwest::Client,
}

impl HttpEmbedder {
    pub fn new(provider: ProviderConfig, model: String, http: reqwest::Client) -> Self {
        Self {
            provider,
            model,
            http,
        }
    }

    fn api_key(&self) -> Result<Option<String>> {
        Ok(secrets::get_secret(&self.provider.id)?.filter(|k| !k.is_empty()))
    }
}

#[derive(Deserialize)]
struct OpenAiEmbeddings {
    #[serde(default)]
    data: Vec<OpenAiEmbeddingItem>,
}

#[derive(Deserialize)]
struct OpenAiEmbeddingItem {
    #[serde(default)]
    index: usize,
    #[serde(default)]
    embedding: Vec<f32>,
}

#[derive(Deserialize)]
struct OllamaEmbeddings {
    /// 新版字段（/api/embed，支持批量）
    #[serde(default)]
    embeddings: Vec<Vec<f32>>,
    /// 旧版字段（/api/embeddings，单条）
    #[serde(default)]
    embedding: Vec<f32>,
}

#[async_trait]
impl Embedder for HttpEmbedder {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let base = self.provider.base_url.trim_end_matches('/');

        match self.provider.kind {
            ProviderKind::Ollama => {
                let resp = self
                    .http
                    .post(format!("{base}/api/embed"))
                    .json(&json!({ "model": self.model, "input": texts }))
                    .send()
                    .await?;
                let resp = crate::llm::sse::ensure_success(resp).await?;
                let parsed: OllamaEmbeddings = resp.json().await?;

                // 兼容新旧两种字段形态
                let vectors = if !parsed.embeddings.is_empty() {
                    parsed.embeddings
                } else if !parsed.embedding.is_empty() {
                    vec![parsed.embedding]
                } else {
                    return Err(Error::decode("Ollama 未返回任何向量"));
                };

                if vectors.len() != texts.len() {
                    return Err(Error::decode(format!(
                        "向量数量与输入不符：期望 {}，实际 {}",
                        texts.len(),
                        vectors.len()
                    )));
                }
                Ok(vectors)
            }

            // OpenAI 兼容：LM Studio / OpenAI / Azure 都走这条
            _ => {
                let mut request = self.http.post(format!("{base}/v1/embeddings"));
                // Azure 的部署名在 URL 里、鉴权用 api-key 头，与其它不同
                if self.provider.kind == ProviderKind::AzureOpenAi {
                    let version = self
                        .provider
                        .api_version
                        .clone()
                        .unwrap_or_else(|| crate::llm::azure::DEFAULT_API_VERSION.to_string());
                    request = self.http.post(format!(
                        "{base}/openai/deployments/{}/embeddings?api-version={version}",
                        self.model
                    ));
                    if let Some(key) = self.api_key()? {
                        request = request.header("api-key", key);
                    }
                } else if let Some(key) = self.api_key()? {
                    request = request.bearer_auth(key);
                }

                let resp = request
                    .json(&json!({ "model": self.model, "input": texts }))
                    .send()
                    .await?;
                let resp = crate::llm::sse::ensure_success(resp).await?;
                let mut parsed: OpenAiEmbeddings = resp.json().await?;

                if parsed.data.len() != texts.len() {
                    return Err(Error::decode(format!(
                        "向量数量与输入不符：期望 {}，实际 {}",
                        texts.len(),
                        parsed.data.len()
                    )));
                }

                // 规范允许乱序返回，必须按 index 排回来，
                // 否则事实和向量会错位 —— 这种错误静默且极难发现
                parsed.data.sort_by_key(|d| d.index);
                Ok(parsed.data.into_iter().map(|d| d.embedding).collect())
            }
        }
    }

    async fn dimensions(&self) -> Result<usize> {
        // 没有专门的「查维度」接口，拿一个短文本探一次
        let probe = self.embed(&["维度探测".to_string()]).await?;
        probe
            .first()
            .map(|v| v.len())
            .filter(|d| *d > 0)
            .ok_or_else(|| Error::decode("无法确定向量维度"))
    }
}

/// 余弦相似度。两个向量都应当非零且等长。
///
/// 召回结果的排序依据。单独抽出来是因为它同时被 LanceDB 之外的
/// 阈值过滤逻辑使用，且是纯函数、值得单测。
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for (x, y) in a.iter().zip(b.iter()) {
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 {
        0.0
    } else {
        dot / denom
    }
}

/// LanceDB 返回的是 L2 距离，转成相似度好让阈值有直观含义。
///
/// 对**归一化后**的向量，L2 距离平方与余弦相似度满足
/// `d² = 2(1 - cos)`，故 `cos = 1 - d²/2`。嵌入模型的输出通常已归一化。
pub fn l2_distance_to_similarity(distance: f32) -> f32 {
    (1.0 - distance * distance / 2.0).clamp(-1.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_vectors_have_similarity_one() {
        let v = vec![0.1, 0.2, 0.3];
        assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn orthogonal_vectors_have_zero_similarity() {
        assert!(cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-6);
    }

    #[test]
    fn opposite_vectors_have_negative_similarity() {
        assert!((cosine_similarity(&[1.0, 0.0], &[-1.0, 0.0]) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn mismatched_or_empty_vectors_are_not_similar() {
        assert_eq!(cosine_similarity(&[1.0, 2.0], &[1.0]), 0.0);
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
        // 全零向量不该产生 NaN
        assert_eq!(cosine_similarity(&[0.0, 0.0], &[0.0, 0.0]), 0.0);
    }

    #[test]
    fn magnitude_does_not_affect_cosine() {
        // 余弦只看方向：放大十倍相似度不变
        let a = [1.0, 2.0, 3.0];
        let b = [10.0, 20.0, 30.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn zero_distance_maps_to_full_similarity() {
        assert!((l2_distance_to_similarity(0.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn distance_of_sqrt_two_maps_to_orthogonal() {
        // 归一化向量正交时 L2 距离为 √2
        let d = 2.0f32.sqrt();
        assert!(l2_distance_to_similarity(d).abs() < 1e-5);
    }

    #[test]
    fn similarity_is_clamped_to_valid_range() {
        // 距离异常大时不该返回超出 [-1,1] 的值
        assert_eq!(l2_distance_to_similarity(100.0), -1.0);
    }
}
