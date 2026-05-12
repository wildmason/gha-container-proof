//! Parser for local action manifests (`action.yml` / `action.yaml`).
//!
//! `gha-container-proof` only classifies actions whose `runs.using` is
//! `docker`; other action shapes are still parsed enough to report
//! `action.using.unsupported`.

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use serde_yaml::{Mapping, Value as YamlValue};
use std::fs;

#[derive(Debug, Clone, Default)]
pub struct ActionManifest {
    pub source_path: Utf8PathBuf,
    pub using: Option<String>,
    pub image: Option<String>,
    pub entrypoint: Option<String>,
    pub pre_entrypoint: Option<String>,
    pub post_entrypoint: Option<String>,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

impl ActionManifest {
    pub fn read(action_path: &Utf8Path) -> Result<Self> {
        let manifest_path = if action_path.is_file() {
            action_path.to_owned()
        } else {
            let yml = action_path.join("action.yml");
            if yml.exists() {
                yml
            } else {
                action_path.join("action.yaml")
            }
        };

        let raw = fs::read_to_string(&manifest_path)
            .with_context(|| format!("reading action manifest {manifest_path}"))?;
        let yaml: YamlValue = serde_yaml::from_str(&raw)
            .with_context(|| format!("parsing action manifest {manifest_path}"))?;

        let mut manifest = ActionManifest {
            source_path: manifest_path,
            ..ActionManifest::default()
        };

        if let Some(runs) = yaml.get("runs").and_then(YamlValue::as_mapping) {
            manifest.using = string_value(runs, "using");
            manifest.image = string_value(runs, "image");
            manifest.entrypoint = string_value(runs, "entrypoint");
            manifest.pre_entrypoint = string_value(runs, "pre-entrypoint");
            manifest.post_entrypoint = string_value(runs, "post-entrypoint");
            manifest.args = string_list(runs, "args");
            manifest.env = string_map(runs, "env");
        }

        Ok(manifest)
    }
}

fn string_value(mapping: &Mapping, key: &str) -> Option<String> {
    mapping
        .get(YamlValue::String(key.to_owned()))
        .and_then(yaml_string)
}

fn string_list(mapping: &Mapping, key: &str) -> Vec<String> {
    let Some(value) = mapping.get(YamlValue::String(key.to_owned())) else {
        return Vec::new();
    };
    match value {
        YamlValue::Sequence(items) => items.iter().filter_map(yaml_string).collect(),
        other => yaml_string(other).into_iter().collect(),
    }
}

fn string_map(mapping: &Mapping, key: &str) -> Vec<(String, String)> {
    let Some(map) = mapping
        .get(YamlValue::String(key.to_owned()))
        .and_then(YamlValue::as_mapping)
    else {
        return Vec::new();
    };
    map.iter()
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

/// Recognized shapes of `runs.image` for a Docker action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DockerImage {
    /// `docker://image[:tag]` reference.
    DockerUri(String),
    /// A Dockerfile path (literal `Dockerfile` or a relative `.Dockerfile`).
    Dockerfile(Utf8PathBuf),
    /// `runs.image` was missing or empty.
    Missing,
}

pub fn classify_image(image: Option<&str>, action_dir: Option<&Utf8Path>) -> DockerImage {
    let Some(raw) = image.map(str::trim) else {
        return DockerImage::Missing;
    };
    if raw.is_empty() {
        return DockerImage::Missing;
    }
    if let Some(uri) = raw.strip_prefix("docker://") {
        return DockerImage::DockerUri(uri.to_owned());
    }
    if raw.eq_ignore_ascii_case("dockerfile")
        || raw.to_ascii_lowercase().ends_with(".dockerfile")
        || raw.to_ascii_lowercase().ends_with("dockerfile")
    {
        let path = if let Some(dir) = action_dir {
            dir.join(raw)
        } else {
            Utf8PathBuf::from(raw)
        };
        return DockerImage::Dockerfile(path);
    }
    // Treat anything else as a Docker URI without the explicit scheme.
    DockerImage::DockerUri(raw.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn reads_minimal_docker_action() {
        let dir = tempdir().unwrap();
        let action = dir.path().join("action.yml");
        std::fs::write(
            &action,
            r#"name: x
description: y
runs:
  using: docker
  image: Dockerfile
  entrypoint: /entrypoint.sh
  pre-entrypoint: /pre.sh
  post-entrypoint: /post.sh
  args:
    - build
"#,
        )
        .unwrap();
        let path = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        let manifest = ActionManifest::read(&path).unwrap();
        assert_eq!(manifest.using.as_deref(), Some("docker"));
        assert_eq!(manifest.image.as_deref(), Some("Dockerfile"));
        assert_eq!(manifest.entrypoint.as_deref(), Some("/entrypoint.sh"));
        assert_eq!(manifest.pre_entrypoint.as_deref(), Some("/pre.sh"));
        assert_eq!(manifest.post_entrypoint.as_deref(), Some("/post.sh"));
        assert_eq!(manifest.args, vec!["build"]);
    }

    #[test]
    fn missing_manifest_errors() {
        let dir = tempdir().unwrap();
        let path = Utf8PathBuf::from_path_buf(dir.path().to_path_buf()).unwrap();
        assert!(ActionManifest::read(&path).is_err());
    }

    #[test]
    fn classify_image_handles_docker_uri() {
        assert_eq!(
            classify_image(Some("docker://alpine:3"), None),
            DockerImage::DockerUri("alpine:3".to_owned())
        );
    }

    #[test]
    fn classify_image_handles_dockerfile() {
        let DockerImage::Dockerfile(path) = classify_image(Some("Dockerfile"), None) else {
            panic!("expected Dockerfile classification");
        };
        assert_eq!(path.as_str(), "Dockerfile");
    }

    #[test]
    fn classify_image_handles_missing() {
        assert_eq!(classify_image(None, None), DockerImage::Missing);
        assert_eq!(classify_image(Some(""), None), DockerImage::Missing);
    }

    #[test]
    fn classify_image_treats_bare_string_as_docker_uri() {
        assert_eq!(
            classify_image(Some("ubuntu:22.04"), None),
            DockerImage::DockerUri("ubuntu:22.04".to_owned())
        );
    }
}
