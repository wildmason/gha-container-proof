//! Public receipt schema for `gha-container-proof`.
//!
//! The shape of every command's output: a [`ContainerProofReceipt`] with a
//! schema version, tool identity, mode, overall compatibility rollup, summary,
//! a list of subjects (job containers, Docker actions, Docker probes), and a
//! flat list of checks aggregated from all subjects.

use chrono::{DateTime, Utc};
use clap::ValueEnum;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub type SchemaVersion = u32;
pub const SCHEMA_VERSION: SchemaVersion = 1;

#[derive(Clone, Copy, Debug, ValueEnum, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Text,
    Json,
    Markdown,
}

#[derive(Clone, Copy, Debug, ValueEnum, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum RunnerOs {
    Linux,
    Windows,
    Macos,
}

impl RunnerOs {
    pub fn gha_name(self) -> &'static str {
        match self {
            Self::Linux => "Linux",
            Self::Windows => "Windows",
            Self::Macos => "macOS",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolInfo {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
    Skip,
}

impl CheckStatus {
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Pass => "✓",
            Self::Warn => "!",
            Self::Fail => "✗",
            Self::Skip => "-",
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Warn => "warn",
            Self::Fail => "fail",
            Self::Skip => "skip",
        }
    }
}

/// Top-level rollup classification.
///
/// `exact` = no warnings or failures and no simulated subjects.
/// `compatible` = warnings present but no failures and no simulated subjects.
/// `simulated` = at least one subject is simulated (expression-bearing image,
///     missing remote action manifest, image pull required while offline, etc.).
/// `unsupported` = at least one subject failed outright (non-Linux job container,
///     `--network` in options, missing required image, missing Dockerfile, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Compatibility {
    Exact,
    Compatible,
    Simulated,
    Unsupported,
}

