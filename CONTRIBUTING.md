# Contributing

Run the local gates before sending changes:

```powershell
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo doc --locked --no-deps
```

Receipts are part of the public contract. Add or rename fields deliberately, and cover behavior changes with CLI tests.

This crate models the GitHub Actions **job container** and **Docker action** surface only. Service containers under `jobs.<id>.services` belong to `gha-service-proof`; do not duplicate them here. If a workflow defines both, `check-workflow` may classify the job-container side and emit `container.services.delegated` pointing to the sibling tool.

Docker probes must remain CLI-only (no socket use, no Docker SDK in v1) and must default to offline behavior. Any operation that pulls an image or contacts a registry must be opt-in behind `--allow-pull` or `--online`.
