use std::{
    ffi::OsString,
    fs, io,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

use anyhow::{Result, anyhow, bail};
use rusqlite::{Connection, OpenFlags};
use serde::Deserialize;

use crate::{indicator, profile::Profile, program};

/// OMP's per-profile store. It holds credentials alongside sessions and
/// settings, so it lives inside the profile's agent directory.
const OMP_DATABASE: &str = "agent.db";

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Tool {
    Claude,
    Codex,
    Opencode,
    Omp,
    PrimeAgent,
}

impl Tool {
    /// Every tool Ditto CLI can launch, in the order they are presented.
    pub const ALL: [Self; 5] = [
        Self::Claude,
        Self::Codex,
        Self::Opencode,
        Self::Omp,
        Self::PrimeAgent,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
            Self::Opencode => "opencode",
            Self::Omp => "OMP",
            Self::PrimeAgent => "Prime Agent",
        }
    }

    /// The name this tool answers to in JSON output and on the command line.
    ///
    /// Kept apart from [`Self::label`], which is written for a person and is
    /// free to change with the interface. Anything parsing Ditto's output
    /// depends on these, so they are the subcommand names and change only with
    /// a deliberate break.
    pub fn key(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Opencode => "opencode",
            Self::Omp => "omp",
            Self::PrimeAgent => "prime-agent",
        }
    }

    fn executable(self) -> OsString {
        let override_variable = match self {
            Self::Claude => "DITTO_CLAUDE_BIN",
            Self::Codex => "DITTO_CODEX_BIN",
            Self::Opencode => "DITTO_OPENCODE_BIN",
            Self::Omp => "DITTO_OMP_BIN",
            Self::PrimeAgent => "DITTO_PRIME_AGENT_BIN",
        };
        std::env::var_os(override_variable).unwrap_or_else(|| match self {
            Self::Claude => OsString::from("claude"),
            Self::Codex => OsString::from("codex"),
            Self::Opencode => OsString::from("opencode"),
            Self::Omp => OsString::from("omp"),
            Self::PrimeAgent => OsString::from("prime-agent"),
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
            // Prime Agent treats an initial slash command exactly as if it was
            // typed into the editor, so this opens the login dialog directly.
            (Self::Login, Tool::PrimeAgent) => Some(&["/login"]),
            (Self::Logout, Tool::Claude) => Some(&["auth", "logout"]),
            (Self::Logout, Tool::Codex) => Some(&["logout"]),
            (Self::Logout, Tool::Opencode) => Some(&["auth", "logout"]),
            (Self::Logout, Tool::PrimeAgent) => Some(&["/logout"]),
            // OMP exposes no authentication command. Its sign-in state is
            // still readable, so it reports status without being signable in
            // or out here.
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

impl AuthStatus {
    /// The stable name for this state in JSON output. See [`Tool::key`].
    pub fn key(self) -> &'static str {
        match self {
            Self::SignedIn => "signed_in",
            Self::SignedOut => "signed_out",
            Self::Unavailable => "unavailable",
        }
    }
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
        // These tools have no status command to ask. Ditto reads the stores
        // their in-app login flows write instead.
        Tool::Omp => return omp_auth_status(profile),
        Tool::PrimeAgent => return prime_agent_auth_status(profile),
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
        Tool::Omp | Tool::PrimeAgent => {
            unreachable!("file-based auth status returned before command execution")
        }
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

/// Prime Agent keeps provider logins beside credentials for optional services.
/// MCP, trace sharing, and web search do not make a model available, so only a
/// remaining entry counts as being signed in to the agent itself.
fn prime_agent_auth_status(profile: &Profile) -> AuthStatus {
    let auth = profile.prime_agent_home.join("auth.json");
    let contents = match fs::read(&auth) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return AuthStatus::SignedOut,
        Err(_) => return AuthStatus::Unavailable,
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&contents) else {
        return AuthStatus::Unavailable;
    };
    let Some(credentials) = value.as_object() else {
        return AuthStatus::Unavailable;
    };

    if credentials.keys().any(|provider| {
        !provider.starts_with("mcp:") && provider != "prime-agent-traces" && provider != "serper"
    }) {
        AuthStatus::SignedIn
    } else {
        AuthStatus::SignedOut
    }
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
        bail!(
            "{} authentication is managed inside {} with `/login` and `/logout`",
            tool.label(),
            tool.label()
        );
    };

    // Signing in is a conversation with the tool, browser round trip and all,
    // so Ctrl-C belongs to it for as long as it lasts.
    let interrupts = Interrupts::leave_to_tool();
    let started = base_command(tool, profile).args(args).status();
    drop(interrupts);
    let status = started.map_err(|error| cannot_launch(tool, error))?;

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
    let mut command = Command::new(program::resolve(&tool.executable()));
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
        Tool::PrimeAgent => {
            command.env("PRIME_AGENT_CODING_AGENT_DIR", &profile.prime_agent_home);
            if profile.managed {
                // A session directory in the user's shared settings or shell
                // would otherwise put transcripts from every account together.
                command
                    .env(
                        "PRIME_AGENT_SESSION_DIR",
                        profile.prime_agent_home.join("sessions"),
                    )
                    .env_remove("PRIME_AGENT_CODING_AGENT_SESSION_DIR");
            }
        }
    }
    command
}