impl Compatibility {
    /// The worse of `self` and `other`. `Unsupported > Simulated > Compatible > Exact`.
    pub fn worse(self, other: Self) -> Self {
        use Compatibility::*;
        match (self, other) {
            (Unsupported, _) | (_, Unsupported) => Unsupported,
            (Simulated, _) | (_, Simulated) => Simulated,
            (Compatible, _) | (_, Compatible) => Compatible,
            _ => Exact,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkModel {
    /// ci-forge owns container networking; user `--network` is rejected.
    CiForgeManaged,
    /// Docker default bridge networking; no networking concerns flagged.
    DockerDefault,
    /// Workflow attempted to set unsupported custom networking.
    UnsupportedCustom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum SubjectKind {
    JobContainer,
    DockerAction,
    DockerProbe,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReceiptSummary {
    pub passed: usize,
    pub warnings: usize,
    pub failed: usize,
    pub skipped: usize,
}

impl ReceiptSummary {
    pub fn from_checks(checks: &[Check]) -> Self {
        let mut summary = Self::default();
        for check in checks {
            summary.tally(check.status);
        }
        summary
    }

    pub fn tally(&mut self, status: CheckStatus) {
        match status {
            CheckStatus::Pass => self.passed += 1,
            CheckStatus::Warn => self.warnings += 1,
            CheckStatus::Fail => self.failed += 1,
            CheckStatus::Skip => self.skipped += 1,
        }
    }

    pub fn add(&mut self, other: &Self) {
        self.passed += other.passed;
        self.warnings += other.warnings;
        self.failed += other.failed;
        self.skipped += other.skipped;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Check {
    pub id: String,
    pub status: CheckStatus,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl Check {
    pub fn new(id: impl Into<String>, status: CheckStatus, message: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            status,
            message: message.into(),
            location: None,
            details: None,
        }
    }

    pub fn pass(id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(id, CheckStatus::Pass, message)
    }

    pub fn warn(id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(id, CheckStatus::Warn, message)
    }

    pub fn fail(id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(id, CheckStatus::Fail, message)
    }

    pub fn skip(id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(id, CheckStatus::Skip, message)
    }

    pub fn at(mut self, location: impl Into<String>) -> Self {
        self.location = Some(location.into());
        self
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
}

/// One container subject in a receipt: a job container, a Docker action, or a
/// Docker probe.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Subject {
    pub kind: SubjectKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dockerfile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runner_os: Option<RunnerOs>,
    pub classification: Compatibility,
    pub network_model: NetworkModel,
    pub requires_docker: bool,
    pub requires_build: bool,
    pub requires_pull: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub credentials_redacted: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env_redacted: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe: Option<ProbeReport>,
    pub summary: ReceiptSummary,
    pub checks: Vec<Check>,
}

impl Subject {
    pub fn new(kind: SubjectKind) -> Self {
        Self {
            kind,
            job_id: None,
            step_id: None,
            action_ref: None,
            image: None,
            dockerfile: None,
            runner_os: None,
            classification: Compatibility::Exact,
            network_model: NetworkModel::DockerDefault,
            requires_docker: false,
            requires_build: false,
            requires_pull: false,
            credentials_redacted: Vec::new(),
            env_redacted: Vec::new(),
            probe: None,
            summary: ReceiptSummary::default(),
            checks: Vec::new(),
        }
    }

    pub fn push(&mut self, check: Check) {
        self.summary.tally(check.status);
        self.checks.push(check);
    }

    /// Recompute `summary` from `checks` and possibly downgrade `classification`
    /// based on observed warnings/failures. Failures always force `Unsupported`;
    /// warnings nudge `Exact` to `Compatible` but do not downgrade `Simulated`.
    pub fn finalize(&mut self) {
        self.summary = ReceiptSummary::from_checks(&self.checks);
        if self.summary.failed > 0 {
            self.classification = Compatibility::Unsupported;
        } else if self.classification == Compatibility::Exact && self.summary.warnings > 0 {
            self.classification = Compatibility::Compatible;
        }
    }
}

/// Probe-specific evidence attached to a `docker-probe` subject.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProbeReport {
    pub docker_cli_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docker_bin: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inspect: Option<ProbeStep>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ProbeStep>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<ProbeStep>,
}

impl ProbeReport {
    pub fn new() -> Self {
        Self {
            docker_cli_available: false,
            docker_bin: None,
            inspect: None,
            tools: Vec::new(),
            commands: Vec::new(),
        }
    }
}

impl Default for ProbeReport {
    fn default() -> Self {
        Self::new()
    }
}

/// One step of probe evidence: the docker command that was invoked, its exit
/// code, and excerpts of stdout/stderr.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProbeStep {
    pub kind: ProbeStepKind,
    pub command: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub elapsed_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spawn_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum ProbeStepKind {
    Inspect,
    Tool,
    Command,
    Pull,
}

/// The top-level receipt. The same shape is emitted by all four commands.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContainerProofReceipt {
    pub schema_version: SchemaVersion,
    pub tool: ToolInfo,
    pub checked_at: DateTime<Utc>,
    pub mode: String,
    pub compatibility: Compatibility,
    pub summary: ReceiptSummary,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subjects: Vec<Subject>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<Check>,
}

impl ContainerProofReceipt {
    /// Construct a receipt from a mode label and a list of subjects, plus any
    /// receipt-level checks (e.g. workflow-scan rollups). Aggregates summary
    /// and compatibility automatically.
    pub fn build(
        mode: impl Into<String>,
        mut subjects: Vec<Subject>,
        receipt_checks: Vec<Check>,
    ) -> Self {
        for subject in &mut subjects {
            subject.finalize();
        }

        let mut summary = ReceiptSummary::from_checks(&receipt_checks);
        for subject in &subjects {
            summary.add(&subject.summary);
        }

        let compatibility = subjects
            .iter()
            .map(|subject| subject.classification)
            .fold(Compatibility::Exact, Compatibility::worse);

        let compatibility = if subjects.is_empty() && summary.failed == 0 {
            // Workflow scans with nothing to classify still report Exact.
            Compatibility::Exact
        } else {
            compatibility
        };

        Self {
            schema_version: SCHEMA_VERSION,
            tool: ToolInfo {
                name: crate::TOOL_NAME.to_owned(),
                version: crate::TOOL_VERSION.to_owned(),
            },
            checked_at: Utc::now(),
            mode: mode.into(),
            compatibility,
            summary,
            subjects,
            checks: receipt_checks,
        }
    }

    /// True when no check is `fail`, used by the CLI to decide exit code. With
    /// `strict`, warnings are treated as failures.
    pub fn is_success(&self, strict: bool) -> bool {
        self.summary.failed == 0 && (!strict || self.summary.warnings == 0)
    }
}

/// Return `true` for environment-variable keys that look secret-ish.
pub fn is_sensitive_key(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    [
        "PASSWORD",
        "PASS",
        "SECRET",
        "TOKEN",
        "CREDENTIAL",
        "API_KEY",
        "ACCESS_KEY",
        "PRIVATE_KEY",
    ]
    .iter()
    .any(|needle| upper.contains(needle))
        || (upper.contains("KEY") && !upper.starts_with("KEYWORD"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compatibility_worse_orders_correctly() {
        use Compatibility::*;
        assert_eq!(Exact.worse(Compatible), Compatible);
        assert_eq!(Compatible.worse(Simulated), Simulated);
        assert_eq!(Simulated.worse(Unsupported), Unsupported);
        assert_eq!(Unsupported.worse(Exact), Unsupported);
    }

    #[test]
    fn subject_finalize_promotes_failures_to_unsupported() {
        let mut subject = Subject::new(SubjectKind::JobContainer);
        subject.classification = Compatibility::Exact;
        subject.push(Check::fail("x", "broken"));
        subject.finalize();
        assert_eq!(subject.classification, Compatibility::Unsupported);
        assert_eq!(subject.summary.failed, 1);
    }

    #[test]
    fn subject_finalize_promotes_warnings_to_compatible() {
        let mut subject = Subject::new(SubjectKind::JobContainer);
        subject.classification = Compatibility::Exact;
        subject.push(Check::warn("x", "iffy"));
        subject.finalize();
        assert_eq!(subject.classification, Compatibility::Compatible);
        assert_eq!(subject.summary.warnings, 1);
    }

    #[test]
    fn subject_finalize_keeps_simulated_with_warnings() {
        let mut subject = Subject::new(SubjectKind::DockerAction);
        subject.classification = Compatibility::Simulated;
        subject.push(Check::warn("x", "iffy"));
        subject.finalize();
        assert_eq!(subject.classification, Compatibility::Simulated);
    }

    #[test]
    fn receipt_build_rolls_up_compatibility() {
        let mut subject = Subject::new(SubjectKind::JobContainer);
        subject.classification = Compatibility::Simulated;
        let receipt = ContainerProofReceipt::build("plan-job", vec![subject], Vec::new());
        assert_eq!(receipt.compatibility, Compatibility::Simulated);
    }

    #[test]
    fn is_sensitive_key_catches_common_shapes() {
        assert!(is_sensitive_key("DATABASE_PASSWORD"));
        assert!(is_sensitive_key("github_token"));
        assert!(is_sensitive_key("API_SECRET"));
        assert!(is_sensitive_key("MY_PRIVATE_KEY"));
        assert!(!is_sensitive_key("NODE_ENV"));
        assert!(!is_sensitive_key("KEYWORD"));
    }
}
