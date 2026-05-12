//! Docker CLI probe.
//!
//! Offline by default — `docker image inspect <image>` to check local
//! availability and `docker run --rm <image> ...` for tool/command probes.
//! No pulls unless `--allow-pull` is set. No daemon socket use beyond what the
//! `docker` CLI itself does. Skips cleanly when the CLI is absent.

use std::process::{Command, Output};
use std::time::Instant;

use camino::Utf8PathBuf;

use crate::model::{
    Check, Compatibility, ProbeReport, ProbeStep, ProbeStepKind, RunnerOs, Subject, SubjectKind,
};

pub const EXCERPT_LIMIT: usize = 4 * 1024;

const DOCKER_BIN_ENV: &str = "GHA_CONTAINER_PROOF_DOCKER";

#[derive(Debug, Clone)]
pub struct ProbeInput {
    pub image: String,
    pub runner_os: RunnerOs,
    pub tools: Vec<String>,
    pub commands: Vec<String>,
    pub allow_pull: bool,
    pub docker_bin: Option<Utf8PathBuf>,
}

pub fn probe(input: &ProbeInput) -> Subject {
    let mut subject = Subject::new(SubjectKind::DockerProbe);
    subject.image = Some(input.image.clone());
    subject.runner_os = Some(input.runner_os);
    subject.requires_docker = true;
    subject.requires_pull = input.allow_pull;

    let mut report = ProbeReport::new();

    let cli = resolve_docker_bin(input.docker_bin.as_deref());
    let cli_path = match cli {
        Some(path) => path,
        None => {
            subject.classification = Compatibility::Simulated;
            subject.push(Check::skip(
                "probe.docker_cli_not_found",
                "docker CLI not found on PATH and no --docker-bin override; skipping every probe",
            ));
            for tool in &input.tools {
                subject.push(Check::skip(
                    "probe.tool_skipped",
                    format!("tool `{tool}` skipped: docker CLI unavailable"),
                ));
            }
            for command in &input.commands {
                subject.push(Check::skip(
                    "probe.command_skipped",
                    format!("command `{command}` skipped: docker CLI unavailable"),
                ));
            }
            subject.probe = Some(report);
            return subject;
        }
    };
    report.docker_cli_available = true;
    report.docker_bin = Some(cli_path.to_string());

    // 1) Inspect image locally.
    let inspect_step = run_docker(
        &cli_path,
        &["image", "inspect", &input.image],
        ProbeStepKind::Inspect,
    );
    let mut image_available = false;

    if let Some(spawn_err) = &inspect_step.spawn_error {
        subject.classification = Compatibility::Unsupported;
        subject.push(Check::fail(
            "probe.docker_daemon_unreachable",
            format!(
                "could not invoke docker for image `{}`: {spawn_err}",
                input.image
            ),
        ));
    } else if inspect_step.success {
        image_available = true;
        subject.push(Check::pass(
            "probe.image_inspect_ok",
            format!("`docker image inspect {}` succeeded", input.image),
        ));
    } else {
        let daemon = inspect_step
            .stderr
            .as_deref()
            .map(|stderr| {
                stderr
                    .to_ascii_lowercase()
                    .contains("cannot connect to the docker daemon")
            })
            .unwrap_or(false);
        if daemon {
            subject.classification = Compatibility::Unsupported;
            subject.push(Check::fail(
                "probe.docker_daemon_unreachable",
                format!(
                    "`docker image inspect {}` could not reach the daemon",
                    input.image
                ),
            ));
        } else {
            subject.requires_pull = input.allow_pull;
            if input.allow_pull {
                subject.push(Check::warn(
                    "probe.image_pull_required",
                    format!(
                        "image `{}` not present locally; `--allow-pull` is set",
                        input.image
                    ),
                ));
            } else {
                subject.classification = Compatibility::Unsupported;
                subject.push(Check::fail(
                    "probe.image_inspect_missing",
                    format!(
                        "image `{}` not present locally and `--allow-pull` was not set",
                        input.image
                    ),
                ));
            }
        }
    }
    report.inspect = Some(inspect_step);

    // 2) Optional pull when allowed and inspect failed.
    if !image_available && input.allow_pull {
        let pull_step = run_docker(&cli_path, &["pull", &input.image], ProbeStepKind::Pull);
        if pull_step.success {
            image_available = true;
            subject.push(Check::pass(
                "probe.image_pull_attempted",
                format!("`docker pull {}` succeeded", input.image),
            ));
        } else if let Some(err) = &pull_step.spawn_error {
            subject.push(Check::fail(
                "probe.image_pull_attempted",
                format!("`docker pull {}` could not spawn: {err}", input.image),
            ));
        } else {
            subject.push(Check::fail(
                "probe.image_pull_attempted",
                format!(
                    "`docker pull {}` exited {}",
                    input.image,
                    pull_step.exit_code.unwrap_or(-1)
                ),
            ));
        }
        report.commands.push(pull_step);
    }

    let can_run = image_available
        && subject
            .checks
            .iter()
            .all(|check| check.id != "probe.docker_daemon_unreachable");

    // 3) Tools.
    for tool in &input.tools {
        if !can_run {
            subject.push(Check::skip(
                "probe.tool_skipped",
                format!("tool `{tool}` skipped: image not available"),
            ));
            continue;
        }
        let step = run_docker(
            &cli_path,
            &["run", "--rm", &input.image, tool, "--version"],
            ProbeStepKind::Tool,
        );
        if let Some(err) = &step.spawn_error {
            subject.push(Check::fail(
                "probe.tool_spawn_failed",
                format!(
                    "`docker run --rm {} {} --version` could not spawn: {err}",
                    input.image, tool
                ),
            ));
        } else if step.success {
            subject.push(Check::pass(
                "probe.tool_ok",
                format!("`{tool} --version` succeeded in `{}`", input.image),
            ));
        } else {
            subject.push(Check::fail(
                "probe.tool_exit_nonzero",
                format!(
                    "`{tool} --version` exited {} in `{}`",
                    step.exit_code.unwrap_or(-1),
                    input.image
                ),
            ));
        }
        report.tools.push(step);
    }

    // 4) Commands.
    for command in &input.commands {
        if !can_run {
            subject.push(Check::skip(
                "probe.command_skipped",
                format!("command `{command}` skipped: image not available"),
            ));
            continue;
        }
        let step = run_docker(
            &cli_path,
            &["run", "--rm", &input.image, "sh", "-c", command],
            ProbeStepKind::Command,
        );
        if let Some(err) = &step.spawn_error {
            subject.push(Check::fail(
                "probe.command_spawn_failed",
                format!(
                    "`docker run --rm {} sh -c {command}` could not spawn: {err}",
                    input.image
                ),
            ));
        } else if step.success {
            subject.push(Check::pass(
                "probe.command_ok",
                format!("command `{command}` succeeded in `{}`", input.image),
            ));
        } else {
            subject.push(Check::fail(
                "probe.command_exit_nonzero",
                format!(
                    "command `{command}` exited {} in `{}`",
                    step.exit_code.unwrap_or(-1),
                    input.image
                ),
            ));
        }
        report.commands.push(step);
    }

    subject.probe = Some(report);
    subject
}

