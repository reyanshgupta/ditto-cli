//! What a profile borrows from the user's own configuration rather than
//! starting life without it.
//!
//! Ditto isolates accounts. Everything else a person sets up once — skills,
//! subagents, slash commands, hooks, plugins, the memory file each tool reads —
//! is not an account, and a profile that started without those would be a
//! different working environment rather than the same one signed in as somebody
//! else. Those paths are linked back to the user's own configuration, so there
//! is one copy of each, edited in one place, and every profile sees it.
//!
//! What is shared is named rather than deduced. Ditto could instead share
//! everything except the credentials it knows about, which would carry a new
//! extension directory across without being taught about it; it would also
//! carry a new credential file across without being taught about it, and hand
//! every profile the account the last one signed in to. A missing feature is an
//! annoyance and a shared account is the failure Ditto exists to prevent, so
//! the list below is an allowlist and the cost of that is keeping it current.
//!
//! Claude Code's `settings.json` is the one piece of configuration that is
//! copied instead of linked, because Ditto writes the profile's status line
//! into it and linking would mean writing that into the user's own file. See
//! [`crate::settings`].

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use crate::profile::{DEFAULT_PROFILE, Profile, Store};

/// Where a directory a profile already had is moved when the user asks Ditto to
/// link over it, so `--adopt` can never be the command that lost something.
const DISPLACED: &str = "before-ditto";

/// The paths Claude Code reads extensions and instructions from, relative to
/// the configuration directory a launch points it at. `settings.json` is absent
/// deliberately: it is copied, not linked.
const CLAUDE: &[&str] = &[
    "skills",
    "agents",
    "commands",
    "hooks",
    "plugins",
    "output-styles",
    "CLAUDE.md",
];

/// The same for Codex, relative to `CODEX_HOME`. `auth.json` and the session
/// and history files stay behind, which is the whole of what a Codex profile
/// keeps to itself.
///
/// `memories/` is left out on purpose. Codex writes it from what happened in a
/// session rather than from anything a person configured, so it reads as state
/// belonging to the account that produced it, and sharing it would carry one
/// profile's work into another's context.
const CODEX: &[&str] = &[
    "skills",
    "rules",
    "prompts",
    "plugins",
    "config.toml",
    "hooks.json",
    "AGENTS.md",
    "instructions.md",
];

/// The same for OMP, relative to the per-profile agent directory OMP keeps.
/// Its `agent.db` holds credentials beside sessions and never moves.
const OMP: &[&str] = &["config.yml", "extensions"];

/// What linking did, so a caller can say so rather than leaving it to be
/// noticed when a skill turns out to be missing.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Linked {
    /// Paths now reading from the user's own configuration, whether this run
    /// made the link or found it already there.
    pub linked: Vec<String>,
    /// Paths left exactly as they were, because the profile has its own and
    /// replacing it was not asked for.
    pub kept: Vec<String>,
    /// Paths that could not be linked, each with the reason. Windows refuses
    /// symbolic links to an account without the privilege, which is the case
    /// this exists to explain rather than swallow.
    pub failed: Vec<(String, String)>,
    /// Whether this run created a link that was not there before, as opposed to
    /// finding every one of them already in place.
    changed: bool,
}

impl Linked {
    pub fn changed(&self) -> bool {
        self.changed
    }
}

/// One path a profile borrows, named for reporting.
struct Borrowed {
    label: String,
    from: PathBuf,
    into: PathBuf,
}

/// Points every path in the allowlist at the user's own configuration.
///
/// `adopt` decides what happens where the profile already has a real directory
/// of its own: without it that path is reported and left alone, and with it the
/// profile's copy is moved aside and the link put in its place. Nothing is ever
/// deleted either way.
pub fn link(source: &Profile, target: &Profile, adopt: bool) -> Result<Linked> {
    let mut result = Linked::default();
    for borrowed in plan(source, target) {
        // A path the user does not have is not a path to share. Linking it
        // anyway would leave a profile pointing at nothing, which every tool
        // reads differently and none of them read as "empty".
        if !borrowed.from.exists() {
            continue;
        }
        match attach(&borrowed, adopt) {
            Ok(Outcome::Created) => {
                result.linked.push(borrowed.label);
                result.changed = true;
            }
            Ok(Outcome::Already) => result.linked.push(borrowed.label),
            Ok(Outcome::Kept) => result.kept.push(borrowed.label),
            Err(error) => result.failed.push((borrowed.label, format!("{error:#}"))),
        }
    }
    Ok(result)
}

