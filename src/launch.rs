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

use crate::{
    indicator,
    profile::{
        LAUNCHED_TOOL_VARIABLE, NATIVE_ENVIRONMENT, NATIVE_HOME_ENVIRONMENT, Profile, preserved,
    },
    program, shared,
    tools::{self, Home},
};

/// OMP's per-profile store. It holds credentials alongside sessions and
/// settings, so it lives inside the profile's agent directory.
const OMP_DATABASE: &str = "agent.db";

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Tool {
    Claude,
    Codex,
    Fx,
    Opencode,
    Omp,
    PrimeAgent,
    Pi,
    /// Any tool `tools.rs` describes. One arm per match reads the entry, so a
    /// tool added there needs nothing added here.
    Generic(&'static tools::Spec),
}

impl Tool {
    const BUILT_IN: [Self; 7] = [
        Self::Claude,
        Self::Codex,
        Self::Fx,
        Self::Opencode,
        Self::Omp,
        Self::PrimeAgent,
        Self::Pi,
    ];

    /// Every tool Ditto CLI can launch, in the order they are presented: the
    /// built-in ones first, then the table in its own order.
    pub const ALL: [Self; Self::BUILT_IN.len() + tools::ALL.len()] = Self::all();

    const fn all() -> [Self; Self::BUILT_IN.len() + tools::ALL.len()] {
        let mut all = [Self::Claude; Self::BUILT_IN.len() + tools::ALL.len()];
        let mut index = 0;
        while index < Self::BUILT_IN.len() {
            all[index] = Self::BUILT_IN[index];
            index += 1;
        }
        let mut described = 0;
        while described < tools::ALL.len() {
            all[index + described] = Self::Generic(&tools::ALL[described]);
            described += 1;
        }
        all
    }

    /// Position in [`Self::ALL`], for state kept per tool without a field per
    /// tool.
    pub fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|tool| *tool == self)
            .expect("every Tool is an entry of Tool::ALL")
    }

    /// The tool a subcommand or a shell function names, if there is one.
    pub fn by_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tool| tool.key() == key)
    }

    /// Whether the executable would be found, honouring the `DITTO_*_BIN`
    /// override the same way a launch does.
    pub fn installed(self) -> bool {
        program::installed(&self.executable())
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
            Self::Fx => "fx",
            Self::Opencode => "opencode",
            Self::Omp => "OMP",
            Self::PrimeAgent => "Prime Agent",
            Self::Pi => "Pi",
            Self::Generic(spec) => spec.label,
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
            Self::Fx => "fx",
            Self::Opencode => "opencode",
            Self::Omp => "omp",
            Self::PrimeAgent => "prime-agent",
            Self::Pi => "pi",
            Self::Generic(spec) => spec.key,
        }
    }

    fn executable(self) -> OsString {
        let override_variable = match self {
            Self::Claude => "DITTO_CLAUDE_BIN",
            Self::Codex => "DITTO_CODEX_BIN",
            Self::Fx => "DITTO_FX_BIN",
            Self::Opencode => "DITTO_OPENCODE_BIN",
            Self::Omp => "DITTO_OMP_BIN",
            Self::PrimeAgent => "DITTO_PRIME_AGENT_BIN",
            Self::Pi => "DITTO_PI_BIN",
            Self::Generic(spec) => spec.bin_variable,
        };
        std::env::var_os(override_variable).unwrap_or_else(|| match self {
            Self::Claude => OsString::from("claude"),
            Self::Codex => OsString::from("codex"),
            Self::Fx => OsString::from("fx"),
            Self::Opencode => OsString::from("opencode"),
            Self::Omp => OsString::from("omp"),
            Self::PrimeAgent => OsString::from("prime-agent"),
            Self::Pi => OsString::from("pi"),
            Self::Generic(spec) => OsString::from(spec.executable),
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
            (Self::Login, Tool::Fx) => Some(&["login"]),
            (Self::Login, Tool::Opencode) => Some(&["auth", "login"]),
            // Prime Agent treats an initial slash command exactly as if it was
            // typed into the editor, so this opens the login dialog directly.
            (Self::Login, Tool::PrimeAgent) => Some(&["/login"]),
            (Self::Logout, Tool::Claude) => Some(&["auth", "logout"]),
            (Self::Logout, Tool::Codex) => Some(&["logout"]),
            (Self::Logout, Tool::Fx) => Some(&["logout"]),
            (Self::Logout, Tool::Opencode) => Some(&["auth", "logout"]),
            (Self::Logout, Tool::PrimeAgent) => Some(&["/logout"]),
            // OMP and Pi expose authentication only inside their interfaces.
            // Their sign-in state is still readable, so they report status
            // without being signable in or out here.
            (_, Tool::Omp | Tool::Pi) => None,
            (Self::Login, Tool::Generic(spec)) => spec.login,
            (Self::Logout, Tool::Generic(spec)) => spec.logout,
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

#[derive(Deserialize)]
struct FxAuthStatus {
    auth: String,
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
        Tool::Fx => &["status", "--json"],
        Tool::Opencode => &["auth", "list"],
        // These tools have no status command to ask. Ditto reads the stores
        // their in-app login flows write instead.
        Tool::Omp => return omp_auth_status(profile),
        Tool::PrimeAgent => return prime_agent_auth_status(profile),
        Tool::Pi => return pi_auth_status(profile),
        Tool::Generic(spec) => return generic_auth_status(tool, spec, profile),
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
        Tool::Fx => parse_fx_auth_status(output.status.success(), &output.stdout),
        Tool::Opencode => parse_opencode_auth_status(output.status.success(), &output.stdout),
        Tool::Omp | Tool::PrimeAgent | Tool::Pi | Tool::Generic(_) => {
            unreachable!("file-based auth status returned before command execution")
        }
    }
}

/// A profile with no database has never launched OMP, which is the same thing
/// as holding no credentials. Anything else that goes wrong is reported as
/// unavailable rather than guessed at: OMP owns this schema, and a future
/// release is free to change it.
/// Read from files rather than asked, since nothing in the table promises a
/// status command. A tool that is not installed is unavailable rather than
/// signed out, so a profile is not told to sign in to thirty tools it does not
/// have; so is one whose login lives somewhere Ditto has no file to read.
fn generic_auth_status(tool: Tool, spec: &'static tools::Spec, profile: &Profile) -> AuthStatus {
    if !tool.installed() || spec.credentials.is_empty() {
        return AuthStatus::Unavailable;
    }
    if spec
        .credentials
        .iter()
        .any(|name| profile.tool_path(spec, name).exists())
    {
        AuthStatus::SignedIn
    } else {
        AuthStatus::SignedOut
    }
}

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
    json_auth_status(&profile.prime_agent_home.join("auth.json"), |provider| {
        !provider.starts_with("mcp:") && provider != "prime-agent-traces" && provider != "serper"
    })
}

/// Pi's auth file contains only model-provider credentials, so any entry makes
/// a model available. Environment credentials stay ambient and are warned
/// about separately rather than attributed to every profile.
fn pi_auth_status(profile: &Profile) -> AuthStatus {
    json_auth_status(&profile.pi_home.join("auth.json"), |_| true)
}

fn json_auth_status(auth: &Path, counts: impl Fn(&str) -> bool) -> AuthStatus {
    let contents = match fs::read(auth) {
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

    if credentials.keys().any(|provider| counts(provider)) {
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

fn parse_fx_auth_status(success: bool, stdout: &[u8]) -> AuthStatus {
    if !success {
        return AuthStatus::Unavailable;
    }
    match serde_json::from_slice::<FxAuthStatus>(stdout) {
        Ok(status) if status.auth == "missing" => AuthStatus::SignedOut,
        Ok(_) => AuthStatus::SignedIn,
        Err(_) => AuthStatus::Unavailable,
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
    preserve_native_environment(&mut command);
    command
        .env("DITTO_PROFILE", &profile.name)
        .env(LAUNCHED_TOOL_VARIABLE, tool.key());
    match tool {
        Tool::Claude => {
            command.env("CLAUDE_CONFIG_DIR", &profile.claude_home);
        }
        Tool::Codex => {
            command.env("CODEX_HOME", &profile.codex_home);
        }
        Tool::Fx => {
            if profile.managed {
                // fx has no state-root override and resolves every private path
                // below `$HOME/.fx`. The private home isolates those files. On
                // macOS its Keychain entries are user-wide, so managed profiles
                // deliberately select the profile-file backend instead.
                command
                    .env("HOME", &profile.fx_home)
                    .env("FX_DISABLE_KEYCHAIN", "1");
            }
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
        Tool::Pi => {
            command.env("PI_CODING_AGENT_DIR", &profile.pi_home);
            if profile.managed {
                // This outranks a session directory in the shared settings, so
                // changing accounts cannot leave their transcripts together.
                command.env(
                    "PI_CODING_AGENT_SESSION_DIR",
                    profile.pi_home.join("sessions"),
                );
            }
        }
        Tool::Generic(spec) => {
            match spec.home {
                Home::Variable { variable, .. } | Home::Parent { variable, .. } => {
                    command.env(variable, profile.tool_home(spec));
                }
                Home::Xdg { .. } => {
                    command
                        .env("XDG_DATA_HOME", profile.xdg_base(spec, "data"))
                        .env("XDG_CONFIG_HOME", profile.xdg_base(spec, "config"))
                        .env("XDG_STATE_HOME", profile.xdg_base(spec, "state"));
                }
                Home::Private { .. } => {
                    if profile.managed {
                        command.env("HOME", profile.tool_home(spec));
                    }
                }
            }
            if profile.managed {
                command.envs(spec.managed_env.iter().copied());
            }
        }
    }
    command
}

/// Carries the roots from before Ditto redirected any tool into every child.
/// A tool can invoke Ditto again through a shell command, and without these
/// copies that nested process would mistake the active account's isolated
/// directories for the user's own configuration.
fn preserve_native_environment(command: &mut Command) {
    // An existing profile marker means an outer Ditto already had the only
    // unmodified view. Its saved values are inherited automatically; filling a
    // missing one now would preserve a directory the outer process redirected.
    if std::env::var_os("DITTO_PROFILE").is_some() {
        return;
    }
    for (variable, preserved) in NATIVE_ENVIRONMENT {
        if std::env::var_os(preserved).is_none()
            && let Some(value) = std::env::var_os(variable)
        {
            command.env(preserved, value);
        }
    }
    for spec in tools::ALL {
        if let Home::Variable { variable, .. } | Home::Parent { variable, .. } = spec.home
            && std::env::var_os(preserved(variable)).is_none()
            && let Some(value) = std::env::var_os(variable)
        {
            command.env(preserved(variable), value);
        }
    }
    if std::env::var_os(NATIVE_HOME_ENVIRONMENT.1).is_none()
        && let Some(value) = std::env::var_os(NATIVE_HOME_ENVIRONMENT.0)
    {
        command.env(NATIVE_HOME_ENVIRONMENT.1, value);
    }
}

/// Set to step out of the way and hand the terminal straight to the tool. The
/// title stops naming the profile, which is the price of a way out if running
/// underneath Ditto ever causes trouble.
#[cfg(unix)]
const NO_PROXY_VARIABLE: &str = "DITTO_NO_PROXY";

/// Set by Orca in every terminal it opens, and read for the reason
/// `HERDR_PANE_ID` is. Orca works out which agent a pane is running from the
/// pane's foreground process and from the title the agent writes, with its
/// Claude Code rules anchored to the first character, and it holds a prompt
/// back until the foreground process is the agent it launched. A proxy fails
/// all three at once. Unlike herdr, Orca has no command that labels an
/// existing pane, so there is nowhere to report the profile instead; Claude
/// Code's status line still names it.
#[cfg(unix)]
const ORCA_PANE_VARIABLE: &str = "ORCA_PANE_KEY";

/// A variable set to nothing is not an Orca terminal. Orca never sets it
/// empty, but a shell that exported it around a launch will.
#[cfg(unix)]
fn inside_orca() -> bool {
    std::env::var_os(ORCA_PANE_VARIABLE).is_some_and(|pane| !pane.is_empty())
}

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
/// herdr and Orca are the other ways out. Both read the foreground process and
/// the title to work out which agent is in a pane and what it is doing, and a
/// proxy costs them both answers, so under either the title is not Ditto's to
/// take. `herdr::report_profile` says the same thing where herdr will show it;
/// Orca has nowhere to say it.
#[cfg(unix)]
fn proxy_wanted() -> bool {
    use std::io::IsTerminal;

    std::env::var_os(NO_PROXY_VARIABLE).is_none()
        && crate::herdr::pane().is_none()
        && !inside_orca()
        && std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
}

/// Mends the links an installer wrote through the ones Ditto shares with, and
/// says so.
///
/// A launch is the moment for it: every tool reads its skills and extensions
/// when it starts, so a skill installed during the last session is repaired
/// before the first read that would have missed it. Saying so on stderr is the
/// difference between a skill appearing and Ditto rewriting the user's links
/// behind their back. See [`shared::repair`] for what goes wrong without it.
fn repair_shared_links(tool: Tool, profile: &Profile) {
    let repaired = shared::repair_for(tool, profile);
    for link in &repaired.links {
        eprintln!("ditto-cli: repaired {link}; it was installed pointing at nothing");
    }
    for (link, reason) in &repaired.failed {
        eprintln!("ditto-cli: could not repair {link}: {reason}");
    }
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

    repair_shared_links(tool, profile);
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
    repair_shared_links(tool, profile);
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
            fx_home: PathBuf::from("/profiles/work/fx-home"),
            omp_home: PathBuf::from("/omp/profiles/work/agent"),
            opencode: OpencodeHome {
                data: PathBuf::from("/profiles/work/opencode/data"),
                config: PathBuf::from("/profiles/work/opencode/config"),
                state: PathBuf::from("/profiles/work/opencode/state"),
            },
            pi_home: PathBuf::from("/profiles/work/pi"),
            prime_agent_home: PathBuf::from("/profiles/work/prime-agent"),
            generic: Vec::new(),
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
    fn fx_uses_a_private_home_and_profile_file_credentials() {
        let profile = profile();
        let command = build_command(Tool::Fx, &profile, &[]);
        let environment = command.get_envs().collect::<Vec<_>>();

        assert!(environment.contains(&(
            std::ffi::OsStr::new("HOME"),
            Some(profile.fx_home.as_os_str())
        )));
        assert!(environment.contains(&(
            std::ffi::OsStr::new("FX_DISABLE_KEYCHAIN"),
            Some(std::ffi::OsStr::new("1"))
        )));
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
    fn pi_uses_isolated_config_and_session_directories() {
        let profile = profile();
        let expected_sessions = profile.pi_home.join("sessions");
        let command = build_command(Tool::Pi, &profile, &[]);
        let environment = command.get_envs().collect::<Vec<_>>();

        assert!(environment.contains(&(
            std::ffi::OsStr::new("PI_CODING_AGENT_DIR"),
            Some(profile.pi_home.as_os_str())
        )));
        assert!(environment.contains(&(
            std::ffi::OsStr::new("PI_CODING_AGENT_SESSION_DIR"),
            Some(expected_sessions.as_os_str())
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
    fn launched_tools_mark_the_environment_as_redirected() {
        let command = build_command(Tool::Pi, &profile(), &[]);
        let launched_tool = command
            .get_envs()
            .find(|(name, _)| *name == LAUNCHED_TOOL_VARIABLE)
            .and_then(|(_, value)| value);

        assert_eq!(launched_tool, Some(std::ffi::OsStr::new("pi")));
    }

    #[test]
    fn stable_tool_keys_include_every_agent() {
        assert_eq!(
            Tool::BUILT_IN.map(Tool::key),
            [
                "claude",
                "codex",
                "fx",
                "opencode",
                "omp",
                "prime-agent",
                "pi"
            ]
        );
        // The table follows in its own order, so a status report and the
        // picker list tools the same way the table does.
        assert_eq!(
            Tool::ALL[Tool::BUILT_IN.len()..]
                .iter()
                .map(|tool| tool.key())
                .collect::<Vec<_>>(),
            tools::ALL.iter().map(|spec| spec.key).collect::<Vec<_>>()
        );
    }

    /// Every entry of the table is pointed at the profile the way its own
    /// `home` says, and nothing it is pointed at lies outside the profile.
    #[test]
    fn table_tools_are_pointed_at_the_profile_their_entry_describes() -> anyhow::Result<()> {
        use std::collections::HashMap;

        use crate::profile::Store;

        let dir = tempfile::tempdir()?;
        let store = Store::new(dir.path().join(".ditto"), dir.path().to_path_buf());
        let profile = store.create_profile("work")?;
        for tool in Tool::ALL {
            let Tool::Generic(spec) = tool else { continue };
            let command = build_command(tool, &profile, &[]);
            let environment: HashMap<_, _> = command
                .get_envs()
                .filter_map(|(name, value)| value.map(|value| (name.to_owned(), value.to_owned())))
                .collect();
            let pointed_at = |name: &str| {
                environment
                    .get(std::ffi::OsStr::new(name))
                    .map(PathBuf::from)
            };
            match spec.home {
                Home::Variable { variable, .. } | Home::Parent { variable, .. } => {
                    assert_eq!(
                        pointed_at(variable),
                        Some(profile.tool_home(spec).to_path_buf()),
                        "{}",
                        spec.key
                    );
                }
                Home::Xdg { .. } => {
                    assert_eq!(
                        pointed_at("XDG_CONFIG_HOME"),
                        Some(profile.xdg_base(spec, "config")),
                        "{}",
                        spec.key
                    );
                    assert_eq!(
                        pointed_at("XDG_DATA_HOME"),
                        Some(profile.xdg_base(spec, "data")),
                        "{}",
                        spec.key
                    );
                }
                Home::Private { .. } => {
                    assert_eq!(
                        pointed_at("HOME"),
                        Some(profile.tool_home(spec).to_path_buf()),
                        "{}",
                        spec.key
                    );
                }
            }
            for (name, value) in spec.managed_env {
                assert_eq!(
                    environment
                        .get(std::ffi::OsStr::new(name))
                        .map(|v| v.to_string_lossy().into_owned())
                        .as_deref(),
                    Some(*value),
                    "{}",
                    spec.key
                );
            }
            assert!(
                profile.tool_root(spec).starts_with(dir.path()),
                "{} escapes the profile",
                spec.key
            );
            assert!(
                profile.tool_root(spec).is_dir() || matches!(spec.home, Home::Xdg { .. }),
                "{} was not created",
                spec.key
            );
        }
        Ok(())
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
            AuthOperation::Login.args(Tool::Fx),
            Some(["login"].as_slice())
        );
        assert_eq!(
            AuthOperation::Logout.args(Tool::Fx),
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
        assert_eq!(AuthOperation::Login.args(Tool::Pi), None);
        assert_eq!(AuthOperation::Logout.args(Tool::Pi), None);
    }

    #[test]
    fn reads_pi_provider_credentials() {
        let temporary = tempfile::tempdir().unwrap();
        let mut profile = profile();
        profile.pi_home = temporary.path().join("pi");
        fs::create_dir_all(&profile.pi_home).unwrap();

        assert_eq!(pi_auth_status(&profile), AuthStatus::SignedOut);
        fs::write(profile.pi_home.join("auth.json"), "{}").unwrap();
        assert_eq!(pi_auth_status(&profile), AuthStatus::SignedOut);
        fs::write(
            profile.pi_home.join("auth.json"),
            r#"{"anthropic":{"type":"oauth"}}"#,
        )
        .unwrap();
        assert_eq!(pi_auth_status(&profile), AuthStatus::SignedIn);
        fs::write(profile.pi_home.join("auth.json"), "not json").unwrap();
        assert_eq!(pi_auth_status(&profile), AuthStatus::Unavailable);
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
    fn parses_fx_auth_status_output() {
        assert_eq!(
            parse_fx_auth_status(true, br#"{"kind":"status","auth":"missing"}"#),
            AuthStatus::SignedOut
        );
        assert_eq!(
            parse_fx_auth_status(true, br#"{"kind":"status","auth":"Codex subscription"}"#),
            AuthStatus::SignedIn
        );
        assert_eq!(parse_fx_auth_status(false, b"{}"), AuthStatus::Unavailable);
        assert_eq!(
            parse_fx_auth_status(true, b"not json"),
            AuthStatus::Unavailable
        );
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
