# Ditto CLI

Ditto CLI is a Rust terminal application for keeping Claude Code, Codex, opencode, OMP, Prime Agent, and Pi profiles isolated. It stores separate configuration and authentication roots for each profile, then launches the official CLIs with the selected profile environment. Users can manage profiles through either the Ratatui interface or equivalent command-line subcommands.

This file is for agents and scripts. The first half is how to *drive* Ditto; the second half is how to *change* it. `CLAUDE.md` points here so there is one copy to keep true.

## Driving Ditto without a terminal

**Never run bare `ditto-cli`.** With no subcommand it opens the interactive picker, which needs a real terminal. Without one it exits 1 and tells you which commands to use instead. Every setting the picker can change has a subcommand, so nothing is out of reach.

Add `--json` to any reporting command and it prints one JSON object on stdout. It is global, so either side of the subcommand works:

```bash
ditto-cli list --json
ditto-cli --json list
```

Errors are prose on stderr with exit 1, or `{"error": "..."}` on stderr with exit 1 when `--json` is set. Success is always exit 0. `--json` covers `list`, `status`, `paths`, `create`, `rename`, `delete`, `sync`, `default`, `workspace`, and `indicator`. The launch commands (`claude`, `codex`, `opencode`, `omp`, `prime-agent`, `pi`) hand the terminal to another program and exit with *its* status, so they have nothing to report; `update` and the hidden `statusline` print prose only, and `shell-init` prints a shell script for a shell to read.

### The config an agent can edit

Ditto's own state is one file, `$DITTO_HOME/state.toml` (default `~/.ditto/state.toml`):

```toml
last_profile = "personal"     # moved by every launch
default_profile = "work"      # the pin; set deliberately, survives launches
```

Prefer the commands over editing this file. They validate the name, refuse to point at a profile that does not exist, and write atomically with owner-only permissions:

```bash
ditto-cli default                # report the pin
ditto-cli default work           # pin 'work'
ditto-cli default --clear        # release it
```

Precedence when a command omits the profile name: the directory's binding (see `workspace`), then `default_profile`, then `last_profile`, then the reserved `default` profile. `list --json` reports the saved values plus `fallback_profile`, which is the answer that precedence produces — read that instead of re-deriving the rule. A launch that reaches the saved values says so on stderr, naming the profile and which value chose it; the reporting commands stay quiet.

A profile's own directories are the tools' config, not Ditto's. `ditto-cli paths <profile> --json` gives you the roots; edit files under them with ordinary tools:

```bash
claude_home=$(ditto-cli paths work --json | jq -r .claude)
# then edit "$claude_home/settings.json", "$claude_home/.claude.json", etc.
```

Prime Agent is pointed at a profile with `PRIME_AGENT_CODING_AGENT_DIR` and, for managed profiles, `PRIME_AGENT_SESSION_DIR`. Its ordinary provider logins live in the selected `auth.json`. Current Prime Agent releases keep Prime Inference's Prime CLI credential separately at `~/.prime/config.json` and expose no override for that path, so Prime Inference remains shared; inherited API-key environment variables are ambient too. Prime Agent's daemon socket is also user-wide, so its agent-list and attach commands can see running agents from other profiles even though each runtime receives its selected auth and session roots. Status reports only the provider entries in the profile's own `auth.json`, excluding MCP, trace-sharing, and web-search credentials.

Pi is pointed at a profile with `PI_CODING_AGENT_DIR` and, for managed profiles, `PI_CODING_AGENT_SESSION_DIR`. Provider logins live in the selected `auth.json`; reusable settings, packages, skills, extensions, prompts, themes, trust decisions, and managed tools link back to the user's own Pi directory. `models.json` stays private because it may contain credentials. Pi has no standalone login command, so launch it and use `/login` or `/logout` inside its interface. Status counts entries in the profile's isolated `auth.json`; inherited API-key variables remain ambient.

The one file Ditto also writes is `<claude_home>/settings.json`, on two occasions and no others. A launch installs the `statusLine` key; if that key holds something Ditto did not write, Ditto reports `foreign` and leaves it alone rather than replacing it. Creating a profile copies the rest of `~/.claude/settings.json` into it, since `CLAUDE_CONFIG_DIR` moves the whole user settings layer and the profile would otherwise have no permission mode, model, or hooks. `ditto-cli sync <profile>` does the same copy on demand, filling in keys the profile has never set and leaving the ones it has unless `--overwrite` is passed. `statusLine` is the one key never copied: it names the profile it was installed for.

`indicator --keep-mine` is the one thing that changes a `statusLine` Ditto did not write, and it keeps the original: the installed command becomes `ditto-cli statusline --with '<their command>'`, which runs their status line with Claude Code's payload and prints the profile in front of it. `indicator --off` reads the original back out of that command and puts it in place unchanged.

