//! Claude Code's `settings.json`, and carrying a person's own preferences into
//! the profiles that would otherwise start without them.
//!
//! Pointing Claude Code at a profile's configuration directory moves the whole
//! user settings layer along with it, so an isolated profile inherits none of
//! the permission mode, model, effort, or hooks its owner set up once and
//! expects everywhere. What Ditto isolates is credentials, and none of those
//! live in this file — Claude Code keeps them in the keychain and in
//! `.claude.json` — so the settings are copied forward at creation and on
//! request afterwards, while the accounts stay apart.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value};

use crate::{
    indicator::{self, Inherit, KEY as OWNED},
    profile::{DEFAULT_PROFILE, Profile, Store, write_private_file},
};

/// Claude Code's settings file, which it reads from whichever configuration
/// directory it was pointed at.
const FILE: &str = "settings.json";

pub fn path(profile: &Profile) -> PathBuf {
    profile.claude_home.join(FILE)
}

/// What a copy did, key by key, so the caller can say so rather than leaving a
/// settings change to be discovered at the next launch.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Copied {
    /// Keys written into the profile.
    pub copied: Vec<String>,
    /// Keys the profile had already answered differently, left as they were.
    pub kept: Vec<String>,
}

impl Copied {
    pub fn changed(&self) -> bool {
        !self.copied.is_empty()
    }
}

/// Copies the source profile's Claude Code settings into the target profile.
///
/// Keys the target has already set are left alone unless `overwrite` is asked
/// for: a profile is a place to differ from the defaults, and a copy that
/// silently undid a per-profile choice would make it useless. Merging is by
/// top-level key rather than deep, so a profile that sets its own `permissions`
/// keeps all of it and does not end up with half of each.
pub fn copy(source: &Profile, target: &Profile, overwrite: bool) -> Result<Copied> {
    if source.claude_home == target.claude_home {
        bail!(
            "'{}' is your existing Claude Code configuration, which is what Ditto copies from",
            target.name
        );
    }

    let from = read(&path(source))?;
    let mut into = read(&path(target))?;
    let mut result = Copied::default();
    // Read before the loop consumes the map. The status line is the one key
    // that cannot travel as it stands, so it is settled on its own terms below.
    let theirs = from.get(OWNED).cloned();

    for (key, value) in from {
        if key == OWNED {
            continue;
        }
        match into.get(&key) {
            Some(existing) if *existing == value => {}
            Some(_) if !overwrite => result.kept.push(key),
            _ => {
                into.insert(key.clone(), value);
                result.copied.push(key);
            }
        }
    }

    let current = into.get(OWNED).cloned();
    match indicator::inherit(theirs.as_ref(), current.as_ref(), overwrite)? {
        Inherit::Install(entry) => {
            into.insert(OWNED.to_owned(), entry);
            result.copied.push(OWNED.to_owned());
        }
        Inherit::Keep => result.kept.push(OWNED.to_owned()),
        Inherit::Already | Inherit::Nothing => {}
    }

    if result.changed() {
        write(&path(target), &into)?;
    }
    // `preserve_order` keeps the settings in the order they were written, which
    // is right for the file and arbitrary for a list someone has to read.
    result.copied.sort_unstable();
    result.kept.sort_unstable();
    Ok(result)
}

/// Gives a profile the settings it should start life with.
///
/// Both `create` and the picker make profiles, and one made in the picker has
/// to be no different from one made at the command line, so the rule lives here
/// rather than at either call site. Quietly, because a problem in the settings
/// being copied is not a reason to fail a creation that otherwise worked — the
/// profile is usable without them, and `ditto-cli sync` reports the failure
/// properly when it is asked for the same thing.
pub fn seed(store: &Store, profile: &Profile) -> Copied {
    match store.load_profile(DEFAULT_PROFILE) {
        Ok(source) => copy(&source, profile, false).unwrap_or_default(),
        Err(_) => Copied::default(),
    }
}

