//! Classifiers for concrete job-container and Docker-action requests.

use camino::{Utf8Path, Utf8PathBuf};

use crate::action::{ActionManifest, DockerImage, classify_image};
use crate::model::{
    Check, Compatibility, NetworkModel, RunnerOs, Subject, SubjectKind, is_sensitive_key,
};
use crate::options::{
    OptionsPlan, apply_options_to_subject, looks_like_windows_host_path, parse_options,
};

/// Concrete plan-job inputs.
#[derive(Debug, Clone)]
pub struct JobPlanInput {
    pub job_id: String,
    pub runner_os: RunnerOs,
    pub runs_on: Vec<String>,
    pub container_image: Option<String>,
    pub env: Vec<(String, String)>,
    pub ports: Vec<String>,
    pub volumes: Vec<String>,
    pub options: String,
    pub credentials_username_present: bool,
    pub credentials_password_present: bool,
    /// Optional source location, used for `check-workflow` to point at the
    /// originating workflow path.
    pub location: Option<String>,
}

/// Concrete plan-action inputs.
#[derive(Debug, Clone)]
pub struct ActionPlanInput {
    pub action_ref: String,
    pub step_id: Option<String>,
    pub action_path: Option<Utf8PathBuf>,
    pub using: Option<String>,
    pub image: Option<String>,
    pub entrypoint: Option<String>,
    pub pre_entrypoint: Option<String>,
    pub post_entrypoint: Option<String>,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub location: Option<String>,
}

/// Classify a job container.
pub fn plan_job(input: &JobPlanInput) -> Subject {
    let mut subject = Subject::new(SubjectKind::JobContainer);
    subject.job_id = Some(input.job_id.clone());
    subject.image = input.container_image.clone();
    subject.runner_os = Some(input.runner_os);
    subject.requires_docker = true;
    subject.network_model = NetworkModel::CiForgeManaged;
    let location = input.location.clone();

    let image = input.container_image.as_deref().unwrap_or("").trim();
    if image.is_empty() {
        subject.push(at(
            &location,
            Check::fail(
                "container.image.declared",
                "job declares a container with no image",
            ),
        ));
    } else {
        subject.push(at(
            &location,
            Check::pass(
                "container.image.declared",
                format!("job container image is `{image}`"),
            ),
        ));
        if image.contains("${{") {
            subject.classification = Compatibility::Simulated;
            subject.push(at(
                &location,
                Check::warn(
                    "container.image.expression",
                    format!("image `{image}` contains an unrendered expression"),
                ),
            ));
        } else {
            subject.push(image_pin_check(image).map_location(location.clone()));
        }
    }

    push_runner_os_checks(&mut subject, input, &location);
    push_runs_on_checks(&mut subject, input, &location);
    push_credentials_checks(&mut subject, input, &location);
    push_env_checks(&mut subject, &input.env, &location);
    push_port_checks(&mut subject, &input.ports, &location);
    push_volume_checks(&mut subject, &input.volumes, &location);

    match parse_options(&input.options) {
        Ok(plan) => apply_options_plan(&mut subject, &plan, &location),
        Err(message) => subject.push(at(
            &location,
            Check::fail("container.options.parse", message),
        )),
    }

    subject
}

