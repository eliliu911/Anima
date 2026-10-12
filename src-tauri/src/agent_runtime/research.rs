//! Research task model.
//!
//! Web search is an explicit, resumable tool/job step: record why search
//! happened, what was queried, what evidence was accepted, then return to the
//! original task with a compact evidence bundle.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResearchTrigger {
    UserRequested,
    LatestInformationNeeded,
    MissingLocalKnowledge,
    DependencyResolution,
    FailureDiagnosis,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchQuery {
    pub id: String,
    pub trigger: ResearchTrigger,
    pub query: String,
    pub rationale: String,
}

impl ResearchQuery {
    pub fn new(
        trigger: ResearchTrigger,
        query: impl Into<String>,
        rationale: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            trigger,
            query: query.into(),
            rationale: rationale.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceItem {
    pub title: String,
    pub url: String,
    pub summary: String,
    pub source_type: String,
    pub retrieved_at_ms: u128,
}

impl EvidenceItem {
    pub fn new(
        title: impl Into<String>,
        url: impl Into<String>,
        summary: impl Into<String>,
        source_type: impl Into<String>,
    ) -> Self {
        Self {
            title: title.into(),
            url: url.into(),
            summary: summary.into(),
            source_type: source_type.into(),
            retrieved_at_ms: now_ms(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceBundle {
    pub query: ResearchQuery,
    pub items: Vec<EvidenceItem>,
    pub conclusion: String,
}

impl EvidenceBundle {
    pub fn new(query: ResearchQuery) -> Self {
        Self {
            query,
            items: Vec::new(),
            conclusion: String::new(),
        }
    }

    pub fn with_item(mut self, item: EvidenceItem) -> Self {
        self.items.push(item);
        self
    }

    pub fn with_conclusion(mut self, conclusion: impl Into<String>) -> Self {
        self.conclusion = conclusion.into();
        self
    }

    pub fn is_actionable(&self) -> bool {
        !self.items.is_empty() && !self.conclusion.trim().is_empty()
    }
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evidence_bundle_requires_sources_and_conclusion() {
        let query = ResearchQuery::new(
            ResearchTrigger::LatestInformationNeeded,
            "ComfyUI Wan 2.2 workflow",
            "模型工作流会变化，需要查最新来源",
        );
        let empty = EvidenceBundle::new(query.clone());
        assert!(!empty.is_actionable());

        let bundle = EvidenceBundle::new(query)
            .with_item(EvidenceItem::new(
                "doc",
                "https://example.com",
                "summary",
                "docs",
            ))
            .with_conclusion("可以作为候选。");
        assert!(bundle.is_actionable());
    }
}
