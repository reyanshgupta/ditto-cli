# Ditto CLI

[![crates.io](https://img.shields.io/crates/v/ditto-cli.svg)](https://crates.io/crates/ditto-cli)
[![npm](https://img.shields.io/npm/v/@reyanshgupta/ditto-cli.svg)](https://www.npmjs.com/package/@reyanshgupta/ditto-cli)
[![MIT license](https://img.shields.io/badge/license-MIT-6f42c1.svg)](LICENSE)
[![Rust 1.85+](https://img.shields.io/badge/rust-1.85%2B-6f42c1.svg)](https://www.rust-lang.org/)

**Keep your work, personal, and client logins apart across your coding agents — Claude Code, Codex, opencode, Gemini CLI, Copilot, Cursor, Grok, Goose, and [thirty more](#supported-agents).**

Each profile gets its own credentials and conversation history. Everything you set up once — skills, subagents, slash commands, hooks, plugins, memory files — stays shared, so a profile is the same working environment signed in as somebody else. Your existing setup keeps working as the `default` profile; nothing is moved, copied, or migrated.

```bash
ditto-cli create work        # a profile with logins of its own
ditto-cli claude work        # launch Claude Code inside it
ditto-cli workspace use work # or bind the project once and stop typing the name
```

Pick a profile, then a tool:

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│            Ditto CLI  choose a profile, then Enter to pick a tool            │
└──────────────────────────────────────────────────────────────────────────────┘
┌ Profiles ──────────────┐┌ Selected profile ──────────────────────────────────┐
│  default  existing     ││work  Isolated profile                              │
│  personal              ││                                                    │
│› work  ★               ││★ Used when no profile is named                     │
│                        ││                                                    │
│                        ││Sign-in status                                      │
│                        ││Claude Code  ● Signed in                            │
│                        ││Codex        ○ Sign in required                     │
│                        ││fx           ● Signed in                            │
│                        ││opencode     ○ Sign in required                     │
│                        ││OMP          ● Signed in                            │
│                        ││Prime Agent  ● Signed in                            │
│                        ││Pi           ○ Sign in required                     │
│                        ││Gemini CLI   ● Signed in                            │
│                        ││Grok         ○ Sign in required                     │
│                        ││                                                    │
│                        ││Profile directories                                 │
│                        ││Claude Code  ~/.ditto/profiles/work/claude          │
│                        ││Codex        ~/.ditto/profiles/work/codex           │
│                        ││fx           ~/.ditto/profiles/work/fx-home/.fx     │
│                        ││opencode     …o/profiles/work/opencode/data/opencode│
│                        ││OMP          ~/.omp/profiles/work/agent             │
│                        ││Prime Agent  ~/.ditto/profiles/work/prime-agent     │
│                        ││Pi           ~/.ditto/profiles/work/pi              │
│                        ││Gemini CLI   …itto/profiles/work/gemini-home/.gemini│
│                        ││Grok         ~/.ditto/profiles/work/grok            │
└────────────────────────┘└────────────────────────────────────────────────────┘
┌──────────────────────────────────────────────────────────────────────────────┐
│                  c Claude Code · x Codex · f fx · o opencode                 │
│                      p OMP · a Prime · i Pi · ⏎ any tool                     │
│                   ↑↓ select · n new · e rename · d default                   │
│                  l sign in · L sign out · r refresh · q quit                 │
└──────────────────────────────────────────────────────────────────────────────┘
```

Ditto CLI takes its name from the shape-shifting Pokémon: one small tool, whichever coding identity you need.

## Contents

**Getting going** — [How it works](#how-it-works) · [Supported agents](#supported-agents) · [Install](#install) · [Update](#update) · [Quick start](#quick-start)

**Everyday use** — [TUI controls](#tui-controls) · [Command-line usage](#command-line-usage) · [Which profile a command uses](#which-profile-a-command-uses) · [Workspaces](#workspaces) · [Shell integration](#shell-integration) · [Knowing which profile you are in](#knowing-which-profile-you-are-in)

**What a profile keeps and shares** — [Settings a new profile inherits](#settings-a-new-profile-inherits) · [Skills, subagents, and everything else you set up once](#skills-subagents-and-everything-else-you-set-up-once) · [Where credentials and files are stored](#where-credentials-and-files-are-stored)

**Reference** — [Scripting and agents](#scripting-and-agents) · [Environment variables](#environment-variables) · [Renaming a profile signs Claude Code out](#renaming-a-profile-signs-claude-code-out) · [Windows notes](#windows-notes) · [Remove Ditto CLI](#remove-ditto-cli) · [Development](#development)

## How it works

Every coding agent keeps its configuration and login somewhere under your home directory, and every one of them can be told to look somewhere else: through a variable of its own (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, `GEMINI_CLI_HOME`), through the XDG base directories, or, when it honours neither, through `HOME` itself. Ditto CLI keeps one such place per profile and starts the agent pointed at it:

```bash
ditto-cli claude work    # claude, with CLAUDE_CONFIG_DIR=~/.ditto/profiles/work/claude
ditto-cli gemini work    # gemini, with GEMINI_CLI_HOME pointing into the same profile
```

Nothing is moved, copied, or swapped, and only the process Ditto CLI started is affected. The profile is the same working environment signed in as somebody else: skills, subagents, commands, hooks, plugins, and memory files are linked back to yours, and only credentials, sessions, and caches stay inside it. Which lever each agent uses is in the table below.

<details>
<summary>Why opencode, Prime Agent, and Pi take more than one variable</summary>

<br>

opencode has no single home variable; it resolves its directories the XDG way, so Ditto CLI pins the three bases that hold credentials, configuration, and session state. `XDG_CACHE_HOME` is deliberately left alone so profiles keep sharing opencode's downloaded tooling. Those variables are set for the launched process and anything it starts, including commands opencode's own tools run.

Prime Agent and Pi settings can name a session directory, so managed profiles set `PRIME_AGENT_SESSION_DIR` or `PI_CODING_AGENT_SESSION_DIR` explicitly. That keeps transcripts and local harness state inside the selected profile even while reusable settings and capabilities are linked to yours.

</details>

## Supported agents

Ditto CLI is not tied to any one agent, vendor, or launcher: anything that runs in a terminal and can be pointed at a directory can have a profile. Every agent is launched by its own command name, and everything after `--` is the agent's own:

```bash
ditto-cli <command> <profile> -- <arguments>
ditto-cli grok work -- login
```

`ditto-cli --help` lists them all. Three levers cover every agent:

- **A variable of its own**, set to a directory inside the profile — `CODEX_HOME`, `GROK_HOME`.
- **The XDG bases**, pinned inside the profile, for agents that follow that convention. `XDG_CACHE_HOME` is left shared.
- **A private `HOME`**, for agents that derive their paths from the home directory and honour nothing else. The profile's home mirrors your real one entry by entry — shell startup files, Git configuration, SSH, toolchains — except for the agent's own directory, so commands the agent runs still find your setup.

| Agent | Command | Pointed at the profile by | Sign-in state read from |
| --- | --- | --- | --- |
| Claude Code | `claude` (`cc`) | `CLAUDE_CONFIG_DIR` | `claude auth status`; on macOS the credential is in the Keychain, keyed to that directory |
| Codex | `codex` (`cx`) | `CODEX_HOME` | `codex login status` |
| fx | `fx` | private `HOME`, with `FX_DISABLE_KEYCHAIN=1` so the login is a file inside it | `fx status --json` |
| opencode | `opencode` (`oc`) | `XDG_DATA_HOME`, `XDG_CONFIG_HOME`, and `XDG_STATE_HOME` | `opencode auth list` |
| OMP | `omp` | `omp --profile <name>` and `OMP_PROFILE` | credentials in its `agent.db` |
| Prime Agent | `prime-agent` (`pa`) | `PRIME_AGENT_CODING_AGENT_DIR` and `PRIME_AGENT_SESSION_DIR` | `auth.json` |
| Pi | `pi` | `PI_CODING_AGENT_DIR` and `PI_CODING_AGENT_SESSION_DIR` | `auth.json` |
| Gemini CLI | `gemini` | `GEMINI_CLI_HOME` | `oauth_creds.json` |
| Qwen Code | `qwen` | `QWEN_HOME` | `oauth_creds.json` |
| OpenClaude | `openclaude` | `OPENCLAUDE_CONFIG_DIR` | `.credentials.json`; the macOS Keychain, which it prefers, is user-wide |
| Copilot | `copilot` | `COPILOT_HOME` | not readable: the token is in the OS keychain, keyed by GitHub account |
| Cursor Agent | `cursor-agent` | `CURSOR_CONFIG_DIR`, with `AGENT_CLI_CREDENTIAL_STORE=file` | not readable |
| Grok | `grok` | `GROK_HOME` | `auth.json` |
| Devin | `devin` | XDG bases | `credentials.toml` |
| Kimi Code | `kimi` | `KIMI_CODE_HOME` | `credentials/` |
| Cline | `cline` | `CLINE_DIR` | `data/settings/providers.json` |
| Codebuff | `codebuff` | private `HOME` (`~/.config/manicode`) | `credentials.json` |
| Continue | `cn` | `CONTINUE_GLOBAL_DIR` | none: current builds have no sign-in |
| Command Code | `command-code` | private `HOME` | `auth.json` |
| Hermes Agent | `hermes` | `HERMES_HOME` | `auth.json`, `.env` |
| OpenClaw | `openclaw` | `OPENCLAW_STATE_DIR` | `credentials/`, `secrets.json`, `.env` |
| Mistral Vibe | `vibe` | `VIBE_HOME`, with `VIBE_TEST_DISABLE_KEYRING=1` | `.env` |
| Rovo Dev | `acli` | private `HOME` (`~/.rovodev` and `~/.acli`) | `~/.acli` |
| Amp | `amp` | XDG bases | `secrets.json` |
| Droid | `droid` | private `HOME`, with `FACTORY_DISABLE_KEYRING=1` | not readable |
| Goose | `goose` | XDG bases, with `GOOSE_DISABLE_KEYRING=1` | `secrets.yaml` |
| Aider | `aider` | private `HOME` | `oauth-keys.env`; API keys come from the environment |
| Crush | `crush` | XDG bases | the data directory's `crush.json` |
| Kilo Code | `kilo` | XDG bases | `auth.json` |
| Kiro | `kiro-cli` | private `HOME` | `data.sqlite3` |
| Auggie | `auggie` | private `HOME` | `session.json` |
| Antigravity | `agy` | private `HOME` | not readable: the Google login is in the OS keychain with no way out, so only `GEMINI_API_KEY` is per profile |
| MiMo Code | `mimo` | XDG bases | `auth.json` |
| Ante | `ante` | `ANTE_HOME` | `auth/` |
| Trae | `traecli` | private `HOME` | `cli/auth.json`; the least certain entry, since Trae's documentation names no variable |
| Autohand | `autohand` | `AUTOHAND_HOME` | not readable: the login shares `config.json` with the settings |

"Not readable" means `ditto-cli status` reports the agent as `unavailable` even when it is installed: the login is somewhere Ditto CLI has no file to check, and a keychain entry is one every profile shares besides. Sign in with the agent's own command through Ditto CLI — `ditto-cli grok work -- login` — or from inside it; the `sign_in` field of `ditto-cli create --json` names the right form for each.

The rule for what a profile shares is the same for every row: settings, instructions, skills, commands, and plugins are linked back to yours; credentials, sessions, and caches are not, and neither are MCP configuration files, because the ones that carry OAuth tokens are named the same as the ones that do not. Each entry's own caveats — an undocumented variable, a keychain with no switch, a fork's macOS data directory — are in the comment beside it in [`src/tools.rs`](src/tools.rs), and `DITTO_<AGENT>_BIN` overrides any agent's executable (`DITTO_CURSOR_AGENT_BIN=agent`).

## Install

Ditto CLI is useful once at least one [supported agent](#supported-agents) is installed.

Take whichever channel you already use. They all install the same binary, and the command is `ditto-cli` either way:

| Channel | Command | Notes |
| --- | --- | --- |
| **Homebrew** | `brew install reyanshgupta/tap/ditto-cli` | macOS and Linux. Prebuilt, so no Rust toolchain. |
| **npm** | `npm install -g @reyanshgupta/ditto-cli` | macOS, Linux, and Windows. The same prebuilt binaries. |
| **binstall** | `cargo binstall ditto-cli` | Takes the released binary rather than compiling one. |
| **Cargo** | `cargo install ditto-cli` | Builds from source; needs Rust 1.85 or newer. |

If you installed Claude Code or Codex with npm, npm is already here. You can also try Ditto CLI without installing it:

```bash
npx @reyanshgupta/ditto-cli list
```

The npm package is scoped because plain `ditto-cli` on npm belongs to an unrelated project. The command it installs is still `ditto-cli`.

`cargo binstall` is a good deal quicker than `cargo install`, because a source build compiles a bundled SQLite before it links anything. To build from the latest source, or from a checkout:

```bash
cargo install --git https://github.com/reyanshgupta/ditto-cli

git clone https://github.com/reyanshgupta/ditto-cli.git
cd ditto-cli
cargo install --path .
```

After a Cargo install, make sure Cargo's binary directory is on your `PATH`:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
```

On Windows, `cargo install` already writes into `%USERPROFILE%\.cargo\bin`, which rustup puts on your `PATH`, and an npm install needs nothing further either.

Ditto CLI installs as `ditto-cli`, not `ditto`: macOS already uses that name for its built-in file-copy utility at `/usr/bin/ditto`.

macOS, Linux, and Windows are all supported, and every command below behaves the same on each. A few smaller things differ on Windows — see [Windows notes](#windows-notes).

### Update

A Homebrew install updates through Homebrew:

```bash
brew upgrade ditto-cli
```

An npm install updates through npm:

```bash
npm install -g @reyanshgupta/ditto-cli@latest
```

A Cargo install updates itself:

```bash
ditto-cli update           # install the newest crates.io release
ditto-cli update --check   # compare versions without installing
ditto-cli update --git     # install from the Git repository instead
```

`update` runs `cargo install` for you, so it needs Rust on your `PATH`. It stops early when you already have the newest release. During a build it shows a compact animated status instead of Cargo's full compilation log; if the build fails, it prints that log so the error is not hidden. Asking crates.io for the published version needs a network connection; without one it says so and installs anyway.

`update` only replaces a copy in Cargo's own bin directory. Run it against a Homebrew install, or a binary from the [releases page](https://github.com/reyanshgupta/ditto-cli/releases), and it says so before it starts, then leaves that copy alone: a `cargo install` would land in a different directory and shadow the existing binary rather than replace it. Use `brew upgrade ditto-cli` or the newer archive instead.

Against an npm install it does not start at all. npm keeps its copy inside its own tree, so `update` names the npm command and stops rather than leaving you with two copies and whichever `PATH` reaches first. Pass `--git` if a source build is what you actually want.

## Quick start

Open the picker:

```bash
ditto-cli
```

1. **Make a profile.** Press `n` and name it, such as `work`.
2. **Sign in.** With the profile selected, press `l` and choose Claude Code, Codex, fx, opencode, or Prime Agent; Prime Agent opens straight onto its `/login` dialog. Every other agent signs in from inside itself — launch it in step 3 and run `/login`, or its own login command, there.
3. **Launch a tool.** Press `Enter` for a list of every installed agent, filtered as you type, or a key directly: `c` Claude Code, `x` Codex, `f` fx, `o` opencode, `p` OMP, `a` Prime Agent, `i` Pi.

Each tool keeps its own credentials. Signing in to one does not copy credentials into another.

Or skip the picker entirely — every one of those steps has a command:

```bash
ditto-cli create work
ditto-cli claude work -- auth login
ditto-cli claude work
```

## TUI controls

| Key | Action |
| --- | --- |
| `↑` / `↓` or `k` / `j` | Select a profile |
| `c` | Launch Claude Code |
| `x` | Launch Codex |
| `f` | Launch fx |
| `o` | Launch opencode |
| `p` | Launch OMP |
| `a` | Launch Prime Agent |
| `i` | Launch Pi |
| `Enter` or `t` | Launch any installed agent: a list of them all, filtered as you type |
| `l` | Sign in with Claude Code, Codex, fx, opencode, or Prime Agent |
| `L` | Sign out, with confirmation |
| `n` | Create a profile |
| `e` | Rename the selected profile |
| `d` | Mark the selected profile `★` as the default, or unset it |
| `r` | Refresh sign-in status |
| `q`, `Esc`, or `Ctrl+C` | Quit or close a dialog |

Sign-in status is checked in the background, so the list stays responsive while each CLI is asked. A spinner marks the tools still being checked.

`Enter` puts every installed agent in one list, narrowed as you type:

```text
┌──────────────────────────────────────────────────────────────────────────────┐
│            Ditto CLI  choose a profile, then Enter to pick a tool            │
└──────────────────────────────────────────────────────────────────────────────┘
┌ Profiles ──────────────┐┌ Selected profile ──────────────────────────────────┐
│  default  existing     ││work  Isolated profile                              │
│  personal              ││                                                    │
│› work  ★               ││★ Used when no profile is named                     │
│                        ││                                                    │
│                        ││Sign-in status                                      │
│                        ││Claude Code  ● Signed in                            │
│                        ││Codex        ○ Sign in required                     │
│       ┌ Launch in 'work' ────────────────────────────────────────────┐       │
│       │› ▏                                                           │       │
│       │                                                              │       │
│       │Claude Code  ● Signed in                                      │       │
│       │Codex        ○ Sign in required                               │       │
│       │fx           ● Signed in                                      │       │
│       │opencode     ○ Sign in required                               │       │
│       │OMP          ● Signed in                                      │       │
│       │Prime Agent  ● Signed in                                      │       │
│       │Pi           ○ Sign in required                               │       │
│       │Gemini CLI   ● Signed in                                      │       │
│       │Grok         ○ Sign in required                               │fx     │
│       │                                                              │pencode│
│       │type to filter · ↑↓ · Enter launches · Esc                    │       │
│       └──────────────────────────────────────────────────────────────┘nt     │
│                        ││Pi           ~/.ditto/profiles/work/pi              │
│                        ││Gemini CLI   …itto/profiles/work/gemini-home/.gemini│
│                        ││Grok         ~/.ditto/profiles/work/grok            │
└────────────────────────┘└────────────────────────────────────────────────────┘
┌──────────────────────────────────────────────────────────────────────────────┐
│                  c Claude Code · x Codex · f fx · o opencode                 │
│                      p OMP · a Prime · i Pi · ⏎ any tool                     │
│                   ↑↓ select · n new · e rename · d default                   │
│                  l sign in · L sign out · r refresh · q quit                 │
└──────────────────────────────────────────────────────────────────────────────┘
```

The selected profile is remembered for the next run. The `★` default is separate and stays put: it is the profile every command uses when you leave the name out, and running `ditto-cli claude personal` once does not move it. Any profile can be the default, including the built-in `default` one. See [which profile a command uses](#which-profile-a-command-uses).

Renaming keeps the profile's settings, session history, and its logins. Claude Code is the exception, and the rename dialog says so before you commit to it: see [renaming a profile signs Claude Code out](#renaming-a-profile-signs-claude-code-out). The built-in `default` profile cannot be renamed.

## Command-line usage

The TUI is optional. Everything it does works directly from the shell.

**Manage profiles**

```bash
ditto-cli create work
ditto-cli rename work client-a
ditto-cli delete work --yes      # removes credentials and sessions for good
ditto-cli list
ditto-cli status client-a        # sign-in state for every tool
ditto-cli paths client-a
ditto-cli sync client-a          # preserve configuration for one profile
ditto-cli sync --all             # preserve it for every managed profile
ditto-cli sync --all --history   # also backfill existing conversations
```

**Launch a tool**

| Command | Alias | Pass arguments through with `--` |
| --- | --- | --- |
| `ditto-cli claude client-a` | `cc` | `ditto-cli cc client-a -- --model opus` |
| `ditto-cli codex client-a` | `cx` | `ditto-cli cx client-a -- --search` |
| `ditto-cli fx client-a` | — | `ditto-cli fx client-a -- login` |
| `ditto-cli opencode client-a` | `oc` | `ditto-cli oc client-a -- --model anthropic/claude-opus-5` |
| `ditto-cli omp client-a` | — | `ditto-cli omp client-a -- --model opus` |
| `ditto-cli prime-agent client-a` | `pa` | `ditto-cli pa client-a -- --model claude-opus-4-1` |
| `ditto-cli pi client-a` | — | `ditto-cli pi client-a -- --model anthropic/claude-opus-4-6` |
| `ditto-cli gemini client-a` | — | `ditto-cli gemini client-a -- --model gemini-2.5-pro`, and so on for every [supported agent](#supported-agents), by its command name |

Everything after `--` goes to the tool untouched, so any flag it accepts works. `ditto-cli --help` lists every agent; the [table](#supported-agents) says how each is isolated.

**Bind a directory, so launches from it need no profile name**

```bash
ditto-cli workspace              # what this directory launches with
ditto-cli workspace use client-a
ditto-cli workspace clear
ditto-cli workspace list
ditto-cli workspace auto off
```

**Choose the profile used when a command names none and no directory is bound**

```bash
ditto-cli default                # report it
ditto-cli default client-a       # pin it
ditto-cli default --clear        # release it
```

**Put Ditto in front of the tools' own names, and show the profile inside Claude Code**

```bash
eval "$(ditto-cli shell-init zsh)"        # bash, fish, zsh; reads SHELL when omitted

ditto-cli indicator client-a              # report the status line setting
ditto-cli indicator client-a --on
ditto-cli indicator client-a --off
ditto-cli indicator client-a --keep-mine  # in front of the status line you have
```

The native authentication commands can be called through a profile too:

```bash
ditto-cli claude client-a -- auth login
ditto-cli codex client-a -- login
ditto-cli opencode client-a -- auth login
ditto-cli prime-agent client-a -- /login
ditto-cli grok client-a -- login           # and so on, for any agent with a login command
```

OMP and Pi expose login only inside their interfaces. Launch the selected profile, then run `/login` or `/logout` there:

```bash
ditto-cli omp client-a
ditto-cli pi client-a
```

Deleting is the one command that destroys something, so it asks to be meant. Without `--yes` it lists the directories it would remove and stops. The built-in `default` profile cannot be deleted; it is your own configuration, which Ditto CLI never created.

Profile names use lowercase letters, numbers, `.`, `-` and `_`, start with a letter or number, and are at most 32 characters. Uppercase names are rejected because OMP accepts only lowercase profile names.

### Which profile a command uses

When the profile name is omitted, Ditto CLI takes the first answer it finds:

1. The directory's binding (see [Workspaces](#workspaces)).
2. The profile pinned as the default, with `d` in the TUI or `ditto-cli default <profile>`.
3. The last profile selected in the TUI.
4. Failing all of those, and before your first selection, `default`.

`ditto-cli list` marks the last selection with `*` and the default with a trailing `default`.

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

Where no directory answers, the launch says which profile it fell back to and why, since nothing in the command or the directory pointed at it:

```console
$ cd ~/scratch
$ ditto-cli omp
ditto-cli: using 'work'; nothing binds this directory, and it is pinned as the default
Bound this directory to 'work' (~/scratch/.ditto.toml).
```

Reporting commands stay quiet about it: for them a fallback is the ordinary case, and they are run in loops.

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

## Shell integration

A binding decides something only for a launch that goes through Ditto. Typing the tool's own name runs the tool, which has never heard of `.ditto.toml`:

```console
$ cd ~/code/client-a   # bound to client-a
$ omp                  # OMP's own default profile, not client-a
```

Nothing Ditto does at launch time can catch that, because Ditto is not in the process. The shell is the only place the bare name can be intercepted, so Ditto writes the functions that do it:

```bash
# ~/.zshrc
eval "$(ditto-cli shell-init zsh)"
```

`bash`, `fish`, and `zsh` are all written for, and `ditto-cli shell-init` with no shell named reads `SHELL`. Fish loads it by piping into `source`:

```fish
# ~/.config/fish/config.fish
ditto-cli shell-init fish | source
```

From then on the tool's own name uses the directory's profile, and arguments pass straight through to the tool:

```console
$ cd ~/code/client-a
$ omp --model opus     # the same as: ditto-cli omp -- --model opus
```

A function is written for every [supported agent](#supported-agents), so `gemini`, `copilot`, or `goose` typed in a bound directory go through Ditto too.

Two ways back out, both printed in the script itself: `command omp` runs OMP with Ditto out of the way, and `ditto-cli omp <profile>` launches another profile for that one run.

A function named after a tool cannot loop back into itself. Ditto starts a tool through `execvp`, which searches `PATH` and never looks at shell functions. And if `ditto-cli` cannot be found at all, each function falls through to the tool, so a half-finished update cannot take `claude` with it.

## Knowing which profile you are in

Once a tool has taken over the terminal, nothing on screen says which profile it is running as. Ditto CLI answers that in the window title for every tool, and inside Claude Code's own interface as well.

Claude Code gets a status line along the bottom of its interface:

```text
⬖ client-a · you@example.com
```

Launching Claude Code through Ditto CLI installs it, and `ditto-cli indicator` turns it on or off by hand. The rest of `settings.json` is written only when a profile is created or synced, described in [settings a new profile inherits](#settings-a-new-profile-inherits).

If the profile already has a `statusLine` of its own, Ditto CLI leaves it alone and says so. Claude Code renders one status line, and replacing the one you configured would quietly take it away. To have both, ask:

```console
$ ditto-cli indicator client-a --keep-mine
client-a: status line on, drawn in front of the one you already had
```

Ditto CLI then runs your status line itself and prints the profile in front of what it says:

```text
⬖ client-a · you@example.com │ ⎇ main ✓  ~/code/client-a  12.4k
```

Your command keeps the payload Claude Code sends it, every line it prints, and the settings it was written with. If it fails or prints nothing, you get the profile on its own rather than a broken line. `ditto-cli indicator --off` puts your original entry back exactly as you wrote it, and a later launch never changes the arrangement on its own — drawing over someone's status line is a thing to be asked for.

A status line set for a project (`.claude/settings.json`, `.claude/settings.local.json`) or by your administrator outranks the profile's, so Ditto CLI's would be installed and never seen. Rather than claim otherwise, `ditto-cli indicator` reports that:

```console
$ ditto-cli indicator client-a
client-a: installed, but a status line that outranks it is showing instead
```

The status line reads `DITTO_PROFILE`, so a `claude` you started yourself still reports the right profile as long as `CLAUDE_CONFIG_DIR` points at one.

Every tool also names the profile in the window and tab title:

```text
ditto:client-a — Codex — my-repo
```

The tools write their own titles and keep updating them as you work, so a title set once before handing over is overwritten within moments. Instead, Ditto CLI runs the tool in a pseudoterminal and rewrites the title sequences on their way to the terminal, adding `ditto:<profile>` in front of whatever the tool called itself. You keep the tool's own title and gain the profile.

Everything else is forwarded byte for byte. Colours, hyperlinks, clipboard writes, mouse reporting, and anything the tool draws are untouched, and the tool still gets a real terminal, the right window size, and your keystrokes as it always did. Ditto CLI exits with the tool's own exit status.

To turn it off, set `DITTO_NO_PROXY=1` and Ditto CLI hands the terminal straight to the tool as it used to. The title stops naming the profile, and the Claude Code status line still works. Ditto CLI also steps aside on its own when output is redirected, or if the pseudoterminal cannot be opened. The pseudoterminal is a macOS and Linux feature; see [Windows notes](#windows-notes).

### Under herdr

[herdr](https://herdr.dev) works out which agent is in a pane, and what that agent is doing, by reading the pane's foreground process and the title the tool writes. Sitting between the tool and the terminal costs it both answers: the foreground process becomes `ditto-cli` while the tool moves to a pseudoterminal of its own, and a title with `ditto:<profile>` in front of it no longer matches the rules herdr reads state from. The pane stops listing an agent, and `herdr agent` stops seeing it.

So Ditto CLI steps aside on its own inside a herdr pane, exactly as `DITTO_NO_PROXY=1` does, and reports the profile to herdr instead:

```text
herdr pane report-metadata <pane> --source ditto --token profile=<profile>
```

herdr shows it as a pane token, and detection, agent state, and the Claude Code status line all keep working. Nothing needs configuring — it keys off `HERDR_PANE_ID`, which herdr sets in every pane it opens. If herdr is not running or its CLI is not on `PATH` the report is skipped and the launch carries on.

### Under Orca

[Orca](https://github.com/stablyai/orca) runs agents side by side in worktrees of their own, and reads the same two things herdr does — the pane's foreground process and the title the agent writes — to know which agent a terminal is running and whether it is working or waiting. It also holds a prompt back until the foreground process is the agent it launched. A pseudoterminal in between costs it all three, so Ditto CLI steps aside inside an Orca terminal as well. It keys off `ORCA_PANE_KEY`, which Orca sets in every terminal it opens. Orca has no command that labels a pane already open, so there is nothing to report the profile to: Claude Code's status line still names it, and the other tools run without it in the title.

Orca starts an agent by typing its command into your shell, which leaves two ways to put a profile in front of it:

- With [shell integration](#shell-integration) loaded, nothing else is needed. The `claude` Orca types is already the function that routes through Ditto CLI.
- Otherwise open Orca's **Settings → Agents** and set the agent's **Command** to `ditto-cli claude --`, or `ditto-cli claude client-a --` to pin one profile. Orca appends its own arguments and the prompt after it, and everything after `--` reaches the tool. The same works for every [supported agent](#supported-agents).

Which profile a worktree launches with follows the usual [resolution](#how-a-directory-is-resolved). A `.ditto.toml` committed at the repository root is checked out into every worktree Orca creates; `ditto-cli workspace` binds a directory that cannot carry one.

Two things to know:

- Orca's own account switchers and Ditto profiles compose. Orca picks a Claude account by swapping the credentials in `~/.claude` and a Codex account by pointing `CODEX_HOME` at a directory of its own; both are what the `default` profile resolves to, so Orca decides who `default` is, and Ditto profiles are everyone else.
- Orca's agent status hooks are written to `~/.claude/settings.json`. A profile created before they were turned on has none; `ditto-cli sync <profile>` copies them in, unless the profile already had `hooks` of its own, which `sync` reports under `kept`.

## Scripting and agents

Add `--json` to any reporting command and it prints one JSON object on stdout. The flag is global, so it reads correctly on either side of the subcommand:

```bash
ditto-cli list --json
ditto-cli --json status client-a
```

```json
{
  "profile": "client-a",
  "managed": true,
  "tools": [
    { "tool": "claude", "label": "Claude Code", "status": "signed_in", "signed_in": true },
    { "tool": "codex", "label": "Codex", "status": "signed_out", "signed_in": false },
    { "tool": "opencode", "label": "opencode", "status": "signed_in", "signed_in": true },
    { "tool": "omp", "label": "OMP", "status": "signed_out", "signed_in": false },
    { "tool": "prime-agent", "label": "Prime Agent", "status": "signed_in", "signed_in": true },
    { "tool": "pi", "label": "Pi", "status": "signed_out", "signed_in": false }
  ]
}
```

The remaining agents follow in the order `--help` lists them, each `unavailable` unless it is installed, so the shape is the same on every machine. The human report shows only the ones you have.

`list`, `status`, `paths`, `create`, `rename`, `delete`, `sync`, `default`, `workspace`, and `indicator` all answer in JSON. Errors become `{"error": "..."}` on stderr, and every failure exits 1. `shell-init` prints a script for a shell to read rather than a report, so `--json` has nothing to do there.

Running `ditto-cli` with no subcommand opens the picker, which needs an interactive terminal. Without one it exits 1 and names the commands to use instead, rather than failing on a terminal that was never there.

```bash
# Any tool needing a sign-in?
ditto-cli status client-a --json | jq -r '.tools[] | select(.signed_in | not) | .tool'
```

The `tool`, `status`, and `indicator` outcome strings are stable identifiers meant to be matched on; the `label` beside them is display text and may be reworded. [AGENTS.md](AGENTS.md) documents the full contract and has more recipes, along with how to edit Ditto CLI itself.

## Settings a new profile inherits

Claude Code reads its settings from the configuration directory it is pointed at, and Ditto CLI points it somewhere else. Left alone, a new profile would start with none of the permission mode, model, effort level, or hooks you set up once and expect everywhere.

So creating a profile copies `~/.claude/settings.json` into it. What Ditto CLI isolates is accounts, and none of those live in that file — Claude Code keeps credentials in the Keychain and the signed-in account in `.claude.json` — so your preferences travel and your logins stay apart. The status line travels too: the entry names the profile it was installed for, so what is copied is the status line underneath it, with the new profile's own indicator drawn in front of yours.

From then on the profile's settings are its own. Change the model in one profile and the others keep theirs; a later `ditto-cli sync` fills in settings the profile has never answered and leaves the ones it has:

```bash
ditto-cli sync client-a              # bring one profile up to date
ditto-cli sync --all                 # bring every managed profile up to date
ditto-cli sync client-a --overwrite  # your configuration wins outright
```

Naming a profile preserves configuration for that profile only; `--all` makes the scope every managed profile. `sync` is also how profiles created before this behaviour existed catch up, including tool directories added by a newer Ditto release.

## Skills, subagents, and everything else you set up once

A profile exists to be signed in as somebody else, not to be a different working environment. Skills, subagents, slash commands, hooks, plugins, and the memory file each tool reads are not accounts, so a profile does not get its own copy of them — it reads yours:

| Tool | Read from your own configuration |
| --- | --- |
| Claude Code | `skills`, `agents`, `commands`, `hooks`, `plugins`, `output-styles`, `CLAUDE.md` |
| Codex | `skills`, `rules`, `prompts`, `plugins`, `config.toml`, `hooks.json`, `AGENTS.md`, `instructions.md` |
| opencode | the whole configuration directory |
| OMP | `config.yml`, `extensions` |
| Prime Agent | `settings.json`, `keybindings.json`, instructions, prompts, skills, extensions, themes, packages, and the global harness |
| Pi | `settings.json`, `keybindings.json`, project trust, instructions, prompts, skills, extensions, themes, packages, and managed tools |
| Every other agent | its settings, instructions, skills, commands, and plugins — the entry in [`src/tools.rs`](src/tools.rs) names them |

These are symbolic links, so a skill you write tomorrow is in every profile the moment you save it, with nothing to sync and no copies to drift apart. Everything else — `.claude.json`, `auth.json`, sessions, session artifacts, history, `agent.db` — stays inside the profile, which is the whole of what a profile keeps to itself. Prime Agent and Pi keep `models.json` private too, because custom provider definitions may contain literal API keys and secret headers.

Conversation history is not linked: chats can contain account- or client-specific work, and shared writable session stores would make every profile's future activity visible to every other profile. Existing conversations, from any agent, can instead be copied once, without replacing anything already in the destination. Choose one profile explicitly or all managed profiles explicitly:

```bash
ditto-cli sync client-a --history
ditto-cli sync --all --history
```

After the copy, every tool keeps writing to that profile's own history store.

What is shared is a named list rather than everything-but-the-credentials. Ditto CLI learning about a new extension directory late costs you a missing feature; sharing a new credential file by accident would cost you the isolation the tool exists for.

Profiles created before this have real directories where the links go. `sync` reports those and leaves them alone; `--adopt` points them at yours, moving what was there aside as `<name>.before-ditto` rather than deleting it:

```bash
ditto-cli sync client-a           # link what can be linked, report what cannot
ditto-cli sync --all              # do that for every managed profile
ditto-cli sync client-a --adopt   # replace the profile's own copies too
```

### Downloaded skills

Sharing a directory by linking it has one cost, and Ditto CLI pays it for you.

A skill installer — `npx skills add`, and anything else that installs into more than one agent — puts the skill somewhere central and then records it in each agent's skills directory as a *relative* link, computed from the directory the agent was pointed at. Under a profile that directory is one of the links above, so the operating system writes the record into your own configuration instead, where the same relative path leads somewhere else. The skill lands on disk intact and is readable from nowhere, in every profile at once.

Ditto CLI is what moved the directory, so Ditto CLI is what can say where those links meant to point. Launching a tool repairs its own, which is the moment before it reads them, and says so:

```
ditto-cli: repaired claude/skills/apple-design; it was installed pointing at nothing
```

`ditto-cli sync <profile>` does the same for every tool at once and reports them under `repaired`. A link is only rewritten when reading it against the path the installer was given names something that exists, so a link that is relative and broken for reasons of its own is left exactly as it is.

## Where credentials and files are stored

Ditto CLI does not ask for passwords, parse OAuth tokens, or keep credentials in its state file. Signing in still happens through the agent itself — its own login command, or `/login` inside it — and each agent stores the result wherever it normally would, under the directory Ditto CLI pointed it at:

- **Claude Code** uses the selected `CLAUDE_CONFIG_DIR`. On macOS the credentials themselves stay in the system Keychain, keyed to that directory's path, which is what keeps two profiles from sharing one login — and why [renaming a profile signs Claude Code out](#renaming-a-profile-signs-claude-code-out).
- **Codex** keeps its auth state under the selected `CODEX_HOME`.
- **fx** keeps its login and sessions under `.fx` in the profile's private home; the macOS Keychain, which it would otherwise use, is refused because it is user-wide.
- **opencode** writes `auth.json` into the selected data directory.
- **OMP** keeps auth, settings, sessions, and caches under `~/.omp/profiles/<name>/agent`.
- **Prime Agent** keeps ordinary provider credentials in the selected agent directory's `auth.json`; its sessions and session artifacts stay below the same profile.
- **Pi** keeps provider credentials in the selected agent directory's `auth.json`; `PI_CODING_AGENT_SESSION_DIR` keeps its transcripts below the same profile.
- **Every other agent** stores its login under the directory or home it was handed; the [Supported agents](#supported-agents) table says which file, and which agents keep it in a keychain instead.

<details>
<summary><strong>Prime Agent has two upstream exceptions to that boundary</strong></summary>

<br>

Prime Inference login uses the Prime CLI credential at `~/.prime/config.json`, for which Prime Agent exposes no path override; that one provider therefore remains shared across Ditto profiles. `PRIME_API_KEY` and other API-key environment variables are shared for the same reason any inherited environment credential is.

Prime Agent also uses one user-wide background-service socket, so its `agents`, `list`, and `attach` views can see running agents from other profiles even though each agent receives its selected auth and session roots.

Ditto's status report counts provider credentials in the isolated `auth.json`, not these ambient sources.

</details>

Ditto CLI's own files are laid out like this:

```text
~/.ditto/
├── state.toml
├── workspaces.json      # directories bound without a file of their own
└── profiles/
    ├── work/
    │   ├── claude/
    │   ├── codex/
    │   ├── opencode/
    │   │   ├── config/opencode/
    │   │   ├── data/opencode/      # auth.json lives here
    │   │   └── state/opencode/
    │   ├── grok/                  # one directory per other agent, named after it
    │   ├── droid-home/            # or a private home, for the agents that need one
    │   ├── prime-agent/            # auth.json and sessions live here
    │   └── pi/                     # auth.json and sessions live here
    └── personal/
        ├── claude/
        ├── codex/
        ├── opencode/
        ├── prime-agent/
        └── pi/
```

The nested `opencode/` directory is opencode's own doing: it appends its name to every XDG base it is given. OMP profiles are the one thing kept outside this tree, since OMP manages its own profile directory.

Directories are created with user-only permissions on macOS and Linux; see [Windows notes](#windows-notes) for the difference there.

The `default` profile points to `~/.claude`, `~/.codex` (or `CODEX_HOME` when set), opencode's own `~/.local/share/opencode` and `~/.config/opencode` (or wherever your `XDG_*` variables already send them), OMP's native `~/.omp/agent` profile, Prime Agent's `~/.prime/agent` directory (or `PRIME_AGENT_CODING_AGENT_DIR` when set), and Pi's `~/.pi/agent` directory (or `PI_CODING_AGENT_DIR` when set); every other agent's `default` is likewise wherever it keeps its files when nothing points it elsewhere, its own variable included. It exposes your existing setup without copying or migrating anything. For opencode the `default` profile resolves the same XDG bases opencode would pick on its own, so pointing at it changes nothing.

## Environment variables

| Variable | Purpose |
| --- | --- |
| `DITTO_HOME` | Move Ditto CLI's state and profile directory from `~/.ditto` |
| `DITTO_CLAUDE_BIN` | Override the `claude` executable |
| `DITTO_CODEX_BIN` | Override the `codex` executable |
| `DITTO_FX_BIN` | Override the `fx` executable |
| `DITTO_OPENCODE_BIN` | Override the `opencode` executable |
| `DITTO_OMP_BIN` | Override the `omp` executable |
| `DITTO_PRIME_AGENT_BIN` | Override the `prime-agent` executable |
| `DITTO_PI_BIN` | Override the `pi` executable |
| `DITTO_<AGENT>_BIN` | Override any other agent's executable, named from its command: `DITTO_CURSOR_AGENT_BIN` |
| `DITTO_PROFILE` | Selected profile name exported to every launched tool, and what Claude Code's status line reports |
| `DITTO_NO_PROXY` | Hand the terminal straight to the tool, leaving the title to it (macOS and Linux) |
| `HERDR_PANE_ID` | Read, not set: herdr names the pane it opened, and Ditto CLI steps aside and reports the profile to herdr. See [Under herdr](#under-herdr) |
| `ORCA_PANE_KEY` | Read, not set: Orca names the terminal it opened, and Ditto CLI steps aside. See [Under Orca](#under-orca) |
| `NO_COLOR` | Draw the Claude Code status line without colour |

Example:

```bash
DITTO_HOME="$HOME/.config/ditto" ditto-cli
```

API-key variables such as `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `OPENCODE_API_KEY`, and `PRIME_API_KEY` are inherited by launched tools. They may override a saved subscription login, so Ditto CLI warns when it sees an `*_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, or `HF_TOKEN` in the environment.

## Renaming a profile signs Claude Code out

Claude Code stores credentials against the configuration directory it was pointed at, so moving that directory loses the sign-in. Renaming a profile moves `~/.ditto/profiles/<name>/claude`, and the credentials do not follow it.

Ditto CLI warns before the rename and tells you how to get back in:

```bash
ditto-cli claude client-a -- auth login
```

Every other agent keeps its credentials in files inside the profile, so they survive a rename untouched — apart from the few the [Supported agents](#supported-agents) table marks as keeping theirs in an OS keychain, which was never per profile to begin with.

## Windows notes

Profiles, launching, sign-in status, the Claude Code status line, and every command above work the same as they do elsewhere. Three things are worth knowing:

- **Window titles.** Naming the profile in the title means sitting between the tool and the terminal, which needs a pseudoterminal; Ditto CLI opens one on macOS and Linux only. On Windows the title is set once before the tool starts, and the tool then paints over it with its own. Claude Code's status line is unaffected, and is the better indicator anyway. Ditto CLI still waits on the tool and exits with its status, and Ctrl-C goes to the tool rather than ending Ditto CLI out from under it.
- **Directory permissions.** Profile directories are created owner-only on macOS and Linux. Windows has no equivalent mode to set, so profiles inherit the access control of the directory they are created in. That is enough under `%USERPROFILE%`; if you point `DITTO_HOME` somewhere else, put it somewhere private.
- **Finding the CLIs.** On Windows they are usually installed by npm, which writes `claude.cmd` rather than `claude.exe`. Windows itself only ever looks for `.exe`, so Ditto CLI searches `PATH` the way a command prompt does, honouring `PATHEXT`. A native `claude.exe` is preferred when both are present.

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

[AGENTS.md](AGENTS.md) has the module layout, the conventions this codebase holds to, and step-by-step recipes for adding a subcommand or a tool. `CLAUDE.md` points at the same file.

## License

Ditto CLI is available under the [MIT License](LICENSE).

Ditto CLI is an independent project. It is not affiliated with Anthropic, OpenAI, Prime Intellect, Nintendo, or The Pokémon Company.
