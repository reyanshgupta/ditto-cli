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
//! Claude Code settings and Codex desktop project definitions are copied
//! instead of linked because their files also carry profile-specific state.
//! See [`crate::settings`] and [`crate::desktop`].
//! Codex's executable plugin cache is also copied, because its trusted loader
//! rejects code reached through a link outside the selected home. See
//! [`crate::codex_plugins`].
//!
//! Linking a directory has one cost, and [`repair`] is what pays it. See the
//! comment there.

use std::{
    fs, io,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result};

use crate::{
    launch::Tool,
    profile::{DEFAULT_PROFILE, Profile, Store, secure_directory, write_private_file},
    tools::{self, Home},
};

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
    "keybindings.json",
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
    "agents",
    "rules",
    "prompts",
    "config.toml",
    "hooks.json",
    "AGENTS.md",
    "instructions.md",
];

/// fx reads all local state below `$HOME/.fx`. These paths are reusable user
/// preferences and capabilities. Provider logins, MCP configuration and OAuth
/// credentials, sessions, prompt history, usage, traces, and recordings are
/// deliberately absent and remain in the selected profile.
const FX: &[&str] = &["settings.json", "AGENTS.md", "skills", "memories.json"];

/// The same for OMP, relative to the per-profile agent directory OMP keeps.
/// Its `agent.db` holds credentials beside sessions and never moves.
const OMP: &[&str] = &["config.yml", "extensions"];

/// Prime Agent's reusable capabilities and instructions, relative to its agent
/// directory. `models.json` is absent because custom providers may put literal
/// API keys and secret headers there; `auth.json` and all session state stay in
/// the profile for the same reason.
const PRIME_AGENT: &[&str] = &[
    "settings.json",
    "keybindings.json",
    "AGENTS.md",
    "CLAUDE.md",
    "SYSTEM.md",
    "APPEND_SYSTEM.md",
    "prompts",
    "skills",
    "extensions",
    "themes",
    "git",
    "harness",
];

/// Pi keeps credentials in `auth.json` and transcripts in `sessions`, while
/// `models.json` may contain literal keys and secret headers. Everything below
/// is reusable configuration, installed packages, or downloaded tooling.
const PI: &[&str] = &[
    "settings.json",
    "keybindings.json",
    "trust.json",
    "AGENTS.md",
    "CLAUDE.md",
    "SYSTEM.md",
    "APPEND_SYSTEM.md",
    "prompts",
    "skills",
    "extensions",
    "themes",
    "git",
    "npm",
    "bin",
];

/// What linking did, so a caller can say so rather than leaving it to be
/// noticed when a skill turns out to be missing.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Linked {
    /// Executable caches copied inside the profile for tools whose trusted
    /// loaders do not accept shared directory links.
    pub copied: Vec<String>,
    /// Local desktop projects copied without the account state stored beside them.
    pub desktop_projects: Vec<String>,
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
        self.changed || !self.desktop_projects.is_empty()
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
    link_selected(source, target, adopt, None, false)
}

/// An extension directory may first appear after a profile was created. Link
/// it before the tool starts, while preserving any copy the profile already
/// owns and avoiding work on unrelated tools' homes.
pub fn link_for(tool: Tool, source: &Profile, target: &Profile) -> Result<Linked> {
    link_selected(source, target, false, Some(tool), true)
}

