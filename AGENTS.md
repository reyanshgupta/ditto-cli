# Ditto CLI

Ditto CLI is a Rust terminal application for keeping Claude Code and OpenAI Codex profiles isolated. It stores separate configuration and authentication roots for each profile, then launches the official CLIs with the selected profile environment. Users can manage profiles through either the Ratatui interface or equivalent command-line subcommands.

## Versioning

Ditto CLI is pre-1.0 and uses `0.x.y` semantic versions.

- Reserve `0.x.0` releases for major project milestones, intentionally breaking changes, or substantial feature sets that establish a new release series.
- Use patch increments for routine features, fixes, documentation, workflow changes, and other incremental work. For example, release a routine change after `0.2.0` as `0.2.1`, not `0.3.0`.
- Keep the version in `Cargo.toml`, the resolved package version in `Cargo.lock`, and the Git tag `vX.Y.Z` aligned when preparing a release.
