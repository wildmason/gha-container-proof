# gha-container-proof Protocol Scope

`gha-container-proof` models two GitHub Actions surfaces that involve Docker but are **not** service containers:

1. **Job containers** — `jobs.<job_id>.container` declared as a bare image string or a full mapping (`image`, `credentials`, `env`, `ports`, `volumes`, `options`).
2. **Docker actions** — composite, JavaScript, and Dockerfile-backed action manifests where `runs.using: docker` or `runs.using: "docker"` is declared, plus `steps[*].uses: docker://...` references.

Primary references:

- GitHub Docs, "Running jobs in a container" — https://docs.github.com/en/actions/using-jobs/running-jobs-in-a-container
- GitHub Docs, "Docker container actions" — https://docs.github.com/en/actions/creating-actions/creating-a-docker-container-action
- GitHub Docs, "Metadata syntax for GitHub Actions" — `runs.using`, `runs.image`, `runs.pre-entrypoint`, `runs.entrypoint`, `runs.post-entrypoint`, `runs.args`
- `actions/runner` job container and container-action implementations

## Boundary with `gha-service-proof`

`gha-service-proof` owns `jobs.<job_id>.services`, service readiness, health checks, port mappings to the host runner, and the service container compatibility surface. `gha-container-proof` does **not** repeat any of that.

If a workflow defines both a job container and one or more services, `check-workflow`:

- Classifies the job-container side (image, options, credentials, env, volumes, ports).
- Emits one `container.services.delegated` subject check pointing at `gha-service-proof` for the service side.

## Compatibility classifications

Each subject is classified with one of four values:

- `exact` — declared shape is fully supported; ci-forge can route to a real Docker container with no compensating behavior.
- `compatible` — declared shape is supported with minor caveats already accounted for in the receipt (warnings present).
- `simulated` — declared shape requires emulation or supplementary data that is not present (e.g. remote action manifest, image pull required while offline). ci-forge can still route, but downstream tools should treat the classification as best-effort.
- `unsupported` — declared shape will fail on real GitHub or ci-forge unless changed.

The top-level receipt rollup uses the worst of these values across all subjects.

## Network model

Each subject declares one of:

- `ci-forge-managed` — ci-forge owns container networking; user-specified `--network` / `--net` is rejected.
- `docker-default` — no networking concerns flagged.
- `unsupported-custom` — workflow attempted to set custom Docker networking that GitHub does not honor for job containers.

## Subject kinds

- `job-container` — primary container for a job.
- `docker-action` — a Docker action invocation (local action with `runs.using: docker`, or a `docker://...` step `uses`).
- `docker-probe` — a Docker CLI probe (`probe` command).

## What is intentionally out of scope

- **Service containers** — owned by `gha-service-proof`.
- **Workflow YAML structural lint** — owned by `gha-workflow-proof`.
- **Runner image / toolcache identity** — owned by `gha-runner-image-proof`.
- **Expression evaluation** — owned by `gha-expression-proof`. `gha-container-proof` preserves expression-bearing strings verbatim, marking them as `simulated` where pinning matters.
- **Real Docker lifecycle** — owned by ci-forge / the executor. `gha-container-proof` records compatibility and probe receipts only.
- **Image pulls** — disabled by default. `--allow-pull` opts probe into pulling when the daemon needs it.

## Action manifest resolution

`plan-action` accepts `--action-path` to read a local `action.yml` (or `action.yaml`). When the path is absent, the action is classified `simulated` with `action.manifest.unavailable` because the manifest cannot be inspected offline.

`docker://` action references are accepted with `--action-ref docker://image` and `--image docker://image`; no local manifest is required and the subject is classified by image identity alone.

## Receipt versioning

The schema is `schema_version: 1`. Additive fields are allowed without bumping the schema version. Breaking changes (renamed or removed fields, changed enum values) require advancing the schema version.
