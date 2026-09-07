# Rust CI and required checks

Rustle runs the same pinned Rust toolchain and xtask-owned command sets locally
and in GitHub Actions. The workflow intentionally has no path filters: changes
to shared source, `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, xtask, build
scripts, and platform adapters always reach every required platform gate.

## Authoritative commands

```bash
# Full local/quality gate: toolchain, fmt, check, Clippy, tests, and rustdoc.
cargo xtask check

# Native runner gate: toolchain, check, Clippy, and tests for the host OS.
cargo xtask check-native

# Dependency policy: advisories, licenses, bans, registries, and git sources.
cargo xtask supply-chain
```

Every Cargo resolution/build command inside these entry points uses
`--locked`. `rust-toolchain.toml` owns the Rust version; the CI setup action
reads that file instead of copying the version into workflow YAML.

## Stable required-check contract

The workflow name is `Rust CI`. Its stable job names are:

- `rust-quality`
- `rust-supply-chain`
- `rust-windows`
- `rust-linux`
- `rust-macos`

These names are an external repository contract. Renaming or replacing one
requires a branch-protection migration, not an ordinary YAML cleanup.

Each job has an explicit timeout, participates in concurrency cancellation,
and runs with only `contents: read`. Pull-request jobs receive no release or
package-manager secrets. Cargo caches are saved only from successful `main`
pushes, and failure artifacts contain only the captured build log under
`target/ci-logs` with seven-day retention. The supply-chain job installs the
exact cargo-deny and cargo-machete versions from their repository version files
and delegates policy arguments to xtask.

## Enabling branch protection

Repository files cannot make their own check names required. After this
workflow has completed successfully at least once on GitHub:

1. Open repository **Settings → Rules → Rulesets** (or Branches/branch
   protection on repositories without rulesets).
2. Target the `main` branch and require a pull request before merging.
3. Require the five check names listed above and require branches to be up to
   date before merging.
4. Do not enable “allow specified actors to bypass” unless the repository has a
   separately reviewed emergency policy.
5. Save the rule, open a test pull request, and confirm a deliberately failing
   formatting/test change blocks merge while a documentation-only change still
   runs all five jobs.

Changing branch protection is an external administrator action. It is not
performed by the workflow or xtask and must be reviewed separately.

## Failure handling

Interactive GUI, audio-device, notification-area, and Explorer-restart tests
remain explicitly ignored in unattended CI. Pure state/contract tests and
native registration code must still compile on the owning OS. A temporary
runner exception must record an issue, owner, and restoration deadline; do not
use permanent `continue-on-error`, broad target exclusions, or lint allows.
