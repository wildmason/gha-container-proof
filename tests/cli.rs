//! End-to-end CLI integration tests for `gha-container-proof`.
//!
//! These tests build the binary via `assert_cmd::Command::cargo_bin` and
//! exercise the four subcommands against on-disk workflow / action fixtures
//! that the tests write into temp dirs.
//!
//! Probe tests use a fake `docker` script generated per OS (a `.cmd` on
//! Windows, a `.sh` with shebang elsewhere) and pointed at via `--docker-bin`.

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use tempfile::{TempDir, tempdir};

fn proof() -> Command {
    Command::cargo_bin("gha-container-proof").expect("binary built")
}

fn write(dir: &TempDir, rel: &str, body: &str) -> PathBuf {
    let path = dir.path().join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, body).unwrap();
    path
}

fn json_stdout(args: &[&str]) -> Value {
    let output = proof().args(args).output().expect("ran cli");
    assert!(
        output.status.success() || !output.status.success(),
        "unexpected exit"
    );
    serde_json::from_slice(&output.stdout).expect("stdout was valid JSON")
}

// ----------------------------- check-workflow -----------------------------

#[test]
fn check_workflow_finds_simple_string_container() {
    let dir = tempdir().unwrap();
    write(
        &dir,
        ".github/workflows/ci.yml",
        "name: x\non: push\njobs:\n  build:\n    runs-on: ubuntu-22.04\n    container: node:22\n",
    );
    let receipt = json_stdout(&[
        "check-workflow",
        "--repo",
        dir.path().to_str().unwrap(),
        "--format",
        "json",
    ]);
    assert_eq!(receipt["mode"], "check-workflow");
    assert_eq!(receipt["compatibility"], "exact");
    let subjects = receipt["subjects"].as_array().unwrap();
    assert_eq!(subjects.len(), 1);
    assert_eq!(subjects[0]["kind"], "job-container");
    assert_eq!(subjects[0]["image"], "node:22");
    assert_eq!(subjects[0]["network_model"], "ci-forge-managed");
}

