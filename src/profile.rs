use std::{
    env, fs,
    path::{Path, PathBuf},
    process,
};

use anyhow::{Context, Result, bail};
use directories::BaseDirs;
use serde::{Deserialize, Serialize};

pub const DEFAULT_PROFILE: &str = "default";
const MAX_PROFILE_NAME_LEN: usize = 32;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Profile {
    pub name: String,
    pub claude_home: PathBuf,
    pub codex_home: PathBuf,
    pub omp_home: PathBuf,
    pub opencode: OpencodeHome,
    pub managed: bool,
}

/// opencode has no single home variable. It resolves its directories from the
/// XDG base variables, so a profile pins the three bases that carry
/// credentials, configuration, and session state. The shared cache is left
/// alone: it holds downloaded tooling rather than anything account specific.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpencodeHome {
    pub data: PathBuf,
    pub config: PathBuf,
    pub state: PathBuf,
}

impl OpencodeHome {
    /// The directories opencode itself creates: it appends its own name to
    /// every XDG base it reads.
    pub fn data_dir(&self) -> PathBuf {
        self.data.join("opencode")
    }

    pub fn config_dir(&self) -> PathBuf {
        self.config.join("opencode")
    }

    fn isolated(root: &Path) -> Self {
        Self {
            data: root.join("data"),
            config: root.join("config"),
            state: root.join("state"),
        }
    }

    fn native(user_home: &Path) -> Self {
        Self {
            data: user_home.join(".local").join("share"),
            config: user_home.join(".config"),
            state: user_home.join(".local").join("state"),
        }
    }

    fn from_environment(user_home: &Path) -> Self {
        let native = Self::native(user_home);
        Self {
            data: xdg_base("XDG_DATA_HOME", native.data),
            config: xdg_base("XDG_CONFIG_HOME", native.config),
            state: xdg_base("XDG_STATE_HOME", native.state),
        }
    }
}

/// The XDG specification requires relative base paths to be ignored, which is
/// what opencode does too.
fn xdg_base(variable: &str, fallback: PathBuf) -> PathBuf {
    env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or(fallback)
}

