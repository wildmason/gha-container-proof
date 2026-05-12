//! Workflow YAML scanner for `check-workflow`.
//!
//! Walks `.github/workflows/*.yml`, finds:
//!
//! - `jobs.<job_id>.container` (string or object),
//! - `steps[*].uses: docker://...`,
//! - `steps[*].uses: ./local-action` whose `action.yml` declares
//!   `runs.using: docker`.
//!
//! Emits one [`Subject`] per surface plus receipt-level rollup checks.

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use serde_yaml::{Mapping, Value as YamlValue};
use std::fs;

use crate::model::{Check, Subject};
use crate::plan::{ActionPlanInput, JobPlanInput, plan_action, plan_job};

#[derive(Debug, Clone)]
pub struct CheckWorkflowOptions {
    pub repo_root: Utf8PathBuf,
    pub workspace: Utf8PathBuf,
    pub workflows: Vec<Utf8PathBuf>,
    pub runner_os: crate::model::RunnerOs,
}

#[derive(Debug, Clone, Default)]
pub struct ScanResult {
    pub subjects: Vec<Subject>,
    pub checks: Vec<Check>,
}

pub fn scan_workflows(options: &CheckWorkflowOptions) -> Result<ScanResult> {
    let workflows = discover_workflows(options)?;
    let mut result = ScanResult::default();

    if workflows.is_empty() {
        result.checks.push(Check::skip(
            "container.workflow.read",
            format!(
                "no workflows discovered under {} or via --workflow",
                options.repo_root.join(".github/workflows")
            ),
        ));
        return Ok(result);
    }

    for workflow in workflows {
        scan_workflow(options, &workflow, &mut result)?;
    }

    Ok(result)
}

fn discover_workflows(options: &CheckWorkflowOptions) -> Result<Vec<Utf8PathBuf>> {
    if !options.workflows.is_empty() {
        return Ok(options.workflows.clone());
    }

    let dir = options.repo_root.join(".github").join("workflows");
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut workflows = Vec::new();
    for item in fs::read_dir(&dir).with_context(|| format!("reading workflows dir {dir}"))? {
        let entry = item?;
        let path = Utf8PathBuf::from_path_buf(entry.path())
            .map_err(|_| anyhow::anyhow!("non-UTF-8 workflow path"))?;
        if matches!(path.extension(), Some("yml" | "yaml")) {
            workflows.push(path);
        }
    }
    workflows.sort();
    Ok(workflows)
}