fn link_selected(
    source: &Profile,
    target: &Profile,
    adopt: bool,
    tool: Option<Tool>,
    seed_only: bool,
) -> Result<Linked> {
    let mut result = Linked::default();
    if tool.is_none_or(|tool| tool == Tool::Codex) {
        match crate::codex_plugins::sync(source, target) {
            Ok(true) => {
                result.copied.push("codex/plugins/cache".to_owned());
                result.changed = true;
            }
            Ok(false) => {}
            Err(error) => result
                .failed
                .push(("codex/plugins/cache".to_owned(), format!("{error:#}"))),
        }
    }
    // Nothing the profile itself lives in is the user's configuration to
    // mirror: linked into the profile's private home, such a directory would
    // make the profile contain itself. Ditto's store is the obvious case, and
    // OMP's per-profile root under `~/.omp` the less obvious one.
    let holds_profile = target.directories();
    if tool.is_none_or(|tool| tool == Tool::Fx) {
        mirror_home(
            "fx/home",
            &source.fx_home,
            &target.fx_home,
            &[".fx"],
            &holds_profile,
            &mut result,
        );
    }
    for spec in tools::ALL {
        if tool.is_some_and(|tool| tool != Tool::Generic(spec)) {
            continue;
        }
        if let Home::Private { native, owned } = spec.home {
            let keep = std::iter::once(native)
                .chain(owned.iter().copied())
                .collect::<Vec<_>>();
            mirror_home(
                &format!("{}/home", spec.key),
                source.tool_home(spec),
                target.tool_home(spec),
                &keep,
                &holds_profile,
                &mut result,
            );
        }
    }
    for borrowed in plan(source, target, tool) {
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
    if tool.is_none_or(|tool| tool == Tool::Codex) {
        match crate::desktop::copy(source, target, seed_only) {
            Ok(projects) => result.desktop_projects = projects,
            Err(error) => result
                .failed
                .push(("codex/desktop-projects".to_owned(), format!("{error:#}"))),
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
        Ok(source) => link_selected(&source, profile, false, None, true).unwrap_or_default(),
        Err(_) => Linked::default(),
    }
}

/// Gives a tool's private HOME the same top-level view as the user's real
/// HOME, except for the tool's own directory. A tool with no state-root
/// override can only be isolated by changing HOME, and without this facade
/// that would also hide the user's shell startup files, Git configuration, SSH
/// setup, and toolchains from every command the agent runs.
///
/// `keep` names what is the tool's own and so is not mirrored: its directory,
/// and anything else in the home that carries its account. A kept path may be
/// nested, as `.config/manicode` is; the directory holding it is then made real
/// and mirrored one level further down, so that the one entry is the profile's
/// while the rest of `.config` stays the user's.
///
/// Existing entries are never replaced here, even under `--adopt`: they may
/// have been written while an older managed profile was active and are profile
/// state, not reusable configuration. Individual failures are reported, while
/// the many successful facade links are summarized as one `<label>` entry.
fn mirror_home(
    label: &str,
    from: &Path,
    into: &Path,
    keep: &[&str],
    holds_profile: &[&Path],
    result: &mut Linked,
) {
    if from == into {
        return;
    }
    let entries = match fs::read_dir(from) {
        Ok(entries) => entries,
        Err(error) => {
            result
                .failed
                .push((label.to_owned(), format!("could not read home: {error}")));
            return;
        }
    };

    let mut visible = false;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                result.failed.push((
                    label.to_owned(),
                    format!("could not read a home entry: {error}"),
                ));
                continue;
            }
        };
        if holds_profile
            .iter()
            .any(|directory| directory.starts_with(entry.path()))
        {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if keep.contains(&name.as_str()) {
            continue;
        }
        let entry_label = format!("{label}/{name}");
        let nested = keep
            .iter()
            .filter_map(|kept| kept.strip_prefix(name.as_str())?.strip_prefix('/'))
            .collect::<Vec<_>>();
        if !nested.is_empty() {
            let inner = into.join(entry.file_name());
            match create_parents(&inner) {
                Ok(()) => mirror_home(
                    &entry_label,
                    &entry.path(),
                    &inner,
                    &nested,
                    holds_profile,
                    result,
                ),
                Err(error) => result
                    .failed
                    .push((entry_label, format!("could not create: {error}"))),
            }
            continue;
        }
        let borrowed = Borrowed {
            label: entry_label.clone(),
            from: entry.path(),
            into: into.join(entry.file_name()),
        };
        match attach(&borrowed, false) {
            Ok(Outcome::Created) => {
                visible = true;
                result.changed = true;
            }
            Ok(Outcome::Already) => visible = true,
            Ok(Outcome::Kept) => {}
            Err(error) => result.failed.push((entry_label, format!("{error:#}"))),
        }
    }
    if visible {
        result.linked.push(label.to_owned());
    }
}

/// Every path the target borrows from the source, in the order they are
/// reported: one tool at a time, and the tools in the order they are listed
/// everywhere else.
///
/// Both sides come from [`paths`] rather than being written out twice, so the
/// two can only ever name the same list in the same order.
fn plan(source: &Profile, target: &Profile, tool: Option<Tool>) -> Vec<Borrowed> {
    paths(source)
        .into_iter()
        .zip(paths(target))
        .filter(|(borrowed, _)| tool.is_none_or(|tool| tool == borrowed.tool))
        .map(|(borrowed, own)| Borrowed {
            label: borrowed.label,
            from: borrowed.path,
            into: own.path,
        })
        .collect()
}

/// One path in the allowlist, as it is inside a single profile.
struct Owned {
    tool: Tool,
    label: String,
    path: PathBuf,
}

