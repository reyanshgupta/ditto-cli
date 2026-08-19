use std::{
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process,
};

use anyhow::{Context, Result, bail};
use directories::BaseDirs;
use serde::{Deserialize, Serialize};

pub const DEFAULT_PROFILE: &str = "default";
const MAX_PROFILE_NAME_LEN: usize = 32;

/// Marks an environment as one Ditto redirected. A nested Ditto command must
/// recover the user's original roots rather than treating the selected
/// profile's roots as the `default` profile.
pub(crate) const LAUNCHED_TOOL_VARIABLE: &str = "DITTO_LAUNCHED_TOOL";

/// The variables Ditto redirects whose ambient values define part of the
/// `default` profile, paired with private copies carried into launched tools.
pub(crate) const NATIVE_ENVIRONMENT: [(&str, &str); 5] = [
    ("XDG_DATA_HOME", "DITTO_NATIVE_XDG_DATA_HOME"),
    ("XDG_CONFIG_HOME", "DITTO_NATIVE_XDG_CONFIG_HOME"),
    ("XDG_STATE_HOME", "DITTO_NATIVE_XDG_STATE_HOME"),
    (
        "PRIME_AGENT_CODING_AGENT_DIR",
        "DITTO_NATIVE_PRIME_AGENT_CODING_AGENT_DIR",
    ),
    ("PI_CODING_AGENT_DIR", "DITTO_NATIVE_PI_CODING_AGENT_DIR"),
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Profile {
    pub name: String,
    pub claude_home: PathBuf,
    pub codex_home: PathBuf,
    pub omp_home: PathBuf,
    pub opencode: OpencodeHome,
    pub pi_home: PathBuf,
    pub prime_agent_home: PathBuf,
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
}

/// The XDG specification requires relative base paths to be ignored, which is
/// what opencode does too.
fn xdg_base(value: Option<OsString>, fallback: PathBuf) -> PathBuf {
    value
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or(fallback)
}

fn configured_home(value: Option<OsString>, user_home: &Path, fallback: PathBuf) -> PathBuf {
    value
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .map(|configured| {
            if configured == Path::new("~") {
                return user_home.to_path_buf();
            }
            match configured.to_str().and_then(|path| path.strip_prefix("~/")) {
                Some(relative) => user_home.join(relative),
                None => configured,
            }
        })
        .unwrap_or(fallback)
}

/// Resolves roots that Ditto itself may have overwritten in a parent process.
///
/// New launches carry the original values under private names. The path check
/// also recognizes children of older Ditto releases, which had only
/// `DITTO_PROFILE` and would otherwise make an upgrade unable to repair the
/// profile it was running inside.
fn native_homes(
    user_home: &Path,
    root: &Path,
    variable: impl Fn(&str) -> Option<OsString>,
) -> (OpencodeHome, PathBuf, PathBuf) {
    let launched = variable(LAUNCHED_TOOL_VARIABLE).is_some();
    let selected = variable("DITTO_PROFILE")
        .filter(|profile| profile != DEFAULT_PROFILE)
        .map(|profile| root.join("profiles").join(profile));

    let original = |name: &str, preserved: &str, suffix: &Path| {
        if let Some(value) = variable(preserved) {
            return Some(value);
        }
        let value = variable(name)?;
        let redirected = launched
            || selected
                .as_ref()
                .is_some_and(|profile| Path::new(&value) == profile.join(suffix));
        (!redirected).then_some(value)
    };

    let opencode_fallback = OpencodeHome::native(user_home);
    let native_opencode = OpencodeHome {
        data: xdg_base(
            original(
                NATIVE_ENVIRONMENT[0].0,
                NATIVE_ENVIRONMENT[0].1,
                Path::new("opencode/data"),
            ),
            opencode_fallback.data,
        ),
        config: xdg_base(
            original(
                NATIVE_ENVIRONMENT[1].0,
                NATIVE_ENVIRONMENT[1].1,
                Path::new("opencode/config"),
            ),
            opencode_fallback.config,
        ),
        state: xdg_base(
            original(
                NATIVE_ENVIRONMENT[2].0,
                NATIVE_ENVIRONMENT[2].1,
                Path::new("opencode/state"),
            ),
            opencode_fallback.state,
        ),
    };
    let native_prime_agent = configured_home(
        original(
            NATIVE_ENVIRONMENT[3].0,
            NATIVE_ENVIRONMENT[3].1,
            Path::new("prime-agent"),
        ),
        user_home,
        user_home.join(".prime").join("agent"),
    );
    let native_pi = configured_home(
        original(
            NATIVE_ENVIRONMENT[4].0,
            NATIVE_ENVIRONMENT[4].1,
            Path::new("pi"),
        ),
        user_home,
        user_home.join(".pi").join("agent"),
    );

    (native_opencode, native_pi, native_prime_agent)
}

#[derive(Clone, Debug)]
pub struct Store {
    root: PathBuf,
    user_home: PathBuf,
    native_opencode: OpencodeHome,
    native_pi: PathBuf,
    native_prime_agent: PathBuf,
}

/// The saved profile a command that named none falls back to, and which saved
/// value it came from.
///
/// The reason travels with the name because a launch that nobody pointed at a
/// profile is the one worth explaining, and explaining it anywhere else would
/// mean restating the precedence rule outside the one place that decides it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Fallback {
    /// Pinned with `ditto-cli default`, or with `d` in the picker.
    Pinned(String),
    /// The profile of the most recent launch, with no pin to outrank it.
    Last(String),
    /// Nothing is saved yet, which is every run before the first launch.
    Reserved,
}

