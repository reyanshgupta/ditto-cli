use std::ffi::OsString;

use clap::{Args, Parser, Subcommand};

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
    /// Show Claude Code, Codex, and opencode authentication status.
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
    /// Show the Claude, Codex, opencode, and OMP directories for a profile.
    Paths {
        /// Profile name. Uses the last selected profile when omitted.
        profile: Option<String>,
    },
    /// Launch Claude Code.
    Claude(LaunchArgs),
    /// Launch Codex.
    Codex(LaunchArgs),
    /// Launch opencode.
    Opencode(LaunchArgs),
    /// Launch Oh My Pi.
    Omp(LaunchArgs),
    /// Update Ditto CLI itself.
    Update(UpdateArgs),
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
