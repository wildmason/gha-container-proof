//! `gha-container-proof` — GitHub Actions job-container and Docker-action
//! compatibility oracle for offline CI.
//!
//! See [`docs/spec.md`](https://github.com/wildmason/gha-container-proof/blob/main/docs/spec.md)
//! for the protocol surface and [`docs/RULES.md`](https://github.com/wildmason/gha-container-proof/blob/main/docs/RULES.md)
//! for the stable check IDs that this crate emits.

mod action;
mod engine;
mod model;
mod options;
mod plan;
mod probe;
mod render;
mod workflow;

pub use action::{ActionManifest, DockerImage, classify_image};
pub use engine::{apply_strict, run_check_workflow, run_plan_action, run_plan_job, run_probe};
pub use model::{
    Check, CheckStatus, Compatibility, ContainerProofReceipt, NetworkModel, OutputFormat,
    ProbeReport, ProbeStep, ProbeStepKind, ReceiptSummary, RunnerOs, SCHEMA_VERSION, SchemaVersion,
    Subject, SubjectKind, ToolInfo, is_sensitive_key,
};
pub use options::{ClassifiedOption, OptionKind, OptionsPlan, parse_options};
pub use plan::{ActionPlanInput, JobPlanInput};
pub use probe::ProbeInput;
pub use render::render_receipt;
pub use workflow::{CheckWorkflowOptions, ScanResult, scan_workflows};

pub const TOOL_NAME: &str = "gha-container-proof";
pub const TOOL_VERSION: &str = env!("CARGO_PKG_VERSION");
