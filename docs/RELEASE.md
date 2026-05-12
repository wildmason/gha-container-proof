# Release

`gha-container-proof` follows the Wildmason proof-tool release lane.

## Local gates

```powershell
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo doc --locked --no-deps
cargo package --locked --allow-dirty
cargo publish --dry-run --locked --allow-dirty
action-proof --repo-root . --manifest action.yml --strict
```

## Tag and release

1. Bump `version` in `Cargo.toml` and update `CHANGELOG.md`.
2. Commit `chore(release): vX.Y.Z`.
3. Tag the release commit with an annotated tag (`git tag -a vX.Y.Z -m "vX.Y.Z"`).
4. Push the commit and the tag.
5. Create the GitHub Release pointing at the tag.
6. `cargo publish --locked`.
7. Move the floating `v1` tag to the release commit **as a lightweight tag** (not a tag-of-tag): `git tag -f v1 <release-commit>` so `git rev-parse v1^{}` peels straight to the release SHA.
8. Run `release-proof` against the published artifacts.

## Notes

- Do not amend or rebase published commits.
- The receipt schema is part of the public contract — additive fields are fine; renames or removals require advancing `schema_version`.
- Probes are CLI-only in v1.0 (no Docker socket or Docker SDK). The default is offline; `--allow-pull` is the only opt-in for network use.