/// Classify a Docker action invocation.
pub fn plan_action(input: &ActionPlanInput) -> Subject {
    let mut subject = Subject::new(SubjectKind::DockerAction);
    subject.action_ref = Some(input.action_ref.clone());
    subject.step_id = input.step_id.clone();
    subject.network_model = NetworkModel::CiForgeManaged;
    subject.requires_docker = true;
    let location = input.location.clone();

    // Try to load the manifest from a local path; fall back to the inline inputs.
    let manifest = input
        .action_path
        .as_ref()
        .map(|path| ActionManifest::read(path));

    let resolved_manifest = match manifest {
        Some(Ok(manifest)) => {
            subject.push(at(
                &location,
                Check::pass(
                    "action.manifest.read",
                    format!("read action manifest at `{}`", manifest.source_path),
                ),
            ));
            Some(manifest)
        }
        Some(Err(err)) => {
            subject.classification = Compatibility::Simulated;
            subject.push(at(
                &location,
                Check::warn(
                    "action.manifest.unavailable",
                    format!("action manifest could not be loaded: {err:#}"),
                ),
            ));
            None
        }
        None => {
            if input.action_ref.starts_with("docker://") {
                subject.push(at(
                    &location,
                    Check::pass(
                        "action.manifest.read",
                        "action reference is a docker:// URI; no manifest required",
                    ),
                ));
            } else if input.action_ref.starts_with("./") || input.action_ref.starts_with('/') {
                subject.classification = Compatibility::Simulated;
                subject.push(at(
                    &location,
                    Check::warn(
                        "action.manifest.unavailable",
                        "local action reference but no --action-path provided; manifest not loaded",
                    ),
                ));
            } else {
                subject.classification = Compatibility::Simulated;
                subject.push(at(
                    &location,
                    Check::warn(
                        "action.manifest.unavailable",
                        format!(
                            "remote action `{}` requires a mirrored manifest path for full classification",
                            input.action_ref
                        ),
                    ),
                ));
            }
            None
        }
    };

    // Merge manifest fields with CLI-supplied overrides; CLI wins where set.
    let using = input
        .using
        .clone()
        .or_else(|| resolved_manifest.as_ref().and_then(|m| m.using.clone()));
    let image_raw = input
        .image
        .clone()
        .or_else(|| resolved_manifest.as_ref().and_then(|m| m.image.clone()));
    let entrypoint = input.entrypoint.clone().or_else(|| {
        resolved_manifest
            .as_ref()
            .and_then(|m| m.entrypoint.clone())
    });
    let pre_entrypoint = input.pre_entrypoint.clone().or_else(|| {
        resolved_manifest
            .as_ref()
            .and_then(|m| m.pre_entrypoint.clone())
    });
    let post_entrypoint = input.post_entrypoint.clone().or_else(|| {
        resolved_manifest
            .as_ref()
            .and_then(|m| m.post_entrypoint.clone())
    });
    let args = if input.args.is_empty() {
        resolved_manifest
            .as_ref()
            .map(|m| m.args.clone())
            .unwrap_or_default()
    } else {
        input.args.clone()
    };
    let env = if input.env.is_empty() {
        resolved_manifest
            .as_ref()
            .map(|m| m.env.clone())
            .unwrap_or_default()
    } else {
        input.env.clone()
    };

    // Quick docker:// fast path uses the raw ref when image was not set. The
    // `--action-path` argument may be a directory or the manifest file itself
    // (e.g. when workflow.rs has already resolved to action.yml); resolve to
    // the containing directory so relative paths like `Dockerfile` work.
    let action_dir = input.action_path.as_deref().map(|p| {
        if p.is_file() {
            p.parent().unwrap_or(p)
        } else {
            p
        }
    });
    let inferred_image = image_raw.clone().or_else(|| {
        if input.action_ref.starts_with("docker://") {
            Some(input.action_ref.clone())
        } else {
            None
        }
    });

    // --using validation.
    match using.as_deref() {
        Some("docker") | Some("Docker") => {
            subject.push(at(
                &location,
                Check::pass("action.using.docker", "action uses `runs.using: docker`"),
            ));
        }
        Some(other) => {
            subject.push(at(
                &location,
                Check::fail(
                    "action.using.unsupported",
                    format!(
                        "action uses `runs.using: {other}`; gha-container-proof only classifies Docker actions"
                    ),
                ),
            ));
        }
        None if input.action_ref.starts_with("docker://") => {
            subject.push(at(
                &location,
                Check::pass(
                    "action.using.docker",
                    "docker:// step has implicit `runs.using: docker`",
                ),
            ));
        }
        None => {
            subject.push(at(
                &location,
                Check::warn(
                    "action.using.unsupported",
                    "no `runs.using` declared and no docker:// shortcut",
                ),
            ));
        }
    }

    // Image classification.
    classify_action_image(
        &mut subject,
        inferred_image.as_deref(),
        action_dir,
        &location,
    );

    if let Some(value) = &entrypoint {
        subject.push(at(
            &location,
            Check::pass(
                "action.entrypoint.declared",
                format!("entrypoint: `{value}`"),
            ),
        ));
    }
    if let Some(value) = &pre_entrypoint {
        subject.push(at(
            &location,
            Check::pass(
                "action.pre_entrypoint.declared",
                format!("pre-entrypoint: `{value}` (requires pre-job hook)"),
            ),
        ));
    }
    if let Some(value) = &post_entrypoint {
        subject.push(at(
            &location,
            Check::pass(
                "action.post_entrypoint.declared",
                format!("post-entrypoint: `{value}` (requires post-job hook)"),
            ),
        ));
    }
    if !args.is_empty() {
        subject.push(at(
            &location,
            Check::pass(
                "action.args.preserved",
                format!("preserved {} arg(s)", args.len()),
            ),
        ));
    }

    push_env_checks(&mut subject, &env, &location);

    subject
}

