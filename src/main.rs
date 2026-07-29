mod cli;
mod indicator;
mod launch;
mod profile;
mod program;
#[cfg(unix)]
mod proxy;
mod ui;
mod update;

use std::io::{self, IsTerminal};

use anyhow::{Result, bail};
use clap::Parser;
use serde_json::{Value, json};

use cli::{
    Cli, Command, DefaultAction, DefaultArgs, DeleteArgs, IndicatorAction, IndicatorArgs,
    LaunchArgs,
};
use launch::{AuthStatus, Tool};
use profile::{DEFAULT_PROFILE, Profile, Store};

fn main() {
    let cli = Cli::parse();
    // Read before the command is consumed, so a failure can answer in the shape
    // the caller asked for rather than dropping to prose halfway through.
    let json = cli.json;

    if let Err(error) = run(cli) {
        if json {
            eprintln!("{}", json!({ "error": format!("{error:#}") }));
        } else {
            eprintln!("ditto-cli: {error:#}");
        }
        std::process::exit(1);
    }
}

fn run(cli: Cli) -> Result<()> {
    let store = Store::discover()?;
    let json = cli.json;

    match cli.command {
        None => run_tui(&store),
        Some(Command::List) => list_profiles(&store, json),
        Some(Command::Status { profile }) => show_status(&store, profile.as_deref(), json),
        Some(Command::Create { name }) => create_profile(&store, &name, json),
        Some(Command::Rename { profile, new_name }) => {
            rename_profile(&store, &profile, &new_name, json)
        }
        Some(Command::Delete(arguments)) => delete_profile(&store, arguments, json),
        Some(Command::Default(arguments)) => set_default_profile(&store, arguments, json),
        Some(Command::Paths { profile }) => show_paths(&store, profile.as_deref(), json),
        Some(Command::Claude(arguments)) => launch_direct(&store, Tool::Claude, arguments),
        Some(Command::Codex(arguments)) => launch_direct(&store, Tool::Codex, arguments),
        Some(Command::Opencode(arguments)) => launch_direct(&store, Tool::Opencode, arguments),
        Some(Command::Omp(arguments)) => launch_direct(&store, Tool::Omp, arguments),
        Some(Command::Indicator(arguments)) => set_indicator(&store, arguments, json),
        Some(Command::Statusline) => indicator::render(),
        Some(Command::Update(arguments)) => update::run(arguments.check, arguments.git),
    }
}

/// Answers in whichever shape the caller asked for. Every reporting command
/// speaks both, so the choice is made here rather than in each of them.
fn report(json: bool, payload: impl FnOnce() -> Value, human: impl FnOnce()) {
    if json {
        println!("{}", payload());
    } else {
        human();
    }
}

/// The picker needs a terminal to draw on and a keyboard to read, and gets
/// neither from a script or an agent. Saying so beats the panic that reaching
/// for an absent terminal would otherwise raise, and naming the commands that
/// do work turns a dead end into a next step.
fn run_tui(store: &Store) -> Result<()> {
    if !io::stdout().is_terminal() || !io::stdin().is_terminal() {
        bail!(
            "the profile picker needs an interactive terminal, and this is not one.\n\
             Every command works without it:\n\
            \x20 ditto-cli list --json\n\
            \x20 ditto-cli status <profile> --json\n\
            \x20 ditto-cli create <profile>\n\
            \x20 ditto-cli default <profile>\n\
             Run `ditto-cli --help` for the full set."
        );
    }

    loop {
        let profiles = store.list_profiles()?;
        let last_profile = store.last_profile()?;
        let default_profile = store.default_profile_name()?;
        let Some(action) = ui::run(store, profiles, last_profile.as_deref(), default_profile)?
        else {
            return Ok(());
        };

        match action {
            ui::UiAction::Launch { tool, profile } => {
                store.save_last_profile(&profile.name)?;
                return launch::launch(tool, &profile, &[]);
            }
            ui::UiAction::Authenticate {
                operation,
                tool,
                profile,
            } => {
                store.save_last_profile(&profile.name)?;
                launch::authenticate(operation, tool, &profile)?;
            }
        }
    }
}

