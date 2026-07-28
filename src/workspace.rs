use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::profile::{Store, validate_profile_name, write_private_file};

/// The per-directory file naming the profile a project launches with. It sits
/// at the project root beside the other dotfiles a repository carries and is
/// meant to be read and committed, so it is written with the ordinary file mode
/// rather than the owner-only one the profile store uses for credentials.
pub const WORKSPACE_FILE: &str = ".ditto.toml";

const REGISTRY_FILE: &str = "workspaces.json";

#[derive(Deserialize, Serialize)]
struct WorkspaceFile {
    profile: String,
}

/// Directories bound without a file of their own, for projects Ditto should not
/// leave a file in.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct Registry {
    /// Absolute directory to profile name. Ordered, so the file reads the same
    /// way twice and an edit by hand does not reshuffle everything around it.
    #[serde(default)]
    workspaces: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Origin {
    File,
    Registry,
}

#[derive(Clone, Debug)]
pub struct Binding {
    pub profile: String,
    /// The directory the binding was found in, which is an ancestor of the
    /// directory searched from whenever the search started further down.
    pub directory: PathBuf,
    pub origin: Origin,
}

impl Binding {
    /// Names the binding by where it is written, since that is what a user has
    /// to open to change it.
    pub fn describe_origin(&self) -> String {
        match self.origin {
            Origin::File => self.directory.join(WORKSPACE_FILE).display().to_string(),
            Origin::Registry => format!("{} in {REGISTRY_FILE}", self.directory.display()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Workspaces {
    registry_path: PathBuf,
    user_home: PathBuf,
}

impl Workspaces {
    pub fn new(store: &Store) -> Self {
        Self {
            registry_path: store.root().join(REGISTRY_FILE),
            user_home: store.user_home().to_path_buf(),
        }
    }

    pub fn registry_path(&self) -> &Path {
        &self.registry_path
    }

    /// The binding that applies to `directory`, found by walking towards the
    /// filesystem root. The nearest directory wins, and within one directory a
    /// committed file outranks the registry, so a project can carry a binding
    /// that overrides whatever this machine recorded for it.
    pub fn find(&self, directory: &Path) -> Result<Option<Binding>> {
        let start = canonical(directory);
        let registry = self.read_registry()?;

        for ancestor in start.ancestors() {
            if let Some(profile) = read_workspace_file(ancestor)? {
                return Ok(Some(Binding {
                    profile,
                    directory: ancestor.to_path_buf(),
                    origin: Origin::File,
                }));
            }

            if let Some(profile) = registry.workspaces.get(&key(ancestor)) {
                validate_profile_name(profile).with_context(|| {
                    format!(
                        "{} names an invalid profile for {}",
                        self.registry_path.display(),
                        ancestor.display()
                    )
                })?;
                return Ok(Some(Binding {
                    profile: profile.clone(),
                    directory: ancestor.to_path_buf(),
                    origin: Origin::Registry,
                }));
            }
        }

        Ok(None)
    }

    pub fn bind_file(&self, directory: &Path, profile: &str) -> Result<PathBuf> {
        validate_profile_name(profile)?;
        let path = canonical(directory).join(WORKSPACE_FILE);
        let body = toml::to_string(&WorkspaceFile {
            profile: profile.to_owned(),
        })
        .context("could not serialize the workspace file")?;
        let contents = format!(
            "# Written by ditto-cli. Names the profile this directory launches with.\n{body}"
        );

        fs::write(&path, contents)
            .with_context(|| format!("could not write {}", path.display()))?;
        Ok(path)
    }

    pub fn bind_registry(&self, directory: &Path, profile: &str) -> Result<PathBuf> {
        validate_profile_name(profile)?;
        let directory = canonical(directory);
        let mut registry = self.read_registry()?;
        registry
            .workspaces
            .insert(key(&directory), profile.to_owned());
        self.write_registry(&registry)?;
        Ok(directory)
    }

    /// Removes both kinds of binding for exactly this directory and reports
    /// what was there. Ancestors are left alone: clearing one directory should
    /// never reach outside it and unbind a whole tree.
    pub fn clear(&self, directory: &Path) -> Result<Vec<String>> {
        let directory = canonical(directory);
        let mut removed = Vec::new();

        let path = directory.join(WORKSPACE_FILE);
        if path.is_file() {
            fs::remove_file(&path)
                .with_context(|| format!("could not remove {}", path.display()))?;
            removed.push(path.display().to_string());
        }

        let mut registry = self.read_registry()?;
        if registry.workspaces.remove(&key(&directory)).is_some() {
            self.write_registry(&registry)?;
            removed.push(format!("{} in {REGISTRY_FILE}", directory.display()));
        }

        Ok(removed)
    }

    pub fn entries(&self) -> Result<Vec<(PathBuf, String)>> {
        Ok(self
            .read_registry()?
            .workspaces
            .into_iter()
            .map(|(directory, profile)| (PathBuf::from(directory), profile))
            .collect())
    }

    /// Whether a launch may write a workspace file into this directory on its
    /// own. The home directory and the filesystem root are refused: a file at
    /// either binds every project underneath it, which is far more than the one
    /// launch that would have written it was asking for.
    pub fn may_auto_bind(&self, directory: &Path) -> bool {
        let directory = canonical(directory);
        directory.parent().is_some() && directory != canonical(&self.user_home)
    }

    fn read_registry(&self) -> Result<Registry> {
        if !self.registry_path.is_file() {
            return Ok(Registry::default());
        }

        let contents = fs::read_to_string(&self.registry_path)
            .with_context(|| format!("could not read {}", self.registry_path.display()))?;
        if contents.trim().is_empty() {
            return Ok(Registry::default());
        }

        serde_json::from_str(&contents)
            .with_context(|| format!("could not parse {}", self.registry_path.display()))
    }

    fn write_registry(&self, registry: &Registry) -> Result<()> {
        let contents = serde_json::to_string_pretty(registry)
            .context("could not serialize the workspace registry")?;
        write_private_file(&self.registry_path, &format!("{contents}\n"))
    }
}

fn read_workspace_file(directory: &Path) -> Result<Option<String>> {
    let path = directory.join(WORKSPACE_FILE);
    if !path.is_file() {
        return Ok(None);
    }

    let contents =
        fs::read_to_string(&path).with_context(|| format!("could not read {}", path.display()))?;
    let file: WorkspaceFile =
        toml::from_str(&contents).with_context(|| format!("could not parse {}", path.display()))?;

    // The name reaches the profile store, which builds paths from it, so a file
    // anyone can edit is checked here rather than trusted.
    validate_profile_name(&file.profile)
        .with_context(|| format!("{} names an invalid profile", path.display()))?;
    Ok(Some(file.profile))
}

/// Resolving symlinks keeps one directory from being bound twice under two
/// names. A path that cannot be resolved is used as it was given, because the
/// operation that needs the directory reports a missing one far better than a
/// lookup that quietly found nothing.
fn canonical(directory: &Path) -> PathBuf {
    fs::canonicalize(directory).unwrap_or_else(|_| directory.to_path_buf())
}

fn key(directory: &Path) -> String {
    directory.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspaces(root: &Path, user_home: &Path) -> Workspaces {
        Workspaces::new(&Store::new(root.to_path_buf(), user_home.to_path_buf()))
    }

    #[test]
    fn finds_the_nearest_workspace_file_from_a_subdirectory() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let home = temporary.path().join("home");
        fs::create_dir_all(&home)?;
        let workspaces = workspaces(&temporary.path().join("ditto"), &home);

        let project = home.join("project");
        let nested = project.join("crates").join("core");
        fs::create_dir_all(&nested)?;
        workspaces.bind_file(&project, "work")?;

        let binding = workspaces.find(&nested)?.expect("a binding");
        assert_eq!(binding.profile, "work");
        assert_eq!(binding.origin, Origin::File);
        assert_eq!(canonical(&binding.directory), canonical(&project));

        // The deeper file wins, which is what lets one crate in a repository
        // opt out of the profile the repository as a whole uses.
        workspaces.bind_file(&nested, "client")?;
        assert_eq!(
            workspaces.find(&nested)?.expect("a binding").profile,
            "client"
        );
        assert_eq!(
            workspaces.find(&project)?.expect("a binding").profile,
            "work"
        );
        Ok(())
    }

    #[test]
    fn prefers_a_committed_file_over_a_registry_entry() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let home = temporary.path().join("home");
        fs::create_dir_all(&home)?;
        let workspaces = workspaces(&temporary.path().join("ditto"), &home);

        let project = home.join("project");
        fs::create_dir_all(&project)?;
        workspaces.bind_registry(&project, "work")?;
        assert_eq!(
            workspaces.find(&project)?.expect("a binding").origin,
            Origin::Registry
        );

        workspaces.bind_file(&project, "client")?;
        let binding = workspaces.find(&project)?.expect("a binding");
        assert_eq!(binding.profile, "client");
        assert_eq!(binding.origin, Origin::File);
        Ok(())
    }

    #[test]
    fn a_nearer_registry_entry_outranks_a_further_file() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let home = temporary.path().join("home");
        fs::create_dir_all(&home)?;
        let workspaces = workspaces(&temporary.path().join("ditto"), &home);

        let project = home.join("project");
        let nested = project.join("vendor");
        fs::create_dir_all(&nested)?;
        workspaces.bind_file(&project, "work")?;
        workspaces.bind_registry(&nested, "client")?;

        // Distance decides before the kind of binding does: a registry entry is
        // the only way to bind a directory whose files are not yours to add to.
        assert_eq!(
            workspaces.find(&nested)?.expect("a binding").profile,
            "client"
        );
        Ok(())
    }

    #[test]
    fn clearing_a_directory_leaves_its_ancestors_bound() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let home = temporary.path().join("home");
        fs::create_dir_all(&home)?;
        let workspaces = workspaces(&temporary.path().join("ditto"), &home);

        let project = home.join("project");
        let nested = project.join("crates");
        fs::create_dir_all(&nested)?;
        workspaces.bind_file(&project, "work")?;
        workspaces.bind_file(&nested, "client")?;
        workspaces.bind_registry(&nested, "client")?;

        let removed = workspaces.clear(&nested)?;
        assert_eq!(removed.len(), 2, "expected both bindings, got {removed:?}");
        assert_eq!(
            workspaces.find(&nested)?.expect("a binding").profile,
            "work"
        );

        assert!(workspaces.clear(&nested)?.is_empty());
        Ok(())
    }