fn classify_action_image(
    subject: &mut Subject,
    image_raw: Option<&str>,
    action_dir: Option<&Utf8Path>,
    location: &Option<String>,
) {
    let classification = classify_image(image_raw, action_dir);
    match classification {
        DockerImage::DockerUri(image) => {
            subject.image = Some(image.clone());
            subject.push(at(
                location,
                Check::pass(
                    "action.image.docker_uri",
                    format!("image is `docker://{image}`"),
                ),
            ));
            // Pin check: unless pinned, classification stays compatible.
            if !image.contains("@sha256:") {
                if let Some((_, tag)) = image.rsplit_once(':') {
                    if tag.eq_ignore_ascii_case("latest") {
                        subject.push(at(
                            location,
                            Check::warn(
                                "container.image.pin",
                                format!("image `{image}` uses `latest`; pin by tag or digest"),
                            ),
                        ));
                    }
                } else {
                    subject.push(at(
                        location,
                        Check::warn(
                            "container.image.pin",
                            format!("image `{image}` has no tag; defaulting to `latest`"),
                        ),
                    ));
                }
            }
        }
        DockerImage::Dockerfile(path) => {
            subject.dockerfile = Some(path.to_string());
            subject.requires_build = true;
            if path.exists() {
                subject.push(at(
                    location,
                    Check::pass(
                        "action.image.dockerfile",
                        format!("Dockerfile available at `{path}` (build required)"),
                    ),
                ));
            } else {
                subject.push(at(
                    location,
                    Check::fail(
                        "action.image.dockerfile_missing",
                        format!("Dockerfile `{path}` does not exist"),
                    ),
                ));
            }
        }
        DockerImage::Missing => {
            subject.push(at(
                location,
                Check::fail("action.image.missing", "`runs.image` is missing or empty"),
            ));
        }
    }
}

fn push_runner_os_checks(subject: &mut Subject, input: &JobPlanInput, location: &Option<String>) {
    match input.runner_os {
        RunnerOs::Linux => subject.push(at(
            location,
            Check::pass("container.runner_os.linux", "configured runner OS is Linux"),
        )),
        other => subject.push(at(
            location,
            Check::fail(
                "container.runner_os.unsupported",
                format!(
                    "job containers require a Linux runner; configured runner OS is {}",
                    other.gha_name()
                ),
            ),
        )),
    }
}

fn push_runs_on_checks(subject: &mut Subject, input: &JobPlanInput, location: &Option<String>) {
    if input.runs_on.is_empty() {
        return;
    }
    let lowered = input
        .runs_on
        .iter()
        .map(|label| label.to_ascii_lowercase())
        .collect::<Vec<_>>();
    if lowered.iter().any(|label| {
        label.contains("ubuntu") || label.contains("linux") || label.contains("self-hosted")
    }) {
        subject.push(at(
            location,
            Check::pass(
                "container.runs_on.linux",
                "runs-on appears compatible with Linux containers",
            ),
        ));
    } else if lowered
        .iter()
        .any(|label| label.contains("windows") || label.contains("macos"))
    {
        subject.push(at(
            location,
            Check::fail(
                "container.runs_on.linux",
                "runs-on targets a non-Linux runner while declaring a job container",
            ),
        ));
    } else if input.runs_on.iter().any(|label| label.contains("${{")) {
        subject.push(at(
            location,
            Check::warn(
                "container.runs_on.linux",
                "runs-on contains expressions; Linux compatibility cannot be proven statically",
            ),
        ));
    } else {
        subject.push(at(
            location,
            Check::warn(
                "container.runs_on.linux",
                "runs-on does not clearly identify a Linux runner",
            ),
        ));
    }
}

fn push_credentials_checks(subject: &mut Subject, input: &JobPlanInput, location: &Option<String>) {
    let username = input.credentials_username_present;
    let password = input.credentials_password_present;
    match (username, password) {
        (true, true) => {
            subject
                .credentials_redacted
                .extend(["username".to_owned(), "password".to_owned()]);
            subject.push(at(
                location,
                Check::pass(
                    "container.credentials.present",
                    "container.credentials.username and .password both present (values redacted)",
                ),
            ));
        }
        (true, false) | (false, true) => {
            if username {
                subject.credentials_redacted.push("username".to_owned());
            }
            if password {
                subject.credentials_redacted.push("password".to_owned());
            }
            subject.push(at(
                location,
                Check::warn(
                    "container.credentials.partial",
                    "container.credentials declared only one of username/password",
                ),
            ));
        }
        (false, false) => {}
    }
}

