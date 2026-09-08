//! Desktop project definitions live beside account and conversation state in
//! Codex's global state file. Sharing that file would defeat profile isolation,
//! so only the fields needed to reopen local folders can travel between homes.
//! The legacy project import at startup was verified in ChatGPT Desktop
//! 26.901.51231; its server IDs and migration bookkeeping belong to each home.

use std::collections::BTreeSet;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{
    profile::{Profile, write_private_file},
    settings,
};

const FILE: &str = ".codex-global-state.json";
const PROJECTS: &str = "local-projects";

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Project {
    id: String,
    name: String,
    root_paths: Vec<String>,
    created_at: u64,
    updated_at: u64,
}

/// Existing desktop processes retain this file in memory and overwrite outside
/// edits. Automatic seeding therefore only writes a new state file; merging an
/// existing one is reserved for an explicit sync with the desktop app closed.
pub fn copy(source: &Profile, target: &Profile, seed_only: bool) -> Result<Vec<String>> {
    let from = source.codex_home.join(FILE);
    let path = target.codex_home.join(FILE);
    if source.codex_home == target.codex_home || (seed_only && path.try_exists()?) {
        return Ok(Vec::new());
    }
    if path.is_symlink() {
        bail!(
            "{} is a symbolic link; keep desktop account state private before syncing projects",
            path.display()
        );
    }
    let source = settings::read(&from)?;
    let Some(projects) = source.get(PROJECTS) else {
        return Ok(Vec::new());
    };
    let projects = projects.as_object().with_context(|| {
        format!(
            "{PROJECTS} in {} does not hold a JSON object",
            from.display()
        )
    })?;
    if projects.is_empty() {
        return Ok(Vec::new());
    }
    let mut state = settings::read(&path)?;
    let mut existing = match state.get(PROJECTS) {
        Some(Value::Object(projects)) => projects.clone(),
        None => Map::new(),
        Some(_) => bail!(
            "{PROJECTS} in {} does not hold a JSON object",
            path.display()
        ),
    };
    let mut order = match state.get("project-order") {
        Some(Value::Array(order)) => order.clone(),
        None => Vec::new(),
        Some(_) => bail!("project-order in {} does not hold an array", path.display()),
    };
    let mut copied = Vec::new();
    for (id, value) in projects {
        if existing.contains_key(id) {
            continue;
        }
        let project: Project = serde_json::from_value(value.clone())
            .with_context(|| format!("invalid desktop project '{id}' in {}", from.display()))?;
        if project.id != *id || project.root_paths.is_empty() {
            bail!("invalid desktop project '{id}' in {}", from.display());
        }
        // A profile may already have added the same folders under its own name
        // and ID. Keep that choice instead of creating a second sidebar entry.
        let roots: BTreeSet<_> = project.root_paths.iter().map(String::as_str).collect();
        if existing.values().any(|entry| {
            entry
                .get("rootPaths")
                .and_then(Value::as_array)
                .is_some_and(|paths| {
                    paths
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<BTreeSet<_>>()
                        == roots
                })
        }) {
            continue;
        }
        copied.push(project.name.clone());
        existing.insert(id.clone(), serde_json::to_value(project)?);
        if !order.contains(&Value::String(id.clone())) {
            order.push(Value::String(id.clone()));
        }
    }
    if !copied.is_empty() {
        // Account-scoped sidebar atoms, server project IDs, thread assignments,
        // and future fields stay in their own home. The desktop app imports
        // these legacy local definitions into its own project store at startup.
        state.insert(PROJECTS.to_owned(), Value::Object(existing));
        state.insert("project-order".to_owned(), Value::Array(order));
        let mut contents = serde_json::to_string_pretty(&state)
            .context("could not serialize desktop project state")?;
        contents.push('\n');
        write_private_file(&path, &contents)?;
    }
    Ok(copied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{DEFAULT_PROFILE, Store};
    use serde_json::json;
    use std::fs;
    use tempfile::tempdir;

    fn project(id: &str, roots: &[&str]) -> Value {
        json!({"id": id, "name": id, "rootPaths": roots, "createdAt": 1, "updatedAt": 2})
    }

    #[test]
    fn carries_only_local_folder_definitions_and_preserves_private_target_state() -> Result<()> {
        let tmp = tempdir()?;
        let store = Store::new(tmp.path().join("ditto"), tmp.path().join("home"));
        let source = store.load_profile(DEFAULT_PROFILE)?;
        let target = store.create_profile("work")?;
        fs::create_dir_all(&source.codex_home)?;
        let mut entry = project("sharepi", &["/repos/pi", "/repos/core"]);
        entry["futureAccountState"] = json!("must not travel");
        fs::write(source.codex_home.join(FILE), json!({
            PROJECTS: {"sharepi": entry}, "electron-persisted-atom-state": {"account": "source"},
            "thread-project-assignments": {"private-thread": "sharepi"},
            "app-server-project-id-by-legacy-project-id-by-host": {"source": "private-id"}
        }).to_string())?;
        let private = json!({"electron-persisted-atom-state": {"account": "target"},
            "selected-project": "mine", "thread-project-assignments": {"my-thread": "mine"}});
        fs::write(target.codex_home.join(FILE), private.to_string())?;
        assert_eq!(copy(&source, &target, false)?, ["sharepi"]);
        let actual = settings::read(&target.codex_home.join(FILE))?;
        for (key, value) in private.as_object().unwrap() {
            assert_eq!(&actual[key], value);
        }
        assert_eq!(
            actual[PROJECTS]["sharepi"],
            project("sharepi", &["/repos/pi", "/repos/core"])
        );
        assert!(!actual.contains_key("app-server-project-id-by-legacy-project-id-by-host"));
        assert_eq!(actual["project-order"], json!(["sharepi"]));
        let before = fs::read(target.codex_home.join(FILE))?;
        assert!(copy(&source, &target, false)?.is_empty());
        assert_eq!(fs::read(target.codex_home.join(FILE))?, before);
        Ok(())
    }

    #[test]
    fn preserves_existing_ids_and_folders_even_when_the_names_differ() -> Result<()> {
        let tmp = tempdir()?;
        let store = Store::new(tmp.path().join("ditto"), tmp.path().join("home"));
        let source = store.load_profile(DEFAULT_PROFILE)?;
        let target = store.create_profile("work")?;
        fs::create_dir_all(&source.codex_home)?;
        fs::write(
            source.codex_home.join(FILE),
            json!({PROJECTS: {
                "same-id": project("same-id", &["/new"]),
                "other-id": project("other-id", &["/two", "/one"])
            }})
            .to_string(),
        )?;
        let before = json!({PROJECTS: {
            "same-id": project("same-id", &["/original"]),
            "mine": project("mine", &["/one", "/two"])
        }, "project-order": ["mine", "same-id"]})
        .to_string();
        fs::write(target.codex_home.join(FILE), &before)?;
        assert!(copy(&source, &target, false)?.is_empty());
        assert_eq!(fs::read_to_string(target.codex_home.join(FILE))?, before);
        Ok(())
    }

    #[test]
    fn seeds_new_homes_without_rewriting_state_a_running_desktop_may_own() -> Result<()> {
        let tmp = tempdir()?;
        let store = Store::new(tmp.path().join("ditto"), tmp.path().join("home"));
        let source = store.load_profile(DEFAULT_PROFILE)?;
        let target = store.create_profile("work")?;
        assert!(copy(&source, &target, true)?.is_empty());
        assert!(!target.codex_home.join(FILE).exists());
        fs::create_dir_all(&source.codex_home)?;
        fs::write(
            source.codex_home.join(FILE),
            json!({PROJECTS: {"pi": project("pi", &["/pi"])}}).to_string(),
        )?;
        assert_eq!(copy(&source, &target, true)?, ["pi"]);
        fs::write(target.codex_home.join(FILE), "{}").unwrap();
        assert!(copy(&source, &target, true)?.is_empty());
        assert_eq!(fs::read_to_string(target.codex_home.join(FILE))?, "{}");
        assert!(copy(&source, &source, false)?.is_empty());
        Ok(())
    }

    #[test]
    fn refuses_malformed_state_without_writing_a_partial_merge() -> Result<()> {
        let tmp = tempdir()?;
        let store = Store::new(tmp.path().join("ditto"), tmp.path().join("home"));
        let source = store.load_profile(DEFAULT_PROFILE)?;
        let target = store.create_profile("work")?;
        fs::create_dir_all(&source.codex_home)?;
        fs::write(
            source.codex_home.join(FILE),
            json!({PROJECTS: {
                "good": project("good", &["/good"]), "bad": {"name": "bad"}
            }})
            .to_string(),
        )?;
        fs::write(target.codex_home.join(FILE), "{}")?;
        assert!(copy(&source, &target, false).is_err());
        assert_eq!(fs::read_to_string(target.codex_home.join(FILE))?, "{}");
        fs::write(
            source.codex_home.join(FILE),
            json!({PROJECTS: {"good": project("good", &["/good"])}}).to_string(),
        )?;
        for invalid in [
            "broken",
            "[]",
            "{\"local-projects\": []}",
            "{\"project-order\": {}}",
        ] {
            fs::write(target.codex_home.join(FILE), invalid)?;
            assert!(copy(&source, &target, false).is_err());
            assert_eq!(fs::read_to_string(target.codex_home.join(FILE))?, invalid);
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn refuses_a_link_to_another_accounts_desktop_state() -> Result<()> {
        let tmp = tempdir()?;
        let store = Store::new(tmp.path().join("ditto"), tmp.path().join("home"));
        let source = store.load_profile(DEFAULT_PROFILE)?;
        let target = store.create_profile("work")?;
        fs::create_dir_all(&source.codex_home)?;
        fs::write(source.codex_home.join(FILE), "{\"account\": \"private\"}")?;
        std::os::unix::fs::symlink(source.codex_home.join(FILE), target.codex_home.join(FILE))?;
        assert!(copy(&source, &target, false).is_err());
        assert!(target.codex_home.join(FILE).is_symlink());
        assert_eq!(
            fs::read_to_string(source.codex_home.join(FILE))?,
            "{\"account\": \"private\"}"
        );
        Ok(())
    }
}