fn scan_workflow(
    options: &CheckWorkflowOptions,
    workflow: &Utf8Path,
    result: &mut ScanResult,
) -> Result<()> {
    let raw =
        fs::read_to_string(workflow).with_context(|| format!("reading workflow {workflow}"))?;
    let yaml: YamlValue =
        serde_yaml::from_str(&raw).with_context(|| format!("parsing workflow {workflow}"))?;

    result.checks.push(
        Check::pass(
            "container.workflow.read",
            format!("workflow `{workflow}` parsed"),
        )
        .at(workflow.to_string()),
    );

    let Some(jobs) = yaml.get("jobs").and_then(YamlValue::as_mapping) else {
        result.checks.push(
            Check::skip(
                "container.workflow.jobs_missing",
                format!("workflow `{workflow}` has no jobs mapping"),
            )
            .at(workflow.to_string()),
        );
        return Ok(());
    };

    let mut produced_any = false;

    for (job_key, job_value) in jobs {
        let job_id = yaml_string(job_key).unwrap_or_else(|| "<unknown>".to_owned());
        let Some(job_mapping) = job_value.as_mapping() else {
            result.checks.push(
                Check::fail("container.workflow.job", "job value must be a mapping")
                    .at(format!("{workflow}:jobs.{job_id}")),
            );
            continue;
        };

        // Job container.
        if let Some(container_value) = get_in(job_mapping, "container") {
            let job_input = job_input_from_container(
                options,
                &job_id,
                container_value,
                runs_on(job_mapping),
                format!("{workflow}:jobs.{job_id}.container"),
            );
            let mut subject = plan_job(&job_input);
            subject.finalize();
            result.subjects.push(subject);
            produced_any = true;
        }

        // Service-container delegation hint.
        if job_mapping
            .get(YamlValue::String("services".to_owned()))
            .and_then(YamlValue::as_mapping)
            .is_some_and(|map| !map.is_empty())
        {
            result.checks.push(
                Check::pass(
                    "container.services.delegated",
                    format!(
                        "job `{job_id}` also declares `services`; delegate to gha-service-proof"
                    ),
                )
                .at(format!("{workflow}:jobs.{job_id}.services")),
            );
        }

        // Steps: docker:// and local action manifests.
        if let Some(steps) = job_mapping
            .get(YamlValue::String("steps".to_owned()))
            .and_then(YamlValue::as_sequence)
        {
            for (index, step) in steps.iter().enumerate() {
                let Some(step_map) = step.as_mapping() else {
                    continue;
                };
                let Some(uses) = string_in(step_map, "uses") else {
                    continue;
                };
                let step_id =
                    string_in(step_map, "id").unwrap_or_else(|| format!("step-{}", index + 1));
                let location = format!("{workflow}:jobs.{job_id}.steps.{step_id}.uses");

                if let Some(image) = uses.strip_prefix("docker://") {
                    let action_input = ActionPlanInput {
                        action_ref: uses.clone(),
                        step_id: Some(step_id),
                        action_path: None,
                        using: Some("docker".to_owned()),
                        image: Some(format!("docker://{image}")),
                        entrypoint: None,
                        pre_entrypoint: None,
                        post_entrypoint: None,
                        args: Vec::new(),
                        env: Vec::new(),
                        location: Some(location),
                    };
                    let mut subject = plan_action(&action_input);
                    subject.finalize();
                    result.subjects.push(subject);
                    produced_any = true;
                } else if uses.starts_with("./") {
                    let action_dir = options.workspace.join(uses.trim_start_matches("./"));
                    let manifest_path = pick_action_file(&action_dir);
                    let action_input = ActionPlanInput {
                        action_ref: uses.clone(),
                        step_id: Some(step_id),
                        action_path: manifest_path.clone(),
                        using: None,
                        image: None,
                        entrypoint: None,
                        pre_entrypoint: None,
                        post_entrypoint: None,
                        args: Vec::new(),
                        env: Vec::new(),
                        location: Some(location),
                    };
                    let mut subject = plan_action(&action_input);
                    // Only count this as a container subject if the action is
                    // actually Docker-flavored. Composite/Node actions are
                    // surfaced as "unsupported" (info only) and dropped here so
                    // the rollup stays focused.
                    let is_docker_action = subject.checks.iter().any(|check| {
                        check.id == "action.using.docker"
                            || check.id == "action.image.docker_uri"
                            || check.id == "action.image.dockerfile"
                            || check.id == "action.image.dockerfile_missing"
                    });
                    if is_docker_action || manifest_path.is_none() {
                        subject.finalize();
                        result.subjects.push(subject);
                        produced_any = true;
                    }
                }
                // Remote `owner/repo@ref` action references are intentionally
                // skipped here; check-workflow is local-first. Use
                // `plan-action` with --action-path on a mirrored manifest.
            }
        }
    }

    if !produced_any {
        result.checks.push(
            Check::skip(
                "container.workflow.no_container",
                format!(
                    "workflow `{workflow}` defines no job containers, docker:// uses, or local Docker actions"
                ),
            )
            .at(workflow.to_string()),
        );
    } else {
        result.checks.push(
            Check::pass(
                "container.workflow.subjects",
                format!(
                    "workflow `{workflow}` produced {} container subject(s)",
                    result
                        .subjects
                        .iter()
                        .filter(|subject| subject.checks.iter().any(|c| c
                            .location
                            .as_deref()
                            .is_some_and(|loc| loc.starts_with(workflow.as_str()))))
                        .count()
                ),
            )
            .at(workflow.to_string()),
        );
    }

    Ok(())
}

fn job_input_from_container(
    options: &CheckWorkflowOptions,
    job_id: &str,
    container: &YamlValue,
    runs_on: Vec<String>,
    location: String,
) -> JobPlanInput {
    let mut input = JobPlanInput {
        job_id: job_id.to_owned(),
        runner_os: options.runner_os,
        runs_on,
        container_image: None,
        env: Vec::new(),
        ports: Vec::new(),
        volumes: Vec::new(),
        options: String::new(),
        credentials_username_present: false,
        credentials_password_present: false,
        location: Some(location),
    };

    if let Some(image) = yaml_string(container) {
        input.container_image = Some(image);
        return input;
    }

    let Some(mapping) = container.as_mapping() else {
        return input;
    };

    input.container_image = string_in(mapping, "image");
    input.env = string_map(mapping, "env");
    input.ports = string_list(mapping, "ports");
    input.volumes = string_list(mapping, "volumes");
    input.options = string_in(mapping, "options").unwrap_or_default();

    if let Some(credentials) = mapping
        .get(YamlValue::String("credentials".to_owned()))
        .and_then(YamlValue::as_mapping)
    {
        input.credentials_username_present = string_in(credentials, "username")
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
        input.credentials_password_present = string_in(credentials, "password")
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
    }

    input
}

fn runs_on(job_mapping: &Mapping) -> Vec<String> {
    let Some(value) = job_mapping.get(YamlValue::String("runs-on".to_owned())) else {
        return Vec::new();
    };
    list_from_value(value)
}

fn pick_action_file(dir: &Utf8Path) -> Option<Utf8PathBuf> {
    let yml = dir.join("action.yml");
    if yml.exists() {
        return Some(yml);
    }
    let yaml = dir.join("action.yaml");
    if yaml.exists() {
        return Some(yaml);
    }
    None
}

fn get_in<'a>(mapping: &'a Mapping, key: &str) -> Option<&'a YamlValue> {
    mapping.get(YamlValue::String(key.to_owned()))
}