/// Every path in the allowlist as it is inside one profile: which tool reads
/// it, the name it is reported by, and where it lives.
fn paths(profile: &Profile) -> Vec<Owned> {
    let mut paths = Vec::new();
    for name in CLAUDE {
        paths.push(Owned {
            tool: Tool::Claude,
            label: format!("claude/{name}"),
            path: profile.claude_home.join(name),
        });
    }
    for name in CODEX {
        paths.push(Owned {
            tool: Tool::Codex,
            label: format!("codex/{name}"),
            path: profile.codex_home.join(name),
        });
    }
    for name in FX {
        paths.push(Owned {
            tool: Tool::Fx,
            label: format!("fx/{name}"),
            path: profile.fx_dir().join(name),
        });
    }
    // opencode keeps configuration and credentials in different XDG bases
    // already, so the whole configuration directory can be shared as one link
    // and the data directory holding `auth.json` stays untouched.
    paths.push(Owned {
        tool: Tool::Opencode,
        label: "opencode/config".to_owned(),
        path: profile.opencode.config_dir(),
    });
    for name in OMP {
        paths.push(Owned {
            tool: Tool::Omp,
            label: format!("omp/{name}"),
            path: profile.omp_home.join(name),
        });
    }
    for name in PRIME_AGENT {
        paths.push(Owned {
            tool: Tool::PrimeAgent,
            label: format!("prime-agent/{name}"),
            path: profile.prime_agent_home.join(name),
        });
    }
    for name in PI {
        paths.push(Owned {
            tool: Tool::Pi,
            label: format!("pi/{name}"),
            path: profile.pi_home.join(name),
        });
    }
    for spec in tools::ALL {
        for name in spec.shared {
            paths.push(Owned {
                tool: Tool::Generic(spec),
                label: format!("{}/{name}", spec.key),
                path: profile.tool_path(spec, name),
            });
        }
    }
    paths
}

/// How deep a shared directory is searched for links to mend.
///
/// A tool writes into the directory it was handed, so the link is usually an
/// entry of that directory. opencode is why this is not one: Ditto shares its
/// whole configuration directory, which puts an installed skill three levels
/// down at `opencode/skills/<name>`. Deeper than that is the tool's own
/// storage — `claude/plugins` alone holds a git checkout per marketplace — and
/// walking it at every launch would cost more than the repair is worth.
const SEARCH_DEPTH: usize = 3;

/// What repairing did, so a launch can say why something appeared.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Repaired {
    /// Links or plugin registries now pointing where they were installed to
    /// point, named by their path inside the profile.
    pub links: Vec<String>,
    /// Links that could not be rewritten, each with the reason.
    pub failed: Vec<(String, String)>,
}

impl Repaired {
    pub fn changed(&self) -> bool {
        !self.links.is_empty()
    }
}

/// Points links a tool wrote through one of Ditto's back at what they meant.
///
/// Sharing a directory by linking it has one cost. A tool is handed a
/// configuration directory whose `skills` is a link to the user's own, and an
/// installer that puts something there and records it as a *relative* link
/// computes that link from the path it was given. The kernel then creates the
/// link in the directory the link leads to, which sits at a different depth, so
/// the relative path lands on nothing. That is what a skill installer does, and
/// it is why a downloaded skill can be missing from every profile at once while
/// the skill itself is on disk and perfectly fine.
///
/// Ditto is what moved the directory, so Ditto is what can say where the link
/// meant to go: read its target from the path the tool was handed rather than
/// from where the link ended up. Nothing is rewritten unless reading it that way
/// names something that exists, so a link that is relative and broken for its
/// own reasons is reported by nobody and left alone.
pub fn repair(profile: &Profile) -> Repaired {
    let mut result = mend(paths(profile));
    repair_plugin_registry(profile, &mut result);
    result
}

/// The same for one tool, which is all a launch of that tool can be about to
/// read.
pub fn repair_for(tool: Tool, profile: &Profile) -> Repaired {
    let mut result = mend(
        paths(profile)
            .into_iter()
            .filter(|owned| owned.tool == tool),
    );
    if tool == Tool::Claude {
        repair_plugin_registry(profile, &mut result);
    }
    result
}

fn repair_plugin_registry(profile: &Profile, result: &mut Repaired) {
    let label = "claude/plugins/installed_plugins.json".to_owned();
    match preserve_claude_plugins(profile) {
        Ok(true) => result.links.push(label),
        Ok(false) => {}
        Err(error) => result.failed.push((label, format!("{error:#}"))),
    }
}

/// Claude records installation paths through the directory it was handed,
/// even when the plugin cache is shared. An absolute path through a profile
/// stops working after rename or deletion, so keep the registry pointing at
/// the cache's real location before moving either profile root.
pub fn preserve_claude_plugins(profile: &Profile) -> Result<bool> {
    let plugins = profile.claude_home.join("plugins");
    if !profile.managed || !fs::symlink_metadata(&plugins).is_ok_and(|entry| entry.is_symlink()) {
        return Ok(false);
    }
    let registry = plugins.join("installed_plugins.json");
    let contents = match fs::read_to_string(&registry) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| format!("could not read {}", registry.display()));
        }
    };
    let mut value: serde_json::Value = serde_json::from_str(&contents)
        .with_context(|| format!("could not parse {}", registry.display()))?;
    let Some(entries) = value
        .get_mut("plugins")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return Ok(false);
    };
    let shared = fs::canonicalize(&plugins)
        .with_context(|| format!("could not resolve {}", plugins.display()))?;
    let Some(profiles_root) = profile.claude_home.parent().and_then(Path::parent) else {
        return Ok(false);
    };
    let mut changed = false;
    for installations in entries
        .values_mut()
        .filter_map(serde_json::Value::as_array_mut)
    {
        for installation in installations {
            let Some(installed) = installation
                .get("installPath")
                .and_then(serde_json::Value::as_str)
            else {
                continue;
            };
            let Ok(relative) = Path::new(installed).strip_prefix(profiles_root) else {
                continue;
            };
            let mut components = relative.components();
            if !matches!(components.next(), Some(Component::Normal(_))) {
                continue;
            }
            let Ok(cache_path) = components.as_path().strip_prefix("claude/plugins") else {
                continue;
            };
            // A stale entry can name an older profile. Recover only a path
            // whose files still exist inside this same shared plugin cache.
            let Ok(actual) = fs::canonicalize(shared.join(cache_path)) else {
                continue;
            };
            if !actual.starts_with(&shared) {
                continue;
            }
            installation["installPath"] = serde_json::Value::String(actual.display().to_string());
            changed = true;
        }
    }
    if changed {
        let mut contents = serde_json::to_string_pretty(&value)?;
        contents.push('\n');
        write_private_file(&registry, &contents)?;
    }
    Ok(changed)
}

