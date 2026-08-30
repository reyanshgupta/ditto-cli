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
    process::{Command as Program, Stdio},
};

use anyhow::{Context, Result};
use directories::BaseDirs;
use serde_json::{Map, Value};

use crate::{
    launch::Tool,
    profile::{DEFAULT_PROFILE, Profile},
    settings::{path as settings_path, read, write},
};

/// The name Claude Code's settings file carries wherever it reads one, which
/// the project files that outrank a profile's share with the profile's own.
const SETTINGS: &str = "settings.json";
/// The one key in that file Ditto ever writes. Public because it is also the
/// one key `settings` cannot copy from one profile to another as it stands: it
/// names the profile it was installed for, so what travels is the status line
/// underneath it. See [`inherit`].
pub const KEY: &str = "statusLine";
/// The Ditto subcommand Claude Code runs to draw the status line.
const SUBCOMMAND: &str = "statusline";
/// Carries the status line a profile already had into the command Ditto
/// installs, so both can be drawn and the original can be handed back.
const WITH: &str = "--with";
/// The same, for a command the shell could not carry literally. See
/// [`wrapped_argument`].
const WITH_ENCODED: &str = "--with-encoded";
/// Names the binary in an installed command, so an entry Ditto wrote can be
/// told apart from one the user configured.
const BINARY: &str = "ditto-cli";
/// Sits between the profile and the status line Ditto is drawing in front of.
const JOIN: &str = " │ ";
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
    /// The profile keeps the status line it had, with the profile drawn in
    /// front of it.
    Alongside,
    Removed,
    /// Ditto's status line came out and the one it was drawn in front of went
    /// back exactly as it was.
    Restored,
    Off,
    /// The profile has a status line Ditto did not write.
    Foreign,
    /// Ditto's status line is installed but never appears, because Claude Code
    /// reads a status line from somewhere that outranks the profile.
    Shadowed,
}

impl Indicator {
    pub fn describe(self) -> &'static str {
        match self {
            Self::Installed => "status line installed",
            Self::AlreadyOn => "status line already on",
            Self::Alongside => "status line on, drawn in front of the one you already had",
            Self::Removed => "status line removed",
            Self::Restored => "status line removed, yours put back",
            Self::Off => "status line off",
            Self::Foreign => {
                "left alone: this profile has its own status line, and --keep-mine draws both"
            }
            Self::Shadowed => "installed, but a status line that outranks it is showing instead",
        }
    }

    /// The stable name for this outcome in JSON output. See [`Tool::key`].
    pub fn key(self) -> &'static str {
        match self {
            Self::Installed => "installed",
            Self::AlreadyOn => "already_on",
            Self::Alongside => "alongside",
            Self::Removed => "removed",
            Self::Restored => "restored",
            Self::Off => "off",
            Self::Foreign => "foreign",
            Self::Shadowed => "shadowed",
        }
    }

    /// Whether the status line is showing once this outcome has happened, which
    /// is what a caller checking the setting actually wants to know. A shadowed
    /// entry is written and not drawn, so the honest answer there is no.
    pub fn is_on(self) -> bool {
        matches!(self, Self::Installed | Self::AlreadyOn | Self::Alongside)
    }
}

/// What to do about a status line the profile already has.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Existing {
    /// Leave it exactly as it is and report [`Indicator::Foreign`]. What a
    /// launch does, because a launch was not asked to change what the user
    /// sees.
    LeaveAlone,
    /// Keep it, and draw the profile in front of whatever it prints.
    KeepAlongside,
}

