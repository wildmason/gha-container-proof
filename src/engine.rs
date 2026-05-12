//! Top-level orchestration for the four CLI commands. Each `run_*` function
//! returns a finished [`ContainerProofReceipt`].

use anyhow::Result;

use crate::model::ContainerProofReceipt;
use crate::plan::{ActionPlanInput, JobPlanInput, plan_action, plan_job};
use crate::probe::{ProbeInput, probe};
use crate::workflow::{CheckWorkflowOptions, scan_workflows};

pub fn run_check_workflow(options: &CheckWorkflowOptions) -> Result<ContainerProofReceipt> {
    let scan = scan_workflows(options)?;
    Ok(ContainerProofReceipt::build(
        "check-workflow",
        scan.subjects,
        scan.checks,
    ))
}

pub fn run_plan_job(input: &JobPlanInput) -> Result<ContainerProofReceipt> {
    let subject = plan_job(input);
    Ok(ContainerProofReceipt::build(
        "plan-job",
        vec![subject],
        Vec::new(),
    ))
}

pub fn run_plan_action(input: &ActionPlanInput) -> Result<ContainerProofReceipt> {
    let subject = plan_action(input);
    Ok(ContainerProofReceipt::build(
        "plan-action",
        vec![subject],
        Vec::new(),
    ))
}

pub fn run_probe(input: &ProbeInput) -> Result<ContainerProofReceipt> {
    let subject = probe(input);
    Ok(ContainerProofReceipt::build(
        "probe",
        vec![subject],
        Vec::new(),
    ))
}

/// Promote selected warnings to failures when strict mode is on.
///
/// Strict mode in v1 is binary: any warning becomes a failure. The
/// [`docs/RULES.md`](../../docs/RULES.md) lists which checks are *typically*
/// promoted, but for stability we apply it uniformly so the CLI's exit code
/// matches the simple "no warnings, no failures" expectation users have.
pub fn apply_strict(receipt: &mut ContainerProofReceipt) {
    for subject in &mut receipt.subjects {
        for check in &mut subject.checks {
            if check.status == crate::model::CheckStatus::Warn {
                check.status = crate::model::CheckStatus::Fail;
            }
        }
        subject.finalize();
    }
    for check in &mut receipt.checks {
        if check.status == crate::model::CheckStatus::Warn {
            check.status = crate::model::CheckStatus::Fail;
        }
    }
    // Recompute summary.
    let mut summary = crate::model::ReceiptSummary::from_checks(&receipt.checks);
    for subject in &receipt.subjects {
        summary.add(&subject.summary);
    }
    receipt.summary = summary;
    receipt.compatibility = receipt
        .subjects
        .iter()
        .map(|subject| subject.classification)
        .fold(
            crate::model::Compatibility::Exact,
            crate::model::Compatibility::worse,
        );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Compatibility, RunnerOs};

    #[test]
    fn run_plan_job_smoke() {
        let receipt = run_plan_job(&JobPlanInput {
            job_id: "build".to_owned(),
            runner_os: RunnerOs::Linux,
            runs_on: vec!["ubuntu-22.04".to_owned()],
            container_image: Some("node:22-bookworm".to_owned()),
            env: Vec::new(),
            ports: Vec::new(),
            volumes: Vec::new(),
            options: String::new(),
            credentials_username_present: false,
            credentials_password_present: false,
            location: None,
        })
        .unwrap();
        assert_eq!(receipt.mode, "plan-job");
        assert_eq!(receipt.subjects.len(), 1);
        assert!(receipt.compatibility != Compatibility::Unsupported);
    }

    #[test]
    fn apply_strict_promotes_warnings() {
        let mut receipt = run_plan_job(&JobPlanInput {
            job_id: "build".to_owned(),
            runner_os: RunnerOs::Linux,
            runs_on: vec!["ubuntu-22.04".to_owned()],
            container_image: Some("node:latest".to_owned()),
            env: Vec::new(),
            ports: Vec::new(),
            volumes: Vec::new(),
            options: String::new(),
            credentials_username_present: false,
            credentials_password_present: false,
            location: None,
        })
        .unwrap();
        assert!(receipt.summary.warnings >= 1);
        apply_strict(&mut receipt);
        assert_eq!(receipt.summary.warnings, 0);
        assert!(receipt.summary.failed >= 1);
    }
}
