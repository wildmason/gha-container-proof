# Changelog

## 1.0.0 - 2026-05-12

- Initial release.
- Added `check-workflow` for scanning workflow YAML to find `jobs.<id>.container` (string and object forms), `steps[*].uses: docker://...`, and local action manifests with `runs.using: docker` (image: `Dockerfile`, `docker://...`, or relative Dockerfile path).
- Added `plan-job` for classifying a concrete rendered job-container request (runner-os, runs-on, image, env, ports, volumes, options) with Docker-option parsing that flags `--network`, `--privileged`, `--pid=host`, `--ipc=host`, host Docker-socket mounts, and Windows host-path mounts into Linux containers.
- Added `plan-action` for classifying a concrete Docker action request: `runs.using: docker` with image `Dockerfile`/`docker://...`, `entrypoint`, `pre-entrypoint`, `post-entrypoint`, `args`, and rendered `INPUT_*` env.
- Added `probe` for offline-by-default Docker CLI probes: `docker image inspect` for local image presence, `docker run --rm` for tool/command probes. Skips cleanly when the Docker CLI is absent. No image pulls unless `--allow-pull` is passed.
- Added stable receipt schema (`schema_version: 1`) with text, JSON, and Markdown rendering.
- Added secret redaction for `container.credentials` values and env keys matching `PASSWORD|PASS|SECRET|TOKEN|CREDENTIAL|KEY|API`.
- Added composite GitHub Action wrapper that installs the crate and dispatches to any of the four commands.
