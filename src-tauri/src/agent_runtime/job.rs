//! Durable job model for long-running Anima pipelines.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use super::tool::RetryPolicy;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    ScriptPlanning,
    CharacterAssets,
    Storyboard,
    ShotVideo,
    AudioGeneration,
    EpisodeAssembly,
    ExportJianying,
    Generic,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Pending,
    Running,
    Paused,
    WaitingApproval,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum JobStepAction {
    RuntimeTool {
        tool_name: String,
        #[serde(default)]
        arguments: Value,
        #[serde(default)]
        network_enabled: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Artifact {
    Text {
        label: String,
        content: String,
    },
    File {
        label: String,
        path: String,
    },
    Url {
        label: String,
        url: String,
    },
    Json {
        label: String,
        value: serde_json::Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobStep {
    pub id: String,
    pub title: String,
    pub status: StepStatus,
    pub attempts: u32,
    #[serde(default)]
    pub retry_policy: RetryPolicy,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub action: Option<JobStepAction>,
    pub message: Option<String>,
    #[serde(default)]
    pub provenance: Vec<Artifact>,
    pub artifacts: Vec<Artifact>,
}

impl JobStep {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            title: title.into(),
            status: StepStatus::Pending,
            attempts: 0,
            retry_policy: RetryPolicy::default(),
            depends_on: Vec::new(),
            action: None,
            message: None,
            provenance: Vec::new(),
            artifacts: Vec::new(),
        }
    }

}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub kind: JobKind,
    pub title: String,
    pub status: JobStatus,
    pub steps: Vec<JobStep>,
    pub artifacts: Vec<Artifact>,
    #[serde(default)]
    pub provenance: Vec<Artifact>,
    pub error: Option<String>,
    pub created_at_ms: u128,
    pub updated_at_ms: u128,
}

impl Job {
    pub fn new(kind: JobKind, title: impl Into<String>, steps: Vec<JobStep>) -> Self {
        let now = now_ms();
        Self {
            id: Uuid::new_v4().to_string(),
            kind,
            title: title.into(),
            status: JobStatus::Pending,
            steps,
            artifacts: Vec::new(),
            provenance: Vec::new(),
            error: None,
            created_at_ms: now,
            updated_at_ms: now,
        }
    }

    pub fn start(&mut self) {
        self.status = JobStatus::Running;
        self.touch();
    }

    pub fn mark_waiting_approval(&mut self) {
        self.status = JobStatus::WaitingApproval;
        self.touch();
    }

    pub fn pause(&mut self, reason: impl Into<String>) {
        self.status = JobStatus::Paused;
        let reason = reason.into();
        if !reason.trim().is_empty() {
            self.error = Some(reason);
        }
        self.touch();
    }

    pub fn resume(&mut self) {
        self.status = JobStatus::Running;
        self.error = None;
        self.touch();
    }

    pub fn start_step(&mut self, step_id: &str) -> bool {
        if !self.dependencies_satisfied(step_id) {
            return false;
        }
        let Some(step) = self.step_mut(step_id) else {
            return false;
        };
        step.status = StepStatus::Running;
        step.attempts = step.attempts.saturating_add(1);
        self.status = JobStatus::Running;
        self.touch();
        true
    }

    pub fn complete_step(&mut self, step_id: &str, artifacts: Vec<Artifact>) -> bool {
        let Some(step) = self.step_mut(step_id) else {
            return false;
        };
        step.status = StepStatus::Succeeded;
        step.artifacts.extend(artifacts);
        self.refresh_terminal_status();
        true
    }

    pub fn skip_step(&mut self, step_id: &str, message: Option<String>) -> bool {
        let Some(step) = self.step_mut(step_id) else {
            return false;
        };
        step.status = StepStatus::Skipped;
        step.message = message;
        self.refresh_terminal_status();
        true
    }

    pub fn set_step_action(&mut self, step_id: &str, action: Option<JobStepAction>) -> bool {
        let Some(step) = self.step_mut(step_id) else {
            return false;
        };
        step.action = action;
        self.touch();
        true
    }

    pub fn fail_step(&mut self, step_id: &str, message: impl Into<String>) -> bool {
        let Some(step) = self.step_mut(step_id) else {
            return false;
        };
        let message = message.into();
        step.status = StepStatus::Failed;
        step.message = Some(message.clone());
        self.status = JobStatus::Failed;
        self.error = Some(message);
        self.touch();
        true
    }

    pub fn cancel(&mut self, reason: impl Into<String>) {
        self.status = JobStatus::Cancelled;
        self.error = Some(reason.into());
        self.touch();
    }

    pub fn progress(&self) -> f32 {
        if self.steps.is_empty() {
            return match self.status {
                JobStatus::Succeeded => 1.0,
                _ => 0.0,
            };
        }
        let done = self
            .steps
            .iter()
            .filter(|step| matches!(step.status, StepStatus::Succeeded | StepStatus::Skipped))
            .count();
        done as f32 / self.steps.len() as f32
    }

    fn step_mut(&mut self, step_id: &str) -> Option<&mut JobStep> {
        self.steps.iter_mut().find(|step| step.id == step_id)
    }

    fn dependencies_satisfied(&self, step_id: &str) -> bool {
        let Some(step) = self.steps.iter().find(|step| step.id == step_id) else {
            return false;
        };
        step.depends_on.iter().all(|dep| {
            self.steps
                .iter()
                .any(|candidate| candidate.id == *dep && candidate.status == StepStatus::Succeeded)
        })
    }

    fn refresh_terminal_status(&mut self) {
        if self
            .steps
            .iter()
            .all(|step| matches!(step.status, StepStatus::Succeeded | StepStatus::Skipped))
        {
            self.status = JobStatus::Succeeded;
        }
        self.touch();
    }

    fn touch(&mut self) {
        self.updated_at_ms = now_ms();
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
    fn job_progress_tracks_completed_steps() {
        let mut job = Job::new(
            JobKind::CharacterAssets,
            "角色资产",
            vec![JobStep::new("三视图"), JobStep::new("表情模组")],
        );
        let first = job.steps[0].id.clone();

        job.start();
        assert_eq!(job.status, JobStatus::Running);
        assert!(job.start_step(&first));
        assert!(job.complete_step(&first, Vec::new()));

        assert_eq!(job.progress(), 0.5);
        assert_eq!(job.status, JobStatus::Running);
    }

    #[test]
    fn job_succeeds_when_all_steps_succeed() {
        let mut job = Job::new(
            JobKind::ShotVideo,
            "镜头视频",
            vec![JobStep::new("生成视频")],
        );
        let step = job.steps[0].id.clone();
        job.start_step(&step);
        job.complete_step(&step, Vec::new());
        assert_eq!(job.status, JobStatus::Succeeded);
        assert_eq!(job.progress(), 1.0);
    }

    #[test]
    fn failed_step_fails_job() {
        let mut job = Job::new(JobKind::AudioGeneration, "音频", vec![JobStep::new("TTS")]);
        let step = job.steps[0].id.clone();
        job.fail_step(&step, "缺少音色文件");
        assert_eq!(job.status, JobStatus::Failed);
        assert!(job.error.as_deref().unwrap().contains("音色"));
    }

    #[test]
    fn job_can_pause_and_resume() {
        let mut job = Job::new(JobKind::Storyboard, "分镜", vec![JobStep::new("镜头表")]);
        job.start();
        job.pause("等待用户确认工作流");
        assert_eq!(job.status, JobStatus::Paused);
        assert!(job.error.as_deref().unwrap().contains("工作流"));

        job.resume();
        assert_eq!(job.status, JobStatus::Running);
        assert!(job.error.is_none());
    }
}