/// Adds the Ditto status line, doing whatever `existing` says about a status
/// line the profile already carries.
///
/// Claude Code draws one status line, so a profile that has one can either keep
/// it or show the profile, never both by accident. Overwriting is not on the
/// list: someone who configured a status line meant it, and taking it away to
/// put a profile name there would be a worse trade than the one they made.
pub fn enable(profile: &Profile, existing: Existing) -> Result<Indicator> {
    let ours = ditto_status_line()?;
    update(profile, |settings| {
        let current = settings.get(KEY).cloned();
        match current {
            // An entry Ditto wrote is rebuilt rather than replaced: this copy
            // of the binary may live somewhere else than the one that wrote it,
            // and anything the entry carries — the status line being drawn in
            // front of, keys Claude Code reads beside the command — has to
            // survive being brought up to date.
            Some(current) if is_ditto(&current, Some(&ours)) => {
                let entry = rebuilt(&current, &ours);
                let alongside = wrapped_command(&entry).is_some();
                if entry == current {
                    let drawn = if alongside {
                        Indicator::Alongside
                    } else {
                        Indicator::AlreadyOn
                    };
                    return (drawn, false);
                }
                settings.insert(KEY.to_owned(), entry);
                let drawn = if alongside {
                    Indicator::Alongside
                } else {
                    Indicator::Installed
                };
                (drawn, true)
            }
            Some(theirs) => match keeping(&theirs, &ours) {
                Some(entry) if existing == Existing::KeepAlongside => {
                    settings.insert(KEY.to_owned(), entry);
                    (Indicator::Alongside, true)
                }
                _ => (Indicator::Foreign, false),
            },
            None => {
                settings.insert(KEY.to_owned(), ours.clone());
                (Indicator::Installed, true)
            }
        }
    })
}

pub fn disable(profile: &Profile) -> Result<Indicator> {
    let ours = ditto_status_line().ok();
    update(profile, |settings| {
        let current = settings.get(KEY).cloned();
        match current {
            None => (Indicator::Off, false),
            Some(current) if !is_ditto(&current, ours.as_ref()) => (Indicator::Foreign, false),
            // Ditto is drawing in front of a status line that was here first,
            // so taking Ditto out means handing that one back rather than
            // leaving the profile with nothing.
            Some(current) => match displaced(&current) {
                Some(theirs) => {
                    settings.insert(KEY.to_owned(), theirs);
                    (Indicator::Restored, true)
                }
                None => {
                    settings.remove(KEY);
                    (Indicator::Removed, true)
                }
            },
        }
    })
}

pub fn state(profile: &Profile) -> Result<Indicator> {
    let ours = ditto_status_line().ok();
    let settings = read(&settings_path(profile))?;
    Ok(match settings.get(KEY) {
        None => Indicator::Off,
        Some(current) if !is_ditto(current, ours.as_ref()) => Indicator::Foreign,
        Some(current) if wrapped_command(current).is_some() => Indicator::Alongside,
        Some(_) => Indicator::AlreadyOn,
    })
}

/// What copying a settings file into a profile should do about its status line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Inherit {
    /// Write this entry into the profile.
    Install(Value),
    /// The profile already carries exactly this.
    Already,
    /// The profile has a status line of its own, which stands.
    Keep,
    /// There is no status line to copy.
    Nothing,
}

/// The status line a profile should carry once its owner's settings have been
/// copied into it.
///
/// An installed entry names the profile it was installed for, so it cannot
/// travel between profiles as it stands. What travels is the status line
/// underneath it — the one its owner wrote — with the new profile's own
/// indicator drawn in front of that. Copying settings forward then costs nobody
/// the status line they configured, which is what dropping the key entirely
/// used to do.
///
/// A profile whose entry is Ditto's own and draws nothing in front of anything
/// has not had a status line chosen for it: that is what a launch installs when
/// it found nothing to keep. Replacing it overwrites no decision, so it happens
/// without being asked for. Anything else the profile carries is a decision and
/// stands unless `overwrite` says otherwise.
pub fn inherit(
    theirs: Option<&Value>,
    current: Option<&Value>,
    overwrite: bool,
) -> Result<Inherit> {
    let Some(theirs) = theirs else {
        return Ok(Inherit::Nothing);
    };
    let ours = ditto_status_line()?;
    let wanted = match underneath(theirs, &ours) {
        Some(own) => keeping(&own, &ours).unwrap_or_else(|| ours.clone()),
        None => ours.clone(),
    };

    Ok(match current {
        None => Inherit::Install(wanted),
        Some(current) if *current == wanted => Inherit::Already,
        // Ditto's own entry, drawn in front of nothing: the default a launch
        // leaves behind rather than something anyone asked for.
        Some(current) if is_ditto(current, Some(&ours)) && wrapped_command(current).is_none() => {
            Inherit::Install(wanted)
        }
        Some(_) if overwrite => Inherit::Install(wanted),
        Some(_) => Inherit::Keep,
    })
}