fn string_in(mapping: &Mapping, key: &str) -> Option<String> {
    get_in(mapping, key).and_then(yaml_string)
}

fn string_list(mapping: &Mapping, key: &str) -> Vec<String> {
    get_in(mapping, key)
        .map(list_from_value)
        .unwrap_or_default()
}

fn list_from_value(value: &YamlValue) -> Vec<String> {
    match value {
        YamlValue::Sequence(items) => items.iter().filter_map(yaml_string).collect(),
        _ => yaml_string(value).into_iter().collect(),
    }
}

fn string_map(mapping: &Mapping, key: &str) -> Vec<(String, String)> {
    let Some(inner) = get_in(mapping, key).and_then(YamlValue::as_mapping) else {
        return Vec::new();
    };
    inner
        .iter()
        .filter_map(|(k, v)| match (yaml_string(k), yaml_string(v)) {
            (Some(k), Some(v)) => Some((k, v)),
            _ => None,
        })
        .collect()
}

fn yaml_string(value: &YamlValue) -> Option<String> {
    match value {
        YamlValue::String(value) => Some(value.clone()),
        YamlValue::Number(value) => Some(value.to_string()),
        YamlValue::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Compatibility, RunnerOs};
    use tempfile::tempdir;

    fn write_workflow(dir: &Utf8Path, yaml: &str) -> Utf8PathBuf {
        let path = dir.join(".github").join("workflows").join("ci.yml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, yaml).unwrap();
        path
    }

    fn options_for(dir: &Utf8Path) -> CheckWorkflowOptions {
        CheckWorkflowOptions {
            repo_root: dir.to_owned(),
            workspace: dir.to_owned(),
            workflows: Vec::new(),
            runner_os: RunnerOs::Linux,
        }
    }

    #[test]
    fn finds_simple_string_container() {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        write_workflow(
            &root,
            "name: x\non: push\njobs:\n  build:\n    runs-on: ubuntu-22.04\n    container: node:22\n    steps:\n      - run: echo\n",
        );
        let result = scan_workflows(&options_for(&root)).unwrap();
        assert_eq!(result.subjects.len(), 1);
        assert_eq!(result.subjects[0].image.as_deref(), Some("node:22"));
    }

    #[test]
    fn finds_object_container_with_options_and_credentials() {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        write_workflow(
            &root,
            "name: x\non: push\njobs:\n  build:\n    runs-on: ubuntu-22.04\n    container:\n      image: node:22\n      credentials:\n        username: u\n        password: p\n      ports:\n        - 3000\n      volumes:\n        - /host:/cache\n      options: --cpus 2\n",
        );
        let result = scan_workflows(&options_for(&root)).unwrap();
        let subject = &result.subjects[0];
        assert_eq!(subject.image.as_deref(), Some("node:22"));
        assert!(
            subject
                .credentials_redacted
                .contains(&"password".to_owned())
        );
        assert!(
            subject
                .checks
                .iter()
                .any(|c| c.id == "container.port.parse")
        );
    }

    #[test]
    fn flags_windows_runs_on() {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        write_workflow(
            &root,
            "name: x\non: push\njobs:\n  build:\n    runs-on: windows-latest\n    container: node:22\n",
        );
        let result = scan_workflows(&options_for(&root)).unwrap();
        let subject = &result.subjects[0];
        assert_eq!(subject.classification, Compatibility::Unsupported);
        assert!(
            subject
                .checks
                .iter()
                .any(|c| c.id == "container.runs_on.linux"
                    && c.status == crate::model::CheckStatus::Fail)
        );
    }

    #[test]
    fn flags_network_in_options() {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        write_workflow(
            &root,
            "name: x\non: push\njobs:\n  build:\n    runs-on: ubuntu-22.04\n    container:\n      image: node:22\n      options: --network host\n",
        );
        let result = scan_workflows(&options_for(&root)).unwrap();
        let subject = &result.subjects[0];
        assert!(
            subject
                .checks
                .iter()
                .any(|c| c.id == "container.options.network")
        );
    }

    #[test]
    fn finds_docker_uses() {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        write_workflow(
            &root,
            "name: x\non: push\njobs:\n  scan:\n    runs-on: ubuntu-22.04\n    steps:\n      - uses: docker://alpine:3\n",
        );
        let result = scan_workflows(&options_for(&root)).unwrap();
        assert_eq!(result.subjects.len(), 1);
        assert_eq!(
            result.subjects[0].action_ref.as_deref(),
            Some("docker://alpine:3")
        );
    }

    #[test]
    fn finds_services_and_emits_delegation_check() {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        write_workflow(
            &root,
            "name: x\non: push\njobs:\n  build:\n    runs-on: ubuntu-22.04\n    container: node:22\n    services:\n      postgres:\n        image: postgres:16\n",
        );
        let result = scan_workflows(&options_for(&root)).unwrap();
        assert!(
            result
                .checks
                .iter()
                .any(|c| c.id == "container.services.delegated")
        );
    }
}