fn push_env_checks(subject: &mut Subject, env: &[(String, String)], location: &Option<String>) {
    for (key, _value) in env {
        if is_sensitive_key(key) {
            subject.env_redacted.push(key.clone());
            subject.push(at(
                location,
                Check::pass(
                    "container.env.redacted",
                    format!("env key `{key}` redacted before recording"),
                ),
            ));
        }
    }
}

fn push_port_checks(subject: &mut Subject, ports: &[String], location: &Option<String>) {
    for raw in ports {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if validate_port_mapping(trimmed) {
            subject.push(at(
                location,
                Check::pass("container.port.parse", format!("port `{trimmed}` parsed")),
            ));
        } else {
            subject.push(at(
                location,
                Check::fail(
                    "container.port.parse",
                    format!("port `{trimmed}` is not in CONTAINER or HOST:CONTAINER form"),
                ),
            ));
        }
    }
}

fn push_volume_checks(subject: &mut Subject, volumes: &[String], location: &Option<String>) {
    for raw in volumes {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.contains("/var/run/docker.sock") {
            subject.push(at(
                location,
                Check::warn(
                    "container.volume.docker_socket",
                    format!("volume `{trimmed}` mounts the Docker socket"),
                ),
            ));
        } else if looks_like_windows_host_path(trimmed) {
            subject.push(at(
                location,
                Check::warn(
                    "container.volume.windows_host_path",
                    format!("volume `{trimmed}` mounts a Windows host path"),
                ),
            ));
        }
        if validate_volume(trimmed) {
            subject.push(at(
                location,
                Check::pass(
                    "container.volume.parse",
                    format!("volume `{trimmed}` parsed"),
                ),
            ));
        } else {
            subject.push(at(
                location,
                Check::fail(
                    "container.volume.parse",
                    format!("volume `{trimmed}` could not be parsed"),
                ),
            ));
        }
    }
}

fn apply_options_plan(subject: &mut Subject, plan: &OptionsPlan, location: &Option<String>) {
    let before = subject.checks.len();
    apply_options_to_subject(plan, subject);
    if let Some(loc) = location {
        for check in subject.checks.iter_mut().skip(before) {
            if check.location.is_none() {
                check.location = Some(loc.clone());
            }
        }
    }
}

fn validate_port_mapping(raw: &str) -> bool {
    let (body, _proto) = raw.rsplit_once('/').unwrap_or((raw, ""));
    let parts = body.split(':').collect::<Vec<_>>();
    match parts.as_slice() {
        [container] => container.parse::<u16>().is_ok(),
        [host, container] => host.parse::<u16>().is_ok() && container.parse::<u16>().is_ok(),
        _ => false,
    }
}

fn validate_volume(raw: &str) -> bool {
    if raw.is_empty() {
        return false;
    }
    // Either a bare container path (`/data`), a HOST:DEST[:MODE] mount, or a
    // Windows HOST:DEST where the drive letter introduces a third colon. The
    // disambiguating signal is: at least one segment is an absolute container
    // path (starts with `/`). looks_like_windows_host_path elsewhere emits the
    // appropriate warning; the parse just decides "well-formed enough".
    raw.split(':').any(|segment| segment.starts_with('/'))
}

fn image_pin_check(image: &str) -> Check {
    if image.contains("@sha256:") {
        return Check::pass(
            "container.image.pin",
            format!("image `{image}` is pinned by digest"),
        );
    }
    let last = image.rsplit('/').next().unwrap_or(image);
    if let Some((_, tag)) = last.rsplit_once(':') {
        if !tag.eq_ignore_ascii_case("latest") && !tag.trim().is_empty() {
            return Check::pass(
                "container.image.pin",
                format!("image `{image}` has an explicit tag `{tag}`"),
            );
        }
    }
    Check::warn(
        "container.image.pin",
        format!("image `{image}` is not pinned by digest or non-latest tag"),
    )
}

fn at(location: &Option<String>, check: Check) -> Check {
    match location {
        Some(loc) if check.location.is_none() => check.at(loc.clone()),
        _ => check,
    }
}

/// Allow `image_pin_check`-style helpers to carry a location through the
/// builder chain without explicit `if let` ladders.
trait WithLocation {
    fn map_location(self, location: Option<String>) -> Self;
}