/// Gives a profile the paths it should start life reading.
///
/// Both `create` and the picker make profiles, and one made in the picker has
/// to be no different from one made at the command line, so this sits beside
/// [`crate::settings::seed`] and is called wherever that one is. Quietly, for
/// the same reason: a link that could not be made is not a reason to fail a
/// creation that otherwise worked, and `ditto-cli sync` reports the failure
/// properly when it is asked for the same thing.
pub fn seed(store: &Store, profile: &Profile) -> Linked {
    match store.load_profile(DEFAULT_PROFILE) {
        Ok(source) => link(&source, profile, false).unwrap_or_default(),
        Err(_) => Linked::default(),
    }
}

/// Every path the target borrows from the source, in the order they are
/// reported: one tool at a time, and the tools in the order they are listed
/// everywhere else.
fn plan(source: &Profile, target: &Profile) -> Vec<Borrowed> {
    let mut plan = Vec::new();
    for name in CLAUDE {
        plan.push(Borrowed {
            label: format!("claude/{name}"),
            from: source.claude_home.join(name),
            into: target.claude_home.join(name),
        });
    }
    for name in CODEX {
        plan.push(Borrowed {
            label: format!("codex/{name}"),
            from: source.codex_home.join(name),
            into: target.codex_home.join(name),
        });
    }
    // opencode keeps configuration and credentials in different XDG bases
    // already, so the whole configuration directory can be shared as one link
    // and the data directory holding `auth.json` stays untouched.
    plan.push(Borrowed {
        label: "opencode/config".to_owned(),
        from: source.opencode.config_dir(),
        into: target.opencode.config_dir(),
    });
    for name in OMP {
        plan.push(Borrowed {
            label: format!("omp/{name}"),
            from: source.omp_home.join(name),
            into: target.omp_home.join(name),
        });
    }
    plan
}

enum Outcome {
    Created,
    Already,
    Kept,
}

fn attach(borrowed: &Borrowed, adopt: bool) -> Result<Outcome> {
    // `symlink_metadata` rather than `metadata`, so a link that already points
    // somewhere is read as a link rather than as whatever it leads to.
    match fs::symlink_metadata(&borrowed.into) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            symlink(&borrowed.from, &borrowed.into)?;
            Ok(Outcome::Created)
        }
        Err(error) => {
            Err(error).with_context(|| format!("could not inspect {}", borrowed.into.display()))
        }
        Ok(existing) if existing.is_symlink() => {
            let points_at = fs::read_link(&borrowed.into)
                .with_context(|| format!("could not read {}", borrowed.into.display()))?;
            if points_at == borrowed.from {
                return Ok(Outcome::Already);
            }
            // A link somebody else made is theirs, and pointing it at Ditto's
            // idea of the right place is the same overreach as overwriting a
            // directory would be.
            if !adopt {
                return Ok(Outcome::Kept);
            }
            fs::remove_file(&borrowed.into)
                .with_context(|| format!("could not replace {}", borrowed.into.display()))?;
            symlink(&borrowed.from, &borrowed.into)?;
            Ok(Outcome::Created)
        }
        Ok(_) if !adopt => Ok(Outcome::Kept),
        Ok(_) => {
            displace(&borrowed.into)?;
            symlink(&borrowed.from, &borrowed.into)?;
            Ok(Outcome::Created)
        }
    }
}

/// Moves what the profile already had out of the way under a name that says why
/// it moved, keeping earlier ones rather than landing on them.
fn displace(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("{} has no parent directory", path.display()))?;
    let name = path
        .file_name()
        .with_context(|| format!("{} does not name a file", path.display()))?
        .to_string_lossy()
        .into_owned();

    for attempt in 0.. {
        let suffix = if attempt == 0 {
            String::new()
        } else {
            format!(".{attempt}")
        };
        let aside = parent.join(format!("{name}.{DISPLACED}{suffix}"));
        if aside.exists() {
            continue;
        }
        return fs::rename(path, &aside)
            .with_context(|| format!("could not move {} to {}", path.display(), aside.display()));
    }
    unreachable!("the loop returns on the first name that is free")
}

#[cfg(unix)]
fn symlink(from: &Path, into: &Path) -> Result<()> {
    std::os::unix::fs::symlink(from, into)
        .with_context(|| format!("could not link {} to {}", into.display(), from.display()))
}

