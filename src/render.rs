//! Text, JSON, and Markdown rendering for [`ContainerProofReceipt`].

use anyhow::Result;

use crate::model::{
    Check, CheckStatus, Compatibility, ContainerProofReceipt, OutputFormat, ProbeStepKind, Subject,
    SubjectKind,
};

pub fn render_receipt(receipt: &ContainerProofReceipt, format: OutputFormat) -> Result<String> {
    match format {
        OutputFormat::Text => Ok(render_text(receipt)),
        OutputFormat::Json => Ok(format!("{}\n", serde_json::to_string_pretty(receipt)?)),
        OutputFormat::Markdown => Ok(render_markdown(receipt)),
    }
}

fn render_text(receipt: &ContainerProofReceipt) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{} {} ({})\n",
        receipt.tool.name, receipt.tool.version, receipt.mode
    ));
    out.push_str(&format!(
        "compatibility: {}\n",
        compatibility_word(receipt.compatibility)
    ));
    out.push_str(&format!(
        "summary: {} passed, {} warned, {} failed, {} skipped\n",
        receipt.summary.passed,
        receipt.summary.warnings,
        receipt.summary.failed,
        receipt.summary.skipped
    ));

    for subject in &receipt.subjects {
        out.push('\n');
        out.push_str(&render_subject_text(subject));
    }

    if !receipt.checks.is_empty() {
        out.push('\n');
        out.push_str("checks:\n");
        for check in &receipt.checks {
            out.push_str(&format!("  {}\n", render_check_line(check)));
        }
    }

    out
}

fn render_subject_text(subject: &Subject) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{} {}\n",
        subject_kind_word(subject.kind),
        subject_id(subject)
    ));
    if let Some(image) = &subject.image {
        out.push_str(&format!("  image: {image}\n"));
    }
    if let Some(dockerfile) = &subject.dockerfile {
        out.push_str(&format!("  dockerfile: {dockerfile}\n"));
    }
    if let Some(action_ref) = &subject.action_ref {
        out.push_str(&format!("  action: {action_ref}\n"));
    }
    if let Some(runner_os) = subject.runner_os {
        out.push_str(&format!("  runner-os: {}\n", runner_os.gha_name()));
    }
    out.push_str(&format!(
        "  classification: {}\n",
        compatibility_word(subject.classification)
    ));
    out.push_str(&format!(
        "  network: {}\n",
        network_word(subject.network_model)
    ));
    out.push_str(&format!(
        "  requires: docker={} build={} pull={}\n",
        subject.requires_docker, subject.requires_build, subject.requires_pull
    ));
    if !subject.credentials_redacted.is_empty() {
        out.push_str(&format!(
            "  credentials redacted: {}\n",
            subject.credentials_redacted.join(", ")
        ));
    }
    if !subject.env_redacted.is_empty() {
        out.push_str(&format!(
            "  env redacted: {}\n",
            subject.env_redacted.join(", ")
        ));
    }
    if let Some(probe) = &subject.probe {
        out.push_str(&format!(
            "  probe: docker-cli={} bin={}\n",
            probe.docker_cli_available,
            probe.docker_bin.as_deref().unwrap_or("(none)")
        ));
        if let Some(inspect) = &probe.inspect {
            out.push_str(&format!(
                "    inspect: success={} exit={:?} elapsed={}ms\n",
                inspect.success, inspect.exit_code, inspect.elapsed_ms
            ));
        }
        for step in probe.tools.iter().chain(probe.commands.iter()) {
            out.push_str(&format!(
                "    {}: success={} exit={:?} cmd=`{}`\n",
                probe_step_kind_word(step.kind),
                step.success,
                step.exit_code,
                step.command
            ));
        }
    }
    for check in &subject.checks {
        out.push_str(&format!("  {}\n", render_check_line(check)));
    }
    out
}

fn render_check_line(check: &Check) -> String {
    let location = match &check.location {
        Some(loc) => format!(" @ {loc}"),
        None => String::new(),
    };
    format!(
        "{} {:<4} {} - {}{}",
        check.status.symbol(),
        check.status.word(),
        check.id,
        check.message,
        location
    )
}

