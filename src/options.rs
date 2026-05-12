//! Parser for `container.options` (the raw Docker option string that ci-forge
//! passes to `docker run`). The parser tokenizes via [`shell_words`], walks the
//! tokens looking for known flags, and emits structured findings that the
//! engine layers into the subject's checks.

use crate::model::{Check, NetworkModel, Subject};
use serde::{Deserialize, Serialize};

/// Result of parsing one options string.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OptionsPlan {
    pub raw: String,
    pub tokens: Vec<String>,
    pub classified: Vec<ClassifiedOption>,
    pub unsupported: Vec<String>,
    pub risky: Vec<String>,
    pub unknown: Vec<String>,
    pub network_model: NetworkModel,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClassifiedOption {
    pub flag: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub kind: OptionKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "kebab-case")]
pub enum OptionKind {
    User,
    Workdir,
    Entrypoint,
    Env,
    Volume,
    Port,
    Cpus,
    Memory,
    Network,
    Privileged,
    HostNamespace,
    SecurityOpt,
    CapAdd,
    Unknown,
}

pub fn parse_options(raw: &str) -> Result<OptionsPlan, String> {
    let trimmed = raw.trim();
    let tokens = if trimmed.is_empty() {
        Vec::new()
    } else {
        shell_words::split(raw).map_err(|err| format!("could not parse Docker options: {err}"))?
    };

    let mut plan = OptionsPlan {
        raw: raw.to_owned(),
        tokens: tokens.clone(),
        classified: Vec::new(),
        unsupported: Vec::new(),
        risky: Vec::new(),
        unknown: Vec::new(),
        network_model: NetworkModel::DockerDefault,
    };

    let mut index = 0usize;
    while index < tokens.len() {
        let token = tokens[index].clone();
        let (flag, inline) = split_flag(&token);
        match flag.as_str() {
            "--network" | "--net" => {
                consume_value(&tokens, &mut index, inline.as_deref());
                plan.unsupported.push(flag.clone());
                plan.classified.push(ClassifiedOption {
                    flag: flag.clone(),
                    value: inline,
                    kind: OptionKind::Network,
                });
                plan.network_model = NetworkModel::UnsupportedCustom;
            }
            "--privileged" => {
                plan.risky.push(flag.clone());
                plan.classified.push(ClassifiedOption {
                    flag,
                    value: None,
                    kind: OptionKind::Privileged,
                });
            }
            "--pid" | "--ipc" | "--uts" if inline.as_deref() == Some("host") => {
                plan.risky.push(format!("{flag}=host"));
                plan.classified.push(ClassifiedOption {
                    flag,
                    value: Some("host".to_owned()),
                    kind: OptionKind::HostNamespace,
                });
            }
            "--pid=host" | "--ipc=host" | "--uts=host" => {
                plan.risky.push(flag.clone());
                plan.classified.push(ClassifiedOption {
                    flag,
                    value: Some("host".to_owned()),
                    kind: OptionKind::HostNamespace,
                });
            }
            "--security-opt" => {
                let value = consume_value(&tokens, &mut index, inline.as_deref());
                plan.risky.push(format!(
                    "--security-opt {}",
                    value.clone().unwrap_or_default()
                ));
                plan.classified.push(ClassifiedOption {
                    flag,
                    value,
                    kind: OptionKind::SecurityOpt,
                });
            }
            "--cap-add" => {
                let value = consume_value(&tokens, &mut index, inline.as_deref());
                plan.risky
                    .push(format!("--cap-add {}", value.clone().unwrap_or_default()));
                plan.classified.push(ClassifiedOption {
                    flag,
                    value,
                    kind: OptionKind::CapAdd,
                });
            }
            "--user" | "-u" => {
                let value = consume_value(&tokens, &mut index, inline.as_deref());
                plan.classified.push(ClassifiedOption {
                    flag,
                    value,
                    kind: OptionKind::User,
                });
            }
            "--workdir" | "-w" => {
                let value = consume_value(&tokens, &mut index, inline.as_deref());
                plan.classified.push(ClassifiedOption {
                    flag,
                    value,
                    kind: OptionKind::Workdir,
                });
            }
            "--entrypoint" => {
                let value = consume_value(&tokens, &mut index, inline.as_deref());
                plan.classified.push(ClassifiedOption {
                    flag,
                    value,
                    kind: OptionKind::Entrypoint,
                });
            }
            "--env" | "-e" => {
                let value = consume_value(&tokens, &mut index, inline.as_deref());
                plan.classified.push(ClassifiedOption {
                    flag,
                    value,
                    kind: OptionKind::Env,
                });
            }
            "--volume" | "-v" => {
                let value = consume_value(&tokens, &mut index, inline.as_deref());
                plan.classified.push(ClassifiedOption {
                    flag,
                    value,
                    kind: OptionKind::Volume,
                });
            }
            "--publish" | "-p" => {
                let value = consume_value(&tokens, &mut index, inline.as_deref());
                plan.classified.push(ClassifiedOption {
                    flag,
                    value,
                    kind: OptionKind::Port,
                });
            }
            "--cpus" => {
                let value = consume_value(&tokens, &mut index, inline.as_deref());
                plan.classified.push(ClassifiedOption {
                    flag,
                    value,
                    kind: OptionKind::Cpus,
                });
            }
            "--memory" | "-m" => {
                let value = consume_value(&tokens, &mut index, inline.as_deref());
                plan.classified.push(ClassifiedOption {
                    flag,
                    value,
                    kind: OptionKind::Memory,
                });
            }
            other if other.starts_with("--") || (other.starts_with('-') && other.len() == 2) => {
                plan.unknown.push(other.to_owned());
                plan.classified.push(ClassifiedOption {
                    flag: other.to_owned(),
                    value: inline,
                    kind: OptionKind::Unknown,
                });
            }
            _ => {
                // Bare positional tokens (image names, sub-args of the previous
                // unknown flag, etc.) are not classified.
            }
        }
        index += 1;
    }

    Ok(plan)
}