/// Set to step out of the way and hand the terminal straight to the tool. The
/// title stops naming the profile, which is the price of a way out if running
/// underneath Ditto ever causes trouble.
#[cfg(unix)]
const NO_PROXY_VARIABLE: &str = "DITTO_NO_PROXY";

/// Puts the profile in front of the user before the tool takes the terminal.
/// Claude Code gets a status line inside its own interface; the rest get the
/// window title.
#[cfg(not(unix))]
fn show_profile(tool: Tool, profile: &Profile) {
    if tool == Tool::Claude {
        indicator::enable_quietly(profile);
    }
    crate::herdr::report_profile(profile);
    indicator::announce(tool, profile);
}

/// Whether the tool should run underneath Ditto rather than replace it.
/// Rewriting the title only means anything on a real terminal, so redirected
/// output is handed over untouched.
///
/// herdr is the other way out. It reads both the foreground process and the
/// title to work out which agent is in a pane and what it is doing, and a
/// proxy costs it both answers, so under herdr the title is not Ditto's to
/// take. `herdr::report_profile` says the same thing where herdr will show it.
#[cfg(unix)]
fn proxy_wanted() -> bool {
    use std::io::IsTerminal;

    std::env::var_os(NO_PROXY_VARIABLE).is_none()
        && crate::herdr::pane().is_none()
        && std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
}

/// Says plainly when a tool is not installed. The operating system reports that
/// as a missing file, which reads as a fault in Ditto rather than as a CLI the
/// user has yet to install.
fn cannot_launch(tool: Tool, error: io::Error) -> anyhow::Error {
    if error.kind() == io::ErrorKind::NotFound {
        return anyhow!(
            "{} is not installed, or its command is not on PATH",
            tool.label()
        );
    }
    anyhow::Error::new(error).context(format!("could not launch {}", tool.label()))
}

/// Hands Ctrl-C to the tool for as long as one is running.
///
/// Unix has nothing to do here: `exec` leaves no Ditto behind, and a proxied
/// tool is given a session of its own, so the key never reaches Ditto either
/// way. Windows delivers it to every process sharing the console at once, and
/// Ditto's own default is to exit, which would hand the shell prompt back while
/// the tool carried on drawing over it. Reporting the event as handled leaves
/// the tool to decide what Ctrl-C means, which for every tool Ditto launches is
/// something other than dying.
struct Interrupts;

impl Interrupts {
    #[cfg(windows)]
    fn leave_to_tool() -> Self {
        unsafe {
            windows_sys::Win32::System::Console::SetConsoleCtrlHandler(
                Some(ignore_interrupt),
                windows_sys::Win32::Foundation::TRUE,
            );
        }
        Self
    }

    #[cfg(not(windows))]
    fn leave_to_tool() -> Self {
        Self
    }
}

impl Drop for Interrupts {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::System::Console::SetConsoleCtrlHandler(
                Some(ignore_interrupt),
                windows_sys::Win32::Foundation::FALSE,
            );
        }
    }
}

