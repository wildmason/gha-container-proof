# Rules

`gha-container-proof` emits check IDs intended to stay stable within a major version.

## Workflow scan

- `container.workflow.read`: workflow file was readable and parseable.
- `container.workflow.jobs_missing`: workflow has no `jobs` mapping; **skip**.
- `container.workflow.no_container`: workflow defines no job containers, `docker://` step uses, or Docker actions; **skip**.
- `container.workflow.subjects`: workflow produced at least one container subject; **pass**.
- `container.services.delegated`: workflow also declares `jobs.<id>.services`; delegated to `gha-service-proof`; **pass**.

## Job container

- `container.image.declared`: job container declares a non-empty image; **pass** or **fail** when blank.
- `container.image.expression`: image contains `${{ ... }}` and was not statically rendered; **warn** (treat as `simulated`).
- `container.image.pin`: image is pinned by digest (`@sha256:`) or an explicit non-`latest` tag; **pass** otherwise **warn**.
- `container.runner_os.linux`: configured runner OS is Linux; **pass**. Non-Linux job container hosts emit `container.runner_os.unsupported` as **fail**.
- `container.runs_on.linux`: `runs-on` looks compatible with Linux; **pass**, **warn** (unknown), or **fail** (`windows-*` / `macos-*`).
- `container.credentials.present`: `container.credentials` declared `username` and `password`; **pass**. Values are never echoed in the receipt.
- `container.credentials.partial`: only one of `username`/`password` was set; **warn**.
- `container.env.redacted`: a sensitive env key (`PASSWORD|PASS|SECRET|TOKEN|CREDENTIAL|KEY|API`) was redacted before being recorded; **pass**.
- `container.port.parse`: port mapping parsed successfully; **pass** otherwise **fail**.
- `container.volume.parse`: volume mapping parsed successfully; **pass** otherwise **fail**.
- `container.volume.windows_host_path`: absolute Windows host path mounted into a Linux container; **warn**.
- `container.volume.docker_socket`: Docker socket mount detected; **warn**.

## Docker options (`container.options`)

- `container.options.parse`: raw options string parsed via shell-words; **pass** otherwise **fail**.
- `container.options.supported`: parsed options contain no unsupported Docker flags; **pass**.
- `container.options.network`: `--network` / `--net` is set; **fail** (ci-forge owns networking).
- `container.options.privileged`: `--privileged` is set; **warn** (**fail** under `--strict`).
- `container.options.host_namespace`: `--pid=host`, `--ipc=host`, or `--uts=host` is set; **warn**.
- `container.options.security_opt`: `--security-opt` is set; **warn**.
- `container.options.cap_add`: `--cap-add` is set; **warn**.
- `container.options.unknown`: an unrecognized Docker flag was found; **warn** (classified `simulated`).
- `container.options.classified`: a recognized flag (`--user`, `--workdir`/`-w`, `--entrypoint`, `--env`/`-e`, `--volume`/`-v`, `--cpus`, `--memory`/`-m`) was classified; **pass**.

## Docker action

- `action.manifest.read`: local action manifest was readable and parseable; **pass**.
- `action.manifest.unavailable`: action reference is remote (`owner/repo@ref`) and no manifest path was provided; **warn** (classified `simulated`).
- `action.using.docker`: `runs.using` is `docker`; **pass**.
- `action.using.unsupported`: `runs.using` is not `docker`; **fail** (this tool only classifies Docker actions).
- `action.image.docker_uri`: `runs.image` is `docker://image`; **pass**.
- `action.image.dockerfile`: `runs.image` is `Dockerfile` or a relative `*.Dockerfile`; **pass** with `requires_build: true`.
- `action.image.dockerfile_missing`: `runs.image` points at a Dockerfile that does not exist; **fail**.
- `action.image.missing`: `runs.image` is empty or absent; **fail**.
- `action.entrypoint.declared`: `runs.entrypoint` is set; **pass**.
- `action.pre_entrypoint.declared`: `runs.pre-entrypoint` is set; **pass** with `requires_pre_hook: true`.
- `action.post_entrypoint.declared`: `runs.post-entrypoint` is set; **pass** with `requires_post_hook: true`.
- `action.args.preserved`: `runs.args` is preserved verbatim; **pass**.
- `action.env.redacted`: a sensitive env key was redacted; **pass**.

## Probe

- `probe.docker_cli_not_found`: Docker CLI is not on PATH; all probes **skip**.
- `probe.docker_daemon_unreachable`: `docker image inspect` could not reach the daemon; image and tool probes **fail**.
- `probe.image_inspect_ok`: `docker image inspect <image>` succeeded; **pass**.
- `probe.image_inspect_missing`: image is not present locally; **fail** unless `--allow-pull` was set (then **warn** with `probe.image_pull_required`).
- `probe.image_inspect_parse_failed`: `docker image inspect` returned unparseable output; **warn**.
- `probe.image_pull_attempted`: probe pulled the image because `--allow-pull` was set; **pass** when successful, **fail** otherwise.
- `probe.tool_ok`: `docker run --rm <image> <tool> --version` succeeded; **pass**.
- `probe.tool_exit_nonzero`: tool probe exited non-zero; **fail**.
- `probe.tool_spawn_failed`: Docker could not spawn the probe command; **fail**.
- `probe.command_ok`: `docker run --rm <image> sh -c "<command>"` succeeded; **pass**.
- `probe.command_exit_nonzero`: command probe exited non-zero; **fail**.
- `probe.command_spawn_failed`: Docker could not spawn the command probe; **fail**.

## Strict mode

`--strict` promotes the following warnings to failures (in addition to any user-supplied warnings):

- `container.options.privileged`
- `container.options.host_namespace`
- `container.options.security_opt`
- `container.options.cap_add`
- `container.options.unknown`
- `container.volume.docker_socket`
- `container.volume.windows_host_path`
- `container.image.expression`
- `container.image.pin`
- `action.manifest.unavailable`