#[derive(Clone, Debug)]
pub struct Store {
    root: PathBuf,
    user_home: PathBuf,
    native_opencode: OpencodeHome,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct State {
    last_profile: Option<String>,
}

impl Store {
    pub fn discover() -> Result<Self> {
        let base_dirs = BaseDirs::new().context("could not determine your home directory")?;
        let user_home = base_dirs.home_dir().to_path_buf();
        let root = env::var_os("DITTO_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| user_home.join(".ditto"));

        let native_opencode = OpencodeHome::from_environment(&user_home);
        Ok(Self {
            root,
            user_home,
            native_opencode,
        })
    }

    #[cfg(test)]
    pub fn new(root: PathBuf, user_home: PathBuf) -> Self {
        let native_opencode = OpencodeHome::native(&user_home);
        Self {
            root,
            user_home,
            native_opencode,
        }
    }

    pub fn user_home(&self) -> &Path {
        &self.user_home
    }

    pub fn list_profiles(&self) -> Result<Vec<Profile>> {
        self.ensure_storage()?;

        let mut profiles = vec![self.default_profile()];
        for entry in fs::read_dir(self.profiles_root())
            .with_context(|| format!("could not read {}", self.profiles_root().display()))?
        {
            let entry = entry.context("could not read a profile directory entry")?;
            if !entry
                .file_type()
                .context("could not inspect a profile directory entry")?
                .is_dir()
            {
                continue;
            }

            let name = entry.file_name().to_string_lossy().into_owned();
            if validate_profile_name(&name).is_ok() && name != DEFAULT_PROFILE {
                profiles.push(self.managed_profile(&name));
            }
        }

        profiles[1..].sort_unstable_by(|left, right| left.name.cmp(&right.name));
        Ok(profiles)
    }

    pub fn create_profile(&self, name: &str) -> Result<Profile> {
        validate_profile_name(name)?;
        if name == DEFAULT_PROFILE {
            bail!("'{DEFAULT_PROFILE}' is reserved for your existing CLI configuration");
        }

        self.ensure_storage()?;
        let profile = self.managed_profile(name);
        let profile_root = self.profile_root(name);
        fs::create_dir(&profile_root)
            .with_context(|| format!("profile '{name}' already exists or could not be created"))?;

        // Parents come before children so every level is created and locked
        // down before the CLIs get a chance to write credentials into it.
        let opencode_root = self.opencode_root(name);
        let directories = [
            profile.claude_home.as_path(),
            profile.codex_home.as_path(),
            opencode_root.as_path(),
            profile.opencode.data.as_path(),
            profile.opencode.config.as_path(),
            profile.opencode.state.as_path(),
        ];

        let result = (|| {
            secure_directory(&profile_root)?;
            for directory in directories {
                fs::create_dir(directory)
                    .with_context(|| format!("could not create {}", directory.display()))?;
                secure_directory(directory)?;
            }
            Ok(())
        })();

        if let Err(error) = result {
            let _ = fs::remove_dir_all(&profile_root);
            return Err(error);
        }

        Ok(profile)
    }
    pub fn rename_profile(&self, current_name: &str, new_name: &str) -> Result<Profile> {
        validate_profile_name(current_name)?;
        validate_profile_name(new_name)?;
        if current_name == DEFAULT_PROFILE {
            bail!("the default profile cannot be renamed");
        }
        if new_name == DEFAULT_PROFILE {
            bail!("'{DEFAULT_PROFILE}' is reserved for your existing CLI configuration");
        }
        if current_name == new_name {
            bail!("profile '{new_name}' already has that name");
        }

        self.ensure_storage()?;
        let source = self.profile_root(current_name);
        if !source.is_dir() {
            bail!("profile '{current_name}' does not exist");
        }
        let destination = self.profile_root(new_name);
        if destination.exists() {
            bail!("profile '{new_name}' already exists");
        }

        let omp_source = self.omp_profile_root(current_name);
        let omp_destination = self.omp_profile_root(new_name);
        let move_omp_profile = omp_source.exists();
        if move_omp_profile && omp_destination.exists() {
            bail!("OMP profile '{new_name}' already exists");
        }

        let was_selected = self.last_profile()?.as_deref() == Some(current_name);
        fs::rename(&source, &destination).with_context(|| {
            format!("could not rename profile '{current_name}' to '{new_name}'")
        })?;

        if move_omp_profile {
            if let Err(omp_error) = fs::rename(&omp_source, &omp_destination) {
                if let Err(rollback_error) = fs::rename(&destination, &source) {
                    bail!(
                        "could not rename OMP profile '{current_name}' to '{new_name}': \
                         {omp_error}; Ditto rollback also failed: {rollback_error}"
                    );
                }
                return Err(omp_error).with_context(|| {
                    format!("could not rename OMP profile '{current_name}' to '{new_name}'")
                });
            }
        }

        if was_selected {
            if let Err(state_error) = self.save_last_profile(new_name) {
                let omp_rollback_error = move_omp_profile
                    .then(|| fs::rename(&omp_destination, &omp_source).err())
                    .flatten();
                let ditto_rollback_error = fs::rename(&destination, &source).err();

                if let Some(rollback_error) = omp_rollback_error.or(ditto_rollback_error) {
                    bail!(
                        "profile was renamed, but the selected profile could not be updated: \
                         {state_error:#}; rollback also failed: {rollback_error}"
                    );
                }
                return Err(state_error)
                    .context("could not update the selected profile; rename was reverted");
            }
        }

        Ok(self.managed_profile(new_name))
    }

    pub fn load_profile(&self, name: &str) -> Result<Profile> {
        validate_profile_name(name)?;
        if name == DEFAULT_PROFILE {
            return Ok(self.default_profile());
        }

        let profile = self.managed_profile(name);
        if !self.profile_root(name).is_dir() {
            bail!("profile '{name}' does not exist; create it with `ditto-cli create {name}`");
        }
        Ok(profile)
    }

    pub fn last_profile(&self) -> Result<Option<String>> {
        let path = self.state_path();
        if !path.exists() {
            return Ok(None);
        }

        let contents = fs::read_to_string(&path)
            .with_context(|| format!("could not read {}", path.display()))?;
        let state: State = toml::from_str(&contents)
            .with_context(|| format!("could not parse {}", path.display()))?;
        Ok(state.last_profile)
    }

    pub fn save_last_profile(&self, name: &str) -> Result<()> {
        self.load_profile(name)?;
        self.ensure_storage()?;

        let state = State {
            last_profile: Some(name.to_owned()),
        };
        let contents = toml::to_string(&state).context("could not serialize profile state")?;
        let destination = self.state_path();
        let temporary = self.root.join(format!(".state.toml.{}.tmp", process::id()));

        fs::write(&temporary, contents)
            .with_context(|| format!("could not write {}", temporary.display()))?;
        secure_file(&temporary)?;
        if let Err(error) = fs::rename(&temporary, &destination) {
            let _ = fs::remove_file(&temporary);
            return Err(error)
                .with_context(|| format!("could not replace {}", destination.display()));
        }
        Ok(())
    }

    fn ensure_storage(&self) -> Result<()> {
        fs::create_dir_all(self.profiles_root())
            .with_context(|| format!("could not create {}", self.profiles_root().display()))?;
        secure_directory(&self.root)?;
        secure_directory(&self.profiles_root())
    }

    fn default_profile(&self) -> Profile {
        Profile {
            name: DEFAULT_PROFILE.to_owned(),
            claude_home: self.user_home.join(".claude"),
            codex_home: self.user_home.join(".codex"),
            omp_home: self.user_home.join(".omp").join("agent"),
            opencode: self.native_opencode.clone(),
            managed: false,
        }
    }

    fn managed_profile(&self, name: &str) -> Profile {
        let root = self.profile_root(name);
        Profile {
            name: name.to_owned(),
            claude_home: root.join("claude"),
            codex_home: root.join("codex"),
            omp_home: self.omp_profile_root(name).join("agent"),
            opencode: OpencodeHome::isolated(&self.opencode_root(name)),
            managed: true,
        }
    }

    fn profiles_root(&self) -> PathBuf {
        self.root.join("profiles")
    }

    fn profile_root(&self, name: &str) -> PathBuf {
        self.profiles_root().join(name)
    }
    fn omp_profile_root(&self, name: &str) -> PathBuf {
        self.user_home.join(".omp").join("profiles").join(name)
    }

    fn opencode_root(&self, name: &str) -> PathBuf {
        self.profile_root(name).join("opencode")
    }

    fn state_path(&self) -> PathBuf {
        self.root.join("state.toml")
    }
}

/// Accepts only names every supported tool can also accept.
///
/// OMP receives the profile name verbatim as `--profile` and `OMP_PROFILE`, and
/// it requires `^[a-z0-9][a-z0-9._-]{0,63}$` while rejecting trailing dots and
/// Windows device names. Ditto applies the same rules at creation time so a
/// profile cannot be created that later fails only when OMP launches.
pub fn validate_profile_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > MAX_PROFILE_NAME_LEN {
        bail!("profile names must contain 1 to {MAX_PROFILE_NAME_LEN} characters");
    }
    if !name.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'_')
    }) {
        bail!("profile names may only contain lowercase letters, numbers, '.', '-' and '_'");
    }
    let first = name.as_bytes()[0];
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        bail!("profile names must start with a lowercase letter or a number");
    }
    if name.ends_with('.') {
        bail!("profile names may not end with '.'");
    }
    let stem = name.split('.').next().unwrap_or(name);
    if is_reserved_device_name(stem) {
        bail!("'{stem}' is a reserved device name on Windows");
    }
    Ok(())
}

