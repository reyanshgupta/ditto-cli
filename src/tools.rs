//! The coding agents Ditto launches that are described by data rather than
//! code.
//!
//! The first tools each earned bespoke handling: opencode spreads itself over
//! three XDG bases, OMP is selected by name rather than by directory, Claude
//! Code carries a status line. The rest differ from one another only in what
//! they are called, which variable moves their directory, and which file in it
//! means they are signed in. A match arm per tool in every module would bury
//! those differences under the repetition, so each of these is one entry here
//! and one arm reading it everywhere else.
//!
//! Every fact in an entry was read from the tool's own documentation or source
//! rather than remembered, and the comment beside an entry says what was
//! uncertain when it was written. The list is Orca's, which runs the same
//! agents side by side and is where Ditto is most often asked to sit.

/// How a tool finds its directory, which is the one thing Ditto has to change
/// to run it as somebody else.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Home {
    /// One variable names the directory. `native` is where the tool keeps it,
    /// relative to the home directory, when the variable is unset.
    Variable {
        variable: &'static str,
        native: &'static str,
    },
    /// The variable names a directory the tool creates `native` inside, as
    /// `GEMINI_CLI_HOME` does. A managed profile is given a directory of its
    /// own for it, the way a private home works, without touching `HOME`.
    Parent {
        variable: &'static str,
        native: &'static str,
    },
    /// The tool appends `subdir` to each XDG base it reads, so a profile pins
    /// the three bases the way it already does for opencode. Paths in the spec
    /// then begin with the base they belong to: `config/`, `data/`, or `state/`.
    Xdg { subdir: &'static str },
    /// Nothing but `HOME` decides, so a managed profile is handed a private one
    /// with the user's real home mirrored into it, as fx is. `native` is the
    /// directory the tool makes inside it, and `owned` names any other entries
    /// of the home that carry its account and so are not mirrored either.
    Private {
        native: &'static str,
        owned: &'static [&'static str],
    },
}

#[derive(Debug, Eq, Hash, PartialEq)]
pub struct Spec {
    /// The subcommand, the shell function, the `tools[].tool` key, and the
    /// directory name inside a profile. The same as the executable, so that
    /// what a person types to run the tool is what they type to run it through
    /// Ditto.
    pub key: &'static str,
    pub label: &'static str,
    pub executable: &'static str,
    /// Overrides the executable, as `DITTO_CLAUDE_BIN` does for Claude Code.
    pub bin_variable: &'static str,
    pub home: Home,
    /// Set for managed profiles only, typically to move a login the tool would
    /// otherwise keep in a user-wide keychain into the profile's directory,
    /// where it can be somebody else's.
    pub managed_env: &'static [(&'static str, &'static str)],
    /// Files whose presence means the profile is signed in, relative to the
    /// tool's directory. Empty when the tool keeps its login only in a keychain
    /// or an environment variable, which Ditto cannot read.
    pub credentials: &'static [&'static str],
    /// Configuration a profile reads from the user's own copy rather than
    /// starting without: settings, instructions, skills, commands. Never a
    /// credential, a session, or a cache; see `shared.rs` for why the list
    /// must stay an allowlist.
    pub shared: &'static [&'static str],
    /// Where conversations live, for `sync --history`.
    pub sessions: &'static [&'static str],
    /// Arguments that open the tool's own sign-in and sign-out. `None` when it
    /// only signs in from inside its interface.
    pub login: Option<&'static [&'static str]>,
    pub logout: Option<&'static [&'static str]>,
}

impl Spec {
    /// Position in [`ALL`], which is how per-tool state is stored without a
    /// field per tool. Found by key rather than by address: `ALL` is a
    /// constant, so a reference taken to it in one place need not point at the
    /// same bytes as one taken elsewhere.
    pub fn index(&'static self) -> usize {
        ALL.iter()
            .position(|spec| spec.key == self.key)
            .expect("every Spec handed out is an entry of tools::ALL")
    }
}