/// The status line a person wrote, read out of an entry that may be Ditto's
/// drawn in front of theirs. Nothing when the entry is Ditto's own and there is
/// nothing of theirs beneath it.
fn underneath(entry: &Value, ours: &Value) -> Option<Value> {
    if is_ditto(entry, Some(ours)) {
        return displaced(entry);
    }
    Some(entry.clone())
}

/// Installs the status line without letting a settings problem stop a launch.
/// The tool the user asked for matters more than the indicator, and
/// `ditto-cli indicator` reports the failure properly when they ask for it.
///
/// A launch never touches a status line the user configured. Drawing in front
/// of one changes what they see, which is a thing to be asked for rather than
/// something a launch does on its way past.
pub fn enable_quietly(profile: &Profile) {
    let _ = enable(profile, Existing::LeaveAlone);
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

/// Ditto's entry with the profile's own status line folded into it, or nothing
/// if that status line is not a command Ditto could run.
///
/// Only a `command` entry can be kept: Claude Code may learn other kinds, and
/// one Ditto does not know how to run is one it must not claim to be drawing.
fn keeping(theirs: &Value, ours: &Value) -> Option<Value> {
    if theirs.get("type").and_then(Value::as_str) != Some("command") {
        return None;
    }
    let command = theirs.get("command").and_then(Value::as_str)?;
    if command.trim().is_empty() {
        return None;
    }
    let mut entry = wrapping(ours, command);
    carry_over(theirs, &mut entry);
    Some(entry)
}

/// The entry Ditto would write now, still saying everything the installed one
/// said. The binary may have moved since it was installed, which is the whole
/// reason to rewrite it, but the status line being drawn in front of and any
/// other keys the entry carries belong to the user and are kept.
fn rebuilt(current: &Value, ours: &Value) -> Value {
    let mut entry = match wrapped_command(current) {
        Some(command) => wrapping(ours, &command),
        None => ours.clone(),
    };
    carry_over(current, &mut entry);
    entry
}

/// The status line an entry of Ditto's is drawn in front of, put back the way
/// its owner wrote it.
fn displaced(current: &Value) -> Option<Value> {
    let command = wrapped_command(current)?;
    let mut entry = serde_json::json!({ "type": "command", "command": command });
    carry_over(current, &mut entry);
    Some(entry)
}

/// Ditto's command with the status line it is drawing in front of appended to
/// it, so the whole arrangement lives in the one string Claude Code runs. A
/// file beside the settings would be tidier to read and would go missing the
/// moment someone edited, moved, or copied the settings on its own.
fn wrapping(ours: &Value, wrapped: &str) -> Value {
    let ours = ours
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let command = format!("{ours} {}", wrapped_argument(wrapped));
    serde_json::json!({ "type": "command", "command": command })
}

/// Moves the keys that are neither the command nor its type across, so a
/// `padding` or anything else Claude Code grows is not lost by rewriting the
/// entry around it.
fn carry_over(from: &Value, into: &mut Value) {
    let (Some(from), Some(into)) = (from.as_object(), into.as_object_mut()) else {
        return;
    };
    for (key, value) in from {
        if key != "command" && key != "type" && !into.contains_key(key) {
            into.insert(key.clone(), value.clone());
        }
    }
}

/// The status line an installed entry is drawing in front of, if any.
fn wrapped_command(entry: &Value) -> Option<String> {
    let command = entry.get("command").and_then(Value::as_str)?;
    if let Some((_, encoded)) = command.split_once(&format!("{WITH_ENCODED} ")) {
        return percent_decode(encoded.trim());
    }
    let (_, quoted) = command.split_once(&format!("{WITH} "))?;
    shell_unquote(quoted.trim())
}

/// Hands a whole command to the copy of Ditto the shell will start, as one
/// argument it can hand back unchanged.
#[cfg(not(windows))]
fn wrapped_argument(command: &str) -> String {
    format!("{WITH} {}", shell_quote(command))
}

/// The command prompt has no escape for a double quote inside a quoted
/// argument, and expands anything between percent signs before Ditto is even
/// started. A command carrying either travels percent-encoded instead, which
/// costs the settings file some readability in the rare case and keeps the
/// common one plain.
#[cfg(windows)]
fn wrapped_argument(command: &str) -> String {
    if command.contains('"') || command.contains('%') {
        format!("{WITH_ENCODED} {}", percent_encode(command))
    } else {
        format!("{WITH} {}", shell_quote(command))
    }
}

/// Reads back what [`shell_quote`] wrote. Both quoting styles are understood
/// wherever Ditto runs, so a settings file does not stop making sense because
/// it was written on another platform.
fn shell_unquote(value: &str) -> Option<String> {
    if let Some(inner) = value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
        return Some(inner.replace(r"'\''", "'"));
    }
    value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .map(str::to_owned)
}

