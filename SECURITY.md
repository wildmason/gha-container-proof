# Security

Please report security issues privately to Wildmason before public disclosure.

`gha-container-proof` does not call any network service by default. It reads workflow YAML, local action manifests, and rendered job-container or Docker-action inputs; it optionally shells out to the Docker CLI for `probe`. Docker probes use `docker image inspect` and `docker run --rm`; the crate never mounts the Docker socket into a probe container by default, never starts background containers, and never pulls images unless `--allow-pull` is explicitly set. Stdout/stderr excerpts are captured in receipts — callers are responsible for ensuring probes do not echo secrets, and credential-shaped inputs (`container.credentials`, env keys matching `PASSWORD|PASS|SECRET|TOKEN|CREDENTIAL|KEY|API`) are redacted before they enter any receipt.
