use std::{ffi::OsString, path::PathBuf};

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "ditto-cli",
    version,
    about = "Launch Claude Code, Codex, opencode, and OMP with isolated profiles"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// List available profiles.
    List,
    /// Show Claude Code, Codex, opencode, and OMP authentication status.
    Status {
        /// Profile name. Uses the last selected profile when omitted.
        profile: Option<String>,
    },
    /// Create an isolated profile.
    Create {
        /// Profile name: letters, numbers, '-' and '_'.
        name: String,
    },
    /// Rename an isolated profile.
    Rename {
        /// Current profile name.
        profile: String,
        /// New profile name.
        new_name: String,
    },
    /// Show or change the profile a directory launches with.
    Workspace(WorkspaceArgs),
    /// Show the Claude, Codex, opencode, and OMP directories for a profile.
    Paths {
        /// Profile name. Uses the last selected profile when omitted.
        profile: Option<String>,
    },
    /// Launch Claude Code.
    #[command(visible_alias = "cc")]
    Claude(LaunchArgs),
    /// Launch Codex.
    #[command(visible_alias = "cx")]
    Codex(LaunchArgs),
    /// Launch opencode.
    #[command(visible_alias = "oc")]
    Opencode(LaunchArgs),
    /// Launch Oh My Pi.
    Omp(LaunchArgs),
    /// Show or change the profile indicator Claude Code displays.
    Indicator(IndicatorArgs),
    /// Print the profile status line. Claude Code runs this itself.
    #[command(hide = true)]
    Statusline,
    /// Update Ditto CLI itself.
    Update(UpdateArgs),
}

#[derive(Debug, Args)]
pub struct WorkspaceArgs {
    #[command(subcommand)]
    pub command: Option<WorkspaceCommand>,
}