/// Encodes everything a shell might read as punctuation, leaving a value made
/// only of characters every shell hands over untouched.
#[cfg_attr(not(windows), allow(dead_code))]
fn percent_encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z' | b'-' | b'_' | b'.' => {
                String::from(byte as char)
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn percent_decode(value: &str) -> Option<String> {
    let raw = value.as_bytes();
    let mut decoded = Vec::with_capacity(raw.len());
    let mut at = 0;
    while at < raw.len() {
        if raw[at] == b'%' {
            let digits = value.get(at + 1..at + 3)?;
            decoded.push(u8::from_str_radix(digits, 16).ok()?);
            at += 3;
        } else {
            decoded.push(raw[at]);
            at += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

/// Recognises a command Ditto wrote, given the one it would write now.
///
/// The entry Ditto is about to install is its own by definition, whatever the
/// binary happens to be called. Beyond that the name has to carry the claim: an
/// entry left by an earlier install, from a directory this copy was never in,
/// still has to be recognised as Ditto's rather than mistaken for the user's
/// and left in place forever.
///
/// The claim is the shape of the command rather than the words in it: the
/// binary, then the subcommand as a word of its own, then nothing or the
/// argument naming a status line to draw in front of. A script the user wrote
/// and called `ditto-cli-statusline` says both words and is still theirs.
fn is_ditto(entry: &Value, ours: Option<&Value>) -> bool {
    if ours == Some(entry) {
        return true;
    }
    let Some(command) = entry.get("command").and_then(Value::as_str) else {
        return false;
    };
    // This copy's own command with a status line to draw in front of appended
    // to it is this copy's command, whatever the binary is called.
    let ours = ours
        .and_then(|ours| ours.get("command"))
        .and_then(Value::as_str);
    if let Some(rest) = ours.and_then(|ours| command.strip_prefix(ours))
        && rest.trim_start().starts_with(WITH)
    {
        return true;
    }
    claims_ditto(command)
}

fn claims_ditto(command: &str) -> bool {
    let words: Vec<&str> = command.split_whitespace().collect();
    let Some(subcommand) = words.iter().position(|word| *word == SUBCOMMAND) else {
        return false;
    };
    if subcommand == 0 || !names_binary(words[subcommand - 1]) {
        return false;
    }
    match words.get(subcommand + 1) {
        None => true,
        // A flag with nothing after it is not something Ditto ever wrote.
        Some(&WITH | &WITH_ENCODED) => words.len() > subcommand + 2,
        Some(_) => false,
    }
}

/// Whether a word of a command names the Ditto binary. The word is whatever
/// the shell was given, so it carries the quotes and the path it was written
/// with, and on Windows an extension as well.
fn names_binary(word: &str) -> bool {
    let word = word.trim_matches(['\'', '"']);
    let name = word.rsplit(['/', '\\']).next().unwrap_or(word);
    let name = name.to_ascii_lowercase();
    name.strip_suffix(".exe").unwrap_or(&name) == BINARY
}

/// Reports a status line that is installed and will never be seen.
///
/// Claude Code reads settings from several places and draws the status line
/// from the one that outranks the rest. The profile's own file is the bottom of
/// that pile, so an entry there can be written, correct, and silently beaten by
/// a project or an administrator. Saying "installed" to someone who is looking
/// at somebody else's status line is the one answer worse than saying no.
///
/// The answer depends on where the caller is standing rather than on the
/// profile, so it is asked for when a person is being told what happened and
/// not on the way past during a launch.
pub fn shadowed(outcome: Indicator) -> Indicator {
    if outcome.is_on() && outranking_status_line() {
        Indicator::Shadowed
    } else {
        outcome
    }
}

fn outranking_status_line() -> bool {
    let directory = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let home = BaseDirs::new().map(|base| base.home_dir().to_path_buf());
    outranking_settings(&directory, home.as_deref())
        .iter()
        .any(|path| read(path).is_ok_and(|settings| settings.contains_key(KEY)))
}

/// The settings files Claude Code reads before the profile's own: the project
/// settings for the directory it was started in, then whatever the machine's
/// administrator set. Ancestors count because Claude Code reads the project it
/// is in, whichever directory below that the user happens to be standing in.
///
/// The walk stops at the home directory, which is not a project however many
/// projects sit inside it. `~/.claude/settings.json` is the file a profile is
/// made of, and counting it as something that outranks a profile would have
/// every default profile reporting itself shadowed by itself.
fn outranking_settings(from: &Path, home: Option<&Path>) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for ancestor in from.ancestors() {
        if Some(ancestor) == home {
            break;
        }
        let project = ancestor.join(".claude");
        paths.push(project.join("settings.local.json"));
        paths.push(project.join(SETTINGS));
    }
    paths.push(PathBuf::from(MANAGED_SETTINGS));
    paths
}

/// Where an administrator's settings live, which outrank everything else
/// Claude Code reads.
#[cfg(target_os = "macos")]
const MANAGED_SETTINGS: &str = "/Library/Application Support/ClaudeCode/managed-settings.json";
#[cfg(all(unix, not(target_os = "macos")))]
const MANAGED_SETTINGS: &str = "/etc/claude-code/managed-settings.json";
#[cfg(windows)]
const MANAGED_SETTINGS: &str = r"C:\ProgramData\ClaudeCode\managed-settings.json";

/// Draws the status line Claude Code asked for, in front of the one the
/// profile already had when there is one to draw.
pub fn render(with: Option<String>, with_encoded: Option<String>) -> Result<()> {
    // Claude Code writes a JSON payload describing the session. Ditto reports
    // the profile rather than anything from it, but the payload is still read:
    // leaving it in the pipe would hand Claude Code a write error, and the
    // status line being drawn in front of is owed the same payload Ditto got.
    let mut payload = String::new();
    let _ = io::stdin().read_to_string(&mut payload);

    let config = env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from);
    let name = profile_name(config.as_deref());
    let account = config.as_deref().and_then(account);
    let ours = status_line(&name, account.as_deref(), colour());

    let wrapped = with.or_else(|| with_encoded.as_deref().and_then(percent_decode));
    match wrapped
        .as_deref()
        .and_then(|command| drawn_by(command, &payload))
    {
        Some(theirs) => println!("{}", joined(&ours, &theirs, colour())),
        None => println!("{ours}"),
    }
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

/// Puts the profile in front of the status line it was drawn beside, keeping
/// every line of it: a status line of several lines is several lines of
/// somebody's work, and cutting it to one would take back what keeping it was
/// for.
fn joined(ours: &str, theirs: &str, colour: bool) -> String {
    let mut lines = theirs.lines();
    let Some(first) = lines.next() else {
        return ours.to_owned();
    };
    let separator = if colour {
        format!("{DIM}{JOIN}{RESET}")
    } else {
        JOIN.to_owned()
    };
    let rest: String = lines.map(|line| format!("\n{line}")).collect();
    format!("{ours}{separator}{first}{rest}")
}

/// Runs the status line the profile already had, handing it the payload Claude
/// Code handed Ditto so it sees exactly what it saw before.
///
/// A command that fails, says nothing, or cannot be started leaves the profile
/// marker on its own. Ditto asked to draw in front of that status line, not to
/// become a way for it to break the line it used to be.
fn drawn_by(command: &str, payload: &str) -> Option<String> {
    let mut child = shell(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Claude Code shows what a status line writes to stderr as an error.
        // That belongs to the command that wrote it, not to Ditto for running
        // it, and a warning on every redraw is worse than none.
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(payload.as_bytes());
    }
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    let drawn = String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_owned();
    (!drawn.trim().is_empty()).then_some(drawn)
}

/// Starts the command the way Claude Code would have started it, so a status
/// line written for a shell keeps working when Ditto is the one running it.
#[cfg(not(windows))]
fn shell(command: &str) -> Program {
    let mut shell = Program::new("sh");
    shell.arg("-c").arg(command);
    shell
}

#[cfg(windows)]
fn shell(command: &str) -> Program {
    use std::os::windows::process::CommandExt;

    let mut shell = Program::new("cmd");
    // Verbatim: the command prompt is being handed a command line, not an
    // argument, and quoting it again would change what it says.
    shell.arg("/C").raw_arg(command);
    shell
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
            fx_home: root.join("fx-home"),
            omp_home: root.join("omp"),
            opencode: OpencodeHome {
                data: root.join("opencode/data"),
                config: root.join("opencode/config"),
                state: root.join("opencode/state"),
            },
            pi_home: root.join("pi"),
            prime_agent_home: root.join("prime-agent"),
            generic: Vec::new(),
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

        assert_eq!(
            enable(&profile, Existing::LeaveAlone)?,
            Indicator::Installed
        );
        assert_eq!(state(&profile)?, Indicator::AlreadyOn);
        // A second call must not rewrite a file that already says the right
        // thing, or every launch would churn the user's settings.
        assert_eq!(
            enable(&profile, Existing::LeaveAlone)?,
            Indicator::AlreadyOn
        );

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

        assert_eq!(enable(&profile, Existing::LeaveAlone)?, Indicator::Foreign);
        assert_eq!(state(&profile)?, Indicator::Foreign);
        assert_eq!(disable(&profile)?, Indicator::Foreign);
        assert_eq!(
            settings(&profile)["statusLine"]["command"],
            "bash ~/mine.sh"
        );
        Ok(())
    }

    /// Claude Code draws one status line, so the only way to have both is for
    /// Ditto to run theirs and print the profile in front of what it says.
    #[test]
    fn draws_the_profile_in_front_of_a_status_line_asked_to_be_kept() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let profile = profile(temporary.path());
        fs::create_dir_all(&profile.claude_home)?;
        let theirs = serde_json::json!({
            "type": "command",
            "command": "npx -y ccstatusline@latest",
            "padding": 0,
        });
        fs::write(
            settings_path(&profile),
            serde_json::json!({ "statusLine": theirs, "theme": "dark" }).to_string(),
        )?;

        assert_eq!(
            enable(&profile, Existing::KeepAlongside)?,
            Indicator::Alongside
        );
        assert_eq!(state(&profile)?, Indicator::Alongside);

        let installed = &settings(&profile)["statusLine"];
        assert_eq!(
            wrapped_command(installed).as_deref(),
            Some("npx -y ccstatusline@latest")
        );
        // Anything the entry carried beside the command belongs to whoever
        // wrote it and has to survive being wrapped.
        assert_eq!(installed["padding"], 0);

        // A launch afterwards must not quietly undo the arrangement, and must
        // still be able to bring the path to the binary up to date.
        assert_eq!(
            enable(&profile, Existing::LeaveAlone)?,
            Indicator::Alongside
        );
        assert_eq!(
            wrapped_command(&settings(&profile)["statusLine"]).as_deref(),
            Some("npx -y ccstatusline@latest")
        );

        // Turning the indicator off hands back exactly what was there before.
        assert_eq!(disable(&profile)?, Indicator::Restored);
        assert_eq!(settings(&profile)["statusLine"], theirs);
        assert_eq!(settings(&profile)["theme"], "dark");
        Ok(())
    }

    /// Ditto runs the kept status line through a shell. An entry of a kind it
    /// does not know how to run cannot be kept, and saying so beats installing
    /// a status line that silently draws half of what it promised.
    #[test]
    fn will_not_claim_to_keep_a_status_line_it_cannot_run() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let profile = profile(temporary.path());
        fs::create_dir_all(&profile.claude_home)?;
        let theirs = r#"{"statusLine":{"type":"something-new","value":"…"}}"#;
        fs::write(settings_path(&profile), theirs)?;

        assert_eq!(
            enable(&profile, Existing::KeepAlongside)?,
            Indicator::Foreign
        );
        assert_eq!(settings(&profile)["statusLine"]["type"], "something-new");
        Ok(())
    }

    /// A command with quotes, spaces, and shell punctuation in it has to come
    /// back out of the settings file byte for byte, or turning the indicator
    /// off would hand back a status line that no longer runs.
    #[test]
    fn carries_an_awkward_command_there_and_back() {
        let ours = status_line_entry("/usr/local/bin/ditto-cli");
        for command in [
            "bash ~/mine.sh",
            r#"sh -c 'printf "%s" "$(git branch --show-current)"'"#,
            "node C:\\Program Files\\bar\\line.js --flag",
            "printf '100%% ready'",
        ] {
            let entry = wrapping(&ours, command);
            assert!(is_ditto(&entry, Some(&ours)), "{command}");
            assert_eq!(wrapped_command(&entry).as_deref(), Some(command));
        }
    }

    /// The point of encoding is that what comes out is made only of characters
    /// no shell reads as punctuation, and that what went in comes back.
    #[test]
    fn percent_encoding_survives_a_round_trip() {
        for value in ["", "plain", r#"a "b" & c% ^d"#, "üñî"] {
            let encoded = percent_encode(value);
            assert!(
                encoded.chars().all(
                    |character| character.is_ascii_alphanumeric() || "-_.%".contains(character)
                ),
                "{encoded}"
            );
            assert_eq!(percent_decode(&encoded).as_deref(), Some(value));
        }
        assert_eq!(percent_decode("%2"), None);
        assert_eq!(percent_decode("%zz"), None);
    }

    #[test]
    fn writes_a_settings_file_a_profile_does_not_have_yet() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let profile = profile(temporary.path());

        assert_eq!(state(&profile)?, Indicator::Off);
        assert_eq!(
            enable(&profile, Existing::LeaveAlone)?,
            Indicator::Installed
        );
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
        assert!(enable(&profile, Existing::LeaveAlone).is_err());
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
        // An entry drawing in front of a status line the user had first.
        assert!(is_ditto(
            &wrapping(&installed, "bash ~/mine.sh"),
            Some(&installed)
        ));

        assert!(!is_ditto(&theirs("bash ~/statusline.sh"), Some(&installed)));
        // A script that merely mentions Ditto is still the user's, however it
        // is spelled. Ditto writes the binary and the subcommand as two words
        // and nothing else does.
        for theirs in [
            theirs("ditto-cli status | head -1"),
            theirs("~/bin/ditto-cli-statusline"),
            theirs("bash ~/.config/ditto-cli/statusline"),
            theirs("ditto-cli statusline-mine"),
        ] {
            assert!(!is_ditto(&theirs, Some(&installed)), "{theirs}");
        }
    }

    /// The status line Claude Code draws comes from the highest-ranking
    /// settings file that names one, and a profile's own file is the lowest of
    /// them. Reporting an entry there as showing when a project's outranks it
    /// would send someone looking for a bug in Ditto.
    #[test]
    fn looks_for_settings_that_outrank_a_profile() {
        let home = Path::new("/home/u");
        let paths = outranking_settings(&home.join("code/repo/src"), Some(home));

        assert!(paths.contains(&home.join("code/repo/.claude/settings.json")));
        assert!(paths.contains(&home.join("code/repo/.claude/settings.local.json")));
        assert!(paths.contains(&home.join("code/.claude/settings.json")));
        assert!(paths.contains(&PathBuf::from(MANAGED_SETTINGS)));
        // The home directory is not a project, and `~/.claude/settings.json` is
        // the default profile itself rather than something above it.
        assert!(!paths.contains(&home.join(".claude/settings.json")));
    }

    #[test]
    fn says_a_status_line_is_on_only_when_it_will_be_seen() {
        assert!(Indicator::Alongside.is_on());
        assert!(!Indicator::Shadowed.is_on());
        assert!(!Indicator::Foreign.is_on());
        // Nothing outranks a profile from a directory that has no project
        // settings above it, so the outcome is left as it was.
        assert_eq!(shadowed(Indicator::Off), Indicator::Off);
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

    /// A status line of several lines is several lines of somebody's work, and
    /// the profile goes in front of it rather than in place of any of it.
    #[test]
    fn puts_the_profile_in_front_of_every_line_it_was_given() {
        assert_eq!(
            joined("⬖ work", "main ✓ 12k tokens", false),
            "⬖ work │ main ✓ 12k tokens"
        );
        assert_eq!(
            joined("⬖ work", "first\nsecond", false),
            "⬖ work │ first\nsecond"
        );
        // Nothing to draw beside leaves the profile on its own rather than
        // trailing a separator with nothing after it.
        assert_eq!(joined("⬖ work", "", false), "⬖ work");
    }

    #[cfg(not(windows))]
    #[test]
    fn runs_the_kept_status_line_with_the_payload_claude_code_sent() {
        let payload = r#"{"workspace":{"current_dir":"/tmp"}}"#;

        assert_eq!(
            drawn_by("cat", payload).as_deref(),
            Some(payload),
            "the command is owed the same payload Ditto was given"
        );
        assert_eq!(
            drawn_by("printf 'a\\nb\\n'", payload).as_deref(),
            Some("a\nb")
        );

        // A status line that fails, says nothing, or does not exist leaves the
        // profile marker on its own instead of taking it down too.
        assert_eq!(drawn_by("exit 1", payload), None);
        assert_eq!(drawn_by("printf ''", payload), None);
        assert_eq!(drawn_by("no-such-status-line-command", payload), None);
    }

    #[test]
    fn titles_name_the_profile_except_where_the_tool_owns_them() {
        let temporary = tempfile::tempdir().unwrap();
        let profile = profile(temporary.path());

        assert_eq!(title(Tool::Codex, &profile), "ditto:work — Codex");
        assert_eq!(title(Tool::Fx, &profile), "ditto:work — fx");
        assert_eq!(title(Tool::Opencode, &profile), "ditto:work — opencode");
        assert_eq!(
            title(Tool::PrimeAgent, &profile),
            "ditto:work — Prime Agent"
        );
        assert_eq!(title(Tool::Pi, &profile), "ditto:work — Pi");
        assert!(sets_own_title(Tool::Claude));
        assert!(!sets_own_title(Tool::Codex));
        assert!(!sets_own_title(Tool::Fx));
        assert!(!sets_own_title(Tool::PrimeAgent));
        assert!(!sets_own_title(Tool::Pi));
    }
}
