use std::{
    ffi::OsString,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;

use crate::{indicator, profile::Profile};

/// OMP's per-profile store. It holds credentials alongside sessions and
/// settings, so it lives inside the profile's agent directory.
const OMP_DATABASE: &str = "agent.db";

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Tool {
    Claude,
    Codex,
    Opencode,
    Omp,
}

impl Tool {
    /// Every tool Ditto CLI can launch, in the order they are presented.
    pub const ALL: [Self; 4] = [Self::Claude, Self::Codex, Self::Opencode, Self::Omp];

    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
            Self::Opencode => "opencode",
            Self::Omp => "OMP",
        }
    }

    fn executable(self) -> OsString {
        let override_variable = match self {
            Self::Claude => "DITTO_CLAUDE_BIN",
            Self::Codex => "DITTO_CODEX_BIN",
            Self::Opencode => "DITTO_OPENCODE_BIN",
            Self::Omp => "DITTO_OMP_BIN",
        };
        std::env::var_os(override_variable).unwrap_or_else(|| match self {
            Self::Claude => OsString::from("claude"),
            Self::Codex => OsString::from("codex"),
            Self::Opencode => OsString::from("opencode"),
            Self::Omp => OsString::from("omp"),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthOperation {
    Login,
    Logout,
}

impl AuthOperation {
    pub fn label(self) -> &'static str {
        match self {
            Self::Login => "Sign in",
            Self::Logout => "Sign out",
        }
    }

    fn args(self, tool: Tool) -> Option<&'static [&'static str]> {
        match (self, tool) {
            (Self::Login, Tool::Claude) => Some(&["auth", "login"]),
            (Self::Login, Tool::Codex) => Some(&["login"]),
            (Self::Login, Tool::Opencode) => Some(&["auth", "login"]),
            (Self::Logout, Tool::Claude) => Some(&["auth", "logout"]),
            (Self::Logout, Tool::Codex) => Some(&["logout"]),
            (Self::Logout, Tool::Opencode) => Some(&["auth", "logout"]),
            // OMP authenticates from its own prompt and exposes no command for
            // it. Its sign-in state is still readable, so it reports status
            // without being signable in or out here.
            (_, Tool::Omp) => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthStatus {
    SignedIn,
    SignedOut,
    Unavailable,
}

#[derive(Deserialize)]
struct ClaudeAuthStatus {
    #[serde(rename = "loggedIn")]
    logged_in: bool,
}

pub fn build_command(tool: Tool, profile: &Profile, args: &[OsString]) -> Command {
    let mut command = base_command(tool, profile);
    command.args(args);
    command
}

pub fn auth_status(tool: Tool, profile: &Profile) -> AuthStatus {
    let status_args: &[&str] = match tool {
        Tool::Claude => &["auth", "status", "--json"],
        Tool::Codex => &["login", "status"],
        Tool::Opencode => &["auth", "list"],
        // OMP has no status command to ask. `/login` writes credentials into
        // the profile's own database, so Ditto reads that instead.
        Tool::Omp => return omp_auth_status(profile),
    };

    let output = base_command(tool, profile)
        .args(status_args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output();
    let Ok(output) = output else {
        return AuthStatus::Unavailable;
    };

    match tool {
        Tool::Claude => parse_claude_auth_status(&output.stdout),
        Tool::Codex => {
            parse_codex_auth_status(output.status.success(), &output.stdout, &output.stderr)
        }
        Tool::Opencode => parse_opencode_auth_status(output.status.success(), &output.stdout),
        Tool::Omp => unreachable!("OMP auth status returned before command execution"),
    }
}

/// A profile with no database has never launched OMP, which is the same thing
/// as holding no credentials. Anything else that goes wrong is reported as
/// unavailable rather than guessed at: OMP owns this schema, and a future
/// release is free to change it.
fn omp_auth_status(profile: &Profile) -> AuthStatus {
    let database = profile.omp_home.join(OMP_DATABASE);
    if !database.exists() {
        return AuthStatus::SignedOut;
    }
    match omp_credential_count(&database) {
        Ok(0) => AuthStatus::SignedOut,
        Ok(_) => AuthStatus::SignedIn,
        Err(_) => AuthStatus::Unavailable,
    }
}

fn omp_credential_count(database: &Path) -> rusqlite::Result<i64> {
    let connection = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    // OMP writes with WAL enabled, so this read can land while OMP itself is
    // running. A short wait rides out that overlap; a long one would stall the
    // interface, which probes every profile it shows.
    connection.busy_timeout(Duration::from_millis(250))?;
    connection.query_row(
        "SELECT count(*) FROM auth_credentials WHERE disabled_cause IS NULL",
        [],
        |row| row.get(0),
    )
}

fn parse_claude_auth_status(stdout: &[u8]) -> AuthStatus {
    serde_json::from_slice::<ClaudeAuthStatus>(stdout)
        .map(|status| {
            if status.logged_in {
                AuthStatus::SignedIn
            } else {
                AuthStatus::SignedOut
            }
        })
        .unwrap_or(AuthStatus::Unavailable)
}

fn parse_codex_auth_status(success: bool, stdout: &[u8], stderr: &[u8]) -> AuthStatus {
    if success {
        return AuthStatus::SignedIn;
    }

    let stdout = String::from_utf8_lossy(stdout);
    let stderr = String::from_utf8_lossy(stderr);
    if stdout.trim() == "Not logged in" || stderr.trim() == "Not logged in" {
        AuthStatus::SignedOut
    } else {
        AuthStatus::Unavailable
    }
}

/// `opencode auth list` exits zero whether or not credentials exist, so the
/// stored credential count in its summary line is what separates a signed-in
/// profile from an empty one.
fn parse_opencode_auth_status(success: bool, stdout: &[u8]) -> AuthStatus {
    if !success {
        return AuthStatus::Unavailable;
    }

    let plain = strip_ansi(&String::from_utf8_lossy(stdout));
    match credential_count(&plain) {
        Some(0) => AuthStatus::SignedOut,
        Some(_) => AuthStatus::SignedIn,
        None => AuthStatus::Unavailable,
    }
}

/// Reads the count out of a trailing `3 credentials` summary. The last match
/// wins because the header names the auth file, not a total.
fn credential_count(text: &str) -> Option<u64> {
    let words = text.split_whitespace().collect::<Vec<_>>();
    words
        .windows(2)
        .rev()
        .find(|pair| pair[1].starts_with("credential"))
        .and_then(|pair| pair[0].parse().ok())
}

/// opencode decorates its output, so the escape sequences come off before the
/// text is read.
fn strip_ansi(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        if character != '\u{1b}' {
            plain.push(character);
            continue;
        }
        if characters.next() == Some('[') {
            // Parameter and intermediate bytes run until the final byte.
            characters
                .by_ref()
                .find(|escaped| matches!(escaped, '\u{40}'..='\u{7e}'));
        }
    }
    plain
}

pub fn authenticate(operation: AuthOperation, tool: Tool, profile: &Profile) -> Result<()> {
    let Some(args) = operation.args(tool) else {
        bail!("OMP authentication is managed inside OMP with `/login` and `/logout`");
    };
    let status = base_command(tool, profile)
        .args(args)
        .status()
        .with_context(|| format!("could not run {} authentication", tool.label()))?;

    if !status.success() {
        bail!(
            "{} for {} failed with {status}",
            operation.label(),
            tool.label()
        );
    }
    Ok(())
}

fn base_command(tool: Tool, profile: &Profile) -> Command {
    let mut command = Command::new(tool.executable());
    command.env("DITTO_PROFILE", &profile.name);
    match tool {
        Tool::Claude => {
            command.env("CLAUDE_CONFIG_DIR", &profile.claude_home);
        }
        Tool::Codex => {
            command.env("CODEX_HOME", &profile.codex_home);
        }
        Tool::Opencode => {
            command
                .env("XDG_DATA_HOME", &profile.opencode.data)
                .env("XDG_CONFIG_HOME", &profile.opencode.config)
                .env("XDG_STATE_HOME", &profile.opencode.state);
        }
        Tool::Omp => {
            command.env_remove("OMP_PROFILE").env_remove("PI_PROFILE");
            if profile.managed {
                command
                    .arg("--profile")
                    .arg(&profile.name)
                    .env("OMP_PROFILE", &profile.name);
            }
        }
    }
    command
}

/// Set to step out of the way and hand the terminal straight to the tool. The
/// title stops naming the profile, which is the price of a way out if running
/// underneath Ditto ever causes trouble.
const NO_PROXY_VARIABLE: &str = "DITTO_NO_PROXY";

/// Puts the profile in front of the user before the tool takes the terminal.
/// Claude Code gets a status line inside its own interface; the rest get the
/// window title.
#[cfg(not(unix))]
fn show_profile(tool: Tool, profile: &Profile) {
    if tool == Tool::Claude {
        indicator::enable_quietly(profile);
    }
    indicator::announce(tool, profile);
}

/// Whether the tool should run underneath Ditto rather than replace it.
/// Rewriting the title only means anything on a real terminal, so redirected
/// output is handed over untouched.
#[cfg(unix)]
fn proxy_wanted() -> bool {
    use std::io::IsTerminal;

    std::env::var_os(NO_PROXY_VARIABLE).is_none()
        && std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
}

#[cfg(unix)]
pub fn launch(tool: Tool, profile: &Profile, args: &[OsString]) -> Result<()> {
    use std::os::unix::process::CommandExt;

    if tool == Tool::Claude {
        indicator::enable_quietly(profile);
    }

    if proxy_wanted() {
        // A pseudoterminal that cannot be opened is no reason to refuse to
        // launch. Handing the terminal over directly costs the profile in the
        // title and nothing else.
        match crate::proxy::run(tool, profile, args) {
            Ok(status) => std::process::exit(crate::proxy::exit_code(status)),
            Err(error) => {
                eprintln!("ditto-cli: {error:#}");
                eprintln!("ditto-cli: launching {} directly", tool.label());
            }
        }
    }

    indicator::announce(tool, profile);
    let error = build_command(tool, profile, args).exec();
    Err(error).with_context(|| format!("could not launch {}", tool.label()))
}

#[cfg(not(unix))]
pub fn launch(tool: Tool, profile: &Profile, args: &[OsString]) -> Result<()> {
    show_profile(tool, profile);
    let status = build_command(tool, profile, args)
        .status()
        .with_context(|| format!("could not launch {}", tool.label()))?;

    if status.success() {
        Ok(())
    } else {
        bail!("{} exited with {status}", tool.label())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    use crate::profile::OpencodeHome;

    fn profile() -> Profile {
        Profile {
            name: "work".to_owned(),
            claude_home: PathBuf::from("/profiles/work/claude"),
            codex_home: PathBuf::from("/profiles/work/codex"),
            omp_home: PathBuf::from("/omp/profiles/work/agent"),
            opencode: OpencodeHome {
                data: PathBuf::from("/profiles/work/opencode/data"),
                config: PathBuf::from("/profiles/work/opencode/config"),
                state: PathBuf::from("/profiles/work/opencode/state"),
            },
            managed: true,
        }
    }

    #[test]
    fn claude_uses_the_selected_config_directory() {
        let command = build_command(Tool::Claude, &profile(), &[]);
        let configured_home = command
            .get_envs()
            .find(|(name, _)| *name == "CLAUDE_CONFIG_DIR")
            .and_then(|(_, value)| value);

        assert_eq!(
            configured_home,
            Some(std::ffi::OsStr::new("/profiles/work/claude"))
        );
    }

    #[test]
    fn codex_uses_the_selected_home() {
        let command = build_command(Tool::Codex, &profile(), &[]);
        let configured_home = command
            .get_envs()
            .find(|(name, _)| *name == "CODEX_HOME")
            .and_then(|(_, value)| value);

        assert_eq!(
            configured_home,
            Some(std::ffi::OsStr::new("/profiles/work/codex"))
        );
    }
    #[test]
    fn opencode_pins_every_xdg_base_that_holds_account_state() {
        let command = build_command(Tool::Opencode, &profile(), &[]);
        let environment = command.get_envs().collect::<Vec<_>>();

        for (variable, expected) in [
            ("XDG_DATA_HOME", "/profiles/work/opencode/data"),
            ("XDG_CONFIG_HOME", "/profiles/work/opencode/config"),
            ("XDG_STATE_HOME", "/profiles/work/opencode/state"),
        ] {
            assert!(
                environment.contains(&(
                    std::ffi::OsStr::new(variable),
                    Some(std::ffi::OsStr::new(expected))
                )),
                "{variable} was not pinned to {expected}"
            );
        }

        // The cache stays shared: it holds downloaded tooling, not credentials.
        assert!(
            !environment
                .iter()
                .any(|(name, _)| *name == std::ffi::OsStr::new("XDG_CACHE_HOME"))
        );
    }

    #[test]
    fn omp_uses_native_named_profile_and_exports_selection() {
        let command = build_command(
            Tool::Omp,
            &profile(),
            &[OsString::from("--model"), OsString::from("opus")],
        );
        let arguments = command.get_args().collect::<Vec<_>>();
        let environment = command.get_envs().collect::<Vec<_>>();

        assert_eq!(
            arguments,
            ["--profile", "work", "--model", "opus"].map(std::ffi::OsStr::new)
        );
        assert!(environment.contains(&(
            std::ffi::OsStr::new("DITTO_PROFILE"),
            Some(std::ffi::OsStr::new("work"))
        )));
        assert!(environment.contains(&(
            std::ffi::OsStr::new("OMP_PROFILE"),
            Some(std::ffi::OsStr::new("work"))
        )));
    }

    #[test]
    fn omp_default_profile_ignores_inherited_profile_selection() {
        let mut default_profile = profile();
        default_profile.name = "default".to_owned();
        default_profile.managed = false;
        let command = build_command(Tool::Omp, &default_profile, &[]);
        let omp_profile = command
            .get_envs()
            .find(|(name, _)| *name == "OMP_PROFILE")
            .map(|(_, value)| value);

        assert_eq!(command.get_args().next(), None);
        assert_eq!(omp_profile, Some(None));
    }

    #[test]
    fn authentication_uses_native_cli_commands() {
        assert_eq!(
            AuthOperation::Login.args(Tool::Claude),
            Some(["auth", "login"].as_slice())
        );
        assert_eq!(
            AuthOperation::Login.args(Tool::Codex),
            Some(["login"].as_slice())
        );
        assert_eq!(
            AuthOperation::Logout.args(Tool::Claude),
            Some(["auth", "logout"].as_slice())
        );
        assert_eq!(
            AuthOperation::Logout.args(Tool::Codex),
            Some(["logout"].as_slice())
        );
        assert_eq!(
            AuthOperation::Login.args(Tool::Opencode),
            Some(["auth", "login"].as_slice())
        );
        assert_eq!(
            AuthOperation::Logout.args(Tool::Opencode),
            Some(["auth", "logout"].as_slice())
        );
        assert_eq!(AuthOperation::Login.args(Tool::Omp), None);
        assert_eq!(AuthOperation::Logout.args(Tool::Omp), None);
    }

    #[test]
    fn parses_decorated_opencode_credential_summary() {
        let signed_in = "\u{1b}[0m\n┌  Credentials \u{1b}[90m~/.local/share/opencode/auth.json\n\
             │\n●  Anthropic \u{1b}[90moauth\n│\n└  3 credentials\n";
        let signed_out = "\u{1b}[0m\n┌  Credentials \u{1b}[90m/tmp/p/opencode/auth.json\n│\n\
             └  0 credentials\n";

        assert_eq!(
            parse_opencode_auth_status(true, signed_in.as_bytes()),
            AuthStatus::SignedIn
        );
        assert_eq!(
            parse_opencode_auth_status(true, signed_out.as_bytes()),
            AuthStatus::SignedOut
        );
        // A lone credential is reported in the singular.
        assert_eq!(
            parse_opencode_auth_status(true, b"\xe2\x94\x94  1 credential\n"),
            AuthStatus::SignedIn
        );
        assert_eq!(
            parse_opencode_auth_status(false, signed_in.as_bytes()),
            AuthStatus::Unavailable
        );
        assert_eq!(
            parse_opencode_auth_status(true, b"command not found\n"),
            AuthStatus::Unavailable
        );
    }

    #[test]
    fn strips_ansi_without_eating_surrounding_text() {
        assert_eq!(strip_ansi("\u{1b}[90mdim\u{1b}[0m text"), "dim text");
        assert_eq!(strip_ansi("plain"), "plain");
        // An unterminated sequence must not swallow the rest of the buffer.
        assert_eq!(strip_ansi("a\u{1b}Xb"), "ab");
    }

    #[test]
    fn parses_native_auth_status_output() {
        assert_eq!(
            parse_claude_auth_status(br#"{"loggedIn":true}"#),
            AuthStatus::SignedIn
        );
        assert_eq!(
            parse_claude_auth_status(br#"{"loggedIn":false}"#),
            AuthStatus::SignedOut
        );
        assert_eq!(
            parse_codex_auth_status(false, b"", b"Not logged in\n"),
            AuthStatus::SignedOut
        );
        assert_eq!(
            parse_codex_auth_status(true, b"Logged in using ChatGPT\n", b""),
            AuthStatus::SignedIn
        );
        assert_eq!(
            parse_codex_auth_status(false, b"", b"configuration error\n"),
            AuthStatus::Unavailable
        );
    }
}