#[derive(Debug, Subcommand)]
pub enum WorkspaceCommand {
    /// Bind a directory to a profile.
    Use {
        /// Profile the directory should launch with.
        profile: String,
        /// Record the binding in Ditto's own registry rather than writing a
        /// file, for directories that are not yours to add a file to.
        #[arg(long)]
        global: bool,
        /// Directory to bind. Uses the current directory when omitted.
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Remove the binding on a directory, leaving its ancestors alone.
    Clear {
        /// Directory to unbind. Uses the current directory when omitted.
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// List the directories recorded in Ditto's own registry.
    List,
    /// Show or change whether launching binds an unbound directory.
    Auto {
        /// Reports the current setting when omitted.
        state: Option<AutoState>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum AutoState {
    On,
    Off,
}

impl AutoState {
    pub fn enabled(self) -> bool {
        matches!(self, Self::On)
    }
}

#[derive(Debug, Args)]
pub struct IndicatorArgs {
    /// Profile name. Uses the saved profile when omitted.
    pub profile: Option<String>,
    /// Show the profile in Claude Code's status line.
    #[arg(long, conflicts_with = "off")]
    pub on: bool,
    /// Take the profile back out of Claude Code's status line.
    #[arg(long)]
    pub off: bool,
}

impl IndicatorArgs {
    /// Neither flag means the command was asked to report rather than change.
    pub fn action(&self) -> Option<IndicatorAction> {
        match (self.on, self.off) {
            (true, _) => Some(IndicatorAction::On),
            (_, true) => Some(IndicatorAction::Off),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IndicatorAction {
    On,
    Off,
}

#[derive(Debug, Args)]
pub struct UpdateArgs {
    /// Compare the installed and published versions without installing.
    #[arg(long)]
    pub check: bool,
    /// Install from the Git repository instead of crates.io.
    #[arg(long)]
    pub git: bool,
}

#[derive(Debug, Args)]
pub struct LaunchArgs {
    /// Profile name. Uses the last selected profile when omitted.
    pub profile: Option<String>,
    /// Arguments passed to the underlying CLI. Place them after `--`.
    #[arg(last = true)]
    pub args: Vec<OsString>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_profile_rename_command() {
        let cli = Cli::try_parse_from(["ditto-cli", "rename", "work", "client"]).unwrap();

        assert!(matches!(
            cli.command,
            Some(Command::Rename {
                profile,
                new_name
            }) if profile == "work" && new_name == "client"
        ));
    }
    #[test]
    fn parses_update_flags() {
        let plain = Cli::try_parse_from(["ditto-cli", "update"]).unwrap();
        assert!(matches!(
            plain.command,
            Some(Command::Update(UpdateArgs { check, git })) if !check && !git
        ));

        let checking = Cli::try_parse_from(["ditto-cli", "update", "--check"]).unwrap();
        assert!(matches!(
            checking.command,
            Some(Command::Update(UpdateArgs { check, git })) if check && !git
        ));

        let from_git = Cli::try_parse_from(["ditto-cli", "update", "--git"]).unwrap();
        assert!(matches!(
            from_git.command,
            Some(Command::Update(UpdateArgs { check, git })) if !check && git
        ));
    }

    #[test]
    fn parses_opencode_launch_arguments() {
        let cli = Cli::try_parse_from([
            "ditto-cli",
            "opencode",
            "work",
            "--",
            "--model",
            "anthropic/claude-opus-5",
        ])
        .unwrap();

        assert!(matches!(
            cli.command,
            Some(Command::Opencode(LaunchArgs { profile, args }))
                if profile.as_deref() == Some("work")
                    && args == [
                        OsString::from("--model"),
                        OsString::from("anthropic/claude-opus-5"),
                    ]
        ));
    }

    #[test]
    fn parses_short_launch_aliases() {
        let claude = Cli::try_parse_from(["ditto-cli", "cc", "work"]).unwrap();
        assert!(matches!(
            claude.command,
            Some(Command::Claude(LaunchArgs { profile, args }))
                if profile.as_deref() == Some("work") && args.is_empty()
        ));

        let codex = Cli::try_parse_from(["ditto-cli", "cx", "work"]).unwrap();
        assert!(matches!(
            codex.command,
            Some(Command::Codex(LaunchArgs { profile, .. })) if profile.as_deref() == Some("work")
        ));

        let opencode =
            Cli::try_parse_from(["ditto-cli", "oc", "work", "--", "--model", "opus"]).unwrap();
        assert!(matches!(
            opencode.command,
            Some(Command::Opencode(LaunchArgs { profile, args }))
                if profile.as_deref() == Some("work")
                    && args == [OsString::from("--model"), OsString::from("opus")]
        ));
    }

    fn indicator_args(arguments: &[&str]) -> IndicatorArgs {
        match Cli::try_parse_from(arguments).unwrap().command {
            Some(Command::Indicator(arguments)) => arguments,
            other => panic!("expected an indicator command, got {other:?}"),
        }
    }

    #[test]
    fn parses_indicator_commands() {
        // A profile alone reports rather than changing anything, so the status
        // line can be checked without touching it.
        let reporting = indicator_args(&["ditto-cli", "indicator", "work"]);
        assert_eq!(reporting.profile.as_deref(), Some("work"));
        assert_eq!(reporting.action(), None);

        assert_eq!(indicator_args(&["ditto-cli", "indicator"]).action(), None);

        let turning_on = indicator_args(&["ditto-cli", "indicator", "work", "--on"]);
        assert_eq!(turning_on.profile.as_deref(), Some("work"));
        assert_eq!(turning_on.action(), Some(IndicatorAction::On));

        let turning_off = indicator_args(&["ditto-cli", "indicator", "--off"]);
        assert_eq!(turning_off.profile, None);
        assert_eq!(turning_off.action(), Some(IndicatorAction::Off));

        // Asking for both at once has no sensible answer.
        assert!(Cli::try_parse_from(["ditto-cli", "indicator", "--on", "--off"]).is_err());
    }

    fn workspace_command(arguments: &[&str]) -> Option<WorkspaceCommand> {
        match Cli::try_parse_from(arguments).unwrap().command {
            Some(Command::Workspace(arguments)) => arguments.command,
            other => panic!("expected a workspace command, got {other:?}"),
        }
    }

    #[test]
    fn parses_workspace_commands() {
        // A bare `workspace` reports the binding rather than changing it.
        assert!(workspace_command(&["ditto-cli", "workspace"]).is_none());

        assert!(matches!(
            workspace_command(&["ditto-cli", "workspace", "use", "work"]),
            Some(WorkspaceCommand::Use { profile, global, path })
                if profile == "work" && !global && path.is_none()
        ));

        assert!(matches!(
            workspace_command(&["ditto-cli", "workspace", "use", "work", "--global"]),
            Some(WorkspaceCommand::Use { global, .. }) if global
        ));

        assert!(matches!(
            workspace_command(&["ditto-cli", "workspace", "clear", "--path", "/tmp/project"]),
            Some(WorkspaceCommand::Clear { path })
                if path.as_deref() == Some(std::path::Path::new("/tmp/project"))
        ));

        assert!(matches!(
            workspace_command(&["ditto-cli", "workspace", "list"]),
            Some(WorkspaceCommand::List)
        ));

        // Binding has no fallback profile to reach for, so the name is required.
        assert!(Cli::try_parse_from(["ditto-cli", "workspace", "use"]).is_err());
    }

    #[test]
    fn parses_the_auto_bind_setting() {
        // No state reports rather than changing, matching how `indicator` and
        // the rest of the reporting commands read their own arguments.
        assert!(matches!(
            workspace_command(&["ditto-cli", "workspace", "auto"]),
            Some(WorkspaceCommand::Auto { state: None })
        ));

        for (argument, expected) in [("on", AutoState::On), ("off", AutoState::Off)] {
            assert!(matches!(
                workspace_command(&["ditto-cli", "workspace", "auto", argument]),
                Some(WorkspaceCommand::Auto { state: Some(state) }) if state == expected
            ));
        }

        assert!(AutoState::On.enabled());
        assert!(!AutoState::Off.enabled());
        assert!(Cli::try_parse_from(["ditto-cli", "workspace", "auto", "maybe"]).is_err());
    }

    #[test]
    fn parses_the_status_line_command_claude_code_runs() {
        let cli = Cli::try_parse_from(["ditto-cli", "statusline"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Statusline)));
    }

    #[test]
    fn parses_omp_launch_arguments() {
        let cli =
            Cli::try_parse_from(["ditto-cli", "omp", "work", "--", "--model", "opus"]).unwrap();

        assert!(matches!(
            cli.command,
            Some(Command::Omp(LaunchArgs { profile, args }))
                if profile.as_deref() == Some("work")
                    && args == [OsString::from("--model"), OsString::from("opus")]
        ));
    }
}
