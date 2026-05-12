# gha-container-proof

`gha-container-proof` is the GitHub Actions **job container** and **Docker action** compatibility oracle for offline CI. It turns `jobs.<job_id>.container` definitions, `steps[*].uses: docker://...` references, and local action manifests with `runs.using: docker` into deterministic receipts with stable check IDs.

It is part of Wildmason's offline GitHub Actions proof-tool lane:

- `gha-workflow-proof` checks workflow structure.
- `gha-eventsmith` creates event/context fixtures.
- `gha-expression-proof` evaluates Actions expressions.
- `gha-cache-proof` models `actions/cache`.
- `gha-artifact-proof` models artifact upload/download.
- `gha-service-proof` models **service containers** (`jobs.<id>.services`) and readiness.
- **`gha-container-proof` models the primary job container and Docker action containers.**
- `gha-github-service-proof` models the GitHub API/service surface.
- `gha-runner-image-proof` validates runner-image manifests and the toolcache.
- `gha-command-proof` checks runtime command and environment-file side effects.

`gha-container-proof` does not duplicate `gha-service-proof`. If a workflow has both job containers and services, `check-workflow` classifies the job-container side and emits `container.services.delegated` pointing at the sibling tool.

## Install

```powershell
cargo install gha-container-proof --locked
```

## Commands

### `check-workflow`

Scan workflow YAML for job containers and Docker action surfaces.

```powershell
gha-container-proof check-workflow `
  --repo . `
  --workspace . `
  --workflow .github/workflows/ci.yml `
  --format json `
  --output target/container-proof.json
