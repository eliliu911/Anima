//! Workspace and sandbox policy.
//!
//! The model should never decide filesystem or network boundaries directly.
//! These types mirror the important Codex ideas in Anima terms: explicit
//! permission profiles, workspace roots, protected metadata, and separate
//! network access.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Coarse execution profile for a turn or job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxMode {
    /// Tools may inspect the workspace but cannot mutate it.
    ReadOnly,
    /// Tools may write inside configured workspace roots, except protected paths.
    #[default]
    WorkspaceWrite,
    /// Tools may access the host. This should require explicit user opt-in.
    FullAccess,
}

/// Network is controlled independently from filesystem access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkPolicy {
    #[default]
    Restricted,
    Enabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileCapability {
    Read,
    Write,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum PathVerdict {
    Allow,
    Deny { reason: String },
}

impl PathVerdict {
    pub fn allowed(&self) -> bool {
        matches!(self, Self::Allow)
    }

    pub fn reason(&self) -> &str {
        match self {
            Self::Allow => "已放行",
            Self::Deny { reason } => reason,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimePolicy {
    pub sandbox: SandboxMode,
    pub network: NetworkPolicy,
    pub workspace_roots: Vec<PathBuf>,
    #[serde(default = "default_true")]
    pub allow_workspace_reads: bool,
    #[serde(default = "default_true")]
    pub allow_workspace_writes: bool,
    /// Path segments that are protected from agent writes even inside a root.
    #[serde(default = "default_protected_segments")]
    pub protected_segments: Vec<String>,
    /// File names that should not be read or written by the agent.
    #[serde(default = "default_secret_names")]
    pub secret_names: Vec<String>,
    /// Project-configured deny patterns, e.g. ".env*" or "node_modules/".
    #[serde(default = "default_deny_patterns")]
    pub deny_patterns: Vec<String>,
    #[serde(default = "default_command_allow_prefixes")]
    pub command_allow_prefixes: Vec<String>,
    #[serde(default)]
    pub command_deny_substrings: Vec<String>,
    /// 这个项目里免审批的**工具名**。命令类工具走前缀，非命令类只能按名字。
    ///
    /// **免审批不等于免策略** —— 工具自己照样调 `evaluate_path` /
    /// `evaluate_command`，deny 名单和不可逆命令照样拦。
    #[serde(default)]
    pub auto_approve_tools: Vec<String>,
}

impl RuntimePolicy {
    pub fn read_only(root: impl Into<PathBuf>) -> Self {
        Self {
            sandbox: SandboxMode::ReadOnly,
            network: NetworkPolicy::Restricted,
            workspace_roots: vec![root.into()],
            allow_workspace_reads: true,
            allow_workspace_writes: false,
            protected_segments: default_protected_segments(),
            secret_names: default_secret_names(),
            deny_patterns: default_deny_patterns(),
            command_allow_prefixes: default_command_allow_prefixes(),
            command_deny_substrings: Vec::new(),
            // 预设不给任何免审批工具 —— 那是项目自己的选择，不是默认值
            auto_approve_tools: Vec::new(),
        }
    }

    pub fn workspace_write(root: impl Into<PathBuf>) -> Self {
        Self {
            sandbox: SandboxMode::WorkspaceWrite,
            network: NetworkPolicy::Restricted,
            workspace_roots: vec![root.into()],
            allow_workspace_reads: true,
            allow_workspace_writes: true,
            protected_segments: default_protected_segments(),
            secret_names: default_secret_names(),
            deny_patterns: default_deny_patterns(),
            command_allow_prefixes: default_command_allow_prefixes(),
            command_deny_substrings: Vec::new(),
            // 预设不给任何免审批工具 —— 那是项目自己的选择，不是默认值
            auto_approve_tools: Vec::new(),
        }
    }

    pub fn from_project_permissions(
        root: impl Into<PathBuf>,
        permissions: &crate::workspace::ProjectPermissions,
    ) -> Self {
        let root = root.into();
        let roots = if permissions.file_access.allowed_paths.is_empty() {
            vec![root]
        } else {
            permissions
                .file_access
                .allowed_paths
                .iter()
                .filter(|p| !p.trim().is_empty())
                .map(PathBuf::from)
                .collect()
        };

        let sandbox = if permissions.terminal_policy.sandbox_mode == "unsandboxed" {
            SandboxMode::FullAccess
        } else if permissions.security_preset == "strict" || !permissions.file_access.allow_writes {
            SandboxMode::ReadOnly
        } else {
            SandboxMode::WorkspaceWrite
        };

        Self {
            sandbox,
            network: NetworkPolicy::Restricted,
            workspace_roots: roots,
            allow_workspace_reads: permissions.file_access.allow_reads,
            allow_workspace_writes: permissions.file_access.allow_writes,
            protected_segments: default_protected_segments(),
            secret_names: default_secret_names(),
            deny_patterns: permissions.file_access.denied_paths.clone(),
            command_allow_prefixes: permissions.terminal_policy.auto_approve_commands.clone(),
            command_deny_substrings: permissions.terminal_policy.blocked_commands.clone(),
            auto_approve_tools: permissions.auto_approve_tools.clone(),
        }
    }

    pub fn command_policy(&self) -> crate::mcp::CommandPolicy {
        crate::mcp::CommandPolicy {
            allow_prefixes: self.command_allow_prefixes.clone(),
            deny_substrings: self.command_deny_substrings.clone(),
        }
    }

    /// Decide whether a concrete path operation is inside the current boundary.
    ///
    /// This is a policy check, not an OS sandbox. Actual command execution still
    /// needs a platform sandbox once the executor is wired in.
    pub fn evaluate_path(&self, path: impl AsRef<Path>, capability: FileCapability) -> PathVerdict {
        let path = normalize_for_policy(path.as_ref());

        if matches!(capability, FileCapability::Read) && !self.allow_workspace_reads {
            return PathVerdict::Deny {
                reason: "当前项目策略禁止读取 workspace 文件".to_string(),
            };
        }

        if matches!(capability, FileCapability::Write) && !self.allow_workspace_writes {
            return PathVerdict::Deny {
                reason: "当前项目策略禁止写入 workspace 文件".to_string(),
            };
        }

        if matches!(capability, FileCapability::Read | FileCapability::Write)
            && self.is_secret_path(&path)
        {
            return PathVerdict::Deny {
                reason: format!(
                    "路径 {} 看起来包含密钥或环境配置，已拒绝访问",
                    path.display()
                ),
            };
        }

        if let Some(pattern) = self
            .deny_patterns
            .iter()
            .find(|pattern| matches_policy_pattern(&path, pattern))
        {
            return PathVerdict::Deny {
                reason: format!("路径 {} 命中项目拒绝规则 {pattern}", path.display()),
            };
        }

        if matches!(capability, FileCapability::Write) && self.is_protected_metadata_path(&path) {
            return PathVerdict::Deny {
                reason: format!("路径 {} 位于受保护的 agent/版本库元数据内", path.display()),
            };
        }

        match self.sandbox {
            SandboxMode::FullAccess => PathVerdict::Allow,
            SandboxMode::ReadOnly if capability == FileCapability::Write => PathVerdict::Deny {
                reason: "当前是只读沙箱，不能写入文件".to_string(),
            },
            SandboxMode::ReadOnly | SandboxMode::WorkspaceWrite => {
                if self.is_inside_workspace(&path) {
                    PathVerdict::Allow
                } else {
                    PathVerdict::Deny {
                        reason: format!("路径 {} 不在授权 workspace 内", path.display()),
                    }
                }
            }
        }
    }

    pub fn allows_network(&self) -> bool {
        self.network == NetworkPolicy::Enabled
    }

    fn is_inside_workspace(&self, path: &Path) -> bool {
        self.workspace_roots
            .iter()
            .map(|root| normalize_for_policy(root))
            .any(|root| path == root || path.starts_with(root))
    }

    fn is_protected_metadata_path(&self, path: &Path) -> bool {
        path.components().any(|component| match component {
            Component::Normal(name) => {
                let name = name.to_string_lossy();
                self.protected_segments.iter().any(|p| p == &name)
            }
            _ => false,
        })
    }

    fn is_secret_path(&self, path: &Path) -> bool {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                let lower = name.to_ascii_lowercase();
                self.secret_names
                    .iter()
                    .any(|secret| lower == *secret || lower.starts_with(&format!("{secret}.")))
            })
    }
}

fn default_protected_segments() -> Vec<String> {
    [".git", ".codex", ".agents"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn default_secret_names() -> Vec<String> {
    [".env", "id_rsa", "id_ed25519", "credentials", "secrets"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn default_deny_patterns() -> Vec<String> {
    [".env*", "id_rsa*", "id_ed25519*"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn default_command_allow_prefixes() -> Vec<String> {
    crate::mcp::CommandPolicy::default().allow_prefixes
}

fn default_true() -> bool {
    true
}

fn matches_policy_pattern(path: &Path, pattern: &str) -> bool {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return false;
    }

    let normalized = path.to_string_lossy().replace('\\', "/");
    let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    let directory_pattern = pattern.trim_end_matches('/');

    if pattern.ends_with('/') {
        return path.components().any(|component| {
            component
                .as_os_str()
                .to_str()
                .is_some_and(|name| name == directory_pattern)
        });
    }

    if let Some((prefix, suffix)) = pattern.split_once('*') {
        return filename.starts_with(prefix) && filename.ends_with(suffix);
    }

    filename == pattern || normalized.ends_with(pattern)
}

fn normalize_for_policy(path: &Path) -> PathBuf {
    if let Ok(canonical) = path.canonicalize() {
        return canonical;
    }

    // The target may not exist yet. Canonicalize the nearest existing parent so
    // writes to new files are still checked against the real workspace root.
    let mut suffix = PathBuf::new();
    let mut cursor = path;
    while let Some(parent) = cursor.parent() {
        if let Some(name) = cursor.file_name() {
            suffix = Path::new(name).join(suffix);
        }
        if let Ok(canonical_parent) = parent.canonicalize() {
            return canonical_parent.join(suffix);
        }
        cursor = parent;
    }
    path.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_write_allows_regular_workspace_files() {
        let dir = tempfile::tempdir().unwrap();
        let policy = RuntimePolicy::workspace_write(dir.path());
        let verdict = policy.evaluate_path(dir.path().join("draft.txt"), FileCapability::Write);
        assert!(verdict.allowed(), "expected allow, got {verdict:?}");
    }

    #[test]
    fn workspace_write_denies_outside_workspace() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let policy = RuntimePolicy::workspace_write(root.path());
        let verdict = policy.evaluate_path(outside.path().join("x.txt"), FileCapability::Write);
        assert!(!verdict.allowed());
        assert!(verdict.reason().contains("不在授权 workspace"));
    }

    #[test]
    fn read_only_denies_writes_but_allows_reads() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("notes.md");
        std::fs::write(&file, "hello").unwrap();
        let policy = RuntimePolicy::read_only(dir.path());

        assert!(policy.evaluate_path(&file, FileCapability::Read).allowed());
        assert!(!policy.evaluate_path(&file, FileCapability::Write).allowed());
    }

    #[test]
    fn protected_metadata_cannot_be_written() {
        let dir = tempfile::tempdir().unwrap();
        let policy = RuntimePolicy::workspace_write(dir.path());
        let verdict = policy.evaluate_path(dir.path().join(".git/config"), FileCapability::Write);
        assert!(!verdict.allowed());
        assert!(verdict.reason().contains("受保护"));
    }

    #[test]
    fn secrets_cannot_be_read() {
        let dir = tempfile::tempdir().unwrap();
        let policy = RuntimePolicy::workspace_write(dir.path());
        let verdict = policy.evaluate_path(dir.path().join(".env.local"), FileCapability::Read);
        assert!(!verdict.allowed());
        assert!(verdict.reason().contains("密钥"));
    }

    #[test]
    fn project_permissions_feed_runtime_policy() {
        let dir = tempfile::tempdir().unwrap();
        let permissions = crate::workspace::ProjectPermissions {
            security_preset: "strict".to_string(),
            file_access: crate::workspace::FileAccessPolicy {
                allow_reads: true,
                allow_writes: false,
                allowed_paths: vec![dir.path().to_string_lossy().to_string()],
                denied_paths: vec!["node_modules/".to_string()],
            },
            terminal_policy: crate::workspace::TerminalCommandPolicy {
                auto_approve_commands: vec![],
                blocked_commands: vec![],
                sandbox_mode: "standard".to_string(),
            },
                    auto_approve_tools: Vec::new(),
        };

        let policy = RuntimePolicy::from_project_permissions(dir.path(), &permissions);
        assert_eq!(policy.sandbox, SandboxMode::ReadOnly);
        assert!(!policy.allow_workspace_writes);
        assert!(!policy
            .evaluate_path(
                dir.path().join("node_modules/pkg/index.js"),
                FileCapability::Read
            )
            .allowed());
    }
}