fn list_profiles(store: &Store, json: bool) -> Result<()> {
    let last_profile = store.last_profile()?;
    let default_profile = store.default_profile_name()?;
    let profiles = store.list_profiles()?;

    let is_default = |profile: &Profile| default_profile.as_deref() == Some(&profile.name);
    let is_last = |profile: &Profile| last_profile.as_deref() == Some(&profile.name);

    report(
        json,
        || {
            json!({
                "profiles": profiles
                    .iter()
                    .map(|profile| json!({
                        "name": profile.name,
                        "managed": profile.managed,
                        "is_default": is_default(profile),
                        "is_last_selected": is_last(profile),
                    }))
                    .collect::<Vec<_>>(),
                "default_profile": default_profile,
                "last_profile": last_profile,
                // What a command with no profile name would actually use, so a
                // caller does not have to re-derive the precedence rule.
                "fallback_profile": default_profile
                    .clone()
                    .or_else(|| last_profile.clone())
                    .unwrap_or_else(|| DEFAULT_PROFILE.to_owned()),
            })
        },
        || {
            for profile in &profiles {
                let selected = if is_last(profile) { "*" } else { " " };
                let kind = if profile.managed {
                    "isolated"
                } else {
                    "native"
                };
                let pinned = if is_default(profile) { "  default" } else { "" };
                println!("{selected} {:<32} {kind}{pinned}", profile.name);
            }
        },
    );
    Ok(())
}

fn show_status(store: &Store, requested_profile: Option<&str>, json: bool) -> Result<()> {
    let profile = resolve_profile(store, requested_profile)?;
    let statuses = Tool::ALL.map(|tool| (tool, launch::auth_status(tool, &profile)));

    report(
        json,
        || {
            json!({
                "profile": profile.name,
                "managed": profile.managed,
                "tools": statuses
                    .iter()
                    .map(|(tool, status)| json!({
                        "tool": tool.key(),
                        "label": tool.label(),
                        "status": status.key(),
                        "signed_in": *status == AuthStatus::SignedIn,
                    }))
                    .collect::<Vec<_>>(),
            })
        },
        || {
            println!("{}", profile.name);
            for (tool, status) in &statuses {
                print_auth_status(*tool, *status);
            }
        },
    );
    Ok(())
}

fn print_auth_status(tool: Tool, status: AuthStatus) {
    let status = match status {
        AuthStatus::SignedIn => "signed in",
        AuthStatus::SignedOut => "sign in required",
        AuthStatus::Unavailable => "CLI or status unavailable",
    };
    println!("  {:<13} {status}", tool.label());
}

fn create_profile(store: &Store, name: &str, json: bool) -> Result<()> {
    let profile = store.create_profile(name)?;
    report(
        json,
        || {
            let mut created = profile_paths(&profile);
            // A new profile is signed in to nothing, so the commands that fix
            // that are part of the answer rather than a note beside it.
            created["created"] = json!(true);
            created["sign_in"] = json!({
                "claude": format!("ditto-cli claude {} -- auth login", profile.name),
                "codex": format!("ditto-cli codex {} -- login", profile.name),
                "opencode": format!("ditto-cli opencode {} -- auth login", profile.name),
                "omp": format!("ditto-cli omp {} (then /login inside OMP)", profile.name),
            });
            created
        },
        || {
            println!("Created profile '{}'.", profile.name);
            print_login_instructions(&profile);
        },
    );
    Ok(())
}

/// Claude Code stores its credentials against the directory it was pointed at,
/// and renaming moves that directory, so the sign-in cannot survive. Saying so
/// before the move beats leaving it to be discovered at the next launch.
fn rename_profile(store: &Store, current_name: &str, new_name: &str, json: bool) -> Result<()> {
    let current = store.load_profile(current_name)?;
    let signs_out = launch::auth_status(Tool::Claude, &current) == AuthStatus::SignedIn;
    if signs_out && !json {
        println!(
            "Claude Code ties its credentials to the profile directory, which this \
             rename moves,\nso '{current_name}' will be signed out."
        );
        println!();
    }

    let profile = store.rename_profile(current_name, new_name)?;
    report(
        json,
        || {
            json!({
                "renamed": true,
                "from": current_name,
                "profile": profile.name,
                // Reported rather than warned about: the caller has already
                // committed by the time this is read.
                "claude_signed_out": signs_out,
            })
        },
        || {
            println!("Renamed profile '{current_name}' to '{}'.", profile.name);
            if signs_out {
                println!();
                println!("Sign Claude Code back in with:");
                println!("  ditto-cli claude {} -- auth login", profile.name);
            }
        },
    );
    Ok(())
}

/// Deleting is the one command that destroys something, so it asks to be meant
/// rather than merely typed. The refusal names the directories so the caller can
/// see what the confirmation is actually for.
fn delete_profile(store: &Store, arguments: DeleteArgs, json: bool) -> Result<()> {
    let name = arguments.profile;
    // Load first, so a name that does not exist fails as a missing profile
    // rather than as a missing confirmation.
    store.load_profile(&name)?;
    let targets = store.deletion_targets(&name);
    let listed = targets
        .iter()
        .map(|path| format!("  {}", path.display()))
        .collect::<Vec<_>>()
        .join("\n");

    if !arguments.yes {
        bail!(
            "deleting '{name}' removes its credentials, settings, and session history \
             for good:\n{listed}\nPass --yes to confirm."
        );
    }

    store.delete_profile(&name)?;
    report(
        json,
        || {
            json!({
                "deleted": true,
                "profile": name,
                "removed": targets
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect::<Vec<_>>(),
            })
        },
        || {
            println!("Deleted profile '{name}'.");
            println!("{listed}");
        },
    );
    Ok(())
}