impl WithLocation for Check {
    fn map_location(self, location: Option<String>) -> Self {
        match location {
            Some(loc) => self.at(loc),
            None => self,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job_input() -> JobPlanInput {
        JobPlanInput {
            job_id: "build".to_owned(),
            runner_os: RunnerOs::Linux,
            runs_on: vec!["ubuntu-22.04".to_owned()],
            container_image: Some("node:22-bookworm".to_owned()),
            env: vec![("NODE_ENV".to_owned(), "test".to_owned())],
            ports: vec!["3000".to_owned()],
            volumes: vec!["/host/cache:/cache".to_owned()],
            options: "--cpus 2".to_owned(),
            credentials_username_present: false,
            credentials_password_present: false,
            location: None,
        }
    }

    #[test]
    fn job_classifies_clean_linux_container() {
        let subject = {
            let mut subject = plan_job(&job_input());
            subject.finalize();
            subject
        };
        // node:22-bookworm has an explicit tag, ubuntu-22.04 is a Linux runner,
        // --cpus 2 is a recognized option — nothing should warn or fail.
        assert_eq!(subject.classification, Compatibility::Exact);
        assert!(subject.requires_docker);
        assert!(
            subject
                .checks
                .iter()
                .any(|c| c.id == "container.image.declared"
                    && c.message.contains("node:22-bookworm"))
        );
    }

    #[test]
    fn job_on_windows_runner_fails() {
        let mut input = job_input();
        input.runner_os = RunnerOs::Windows;
        let mut subject = plan_job(&input);
        subject.finalize();
        assert_eq!(subject.classification, Compatibility::Unsupported);
        assert!(
            subject
                .checks
                .iter()
                .any(|c| c.id == "container.runner_os.unsupported")
        );
    }

    #[test]
    fn job_with_network_option_fails() {
        let mut input = job_input();
        input.options = "--network host".to_owned();
        let mut subject = plan_job(&input);
        subject.finalize();
        assert_eq!(subject.classification, Compatibility::Unsupported);
        assert!(
            subject
                .checks
                .iter()
                .any(|c| c.id == "container.options.network")
        );
        assert_eq!(subject.network_model, NetworkModel::UnsupportedCustom);
    }

    #[test]
    fn job_redacts_password_env() {
        let mut input = job_input();
        input
            .env
            .push(("DATABASE_PASSWORD".to_owned(), "secret".to_owned()));
        let subject = plan_job(&input);
        assert!(
            subject
                .env_redacted
                .contains(&"DATABASE_PASSWORD".to_owned())
        );
    }

    #[test]
    fn action_with_docker_uri_classifies_exact() {
        let mut subject = plan_action(&ActionPlanInput {
            action_ref: "docker://alpine:3.20".to_owned(),
            step_id: Some("step-1".to_owned()),
            action_path: None,
            using: None,
            image: Some("docker://alpine:3.20".to_owned()),
            entrypoint: None,
            pre_entrypoint: None,
            post_entrypoint: None,
            args: Vec::new(),
            env: Vec::new(),
            location: None,
        });
        subject.finalize();
        assert_eq!(subject.classification, Compatibility::Exact);
        assert!(
            subject
                .checks
                .iter()
                .any(|c| c.id == "action.image.docker_uri")
        );
    }

    #[test]
    fn action_with_missing_dockerfile_fails() {
        let mut subject = plan_action(&ActionPlanInput {
            action_ref: "./missing-action".to_owned(),
            step_id: None,
            action_path: None,
            using: Some("docker".to_owned()),
            image: Some("Dockerfile".to_owned()),
            entrypoint: None,
            pre_entrypoint: None,
            post_entrypoint: None,
            args: Vec::new(),
            env: Vec::new(),
            location: None,
        });
        subject.finalize();
        assert_eq!(subject.classification, Compatibility::Unsupported);
        assert!(
            subject
                .checks
                .iter()
                .any(|c| c.id == "action.image.dockerfile_missing")
        );
    }

    #[test]
    fn action_unsupported_using_fails() {
        let mut subject = plan_action(&ActionPlanInput {
            action_ref: "./javascript-action".to_owned(),
            step_id: None,
            action_path: None,
            using: Some("node20".to_owned()),
            image: None,
            entrypoint: None,
            pre_entrypoint: None,
            post_entrypoint: None,
            args: Vec::new(),
            env: Vec::new(),
            location: None,
        });
        subject.finalize();
        assert_eq!(subject.classification, Compatibility::Unsupported);
        assert!(
            subject
                .checks
                .iter()
                .any(|c| c.id == "action.using.unsupported")
        );
    }
}