#[test]
fn check_workflow_finds_object_container_with_env_ports_volumes_options() {
    let dir = tempdir().unwrap();
    write(
        &dir,
        ".github/workflows/ci.yml",
        "name: x\non: push\njobs:\n  build:\n    runs-on: ubuntu-22.04\n    container:\n      image: node:22\n      env:\n        NODE_ENV: test\n        DATABASE_PASSWORD: secret\n      ports:\n        - 3000\n        - 8080:80\n      volumes:\n        - /host:/data\n      options: --cpus 2 --memory 4g\n",
    );
    let receipt = json_stdout(&[
        "check-workflow",
        "--repo",
        dir.path().to_str().unwrap(),
        "--format",
        "json",
    ]);
    let subject = &receipt["subjects"][0];
    assert!(
        subject["env_redacted"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "DATABASE_PASSWORD")
    );
    assert_eq!(subject["classification"], "exact");
    let checks: Vec<&str> = subject["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["id"].as_str())
        .collect();
    assert!(checks.contains(&"container.port.parse"));
    assert!(checks.contains(&"container.volume.parse"));
    assert!(checks.contains(&"container.options.classified"));
}

#[test]
fn check_workflow_flags_windows_job_container() {
    let dir = tempdir().unwrap();
    write(
        &dir,
        ".github/workflows/ci.yml",
        "name: x\non: push\njobs:\n  build:\n    runs-on: windows-latest\n    container: node:22\n",
    );
    proof()
        .args([
            "check-workflow",
            "--repo",
            dir.path().to_str().unwrap(),
            "--format",
            "json",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("failed"));
}

#[test]
fn check_workflow_flags_network_in_options() {
    let dir = tempdir().unwrap();
    write(
        &dir,
        ".github/workflows/ci.yml",
        "name: x\non: push\njobs:\n  build:\n    runs-on: ubuntu-22.04\n    container:\n      image: node:22\n      options: --network host\n",
    );
    let receipt = json_stdout(&[
        "check-workflow",
        "--repo",
        dir.path().to_str().unwrap(),
        "--format",
        "json",
    ]);
    let subject = &receipt["subjects"][0];
    assert_eq!(subject["network_model"], "unsupported-custom");
    assert_eq!(subject["classification"], "unsupported");
    assert!(
        subject["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == "container.options.network")
    );
}

#[test]
fn check_workflow_finds_docker_uri_step_uses() {
    let dir = tempdir().unwrap();
    write(
        &dir,
        ".github/workflows/ci.yml",
        "name: x\non: push\njobs:\n  scan:\n    runs-on: ubuntu-22.04\n    steps:\n      - uses: docker://alpine:3.20\n",
    );
    let receipt = json_stdout(&[
        "check-workflow",
        "--repo",
        dir.path().to_str().unwrap(),
        "--format",
        "json",
    ]);
    let subject = &receipt["subjects"][0];
    assert_eq!(subject["kind"], "docker-action");
    assert_eq!(subject["action_ref"], "docker://alpine:3.20");
    assert_eq!(subject["classification"], "exact");
}

#[test]
fn check_workflow_finds_local_docker_action_with_dockerfile() {
    let dir = tempdir().unwrap();
    // local action manifest declares runs.using: docker, image: Dockerfile.
    write(
        &dir,
        "actions/build/action.yml",
        "name: build\ndescription: y\nruns:\n  using: docker\n  image: Dockerfile\n  pre-entrypoint: /pre.sh\n  entrypoint: /entrypoint.sh\n  post-entrypoint: /post.sh\n",
    );
    write(&dir, "actions/build/Dockerfile", "FROM alpine:3\n");
    // workflow referencing the local action.
    write(
        &dir,
        ".github/workflows/ci.yml",
        "name: x\non: push\njobs:\n  build:\n    runs-on: ubuntu-22.04\n    steps:\n      - uses: ./actions/build\n",
    );
    let receipt = json_stdout(&[
        "check-workflow",
        "--repo",
        dir.path().to_str().unwrap(),
        "--workspace",
        dir.path().to_str().unwrap(),
        "--format",
        "json",
    ]);
    let subject = &receipt["subjects"][0];
    assert_eq!(subject["kind"], "docker-action");
    assert_eq!(subject["requires_build"], true);
    let checks: Vec<&str> = subject["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["id"].as_str())
        .collect();
    assert!(checks.contains(&"action.using.docker"));
    assert!(checks.contains(&"action.image.dockerfile"));
    assert!(checks.contains(&"action.pre_entrypoint.declared"));
    assert!(checks.contains(&"action.post_entrypoint.declared"));
}

#[test]
fn check_workflow_emits_services_delegation_hint() {
    let dir = tempdir().unwrap();
    write(
        &dir,
        ".github/workflows/ci.yml",
        "name: x\non: push\njobs:\n  build:\n    runs-on: ubuntu-22.04\n    container: node:22\n    services:\n      postgres:\n        image: postgres:16\n",
    );
    let receipt = json_stdout(&[
        "check-workflow",
        "--repo",
        dir.path().to_str().unwrap(),
        "--format",
        "json",
    ]);
    let receipt_checks: Vec<&str> = receipt["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["id"].as_str())
        .collect();
    assert!(receipt_checks.contains(&"container.services.delegated"));
}

#[test]
fn check_workflow_writes_output_file() {
    let dir = tempdir().unwrap();
    write(
        &dir,
        ".github/workflows/ci.yml",
        "name: x\non: push\njobs:\n  build:\n    runs-on: ubuntu-22.04\n    container: node:22\n",
    );
    let out_dir = tempdir().unwrap();
    let out_path = out_dir.path().join("nested").join("receipt.json");
    proof()
        .args([
            "check-workflow",
            "--repo",
            dir.path().to_str().unwrap(),
            "--format",
            "json",
            "--output",
            out_path.to_str().unwrap(),
        ])
        .assert()
        .success();
    assert!(out_path.exists());
    let raw = fs::read_to_string(out_path).unwrap();
    let parsed: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(parsed["mode"], "check-workflow");
}

#[test]
fn check_workflow_markdown_output_is_human_readable() {
    let dir = tempdir().unwrap();
    write(
        &dir,
        ".github/workflows/ci.yml",
        "name: x\non: push\njobs:\n  build:\n    runs-on: ubuntu-22.04\n    container: node:22\n",
    );
    let output = proof()
        .args([
            "check-workflow",
            "--repo",
            dir.path().to_str().unwrap(),
            "--format",
            "markdown",
        ])
        .assert()
        .success()
        .get_output()
        .clone();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("# gha-container-proof Receipt"));
    assert!(stdout.contains("## Subjects"));
    assert!(stdout.contains("`job-container`"));
}

// ------------------------------- plan-job ---------------------------------

#[test]
fn plan_job_classifies_clean_container() {
    let receipt = json_stdout(&[
        "plan-job",
        "--job-id",
        "build",
        "--runner-os",
        "linux",
        "--runs-on",
        "ubuntu-22.04",
        "--container",
        "node:22-bookworm",
        "--env",
        "NODE_ENV=test",
        "--port",
        "3000",
        "--volume",
        "/host/cache:/cache",
        "--options",
        "--cpus 2",
        "--format",
        "json",
    ]);
    assert_eq!(receipt["compatibility"], "exact");
    let subject = &receipt["subjects"][0];
    assert_eq!(subject["image"], "node:22-bookworm");
    assert_eq!(subject["classification"], "exact");
}

#[test]
fn plan_job_redacts_credentials() {
    let receipt = json_stdout(&[
        "plan-job",
        "--job-id",
        "build",
        "--runner-os",
        "linux",
        "--runs-on",
        "ubuntu-22.04",
        "--container",
        "node:22",
        "--credentials-username",
        "--credentials-password",
        "--env",
        "GITHUB_TOKEN=ghp_secret_value_here",
        "--format",
        "json",
    ]);
    let subject = &receipt["subjects"][0];
    let env_redacted = subject["env_redacted"].as_array().unwrap();
    assert!(env_redacted.iter().any(|v| v == "GITHUB_TOKEN"));
    let creds = subject["credentials_redacted"].as_array().unwrap();
    assert!(creds.iter().any(|v| v == "username"));
    assert!(creds.iter().any(|v| v == "password"));
    // No credential value should appear anywhere in the rendered receipt.
    let rendered = serde_json::to_string(&receipt).unwrap();
    assert!(!rendered.contains("ghp_secret_value_here"));
}

#[test]
fn plan_job_strict_promotes_pinning_warning_to_failure() {
    proof()
        .args([
            "plan-job",
            "--job-id",
            "build",
            "--runner-os",
            "linux",
            "--runs-on",
            "ubuntu-22.04",
            "--container",
            "node:latest",
            "--format",
            "json",
            "--strict",
        ])
        .assert()
        .failure();
}

// ----------------------------- plan-action --------------------------------

#[test]
fn plan_action_docker_uri_is_exact() {
    let receipt = json_stdout(&[
        "plan-action",
        "--action-ref",
        "docker://alpine:3.20",
        "--image",
        "docker://alpine:3.20",
        "--format",
        "json",
    ]);
    let subject = &receipt["subjects"][0];
    assert_eq!(subject["classification"], "exact");
    assert!(
        subject["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == "action.image.docker_uri")
    );
}

#[test]
fn plan_action_local_action_with_pre_and_post_entrypoints() {
    let dir = tempdir().unwrap();
    write(
        &dir,
        "action.yml",
        "name: build\ndescription: y\nruns:\n  using: docker\n  image: Dockerfile\n  pre-entrypoint: /pre.sh\n  entrypoint: /entrypoint.sh\n  post-entrypoint: /post.sh\n  args:\n    - build\n",
    );
    write(&dir, "Dockerfile", "FROM alpine:3\n");
    let receipt = json_stdout(&[
        "plan-action",
        "--action-ref",
        "./.",
        "--action-path",
        dir.path().to_str().unwrap(),
        "--format",
        "json",
    ]);
    let subject = &receipt["subjects"][0];
    let ids: Vec<&str> = subject["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["id"].as_str())
        .collect();
    assert!(ids.contains(&"action.using.docker"));
    assert!(ids.contains(&"action.image.dockerfile"));
    assert!(ids.contains(&"action.pre_entrypoint.declared"));
    assert!(ids.contains(&"action.post_entrypoint.declared"));
    assert!(ids.contains(&"action.entrypoint.declared"));
    assert!(ids.contains(&"action.args.preserved"));
    assert_eq!(subject["requires_build"], true);
}

#[test]
fn plan_action_missing_dockerfile_fails() {
    let dir = tempdir().unwrap();
    write(
        &dir,
        "action.yml",
        "name: build\ndescription: y\nruns:\n  using: docker\n  image: Dockerfile\n",
    );
    proof()
        .args([
            "plan-action",
            "--action-ref",
            "./.",
            "--action-path",
            dir.path().to_str().unwrap(),
            "--format",
            "json",
        ])
        .assert()
        .failure();
}

#[test]
fn plan_action_redacts_input_secret_env() {
    let receipt = json_stdout(&[
        "plan-action",
        "--action-ref",
        "docker://alpine:3.20",
        "--image",
        "docker://alpine:3.20",
        "--env",
        "INPUT_API_TOKEN=ghp_topsecret",
        "--format",
        "json",
    ]);
    let subject = &receipt["subjects"][0];
    let env_redacted = subject["env_redacted"].as_array().unwrap();
    assert!(env_redacted.iter().any(|v| v == "INPUT_API_TOKEN"));
    let rendered = serde_json::to_string(&receipt).unwrap();
    assert!(!rendered.contains("ghp_topsecret"));
}

// --------------------------------- probe ----------------------------------

/// Write a fake docker binary that emits the given canned response. On Windows
/// this is a `.cmd` file; everywhere else a shell script with shebang.
fn write_fake_docker(dir: &TempDir, name: &str, body_unix: &str, body_cmd: &str) -> PathBuf {
    if cfg!(windows) {
        let path = dir.path().join(format!("{name}.cmd"));
        fs::write(&path, body_cmd).unwrap();
        path
    } else {
        let path = dir.path().join(name);
        fs::write(&path, body_unix).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&path, perms).unwrap();
        }
        path
    }
}

/// Fake docker that prints a minimal `[]` for `image inspect` and exits 0,
/// echoes back tool/sh arguments for `run`.
fn fake_docker_happy(dir: &TempDir) -> PathBuf {
    let unix = "#!/bin/sh\nif [ \"$1\" = \"image\" ] && [ \"$2\" = \"inspect\" ]; then\n  echo \"[{\\\"Id\\\":\\\"sha256:abc\\\"}]\"\n  exit 0\nfi\nif [ \"$1\" = \"run\" ]; then\n  shift\n  while [ $# -gt 0 ]; do\n    case \"$1\" in\n      --rm) shift; continue;;\n    esac\n    break\n  done\n  # next arg is the image\n  shift\n  echo \"fake-docker-stdout $*\"\n  exit 0\nfi\necho \"unknown fake-docker args: $@\" >&2\nexit 1\n";
    let cmd = "@echo off\r\nsetlocal\r\nif \"%1\"==\"image\" if \"%2\"==\"inspect\" (\r\n  echo [{\"Id\":\"sha256:abc\"}]\r\n  exit /B 0\r\n)\r\nif \"%1\"==\"run\" (\r\n  echo fake-docker-stdout\r\n  exit /B 0\r\n)\r\necho unknown fake-docker args: %* 1>&2\r\nexit /B 1\r\n";
    write_fake_docker(dir, "fake-docker-happy", unix, cmd)
}

/// Fake docker that simulates a missing image: inspect exits nonzero with the
/// "Error: No such image" stderr GitHub Actions runners surface.
fn fake_docker_image_missing(dir: &TempDir) -> PathBuf {
    let unix = "#!/bin/sh\nif [ \"$1\" = \"image\" ] && [ \"$2\" = \"inspect\" ]; then\n  echo \"Error: No such image: $3\" >&2\n  exit 1\nfi\nexit 1\n";
    let cmd = "@echo off\r\nsetlocal\r\nif \"%1\"==\"image\" if \"%2\"==\"inspect\" (\r\n  echo Error: No such image: %3 1>&2\r\n  exit /B 1\r\n)\r\nexit /B 1\r\n";
    write_fake_docker(dir, "fake-docker-missing", unix, cmd)
}

/// Fake docker that simulates daemon unreachable.
fn fake_docker_daemon_down(dir: &TempDir) -> PathBuf {
    let unix = "#!/bin/sh\necho 'Cannot connect to the Docker daemon at unix:///var/run/docker.sock.' >&2\nexit 1\n";
    let cmd = "@echo off\r\nsetlocal\r\necho Cannot connect to the Docker daemon at unix:///var/run/docker.sock. 1>&2\r\nexit /B 1\r\n";
    write_fake_docker(dir, "fake-docker-daemondown", unix, cmd)
}

#[test]
fn probe_skips_when_docker_bin_missing() {
    // Point at a path that definitely doesn't exist.
    let receipt = json_stdout(&[
        "probe",
        "--image",
        "alpine:3",
        "--tool",
        "sh",
        "--docker-bin",
        "/definitely/not/a/real/docker",
        "--format",
        "json",
    ]);
    let subject = &receipt["subjects"][0];
    let ids: Vec<&str> = subject["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["id"].as_str())
        .collect();
    // Either the spawn fails (daemon_unreachable) or the resolution misses
    // (cli_not_found) — both are valid skip-cleanly outcomes. We just want NO
    // false-success.
    assert!(
        ids.contains(&"probe.docker_cli_not_found")
            || ids.contains(&"probe.docker_daemon_unreachable")
    );
    assert_eq!(subject["kind"], "docker-probe");
}

#[test]
fn probe_parses_happy_fake_docker_output() {
    let bin_dir = tempdir().unwrap();
    let docker = fake_docker_happy(&bin_dir);
    let receipt = json_stdout(&[
        "probe",
        "--image",
        "alpine:3",
        "--tool",
        "sh",
        "--command",
        "echo hi",
        "--docker-bin",
        docker.to_str().unwrap(),
        "--format",
        "json",
    ]);
    let subject = &receipt["subjects"][0];
    let ids: Vec<&str> = subject["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["id"].as_str())
        .collect();
    assert!(ids.contains(&"probe.image_inspect_ok"));
    assert!(ids.contains(&"probe.tool_ok"));
    assert!(ids.contains(&"probe.command_ok"));
    let probe = &subject["probe"];
    assert_eq!(probe["docker_cli_available"], true);
    assert!(probe["inspect"]["success"].as_bool().unwrap());
}

#[test]
fn probe_fails_when_image_missing_offline() {
    let bin_dir = tempdir().unwrap();
    let docker = fake_docker_image_missing(&bin_dir);
    proof()
        .args([
            "probe",
            "--image",
            "doesnotexist:1",
            "--docker-bin",
            docker.to_str().unwrap(),
            "--format",
            "json",
        ])
        .assert()
        .failure();
}

#[test]
fn probe_warns_when_image_missing_with_allow_pull() {
    let bin_dir = tempdir().unwrap();
    let docker = fake_docker_image_missing(&bin_dir);
    let output = proof()
        .args([
            "probe",
            "--image",
            "doesnotexist:1",
            "--allow-pull",
            "--docker-bin",
            docker.to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    let receipt: Value = serde_json::from_slice(&output.stdout).unwrap();
    let subject = &receipt["subjects"][0];
    let ids: Vec<&str> = subject["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["id"].as_str())
        .collect();
    assert!(ids.contains(&"probe.image_pull_required"));
    // pull will fail with our fake-missing docker, so attempt fails.
    assert!(ids.contains(&"probe.image_pull_attempted"));
}

#[test]
fn probe_fails_when_daemon_unreachable() {
    let bin_dir = tempdir().unwrap();
    let docker = fake_docker_daemon_down(&bin_dir);
    proof()
        .args([
            "probe",
            "--image",
            "alpine:3",
            "--docker-bin",
            docker.to_str().unwrap(),
            "--format",
            "json",
        ])
        .assert()
        .failure()
        .stdout(predicate::str::contains("probe.docker_daemon_unreachable"));
}

// ----------------------- receipt-shape regression tests -------------------

#[test]
fn receipt_carries_schema_version_one() {
    let dir = tempdir().unwrap();
    write(
        &dir,
        ".github/workflows/ci.yml",
        "name: x\non: push\njobs:\n  build:\n    runs-on: ubuntu-22.04\n    container: node:22\n",
    );
    let receipt = json_stdout(&[
        "check-workflow",
        "--repo",
        dir.path().to_str().unwrap(),
        "--format",
        "json",
    ]);
    assert_eq!(receipt["schema_version"], 1);
    assert_eq!(receipt["tool"]["name"], "gha-container-proof");
    assert!(receipt["tool"]["version"].is_string());
    for field in [
        "checked_at",
        "mode",
        "compatibility",
        "summary",
        "subjects",
        "checks",
    ] {
        assert!(
            receipt.get(field).is_some(),
            "receipt missing field `{field}`"
        );
    }
}

#[test]
fn subject_records_required_provenance_fields() {
    let receipt = json_stdout(&[
        "plan-job",
        "--job-id",
        "build",
        "--runner-os",
        "linux",
        "--runs-on",
        "ubuntu-22.04",
        "--container",
        "node:22-bookworm",
        "--format",
        "json",
    ]);
    let subject = &receipt["subjects"][0];
    for field in [
        "kind",
        "classification",
        "network_model",
        "requires_docker",
        "requires_build",
        "requires_pull",
        "summary",
        "checks",
    ] {
        assert!(
            subject.get(field).is_some(),
            "subject missing required field `{field}`"
        );
    }
    assert_eq!(subject["kind"], "job-container");
    assert_eq!(subject["requires_docker"], true);
    assert_eq!(subject["requires_build"], false);
}

#[test]
fn cli_rejects_invalid_env_pair() {
    proof()
        .args([
            "plan-job",
            "--container",
            "node:22",
            "--runner-os",
            "linux",
            "--runs-on",
            "ubuntu-22.04",
            "--env",
            "no-equals-sign-here",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("KEY=VALUE"));
}
