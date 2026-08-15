mod cli;
mod herdr;
mod indicator;
mod launch;
mod profile;
mod program;
#[cfg(unix)]
mod proxy;
mod settings;
mod shared;
mod shell;
mod ui;
mod update;
mod workspace;

use std::{
    io::{self, IsTerminal},
    path::PathBuf,
};

use anyhow::{Context, Result, bail};
use clap::Parser;
use serde_json::{Value, json};

use cli::{
    AutoState, Cli, Command, DefaultAction, DefaultArgs, DeleteArgs, IndicatorAction,
    IndicatorArgs, LaunchArgs, ShellInitArgs, SyncArgs, WorkspaceArgs, WorkspaceCommand,
};
use indicator::Existing;
use launch::{AuthStatus, Tool};
use profile::{DEFAULT_PROFILE, Fallback, Profile, Store};
use workspace::{WORKSPACE_FILE, Workspaces};

fn main() {
    restore_sigpipe();

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

/// Ends quietly when the reader of Ditto's output goes away.
///
/// Rust ignores `SIGPIPE` so that a write to a closed pipe returns an error
/// instead of killing the process, but the print macros then panic on that
/// error. Every reporting command here prints lines someone may well pipe into
/// `head`, and a panic and a backtrace is not the answer that deserves, so the
/// default disposition is restored and Ditto ends the way every other tool in
/// the pipeline does.
#[cfg(unix)]
fn restore_sigpipe() {
    // SAFETY: called before any thread is started, and `SIG_DFL` is the
    // disposition the process began life with before Rust's runtime changed it.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
fn restore_sigpipe() {}

fn run(cli: Cli) -> Result<()> {
    let store = Store::discover()?;
    let workspaces = Workspaces::new(&store);
    let json = cli.json;

    match cli.command {
        None => run_tui(&store, &workspaces),
        Some(Command::List) => list_profiles(&store, &workspaces, json),
        Some(Command::Status { profile }) => {
            show_status(&store, &workspaces, profile.as_deref(), json)
        }
        Some(Command::Create { name }) => create_profile(&store, &name, json),
        Some(Command::Rename { profile, new_name }) => {
            rename_profile(&store, &profile, &new_name, json)
        }
        Some(Command::Delete(arguments)) => delete_profile(&store, arguments, json),
        Some(Command::Sync(arguments)) => sync_settings(&store, &workspaces, arguments, json),
        Some(Command::Default(arguments)) => {
            set_default_profile(&store, &workspaces, arguments, json)
        }
        Some(Command::Workspace(arguments)) => run_workspace(&store, &workspaces, arguments, json),
        Some(Command::Paths { profile }) => {
            show_paths(&store, &workspaces, profile.as_deref(), json)
        }
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
        Some(Command::PrimeAgent(arguments)) => {
            launch_direct(&store, &workspaces, Tool::PrimeAgent, arguments)
        }
        Some(Command::ShellInit(arguments)) => print_shell_init(arguments),
        Some(Command::Indicator(arguments)) => set_indicator(&store, &workspaces, arguments, json),
        Some(Command::Statusline(arguments)) => {
            indicator::render(arguments.with, arguments.with_encoded)
        }
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
fn run_tui(store: &Store, workspaces: &Workspaces) -> Result<()> {
    if !io::stdout().is_terminal() || !io::stdin().is_terminal() {
        bail!(
            "the profile picker needs an interactive terminal, and this is not one.\n\
             Every command works without it:\n\
            \x20 ditto-cli list --json\n\
            \x20 ditto-cli status <profile> --json\n\
            \x20 ditto-cli create <profile>\n\
            \x20 ditto-cli default <profile>\n\
            \x20 ditto-cli workspace use <profile>\n\
             Run `ditto-cli --help` for the full set."
        );
    }

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

fn list_profiles(store: &Store, workspaces: &Workspaces, json: bool) -> Result<()> {
    let last_profile = store.last_profile()?;
    let default_profile = store.default_profile_name()?;
    let profiles = store.list_profiles()?;
    let binding = current_binding(workspaces);
    let fallback = effective_fallback(store, binding.as_ref())?;

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
                // caller does not have to re-derive the precedence rule. The
                // directory outranks both saved values, so it is answered from
                // here rather than from the state file alone.
                "fallback_profile": fallback,
                "workspace": binding_payload(binding.as_ref()),
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

fn show_status(
    store: &Store,
    workspaces: &Workspaces,
    requested_profile: Option<&str>,
    json: bool,
) -> Result<()> {
    let (profile, _) = resolve_profile(store, workspaces, requested_profile)?;
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
    // Claude Code reads its settings from the directory a launch moves, so a
    // new profile would otherwise start with none of the permission mode,
    // model, or hooks its owner set up once and expects everywhere.
    let copied = settings::seed(store, &profile);
    // Skills, subagents, commands, hooks and plugins are not accounts, and a
    // profile that started without them would be a different working
    // environment rather than the same one signed in as somebody else.
    let linked = shared::seed(store, &profile);

    report(
        json,
        || {
            let mut created = profile_paths(&profile);
            // A new profile is signed in to nothing, so the commands that fix
            // that are part of the answer rather than a note beside it.
            created["created"] = json!(true);
            created["settings_copied"] = json!(copied.copied);
            created["shared"] = json!(linked.linked);
            created["sign_in"] = json!({
                "claude": format!("ditto-cli claude {} -- auth login", profile.name),
                "codex": format!("ditto-cli codex {} -- login", profile.name),
                "opencode": format!("ditto-cli opencode {} -- auth login", profile.name),
                "omp": format!("ditto-cli omp {} (then /login inside OMP)", profile.name),
                "prime-agent": format!(
                    "ditto-cli prime-agent {} -- /login",
                    profile.name
                ),
            });
            created
        },
        || {
            println!("Created profile '{}'.", profile.name);
            if copied.changed() {
                println!(
                    "Copied your Claude Code settings into it: {}.",
                    copied.copied.join(", ")
                );
            }
            if !linked.linked.is_empty() {
                println!("Reading yours for: {}.", linked.linked.join(", "));
            }
            print_login_instructions(&profile);
        },
    );
    Ok(())
}

/// Brings a profile that already exists up to the settings a new one would be
/// created with. Profiles made before Ditto copied anything started empty, and
/// a setting changed since then reaches them no other way.
fn sync_settings(
    store: &Store,
    workspaces: &Workspaces,
    arguments: SyncArgs,
    json: bool,
) -> Result<()> {
    let (profile, _) = resolve_profile(store, workspaces, arguments.profile.as_deref())?;
    let source = store.load_profile(DEFAULT_PROFILE)?;
    let copied = settings::copy(&source, &profile, arguments.overwrite)?;
    let linked = shared::link(&source, &profile, arguments.adopt)?;
    // A launch repairs the tool it is about to start; asking for a sync is
    // asking about the profile, so this answers for every tool at once.
    let repaired = shared::repair(&profile);

    report(
        json,
        || {
            json!({
                "profile": profile.name,
                "source": source.name,
                "copied": copied.copied,
                "kept": copied.kept,
                "shared": linked.linked,
                "shared_kept": linked.kept,
                "shared_failed": linked
                    .failed
                    .iter()
                    .map(|(path, reason)| json!({ "path": path, "reason": reason }))
                    .collect::<Vec<_>>(),
                "repaired": repaired.links,
                "repair_failed": repaired
                    .failed
                    .iter()
                    .map(|(path, reason)| json!({ "path": path, "reason": reason }))
                    .collect::<Vec<_>>(),
                "changed": copied.changed() || linked.changed() || repaired.changed(),
            })
        },
        || {
            if copied.changed() {
                println!(
                    "Copied into '{}': {}.",
                    profile.name,
                    copied.copied.join(", ")
                );
            } else {
                println!(
                    "'{}' already has every setting your own configuration sets.",
                    profile.name
                );
            }
            if !copied.kept.is_empty() {
                println!(
                    "Left '{}' as it is for: {}.",
                    profile.name,
                    copied.kept.join(", ")
                );
                println!(
                    "  Replace those too with `ditto-cli sync {} --overwrite`.",
                    profile.name
                );
            }
            if !linked.linked.is_empty() {
                println!("Reading yours for: {}.", linked.linked.join(", "));
            }
            if !linked.kept.is_empty() {
                println!(
                    "'{}' has its own and keeps it: {}.",
                    profile.name,
                    linked.kept.join(", ")
                );
                println!(
                    "  Point those at yours too with `ditto-cli sync {} --adopt`, \
                     which moves what is there aside rather than deleting it.",
                    profile.name
                );
            }
            for (path, reason) in &linked.failed {
                println!("Could not share {path}: {reason}");
            }
            if !repaired.links.is_empty() {
                println!(
                    "Repaired links installed pointing at nothing: {}.",
                    repaired.links.join(", ")
                );
            }
            for (path, reason) in &repaired.failed {
                println!("Could not repair {path}: {reason}");
            }
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
fn set_default_profile(
    store: &Store,
    workspaces: &Workspaces,
    arguments: DefaultArgs,
    json: bool,
) -> Result<()> {
    match arguments.action() {
        Some(DefaultAction::Pin(name)) => store.set_default_profile_name(Some(name))?,
        Some(DefaultAction::Clear) => store.set_default_profile_name(None)?,
        None => {}
    }

    let default_profile = store.default_profile_name()?;
    let binding = current_binding(workspaces);
    let fallback = effective_fallback(store, binding.as_ref())?;

    report(
        json,
        || {
            json!({
                "default_profile": default_profile,
                "fallback_profile": fallback,
                "workspace": binding_payload(binding.as_ref()),
            })
        },
        || {
            match &default_profile {
                Some(name) => println!("default profile: {name}"),
                // Naming what happens instead keeps the empty answer useful.
                None => println!("no default profile; commands fall back to {fallback}"),
            }
            // The pin is not what this directory would actually use, and saying
            // only the pin would read as though it were.
            if let Some(binding) = &binding {
                println!(
                    "here, {} outranks it and names '{}'",
                    binding.describe_origin(),
                    binding.profile
                );
            }
        },
    );
    Ok(())
}

/// Prints shell functions rather than a report, so it has no JSON form: the
/// output is a script for a shell to read, and wrapping it in JSON would leave
/// the caller to unwrap it before the shell could.
fn print_shell_init(arguments: ShellInitArgs) -> Result<()> {
    let shell = match arguments.shell {
        Some(shell) => shell,
        None => shell::detect()?,
    };
    print!("{}", shell::script(shell));
    Ok(())
}

fn set_indicator(
    store: &Store,
    workspaces: &Workspaces,
    arguments: IndicatorArgs,
    json: bool,
) -> Result<()> {
    let (profile, _) = resolve_profile(store, workspaces, arguments.profile.as_deref())?;
    let existing = if arguments.keep_mine {
        Existing::KeepAlongside
    } else {
        Existing::LeaveAlone
    };
    // Whether the status line will actually be seen depends on the directory
    // this was typed in, so it is answered here rather than by the profile.
    let outcome = indicator::shadowed(match arguments.action() {
        Some(IndicatorAction::On) => indicator::enable(&profile, existing)?,
        Some(IndicatorAction::Off) => indicator::disable(&profile)?,
        None => indicator::state(&profile)?,
    });

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

fn show_paths(
    store: &Store,
    workspaces: &Workspaces,
    requested_profile: Option<&str>,
    json: bool,
) -> Result<()> {
    let (profile, _) = resolve_profile(store, workspaces, requested_profile)?;
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
            println!("prime-agent={}", profile.prime_agent_home.display());
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
        "prime_agent": profile.prime_agent_home.display().to_string(),
    })
}

fn launch_direct(
    store: &Store,
    workspaces: &Workspaces,
    tool: Tool,
    arguments: LaunchArgs,
) -> Result<()> {
    let (profile, fell_back) = resolve_profile(store, workspaces, arguments.profile.as_deref())?;
    store.save_last_profile(&profile.name)?;

    // The saved fallback is the one answer nothing on screen points at: the
    // command did not name it and the directory does not either, which is how a
    // launch ends up in the wrong profile with nothing having said so. Saying it
    // before the tool takes the terminal is the last chance to notice. Reporting
    // commands stay quiet, since a fallback is their ordinary case and they are
    // run in loops.
    if let Some(fallback) = &fell_back {
        eprintln!(
            "ditto-cli: using '{}'; nothing binds this directory, and it is {}",
            profile.name,
            fallback.describe()
        );
    }

    let bound = auto_bind(store, workspaces, &profile.name)?;
    if fell_back.is_some() && !bound {
        eprintln!(
            "ditto-cli: bind this directory with `ditto-cli workspace use <profile>`, or name a \
             profile with `ditto-cli {} <profile>`",
            tool.key()
        );
    }

    launch::launch(tool, &profile, &arguments.args)
}

/// An explicit name always wins. Without one the directory decides, then the
/// saved fallback, and with nothing saved this is the user's existing CLI
/// configuration.
///
/// The second half of the answer is the saved value that chose the profile, and
/// only when neither the command nor the directory did. Callers that hand the
/// terminal to a tool report it; the rest have no reason to.
fn resolve_profile(
    store: &Store,
    workspaces: &Workspaces,
    requested_profile: Option<&str>,
) -> Result<(Profile, Option<Fallback>)> {
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
        return Ok((profile, None));
    }

    if let Some(binding) = binding {
        match store.load_profile(&binding.profile) {
            Ok(profile) => return Ok((profile, None)),
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

    let fallback = store.fallback_profile()?;
    let profile = store.load_profile(fallback.name())?;
    Ok((profile, Some(fallback)))
}

/// The profile a command naming none would use here: the directory's binding
/// when it has a usable one, and the saved fallback otherwise.
///
/// Both `list` and `default` report this, so the precedence rule is stated once
/// rather than restated wherever it is answered.
fn effective_fallback(store: &Store, binding: Option<&workspace::Binding>) -> Result<String> {
    if let Some(binding) = binding {
        // A binding naming a profile that no longer exists is one `resolve`
        // would fall through, so it is not the answer here either.
        if store.load_profile(&binding.profile).is_ok() {
            return Ok(binding.profile.clone());
        }
    }
    Ok(store.fallback_profile()?.name().to_owned())
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
///
/// Answers whether the directory came out of this bound, so a caller does not
/// have to work out for itself which of the reasons for declining applied.
fn auto_bind(store: &Store, workspaces: &Workspaces, profile: &str) -> Result<bool> {
    if !store.workspace_auto_bind()? {
        return Ok(false);
    }
    let Ok(directory) = current_directory() else {
        return Ok(false);
    };
    if !workspaces.may_auto_bind(&directory) || workspaces.find(&directory)?.is_some() {
        return Ok(false);
    }

    match workspaces.bind_file(&directory, profile) {
        Ok(path) => {
            println!("Bound this directory to '{profile}' ({}).", path.display());
            Ok(true)
        }
        // A directory there is no permission to write in is not a reason to
        // refuse the launch that was actually asked for.
        Err(error) => {
            eprintln!("ditto-cli: could not bind this directory: {error:#}");
            Ok(false)
        }
    }
}

fn run_workspace(
    store: &Store,
    workspaces: &Workspaces,
    arguments: WorkspaceArgs,
    json: bool,
) -> Result<()> {
    match arguments.command {
        None => show_workspace(workspaces, json),
        Some(WorkspaceCommand::Use {
            profile,
            global,
            path,
        }) => bind_workspace(store, workspaces, &profile, global, path, json),
        Some(WorkspaceCommand::Clear { path }) => clear_workspace(workspaces, path, json),
        Some(WorkspaceCommand::List) => list_workspaces(workspaces, json),
        Some(WorkspaceCommand::Auto { state }) => set_auto_bind(store, state, json),
    }
}

/// A binding in the shape every workspace command reports it, or null where
/// there is none, so a caller can read one field rather than tell an absent
/// binding apart from a failed lookup.
fn binding_payload(binding: Option<&workspace::Binding>) -> Value {
    match binding {
        Some(binding) => json!({
            "profile": binding.profile,
            "directory": binding.directory.display().to_string(),
            "origin": binding.origin_key(),
            "source": binding.describe_origin(),
        }),
        None => Value::Null,
    }
}

fn show_workspace(workspaces: &Workspaces, json: bool) -> Result<()> {
    let directory = current_directory()?;
    let binding = workspaces.find(&directory)?;

    report(
        json,
        || {
            json!({
                "directory": directory.display().to_string(),
                "bound": binding.is_some(),
                "binding": binding_payload(binding.as_ref()),
            })
        },
        || {
            println!("directory={}", directory.display());
            match &binding {
                Some(binding) => {
                    println!("profile={}", binding.profile);
                    println!("source={}", binding.describe_origin());
                }
                None => println!("profile=<unbound>"),
            }
        },
    );
    Ok(())
}

fn bind_workspace(
    store: &Store,
    workspaces: &Workspaces,
    profile: &str,
    global: bool,
    path: Option<PathBuf>,
    json: bool,
) -> Result<()> {
    // Checked before anything is written, so a typo cannot leave a directory
    // bound to a profile that has never existed.
    let profile = store.load_profile(profile)?;
    let directory = directory_argument(path)?;

    let (directory, written) = if global {
        let directory = workspaces.bind_registry(&directory, &profile.name)?;
        let written = workspaces.registry_path().display().to_string();
        (directory, written)
    } else {
        let path = workspaces.bind_file(&directory, &profile.name)?;
        (directory, path.display().to_string())
    };

    report(
        json,
        || {
            json!({
                "profile": profile.name,
                "directory": directory.display().to_string(),
                "origin": if global { "registry" } else { "file" },
                "written": written,
            })
        },
        || {
            println!("Bound {} to '{}'.", directory.display(), profile.name);
            if global {
                println!("Recorded in {written}.");
            } else {
                println!("Wrote {written}.");
            }
        },
    );
    Ok(())
}

fn clear_workspace(workspaces: &Workspaces, path: Option<PathBuf>, json: bool) -> Result<()> {
    let directory = directory_argument(path)?;
    let removed = workspaces.clear(&directory)?;
    // Clearing only ever touches the one directory, so an ancestor can still be
    // answering for it and the caller should not have to re-run to discover it.
    let inherited = workspaces.find(&directory)?;

    report(
        json,
        || {
            json!({
                "directory": directory.display().to_string(),
                "removed": removed,
                "inherits": binding_payload(inherited.as_ref()),
            })
        },
        || {
            if removed.is_empty() {
                println!("{} was not bound.", directory.display());
            } else {
                for entry in &removed {
                    println!("Removed {entry}.");
                }
            }
            if let Some(binding) = &inherited {
                println!(
                    "{} now inherits '{}' from {}.",
                    directory.display(),
                    binding.profile,
                    binding.describe_origin()
                );
            }
        },
    );
    Ok(())
}

fn list_workspaces(workspaces: &Workspaces, json: bool) -> Result<()> {
    let entries = workspaces.entries()?;

    report(
        json,
        || {
            json!({
                "workspaces": entries
                    .iter()
                    .map(|(directory, profile)| json!({
                        "directory": directory.display().to_string(),
                        "profile": profile,
                    }))
                    .collect::<Vec<_>>(),
                "registry": workspaces.registry_path().display().to_string(),
                // Files are found by walking up from wherever Ditto runs, so
                // there is no set of them to enumerate. Saying so in the payload
                // keeps this from reading as every binding that exists.
                "includes_files": false,
            })
        },
        || {
            if entries.is_empty() {
                println!("No directories are recorded in the registry.");
            } else {
                for (directory, profile) in &entries {
                    println!("{:<48} {profile}", directory.display());
                }
            }
            println!();
            println!(
                "{WORKSPACE_FILE} files are not listed: they are found by walking up from the \
                 directory"
            );
            println!("Ditto runs in. Run `ditto-cli workspace` to see the one in effect here.");
        },
    );
    Ok(())
}

fn set_auto_bind(store: &Store, state: Option<AutoState>, json: bool) -> Result<()> {
    if let Some(state) = state {
        store.set_workspace_auto_bind(state.enabled())?;
    }

    let enabled = store.workspace_auto_bind()?;
    report(
        json,
        || json!({ "auto_bind": enabled }),
        || {
            println!(
                "Launching from an unbound directory {} it.",
                if enabled { "binds" } else { "does not bind" }
            );
        },
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
        "Open `ditto-cli`, select '{}', then press l to sign in to Claude Code, Codex, opencode, or Prime Agent.",
        profile.name
    );
    println!();
    println!("Or authenticate directly:");
    println!("  ditto-cli claude {} -- auth login", profile.name);
    println!("  ditto-cli codex {} -- login", profile.name);
    println!("  ditto-cli opencode {} -- auth login", profile.name);
    println!("  ditto-cli prime-agent {} -- /login", profile.name);
    println!();
    println!("Launch OMP, then use `/login` inside it:");
    println!("  ditto-cli omp {}", profile.name);
}
