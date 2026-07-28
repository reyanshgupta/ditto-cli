# Ditto CLI

[![crates.io](https://img.shields.io/crates/v/ditto-cli.svg)](https://crates.io/crates/ditto-cli)
[![MIT license](https://img.shields.io/badge/license-MIT-6f42c1.svg)](LICENSE)
[![Rust 1.85+](https://img.shields.io/badge/rust-1.85%2B-6f42c1.svg)](https://www.rust-lang.org/)

Keep work, personal, and client Claude Code, Codex, opencode, and OMP logins apart.

Ditto CLI gives each profile its own authentication, settings, and session history. Pick a profile in the terminal, then launch Claude Code, Codex, opencode, or OMP. Your existing setup stays available as the `default` profile.

Ditto CLI takes its name from the shape-shifting Pokémon: one small tool, whichever coding identity you need.

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│                    Ditto CLI  choose a profile, then a tool                  │
└──────────────────────────────────────────────────────────────────────────────┘
┌ Profiles ──────────────┐┌ Selected profile ──────────────────────────────────┐
│  default  existing     ││work  Isolated profile                              │
│› work              ★   ││                                                    │
│  personal              ││★ Used when no profile is named                     │
│                        ││                                                    │
│                        ││Sign-in status                                      │
│                        ││Claude Code  ● Signed in                            │
│                        ││Codex        ○ Sign in required                     │
│                        ││opencode     ⠹ Checking                             │
│                        ││OMP          · Use /login inside OMP                │
│                        ││                                                    │
│                        ││Profile directories                                 │
│                        ││Claude Code  ~/.ditto/profiles/work/claude          │
│                        ││Codex        ~/.ditto/profiles/work/codex           │
│                        ││opencode     …/work/opencode/data/opencode          │
│                        ││OMP          ~/.omp/profiles/work/agent             │
└────────────────────────┘└────────────────────────────────────────────────────┘
┌──────────────────────────────────────────────────────────────────────────────┐
│              c Claude Code · x Codex · o opencode · p OMP                    │
│              ↑↓ select · n new · e rename · d default                        │
│              l sign in · L sign out · r refresh · q quit                     │
└──────────────────────────────────────────────────────────────────────────────┘
```

## Why use it?

Claude Code, Codex, opencode, and OMP keep user-level configuration and login state on disk. That works until you need separate accounts for different jobs. Manually moving auth files around is easy to get wrong, and it is hard to tell which account a new session will use.

Ditto CLI launches each tool with the selected profile:

| Tool | Setting used by Ditto CLI |
| --- | --- |
| Claude Code | `CLAUDE_CONFIG_DIR=~/.ditto/profiles/<name>/claude` |
| Codex | `CODEX_HOME=~/.ditto/profiles/<name>/codex` |
| opencode | `XDG_DATA_HOME`, `XDG_CONFIG_HOME`, and `XDG_STATE_HOME` under `~/.ditto/profiles/<name>/opencode` |
| OMP | `omp --profile <name>` and `OMP_PROFILE=<name>` |

opencode has no single home variable; it resolves its directories the XDG way, so Ditto CLI pins the three bases that hold credentials, configuration, and session state. `XDG_CACHE_HOME` is deliberately left alone so profiles keep sharing opencode's downloaded tooling.

No config files are swapped. Profiles remain independent, and switching only affects the process launched by Ditto CLI.

## Requirements

Install at least one of the supported CLIs:

- [Claude Code](https://code.claude.com/docs/en/setup)
- [OpenAI Codex CLI](https://github.com/openai/codex)
- [opencode](https://opencode.ai/docs/)
- [Oh My Pi](https://github.com/can1357/oh-my-pi)

Building Ditto CLI requires Rust 1.85 or newer.

## Install

From crates.io:

```bash
cargo install ditto-cli
```

From the latest GitHub source:

```bash
cargo install --git https://github.com/reyanshgupta/ditto-cli
```

From a local checkout:

```bash
git clone https://github.com/reyanshgupta/ditto-cli.git
cd ditto-cli
cargo install --path .
```

Make sure Cargo's binary directory is in your `PATH`:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

Ditto CLI installs as `ditto-cli`. macOS already uses `ditto` for its built-in file-copy utility at `/usr/bin/ditto`.

## Update

```bash
ditto-cli update           # install the newest crates.io release
ditto-cli update --check   # compare versions without installing
ditto-cli update --git     # install from the Git repository instead
```

`update` runs `cargo install` for you, so it needs Rust on your `PATH`. It stops early when you already have the newest release, and asking crates.io for the published version needs a network connection; without one it says so and installs anyway.

If you are running a binary from the [releases page](https://github.com/reyanshgupta/ditto-cli/releases) rather than a `cargo install` copy, `update` says so before it starts: cargo writes into its own bin directory and leaves the downloaded binary untouched. Replace that file yourself, or download the newer archive.

## Quick start

Open Ditto CLI:

```bash
ditto-cli
```

Then:

1. Press `n` and name the profile, such as `work`.
2. For Claude Code, Codex, or opencode, select the profile and press `l` to run the official login flow.
3. Press `c` for Claude Code, `x` for Codex, or `o` for opencode.
4. Press `p` to launch OMP, then use `/login anthropic` or `/login openai-codex` inside OMP.

Each tool keeps its own credentials. Signing in to one does not copy credentials into another.

## TUI controls

| Key | Action |
| --- | --- |
| `↑` / `↓` or `k` / `j` | Select a profile |
| `c` | Launch Claude Code |
| `x` | Launch Codex |
| `o` | Launch opencode |
| `p` | Launch OMP |
| `l` | Sign in with Claude Code, Codex, or opencode |
| `L` | Sign out, with confirmation |
| `n` | Create a profile |
| `e` | Rename the selected profile |
| `d` | Make the selected profile the default, or unset it |
| `r` | Refresh sign-in status |
| `q`, `Esc`, or `Ctrl+C` | Quit or close a dialog |

Sign-in status is checked in the background, so the list stays responsive while each CLI is asked. A spinner marks the tools still being checked.

The selected profile is remembered for the next run.

Pressing `d` marks the selected profile with a `★` and makes it the profile every command uses when you leave the name out. Pressing `d` on it again removes the mark. Unlike the remembered selection, the default stays put: running `ditto-cli claude personal` once does not move it. Any profile can be the default, including the built-in `default` one.

Renaming keeps the profile's settings, session history, and its Codex and opencode logins. Claude Code is the exception, and the rename dialog says so before you commit to it: see [renaming a profile signs Claude Code out](#renaming-a-profile-signs-claude-code-out). The built-in `default` profile cannot be renamed.

## Command-line usage

The TUI is optional. Every launch command works directly from the shell:

```bash
# Profiles
ditto-cli create work
ditto-cli rename work client-a
ditto-cli list
ditto-cli status client-a   # sign-in state for all four tools
ditto-cli paths client-a

# Bind a directory to a profile
ditto-cli workspace                  # what this directory launches with
ditto-cli workspace use client-a
ditto-cli workspace clear
ditto-cli workspace list
ditto-cli workspace auto off

# Which profile am I in?
ditto-cli indicator client-a         # report the status line setting
ditto-cli indicator client-a --on    # show the profile inside Claude Code
ditto-cli indicator client-a --off

# Ditto CLI itself
ditto-cli update --check

# Launch a tool
ditto-cli claude client-a
ditto-cli codex client-a
ditto-cli opencode client-a
ditto-cli omp client-a

# Short aliases
ditto-cli cc client-a       # Claude Code
ditto-cli cx client-a       # Codex
ditto-cli oc client-a       # opencode

# Pass arguments to the underlying CLI after --
ditto-cli claude client-a -- --model opus
ditto-cli codex client-a -- --search
ditto-cli opencode client-a -- --model anthropic/claude-opus-5
ditto-cli omp client-a -- --model opus
```

If the profile name is omitted, Ditto CLI asks the directory first (see [Workspaces](#workspaces)). Failing that it uses the profile marked as the default with `d` in the TUI, then the last selected profile, and before the first selection `default`. `ditto-cli list` marks the last selection with `*` and the default with a trailing `default`.

Profile names use lowercase letters, numbers, `.`, `-` and `_`, start with a letter or number, and are at most 32 characters. Uppercase names are rejected because OMP accepts only lowercase profile names.

You can also call the native authentication commands through a profile:

```bash
ditto-cli claude client-a -- auth login
ditto-cli codex client-a -- login
ditto-cli opencode client-a -- auth login
```

## Workspaces

A project usually belongs to one profile for its whole life. A workspace records that once, so every launch from the project uses the right profile without being told:

```bash
cd ~/code/client-a
ditto-cli workspace use client-a
ditto-cli claude              # runs as client-a, from here and every subdirectory
```

`ditto-cli workspace use` writes a `.ditto.toml` at the project root:

```toml
# Written by ditto-cli. Names the profile this directory launches with.
profile = "client-a"
```

It is a normal file: readable, editable by hand, and safe to commit if everyone on the project uses the same profile. Add it to `.gitignore` if they do not.

### Directories are bound automatically

Launching from a directory that nothing yet answers for binds it to the profile that launch used:

```console
$ cd ~/code/new-project
$ ditto-cli cc client-a
Bound this directory to 'client-a' (~/code/new-project/.ditto.toml).
```

The next `ditto-cli cc` from that project needs no name. Two directories are never bound this way: your home directory and the filesystem root, since a file at either would capture every project underneath it. Turn the behaviour off with `ditto-cli workspace auto off`.

### How a directory is resolved

Ditto walks from the current directory towards the filesystem root and stops at the first binding it finds, so the nearest one wins and a subdirectory can overrule the repository it sits in. A directory already answered for by an ancestor is left alone, which is what stops every subdirectory of a bound project from collecting a file of its own.

Naming a profile explicitly always wins, for that run only:

```console
$ cd ~/code/client-a          # bound to client-a
$ ditto-cli cc work
ditto-cli: using 'work' for this run; ~/code/client-a/.ditto.toml still names 'client-a'
```

The binding is untouched. Use `ditto-cli workspace use work` to change it for good.

### Directories you cannot leave a file in

For a repository that is not yours to add a file to, record the binding in Ditto's own registry instead:

```bash
ditto-cli workspace use client-a --global
```

That writes `~/.ditto/workspaces.json` rather than a file in the project. `ditto-cli workspace list` prints everything recorded there. Files cannot be listed, because they are found by walking up from wherever Ditto runs — `ditto-cli workspace` reports the one in effect where you are.

Where a directory has both, the file wins, so a project can carry a binding that overrides whatever this machine recorded for it. Between two directories, distance decides first: a nearer registry entry beats a further file.

### Clearing a binding

```console
$ ditto-cli workspace clear
Removed ~/code/client-a/crates/core/.ditto.toml.
~/code/client-a/crates/core now inherits 'client-a' from ~/code/client-a/.ditto.toml.
```

Clearing touches only the directory you name, never its ancestors, and reports what the directory falls back to.

## Knowing which profile you are in

Once a tool has taken over the terminal, nothing on screen says which profile it is running as. Ditto CLI answers that in the title for every tool, and inside Claude Code's own interface as well.

Claude Code gets a status line along the bottom of its interface:

```text
⬖ client-a · you@example.com
```

Launching Claude Code through Ditto CLI installs it, and `ditto-cli indicator` turns it on or off by hand. If the profile already has a `statusLine` of its own, Ditto CLI leaves it alone and says so: Claude Code renders only one, and replacing yours would quietly take it away. Nothing else in `settings.json` is touched.

The status line reads `DITTO_PROFILE`, so a `claude` you started yourself still reports the right profile as long as `CLAUDE_CONFIG_DIR` points at one.

Every tool also names the profile in the window and tab title:

```text
ditto:client-a — Codex — my-repo
```

All four tools write their own titles and keep updating them as you work, so a title set once before handing over is overwritten within moments. Instead, Ditto CLI runs the tool in a pseudoterminal and rewrites the title sequences on their way to the terminal, adding `ditto:<profile>` in front of whatever the tool called itself. You keep the tool's own title and gain the profile.

Everything else is forwarded byte for byte. Colours, hyperlinks, clipboard writes, mouse reporting, and anything the tool draws are untouched, and the tool still gets a real terminal, the right window size, and your keystrokes as it always did. Ditto CLI exits with the tool's own exit status.

To turn it off, set `DITTO_NO_PROXY=1` and Ditto CLI hands the terminal straight to the tool as it used to. The title stops naming the profile, and the Claude Code status line still works. Ditto CLI also steps aside on its own when output is redirected, or if the pseudoterminal cannot be opened.

## Renaming a profile signs Claude Code out

Claude Code stores credentials against the configuration directory it was pointed at, so moving that directory loses the sign-in. Renaming a profile moves `~/.ditto/profiles/<name>/claude`, and the credentials do not follow it.

Ditto CLI warns before the rename and tells you how to get back in:

```bash
ditto-cli claude client-a -- auth login
```

Codex and opencode keep their credentials in files inside the profile, so they survive a rename untouched.

## Where credentials are stored

Ditto CLI does not ask for passwords, parse OAuth tokens, or keep credentials in its state file. Claude Code, Codex, and opencode authentication still runs through their installed CLIs. OMP authentication runs through `/login` inside OMP.

Codex keeps its auth state under the selected `CODEX_HOME`. Claude Code uses the selected `CLAUDE_CONFIG_DIR`; on macOS the credentials themselves stay in the system Keychain, keyed to that directory's path, which is what keeps two profiles from sharing one login — and why [renaming a profile signs Claude Code out](#renaming-a-profile-signs-claude-code-out). opencode writes `auth.json` into the selected data directory. OMP keeps auth, settings, sessions, and caches under `~/.omp/profiles/<name>/agent`.

Ditto CLI's files are laid out like this:

```text
~/.ditto/
├── state.toml
├── workspaces.json      # directories bound without a file of their own
└── profiles/
    ├── work/
    │   ├── claude/
    │   ├── codex/
    │   └── opencode/
    │       ├── config/opencode/
    │       ├── data/opencode/      # auth.json lives here
    │       └── state/opencode/
    └── personal/
        ├── claude/
        ├── codex/
        └── opencode/
```

The nested `opencode/` directory is opencode's own doing: it appends its name to every XDG base it is given.

OMP profiles remain in OMP's native profile directory:

```text
~/.omp/profiles/<name>/agent/
```

Directories are created with user-only permissions on Unix systems.

The `default` profile points to `~/.claude`, `~/.codex`, opencode's own `~/.local/share/opencode` and `~/.config/opencode` (or wherever your `XDG_*` variables already send them), and OMP's native `~/.omp/agent` profile. It exposes your existing setup without copying or migrating anything. For opencode the `default` profile resolves the same XDG bases opencode would pick on its own, so pointing at it changes nothing.

## Environment variables

| Variable | Purpose |
| --- | --- |
| `DITTO_HOME` | Move Ditto CLI's state and profile directory from `~/.ditto` |
| `DITTO_CLAUDE_BIN` | Override the `claude` executable |
| `DITTO_CODEX_BIN` | Override the `codex` executable |
| `DITTO_OPENCODE_BIN` | Override the `opencode` executable |
| `DITTO_OMP_BIN` | Override the `omp` executable |
| `DITTO_PROFILE` | Selected profile name exported to every launched tool, and what Claude Code's status line reports |
| `DITTO_NO_PROXY` | Hand the terminal straight to the tool, leaving the title to it |
| `NO_COLOR` | Draw the Claude Code status line without colour |

Example:

```bash
DITTO_HOME="$HOME/.config/ditto" ditto-cli
```

`ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `OPENAI_API_KEY`, and `OPENCODE_API_KEY` are inherited by launched tools. They may override a saved subscription login, so Ditto CLI shows a warning when one is set.

Launching opencode sets `XDG_DATA_HOME`, `XDG_CONFIG_HOME`, and `XDG_STATE_HOME` for that process and anything it starts, including commands opencode's own tools run.

## Remove Ditto CLI

Uninstall the binary:

```bash
cargo uninstall ditto-cli
```

Profiles are deliberately left alone. If you no longer need their settings, sessions, or credentials, remove `~/.ditto` and any matching `~/.omp/profiles/<name>` directories yourself.

## Development

```bash
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

## License

Ditto CLI is available under the [MIT License](LICENSE).

Ditto CLI is an independent project. It is not affiliated with Anthropic, OpenAI, Nintendo, or The Pokémon Company.
