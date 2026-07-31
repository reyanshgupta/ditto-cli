# Ditto CLI

See [AGENTS.md](AGENTS.md).

It covers driving Ditto without a terminal (never run bare `ditto-cli`; use `--json`), where the config lives, the module layout, the comment and error conventions, the recipes for adding a subcommand or a tool, and how to cut a release.

Read [Releasing](AGENTS.md#releasing) before publishing one. The short version: a release is published by pushing the `vX.Y.Z` tag, not by the version-bump commit, and a bump left untagged fails silently.

Kept as a pointer rather than a copy so the two cannot drift apart.