fn mend(paths: impl IntoIterator<Item = Owned>) -> Repaired {
    let mut result = Repaired::default();
    for owned in paths {
        // Only a directory Ditto redirected can hold one of these. Where the
        // profile kept its own, the path a tool was handed is the path it wrote
        // to, and a relative link resolves the way its author meant.
        if fs::symlink_metadata(&owned.path).is_ok_and(|entry| entry.is_symlink()) {
            search(&owned.label, &owned.path, SEARCH_DEPTH, &mut result);
        }
    }
    result
}

/// Walks a shared directory by the path the profile knows it as, which is the
/// path a tool was given and the only one its relative links can be read
/// against.
fn search(label: &str, directory: &Path, depth: usize, result: &mut Repaired) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        let label = format!("{label}/{}", entry.file_name().to_string_lossy());
        if kind.is_symlink() {
            let Some(meant) = intended(&path) else {
                continue;
            };
            match relink(&path, &meant) {
                Ok(()) => result.links.push(label),
                Err(error) => result.failed.push((label, format!("{error:#}"))),
            }
        } else if kind.is_dir() && depth > 1 {
            search(&label, &path, depth - 1, result);
        }
    }
}

/// Where a link was meant to point, if it is one of the links this repairs.
fn intended(link: &Path) -> Option<PathBuf> {
    let target = fs::read_link(link).ok()?;
    // An absolute link says where it goes and got there whatever Ditto did, so
    // one that leads nowhere is broken for a reason of its own.
    if target.is_absolute() {
        return None;
    }
    // `exists` follows the link from where it really is, which is the reading
    // that failed. A link that survives it means what it says.
    if link.exists() {
        return None;
    }
    let meant = resolve(&link.parent()?.join(target));
    meant.exists().then_some(meant)
}

/// Resolves `.` and `..` without asking the filesystem.
///
/// `canonicalize` is the wrong tool here: it follows the very link that moved
/// the directory, and would answer with where the link *is* rather than where
/// the tool believed it was writing. That difference is the whole repair.
fn resolve(path: &Path) -> PathBuf {
    let mut resolved = PathBuf::new();
    for component in path.components() {
        match component {
            // A `..` with nothing left to remove is the tool climbing past the
            // root, which is exactly how the broken link got there. Staying at
            // the root matches what the kernel does with the same path.
            Component::ParentDir => {
                resolved.pop();
            }
            Component::CurDir => {}
            component => resolved.push(component),
        }
    }
    resolved
}

/// Rewrites the link as an absolute one, so it reads the same from the profile
/// and from the user's own configuration and cannot break again the next time a
/// profile is added.
fn relink(link: &Path, target: &Path) -> Result<()> {
    remove_link(link).with_context(|| format!("could not replace {}", link.display()))?;
    symlink(target, link)
}

#[cfg(unix)]
fn remove_link(link: &Path) -> io::Result<()> {
    fs::remove_file(link)
}

/// Windows records whether a link was made to a file or to a directory and
/// refuses `remove_file` for the directory kind, so both have to be offered.
#[cfg(windows)]
fn remove_link(link: &Path) -> io::Result<()> {
    fs::remove_file(link).or_else(|_| fs::remove_dir(link))
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
            if let Some(parent) = borrowed.into.parent() {
                create_parents(parent)?;
            }
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
            adopt_path(borrowed)
        }
        Ok(_) if !adopt => Ok(Outcome::Kept),
        Ok(_) => adopt_path(borrowed),
    }
}

/// Nested allowlist entries need parents that a fresh CLI has not created yet.
/// Only new directories are secured: an existing parent may be the user's own
/// directory reached through a link, whose permissions are theirs to choose.
fn create_parents(path: &Path) -> Result<()> {
    if path.is_dir() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        create_parents(parent)?;
    }
    match fs::create_dir(path) {
        Ok(()) => secure_directory(path),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists && path.is_dir() => Ok(()),
        Err(error) => Err(error).with_context(|| format!("could not create {}", path.display())),
    }
}