Claude Code reads settings from several files and the profile's is the lowest-ranking of them, so an entry can be installed and never drawn. `Indicator::shadowed` says so, and is applied where a person is being told what happened rather than during a launch, since the answer depends on the directory the command was typed in.

The other thing a launch writes is a repair, and it is the one case where Ditto rewrites something a tool wrote. A skill installer records what it installed as a *relative* symbolic link computed from the configuration directory the tool was pointed at; under a profile that directory is one of `shared.rs`'s links, so the operating system creates the record in the user's own directory instead and the relative path in it now leads somewhere else. The skill is on disk and readable from nowhere. `shared::repair_for` re-reads such a link against the path the installer was handed and, only when that names something that exists, rewrites it as an absolute link — reported on stderr at launch, and under `repaired` by `sync`, which does every tool rather than the one being started.

### Recipes

```bash
# Create a profile and make it the one unnamed commands use.
ditto-cli create work --json
ditto-cli default work --json

# Bring a profile's Claude Code settings up to the user's own. `copied` names
# the keys written, `kept` the ones the profile had already answered, and
# `repaired` the installed links that were pointing at nothing.
ditto-cli sync work --json

# Which profiles exist, and which would a bare command use?
ditto-cli list --json | jq '{fallback: .fallback_profile, names: [.profiles[].name]}'

# Is a profile signed in? Per tool, with stable keys.
ditto-cli status work --json | jq '.tools[] | select(.signed_in | not) | .tool'

# Delete a profile. Destructive, so --yes is required; without it the
# command lists the directories it would remove and exits 1.
ditto-cli delete work --yes --json

# Run a tool inside a profile. Everything after -- goes to that tool.
ditto-cli claude work -- --model opus
ditto-cli prime-agent work -- --model claude-opus-4-1
ditto-cli pi work -- --model anthropic/claude-opus-4-6
```

`DITTO_HOME` relocates the whole store, which is what makes a hermetic run possible:

```bash
DITTO_HOME=$(mktemp -d) ditto-cli create scratch --json
```

### Stable output contract

These strings are an interface. Changing one breaks callers, so treat it as a deliberate break, not a wording fix.

| Field | Values |
| --- | --- |
| `tools[].tool` | `claude`, `codex`, `opencode`, `omp`, `prime-agent`, `pi` |
| `tools[].status` | `signed_in`, `signed_out`, `unavailable` |
| `indicator.outcome` | `installed`, `already_on`, `alongside`, `removed`, `restored`, `off`, `foreign`, `shadowed` |
| `profiles[].managed` | `false` only for the reserved `default` profile |

They live in `Tool::key`, `AuthStatus::key`, and `Indicator::key`, each kept deliberately apart from the `label`/`describe` method beside it. Labels are written for a person and may be reworded freely; keys may not.

## Editing Ditto itself

### Layout

Everything is one binary crate under `src/`. There is no `tests/` directory: unit tests live in a `mod tests` at the bottom of the file they cover.

| File | Holds |
| --- | --- |
| `main.rs` | Subcommand dispatch, and the human/JSON reporting for each. The picker's terminal guard. |
| `cli.rs` | The clap types. Parsing only — no behaviour, no filesystem. |
| `profile.rs` | `Store` and `Profile`: where a profile's directories are, creating, renaming, deleting, and `state.toml`. Also `write_private_file`, the atomic owner-only write everything else uses. |
| `launch.rs` | `Tool`, running a tool with the profile's environment, and reading each tool's sign-in state. |
| `indicator.rs` | Claude Code's `statusLine` in `settings.json`, the `statusline` subcommand that draws it, and terminal titles. |
| `settings.rs` | Reading and writing Claude Code's `settings.json`, and copying the user's own settings into a profile at creation or on `sync`. |
| `shared.rs` | The allowlist of configuration and extension paths a profile links back to the user's own — skills, subagents, commands, hooks, plugins — the linking itself, and `repair`, which mends the links an installer wrote through one of Ditto's. |
| `ui.rs` | The Ratatui picker. |
| `proxy.rs` | The Unix pseudoterminal that rewrites title sequences on their way out. `#[cfg(unix)]`. |
| `herdr.rs` | Reading `HERDR_PANE_ID`, and reporting the profile to herdr as pane metadata. Why the proxy is skipped inside a herdr pane. |
| `program.rs` | `PATH` lookup honouring `PATHEXT`, so npm's `claude.cmd` shims are found on Windows. |
| `shell.rs` | The `shell-init` functions that route a tool's own name through Ditto, and reading `SHELL` to pick a dialect. |
| `update.rs` | `ditto-cli update`, which shells out to `cargo install`, or names the npm command when npm is what installed this copy. |

### Checks

Run all three before calling a change done. CI runs the same.

