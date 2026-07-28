mod cli;
mod indicator;
mod launch;
mod profile;
#[cfg(unix)]
mod proxy;
mod ui;
mod update;
mod workspace;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;

use cli::{
    AutoState, Cli, Command, IndicatorAction, IndicatorArgs, LaunchArgs, WorkspaceArgs,
    WorkspaceCommand,
};
use launch::{AuthStatus, Tool};
use profile::{DEFAULT_PROFILE, Profile, Store};
use workspace::{WORKSPACE_FILE, Workspaces};

fn main() {
    if let Err(error) = run() {
        eprintln!("ditto-cli: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let store = Store::discover()?;
    let workspaces = Workspaces::new(&store);

    match cli.command {
        None => run_tui(&store, &workspaces),
        Some(Command::List) => list_profiles(&store),
        Some(Command::Status { profile }) => show_status(&store, &workspaces, profile.as_deref()),
        Some(Command::Create { name }) => create_profile(&store, &name),
        Some(Command::Rename { profile, new_name }) => rename_profile(&store, &profile, &new_name),
        Some(Command::Workspace(arguments)) => run_workspace(&store, &workspaces, arguments),
        Some(Command::Paths { profile }) => show_paths(&store, &workspaces, profile.as_deref()),
        Some(Command::Claude(arguments)) => {
            launch_direct(&store, &workspaces, Tool::Claude, arguments)
        }
        Some(Command::Codex(arguments)) => {
            launch_direct(&store, &workspaces, Tool::Codex, arguments)
        }
        Some(Command::Opencode(arguments)) => {
            launch_direct(&store, &workspaces, Tool::Opencode, arguments)
        }
        Some(Command::Omp(arguments)) => launch_direct(&store, &workspaces, Tool::Omp, arguments),
        Some(Command::Indicator(arguments)) => set_indicator(&store, &workspaces, arguments),
        Some(Command::Statusline) => indicator::render(),
        Some(Command::Update(arguments)) => update::run(arguments.check, arguments.git),
    }
}

fn run_tui(store: &Store, workspaces: &Workspaces) -> Result<()> {
    // The directory decides where the cursor opens, falling back to the last
    // launch as it did before there were workspaces. Resolved once, outside the
    // loop, so signing in and coming back does not move the cursor off the
    // profile that was just signed in.
    let mut selected = match current_binding(workspaces) {
        Some(binding) if store.load_profile(&binding.profile).is_ok() => Some(binding.profile),
        _ => store.last_profile()?,
    };

    loop {
        let profiles = store.list_profiles()?;
        let default_profile = store.default_profile_name()?;
        let Some(action) = ui::run(store, profiles, selected.as_deref(), default_profile)? else {
            return Ok(());
        };

        match action {
            ui::UiAction::Launch { tool, profile } => {
                store.save_last_profile(&profile.name)?;
                auto_bind(store, workspaces, &profile.name)?;
                return launch::launch(tool, &profile, &[]);
            }
            ui::UiAction::Authenticate {
                operation,
                tool,
                profile,
            } => {
                store.save_last_profile(&profile.name)?;
                selected = Some(profile.name.clone());
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
fn show_status(
    store: &Store,
    workspaces: &Workspaces,
    requested_profile: Option<&str>,
) -> Result<()> {
    let profile = resolve_profile(store, workspaces, requested_profile)?;
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

fn set_indicator(store: &Store, workspaces: &Workspaces, arguments: IndicatorArgs) -> Result<()> {
    let profile = resolve_profile(store, workspaces, arguments.profile.as_deref())?;
    let outcome = match arguments.action() {
        Some(IndicatorAction::On) => indicator::enable(&profile)?,
        Some(IndicatorAction::Off) => indicator::disable(&profile)?,
        None => indicator::state(&profile)?,
    };
    println!("{}: {}", profile.name, outcome.describe());
    Ok(())
}

fn show_paths(
    store: &Store,
    workspaces: &Workspaces,
    requested_profile: Option<&str>,
) -> Result<()> {
    let profile = resolve_profile(store, workspaces, requested_profile)?;
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

fn launch_direct(
    store: &Store,
    workspaces: &Workspaces,
    tool: Tool,
    arguments: LaunchArgs,
) -> Result<()> {
    let profile = resolve_profile(store, workspaces, arguments.profile.as_deref())?;
    store.save_last_profile(&profile.name)?;
    auto_bind(store, workspaces, &profile.name)?;
    launch::launch(tool, &profile, &arguments.args)
}

/// An explicit name always wins. Without one the directory decides, then the
/// saved fallback, and with nothing saved this is the user's existing CLI
/// configuration.
fn resolve_profile(
    store: &Store,
    workspaces: &Workspaces,
    requested_profile: Option<&str>,
) -> Result<Profile> {
    let binding = current_binding(workspaces);

    if let Some(name) = requested_profile {
        let profile = store.load_profile(name)?;
        // Naming a profile inside a bound directory is a one-off, not a
        // rebinding, so the disagreement is worth saying out loud rather than
        // leaving to be noticed later.
        if let Some(binding) = &binding {
            if binding.profile != profile.name {
                eprintln!(
                    "ditto-cli: using '{}' for this run; {} still names '{}'",
                    profile.name,
                    binding.describe_origin(),
                    binding.profile
                );
            }
        }
        return Ok(profile);
    }

    if let Some(binding) = binding {
        match store.load_profile(&binding.profile) {
            Ok(profile) => return Ok(profile),
            // A binding outlives the profile it names as soon as that profile
            // is deleted. Saying so and falling through beats refusing to
            // launch anything from the directory until the file is repaired.
            Err(error) => eprintln!(
                "ditto-cli: {} names profile '{}', which is unavailable: {error:#}",
                binding.describe_origin(),
                binding.profile
            ),
        }
    }

    let name = store
        .fallback_profile_name()?
        .unwrap_or_else(|| DEFAULT_PROFILE.to_owned());
    store.load_profile(&name)
}

/// The binding covering the current directory. A directory that cannot be read
/// or a file that cannot be parsed is reported and then ignored, because every
/// caller has a working answer without one and none of them is worth failing.
fn current_binding(workspaces: &Workspaces) -> Option<workspace::Binding> {
    match current_directory().and_then(|directory| workspaces.find(&directory)) {
        Ok(binding) => binding,
        Err(error) => {
            eprintln!("ditto-cli: ignoring this directory's profile: {error:#}");
            None
        }
    }
}

/// Records the profile a directory launched with, so the next launch from it
/// needs no name. A directory already answered for by itself or an ancestor is
/// left alone, which is what keeps every subdirectory of a bound project from
/// collecting a file of its own.
fn auto_bind(store: &Store, workspaces: &Workspaces, profile: &str) -> Result<()> {
    if !store.workspace_auto_bind()? {
        return Ok(());
    }
    let Ok(directory) = current_directory() else {
        return Ok(());
    };
    if !workspaces.may_auto_bind(&directory) || workspaces.find(&directory)?.is_some() {
        return Ok(());
    }

    match workspaces.bind_file(&directory, profile) {
        Ok(path) => println!("Bound this directory to '{profile}' ({}).", path.display()),
        // A directory there is no permission to write in is not a reason to
        // refuse the launch that was actually asked for.
        Err(error) => eprintln!("ditto-cli: could not bind this directory: {error:#}"),
    }
    Ok(())
}

fn run_workspace(store: &Store, workspaces: &Workspaces, arguments: WorkspaceArgs) -> Result<()> {
    match arguments.command {
        None => show_workspace(workspaces),
        Some(WorkspaceCommand::Use {
            profile,
            global,
            path,
        }) => bind_workspace(store, workspaces, &profile, global, path),
        Some(WorkspaceCommand::Clear { path }) => clear_workspace(workspaces, path),
        Some(WorkspaceCommand::List) => list_workspaces(workspaces),
        Some(WorkspaceCommand::Auto { state }) => set_auto_bind(store, state),
    }
}

fn show_workspace(workspaces: &Workspaces) -> Result<()> {
    let directory = current_directory()?;
    println!("directory={}", directory.display());
    match workspaces.find(&directory)? {
        Some(binding) => {
            println!("profile={}", binding.profile);
            println!("source={}", binding.describe_origin());
        }
        None => println!("profile=<unbound>"),
    }
    Ok(())
}

fn bind_workspace(
    store: &Store,
    workspaces: &Workspaces,
    profile: &str,
    global: bool,
    path: Option<PathBuf>,
) -> Result<()> {
    // Checked before anything is written, so a typo cannot leave a directory
    // bound to a profile that has never existed.
    let profile = store.load_profile(profile)?;
    let directory = directory_argument(path)?;

    if global {
        let directory = workspaces.bind_registry(&directory, &profile.name)?;
        println!("Bound {} to '{}'.", directory.display(), profile.name);
        println!("Recorded in {}.", workspaces.registry_path().display());
    } else {
        let path = workspaces.bind_file(&directory, &profile.name)?;
        println!("Bound {} to '{}'.", directory.display(), profile.name);
        println!("Wrote {}.", path.display());
    }
    Ok(())
}

fn clear_workspace(workspaces: &Workspaces, path: Option<PathBuf>) -> Result<()> {
    let directory = directory_argument(path)?;
    let removed = workspaces.clear(&directory)?;
    if removed.is_empty() {
        println!("{} was not bound.", directory.display());
    } else {
        for entry in removed {
            println!("Removed {entry}.");
        }
    }

    // Clearing only ever touches the one directory, so an ancestor can still be
    // answering for it and the user should not have to re-run to discover that.
    if let Some(binding) = workspaces.find(&directory)? {
        println!(
            "{} now inherits '{}' from {}.",
            directory.display(),
            binding.profile,
            binding.describe_origin()
        );
    }
    Ok(())
}

fn list_workspaces(workspaces: &Workspaces) -> Result<()> {
    let entries = workspaces.entries()?;
    if entries.is_empty() {
        println!("No directories are recorded in the registry.");
    } else {
        for (directory, profile) in entries {
            println!("{:<48} {profile}", directory.display());
        }
    }

    // Only the registry can be enumerated. Saying so keeps this from reading as
    // the complete list of bindings when most of them are usually files.
    println!();
    println!(
        "{WORKSPACE_FILE} files are not listed: they are found by walking up from the directory"
    );
    println!("Ditto runs in. Run `ditto-cli workspace` to see the one in effect here.");
    Ok(())
}

fn set_auto_bind(store: &Store, state: Option<AutoState>) -> Result<()> {
    if let Some(state) = state {
        store.set_workspace_auto_bind(state.enabled())?;
    }

    let enabled = store.workspace_auto_bind()?;
    println!(
        "Launching from an unbound directory {} it.",
        if enabled { "binds" } else { "does not bind" }
    );
    Ok(())
}

fn directory_argument(path: Option<PathBuf>) -> Result<PathBuf> {
    match path {
        Some(path) => {
            if !path.is_dir() {
                anyhow::bail!("{} is not a directory", path.display());
            }
            Ok(path)
        }
        None => current_directory(),
    }
}

fn current_directory() -> Result<PathBuf> {
    std::env::current_dir().context("could not determine the current directory")
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
