//! Shows which profile a running tool is using.
//!
//! Claude Code renders a status line at the bottom of its interface, so Ditto
//! installs one that names the profile. The other tools have no such hook but
//! leave the terminal title alone, so Ditto claims that instead. Neither costs
//! the user a keystroke or a line of their own output.

use std::{
    env, fs,
    io::{self, IsTerminal, Read, Write},
    path::{Path, PathBuf},
    process,
};

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};

use crate::{
    launch::Tool,
    profile::{DEFAULT_PROFILE, Profile, secure_file},
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
}

/// Adds the Ditto status line unless the profile already carries one of its
/// own. Someone who configured a status line meant it, and Claude Code renders
/// only one, so replacing it would silently take a feature away.
pub fn enable(profile: &Profile) -> Result<Indicator> {
    install(profile, &ditto_status_line()?)
}

/// Takes the entry rather than building it, so a test can install a status line
/// naming a known path instead of whichever binary happens to be running.
fn install(profile: &Profile, entry: &Value) -> Result<Indicator> {
    update(profile, |settings| match settings.get("statusLine") {
        Some(existing) if !is_ditto(existing) => (Indicator::Foreign, false),
        Some(existing) if existing == entry => (Indicator::AlreadyOn, false),
        _ => {
            settings.insert("statusLine".to_owned(), entry.clone());
            (Indicator::Installed, true)
        }
    })
}

pub fn disable(profile: &Profile) -> Result<Indicator> {
    update(profile, |settings| match settings.get("statusLine") {
        None => (Indicator::Off, false),
        Some(existing) if !is_ditto(existing) => (Indicator::Foreign, false),
        Some(_) => {
            settings.remove("statusLine");
            (Indicator::Removed, true)
        }
    })
}

pub fn state(profile: &Profile) -> Result<Indicator> {
    let settings = read(&settings_path(profile))?;
    Ok(match settings.get("statusLine") {
        None => Indicator::Off,
        Some(existing) if is_ditto(existing) => Indicator::AlreadyOn,
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

/// Replaces the file in one step so a crash cannot leave Claude Code with a
/// half-written settings file it refuses to start from.
fn write(path: &Path, settings: &Map<String, Value>) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("could not create {}", parent.display()))?;

    let mut contents = serde_json::to_string_pretty(settings)
        .context("could not serialize Claude Code settings")?;
    contents.push('\n');

    let temporary = parent.join(format!(".{SETTINGS}.{}.tmp", process::id()));
    fs::write(&temporary, contents)
        .with_context(|| format!("could not write {}", temporary.display()))?;
    secure_file(&temporary)?;
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error).with_context(|| format!("could not replace {}", path.display()));
    }
    Ok(())
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
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

/// Recognises a command Ditto wrote. It always names the Ditto binary and ends
/// in the subcommand, which no hand-written status line would do by accident.
fn is_ditto(entry: &Value) -> bool {
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
    let _ = write!(stdout, "\u{1b}]0;{}\u{7}", title(tool, profile));
    let _ = stdout.flush();
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

    /// A status line naming an installed Ditto, so these tests do not depend on
    /// where the test binary itself was built. Under `cargo test` the running
    /// executable is `deps/ditto_cli-<hash>`, which `is_ditto` does not
    /// recognise; asserting against it only ever passed because the build path
    /// happened to sit under a directory called `ditto-cli`.
    fn installed_entry() -> Value {
        status_line_entry("/usr/local/bin/ditto-cli")
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

        let entry = installed_entry();
        assert_eq!(install(&profile, &entry)?, Indicator::Installed);
        assert_eq!(state(&profile)?, Indicator::AlreadyOn);
        // A second call must not rewrite a file that already says the right
        // thing, or every launch would churn the user's settings.
        assert_eq!(install(&profile, &entry)?, Indicator::AlreadyOn);

        let installed = settings(&profile);
        assert_eq!(installed["theme"], "dark");
        assert_eq!(installed["model"], "opus");
        assert!(is_ditto(&installed["statusLine"]));

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

        assert_eq!(install(&profile, &installed_entry())?, Indicator::Foreign);
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
        assert_eq!(install(&profile, &installed_entry())?, Indicator::Installed);
        assert!(is_ditto(&settings(&profile)["statusLine"]));
        Ok(())
    }

    #[test]
    fn refuses_to_guess_at_settings_it_cannot_read() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let profile = profile(temporary.path());
        fs::create_dir_all(&profile.claude_home)?;
        fs::write(settings_path(&profile), "{ not json")?;

        // Overwriting here would throw away whatever the user meant to write.
        assert!(install(&profile, &installed_entry()).is_err());
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
        assert!(is_ditto(&installed_entry()));
        // Recognition is by binary name, so a Ditto installed under a different
        // one is not recognised. That only ever hides a status line Ditto wrote
        // from a later `--off`, never overwrites one the user configured.
        assert!(!is_ditto(&status_line_entry("/usr/local/bin/ditto")));
        assert!(!is_ditto(&serde_json::json!({
            "type": "command",
            "command": "bash ~/statusline.sh",
        })));
        // A script that merely mentions Ditto is still the user's.
        assert!(!is_ditto(&serde_json::json!({
            "type": "command",
            "command": "ditto-cli status | head -1",
        })));
    }

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

    #[test]
    fn draws_the_profile_with_and_without_an_account() {
        assert_eq!(
            status_line("share", Some("me@example.com"), false),
            "⬖ share · me@example.com"
        );
        assert_eq!(status_line("share", None, false), "⬖ share");
        assert!(status_line("share", None, true).contains(PURPLE));
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
