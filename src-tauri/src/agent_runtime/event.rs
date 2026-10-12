//! Runtime event log.
//!
//! UI progress, copyable logs, resume, and debugging should all read from this
//! append-only shape instead of scraping ad-hoc strings.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use super::approval::ApprovalRequest;
use super::job::JobStatus;
use super::tool::ToolOutcome;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuntimeEvent {
    SessionStarted {
        session_id: String,
    },
    TurnStarted {
        session_id: String,
        turn_id: String,
    },
    TurnFinished {
        session_id: String,
        turn_id: String,
    },
    ToolStarted {
        turn_id: String,
        call_id: String,
        tool_name: String,
    },
    ToolFinished {
        turn_id: String,
        outcome: ToolOutcome,
    },
    ImageGenerationProgress {
        turn_id: String,
        call_id: String,
        prompt: String,
        width: u32,
        height: u32,
        /// 这一张用的是哪个底模（`Z-Image Turbo` / `Chroma1-HD` / `SDXL`）。
        ///
        /// 骨架屏要显示它 —— 三个引擎的耗时差到 8.5 倍（45s / 331s / 39s），
        /// 不告诉用户在用哪个，等 5 分钟和等 45 秒看起来是一样的。
        engine: String,
        stage: String,
        label: String,
        progress: f32,
        prompt_id: Option<String>,
        output_path: Option<String>,
        message: Option<String>,
    },
    VideoGenerationProgress {
        turn_id: String,
        call_id: String,
        prompt: String,
        duration_secs: u32,
        stage: String,
        label: String,
        progress: f32,
        prompt_id: Option<String>,
        output_path: Option<String>,
        message: Option<String>,
    },
    ApprovalRequested {
        turn_id: String,
        request: ApprovalRequest,
    },
    JobCreated {
        job_id: String,
        kind: String,
        title: String,
    },
    JobStatusChanged {
        job_id: String,
        status: JobStatus,
    },
    JobStepChanged {
        job_id: String,
        step_id: String,
        status: String,
        message: Option<String>,
    },
    Log {
        target: String,
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventLog {
    capacity: usize,
    events: VecDeque<RuntimeEvent>,
}

impl EventLog {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            events: VecDeque::new(),
        }
    }

    pub fn push(&mut self, event: RuntimeEvent) {
        if self.events.len() == self.capacity {
            self.events.pop_front();
        }
        self.events.push_back(event);
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn recent(&self) -> Vec<RuntimeEvent> {
        self.events.iter().cloned().collect()
    }
}

impl Default for EventLog {
    fn default() -> Self {
        Self::new(500)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_log_keeps_recent_events_only() {
        let mut log = EventLog::new(2);
        log.push(RuntimeEvent::Log {
            target: "a".into(),
            message: "1".into(),
        });
        log.push(RuntimeEvent::Log {
            target: "a".into(),
            message: "2".into(),
        });
        log.push(RuntimeEvent::Log {
            target: "a".into(),
            message: "3".into(),
        });

        assert_eq!(log.len(), 2);
        let events = log.recent();
        assert!(matches!(
            &events[0],
            RuntimeEvent::Log { message, .. } if message == "2"
        ));
    }
}