fn split_flag(token: &str) -> (String, Option<String>) {
    if let Some((flag, value)) = token.split_once('=') {
        (flag.to_owned(), Some(value.to_owned()))
    } else {
        (token.to_owned(), None)
    }
}

fn consume_value(tokens: &[String], index: &mut usize, inline: Option<&str>) -> Option<String> {
    if let Some(value) = inline {
        return Some(value.to_owned());
    }
    if *index + 1 < tokens.len() {
        *index += 1;
        return Some(tokens[*index].clone());
    }
    None
}

/// Apply the parsed options plan to a subject by appending the appropriate
/// checks and updating the subject's `network_model` and classification.
pub fn apply_options_to_subject(plan: &OptionsPlan, subject: &mut Subject) {
    if plan.tokens.is_empty() {
        subject.push(Check::pass(
            "container.options.parse",
            "no Docker options to parse",
        ));
        return;
    }
    subject.push(Check::pass(
        "container.options.parse",
        format!("parsed {} option token(s)", plan.tokens.len()),
    ));

    for option in &plan.classified {
        match option.kind {
            OptionKind::Network => {
                let value = option.value.clone().unwrap_or_default();
                subject.push(Check::fail(
                    "container.options.network",
                    format!(
                        "{} {} is not supported under ci-forge-managed networking",
                        option.flag, value
                    ),
                ));
            }
            OptionKind::Privileged => {
                subject.push(Check::warn(
                    "container.options.privileged",
                    "--privileged broadens container isolation",
                ));
            }
            OptionKind::HostNamespace => {
                subject.push(Check::warn(
                    "container.options.host_namespace",
                    format!(
                        "{}={} broadens container isolation",
                        option.flag,
                        option.value.as_deref().unwrap_or("host")
                    ),
                ));
            }
            OptionKind::SecurityOpt => {
                subject.push(Check::warn(
                    "container.options.security_opt",
                    format!(
                        "--security-opt {} broadens container isolation",
                        option.value.as_deref().unwrap_or("")
                    ),
                ));
            }
            OptionKind::CapAdd => {
                subject.push(Check::warn(
                    "container.options.cap_add",
                    format!(
                        "--cap-add {} broadens container capabilities",
                        option.value.as_deref().unwrap_or("")
                    ),
                ));
            }
            OptionKind::Volume => {
                let value = option.value.clone().unwrap_or_default();
                if value.contains("/var/run/docker.sock") {
                    subject.push(Check::warn(
                        "container.volume.docker_socket",
                        format!(
                            "{} {} mounts the Docker socket; this grants host-level control",
                            option.flag, value
                        ),
                    ));
                } else if looks_like_windows_host_path(&value) {
                    subject.push(Check::warn(
                        "container.volume.windows_host_path",
                        format!(
                            "{} {} mounts a Windows host path into a Linux container",
                            option.flag, value
                        ),
                    ));
                } else {
                    subject.push(Check::pass(
                        "container.options.classified",
                        format!("{} {} classified", option.flag, value),
                    ));
                }
            }
            OptionKind::Unknown => {
                subject.push(Check::warn(
                    "container.options.unknown",
                    format!("unknown Docker flag {}", option.flag),
                ));
            }
            _ => {
                subject.push(Check::pass(
                    "container.options.classified",
                    format!(
                        "{} {} classified as {:?}",
                        option.flag,
                        option.value.as_deref().unwrap_or("(none)"),
                        option.kind
                    ),
                ));
            }
        }
    }

    if plan.unsupported.is_empty() {
        subject.push(Check::pass(
            "container.options.supported",
            "options contain no unsupported Docker flags",
        ));
    }

    // Only override subject.network_model when the options parser found an
    // explicit --network / --net flag. Otherwise leave whatever the subject's
    // surface (job-container, docker-action) already declared.
    if plan.network_model == NetworkModel::UnsupportedCustom {
        subject.network_model = NetworkModel::UnsupportedCustom;
    }
}

