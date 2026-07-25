mod cli;
mod indicator;
mod launch;
mod profile;
#[cfg(unix)]
mod proxy;
mod ui;
mod update;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Command, IndicatorAction, IndicatorArgs, LaunchArgs};
use launch::{AuthStatus, Tool};
use profile::{DEFAULT_PROFILE, Profile, Store};

fn main() {
    if let Err(error) = run() {
        eprintln!("ditto-cli: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let store = Store::discover()?;

    match cli.command {
        None => run_tui(&store),
        Some(Command::List) => list_profiles(&store),
        Some(Command::Status { profile }) => show_status(&store, profile.as_deref()),
        Some(Command::Create { name }) => create_profile(&store, &name),
        Some(Command::Rename { profile, new_name }) => rename_profile(&store, &profile, &new_name),
        Some(Command::Paths { profile }) => show_paths(&store, profile.as_deref()),
        Some(Command::Claude(arguments)) => launch_direct(&store, Tool::Claude, arguments),
        Some(Command::Codex(arguments)) => launch_direct(&store, Tool::Codex, arguments),
        Some(Command::Opencode(arguments)) => launch_direct(&store, Tool::Opencode, arguments),
        Some(Command::Omp(arguments)) => launch_direct(&store, Tool::Omp, arguments),
        Some(Command::Indicator(arguments)) => set_indicator(&store, arguments),
        Some(Command::Statusline) => indicator::render(),
        Some(Command::Update(arguments)) => update::run(arguments.check, arguments.git),
    }
}

fn run_tui(store: &Store) -> Result<()> {
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

fn list_profiles(store: &Store) -> Result<()> {
    let last_profile = store.last_profile()?;
    let default_profile = store.default_profile_name()?;
    for profile in store.list_profiles()? {
        let selected = if last_profile.as_deref() == Some(&profile.name) {
            "*"
        } else {
            " "
        };
        let kind = if profile.managed {
            "isolated"
        } else {
            "native"
        };
        let pinned = if default_profile.as_deref() == Some(&profile.name) {
            "  default"
        } else {
            ""
        };
        println!("{selected} {:<32} {kind}{pinned}", profile.name);
    }
    Ok(())
}
fn show_status(store: &Store, requested_profile: Option<&str>) -> Result<()> {
    let profile = resolve_profile(store, requested_profile)?;
    println!("{}", profile.name);
    for tool in Tool::ALL {
        print_auth_status(tool, launch::auth_status(tool, &profile));
    }
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

fn create_profile(store: &Store, name: &str) -> Result<()> {
    let profile = store.create_profile(name)?;
    println!("Created profile '{}'.", profile.name);
    print_login_instructions(&profile);
    Ok(())
}
/// Claude Code stores its credentials against the directory it was pointed at,
/// and renaming moves that directory, so the sign-in cannot survive. Saying so
/// before the move beats leaving it to be discovered at the next launch.
fn rename_profile(store: &Store, current_name: &str, new_name: &str) -> Result<()> {
    let current = store.load_profile(current_name)?;
    let signs_out = launch::auth_status(Tool::Claude, &current) == AuthStatus::SignedIn;
    if signs_out {
        println!(
            "Claude Code ties its credentials to the profile directory, which this \
             rename moves,\nso '{current_name}' will be signed out."
        );
        println!();
    }

    let profile = store.rename_profile(current_name, new_name)?;
    println!("Renamed profile '{current_name}' to '{}'.", profile.name);
    if signs_out {
        println!();
        println!("Sign Claude Code back in with:");
        println!("  ditto-cli claude {} -- auth login", profile.name);
    }
    Ok(())
}

fn set_indicator(store: &Store, arguments: IndicatorArgs) -> Result<()> {
    let profile = resolve_profile(store, arguments.profile.as_deref())?;
    let outcome = match arguments.action() {
        Some(IndicatorAction::On) => indicator::enable(&profile)?,
        Some(IndicatorAction::Off) => indicator::disable(&profile)?,
        None => indicator::state(&profile)?,
    };
    println!("{}: {}", profile.name, outcome.describe());
    Ok(())
}

fn show_paths(store: &Store, requested_profile: Option<&str>) -> Result<()> {
    let profile = resolve_profile(store, requested_profile)?;
    println!("profile={}", profile.name);
    println!("claude={}", profile.claude_home.display());
    println!("codex={}", profile.codex_home.display());
    println!("opencode={}", profile.opencode.data_dir().display());
    println!(
        "opencode-config={}",
        profile.opencode.config_dir().display()
    );
    println!("omp={}", profile.omp_home.display());
    Ok(())
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