```bash
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

### Conventions

**Comments say why, not what.** This is the strongest convention in the codebase and the easiest to break. Comments are full prose sentences explaining the reasoning a reader could not recover from the code — a constraint another tool imposes, a failure mode being avoided, a tempting alternative that is wrong. Do not add comments that restate the line below them.

```rust
/// Claude Code stores its credentials against the directory it was pointed at,
/// and renaming moves that directory, so the sign-in cannot survive. Saying so
/// before the move beats leaving it to be discovered at the next launch.
```

**Errors** are `anyhow::Result`, with `bail!` for a new one and `.with_context(|| ...)` when passing one up. Context names the path or profile involved. Messages are lowercase, specific, and where possible say what to do next: `` "profile '{name}' does not exist; create it with `ditto-cli create {name}`" ``.

**Writing files.** Use `profile::write_private_file`. It writes a temporary carrying the process id, sets owner-only permissions, then renames over the target, so a crash cannot leave a half-written settings file for a tool to refuse to start from. Never `fs::write` into a profile directory. New directories go through `secure_directory` for the same reason.

**Never clobber a user's configuration.** Ditto owns exactly one key in `settings.json`. The `Foreign` outcome exists so that a `statusLine` Ditto did not write is reported and left in place, `Alongside` exists so that the way to have both is to keep theirs rather than to overwrite it, and `settings::copy` withholds every key the profile has already answered unless `--overwrite` asks for it. Extend that pattern rather than working around it: when Ditto has to sit where something of the user's already is, run theirs, keep everything it carried, and be able to hand it back.

**Isolate accounts, share everything else.** A profile exists to be signed in as somebody else, not to be a different working environment. Credentials and session state stay in the profile; skills, subagents, commands, hooks, plugins and the memory files are linked back to the user's own configuration by `shared.rs`, so there is one copy edited in one place. That list is an allowlist and must stay one: teaching Ditto a new extension directory late costs a missing feature, and sharing a credential file by accident costs the isolation the tool exists for. Claude Code's `settings.json` is the one thing copied rather than linked, because Ditto writes the status line into it.

**Cross-platform.** Windows is supported. Gate with `#[cfg(unix)]` / `#[cfg(windows)]`, and prefer `crossterm` over writing escape sequences directly. `program.rs` compiles on every platform even though only Windows calls it, on the reasoning that a rule nobody can exercise is a rule nobody checks — keep that property when touching it.

**Profile names** are validated once, in `profile::validate_profile_name`, against the strictest tool: OMP takes the name verbatim and requires `^[a-z0-9][a-z0-9._-]{0,63}$`, rejecting trailing dots and Windows device names. Ditto applies those rules at creation so a name cannot be created that later fails only when OMP launches. Do not loosen this in one place.

**Tests** use `tempfile::tempdir()` with `Store::new(root, user_home)`, which is `#[cfg(test)]` precisely so tests never touch a real home directory. Test names are sentences about the behaviour being protected (`deletes_a_profile_and_the_state_that_named_it`), and a test guarding a subtle rule carries a comment saying which rule.

### Adding a subcommand

1. Add the variant to `Command` in `cli.rs`, with a doc comment — clap shows it in `--help`. Give it an `Args` struct if it takes more than a name.
2. Add a parse test in `cli.rs`, including the combinations that must be rejected.
3. Add the arm to `run()` in `main.rs` and write the handler. Report through the `report` helper so the command answers in both prose and JSON; do not `println!` a result outside it.
4. If it touches the store, put the filesystem and `state.toml` work in `profile.rs` and keep `main.rs` to presentation.
5. Destructive commands require an explicit `--yes`, and the refusal lists what would be lost. See `delete_profile`.
6. Update `README.md` (the command-line usage section) and this file's tables if you added output keys.

### Adding a tool

Extend `Tool` in `launch.rs`: `ALL`, `label`, `key`, `executable` (with its `DITTO_*_BIN` override), the status args in `auth_status`, and `AuthOperation::args`. Then give it a home in `Profile` and `Store::managed_profile`/`default_profile` in `profile.rs`, add it to `create_profile`'s directory list, and surface it in `paths`, `status`, and `ui.rs`. Add its configuration and extension paths to the allowlist in `shared.rs` and its credential and session paths to nothing, so a profile of the new tool starts as the same working environment signed in as somebody else. `shell.rs` needs nothing: it writes a function for every entry in `Tool::ALL`, and a test fails if one is missing.

### Versioning

Ditto CLI is pre-1.0 and uses `0.x.y` semantic versions.

- Reserve `0.x.0` releases for major project milestones, intentionally breaking changes, or substantial feature sets that establish a new release series.
- Use patch increments for routine features, fixes, documentation, workflow changes, and other incremental work. For example, release a routine change after `0.2.0` as `0.2.1`, not `0.3.0`.
- Keep the version in `Cargo.toml`, the resolved package version in `Cargo.lock`, and the Git tag `vX.Y.Z` aligned when preparing a release.