pub fn looks_like_windows_host_path(value: &str) -> bool {
    // A Docker volume that mounts a Windows host path takes the shape
    // `C:\foo:/dest` or `C:/foo:/dest`. Splitting on `:` puts the drive letter
    // in the first segment all by itself. Otherwise look for embedded
    // backslashes anywhere in the value.
    let first = value.split(':').next().unwrap_or("");
    if first.len() == 1
        && first
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
    {
        return true;
    }
    value.contains('\\')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SubjectKind;

    #[test]
    fn empty_options_parse_clean() {
        let plan = parse_options("").unwrap();
        assert!(plan.tokens.is_empty());
        assert_eq!(plan.network_model, NetworkModel::DockerDefault);
    }

    #[test]
    fn network_flag_is_unsupported() {
        let plan = parse_options("--network host").unwrap();
        assert_eq!(plan.unsupported, vec!["--network"]);
        assert_eq!(plan.network_model, NetworkModel::UnsupportedCustom);
    }

    #[test]
    fn net_flag_is_unsupported() {
        let plan = parse_options("--net=bridge").unwrap();
        assert_eq!(plan.unsupported, vec!["--net"]);
    }

    #[test]
    fn privileged_is_risky() {
        let plan = parse_options("--privileged").unwrap();
        assert_eq!(plan.risky, vec!["--privileged"]);
    }

    #[test]
    fn pid_host_is_risky() {
        let plan = parse_options("--pid=host").unwrap();
        assert!(plan.risky.iter().any(|item| item.contains("host")));
    }

    #[test]
    fn cpus_and_memory_are_classified() {
        let plan = parse_options("--cpus 2 --memory 4g").unwrap();
        assert!(
            plan.classified
                .iter()
                .any(|c| c.kind == OptionKind::Cpus && c.value.as_deref() == Some("2"))
        );
        assert!(
            plan.classified
                .iter()
                .any(|c| c.kind == OptionKind::Memory && c.value.as_deref() == Some("4g"))
        );
    }

    #[test]
    fn unknown_flag_is_flagged() {
        let plan = parse_options("--frobulate yes").unwrap();
        assert_eq!(plan.unknown, vec!["--frobulate"]);
    }

    #[test]
    fn docker_socket_mount_warns_subject() {
        let plan = parse_options("-v /var/run/docker.sock:/var/run/docker.sock").unwrap();
        let mut subject = Subject::new(SubjectKind::JobContainer);
        apply_options_to_subject(&plan, &mut subject);
        assert!(
            subject
                .checks
                .iter()
                .any(|check| check.id == "container.volume.docker_socket")
        );
    }

    #[test]
    fn windows_host_path_warns_subject() {
        let plan = parse_options("-v C:\\work:/work").unwrap();
        let mut subject = Subject::new(SubjectKind::JobContainer);
        apply_options_to_subject(&plan, &mut subject);
        assert!(
            subject
                .checks
                .iter()
                .any(|check| check.id == "container.volume.windows_host_path")
        );
    }

    #[test]
    fn looks_like_windows_host_path_basics() {
        assert!(looks_like_windows_host_path("C:\\work:/work"));
        assert!(looks_like_windows_host_path("D:/data:/data"));
        assert!(looks_like_windows_host_path("Z:\\repo"));
        assert!(!looks_like_windows_host_path("/host/cache:/cache"));
        assert!(!looks_like_windows_host_path("named-volume:/data"));
    }
}
