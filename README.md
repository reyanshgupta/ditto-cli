# Ditto CLI

[![crates.io](https://img.shields.io/crates/v/ditto-cli.svg)](https://crates.io/crates/ditto-cli)
[![npm](https://img.shields.io/npm/v/@reyanshgupta/ditto-cli.svg)](https://www.npmjs.com/package/@reyanshgupta/ditto-cli)
[![MIT license](https://img.shields.io/badge/license-MIT-6f42c1.svg)](LICENSE)
[![Rust 1.85+](https://img.shields.io/badge/rust-1.85%2B-6f42c1.svg)](https://www.rust-lang.org/)

Keep work, personal, and client Claude Code, Codex, opencode, OMP, Prime Agent, and Pi logins apart.

Ditto CLI gives each profile its own authentication and session history while keeping the capabilities you configured once. Pick a profile in the terminal, then launch any of the six tools. Your existing setup stays available as the `default` profile.

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
│                        ││OMP          ○ Sign in required                     │
│                        ││Prime Agent  ● Signed in                            │
│                        ││Pi           ○ Sign in required                     │
│                        ││                                                    │
│                        ││Profile directories                                 │
│                        ││Claude Code  ~/.ditto/profiles/work/claude          │
│                        ││Codex        ~/.ditto/profiles/work/codex           │
│                        ││opencode     …/work/opencode/data/opencode          │
│                        ││OMP          ~/.omp/profiles/work/agent             │
│                        ││Prime Agent  …/.ditto/profiles/work/prime-agent     │
│                        ││Pi           ~/.ditto/profiles/work/pi              │
└────────────────────────┘└────────────────────────────────────────────────────┘
┌──────────────────────────────────────────────────────────────────────────────┐
│     c Claude Code · x Codex · o opencode · p OMP · a Prime · i Pi           │
│              ↑↓ select · n new · e rename · d default                        │
│              l sign in · L sign out · r refresh · q quit                     │
└──────────────────────────────────────────────────────────────────────────────┘
```

## How it works

Claude Code, Codex, opencode, OMP, Prime Agent, and Pi keep user-level configuration and login state on disk. That works until you need separate accounts for different jobs. Manually moving auth files around is easy to get wrong, and it is hard to tell which account a new session will use.

Instead, Ditto CLI launches each tool pointed at the selected profile:

| Tool | Setting used by Ditto CLI |
| --- | --- |
| Claude Code | `CLAUDE_CONFIG_DIR=~/.ditto/profiles/<name>/claude` |
| Codex | `CODEX_HOME=~/.ditto/profiles/<name>/codex` |
| opencode | `XDG_DATA_HOME`, `XDG_CONFIG_HOME`, and `XDG_STATE_HOME` under `~/.ditto/profiles/<name>/opencode` |
| OMP | `omp --profile <name>` and `OMP_PROFILE=<name>` |
| Prime Agent | `PRIME_AGENT_CODING_AGENT_DIR=~/.ditto/profiles/<name>/prime-agent` and a matching `PRIME_AGENT_SESSION_DIR` |
| Pi | `PI_CODING_AGENT_DIR=~/.ditto/profiles/<name>/pi` and a matching `PI_CODING_AGENT_SESSION_DIR` |

opencode has no single home variable; it resolves its directories the XDG way, so Ditto CLI pins the three bases that hold credentials, configuration, and session state. `XDG_CACHE_HOME` is deliberately left alone so profiles keep sharing opencode's downloaded tooling. Those variables are set for the launched process and anything it starts, including commands opencode's own tools run.

Prime Agent and Pi settings can name a session directory, so managed profiles set `PRIME_AGENT_SESSION_DIR` or `PI_CODING_AGENT_SESSION_DIR` explicitly. That keeps transcripts and local harness state inside the selected profile even while reusable settings and capabilities are linked to yours.

No config files are swapped. Profiles remain independent, and switching only affects the process Ditto CLI launched.

## Install

Ditto CLI needs at least one of the supported CLIs to be useful: [Claude Code](https://code.claude.com/docs/en/setup), [OpenAI Codex CLI](https://github.com/openai/codex), [opencode](https://opencode.ai/docs/), [Oh My Pi](https://github.com/can1357/oh-my-pi), [Prime Agent](https://github.com/PrimeIntellect-ai/prime-agent), or [Pi](https://pi.dev).

On macOS and Linux, Homebrew installs the prebuilt binary for your platform, so it needs no Rust toolchain:

```bash
brew install reyanshgupta/tap/ditto-cli
```

npm carries the same prebuilt binaries, on macOS, Linux, and Windows alike. If you installed Claude Code or Codex with npm, it is already here:

```bash
npm install -g @reyanshgupta/ditto-cli
npx @reyanshgupta/ditto-cli list        # or try it without installing
```

The npm package is scoped because plain `ditto-cli` on npm belongs to an unrelated project. The command it installs is still `ditto-cli`.

Cargo builds from source instead, and needs Rust 1.85 or newer:

```bash
cargo install ditto-cli                                          # from crates.io
cargo install --git https://github.com/reyanshgupta/ditto-cli    # from the latest source
```

`cargo binstall ditto-cli` takes the released binary instead of compiling one, which is a good deal quicker: a source build compiles a bundled SQLite before it links anything.

Or from a local checkout:

```bash
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

macOS, Linux, and Windows are all supported, and every command below behaves the same on each. Two smaller things differ on Windows — see [Windows notes](#windows-notes).

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

Open Ditto CLI:

```bash
ditto-cli
```

Then:

1. Press `n` and name the profile, such as `work`.
2. Select the profile and press `l`, then choose Claude Code, Codex, opencode, or Prime Agent. Prime Agent opens directly on its `/login` dialog.
3. Press `c` for Claude Code, `x` for Codex, `o` for opencode, or `a` for Prime Agent.
4. Press `p` for OMP or `i` for Pi, then use `/login` inside it.

Each tool keeps its own credentials. Signing in to one does not copy credentials into another.

## TUI controls

| Key | Action |
| --- | --- |
| `↑` / `↓` or `k` / `j` | Select a profile |
| `c` | Launch Claude Code |
| `x` | Launch Codex |
| `o` | Launch opencode |
| `p` | Launch OMP |
| `a` | Launch Prime Agent |
| `i` | Launch Pi |
| `l` | Sign in with Claude Code, Codex, opencode, or Prime Agent |
| `L` | Sign out, with confirmation |
| `n` | Create a profile |
| `e` | Rename the selected profile |
| `d` | Mark the selected profile `★` as the default, or unset it |
| `r` | Refresh sign-in status |
| `q`, `Esc`, or `Ctrl+C` | Quit or close a dialog |

Sign-in status is checked in the background, so the list stays responsive while each CLI is asked. A spinner marks the tools still being checked.

The selected profile is remembered for the next run. The `★` default is separate and stays put: it is the profile every command uses when you leave the name out, and running `ditto-cli claude personal` once does not move it. Any profile can be the default, including the built-in `default` one. See [which profile a command uses](#which-profile-a-command-uses).

Renaming keeps the profile's settings, session history, and its Codex, opencode, Prime Agent, and Pi logins. Claude Code is the exception, and the rename dialog says so before you commit to it: see [renaming a profile signs Claude Code out](#renaming-a-profile-signs-claude-code-out). The built-in `default` profile cannot be renamed.

## Command-line usage

The TUI is optional. Everything it does works directly from the shell:

```bash
# Profiles
ditto-cli create work
ditto-cli rename work client-a
ditto-cli delete work --yes   # removes credentials and sessions for good
ditto-cli list
ditto-cli status client-a     # sign-in state for all six tools
ditto-cli paths client-a
ditto-cli sync client-a       # re-copy your Claude Code settings into it

# Launch a tool, with short aliases where shown
ditto-cli claude client-a     # or: cc
ditto-cli codex client-a      # or: cx
ditto-cli opencode client-a   # or: oc
ditto-cli omp client-a
ditto-cli prime-agent client-a # or: pa
ditto-cli pi client-a

# Pass arguments to the underlying CLI after --
ditto-cli claude client-a -- --model opus
ditto-cli codex client-a -- --search
ditto-cli opencode client-a -- --model anthropic/claude-opus-5
ditto-cli omp client-a -- --model opus
ditto-cli prime-agent client-a -- --model claude-opus-4-1
ditto-cli pi client-a -- --model anthropic/claude-opus-4-6

# Bind a directory to a profile
ditto-cli workspace                  # what this directory launches with
ditto-cli workspace use client-a
ditto-cli workspace clear
ditto-cli workspace list
ditto-cli workspace auto off

# Put Ditto in front of the tools' own names
eval "$(ditto-cli shell-init zsh)"   # bash, fish, zsh; reads SHELL when omitted

# The profile used when a command names none and no directory is bound
ditto-cli default             # report it
ditto-cli default client-a    # pin it
ditto-cli default --clear     # release it

# Show the profile inside Claude Code
ditto-cli indicator client-a             # report the status line setting
ditto-cli indicator client-a --on
ditto-cli indicator client-a --off
ditto-cli indicator client-a --keep-mine # in front of the status line you have
```

The native authentication commands can be called through a profile too:

```bash
ditto-cli claude client-a -- auth login
ditto-cli codex client-a -- login
ditto-cli opencode client-a -- auth login
ditto-cli prime-agent client-a -- /login
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

All six tools write their own titles and keep updating them as you work, so a title set once before handing over is overwritten within moments. Instead, Ditto CLI runs the tool in a pseudoterminal and rewrites the title sequences on their way to the terminal, adding `ditto:<profile>` in front of whatever the tool called itself. You keep the tool's own title and gain the profile.

Everything else is forwarded byte for byte. Colours, hyperlinks, clipboard writes, mouse reporting, and anything the tool draws are untouched, and the tool still gets a real terminal, the right window size, and your keystrokes as it always did. Ditto CLI exits with the tool's own exit status.

To turn it off, set `DITTO_NO_PROXY=1` and Ditto CLI hands the terminal straight to the tool as it used to. The title stops naming the profile, and the Claude Code status line still works. Ditto CLI also steps aside on its own when output is redirected, or if the pseudoterminal cannot be opened. The pseudoterminal is a macOS and Linux feature; see [Windows notes](#windows-notes).

### Under herdr

[herdr](https://herdr.dev) works out which agent is in a pane, and what that agent is doing, by reading the pane's foreground process and the title the tool writes. Sitting between the tool and the terminal costs it both answers: the foreground process becomes `ditto-cli` while the tool moves to a pseudoterminal of its own, and a title with `ditto:<profile>` in front of it no longer matches the rules herdr reads state from. The pane stops listing an agent, and `herdr agent` stops seeing it.

So Ditto CLI steps aside on its own inside a herdr pane, exactly as `DITTO_NO_PROXY=1` does, and reports the profile to herdr instead:

```text
herdr pane report-metadata <pane> --source ditto --token profile=<profile>
```

herdr shows it as a pane token, and detection, agent state, and the Claude Code status line all keep working. Nothing needs configuring — it keys off `HERDR_PANE_ID`, which herdr sets in every pane it opens. If herdr is not running or its CLI is not on `PATH` the report is skipped and the launch carries on.

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

`list`, `status`, `paths`, `create`, `rename`, `delete`, `default`, `workspace`, and `indicator` all answer in JSON. Errors become `{"error": "..."}` on stderr, and every failure exits 1. `shell-init` prints a script for a shell to read rather than a report, so `--json` has nothing to do there.

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
ditto-cli sync client-a              # bring it up to date, keeping its own answers
ditto-cli sync client-a --overwrite  # your configuration wins outright
```

`sync` is also how profiles created before this behaviour existed catch up.

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

These are symbolic links, so a skill you write tomorrow is in every profile the moment you save it, with nothing to sync and no copies to drift apart. Everything else — `.claude.json`, `auth.json`, sessions, session artifacts, history, `agent.db` — stays inside the profile, which is the whole of what a profile keeps to itself. Prime Agent and Pi keep `models.json` private too, because custom provider definitions may contain literal API keys and secret headers.

What is shared is a named list rather than everything-but-the-credentials. Ditto CLI learning about a new extension directory late costs you a missing feature; sharing a new credential file by accident would cost you the isolation the tool exists for.

Profiles created before this have real directories where the links go. `sync` reports those and leaves them alone; `--adopt` points them at yours, moving what was there aside as `<name>.before-ditto` rather than deleting it:

```bash
ditto-cli sync client-a           # link what can be linked, report what cannot
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

Ditto CLI does not ask for passwords, parse OAuth tokens, or keep credentials in its state file. Claude Code, Codex, and opencode authentication still runs through their installed CLIs, and OMP, Prime Agent, and Pi authentication through `/login` inside their interfaces. Each tool stores the result wherever it normally would, under the directory Ditto CLI pointed it at:

- **Claude Code** uses the selected `CLAUDE_CONFIG_DIR`. On macOS the credentials themselves stay in the system Keychain, keyed to that directory's path, which is what keeps two profiles from sharing one login — and why [renaming a profile signs Claude Code out](#renaming-a-profile-signs-claude-code-out).
- **Codex** keeps its auth state under the selected `CODEX_HOME`.
- **opencode** writes `auth.json` into the selected data directory.
- **OMP** keeps auth, settings, sessions, and caches under `~/.omp/profiles/<name>/agent`.
- **Prime Agent** keeps ordinary provider credentials in the selected agent directory's `auth.json`; its sessions and session artifacts stay below the same profile.
- **Pi** keeps provider credentials in the selected agent directory's `auth.json`; `PI_CODING_AGENT_SESSION_DIR` keeps its transcripts below the same profile.

Prime Agent currently has two upstream exceptions to that boundary. Prime Inference login uses the Prime CLI credential at `~/.prime/config.json`, for which Prime Agent exposes no path override; that one provider therefore remains shared across Ditto profiles. `PRIME_API_KEY` and other API-key environment variables are shared for the same reason any inherited environment credential is. Prime Agent also uses one user-wide background-service socket, so its `agents`, `list`, and `attach` views can see running agents from other profiles even though each agent receives its selected auth and session roots. Ditto's status report counts provider credentials in the isolated `auth.json`, not these ambient sources.

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

The `default` profile points to `~/.claude`, `~/.codex`, opencode's own `~/.local/share/opencode` and `~/.config/opencode` (or wherever your `XDG_*` variables already send them), OMP's native `~/.omp/agent` profile, Prime Agent's `~/.prime/agent` directory (or `PRIME_AGENT_CODING_AGENT_DIR` when set), and Pi's `~/.pi/agent` directory (or `PI_CODING_AGENT_DIR` when set). It exposes your existing setup without copying or migrating anything. For opencode the `default` profile resolves the same XDG bases opencode would pick on its own, so pointing at it changes nothing.

## Environment variables

| Variable | Purpose |
| --- | --- |
| `DITTO_HOME` | Move Ditto CLI's state and profile directory from `~/.ditto` |
| `DITTO_CLAUDE_BIN` | Override the `claude` executable |
| `DITTO_CODEX_BIN` | Override the `codex` executable |
| `DITTO_OPENCODE_BIN` | Override the `opencode` executable |
| `DITTO_OMP_BIN` | Override the `omp` executable |
| `DITTO_PRIME_AGENT_BIN` | Override the `prime-agent` executable |
| `DITTO_PI_BIN` | Override the `pi` executable |
| `DITTO_PROFILE` | Selected profile name exported to every launched tool, and what Claude Code's status line reports |
| `DITTO_NO_PROXY` | Hand the terminal straight to the tool, leaving the title to it (macOS and Linux) |
| `HERDR_PANE_ID` | Read, not set: herdr names the pane it opened, and Ditto CLI steps aside and reports the profile to herdr. See [Under herdr](#under-herdr) |
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

Codex, opencode, Prime Agent, and Pi keep their ordinary credentials in files inside the profile, so they survive a rename untouched.

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