fn is_reserved_device_name(stem: &str) -> bool {
    matches!(stem, "con" | "prn" | "aux" | "nul")
        || stem
            .strip_prefix("com")
            .or_else(|| stem.strip_prefix("lpt"))
            .is_some_and(|port| port.len() == 1 && port.as_bytes()[0].is_ascii_digit())
}

#[cfg(unix)]
fn secure_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("could not secure {}", path.display()))
}

#[cfg(not(unix))]
fn secure_directory(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn secure_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("could not secure {}", path.display()))
}

#[cfg(not(unix))]
fn secure_file(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_profile_names() {
        for name in ["", "../work", "work/client", "has space", ".", ".."] {
            assert!(validate_profile_name(name).is_err(), "accepted {name:?}");
        }
    }

    #[test]
    fn rejects_profile_names_omp_cannot_accept() {
        for name in [
            "Work",
            "DHA",
            ".hidden",
            "-leading",
            "_leading",
            "trailing.",
            "con",
            "nul.txt",
            "com1",
            "lpt9",
        ] {
            assert!(validate_profile_name(name).is_err(), "accepted {name:?}");
        }
    }

    #[test]
    fn accepts_portable_profile_names() {
        for name in [
            "work",
            "client-1",
            "personal_2",
            "app.v2",
            "0",
            "com",
            "lpt0x",
        ] {
            assert!(validate_profile_name(name).is_ok(), "rejected {name:?}");
        }
    }

    #[test]
    fn creates_and_remembers_an_isolated_profile() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let store = Store::new(
            temporary.path().join("ditto"),
            temporary.path().join("home"),
        );

        let profile = store.create_profile("work")?;
        assert!(profile.claude_home.is_dir());
        assert!(profile.codex_home.is_dir());
        assert!(profile.opencode.data.is_dir());
        assert!(profile.opencode.config.is_dir());
        assert!(profile.opencode.state.is_dir());
        assert!(store.create_profile("work").is_err());

        store.save_last_profile("work")?;
        assert_eq!(store.last_profile()?.as_deref(), Some("work"));
        assert_eq!(
            store
                .list_profiles()?
                .into_iter()
                .map(|profile| profile.name)
                .collect::<Vec<_>>(),
            ["default", "work"]
        );
        Ok(())
    }

    #[test]
    fn separates_opencode_xdg_bases_per_profile() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let home = temporary.path().join("home");
        let store = Store::new(temporary.path().join("ditto"), home.clone());
        let work = store.create_profile("work")?;
        let personal = store.create_profile("personal")?;

        // opencode appends its own name to each base, so distinct bases are
        // what keeps two profiles from sharing one auth.json.
        assert_ne!(work.opencode.data_dir(), personal.opencode.data_dir());
        assert_ne!(work.opencode.data_dir(), work.opencode.config_dir());
        assert!(
            work.opencode
                .data_dir()
                .starts_with(work.claude_home.parent().unwrap())
        );

        let default = store.load_profile(DEFAULT_PROFILE)?;
        assert_eq!(
            default.opencode.data_dir(),
            home.join(".local").join("share").join("opencode")
        );
        assert_eq!(
            default.opencode.config_dir(),
            home.join(".config").join("opencode")
        );
        Ok(())
    }

    #[test]
    fn renames_profile_data_and_selected_state() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let store = Store::new(
            temporary.path().join("ditto"),
            temporary.path().join("home"),
        );
        let original = store.create_profile("work")?;
        std::fs::write(original.claude_home.join("marker"), "kept")?;
        std::fs::create_dir_all(original.opencode.data_dir())?;
        std::fs::write(original.opencode.data_dir().join("auth.json"), "kept")?;
        std::fs::create_dir_all(&original.omp_home)?;
        std::fs::write(original.omp_home.join("marker"), "kept")?;
        store.save_last_profile("work")?;

        let renamed = store.rename_profile("work", "client")?;

        assert_eq!(renamed.name, "client");
        assert_eq!(
            std::fs::read_to_string(renamed.claude_home.join("marker"))?,
            "kept"
        );
        assert_eq!(
            std::fs::read_to_string(renamed.opencode.data_dir().join("auth.json"))?,
            "kept"
        );
        assert_eq!(
            std::fs::read_to_string(renamed.omp_home.join("marker"))?,
            "kept"
        );
        assert!(store.load_profile("work").is_err());
        assert_eq!(store.last_profile()?.as_deref(), Some("client"));
        Ok(())
    }

    #[test]
    fn rejects_ambiguous_or_destructive_renames() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let store = Store::new(
            temporary.path().join("ditto"),
            temporary.path().join("home"),
        );
        store.create_profile("work")?;
        store.create_profile("client")?;

        assert!(store.rename_profile("default", "native").is_err());
        assert!(store.rename_profile("work", "default").is_err());
        assert!(store.rename_profile("work", "client").is_err());
        assert!(store.rename_profile("work", "Work").is_err());
        assert!(store.load_profile("work").is_ok());
        Ok(())
    }
}