fn resolve_docker_bin(override_path: Option<&camino::Utf8Path>) -> Option<Utf8PathBuf> {
    if let Some(path) = override_path {
        return Some(path.to_owned());
    }
    if let Ok(value) = std::env::var(DOCKER_BIN_ENV) {
        if !value.trim().is_empty() {
            return Some(Utf8PathBuf::from(value));
        }
    }
    let found = which::which("docker").ok()?;
    Utf8PathBuf::from_path_buf(found).ok()
}

fn run_docker(bin: &camino::Utf8Path, argv: &[&str], kind: ProbeStepKind) -> ProbeStep {
    let display = format!("{} {}", bin, argv.join(" "));
    let start = Instant::now();
    let result = Command::new(bin.as_str()).args(argv).output();
    let elapsed_ms = start.elapsed().as_millis();
    match result {
        Ok(output) => probe_step_from_output(kind, display, output, elapsed_ms),
        Err(err) => ProbeStep {
            kind,
            command: display,
            success: false,
            exit_code: None,
            elapsed_ms,
            stdout: None,
            stderr: None,
            spawn_error: Some(err.to_string()),
        },
    }
}

fn probe_step_from_output(
    kind: ProbeStepKind,
    command: String,
    output: Output,
    elapsed_ms: u128,
) -> ProbeStep {
    let stdout = excerpt(&String::from_utf8_lossy(&output.stdout));
    let stderr = excerpt(&String::from_utf8_lossy(&output.stderr));
    ProbeStep {
        kind,
        command,
        success: output.status.success(),
        exit_code: output.status.code(),
        elapsed_ms,
        stdout: if stdout.is_empty() {
            None
        } else {
            Some(stdout)
        },
        stderr: if stderr.is_empty() {
            None
        } else {
            Some(stderr)
        },
        spawn_error: None,
    }
}

pub fn excerpt(text: &str) -> String {
    if text.len() <= EXCERPT_LIMIT {
        return text.to_owned();
    }
    let mut out = text[..EXCERPT_LIMIT].to_owned();
    out.push_str("…<truncated>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_docker_skips_probes() {
        let input = ProbeInput {
            image: "alpine:3".to_owned(),
            runner_os: RunnerOs::Linux,
            tools: vec!["sh".to_owned()],
            commands: vec!["echo hi".to_owned()],
            allow_pull: false,
            docker_bin: Some(Utf8PathBuf::from("/nonexistent/docker")),
        };
        let mut subject = probe(&input);
        subject.finalize();
        // spawn failure path produces docker_daemon_unreachable; that is OK.
        // The contract we care about is: probes are not silently green.
        assert!(
            subject
                .checks
                .iter()
                .any(|c| c.id == "probe.docker_daemon_unreachable"
                    || c.id == "probe.docker_cli_not_found")
        );
    }

    #[test]
    fn excerpt_truncates_long_text() {
        let big = "a".repeat(EXCERPT_LIMIT + 100);
        let trimmed = excerpt(&big);
        assert!(trimmed.ends_with("…<truncated>"));
        assert!(trimmed.len() < big.len());
    }
}