impl Fallback {
    pub fn name(&self) -> &str {
        match self {
            Self::Pinned(name) | Self::Last(name) => name,
            Self::Reserved => DEFAULT_PROFILE,
        }
    }

    /// Completes the sentence "it is ...", so a launch can say why it arrived
    /// at a profile in one line rather than in a paragraph nobody reads.
    pub fn describe(&self) -> &'static str {
        match self {
            Self::Pinned(_) => "pinned as the default",
            Self::Last(_) => "the profile launched last",
            Self::Reserved => "your own configuration, which nothing has replaced yet",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct State {
    /// The profile of the most recent launch. Every launch moves it.
    #[serde(skip_serializing_if = "Option::is_none")]
    last_profile: Option<String>,
    /// The profile pinned in the interface. It outranks `last_profile` when a
    /// command omits the profile name, so launching another profile once does
    /// not quietly move it.
    #[serde(skip_serializing_if = "Option::is_none")]
    default_profile: Option<String>,
    /// Whether launching from an unbound directory records the profile it used
    /// there. Absent means on, so the behaviour does not wait for a setting to
    /// be written before it starts.
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace_auto_bind: Option<bool>,
}

impl Store {
    pub fn discover() -> Result<Self> {
        let base_dirs = BaseDirs::new().context("could not determine your home directory")?;
        let user_home = base_dirs.home_dir().to_path_buf();
        let root = env::var_os("DITTO_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| user_home.join(".ditto"));

        let (native_opencode, native_pi, native_prime_agent) =
            native_homes(&user_home, &root, |name| env::var_os(name));
        Ok(Self {
            root,
            user_home,
            native_opencode,
            native_pi,
            native_prime_agent,
        })
    }

    #[cfg(test)]
    pub fn new(root: PathBuf, user_home: PathBuf) -> Self {
        let native_opencode = OpencodeHome::native(&user_home);
        let native_pi = user_home.join(".pi").join("agent");
        let native_prime_agent = user_home.join(".prime").join("agent");
        Self {
            root,
            user_home,
            native_opencode,
            native_pi,
            native_prime_agent,
        }
    }

    pub fn user_home(&self) -> &Path {
        &self.user_home
    }

    /// Ditto's own directory, which the workspace registry lives in beside the
    /// state file and the profiles.
    pub fn root(&self) -> &Path {
        &self.root
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

        let result = (|| {
            secure_directory(&profile_root)?;
            self.ensure_profile_directories(&profile)
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

        // Both the last-used and the pinned profile are stored by name, so a
        // rename has to carry them across or they would point at nothing.
        let mut state = self.read_state()?;
        let mut state_is_stale = false;
        for slot in [&mut state.last_profile, &mut state.default_profile] {
            if slot.as_deref() == Some(current_name) {
                *slot = Some(new_name.to_owned());
                state_is_stale = true;
            }
        }

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

        if state_is_stale {
            if let Err(state_error) = self.write_state(&state) {
                let omp_rollback_error = move_omp_profile
                    .then(|| fs::rename(&omp_destination, &omp_source).err())
                    .flatten();
                let ditto_rollback_error = fs::rename(&destination, &source).err();

                if let Some(rollback_error) = omp_rollback_error.or(ditto_rollback_error) {
                    bail!(
                        "profile was renamed, but the saved profile selection could not be \
                         updated: {state_error:#}; rollback also failed: {rollback_error}"
                    );
                }
                return Err(state_error)
                    .context("could not update the saved profile selection; rename was reverted");
            }
        }

        Ok(self.managed_profile(new_name))
    }

    /// Removes an isolated profile, its OMP counterpart, and any state that
    /// named it.
    ///
    /// The state is cleared before the directories go, because the two failures
    /// are not equally bad. A pin left pointing at a profile that no longer
    /// exists makes every command that omits a name fail, while a pin released
    /// from a profile that survived a failed removal costs one keystroke to set
    /// again.
    pub fn delete_profile(&self, name: &str) -> Result<()> {
        validate_profile_name(name)?;
        if name == DEFAULT_PROFILE {
            bail!("'{DEFAULT_PROFILE}' is your existing CLI configuration and cannot be deleted");
        }

        self.ensure_storage()?;
        let root = self.profile_root(name);
        if !root.is_dir() {
            bail!("profile '{name}' does not exist");
        }

        let mut state = self.read_state()?;
        let mut state_is_stale = false;
        for slot in [&mut state.last_profile, &mut state.default_profile] {
            if slot.as_deref() == Some(name) {
                *slot = None;
                state_is_stale = true;
            }
        }
        if state_is_stale {
            self.write_state(&state)?;
        }

        fs::remove_dir_all(&root)
            .with_context(|| format!("could not delete {}", root.display()))?;

        let omp_root = self.omp_profile_root(name);
        if omp_root.exists() {
            fs::remove_dir_all(&omp_root)
                .with_context(|| format!("could not delete {}", omp_root.display()))?;
        }
        Ok(())
    }

    /// Every directory `delete_profile` would remove, for a command that has to
    /// say what it is about to destroy before it is allowed to.
    pub fn deletion_targets(&self, name: &str) -> Vec<PathBuf> {
        let omp_root = self.omp_profile_root(name);
        let mut targets = vec![self.profile_root(name)];
        if omp_root.exists() {
            targets.push(omp_root);
        }
        targets
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

    /// Adds tool roots introduced after a profile was created. Syncing is the
    /// explicit migration point; reporting commands must not mutate a profile
    /// merely because they inspected it.
    pub fn ensure_profile_directories(&self, profile: &Profile) -> Result<()> {
        if !profile.managed {
            return Ok(());
        }

        // Parents come before children so every level is locked down before a
        // CLI gets a chance to put credentials in it.
        let opencode_root = self.opencode_root(&profile.name);
        let omp_root = self.omp_profile_root(&profile.name);
        let directories = [
            profile.claude_home.as_path(),
            profile.codex_home.as_path(),
            opencode_root.as_path(),
            profile.opencode.data.as_path(),
            profile.opencode.config.as_path(),
            profile.opencode.state.as_path(),
            omp_root.as_path(),
            profile.omp_home.as_path(),
            profile.pi_home.as_path(),
            profile.prime_agent_home.as_path(),
        ];
        for directory in directories {
            fs::create_dir_all(directory)
                .with_context(|| format!("could not create {}", directory.display()))?;
            secure_directory(directory)?;
        }
        Ok(())
    }

    pub fn last_profile(&self) -> Result<Option<String>> {
        Ok(self.read_state()?.last_profile)
    }

    /// The profile pinned in the interface, if any. Distinct from
    /// [`Self::default_profile`], which is the reserved profile that wraps the
    /// user's existing CLI configuration.
    pub fn default_profile_name(&self) -> Result<Option<String>> {
        Ok(self.read_state()?.default_profile)
    }

    /// The profile a command uses when it does not name one, and which saved
    /// value chose it. A pin outranks the last launch, so launching another
    /// profile once does not move it. Both come from one read so a concurrent
    /// write cannot be half seen.
    pub fn fallback_profile(&self) -> Result<Fallback> {
        let state = self.read_state()?;
        Ok(match (state.default_profile, state.last_profile) {
            (Some(name), _) => Fallback::Pinned(name),
            (None, Some(name)) => Fallback::Last(name),
            (None, None) => Fallback::Reserved,
        })
    }

    pub fn save_last_profile(&self, name: &str) -> Result<()> {
        self.load_profile(name)?;
        let mut state = self.read_state()?;
        state.last_profile = Some(name.to_owned());
        self.write_state(&state)
    }

    /// Pins the profile that commands fall back to when they omit a name, or
    /// clears the pin when given `None`.
    pub fn set_default_profile_name(&self, name: Option<&str>) -> Result<()> {
        if let Some(name) = name {
            self.load_profile(name)?;
        }
        let mut state = self.read_state()?;
        state.default_profile = name.map(str::to_owned);
        self.write_state(&state)
    }

    /// Whether a launch records the profile it used in the directory it ran
    /// from. On unless it has been turned off, since a binding that has to be
    /// asked for is one most directories would never get.
    pub fn workspace_auto_bind(&self) -> Result<bool> {
        Ok(self.read_state()?.workspace_auto_bind.unwrap_or(true))
    }

    pub fn set_workspace_auto_bind(&self, enabled: bool) -> Result<()> {
        let mut state = self.read_state()?;
        state.workspace_auto_bind = Some(enabled);
        self.write_state(&state)
    }

    fn read_state(&self) -> Result<State> {
        let path = self.state_path();
        if !path.exists() {
            return Ok(State::default());
        }

        let contents = fs::read_to_string(&path)
            .with_context(|| format!("could not read {}", path.display()))?;
        toml::from_str(&contents).with_context(|| format!("could not parse {}", path.display()))
    }

    /// Rewrites the whole state file, so callers pass a value they read back
    /// first rather than a fresh one that would drop the other fields.
    fn write_state(&self, state: &State) -> Result<()> {
        self.ensure_storage()?;

        let contents = toml::to_string(state).context("could not serialize profile state")?;
        write_private_file(&self.state_path(), &contents)
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
            pi_home: self.native_pi.clone(),
            prime_agent_home: self.native_prime_agent.clone(),
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
            pi_home: root.join("pi"),
            prime_agent_home: root.join("prime-agent"),
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

/// Writes a file whole and leaves it readable only by its owner.
///
/// Replacing the file in one step is what keeps a crash from leaving half of a
/// settings file behind for a tool to refuse to start from, and the temporary
/// carries the process id so two copies of Ditto cannot land on each other.
pub fn write_private_file(path: &Path, contents: &str) -> Result<()> {
    write_private_bytes(path, contents.as_bytes())
}

pub(crate) fn write_private_bytes(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("{} has no parent directory", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("could not create {}", parent.display()))?;

    let name = path
        .file_name()
        .with_context(|| format!("{} does not name a file", path.display()))?
        .to_string_lossy()
        .into_owned();
    let temporary = parent.join(format!(".{name}.{}.tmp", process::id()));

    fs::write(&temporary, contents)
        .with_context(|| format!("could not write {}", temporary.display()))?;
    secure_file(&temporary)?;
    if let Err(error) = replace(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error).with_context(|| format!("could not replace {}", path.display()));
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    fs::rename(temporary, destination)
}

/// Windows refuses to replace a file that another program has open, and a virus
/// scanner reading a file Ditto has only just written counts as one. Waiting it
/// out beats reporting a failure the user can neither see nor act on; the waits
/// stay short because the alternative to succeeding is trying again, not
/// hanging.
#[cfg(windows)]
fn replace(temporary: &Path, destination: &Path) -> std::io::Result<()> {
    const ATTEMPTS: u32 = 5;

    for attempt in 1..ATTEMPTS {
        match fs::rename(temporary, destination) {
            Ok(()) => return Ok(()),
            Err(_) => {
                std::thread::sleep(std::time::Duration::from_millis(20 * u64::from(attempt)));
            }
        }
    }
    fs::rename(temporary, destination)
}

#[cfg(unix)]
pub(crate) fn secure_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("could not secure {}", path.display()))
}

/// Windows has no mode to set. A profile lives under the user's own directory,
/// which the system already restricts to that user, so the inherited access
/// control is what stands in for the mode set above.
#[cfg(not(unix))]
pub(crate) fn secure_directory(_path: &Path) -> Result<()> {
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
            "ACME",
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
    fn does_not_mistake_a_launched_profile_for_the_native_roots() {
        let home = std::env::temp_dir().join("ditto-native-home");
        let root = home.join(".ditto");
        let profile = root.join("profiles/work");
        let values = std::collections::HashMap::from([
            ("DITTO_PROFILE", OsString::from("work")),
            (
                "XDG_DATA_HOME",
                profile.join("opencode/data").into_os_string(),
            ),
            (
                "XDG_CONFIG_HOME",
                profile.join("opencode/config").into_os_string(),
            ),
            (
                "XDG_STATE_HOME",
                profile.join("opencode/state").into_os_string(),
            ),
            (
                "PRIME_AGENT_CODING_AGENT_DIR",
                profile.join("prime-agent").into_os_string(),
            ),
            ("PI_CODING_AGENT_DIR", profile.join("pi").into_os_string()),
        ]);

        let (opencode, pi, prime_agent) =
            native_homes(&home, &root, |name| values.get(name).cloned());

        assert_eq!(opencode, OpencodeHome::native(&home));
        assert_eq!(pi, home.join(".pi/agent"));
        assert_eq!(prime_agent, home.join(".prime/agent"));
    }

    #[test]
    fn carries_custom_native_roots_through_a_launched_tool() {
        let home = std::env::temp_dir().join("ditto-native-home");
        let root = home.join(".ditto");
        let custom = std::env::temp_dir().join("ditto-custom-opencode");
        let values = std::collections::HashMap::from([
            (LAUNCHED_TOOL_VARIABLE, OsString::from("pi")),
            (
                "DITTO_NATIVE_XDG_DATA_HOME",
                custom.join("data").into_os_string(),
            ),
            (
                "DITTO_NATIVE_XDG_CONFIG_HOME",
                custom.join("config").into_os_string(),
            ),
            (
                "DITTO_NATIVE_XDG_STATE_HOME",
                custom.join("state").into_os_string(),
            ),
            (
                "DITTO_NATIVE_PRIME_AGENT_CODING_AGENT_DIR",
                OsString::from("~/prime"),
            ),
            ("DITTO_NATIVE_PI_CODING_AGENT_DIR", OsString::from("~/pi")),
        ]);

        let (opencode, pi, prime_agent) =
            native_homes(&home, &root, |name| values.get(name).cloned());

        assert_eq!(opencode.data, custom.join("data"));
        assert_eq!(opencode.config, custom.join("config"));
        assert_eq!(opencode.state, custom.join("state"));
        assert_eq!(pi, home.join("pi"));
        assert_eq!(prime_agent, home.join("prime"));
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
        assert!(profile.omp_home.is_dir());
        assert!(profile.pi_home.is_dir());
        assert!(profile.prime_agent_home.is_dir());
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
        assert_eq!(default.pi_home, home.join(".pi").join("agent"));
        assert_eq!(default.prime_agent_home, home.join(".prime").join("agent"));
        Ok(())
    }

    #[test]
    fn provisions_tool_roots_added_after_a_profile_was_created() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let store = Store::new(
            temporary.path().join("ditto"),
            temporary.path().join("home"),
        );
        fs::create_dir_all(store.profile_root("old"))?;
        let profile = store.load_profile("old")?;

        store.ensure_profile_directories(&profile)?;

        assert!(profile.omp_home.is_dir());
        assert!(profile.pi_home.is_dir());
        assert!(profile.prime_agent_home.is_dir());
        Ok(())
    }

    #[test]
    fn keeps_the_pinned_and_last_profile_independent() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let store = Store::new(
            temporary.path().join("ditto"),
            temporary.path().join("home"),
        );
        store.create_profile("work")?;
        store.create_profile("personal")?;

        assert_eq!(store.default_profile_name()?, None);
        store.set_default_profile_name(Some("work"))?;
        store.save_last_profile("personal")?;

        // Writing one field must not drop the other: a launch of another
        // profile is exactly what the pin is supposed to survive.
        assert_eq!(store.default_profile_name()?.as_deref(), Some("work"));
        assert_eq!(store.last_profile()?.as_deref(), Some("personal"));
        // The fallback carries which value answered, not just the name, because
        // a launch nobody pointed at a profile has to be able to say why.
        assert_eq!(
            store.fallback_profile()?,
            Fallback::Pinned("work".to_owned())
        );

        // Releasing the pin hands the fallback back to the last launch rather
        // than dropping all the way to the reserved profile.
        store.set_default_profile_name(None)?;
        assert_eq!(store.default_profile_name()?, None);
        assert_eq!(store.last_profile()?.as_deref(), Some("personal"));
        assert_eq!(
            store.fallback_profile()?,
            Fallback::Last("personal".to_owned())
        );
        Ok(())
    }

    #[test]
    fn refuses_to_pin_a_profile_that_does_not_exist() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let store = Store::new(
            temporary.path().join("ditto"),
            temporary.path().join("home"),
        );

        assert!(store.set_default_profile_name(Some("missing")).is_err());
        assert_eq!(store.default_profile_name()?, None);

        // The reserved profile is a legitimate pin: it is how a user goes back
        // to their existing configuration without clearing the setting.
        store.set_default_profile_name(Some(DEFAULT_PROFILE))?;
        assert_eq!(
            store.default_profile_name()?.as_deref(),
            Some(DEFAULT_PROFILE)
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
        std::fs::write(original.pi_home.join("auth.json"), "kept")?;
        std::fs::write(original.prime_agent_home.join("auth.json"), "kept")?;
        store.save_last_profile("work")?;
        store.set_default_profile_name(Some("work"))?;

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
        assert_eq!(
            std::fs::read_to_string(renamed.pi_home.join("auth.json"))?,
            "kept"
        );
        assert_eq!(
            std::fs::read_to_string(renamed.prime_agent_home.join("auth.json"))?,
            "kept"
        );
        assert!(store.load_profile("work").is_err());
        assert_eq!(store.last_profile()?.as_deref(), Some("client"));
        // Both are stored by name, so a rename has to carry the pin across or
        // it would point at a profile that no longer exists.
        assert_eq!(store.default_profile_name()?.as_deref(), Some("client"));
        Ok(())
    }

    #[test]
    fn deletes_a_profile_and_the_state_that_named_it() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let store = Store::new(
            temporary.path().join("ditto"),
            temporary.path().join("home"),
        );
        let profile = store.create_profile("work")?;
        store.create_profile("personal")?;
        std::fs::create_dir_all(&profile.omp_home)?;
        store.save_last_profile("work")?;
        store.set_default_profile_name(Some("work"))?;

        store.delete_profile("work")?;

        assert!(!profile.claude_home.exists());
        assert!(!profile.omp_home.exists());
        assert!(!profile.pi_home.exists());
        assert!(!profile.prime_agent_home.exists());
        assert!(store.load_profile("work").is_err());
        // A pin left pointing at a deleted profile would break every command
        // that omits a name, so deleting has to release it.
        assert_eq!(store.default_profile_name()?, None);
        assert_eq!(store.last_profile()?, None);
        assert_eq!(store.fallback_profile()?, Fallback::Reserved);
        assert_eq!(store.fallback_profile()?.name(), DEFAULT_PROFILE);
        // Only the named profile goes.
        assert!(store.load_profile("personal").is_ok());
        Ok(())
    }

    #[test]
    fn leaves_state_naming_another_profile_alone_when_deleting() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let store = Store::new(
            temporary.path().join("ditto"),
            temporary.path().join("home"),
        );
        store.create_profile("work")?;
        store.create_profile("personal")?;
        store.set_default_profile_name(Some("personal"))?;
        store.save_last_profile("personal")?;

        store.delete_profile("work")?;

        assert_eq!(store.default_profile_name()?.as_deref(), Some("personal"));
        assert_eq!(store.last_profile()?.as_deref(), Some("personal"));
        Ok(())
    }

    #[test]
    fn refuses_to_delete_what_it_does_not_own() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let store = Store::new(
            temporary.path().join("ditto"),
            temporary.path().join("home"),
        );

        // The reserved profile is the user's own configuration, so deleting it
        // would take a setup Ditto never created.
        assert!(store.delete_profile(DEFAULT_PROFILE).is_err());
        assert!(store.delete_profile("missing").is_err());
        assert!(store.delete_profile("../escape").is_err());
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