    #[test]
    fn refuses_to_bind_a_profile_name_a_path_could_escape_through() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let home = temporary.path().join("home");
        let project = home.join("project");
        fs::create_dir_all(&project)?;
        let workspaces = workspaces(&temporary.path().join("ditto"), &home);

        assert!(workspaces.bind_file(&project, "../escape").is_err());
        assert!(workspaces.bind_registry(&project, "../escape").is_err());

        // A file written by hand is checked on the way back in too, since the
        // name it carries is what the profile store builds directories from.
        fs::write(
            project.join(WORKSPACE_FILE),
            "profile = \"../../elsewhere\"\n",
        )?;
        assert!(workspaces.find(&project).is_err());
        Ok(())
    }

    #[test]
    fn never_auto_binds_the_home_directory_or_the_root() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let home = temporary.path().join("home");
        let project = home.join("project");
        fs::create_dir_all(&project)?;
        let workspaces = workspaces(&temporary.path().join("ditto"), &home);

        assert!(!workspaces.may_auto_bind(&home));
        assert!(!workspaces.may_auto_bind(Path::new("/")));
        assert!(workspaces.may_auto_bind(&project));
        Ok(())
    }

    #[test]
    fn reports_an_unparseable_file_rather_than_ignoring_it() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let home = temporary.path().join("home");
        let project = home.join("project");
        fs::create_dir_all(&project)?;
        let workspaces = workspaces(&temporary.path().join("ditto"), &home);

        fs::write(project.join(WORKSPACE_FILE), "profile =\n")?;
        assert!(workspaces.find(&project).is_err());
        Ok(())
    }
}
