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
│› work                  ││                                                    │
│  personal              ││Sign-in status                                      │
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
│   ↑↓ select · n new · e rename · l sign in · L sign out · r refresh · q quit │
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
| `r` | Refresh sign-in status |
| `q`, `Esc`, or `Ctrl+C` | Quit or close a dialog |

Sign-in status is checked in the background, so the list stays responsive while each CLI is asked. A spinner marks the tools still being checked.

The selected profile is remembered for the next run.

Renaming keeps the profile's logins, settings, and session history. The built-in `default` profile cannot be renamed.

## Command-line usage

The TUI is optional. Every launch command works directly from the shell:

```bash
# Profiles
ditto-cli create work
ditto-cli rename work client-a
ditto-cli list
ditto-cli status client-a   # sign-in state for Claude Code, Codex, and opencode
ditto-cli paths client-a

# Ditto CLI itself
ditto-cli update --check

# Launch a tool
ditto-cli claude client-a
ditto-cli codex client-a
ditto-cli opencode client-a
ditto-cli omp client-a

# Pass arguments to the underlying CLI after --
ditto-cli claude client-a -- --model opus
ditto-cli codex client-a -- --search
ditto-cli opencode client-a -- --model anthropic/claude-opus-5
ditto-cli omp client-a -- --model opus
```

If the profile name is omitted, Ditto CLI uses the last selected profile. Before the first selection it uses `default`.

You can also call the native authentication commands through a profile:

```bash
ditto-cli claude client-a -- auth login
ditto-cli codex client-a -- login
ditto-cli opencode client-a -- auth login
```

## Where credentials are stored

Ditto CLI does not ask for passwords, parse OAuth tokens, or keep credentials in its state file. Claude Code, Codex, and opencode authentication still runs through their installed CLIs. OMP authentication runs through `/login` inside OMP.

Codex keeps its auth state under the selected `CODEX_HOME`. Claude Code uses the selected `CLAUDE_CONFIG_DIR`; on macOS, sensitive Claude credentials remain in the system Keychain. opencode writes `auth.json` into the selected data directory. OMP keeps auth, settings, sessions, and caches under `~/.omp/profiles/<name>/agent`.

Ditto CLI's files are laid out like this:

```text
~/.ditto/
├── state.toml
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
| `DITTO_PROFILE` | Selected profile name exported to every launched tool |

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