### Releasing

**The version bump does not publish anything. The tag does.** `.github/workflows/publish.yml` fires on a pushed tag matching `v[0-9]+.[0-9]+.[0-9]+`, and nothing else does. A `Release X.Y.Z` commit sitting on `main` with no tag leaves crates.io on the previous version, and nothing reports a failure, because no run happened. 0.3.2 was lost this way.

```bash
# 1. Bump Cargo.toml, let the lockfile catch up, commit both.
cargo check
git commit -am "Release X.Y.Z"
git push origin main

# 2. Tag that commit and push the tag. This is the step that publishes.
git tag -a vX.Y.Z -m "Ditto CLI X.Y.Z"
git push origin vX.Y.Z
```

The workflow checks the tag against `cargo metadata`'s version and stops if they disagree, runs the suite on Linux, macOS and Windows, publishes with `cargo publish --locked`, builds the per-platform binaries, updates the Homebrew tap, and publishes the npm packages. Confirm the release landed rather than assuming the push was enough:

```bash
gh run list --workflow=publish.yml --limit 1
# crates.io turns away a request that names no client, and the body it answers
# with has no version in it, so a missing User-Agent reads as an unpublished
# crate rather than as the refusal it is.
curl -s -H 'User-Agent: ditto-cli release check' \
  https://crates.io/api/v1/crates/ditto-cli | jq -r .crate.max_version
npm view @reyanshgupta/ditto-cli version
```

`workflow_dispatch` re-runs a release from an existing tag, which is how to retry one whose publish succeeded but whose later jobs did not: every publishing job asks its registry first and skips a version already there. The tap and npm jobs are the steps that check out `main` rather than the tag, so a fix to the rendering tooling reaches a tag cut before that fix existed. The tap job needs `TAP_GITHUB_TOKEN`, which a contributor's fork will not have, so a missing one warns and skips rather than failing a release that already went out.

### Distribution

Five channels carry the same four binaries, and all of them are rendered from what the release published rather than rebuilt, so none can describe an archive nobody downloaded.

| Channel | Built by | Notes |
| --- | --- | --- |
| crates.io | `cargo publish --locked` | The only one that compiles on the user's machine. |
| Releases page | The `build` job | The archives every other channel reads. |
| Homebrew tap | `.github/homebrew/render-formula.sh` | Checksums read out of `SHA256SUMS`. |
| npm | `.github/npm/render-packages.sh` | Binaries unpacked out of the archives themselves. |
| `cargo binstall` | `[package.metadata.binstall]` | No job; it reads the archives off the releases page. |

npm is five packages, not one. npm cannot publish one package carrying four platforms' binaries, so a release is a wrapper package plus a binary package per platform that npm installs only when `os` and `cpu` match. The binary packages publish first: the wrapper names them as optional dependencies, so a wrapper reaching the registry ahead of them is a version that installs and then has nothing to run. The wrapper's `bin` is the launcher in `.github/npm/launcher.js`, which finds the binary package that was installed and hands the terminal to it — it also sets `DITTO_INSTALL_SOURCE=npm`, which is how `update.rs` knows to name the npm command instead of reaching for cargo.

npm authenticates the job as the workflow, not as a token. Each of the five packages names `publish.yml` in this repository as its trusted publisher, and the OIDC identity the runner mints is what proves it really is this file running — which is why the job asks for `id-token: write` and why renaming the workflow file would break publishing until the five configurations were repointed at the new name. Provenance comes with it, unasked.

Configuring that needs the package to already exist, on npm's side, so the first version of each was published by hand. Neither the website nor `npm trust` will attach a publisher to a name nobody has published. A sixth package, should there ever be one, is the same story: publish it once from a laptop, then

```bash
npm trust github @reyanshgupta/ditto-cli-<platform> \
  --repo reyanshgupta/ditto-cli --file publish.yml --allow-publish
```

`NPM_TOKEN` is a fallback and not the mechanism. npm reaches for an OIDC environment before it reaches for a token, so the token only publishes a package whose trusted publisher is not configured, and is ignored once one is. Do not put a classic *publish* token there: it asks for a one-time password on every write when the account requires 2FA for them, and there is nobody at a runner to type one, so the job gets as far as signing provenance and uploading before npm turns it away with `EOTP`. npm began restricting bypass-2FA granular tokens on 31 July 2026 and removes their publishing entirely in January 2027, which is the other reason the token is the fallback rather than the plan.

The binstall metadata spells out the release's archive names because they are not the ones it guesses. Renaming an archive in the packaging steps means changing `pkg-url` in `Cargo.toml` in the same commit, or `cargo binstall ditto-cli` starts 404ing with nothing to say why.