fn render_markdown(receipt: &ContainerProofReceipt) -> String {
    let mut out = String::new();
    out.push_str("# gha-container-proof Receipt\n\n");
    out.push_str(&format!(
        "- Tool: `{}` `{}`\n",
        receipt.tool.name, receipt.tool.version
    ));
    out.push_str(&format!("- Mode: `{}`\n", markdown_escape(&receipt.mode)));
    out.push_str(&format!(
        "- Compatibility: `{}`\n",
        compatibility_word(receipt.compatibility)
    ));
    out.push_str(&format!("- Checked at: `{}`\n", receipt.checked_at));
    out.push_str(&format!(
        "- Summary: **{} passed**, **{} warned**, **{} failed**, **{} skipped**\n\n",
        receipt.summary.passed,
        receipt.summary.warnings,
        receipt.summary.failed,
        receipt.summary.skipped
    ));

    if !receipt.subjects.is_empty() {
        out.push_str("## Subjects\n\n");
        out.push_str("| Kind | Identifier | Image | Classification | Network | Docker | Build | Pull | Checks |\n");
        out.push_str("| --- | --- | --- | --- | --- | --- | --- | --- | --- |\n");
        for subject in &receipt.subjects {
            out.push_str(&format!(
                "| `{}` | `{}` | `{}` | `{}` | `{}` | {} | {} | {} | {}/{}/{}/{} |\n",
                subject_kind_word(subject.kind),
                markdown_escape(&subject_id(subject)),
                markdown_escape(subject.image.as_deref().unwrap_or("-")),
                compatibility_word(subject.classification),
                network_word(subject.network_model),
                bool_emoji(subject.requires_docker),
                bool_emoji(subject.requires_build),
                bool_emoji(subject.requires_pull),
                subject.summary.passed,
                subject.summary.warnings,
                subject.summary.failed,
                subject.summary.skipped
            ));
        }
        out.push('\n');
    }

    let failed: Vec<&Check> = subject_check_iter(receipt)
        .filter(|check| check.status == CheckStatus::Fail)
        .collect();
    if !failed.is_empty() {
        out.push_str("## Failed checks\n\n");
        for check in failed {
            out.push_str(&format!(
                "- `{}` — {}{}\n",
                check.id,
                markdown_escape(&check.message),
                check
                    .location
                    .as_deref()
                    .map(|loc| format!(" _(at `{}`)_", markdown_escape(loc)))
                    .unwrap_or_default()
            ));
        }
        out.push('\n');
    }

    let warnings: Vec<&Check> = subject_check_iter(receipt)
        .filter(|check| check.status == CheckStatus::Warn)
        .collect();
    if !warnings.is_empty() {
        out.push_str("## Warnings\n\n");
        for check in warnings {
            out.push_str(&format!(
                "- `{}` — {}{}\n",
                check.id,
                markdown_escape(&check.message),
                check
                    .location
                    .as_deref()
                    .map(|loc| format!(" _(at `{}`)_", markdown_escape(loc)))
                    .unwrap_or_default()
            ));
        }
        out.push('\n');
    }

    let probes: Vec<&Subject> = receipt
        .subjects
        .iter()
        .filter(|subject| subject.kind == SubjectKind::DockerProbe)
        .collect();
    if !probes.is_empty() {
        out.push_str("## Probe evidence\n\n");
        for subject in probes {
            out.push_str(&format!(
                "### `{}`\n\n",
                markdown_escape(subject.image.as_deref().unwrap_or("(unnamed)"))
            ));
            if let Some(probe) = &subject.probe {
                out.push_str(&format!(
                    "- Docker CLI available: `{}`\n",
                    probe.docker_cli_available
                ));
                if let Some(bin) = &probe.docker_bin {
                    out.push_str(&format!("- Docker bin: `{}`\n", markdown_escape(bin)));
                }
                if let Some(inspect) = &probe.inspect {
                    out.push_str(&format!(
                        "- `inspect`: success=`{}`, exit=`{:?}`, elapsed=`{}ms`\n",
                        inspect.success, inspect.exit_code, inspect.elapsed_ms
                    ));
                }
                for step in probe.tools.iter().chain(probe.commands.iter()) {
                    out.push_str(&format!(
                        "- `{}`: `{}` exit=`{:?}` elapsed=`{}ms`\n",
                        probe_step_kind_word(step.kind),
                        markdown_escape(&step.command),
                        step.exit_code,
                        step.elapsed_ms
                    ));
                }
            }
            out.push('\n');
        }
    }

    out
}