/// A profile that has never run Claude Code has no settings file yet, which
/// reads the same as one with nothing configured.
pub fn read(path: &Path) -> Result<Map<String, Value>> {
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

pub fn write(path: &Path, settings: &Map<String, Value>) -> Result<()> {
    let mut contents = serde_json::to_string_pretty(settings)
        .context("could not serialize Claude Code settings")?;
    contents.push('\n');
    write_private_file(path, &contents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn store(root: &Path) -> Store {
        Store::new(root.join("ditto"), root.join("home"))
    }

    fn given(profile: &Profile, contents: &str) {
        fs::create_dir_all(&profile.claude_home).unwrap();
        fs::write(path(profile), contents).unwrap();
    }

    fn settings(profile: &Profile) -> Value {
        Value::Object(read(&path(profile)).unwrap())
    }

    #[test]
    fn copies_the_users_own_settings_into_a_profile() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        given(
            &source,
            r#"{"permissions":{"defaultMode":"auto"},"model":"opus"}"#,
        );

        let target = store.create_profile("work").unwrap();
        let copied = copy(&source, &target, false).unwrap();

        assert_eq!(copied.copied, ["model", "permissions"]);
        assert_eq!(settings(&target)["permissions"]["defaultMode"], "auto");
        assert_eq!(settings(&target)["model"], "opus");
    }

    /// The status line names the profile it was installed for, so it cannot
    /// travel as it stands. The one underneath it is the user's own and does,
    /// with the profile drawn in front of it: dropping the key outright used to
    /// cost people the status line they had configured.
    #[test]
    fn keeps_the_status_line_you_had_and_draws_the_profile_in_front() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        given(
            &source,
            r#"{"statusLine":{"type":"command","command":"mine.sh"},"theme":"dark"}"#,
        );

        let target = store.create_profile("work").unwrap();
        let copied = copy(&source, &target, false).unwrap();

        assert_eq!(copied.copied, ["statusLine", "theme"]);
        let installed = settings(&target)["statusLine"]["command"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(installed.contains("statusline"), "{installed}");
        assert!(installed.contains("mine.sh"), "{installed}");
    }

    /// A profile that has only ever been launched carries Ditto's own entry
    /// drawn in front of nothing, which is a default rather than a decision.
    /// Syncing replaces it, so the profiles made before any of this could be
    /// brought up to date without `--overwrite`.
    #[test]
    fn replaces_the_bare_indicator_a_launch_left_behind() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        given(
            &source,
            r#"{"statusLine":{"type":"command","command":"mine.sh"}}"#,
        );

        let target = store.create_profile("work").unwrap();
        crate::indicator::enable(&target, crate::indicator::Existing::LeaveAlone).unwrap();
        let copied = copy(&source, &target, false).unwrap();

        assert_eq!(copied.copied, ["statusLine"]);
        assert!(
            settings(&target)["statusLine"]["command"]
                .as_str()
                .unwrap()
                .contains("mine.sh")
        );
    }

    /// A status line the profile was given on purpose is a decision, and stands.
    #[test]
    fn leaves_a_status_line_the_profile_chose_for_itself() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        given(
            &source,
            r#"{"statusLine":{"type":"command","command":"mine.sh"}}"#,
        );

        let target = store.create_profile("work").unwrap();
        given(
            &target,
            r#"{"statusLine":{"type":"command","command":"theirs.sh"}}"#,
        );
        let copied = copy(&source, &target, false).unwrap();

        assert_eq!(copied.kept, ["statusLine"]);
        assert_eq!(settings(&target)["statusLine"]["command"], "theirs.sh");
    }

    /// A profile exists to differ from the defaults, so what it says about a
    /// setting outranks what the user's own configuration says.
    #[test]
    fn leaves_settings_the_profile_has_already_answered() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        given(&source, r#"{"model":"opus","theme":"dark"}"#);

        let target = store.create_profile("work").unwrap();
        given(&target, r#"{"model":"sonnet"}"#);
        let copied = copy(&source, &target, false).unwrap();

        assert_eq!(copied.copied, ["theme"]);
        assert_eq!(copied.kept, ["model"]);
        assert_eq!(settings(&target)["model"], "sonnet");
    }

    #[test]
    fn replaces_answered_settings_only_when_asked_to() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        given(&source, r#"{"model":"opus"}"#);

        let target = store.create_profile("work").unwrap();
        given(&target, r#"{"model":"sonnet"}"#);
        let copied = copy(&source, &target, true).unwrap();

        assert_eq!(copied.copied, ["model"]);
        assert!(copied.kept.is_empty());
        assert_eq!(settings(&target)["model"], "opus");
    }

    /// A setting that already matches was neither copied nor withheld, and
    /// reporting it as either would misdescribe what happened.
    #[test]
    fn says_nothing_about_settings_that_already_match() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        given(&source, r#"{"theme":"dark"}"#);

        let target = store.create_profile("work").unwrap();
        given(&target, r#"{"theme":"dark"}"#);
        let copied = copy(&source, &target, false).unwrap();

        assert!(!copied.changed());
        assert!(copied.kept.is_empty());
    }

    #[test]
    fn refuses_to_copy_the_default_profile_onto_itself() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();

        assert!(copy(&source, &source, false).is_err());
    }

    /// Creating a profile must not fail because the settings being copied are
    /// unreadable; the profile itself is fine, and `sync` explains the rest.
    #[test]
    fn a_broken_settings_file_does_not_stop_a_quiet_copy() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        given(&source, "{ not json");

        let target = store.create_profile("work").unwrap();

        assert!(!seed(&store, &target).changed());
        assert!(copy(&source, &target, false).is_err());
    }
}
