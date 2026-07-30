//! Shows which profile a running tool is using.
//!
//! Claude Code renders a status line at the bottom of its interface, so Ditto
//! installs one that names the profile. The other tools have no such hook but
//! leave the terminal title alone, so Ditto claims that instead. Neither costs
//! the user a keystroke or a line of their own output.

use std::{
    env, fs,
    io::{self, IsTerminal, Read},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};

use crate::{
    launch::Tool,
    profile::{DEFAULT_PROFILE, Profile, write_private_file},
};

/// Claude Code's per-directory settings file, which is also where a profile
/// keeps the status line Ditto installs.
const SETTINGS: &str = "settings.json";
/// The Ditto subcommand Claude Code runs to draw the status line.
const SUBCOMMAND: &str = "statusline";
/// Names the binary in an installed command, so an entry Ditto wrote can be
/// told apart from one the user configured.
const BINARY: &str = "ditto-cli";
/// Claude Code reads the profile from here when Ditto launched it. Falling back
/// to the configuration directory covers a `claude` started by hand.
const PROFILE_VARIABLE: &str = "DITTO_PROFILE";
const PROFILE_MARK: &str = "⬖";

const PURPLE: &str = "\u{1b}[38;2;190;134;255m";
const DIM: &str = "\u{1b}[2m";
const RESET: &str = "\u{1b}[0m";

/// What a request to change the status line actually did. Ditto never replaces
/// a status line it did not write, so "left alone" is a normal answer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Indicator {
    Installed,
    AlreadyOn,
    Removed,
    Off,
    /// The profile has a status line Ditto did not write.
    Foreign,
}

impl Indicator {
    pub fn describe(self) -> &'static str {
        match self {
            Self::Installed => "status line installed",
            Self::AlreadyOn => "status line already on",
            Self::Removed => "status line removed",
            Self::Off => "status line off",
            Self::Foreign => "left alone: this profile has its own status line",
        }
    }

    /// The stable name for this outcome in JSON output. See [`Tool::key`].
    pub fn key(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::AlreadyOn => "already_on",
            Self::Removed => "removed",
            Self::Off => "off",
            Self::Foreign => "foreign",
        }
    }

    /// Whether the status line is showing once this outcome has happened, which
    /// is what a caller checking the setting actually wants to know.
    pub fn is_on(self) -> bool {
        matches!(self, Self::Installed | Self::AlreadyOn)
    }
}

/// Adds the Ditto status line unless the profile already carries one of its
/// own. Someone who configured a status line meant it, and Claude Code renders
/// only one, so replacing it would silently take a feature away.
pub fn enable(profile: &Profile) -> Result<Indicator> {
    let entry = ditto_status_line()?;
    update(profile, |settings| match settings.get("statusLine") {
        Some(existing) if !is_ditto(existing, Some(&entry)) => (Indicator::Foreign, false),
        Some(existing) if *existing == entry => (Indicator::AlreadyOn, false),
        _ => {
            settings.insert("statusLine".to_owned(), entry.clone());
            (Indicator::Installed, true)
        }
    })
}

pub fn disable(profile: &Profile) -> Result<Indicator> {
    let ours = ditto_status_line().ok();
    update(profile, |settings| match settings.get("statusLine") {
        None => (Indicator::Off, false),
        Some(existing) if !is_ditto(existing, ours.as_ref()) => (Indicator::Foreign, false),
        Some(_) => {
            settings.remove("statusLine");
            (Indicator::Removed, true)
        }
    })
}

pub fn state(profile: &Profile) -> Result<Indicator> {
    let ours = ditto_status_line().ok();
    let settings = read(&settings_path(profile))?;
    Ok(match settings.get("statusLine") {
        None => Indicator::Off,
        Some(existing) if is_ditto(existing, ours.as_ref()) => Indicator::AlreadyOn,
        Some(_) => Indicator::Foreign,
    })
}

/// Installs the status line without letting a settings problem stop a launch.
/// The tool the user asked for matters more than the indicator, and
/// `ditto-cli indicator` reports the failure properly when they ask for it.
pub fn enable_quietly(profile: &Profile) {
    let _ = enable(profile);
}

fn update(
    profile: &Profile,
    change: impl FnOnce(&mut Map<String, Value>) -> (Indicator, bool),
) -> Result<Indicator> {
    let path = settings_path(profile);
    let mut settings = read(&path)?;
    let (outcome, changed) = change(&mut settings);
    if changed {
        write(&path, &settings)?;
    }
    Ok(outcome)
}