/// Windows tells directory links and file links apart, and refuses both to an
/// account without the privilege that Developer Mode grants. Saying so is
/// better than falling back to a copy: a copy would look like it worked and
/// then drift away from the configuration it was made from.
#[cfg(windows)]
fn symlink(from: &Path, into: &Path) -> Result<()> {
    let result = if from.is_dir() {
        std::os::windows::fs::symlink_dir(from, into)
    } else {
        std::os::windows::fs::symlink_file(from, into)
    };
    result.with_context(|| {
        format!(
            "could not link {} to {} (Windows allows this to an administrator, \
             or to any account with Developer Mode turned on)",
            into.display(),
            from.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn store(root: &Path) -> Store {
        Store::new(root.join("ditto"), root.join("home"))
    }

    fn given_directory(path: &Path, file: &str) {
        fs::create_dir_all(path).unwrap();
        fs::write(path.join(file), "yours").unwrap();
    }

    #[test]
    fn a_profile_reads_the_skills_you_already_had() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        given_directory(&source.claude_home.join("skills"), "humanizer.md");

        let target = store.create_profile("work").unwrap();
        let linked = link(&source, &target, false).unwrap();

        assert!(linked.changed());
        assert!(linked.linked.contains(&"claude/skills".to_owned()));
        assert_eq!(
            fs::read_to_string(target.claude_home.join("skills/humanizer.md")).unwrap(),
            "yours"
        );
    }

    /// A skill written after the profile was made has to be there too, which is
    /// the whole reason for a link rather than a copy.
    #[test]
    fn a_skill_added_later_shows_up_without_syncing() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        given_directory(&source.claude_home.join("skills"), "first.md");

        let target = store.create_profile("work").unwrap();
        link(&source, &target, false).unwrap();
        fs::write(source.claude_home.join("skills/second.md"), "later").unwrap();

        assert_eq!(
            fs::read_to_string(target.claude_home.join("skills/second.md")).unwrap(),
            "later"
        );
    }

    #[test]
    fn linking_twice_changes_nothing_the_second_time() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        given_directory(&source.claude_home.join("skills"), "one.md");
        let target = store.create_profile("work").unwrap();

        assert!(link(&source, &target, false).unwrap().changed());
        let again = link(&source, &target, false).unwrap();
        assert!(!again.changed());
        assert!(again.linked.contains(&"claude/skills".to_owned()));
    }

    /// The profiles that existed before Ditto linked anything have real
    /// directories where the links go, and creating a profile must not be the
    /// thing that throws one away.
    #[test]
    fn leaves_a_directory_the_profile_already_has() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        given_directory(&source.claude_home.join("plugins"), "yours.json");

        let target = store.create_profile("work").unwrap();
        given_directory(&target.claude_home.join("plugins"), "theirs.json");
        let linked = link(&source, &target, false).unwrap();

        assert!(linked.kept.contains(&"claude/plugins".to_owned()));
        assert!(target.claude_home.join("plugins/theirs.json").exists());
    }

    #[test]
    fn adopting_keeps_what_it_moved_aside() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        given_directory(&source.claude_home.join("plugins"), "yours.json");

        let target = store.create_profile("work").unwrap();
        given_directory(&target.claude_home.join("plugins"), "theirs.json");
        let linked = link(&source, &target, true).unwrap();

        assert!(linked.linked.contains(&"claude/plugins".to_owned()));
        assert!(target.claude_home.join("plugins/yours.json").exists());
        assert!(
            target
                .claude_home
                .join(format!("plugins.{DISPLACED}/theirs.json"))
                .exists()
        );
    }

    #[test]
    fn shares_nothing_you_do_not_have() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        let target = store.create_profile("work").unwrap();

        let linked = link(&source, &target, false).unwrap();

        assert!(!linked.changed());
        assert!(linked.linked.is_empty());
        assert!(!target.claude_home.join("skills").exists());
    }

    /// The credentials are the point. Nothing that carries an account may be on
    /// the list, however convenient sharing it would be.
    #[test]
    fn never_shares_anything_holding_an_account() {
        let account = [
            ".claude.json",
            "sessions",
            "projects",
            "history.jsonl",
            "auth.json",
            "account.json",
            "agent.db",
            "state",
        ];
        for name in account {
            assert!(
                !CLAUDE.contains(&name) && !CODEX.contains(&name) && !OMP.contains(&name),
                "'{name}' carries an account and must not be shared between profiles"
            );
        }
    }

    /// Deleting a profile removes the links and never follows them out to the
    /// configuration they point at. `remove_dir_all` unlinks a symbolic link
    /// rather than descending into it, and this is the test that says so before
    /// a future change quietly relies on the opposite.
    #[test]
    fn deleting_a_profile_leaves_your_own_configuration_alone() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        given_directory(&source.claude_home.join("skills"), "humanizer.md");

        let target = store.create_profile("work").unwrap();
        link(&source, &target, false).unwrap();
        store.delete_profile("work").unwrap();

        assert!(source.claude_home.join("skills/humanizer.md").exists());
    }
}