fn subject_check_iter(receipt: &ContainerProofReceipt) -> impl Iterator<Item = &Check> {
    receipt
        .checks
        .iter()
        .chain(receipt.subjects.iter().flat_map(|subject| &subject.checks))
}

fn subject_id(subject: &Subject) -> String {
    match subject.kind {
        SubjectKind::JobContainer => subject.job_id.clone().unwrap_or_else(|| "<job>".to_owned()),
        SubjectKind::DockerAction => {
            let step = subject.step_id.as_deref().unwrap_or("<step>");
            let action = subject.action_ref.as_deref().unwrap_or("<action>");
            format!("{step}: {action}")
        }
        SubjectKind::DockerProbe => subject
            .image
            .clone()
            .unwrap_or_else(|| "<image>".to_owned()),
    }
}

fn subject_kind_word(kind: SubjectKind) -> &'static str {
    match kind {
        SubjectKind::JobContainer => "job-container",
        SubjectKind::DockerAction => "docker-action",
        SubjectKind::DockerProbe => "docker-probe",
    }
}

fn compatibility_word(c: Compatibility) -> &'static str {
    match c {
        Compatibility::Exact => "exact",
        Compatibility::Compatible => "compatible",
        Compatibility::Simulated => "simulated",
        Compatibility::Unsupported => "unsupported",
    }
}

fn network_word(model: crate::model::NetworkModel) -> &'static str {
    match model {
        crate::model::NetworkModel::CiForgeManaged => "ci-forge-managed",
        crate::model::NetworkModel::DockerDefault => "docker-default",
        crate::model::NetworkModel::UnsupportedCustom => "unsupported-custom",
    }
}

fn probe_step_kind_word(kind: ProbeStepKind) -> &'static str {
    match kind {
        ProbeStepKind::Inspect => "inspect",
        ProbeStepKind::Tool => "tool",
        ProbeStepKind::Command => "command",
        ProbeStepKind::Pull => "pull",
    }
}

fn bool_emoji(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn markdown_escape(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Compatibility, ContainerProofReceipt, NetworkModel, Subject, SubjectKind};

    fn fixture() -> ContainerProofReceipt {
        let mut subject = Subject::new(SubjectKind::JobContainer);
        subject.job_id = Some("build".to_owned());
        subject.image = Some("node:22".to_owned());
        subject.network_model = NetworkModel::CiForgeManaged;
        subject.requires_docker = true;
        subject.push(crate::model::Check::pass("container.image.declared", "ok"));
        ContainerProofReceipt::build("plan-job", vec![subject], Vec::new())
    }

    #[test]
    fn text_includes_summary_and_subject() {
        let out = render_text(&fixture());
        assert!(out.contains("gha-container-proof"));
        assert!(out.contains("compatibility: exact"));
        assert!(out.contains("job-container build"));
        assert!(out.contains("node:22"));
    }

    #[test]
    fn markdown_includes_subjects_table() {
        let out = render_markdown(&fixture());
        assert!(out.contains("# gha-container-proof Receipt"));
        assert!(out.contains("## Subjects"));
        assert!(out.contains("`job-container`"));
        assert!(out.contains("`build`"));
    }

    #[test]
    fn json_is_pretty_and_includes_schema_version() {
        let out = render_receipt(&fixture(), OutputFormat::Json).unwrap();
        assert!(out.contains("\"schema_version\": 1"));
        assert!(out.contains("\"compatibility\""));
    }

    #[test]
    fn unused_compatibility_word_remains_stable() {
        // sanity check enum word coverage
        assert_eq!(
            compatibility_word(Compatibility::Unsupported),
            "unsupported"
        );
    }
}