fn settings_path(profile: &Profile) -> PathBuf {
    profile.claude_home.join(SETTINGS)
}

/// A profile that has never run Claude Code has no settings file yet, which
/// reads the same as one with nothing configured.
fn read(path: &Path) -> Result<Map<String, Value>> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(error) => {
            return Err(error).with_context(|| format!("could not read {}", path.display()));
        }
    };
    if contents.trim().is_empty() {
        return Ok(Map::new());
    }

    match serde_json::from_str(&contents)
        .with_context(|| format!("could not parse {}", path.display()))?
    {
        Value::Object(settings) => Ok(settings),
        _ => bail!("{} does not hold a JSON object", path.display()),
    }
}

fn write(path: &Path, settings: &Map<String, Value>) -> Result<()> {
    let mut contents = serde_json::to_string_pretty(settings)
        .context("could not serialize Claude Code settings")?;
    contents.push('\n');
    write_private_file(path, &contents)
}

fn ditto_status_line() -> Result<Value> {
    let executable = env::current_exe().context("could not locate the Ditto CLI binary")?;
    Ok(status_line_entry(&executable.to_string_lossy()))
}

fn status_line_entry(executable: &str) -> Value {
    let command = format!("{} {SUBCOMMAND}", shell_quote(executable));
    serde_json::json!({ "type": "command", "command": command })
}

/// Claude Code runs the command through a shell, so a home directory with a
/// space or a quote in it has to survive the trip.
#[cfg(not(windows))]
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// The shell on Windows is the command prompt, which treats a single quote as
/// an ordinary character and would look for a program whose name begins with
/// one. Double quotes are the only grouping it understands, and a path cannot
/// contain one, so there is nothing left to escape inside them.
#[cfg(windows)]
fn shell_quote(value: &str) -> String {
    format!("\"{value}\"")
}

/// Recognises a command Ditto wrote, given the one it would write now.
///
/// The entry Ditto is about to install is its own by definition, whatever the
/// binary happens to be called. Beyond that the name has to carry the claim: an
/// entry left by an earlier install, from a directory this copy was never in,
/// still has to be recognised as Ditto's rather than mistaken for the user's
/// and left in place forever. Naming the binary and ending in the subcommand is
/// what no hand-written status line would do by accident.
fn is_ditto(entry: &Value, ours: Option<&Value>) -> bool {
    if ours == Some(entry) {
        return true;
    }
    entry
        .get("command")
        .and_then(Value::as_str)
        .is_some_and(|command| {
            let command = command.trim_end();
            command.ends_with(SUBCOMMAND) && command.contains(BINARY)
        })
}

/// Draws the status line Claude Code asked for.
pub fn render() -> Result<()> {
    // Claude Code writes a JSON payload describing the session. Ditto reports
    // the profile rather than anything from it, but the payload is still read:
    // leaving it in the pipe would hand Claude Code a write error.
    let mut payload = String::new();
    let _ = io::stdin().read_to_string(&mut payload);

    let config = env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from);
    let name = profile_name(config.as_deref());
    let account = config.as_deref().and_then(account);
    println!("{}", status_line(&name, account.as_deref(), colour()));
    Ok(())
}

fn status_line(name: &str, account: Option<&str>, colour: bool) -> String {
    let account = account.map(|account| format!(" · {account}"));
    let account = account.as_deref().unwrap_or_default();
    if colour {
        format!("{PURPLE}{PROFILE_MARK} {name}{RESET}{DIM}{account}{RESET}")
    } else {
        format!("{PROFILE_MARK} {name}{account}")
    }
}

fn colour() -> bool {
    env::var_os("NO_COLOR").is_none()
}

/// The profile Ditto launched. A `claude` started by hand carries no such
/// marker, so the configuration directory is read instead.
fn profile_name(config: Option<&Path>) -> String {
    if let Some(name) = env::var(PROFILE_VARIABLE)
        .ok()
        .filter(|name| !name.is_empty())
    {
        return name;
    }
    config
        .and_then(profile_name_from_path)
        .unwrap_or_else(|| DEFAULT_PROFILE.to_owned())
}