pub const ALL: &[Spec] = &[
    Spec {
        key: "gemini",
        label: "Gemini CLI",
        executable: "gemini",
        bin_variable: "DITTO_GEMINI_BIN",
        // `GEMINI_CLI_HOME` is the directory `.gemini` is created inside, not
        // the directory itself. Leave `GEMINI_FORCE_ENCRYPTED_FILE_STORAGE`
        // unset: it moves the login into the keychain, which is user-wide.
        home: Home::Parent {
            variable: "GEMINI_CLI_HOME",
            native: ".gemini",
        },
        managed_env: &[],
        credentials: &["oauth_creds.json", "google_accounts.json"],
        shared: &[
            "settings.json",
            "GEMINI.md",
            "keybindings.json",
            "commands",
            "skills",
            "agents",
            "policies",
            "extensions",
            "trustedFolders.json",
        ],
        sessions: &["tmp", "history"],
        login: None,
        logout: None,
    },
    Spec {
        key: "qwen",
        label: "Qwen Code",
        executable: "qwen",
        bin_variable: "DITTO_QWEN_BIN",
        home: Home::Variable {
            variable: "QWEN_HOME",
            native: ".qwen",
        },
        managed_env: &[],
        credentials: &["oauth_creds.json"],
        // `settings.json` carries `modelProviders[].apiKey`, so it stays with
        // the account rather than being shared.
        shared: &["memory.md", "commands", "skills", "workflows", "extensions"],
        sessions: &["projects", "file-history"],
        login: None,
        logout: None,
    },
    Spec {
        key: "openclaude",
        label: "OpenClaude",
        executable: "openclaude",
        bin_variable: "DITTO_OPENCLAUDE_BIN",
        // Reads its own variable and deliberately ignores `CLAUDE_CONFIG_DIR`.
        // The login goes to the macOS Keychain first, which is user-wide, with
        // `.credentials.json` only as its fallback; no switch forces the file.
        home: Home::Variable {
            variable: "OPENCLAUDE_CONFIG_DIR",
            native: ".openclaude",
        },
        managed_env: &[],
        credentials: &[".credentials.json"],
        shared: &[
            "settings.json",
            "keybindings.json",
            "CLAUDE.md",
            "agents",
            "commands",
            "skills",
            "plugins",
            "hooks",
        ],
        sessions: &["projects"],
        login: Some(&["auth", "login"]),
        logout: Some(&["auth", "logout"]),
    },
    Spec {
        key: "copilot",
        label: "Copilot",
        executable: "copilot",
        bin_variable: "DITTO_COPILOT_BIN",
        // The token lives in the OS keychain keyed by GitHub host and user,
        // not by `COPILOT_HOME`, and `config.json` exists whether or not it
        // holds the plaintext fallback, so there is no file to read a login
        // from. `COPILOT_GITHUB_TOKEN` in the environment outranks both.
        home: Home::Variable {
            variable: "COPILOT_HOME",
            native: ".copilot",
        },
        managed_env: &[],
        credentials: &[],
        shared: &[
            "settings.json",
            "lsp-config.json",
            "hooks",
            "skills",
            "agents",
            "instructions",
        ],
        sessions: &["session-state", "history-session-state"],
        login: Some(&["login"]),
        logout: None,
    },
    Spec {
        key: "cursor-agent",
        label: "Cursor Agent",
        // Newer installers name the binary `agent` and keep `cursor-agent` as
        // the compatible name; `DITTO_CURSOR_AGENT_BIN=agent` if only that one
        // is present. `CURSOR_CONFIG_DIR` is documented for `cli-config.json`
        // alone, so whether it also moves the chats is unverified.
        executable: "cursor-agent",
        bin_variable: "DITTO_CURSOR_AGENT_BIN",
        home: Home::Variable {
            variable: "CURSOR_CONFIG_DIR",
            native: ".cursor",
        },
        // The credential store is the OS keychain unless asked for a file; the
        // file's name is not documented, so the login still cannot be read.
        managed_env: &[("AGENT_CLI_CREDENTIAL_STORE", "file")],
        credentials: &[],
        shared: &[
            "settings.json",
            "sandbox.json",
            "hooks.json",
            "agents",
            "skills",
            "plugins",
        ],
        sessions: &["chats", "projects"],
        login: Some(&["login"]),
        logout: Some(&["logout"]),
    },
    Spec {
        key: "grok",
        label: "Grok",
        executable: "grok",
        bin_variable: "DITTO_GROK_BIN",
        home: Home::Variable {
            variable: "GROK_HOME",
            native: ".grok",
        },
        managed_env: &[],
        credentials: &["auth.json"],
        shared: &[
            "config.toml",
            "managed_config.toml",
            "hooks",
            "skills",
            "rules",
            "workflows",
            "memory",
        ],
        sessions: &["sessions"],
        login: Some(&["login"]),
        logout: Some(&["logout"]),
    },
    Spec {
        key: "devin",
        label: "Devin",
        executable: "devin",
        bin_variable: "DITTO_DEVIN_BIN",
        // Only the data base is documented by name; the configuration is said
        // to follow the XDG convention without the variable being named, and
        // Orca's `DEVIN_HOME` is not in Devin's own documentation. The data
        // base also holds the downloaded CLI versions, so a managed profile
        // downloads its own copy once.
        home: Home::Xdg { subdir: "devin" },
        managed_env: &[],
        credentials: &["data/credentials.toml"],
        shared: &["config/config.json", "config/skills"],
        sessions: &["data/cli/transcripts"],
        login: Some(&["auth", "login"]),
        logout: Some(&["auth", "logout"]),
    },
    Spec {
        key: "kimi",
        label: "Kimi Code",
        executable: "kimi",
        bin_variable: "DITTO_KIMI_BIN",
        home: Home::Variable {
            variable: "KIMI_CODE_HOME",
            native: ".kimi-code",
        },
        managed_env: &[],
        credentials: &["credentials"],
        // `config.toml` holds `[providers.*].api_key`, so it stays with the
        // account.
        shared: &["tui.toml", "AGENTS.md", "skills", "plugins"],
        sessions: &["sessions", "user-history"],
        login: Some(&["login"]),
        logout: None,
    },
    Spec {
        key: "cline",
        label: "Cline",
        executable: "cline",
        bin_variable: "DITTO_CLINE_BIN",
        // `CLINE_DIR` is read by the source but absent from the documented
        // variable table; `--config <dir>` is its documented equivalent.
        home: Home::Variable {
            variable: "CLINE_DIR",
            native: ".cline",
        },
        managed_env: &[],
        credentials: &["data/settings/providers.json"],
        shared: &[
            "rules",
            "skills",
            "hooks",
            "workflows",
            "agents",
            "plugins",
            "data/settings/global-settings.json",
            "data/settings/models.json",
        ],
        sessions: &["data/db", "data/sessions"],
        login: Some(&["auth"]),
        logout: None,
    },
    Spec {
        key: "codebuff",
        label: "Codebuff",
        executable: "codebuff",
        bin_variable: "DITTO_CODEBUFF_BIN",
        // Hard-coded below the home directory and not XDG, despite the path.
        // The npm package is a launcher that downloads the real binary into
        // this directory, which is why that download is shared rather than
        // repeated per profile.
        home: Home::Private {
            native: ".config/manicode",
            owned: &[],
        },
        managed_env: &[],
        credentials: &["credentials.json"],
        shared: &["settings.json", "codebuff"],
        sessions: &["projects"],
        login: Some(&["login"]),
        logout: None,
    },
    Spec {
        key: "cn",
        label: "Continue",
        executable: "cn",
        bin_variable: "DITTO_CN_BIN",
        // Current builds have no sign-in at all: Hub authentication was
        // removed and model keys go in `config.yaml`, so there is no login to
        // read. `auth.json` under this directory is where one would return.
        home: Home::Variable {
            variable: "CONTINUE_GLOBAL_DIR",
            native: ".continue",
        },
        managed_env: &[],
        credentials: &[],
        shared: &[
            "config.yaml",
            "rules",
            "prompts",
            "skills",
            "permissions.yaml",
            "settings.json",
            ".continuerc.json",
        ],
        sessions: &["sessions"],
        login: None,
        logout: None,
    },
    Spec {
        key: "command-code",
        label: "Command Code",
        executable: "command-code",
        bin_variable: "DITTO_COMMAND_CODE_BIN",
        // Closed source; every path in the bundle is `HOME`, `USERPROFILE`, or
        // the home directory, and no variable of its own was found.
        home: Home::Private {
            native: ".commandcode",
            owned: &[],
        },
        managed_env: &[],
        credentials: &["auth.json"],
        shared: &[
            "settings.json",
            "providers.json",
            "keybindings.json",
            "skills",
            "agents",
            "commands",
            "mods",
        ],
        sessions: &["projects", "file-history"],
        login: Some(&["login"]),
        logout: Some(&["logout"]),
    },
    Spec {
        key: "hermes",
        label: "Hermes Agent",
        executable: "hermes",
        bin_variable: "DITTO_HERMES_BIN",
        // Hermes falls back to the user's own `auth.json` when `HERMES_HOME`
        // is inside `~/.hermes` or its parent is named `profiles`. A managed
        // profile's directory is neither, which is what keeps that off.
        home: Home::Variable {
            variable: "HERMES_HOME",
            native: ".hermes",
        },
        managed_env: &[],
        credentials: &["auth.json", ".env"],
        shared: &["config.yaml", "SOUL.md", "skills"],
        sessions: &["sessions"],
        login: None,
        logout: None,
    },
    Spec {
        key: "openclaw",
        label: "OpenClaw",
        executable: "openclaw",
        bin_variable: "DITTO_OPENCLAW_BIN",
        // `openclaw.json` carries `auth.profiles` beside the preferences, so it
        // stays with the account rather than being shared.
        home: Home::Variable {
            variable: "OPENCLAW_STATE_DIR",
            native: ".openclaw",
        },
        managed_env: &[],
        credentials: &["credentials", "secrets.json", ".env"],
        shared: &["skills"],
        sessions: &["agents"],
        login: Some(&["models", "auth", "login"]),
        logout: None,
    },
    Spec {
        key: "vibe",
        label: "Mistral Vibe",
        executable: "vibe",
        bin_variable: "DITTO_VIBE_BIN",
        home: Home::Variable {
            variable: "VIBE_HOME",
            native: ".vibe",
        },
        // The key goes to the OS keyring unless the keyring is refused, in
        // which case it is written to `.env` here. The variable is named for
        // Vibe's tests but is how its source decides.
        managed_env: &[("VIBE_TEST_DISABLE_KEYRING", "1")],
        credentials: &[".env"],
        shared: &[
            "config.toml",
            "agents",
            "prompts",
            "skills",
            "tools",
            "plugins",
            "hooks.toml",
            "AGENTS.md",
            "trusted_folders.toml",
        ],
        sessions: &["logs/session"],
        login: None,
        logout: None,
    },
    Spec {
        key: "acli",
        label: "Rovo Dev",
        // Rovo Dev is a plugin of Atlassian's `acli` (`acli rovodev run`) with
        // no binary of its own. It keeps its files in `~/.rovodev` and its
        // login in `~/.acli`, both hard-coded, so both stay out of the mirror.
        // What `~/.acli` holds is undocumented.
        executable: "acli",
        bin_variable: "DITTO_ACLI_BIN",
        home: Home::Private {
            native: ".rovodev",
            owned: &[".acli"],
        },
        managed_env: &[],
        credentials: &["../.acli"],
        shared: &["config.yml", "AGENTS.md"],
        sessions: &["sessions"],
        login: Some(&["rovodev", "auth", "login"]),
        logout: Some(&["rovodev", "auth", "logout"]),
    },
    Spec {
        key: "amp",
        label: "Amp",
        executable: "amp",
        bin_variable: "DITTO_AMP_BIN",
        // Read out of the shipped bundle rather than documented: settings
        // under the config base, `secrets.json` under the data base. On macOS
        // its daemon and IDE state ignore the data base and stay under the
        // home directory, and threads live on ampcode.com rather than on disk.
        home: Home::Xdg { subdir: "amp" },
        managed_env: &[],
        credentials: &["data/secrets.json"],
        shared: &[
            "config/settings.json",
            "config/settings.jsonc",
            "config/plugins",
            "config/skills",
            "config/AGENTS.md",
        ],
        sessions: &[],
        login: Some(&["login"]),
        logout: Some(&["logout"]),
    },
    Spec {
        key: "droid",
        label: "Droid",
        executable: "droid",
        bin_variable: "DITTO_DROID_BIN",
        // Closed source with no variable of its own. The login goes to the OS
        // keyring unless refused; where the fallback file lands is not
        // documented, so the login still cannot be read.
        home: Home::Private {
            native: ".factory",
            owned: &[],
        },
        managed_env: &[("FACTORY_DISABLE_KEYRING", "1")],
        credentials: &[],
        shared: &[
            "settings.json",
            "hooks.json",
            "config.json",
            "commands",
            "droids",
            "skills",
            "AGENTS.md",
            "specs",
            "docs",
        ],
        sessions: &["projects"],
        login: None,
        logout: None,
    },
    Spec {
        key: "goose",
        label: "Goose",
        executable: "goose",
        bin_variable: "DITTO_GOOSE_BIN",
        // `GOOSE_PATH_ROOT` would move the same directories, but Goose's
        // built-in MCP servers read the XDG bases directly and ignore it, so
        // the bases are what a profile pins. Every secret sits in one keyring
        // item unless the keyring is refused, in which case `secrets.yaml`
        // holds them.
        home: Home::Xdg { subdir: "goose" },
        managed_env: &[("GOOSE_DISABLE_KEYRING", "1")],
        credentials: &["config/secrets.yaml"],
        shared: &[
            "config/config.yaml",
            "config/permission.yaml",
            "config/permissions",
            "config/prompts",
            "config/recipes",
            "config/skills",
            "config/agents",
            "config/custom_providers",
            "config/.goosehints",
            "config/AGENTS.md",
        ],
        sessions: &["data/sessions"],
        login: None,
        logout: None,
    },
    Spec {
        key: "aider",
        label: "Aider",
        executable: "aider",
        bin_variable: "DITTO_AIDER_BIN",
        // Keeps `~/.aider.conf.yml`, `~/.env`, and `~/.aider/` beside each
        // other with no variable for any of them. The mirror carries the
        // configuration files across; `.env` holds API keys and stays behind.
        // Chat and input histories live in the repository, not the home.
        home: Home::Private {
            native: ".aider",
            owned: &[".env"],
        },
        managed_env: &[],
        credentials: &["oauth-keys.env"],
        shared: &[],
        sessions: &[],
        login: None,
        logout: None,
    },
    Spec {
        key: "crush",
        label: "Crush",
        executable: "crush",
        bin_variable: "DITTO_CRUSH_BIN",
        // `CRUSH_GLOBAL_CONFIG` and `CRUSH_GLOBAL_DATA` move only the two
        // `crush.json` files; the bases move everything. Conversations live in
        // the project's `.crush/` directory rather than the home.
        home: Home::Xdg { subdir: "crush" },
        managed_env: &[],
        credentials: &["data/crush.json"],
        shared: &[
            "config/crush.json",
            "config/crushrc",
            "config/CRUSH.md",
            "config/skills",
        ],
        sessions: &[],
        login: Some(&["login"]),
        logout: Some(&["logout"]),
    },
    Spec {
        key: "kilo",
        label: "Kilo Code",
        executable: "kilo",
        bin_variable: "DITTO_KILO_BIN",
        // An opencode fork with opencode's split: configuration under one base
        // and credentials under another, so the configuration directory is
        // shared whole, as opencode's is. `KILO_CONFIG_DIR` only adds a
        // directory to the ones read.
        home: Home::Xdg { subdir: "kilo" },
        managed_env: &[],
        credentials: &["data/auth.json"],
        shared: &["config"],
        sessions: &["data/storage"],
        login: Some(&["auth", "login"]),
        logout: Some(&["auth", "logout"]),
    },
    Spec {
        key: "kiro-cli",
        label: "Kiro",
        executable: "kiro-cli",
        bin_variable: "DITTO_KIRO_CLI_BIN",
        // `KIRO_HOME` moves `~/.kiro` but not the data directory holding the
        // login, which only an undocumented variable moves and which is under
        // `Library/Application Support` on macOS and the XDG data base
        // elsewhere. Both are derived from the home directory, so a private
        // home moves everything at once, with those two kept out of the mirror.
        home: Home::Private {
            native: ".kiro",
            owned: &[
                ".local/share/kiro-cli",
                "Library/Application Support/kiro-cli",
            ],
        },
        managed_env: &[],
        credentials: &[
            "../Library/Application Support/kiro-cli/data.sqlite3",
            "../.local/share/kiro-cli/data.sqlite3",
        ],
        shared: &[
            "settings/cli.json",
            "settings/permissions.yaml",
            "agents",
            "steering",
            "skills",
            "hooks",
            "prompts",
            "powers",
        ],
        sessions: &["sessions"],
        login: Some(&["login"]),
        logout: Some(&["logout"]),
    },
    Spec {
        key: "auggie",
        label: "Auggie",
        executable: "auggie",
        bin_variable: "DITTO_AUGGIE_BIN",
        // `--augment-cache-dir` moves the session and the sessions, but the
        // rules, commands, and skills stay under the home directory.
        home: Home::Private {
            native: ".augment",
            owned: &[],
        },
        managed_env: &[],
        credentials: &["session.json"],
        shared: &[
            "settings.json",
            "commands",
            "rules",
            "skills",
            "agents",
            "plugins",
        ],
        sessions: &["sessions"],
        login: Some(&["login"]),
        logout: Some(&["logout"]),
    },
    Spec {
        key: "agy",
        label: "Antigravity",
        executable: "agy",
        bin_variable: "DITTO_AGY_BIN",
        // Closed source and derived from the home directory alone, with
        // nothing honoured from the environment. The Google login goes to the
        // OS keychain with no way to refuse it, so a profile isolates the
        // configuration and conversations while the login stays the user's;
        // only `GEMINI_API_KEY` is profile-local. `~/.gemini/config` and
        // `GEMINI.md` are shared with the Antigravity IDE and reach the
        // profile through the mirror.
        home: Home::Private {
            native: ".gemini/antigravity-cli",
            owned: &[],
        },
        managed_env: &[],
        credentials: &[],
        shared: &["settings.json", "keybindings.json", "skills", "plugins"],
        sessions: &["conversations", "brain"],
        login: None,
        logout: None,
    },
    Spec {
        key: "mimo",
        label: "MiMo Code",
        executable: "mimo",
        bin_variable: "DITTO_MIMO_BIN",
        // An opencode fork. `MIMOCODE_HOME` would move the cache along with
        // everything else, so the bases are pinned instead, as for opencode.
        home: Home::Xdg { subdir: "mimocode" },
        managed_env: &[],
        credentials: &["data/auth.json"],
        shared: &["config"],
        sessions: &["data/storage", "data/snapshot"],
        login: Some(&["providers", "login"]),
        logout: Some(&["providers", "logout"]),
    },
    Spec {
        key: "ante",
        label: "Ante",
        executable: "ante",
        bin_variable: "DITTO_ANTE_BIN",
        home: Home::Variable {
            variable: "ANTE_HOME",
            native: ".ante",
        },
        managed_env: &[],
        credentials: &["auth"],
        // `models` holds downloaded local models, shared so that a profile
        // does not download them again.
        shared: &[
            "settings.json",
            "catalog.json",
            "AGENTS.md",
            "skills",
            "agents",
            "offline-config.json",
            "verified_models.json",
            "models",
        ],
        sessions: &["sessions", "projects"],
        login: Some(&["auth", "login"]),
        logout: None,
    },
    Spec {
        key: "traecli",
        label: "Trae",
        executable: "traecli",
        bin_variable: "DITTO_TRAECLI_BIN",
        // The least certain entry. Trae's documentation describes a 1.0 layout
        // with no variable; integrators of the current release agree on
        // `~/.trae` with `TRAE_HOME` moving it, but that is unofficial, and
        // the installer could not be reached to check. A private home moves
        // whatever the binary derives from the home directory. The SSO token
        // may also be in the OS keyring, which a profile does not isolate.
        home: Home::Private {
            native: ".trae",
            owned: &[],
        },
        managed_env: &[],
        credentials: &["cli/auth.json"],
        shared: &[],
        sessions: &["cli/sessions", "cli/archived_sessions"],
        login: None,
        logout: None,
    },
    Spec {
        key: "autohand",
        label: "Autohand",
        executable: "autohand",
        bin_variable: "DITTO_AUTOHAND_BIN",
        // The login sits in `config.json` beside the settings, so neither can
        // be shared nor read for a sign-in state.
        home: Home::Variable {
            variable: "AUTOHAND_HOME",
            native: ".autohand",
        },
        managed_env: &[],
        credentials: &[],
        shared: &[
            "AGENTS.md",
            "skills",
            "commands",
            "agents",
            "tools",
            "extensions",
            "plans",
            "themes",
            "hooks",
        ],
        sessions: &["sessions"],
        login: Some(&["--login"]),
        logout: Some(&["--logout"]),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_unique_and_match_what_a_shell_can_call() {
        // A key is a subcommand, a shell function name, and a directory name at
        // once, so it has to be something all three accept, and no two tools
        // can answer to the same one.
        let mut seen = std::collections::HashSet::new();
        for spec in ALL {
            assert!(seen.insert(spec.key), "duplicate key {}", spec.key);
            assert!(
                spec.key
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "key {} is not a plain command name",
                spec.key
            );
            assert_eq!(
                spec.bin_variable,
                format!("DITTO_{}_BIN", spec.key.to_uppercase().replace('-', "_")),
                "{} names its override differently from every other tool",
                spec.key
            );
            let paths = spec
                .credentials
                .iter()
                .chain(spec.shared)
                .chain(spec.sessions);
            match spec.home {
                Home::Xdg { .. } => {
                    for path in paths {
                        assert!(
                            ["config", "data", "state"]
                                .iter()
                                .any(|base| path == base || path.starts_with(&format!("{base}/"))),
                            "{}: XDG path {path} must name its base first",
                            spec.key
                        );
                    }
                }
                Home::Variable { native, .. } | Home::Parent { native, .. } => {
                    assert!(!native.is_empty(), "{} has no native directory", spec.key);
                }
                Home::Private { native, owned } => {
                    assert!(!native.is_empty(), "{} has no native directory", spec.key);
                    for entry in owned {
                        assert!(!entry.is_empty(), "{} owns an empty path", spec.key);
                    }
                }
            }
        }
    }

    /// The label is drawn in a fixed-width column of the picker, and one that
    /// overflows it pushes the state off the row.
    #[test]
    fn labels_fit_the_pickers_tool_column() {
        for spec in ALL {
            assert!(
                spec.label.chars().count() <= 13,
                "{} is too wide",
                spec.label
            );
        }
    }
}