```

When `--workflow` is omitted, the command scans `.github/workflows/*.yml` and `.github/workflows/*.yaml`. The scan detects:

- `jobs.<job_id>.container` as a bare image string or full object (`image`, `credentials`, `env`, `ports`, `volumes`, `options`).
- `steps[*].uses: docker://image`.
- `steps[*].uses: ./local-action` whose `action.yml` declares `runs.using: docker` with `image: Dockerfile`, `image: docker://...`, or a relative Dockerfile path.
- Remote action refs (`owner/repo@ref`) are classified as **simulated — external action manifest unavailable unless mirrored or provided**.

### `plan-job`

Classify a concrete rendered job-container request.

```bash
gha-container-proof plan-job \
  --job-id build \
  --runner-os linux \
  --runs-on ubuntu-22.04 \
  --container node:22-bookworm \
  --env NODE_ENV=test \
  --port 3000 \
  --volume /host/cache:/cache \
  --options "--cpus 2" \
  --format json \
  --output receipt.json
```

### `plan-action`

Classify a concrete Docker action request.

```bash
gha-container-proof plan-action \
  --action-ref ./actions/build-image \
  --action-path ./actions/build-image \
  --using docker \
  --image Dockerfile \
  --entrypoint /entrypoint.sh \
  --pre-entrypoint /pre.sh \
  --post-entrypoint /post.sh \
  --args "build" \
  --env INPUT_TARGET=release \
  --format json \
  --output receipt.json
```

Also supports `docker://` action references without a local path:

```bash
gha-container-proof plan-action --action-ref docker://alpine:3.20 --image docker://alpine:3.20 --format json
```

### `probe`

Optionally use the Docker CLI to verify runtime availability and image behavior. Offline by default — does not pull images unless `--allow-pull` is set.

```bash
gha-container-proof probe \
  --image node:22-bookworm \
  --runner-os linux \
  --tool node --tool bash \
  --command "node --version" \
  --format json \
  --output probe.json
```

- `docker image inspect <image>` records local image availability.
- `docker run --rm <image> <tool> --version` and `docker run --rm <image> sh -c "<command>"` record probe evidence.
- Captures command kind, image, exit code, stdout/stderr excerpts, and elapsed time.
- Skips cleanly when the Docker CLI is not on PATH (`probe.docker_cli_not_found`) and fails clearly when the daemon is unreachable (`probe.docker_daemon_unreachable`).
- Override the Docker binary with `--docker-bin <PATH>` or the `GHA_CONTAINER_PROOF_DOCKER` environment variable (used by tests to run against a fake docker).

## What It Models

### Job containers

- Container jobs require a Linux Docker runner; Windows and macOS hosted runners are unsupported.
- When a job runs in a container, shell steps execute inside the job container; networking is Docker-internal.
- `container.credentials` is recognized and classified as requiring secret-safe handling; credential values are redacted in receipts.
- `container.options` are parsed enough to flag known unsupported or dangerous options.
- `--network` / `--net` are unsupported under ci-forge-managed networking and emit a failure.
- `--privileged`, `--pid=host`, `--ipc=host`, `--security-opt`, and Docker socket mounts are flagged as risky.
- Absolute Windows host paths (`C:\...`, `D:\...`) mounted into Linux containers warn.
- `--user`, `--workdir`/`-w`, `--entrypoint`, `--env`/`-e`, `--volume`/`-v`, `--cpus`, `--memory`/`-m` are classified explicitly.
- Unknown Docker options are classified as `simulated/unknown` rather than silently accepted.

### Docker actions

- `runs.using: docker` actions run in a new container for the action.
- `runs.image: Dockerfile` (or a relative `*.Dockerfile` path) requires build support.
- `runs.image: docker://...` requires image availability.
- `runs.pre-entrypoint`, `runs.entrypoint`, and `runs.post-entrypoint` are recognized.
- `runs.args` is preserved.
- Action `INPUT_*` env can be passed through for receipts; the executor (ci-forge) owns post-state (`STATE_*`) handoff.
- Dockerfile-backed actions detect missing Dockerfile, unsupported context, and build requirement.

## Unsupported and warning surfaces

- Job container on non-Linux runner → **fail**.
- Docker unavailable on probe → **skip** (`probe.docker_cli_not_found`) or **fail** (`probe.docker_daemon_unreachable`).
- Image missing locally while offline → **fail** unless `--allow-pull` is set.
- `--network` / `--net` in options when ci-forge owns networks → **fail**.
- `--privileged` → **warn** (or **fail** under `--strict`).
- Host Docker socket mount → **warn** (high-risk; explicit compatibility).
- Absolute Windows host mount into Linux container → **warn**.
- `container.credentials` values → redacted; absence checked.
- Unknown Docker options → **warn** (`container.options.unknown`).
- Remote Docker action manifest unavailable → **simulated** (`action.manifest.unavailable`).

## Receipt shape

Every command emits the same top-level receipt:

```json
{
  "schema_version": 1,
  "tool": { "name": "gha-container-proof", "version": "1.0.0" },
  "checked_at": "2026-...",
  "mode": "check-workflow",
  "compatibility": "compatible",
  "summary": { "passed": 0, "warnings": 0, "failed": 0, "skipped": 0 },
  "subjects": [ ... ],
  "checks": [ ... ]
}
```

Each subject describes one job container, Docker action, or Docker probe:

```json
{
  "kind": "job-container",
  "job_id": "build",
  "image": "node:22-bookworm",
  "runner_os": "linux",
  "classification": "compatible",
  "network_model": "ci-forge-managed",
  "requires_docker": true,
  "requires_build": false,
  "requires_pull": false,
  "checks": [
    { "id": "container.image.declared", "status": "pass", "message": "...", "details": {} }
  ]
}
```

See [docs/spec.md](docs/spec.md) for the protocol surface and [docs/RULES.md](docs/RULES.md) for stable check IDs.

## GitHub Action

```yaml
- uses: wildmason/gha-container-proof@v1
  with:
    command: check-workflow
    repo: .
    workflow: .github/workflows/ci.yml
    format: json
    output: target/container-proof.json
```

The action installs the crate with Cargo unless `gha-container-proof` is already on `PATH`.

## Exit behavior

The CLI exits `0` when there are no failed checks. With `--strict`, warnings also fail the run.

## Limits

`gha-container-proof` is a compatibility oracle, not a runner. It does not start long-lived job containers as the source of truth for execution — ci-forge owns the Docker lifecycle and attaches these receipts beside `gha-service-proof`, `gha-cache-proof`, `gha-artifact-proof`, `gha-github-service-proof`, and the rest.

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
