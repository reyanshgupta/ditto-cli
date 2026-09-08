# Compatibility audit — updated 8 September 2026

This audit found reproducible cases where profile switching hid extensions or
kept the previous profile's environment. The fixes are covered by regression
tests; this is not a guarantee for every upstream agent version or login backend.
All write tests used temporary homes and stores, without real credentials.

## Fixed

| Problem | Result after the fix |
| --- | --- |
| Claude installed plugins through a profile and saved that profile's absolute path in the shared registry. Rename or deletion left the files present but unreachable. | Normalize affected `installPath` entries to the real shared cache before rename or deletion. Launch and sync recover older stale entries when their cache still exists. Preserve other metadata. |
| Nested launches of `default` inherited the parent's private home or session directory. | Restore the original environment, including originally unset overrides, before applying the selected profile. Native Windows private-home launches also set `USERPROFILE`. |
| New profiles could not link nested settings or skills because their parent directories did not exist. | Create private parent directories before linking. The lifecycle test covers every allowlisted path, including Devin, Cline, Amp, Goose, Crush, and Kiro. |
| A custom `CLAUDE_CONFIG_DIR` was ignored when finding the default setup. | Use and preserve the configured native root, including nested Ditto invocations. |
| Codex custom agents and Claude keyboard shortcuts were omitted. | Share `codex/agents` and `claude/keybindings.json`. |
| Claude sync skipped newly enabled plugins when the profile already had an `enabledPlugins` map. | Merge plugin and marketplace entries individually, retaining existing choices, including explicit `false` values. |
| A shared directory first created after profile creation stayed invisible until sync. | Link newly available paths for the selected tool before launch, preserving existing profile copies. |
| `sync --adopt` discarded foreign symlinks and could overwrite dangling backup links. | Back up both files and links, retain previous backups, and restore the original if replacement fails. |
| Automatic workspace binding printed prose into the launched agent's stdout. | Send the binding notice to stderr so JSON and other piped output remain usable. |

Creation also exposes individual linking errors as `shared_failed` in JSON and
prints them in its human report.

## Validation

- `cargo test`: 166 passing tests. The new lifecycle coverage checks creation,
  repeated sync, rename, deletion, and the absence of every table-defined
  credential/session fixture from shared paths.
- `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings`: passed.
- `cargo build && python3 .github/smoke.py`: passed on macOS. The compiled CLI
  launches stand-ins for all 36 agents, checks roots and argument forwarding,
  preserves exit code 23, keeps stdout parseable, and exercises profile creation,
  sync, nested launches, rename, and deletion. CI runs this on Linux and macOS;
  the Rust test matrix also includes Windows. Those remote CI runs were not
  executed as part of this local audit.
- Claude Code **2.1.263**: installed local fixture plugins using its real plugin
  commands; verified discovery through Ditto, a later install followed by sync,
  recovery of the reproduced stale registry entry, and new profile-installed
  plugins surviving immediate rename and deletion. The native registry retained
  all five fixture plugins with readable installation paths.
- Codex CLI **0.153.1**: installed and listed local fixture plugins through both
  native and managed roots, installed another plugin from a managed profile,
  checked discovery after rename, and verified native files survived deletion.
  Its real app-server `skills/list` also discovered a shared fixture skill.
- Pi **0.84.2** and Prime Agent **0.8.1** skill loaders were invoked through Ditto's
  launch environment and discovered shared fixture skills without model calls.

## Plugin update propagation

`python3 .github/plugin-updates.py` tests actual installed Claude and Codex CLIs
using a local marketplace and two managed profiles. Claude updates from 1.0.0 to
2.0.0 in the native setup, then to 3.0.0 from a managed profile, reach the native
setup and both profiles without a Ditto sync.

Codex uses the profile-local executable cache introduced in 0.4.3, rather than
a shared plugin directory. A native update to 2.0.0 is copied into both profiles
at their next launch, and the test verifies code resolves inside each selected
home. Updating to 3.0.0 inside `work` stays local: the default and `personal`
profiles retain 2.0.0. Existing copied files and links are preserved; in-place
changes to an existing version or a `latest` link need Codex's own update inside
the profile. The earlier audit's bidirectional Codex result applied to the old
shared cache and does not describe this release.

These checks use new CLI processes; already-running sessions may need their
harness's reload command or a restart.

The Rust update test also replaces every allowlisted source file or directory
and confirms that two existing profiles see the replacement without sync.
Ditto shares installed files; each harness remains responsible for fetching
plugin updates. Updating an existing plugin is distinct from enabling a new
Claude plugin, whose per-profile settings still require sync.

**Prime Agent 0.8.1 has a confirmed update gap.** Its native settings writer uses
an atomic rename that replaces the profile's `settings.json` symlink with a local
file. A package-list change then stops reaching the native setup and other
profiles, and later native package-list changes do not reach that profile.
`sync` reports `prime-agent/settings.json` in `shared_kept` and preserves the
local choice; it does not automatically reconcile the two versions. The native
Pi 0.84.2 writer was tested with the same operation and retained the shared link.
This means update propagation is verified for the shared files and the tested
Claude/Codex flows, but is not universally reliable for every harness's settings
writer. No automatic promotion or overwrite of those divergent settings was added.

The path checks agree with the upstream documentation for
[Claude's configuration roots](https://code.claude.com/docs/en/settings),
[Claude's keyboard shortcuts](https://code.claude.com/docs/en/keybindings),
[plugin enablement](https://code.claude.com/docs/en/discover-plugins), and
[Codex custom agents](https://developers.openai.com/codex/subagents).

## Boundaries and remaining compatibility risks

- Live OAuth sign-ins, paid model requests, remote marketplaces, full interactive
  agent sessions, and every supported agent's current binary were not exercised.
  Stand-ins validate Ditto's dispatch contract, not an upstream binary's behavior.
- Some tools keep credentials in user-wide keychains or services. Prime
  Inference's separate CLI credential and Prime Agent's daemon are examples.
  The tool-specific caveats in `src/tools.rs` remain relevant.
- Environment API keys and keys embedded in shared configuration remain shared.
  Checking that credential filenames are excluded cannot establish that every
  user-authored settings file is credential-free.
- Sharing uses an allowlist. Newly introduced upstream directories and arbitrary
  relative paths to custom configuration files may need additional support.
  A tool that atomically replaces a shared file's symlink can also leave a local
  copy; sync reports that copy as kept instead of overwriting it.
- An extension first installed only inside a profile whose native extension
  directory does not exist remains local to that profile. Sync does not promote
  local installations into the native setup. `--adopt` backs local copies up;
  it does not merge their contents.
- Shared plugin caches are writable by the agents themselves. Explicit plugin
  uninstall or update commands can affect the shared installation. Ditto's
  profile deletion preserves shared directories; it cannot undo an upstream
  plugin uninstall.

To repair existing managed profiles after installing a build containing these
fixes, run `ditto-cli sync --all --json` and inspect `shared_failed`,
`shared_kept`, and `repair_failed` for each profile. History copying and adoption
remain explicit options.