/// Shows, pins, or releases the profile that commands fall back to. The pin is
/// otherwise reachable only by pressing `d` in the picker, which a script and an
/// agent have no way to do.
fn set_default_profile(store: &Store, arguments: DefaultArgs, json: bool) -> Result<()> {
    match arguments.action() {
        Some(DefaultAction::Pin(name)) => store.set_default_profile_name(Some(name))?,
        Some(DefaultAction::Clear) => store.set_default_profile_name(None)?,
        None => {}
    }

    let default_profile = store.default_profile_name()?;
    let fallback = store
        .fallback_profile_name()?
        .unwrap_or_else(|| DEFAULT_PROFILE.to_owned());

    report(
        json,
        || {
            json!({
                "default_profile": default_profile,
                "fallback_profile": fallback,
            })
        },
        || match &default_profile {
            Some(name) => println!("default profile: {name}"),
            // Naming what happens instead keeps the empty answer useful.
            None => println!("no default profile; commands fall back to {fallback}"),
        },
    );
    Ok(())
}

fn set_indicator(store: &Store, arguments: IndicatorArgs, json: bool) -> Result<()> {
    let profile = resolve_profile(store, arguments.profile.as_deref())?;
    let outcome = match arguments.action() {
        Some(IndicatorAction::On) => indicator::enable(&profile)?,
        Some(IndicatorAction::Off) => indicator::disable(&profile)?,
        None => indicator::state(&profile)?,
    };

    report(
        json,
        || {
            json!({
                "profile": profile.name,
                "outcome": outcome.key(),
                "on": outcome.is_on(),
                "description": outcome.describe(),
            })
        },
        || println!("{}: {}", profile.name, outcome.describe()),
    );
    Ok(())
}

fn show_paths(store: &Store, requested_profile: Option<&str>, json: bool) -> Result<()> {
    let profile = resolve_profile(store, requested_profile)?;
    report(
        json,
        || profile_paths(&profile),
        || {
            println!("profile={}", profile.name);
            println!("claude={}", profile.claude_home.display());
            println!("codex={}", profile.codex_home.display());
            println!("opencode={}", profile.opencode.data_dir().display());
            println!(
                "opencode-config={}",
                profile.opencode.config_dir().display()
            );
            println!("omp={}", profile.omp_home.display());
        },
    );
    Ok(())
}

/// The directories a profile owns, in the shape both `paths` and `create`
/// report them.
fn profile_paths(profile: &Profile) -> Value {
    json!({
        "profile": profile.name,
        "managed": profile.managed,
        "claude": profile.claude_home.display().to_string(),
        "codex": profile.codex_home.display().to_string(),
        "opencode": profile.opencode.data_dir().display().to_string(),
        "opencode_config": profile.opencode.config_dir().display().to_string(),
        "omp": profile.omp_home.display().to_string(),
    })
}

fn launch_direct(store: &Store, tool: Tool, arguments: LaunchArgs) -> Result<()> {
    let profile = resolve_profile(store, arguments.profile.as_deref())?;
    store.save_last_profile(&profile.name)?;
    launch::launch(tool, &profile, &arguments.args)
}

/// An explicit name always wins. Without one the saved fallback applies, and
/// with nothing saved this is the user's existing CLI configuration.
fn resolve_profile(store: &Store, requested_profile: Option<&str>) -> Result<Profile> {
    let name = match requested_profile {
        Some(name) => name.to_owned(),
        None => store
            .fallback_profile_name()?
            .unwrap_or_else(|| DEFAULT_PROFILE.to_owned()),
    };
    store.load_profile(&name)
}

fn print_login_instructions(profile: &Profile) {
    println!();
    println!(
        "Open `ditto-cli`, select '{}', then press l to sign in.",
        profile.name
    );
    println!();
    println!("Or authenticate directly:");
    println!("  ditto-cli claude {} -- auth login", profile.name);
    println!("  ditto-cli codex {} -- login", profile.name);
    println!("  ditto-cli opencode {} -- auth login", profile.name);
    println!();
    println!("Launch OMP, then use `/login` for each subscription provider:");
    println!("  ditto-cli omp {}", profile.name);
}