fn adopt_path(borrowed: &Borrowed) -> Result<Outcome> {
    let aside = displace(&borrowed.into)?;
    if let Err(error) = symlink(&borrowed.from, &borrowed.into) {
        // Windows can allow the rename but refuse the link. Put the original
        // back so a failed sync does not hide previously working capabilities.
        fs::rename(&aside, &borrowed.into).with_context(|| {
            format!(
                "{error:#}; could not restore {}; the original remains at {}",
                borrowed.into.display(),
                aside.display()
            )
        })?;
        return Err(error);
    }
    Ok(Outcome::Created)
}

/// Moves what the profile already had out of the way under a name that says why
/// it moved, keeping earlier ones rather than landing on them.
fn displace(path: &Path) -> Result<PathBuf> {
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
        match fs::symlink_metadata(&aside) {
            Ok(_) => continue,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("could not inspect {}", aside.display()));
            }
        }
        fs::rename(path, &aside)
            .with_context(|| format!("could not move {} to {}", path.display(), aside.display()))?;
        return Ok(aside);
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

    /// The path `to` is written as from `from`, which is what an installer
    /// computes before recording where it put something.
    fn relative(from: &Path, to: &Path) -> PathBuf {
        let mut from = from.components().peekable();
        let mut to = to.components().peekable();
        while from.peek().is_some() && from.peek() == to.peek() {
            from.next();
            to.next();
        }

        let mut relative = PathBuf::new();
        for _ in from {
            relative.push("..");
        }
        relative.extend(to);
        relative
    }

    /// Records an installed skill the way a skill installer does: a relative
    /// link, computed from the directory the tool was handed rather than from
    /// the directory that link leads to.
    fn installed_through(handed: &Path, name: &str, skill: &Path) {
        symlink(&relative(handed, skill), &handed.join(name)).unwrap();
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

    #[test]
    fn syncing_and_adopting_keep_codex_plugin_code_local() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        given_directory(
            &source.codex_home.join("plugins/cache/browser"),
            "service.mjs",
        );
        let target = store.create_profile("work").unwrap();
        let first = link(&source, &target, false).unwrap();
        assert_eq!(first.copied, ["codex/plugins/cache"]);
        let second = link(&source, &target, true).unwrap();
        assert!(second.copied.is_empty());
        assert!(
            !fs::symlink_metadata(target.codex_home.join("plugins"))
                .unwrap()
                .is_symlink()
        );
        assert!(
            fs::canonicalize(target.codex_home.join("plugins/cache/browser/service.mjs"))
                .unwrap()
                .starts_with(fs::canonicalize(&target.codex_home).unwrap())
        );
    }

    #[test]
    fn every_shared_path_survives_creation_sync_rename_and_deletion() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        for owned in paths(&source) {
            if owned.path.extension().is_some() {
                fs::create_dir_all(owned.path.parent().unwrap()).unwrap();
                fs::write(&owned.path, "original").unwrap();
            } else {
                given_directory(&owned.path, "capability.txt");
            }
        }
        for spec in tools::ALL {
            for name in spec.credentials.iter().chain(spec.sessions) {
                given_directory(&source.tool_path(spec, name), "private.txt");
            }
        }
        let target = store.create_profile("work").unwrap();
        let linked = link(&source, &target, false).unwrap();
        assert!(linked.failed.is_empty(), "{:?}", linked.failed);
        for spec in tools::ALL {
            for name in spec.credentials.iter().chain(spec.sessions) {
                assert!(
                    !target.tool_path(spec, name).join("private.txt").exists(),
                    "{}/{name} leaked account state",
                    spec.key
                );
            }
        }
        for borrowed in plan(&source, &target, None) {
            assert_eq!(
                fs::canonicalize(&borrowed.into).unwrap(),
                fs::canonicalize(&borrowed.from).unwrap(),
                "{}",
                borrowed.label
            );
        }
        assert!(!link(&source, &target, false).unwrap().changed());
        let renamed = store.rename_profile("work", "client").unwrap();
        for borrowed in plan(&source, &renamed, None) {
            assert!(borrowed.into.exists(), "{}", borrowed.label);
        }
        store.delete_profile("client").unwrap();
        for owned in paths(&source) {
            assert!(owned.path.exists(), "{}", owned.label);
        }
    }

    #[test]
    fn updates_to_every_shared_path_reach_existing_profiles_without_sync() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        for owned in paths(&source) {
            if owned.path.extension().is_some() {
                fs::create_dir_all(owned.path.parent().unwrap()).unwrap();
                fs::write(&owned.path, "version 1").unwrap();
            } else {
                given_directory(&owned.path, "version.txt");
            }
        }
        let first = store.create_profile("work").unwrap();
        let second = store.create_profile("personal").unwrap();
        for target in [&first, &second] {
            assert!(link(&source, target, false).unwrap().failed.is_empty());
        }

        // Updaters commonly replace files or whole version directories instead
        // of editing them in place. Links must follow the stable source path.
        for owned in paths(&source) {
            if owned.path.is_file() {
                write_private_file(&owned.path, "version 2").unwrap();
            } else {
                let previous = owned.path.with_extension("previous");
                fs::rename(&owned.path, &previous).unwrap();
                fs::create_dir(&owned.path).unwrap();
                fs::write(owned.path.join("version.txt"), "version 2").unwrap();
                fs::remove_dir_all(previous).unwrap();
            }
        }
        for target in [&first, &second] {
            for owned in paths(target) {
                let file = if owned.path.is_dir() {
                    owned.path.join("version.txt")
                } else {
                    owned.path
                };
                assert_eq!(
                    fs::read_to_string(file).unwrap(),
                    "version 2",
                    "{}",
                    owned.label
                );
            }
        }
    }

    #[test]
    fn an_upstream_writer_replacing_a_shared_file_is_reported_as_a_local_copy() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        fs::create_dir_all(&source.prime_agent_home).unwrap();
        let native = source.prime_agent_home.join("settings.json");
        fs::write(&native, r#"{"packages":["fixture@1"]}"#).unwrap();
        let target = store.create_profile("work").unwrap();
        link(&source, &target, false).unwrap();

        // Prime Agent's atomic save replaces the link itself. Recovering by
        // overwriting either copy would discard a choice the user just made.
        let local = target.prime_agent_home.join("settings.json");
        write_private_file(&local, r#"{"packages":["local@2"]}"#).unwrap();
        write_private_file(&native, r#"{"packages":["fixture@3"]}"#).unwrap();
        let linked = link(&source, &target, false).unwrap();

        assert!(
            linked
                .kept
                .contains(&"prime-agent/settings.json".to_owned())
        );
        assert_eq!(
            fs::read_to_string(local).unwrap(),
            r#"{"packages":["local@2"]}"#
        );
        assert_eq!(
            fs::read_to_string(native).unwrap(),
            r#"{"packages":["fixture@3"]}"#
        );
    }

    #[test]
    fn a_codex_profile_keeps_the_custom_agents_its_config_references() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        given_directory(&source.codex_home.join("agents"), "reviewer.toml");
        fs::write(
            source.codex_home.join("config.toml"),
            "[agents.reviewer]\nconfig_file = 'agents/reviewer.toml'\n",
        )
        .unwrap();
        let target = store.create_profile("work").unwrap();
        link(&source, &target, false).unwrap();
        assert_eq!(
            fs::read_to_string(target.codex_home.join("agents/reviewer.toml")).unwrap(),
            "yours"
        );
    }

    #[test]
    fn claude_plugins_installed_through_a_profile_survive_rename_and_deletion() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        let cache = "plugins/cache/local/fixture/1.0.0";
        given_directory(&source.claude_home.join(cache), "plugin.json");
        let target = store.create_profile("work").unwrap();
        link(&source, &target, false).unwrap();
        let registry = source.claude_home.join("plugins/installed_plugins.json");
        let mut contents = serde_json::json!({
            "version": 2,
            "plugins": {"fixture@local": [{
                "scope": "user", "installPath": target.claude_home.join(cache),
                "version": "1.0.0", "unknownFutureField": true
            }]}
        });
        fs::write(&registry, contents.to_string()).unwrap();
        let renamed = store.rename_profile("work", "client").unwrap();
        contents["plugins"]["fixture@local"][0]["installPath"] =
            serde_json::json!(fs::canonicalize(source.claude_home.join(cache)).unwrap());
        let read = || {
            serde_json::from_str::<serde_json::Value>(&fs::read_to_string(&registry).unwrap())
                .unwrap()
        };
        assert_eq!(read(), contents);

        let mut installed_after_rename = contents.clone();
        installed_after_rename["plugins"]["fixture@local"][0]["installPath"] =
            serde_json::json!(renamed.claude_home.join(cache));
        fs::write(&registry, installed_after_rename.to_string()).unwrap();
        store.delete_profile("client").unwrap();
        assert_eq!(read(), contents);
        assert!(source.claude_home.join(cache).join("plugin.json").exists());
    }

    #[test]
    fn repairs_stale_claude_plugin_paths_without_changing_unrelated_records() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        let cache = "plugins/cache/local/fixture/1.0.0";
        given_directory(&source.claude_home.join(cache), "plugin.json");
        let target = store.create_profile("work").unwrap();
        link(&source, &target, false).unwrap();
        let registry = source.claude_home.join("plugins/installed_plugins.json");
        let unrelated = temporary.path().join("unrelated-plugin");
        given_directory(&unrelated, "plugin.json");
        let mut contents = serde_json::json!({"version": 2, "plugins": {
            "fixture@local": [{"installPath": store.root().join("profiles/deleted/claude").join(cache)}],
            "unrelated@local": [{"installPath": unrelated}],
            "missing@local": [{"installPath": target.claude_home.join("plugins/cache/missing")}]
        }});
        fs::write(&registry, contents.to_string()).unwrap();
        let repaired = repair_for(Tool::Claude, &target);
        assert!(repaired.failed.is_empty());
        assert_eq!(repaired.links, ["claude/plugins/installed_plugins.json"]);
        contents["plugins"]["fixture@local"][0]["installPath"] =
            serde_json::json!(fs::canonicalize(source.claude_home.join(cache)).unwrap());
        let actual: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&registry).unwrap()).unwrap();
        assert_eq!(actual, contents);
        assert!(!repair_for(Tool::Claude, &target).changed());
    }

    #[test]
    fn adopting_preserves_foreign_links_and_dangling_backups() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        given_directory(&source.claude_home.join("plugins"), "shared.json");
        let target = store.create_profile("work").unwrap();
        let private = target.claude_home.join("private-plugins");
        given_directory(&private, "local.json");
        symlink(&private, &target.claude_home.join("plugins")).unwrap();
        let old_backup = target.claude_home.join("plugins.before-ditto");
        symlink(&private, &old_backup).unwrap();
        fs::remove_dir_all(&private).unwrap();

        link(&source, &target, true).unwrap();

        assert_eq!(fs::read_link(&old_backup).unwrap(), private);
        assert_eq!(
            fs::read_link(target.claude_home.join("plugins.before-ditto.1")).unwrap(),
            private
        );
        assert!(target.claude_home.join("plugins/shared.json").exists());
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
    fn launching_links_a_skills_directory_created_after_the_profile() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        let target = store.create_profile("work").unwrap();
        link(&source, &target, false).unwrap();
        given_directory(&source.claude_home.join("skills"), "new.md");
        given_directory(&source.codex_home.join("skills"), "other.md");

        let linked = link_for(Tool::Claude, &source, &target).unwrap();

        assert_eq!(linked.linked, ["claude/skills"]);
        assert!(target.claude_home.join("skills/new.md").exists());
        assert!(!target.codex_home.join("skills").exists());
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
            "chatgpt-auth.json",
            "grok-auth.json",
            "api-key",
            "mcp.json",
            "mcp-credentials",
            "oauth.json",
            "account.json",
            "agent.db",
            "state",
            "session-artifacts",
            "session-leases",
            "cron-jobs.json",
            "models.json",
        ];
        for name in account {
            assert!(
                !CLAUDE.contains(&name)
                    && !CODEX.contains(&name)
                    && !FX.contains(&name)
                    && !OMP.contains(&name)
                    && !PRIME_AGENT.contains(&name)
                    && !PI.contains(&name)
                    && !tools::ALL.iter().any(|spec| spec.shared.contains(&name)),
                "'{name}' carries an account and must not be shared between profiles"
            );
        }
    }

    #[test]
    fn a_pi_profile_reads_the_global_skills() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        given_directory(&source.pi_home.join("skills"), "profile-test.md");

        let target = store.create_profile("work").unwrap();
        let linked = link(&source, &target, false).unwrap();

        assert!(linked.linked.contains(&"pi/skills".to_owned()));
        assert_eq!(
            fs::read_to_string(target.pi_home.join("skills/profile-test.md")).unwrap(),
            "yours"
        );
    }

    #[test]
    fn a_prime_agent_profile_reads_the_global_harness() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        given_directory(
            &source.prime_agent_home.join("harness"),
            "harness_state.json",
        );

        let target = store.create_profile("work").unwrap();
        let linked = link(&source, &target, false).unwrap();

        assert!(linked.linked.contains(&"prime-agent/harness".to_owned()));
        assert_eq!(
            fs::read_to_string(target.prime_agent_home.join("harness/harness_state.json")).unwrap(),
            "yours"
        );
    }

    /// The failure [`repair`] exists for. An installer puts the skill in one
    /// place and records it in the directory Ditto handed the tool as a link
    /// computed from that path; the kernel writes the link into the user's own
    /// directory instead, where the same path leads somewhere else entirely.
    #[test]
    fn a_link_installed_through_ours_is_pointed_back_at_the_skill() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        given_directory(&source.claude_home.join("skills"), "yours.md");
        let skill = temporary.path().join("home/.agents/skills/apple-design");
        given_directory(&skill, "SKILL.md");

        let target = store.create_profile("work").unwrap();
        link(&source, &target, false).unwrap();
        let handed = target.claude_home.join("skills");
        installed_through(&handed, "apple-design", &skill);
        assert!(!handed.join("apple-design").exists());

        let repaired = repair(&target);

        assert!(repaired.changed());
        assert_eq!(repaired.links, ["claude/skills/apple-design"]);
        assert_eq!(
            fs::read_to_string(handed.join("apple-design/SKILL.md")).unwrap(),
            "yours"
        );
        // The link really is in the user's own configuration, which is where
        // every other profile and an unproxied tool will read it from.
        assert!(
            source
                .claude_home
                .join("skills/apple-design/SKILL.md")
                .exists()
        );
    }

    /// opencode has no directory of its own for extensions, so Ditto shares the
    /// whole configuration directory and an installed skill lands below the
    /// link rather than inside it.
    #[test]
    fn repairs_a_link_written_below_the_directory_a_tool_was_given() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        given_directory(&source.opencode.config_dir(), "opencode.json");
        let skill = temporary.path().join("home/.agents/skills/apple-design");
        given_directory(&skill, "SKILL.md");

        let target = store.create_profile("work").unwrap();
        link(&source, &target, false).unwrap();
        let handed = target.opencode.config_dir().join("skills");
        fs::create_dir_all(&handed).unwrap();
        installed_through(&handed, "apple-design", &skill);

        let repaired = repair(&target);

        assert_eq!(repaired.links, ["opencode/config/skills/apple-design"]);
        assert!(handed.join("apple-design/SKILL.md").exists());
    }

    #[test]
    fn repairs_only_the_tool_a_launch_is_about_to_start() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        given_directory(&source.claude_home.join("skills"), "yours.md");
        given_directory(&source.codex_home.join("skills"), "yours.md");
        let skill = temporary.path().join("home/.agents/skills/apple-design");
        given_directory(&skill, "SKILL.md");

        let target = store.create_profile("work").unwrap();
        link(&source, &target, false).unwrap();
        let claude = target.claude_home.join("skills");
        let codex = target.codex_home.join("skills");
        installed_through(&claude, "apple-design", &skill);
        installed_through(&codex, "apple-design", &skill);

        let repaired = repair_for(Tool::Codex, &target);

        assert_eq!(repaired.links, ["codex/skills/apple-design"]);
        assert!(codex.join("apple-design").exists());
        assert!(!claude.join("apple-design").exists());
    }

    /// A link that reads correctly means what it says, wherever its author
    /// computed it from, and rewriting one would be Ditto tidying somebody
    /// else's configuration for them.
    #[test]
    fn leaves_a_link_that_already_reads_correctly_alone() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        let skills = source.claude_home.join("skills");
        given_directory(&skills, "yours.md");
        let skill = temporary.path().join("home/.agents/skills/apple-design");
        given_directory(&skill, "SKILL.md");
        // Written from the user's own directory, which is what an installer run
        // outside Ditto produces.
        installed_through(&skills, "apple-design", &skill);

        let target = store.create_profile("work").unwrap();
        link(&source, &target, false).unwrap();
        let repaired = repair(&target);

        assert!(!repaired.changed());
        assert_eq!(
            fs::read_link(skills.join("apple-design")).unwrap(),
            relative(&skills, &skill)
        );
    }

    #[test]
    fn leaves_broken_links_it_did_not_cause_alone() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        given_directory(&source.claude_home.join("skills"), "yours.md");

        let target = store.create_profile("work").unwrap();
        link(&source, &target, false).unwrap();
        let handed = target.claude_home.join("skills");
        // An absolute link says where it goes and got there whatever Ditto did.
        symlink(Path::new("/nowhere/at/all"), &handed.join("absolute")).unwrap();
        // A relative one that leads nowhere from the profile either was not
        // written against the path Ditto handed out.
        symlink(Path::new("../../nowhere"), &handed.join("relative")).unwrap();

        let repaired = repair(&target);

        assert!(!repaired.changed());
        assert!(repaired.failed.is_empty());
        assert_eq!(
            fs::read_link(handed.join("absolute")).unwrap(),
            Path::new("/nowhere/at/all")
        );
        assert_eq!(
            fs::read_link(handed.join("relative")).unwrap(),
            Path::new("../../nowhere")
        );
    }

    /// Where the profile kept a directory of its own, the path a tool was handed
    /// is the path it wrote to, so nothing there was computed against a link and
    /// nothing there is Ditto's to rewrite.
    #[test]
    fn leaves_a_directory_the_profile_owns_alone() {
        let temporary = tempdir().unwrap();
        let store = store(temporary.path());
        let source = store.load_profile(DEFAULT_PROFILE).unwrap();
        given_directory(&source.claude_home.join("skills"), "yours.md");
        let skill = temporary.path().join("home/.agents/skills/apple-design");
        given_directory(&skill, "SKILL.md");

        let target = store.create_profile("work").unwrap();
        let handed = target.claude_home.join("skills");
        given_directory(&handed, "theirs.md");
        link(&source, &target, false).unwrap();
        symlink(Path::new("../../nowhere"), &handed.join("apple-design")).unwrap();

        assert!(!repair(&target).changed());
        assert_eq!(
            fs::read_link(handed.join("apple-design")).unwrap(),
            Path::new("../../nowhere")
        );
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