/// Answering that an event was handled is what keeps the default handler, which
/// would end Ditto, from running after this one. Only the two the user presses
/// are answered for: closing the window, logging off and shutting down are all
/// meant to end Ditto, and holding them would only delay it.
#[cfg(windows)]
unsafe extern "system" fn ignore_interrupt(event: u32) -> windows_sys::core::BOOL {
    use windows_sys::Win32::{
        Foundation::{FALSE, TRUE},
        System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT},
    };

    if matches!(event, CTRL_C_EVENT | CTRL_BREAK_EVENT) {
        TRUE
    } else {
        FALSE
    }
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

    crate::herdr::report_profile(profile);
    indicator::announce(tool, profile);
    let error = build_command(tool, profile, args).exec();
    Err(cannot_launch(tool, error))
}

/// Windows has no `exec`, so Ditto waits on the tool instead of being replaced
/// by it and then exits the way the tool did. A caller that reads the exit code
/// sees what it would have seen had it run the tool itself.
#[cfg(not(unix))]
pub fn launch(tool: Tool, profile: &Profile, args: &[OsString]) -> Result<()> {
    show_profile(tool, profile);

    let interrupts = Interrupts::leave_to_tool();
    let started = build_command(tool, profile, args).status();
    drop(interrupts);

    let status = started.map_err(|error| cannot_launch(tool, error))?;
    std::process::exit(status.code().unwrap_or(1));
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
            prime_agent_home: PathBuf::from("/profiles/work/prime-agent"),
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
    fn prime_agent_uses_isolated_config_and_session_directories() {
        let profile = profile();
        let expected_sessions = profile.prime_agent_home.join("sessions");
        let command = build_command(Tool::PrimeAgent, &profile, &[]);
        let environment = command.get_envs().collect::<Vec<_>>();

        assert!(environment.contains(&(
            std::ffi::OsStr::new("PRIME_AGENT_CODING_AGENT_DIR"),
            Some(profile.prime_agent_home.as_os_str())
        )));
        assert!(environment.contains(&(
            std::ffi::OsStr::new("PRIME_AGENT_SESSION_DIR"),
            Some(expected_sessions.as_os_str())
        )));
        assert!(environment.contains(&(
            std::ffi::OsStr::new("PRIME_AGENT_CODING_AGENT_SESSION_DIR"),
            None
        )));
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
    fn stable_tool_keys_include_prime_agent() {
        assert_eq!(
            Tool::ALL.map(Tool::key),
            ["claude", "codex", "opencode", "omp", "prime-agent"]
        );
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
        assert_eq!(
            AuthOperation::Login.args(Tool::PrimeAgent),
            Some(["/login"].as_slice())
        );
        assert_eq!(
            AuthOperation::Logout.args(Tool::PrimeAgent),
            Some(["/logout"].as_slice())
        );
    }

    #[test]
    fn reads_prime_agent_provider_credentials_without_counting_services() {
        let temporary = tempfile::tempdir().unwrap();
        let mut profile = profile();
        profile.prime_agent_home = temporary.path().join("prime-agent");
        fs::create_dir_all(&profile.prime_agent_home).unwrap();

        assert_eq!(prime_agent_auth_status(&profile), AuthStatus::SignedOut);
        fs::write(profile.prime_agent_home.join("auth.json"), "{}").unwrap();
        assert_eq!(prime_agent_auth_status(&profile), AuthStatus::SignedOut);

        fs::write(
            profile.prime_agent_home.join("auth.json"),
            r#"{
                "mcp:notion": {"type": "oauth"},
                "prime-agent-traces": {"type": "api_key"},
                "serper": {"type": "api_key"}
            }"#,
        )
        .unwrap();
        assert_eq!(prime_agent_auth_status(&profile), AuthStatus::SignedOut);

        fs::write(
            profile.prime_agent_home.join("auth.json"),
            r#"{"anthropic":{"type":"oauth"}}"#,
        )
        .unwrap();
        assert_eq!(prime_agent_auth_status(&profile), AuthStatus::SignedIn);

        fs::write(profile.prime_agent_home.join("auth.json"), "not json").unwrap();
        assert_eq!(prime_agent_auth_status(&profile), AuthStatus::Unavailable);
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