/// An isolated profile keeps Claude Code's home at
/// `<root>/profiles/<name>/claude`, so the directory above it carries the name.
fn profile_name_from_path(config: &Path) -> Option<String> {
    let profile = config.parent()?;
    if profile.parent()?.file_name()? != "profiles" {
        return None;
    }
    Some(profile.file_name()?.to_str()?.to_owned())
}

/// Claude Code records the signed-in account beside its settings. Reading the
/// file keeps the status line cheap: asking the CLI would start a process on
/// every redraw.
fn account(config: &Path) -> Option<String> {
    let contents = fs::read_to_string(config.join(".claude.json")).ok()?;
    let settings: Value = serde_json::from_str(&contents).ok()?;
    settings
        .get("oauthAccount")?
        .get("emailAddress")?
        .as_str()
        .map(str::to_owned)
}

/// Claude Code paints the title with the task it is working on, which is worth
/// more than a profile name that never changes. The rest leave the title
/// alone, so Ditto can say what they are signed in as.
fn sets_own_title(tool: Tool) -> bool {
    matches!(tool, Tool::Claude)
}

fn title(tool: Tool, profile: &Profile) -> String {
    format!("ditto:{} — {}", profile.name, tool.label())
}

/// Names the window and the tab before the tool takes over the terminal.
/// Redirected output is left untouched, where the escape would land in a file
/// instead of a title bar.
pub fn announce(tool: Tool, profile: &Profile) {
    let mut stdout = io::stdout();
    if sets_own_title(tool) || !stdout.is_terminal() {
        return;
    }
    // Through crossterm rather than by writing the escape sequence directly: a
    // Windows console that has not been put into virtual terminal mode prints
    // the sequence instead of obeying it, and crossterm reaches for the console
    // API there instead.
    let _ = crossterm::execute!(stdout, crossterm::terminal::SetTitle(title(tool, profile)));
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::profile::OpencodeHome;

    fn profile(root: &Path) -> Profile {
        Profile {
            name: "work".to_owned(),
            claude_home: root.join("claude"),
            codex_home: root.join("codex"),
            omp_home: root.join("omp"),
            opencode: OpencodeHome {
                data: root.join("opencode/data"),
                config: root.join("opencode/config"),
                state: root.join("opencode/state"),
            },
            managed: true,
        }
    }

    fn settings(profile: &Profile) -> Map<String, Value> {
        read(&settings_path(profile)).unwrap()
    }

    #[test]
    fn installs_once_and_leaves_the_rest_of_the_settings_alone() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let profile = profile(temporary.path());
        fs::create_dir_all(&profile.claude_home)?;
        fs::write(
            settings_path(&profile),
            r#"{"theme":"dark","model":"opus"}"#,
        )?;

        assert_eq!(enable(&profile)?, Indicator::Installed);
        assert_eq!(state(&profile)?, Indicator::AlreadyOn);
        // A second call must not rewrite a file that already says the right
        // thing, or every launch would churn the user's settings.
        assert_eq!(enable(&profile)?, Indicator::AlreadyOn);

        let installed = settings(&profile);
        assert_eq!(installed["theme"], "dark");
        assert_eq!(installed["model"], "opus");
        assert!(is_ditto(
            &installed["statusLine"],
            ditto_status_line().ok().as_ref()
        ));

        assert_eq!(disable(&profile)?, Indicator::Removed);
        assert_eq!(state(&profile)?, Indicator::Off);
        assert_eq!(disable(&profile)?, Indicator::Off);
        assert_eq!(settings(&profile)["theme"], "dark");
        Ok(())
    }

    #[test]
    fn never_touches_a_status_line_someone_else_configured() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let profile = profile(temporary.path());
        fs::create_dir_all(&profile.claude_home)?;
        let theirs = r#"{"statusLine":{"type":"command","command":"bash ~/mine.sh"}}"#;
        fs::write(settings_path(&profile), theirs)?;

        assert_eq!(enable(&profile)?, Indicator::Foreign);
        assert_eq!(state(&profile)?, Indicator::Foreign);
        assert_eq!(disable(&profile)?, Indicator::Foreign);
        assert_eq!(
            settings(&profile)["statusLine"]["command"],
            "bash ~/mine.sh"
        );
        Ok(())
    }

    #[test]
    fn writes_a_settings_file_a_profile_does_not_have_yet() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let profile = profile(temporary.path());

        assert_eq!(state(&profile)?, Indicator::Off);
        assert_eq!(enable(&profile)?, Indicator::Installed);
        assert_eq!(state(&profile)?, Indicator::AlreadyOn);
        Ok(())
    }

    #[test]
    fn refuses_to_guess_at_settings_it_cannot_read() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let profile = profile(temporary.path());
        fs::create_dir_all(&profile.claude_home)?;
        fs::write(settings_path(&profile), "{ not json")?;

        // Overwriting here would throw away whatever the user meant to write.
        assert!(enable(&profile).is_err());
        assert_eq!(fs::read_to_string(settings_path(&profile))?, "{ not json");
        Ok(())
    }

    #[test]
    fn reads_the_profile_out_of_an_isolated_configuration_path() {
        assert_eq!(
            profile_name_from_path(Path::new("/home/u/.ditto/profiles/work/claude")).as_deref(),
            Some("work")
        );
        // The user's own directory is not laid out that way, so it has no name
        // to report and falls back to the default profile.
        assert_eq!(profile_name_from_path(Path::new("/home/u/.claude")), None);
        assert_eq!(
            profile_name_from_path(Path::new("/home/u/elsewhere/work/claude")),
            None
        );
    }

    #[test]
    fn recognises_only_the_command_it_writes() {
        let installed = status_line_entry("/usr/local/bin/ditto-cli");
        let theirs = |command: &str| serde_json::json!({ "type": "command", "command": command });

        assert!(is_ditto(&installed, Some(&installed)));
        // An earlier install, from a directory this copy has never been in.
        assert!(is_ditto(
            &status_line_entry("/opt/ditto-cli/bin/ditto-cli"),
            Some(&installed)
        ));
        assert!(!is_ditto(&theirs("bash ~/statusline.sh"), Some(&installed)));
        // A script that merely mentions Ditto is still the user's.
        assert!(!is_ditto(
            &theirs("ditto-cli status | head -1"),
            Some(&installed)
        ));
    }

    /// The binary is not always called `ditto-cli`: `cargo test` builds it as
    /// `ditto_cli-<hash>`, and nothing stops a user renaming their copy. What
    /// Ditto has just written is Ditto's whatever it is called, or `enable`
    /// would install a status line and then report it as somebody else's.
    #[test]
    fn recognises_its_own_entry_whatever_the_binary_is_called() {
        let ours = status_line_entry("/home/u/bin/statusline-helper");

        assert!(is_ditto(&ours, Some(&ours)));
        // Without that same copy to compare against there is nothing in the
        // command to claim, and an unclaimed status line is left alone.
        assert!(!is_ditto(&ours, None));
    }

    #[cfg(not(windows))]
    #[test]
    fn quotes_paths_a_shell_would_otherwise_split() {
        assert_eq!(
            shell_quote("/opt/my tools/ditto-cli"),
            "'/opt/my tools/ditto-cli'"
        );
        assert_eq!(
            shell_quote("/o'clock/ditto-cli"),
            r"'/o'\''clock/ditto-cli'"
        );
    }

    #[cfg(windows)]
    #[test]
    fn quotes_paths_the_command_prompt_would_otherwise_split() {
        assert_eq!(
            shell_quote(r"C:\Program Files\ditto\ditto-cli.exe"),
            "\"C:\\Program Files\\ditto\\ditto-cli.exe\""
        );
        // A single quote is an ordinary character in a Windows path and must
        // stay one rather than being taken for punctuation.
        assert_eq!(
            shell_quote(r"C:\o'clock\ditto-cli.exe"),
            "\"C:\\o'clock\\ditto-cli.exe\""
        );
    }

    #[test]
    fn draws_the_profile_with_and_without_an_account() {
        assert_eq!(
            status_line("work", Some("me@example.com"), false),
            "⬖ work · me@example.com"
        );
        assert_eq!(status_line("work", None, false), "⬖ work");
        assert!(status_line("work", None, true).contains(PURPLE));
    }

    #[test]
    fn titles_name_the_profile_except_where_the_tool_owns_them() {
        let temporary = tempfile::tempdir().unwrap();
        let profile = profile(temporary.path());

        assert_eq!(title(Tool::Codex, &profile), "ditto:work — Codex");
        assert_eq!(title(Tool::Opencode, &profile), "ditto:work — opencode");
        assert!(sets_own_title(Tool::Claude));
        assert!(!sets_own_title(Tool::Codex));
    }
}
