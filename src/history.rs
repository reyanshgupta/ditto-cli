//! One-time conversation backfills for profiles that should start with the
//! history a person already had without sharing future account activity.

use std::{
    collections::HashSet,
    ffi::OsString,
    fs,
    path::Path,
    process::{self, Stdio},
};

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OpenFlags};

use crate::{
    launch::{self, Tool},
    profile::{Profile, secure_directory, write_private_bytes},
    tools,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Counts {
    pub copied: usize,
    pub kept: usize,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Backfilled {
    pub tools: Vec<(Tool, Counts)>,
}

impl Backfilled {
    pub fn changed(&self) -> bool {
        self.tools.iter().any(|(_, counts)| counts.copied > 0)
    }
}

/// Copies resumable conversations and their tool-owned artifacts without
/// replacing anything the target profile already wrote.
///
/// Linking these paths would make future client and account conversations
/// visible everywhere. A copy keeps the history available at the moment the
/// user asks for it, then lets each profile diverge safely.
pub fn backfill(source: &Profile, target: &Profile) -> Result<Backfilled> {
    let mut result = Backfilled::default();

    let mut claude = Counts::default();
    for name in ["projects", "file-history", "tasks"] {
        copy_tree(
            &source.claude_home.join(name),
            &target.claude_home.join(name),
            &mut claude,
        )?;
    }
    result.tools.push((Tool::Claude, claude));

    let mut codex = Counts::default();
    for name in ["sessions", "archived_sessions", "generated_images"] {
        copy_tree(
            &source.codex_home.join(name),
            &target.codex_home.join(name),
            &mut codex,
        )?;
    }
    result.tools.push((Tool::Codex, codex));

    let mut fx = Counts::default();
    copy_tree(
        &source.fx_dir().join("sessions"),
        &target.fx_dir().join("sessions"),
        &mut fx,
    )?;
    result.tools.push((Tool::Fx, fx));

    result
        .tools
        .push((Tool::Opencode, backfill_opencode(source, target)?));

    let mut omp = Counts::default();
    for name in ["sessions", "blobs"] {
        copy_tree(
            &source.omp_home.join(name),
            &target.omp_home.join(name),
            &mut omp,
        )?;
    }
    result.tools.push((Tool::Omp, omp));

    let mut prime_agent = Counts::default();
    for name in ["sessions", "session-artifacts"] {
        copy_tree(
            &source.prime_agent_home.join(name),
            &target.prime_agent_home.join(name),
            &mut prime_agent,
        )?;
    }
    result.tools.push((Tool::PrimeAgent, prime_agent));

    result.tools.push((Tool::Pi, backfill_pi(source, target)?));
    // Every entry here is printed, so the table's tools appear only when they
    // had something to copy; thirty lines saying nothing moved would bury the
    // ones that did.
    for spec in tools::ALL {
        let mut counts = Counts::default();
        for name in spec.sessions {
            copy_tree(
                &source.tool_path(spec, name),
                &target.tool_path(spec, name),
                &mut counts,
            )?;
        }
        if counts.copied + counts.kept > 0 {
            result.tools.push((Tool::Generic(spec), counts));
        }
    }
    Ok(result)
}

fn copy_tree(from: &Path, into: &Path, result: &mut Counts) -> Result<()> {
    if !from.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(into).with_context(|| format!("could not create {}", into.display()))?;
    secure_directory(into)?;

    for entry in fs::read_dir(from).with_context(|| format!("could not read {}", from.display()))? {
        let entry =
            entry.with_context(|| format!("could not read an entry in {}", from.display()))?;
        let source = entry.path();
        let target = into.join(entry.file_name());
        let kind = entry
            .file_type()
            .with_context(|| format!("could not inspect {}", source.display()))?;
        if kind.is_dir() {
            copy_tree(&source, &target, result)?;
        } else if kind.is_file() {
            copy_file(&source, &target, result)?;
        }
    }
    Ok(())
}

fn copy_file(source: &Path, target: &Path, result: &mut Counts) -> Result<()> {
    match fs::symlink_metadata(target) {
        Ok(_) => {
            result.kept += 1;
            return Ok(());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("could not inspect {}", target.display()));
        }
    }
    let contents =
        fs::read(source).with_context(|| format!("could not read {}", source.display()))?;
    write_private_bytes(target, &contents)?;
    result.copied += 1;
    Ok(())
}

/// Pi's native store nests sessions by project, while Ditto gives managed
/// profiles one explicit flat session directory. Only the direct JSONL files
/// under each project are resumable; deeper files belong to extensions and
/// subagents.
fn backfill_pi(source: &Profile, target: &Profile) -> Result<Counts> {
    let from = source.pi_home.join("sessions");
    if !from.is_dir() {
        return Ok(Counts::default());
    }
    let into = target.pi_home.join("sessions");
    fs::create_dir_all(&into).with_context(|| format!("could not create {}", into.display()))?;
    secure_directory(&into)?;

    let mut result = Counts::default();
    copy_pi_sessions(&from, &into, true, &mut result)?;
    Ok(result)
}

fn copy_pi_sessions(from: &Path, into: &Path, descend: bool, result: &mut Counts) -> Result<()> {
    for entry in fs::read_dir(from).with_context(|| format!("could not read {}", from.display()))? {
        let entry =
            entry.with_context(|| format!("could not read an entry in {}", from.display()))?;
        let source = entry.path();
        let kind = entry
            .file_type()
            .with_context(|| format!("could not inspect {}", source.display()))?;
        if kind.is_dir() {
            if descend {
                copy_pi_sessions(&source, into, false, result)?;
            }
            continue;
        }
        if kind.is_file()
            && source
                .extension()
                .is_some_and(|extension| extension == "jsonl")
        {
            copy_file(&source, &into.join(entry.file_name()), result)?;
        }
    }
    Ok(())
}

/// opencode stores sessions beside credentials in SQLite, so copying its data
/// directory would leak accounts and copying the live database would replace
/// target sessions. Its own export/import commands provide the safe boundary.
fn backfill_opencode(source: &Profile, target: &Profile) -> Result<Counts> {
    let source_database = source.opencode.data_dir().join("opencode.db");
    if !source_database.is_file() {
        return Ok(Counts::default());
    }
    let source_ids = opencode_session_ids(&source_database)?;
    let target_ids = opencode_session_ids(&target.opencode.data_dir().join("opencode.db"))?;
    let mut result = Counts {
        copied: 0,
        kept: source_ids.intersection(&target_ids).count(),
    };
    let missing = source_ids.difference(&target_ids).collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(result);
    }

    // Alternating export and import can make opencode's global server answer
    // the next export against the target store. Finish every source read first,
    // then start writing the target.
    let temporary = target
        .opencode
        .data
        .join(format!(".ditto-history-{}", process::id()));
    fs::create_dir(&temporary)
        .with_context(|| format!("could not create {}", temporary.display()))?;
    secure_directory(&temporary)?;
    let backfilled = (|| {
        let mut exports = Vec::with_capacity(missing.len());
        for (index, id) in missing.into_iter().enumerate() {
            let path = temporary.join(format!("{index}.json"));
            export_opencode_session(source, id, &path)?;
            exports.push((id, path));
        }
        for (id, path) in exports {
            import_opencode_session(target, id, &path)?;
            result.copied += 1;
        }
        Ok(result)
    })();
    let _ = fs::remove_dir_all(&temporary);
    backfilled
}

fn opencode_session_ids(database: &Path) -> Result<HashSet<String>> {
    if !database.is_file() {
        return Ok(HashSet::new());
    }
    let connection = Connection::open_with_flags(database, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("could not read {}", database.display()))?;
    let mut statement = connection
        .prepare("SELECT id FROM session")
        .with_context(|| format!("could not read sessions from {}", database.display()))?;
    let ids = statement
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<HashSet<String>>>()?;
    Ok(ids)
}

fn import_opencode_session(target: &Profile, id: &str, path: &Path) -> Result<()> {
    let imported = launch::build_command(
        Tool::Opencode,
        target,
        &[
            OsString::from("--pure"),
            OsString::from("import"),
            path.as_os_str().to_owned(),
        ],
    )
    .stdin(Stdio::null())
    .output()
    .with_context(|| format!("could not import opencode session {id}"))?;
    if !imported.status.success() {
        bail!(
            "could not import opencode session {id}: {}",
            String::from_utf8_lossy(&imported.stderr).trim()
        );
    }
    Ok(())
}

/// opencode's Bun process can report success before a large export is fully
/// flushed. Validation and retries keep a partial snapshot out of the target.
fn export_opencode_session(source: &Profile, id: &str, path: &Path) -> Result<()> {
    let mut last_error = String::new();
    for _ in 0..3 {
        write_private_bytes(path, &[])?;
        let output = fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(path)
            .with_context(|| format!("could not open {}", path.display()))?;
        let exported = launch::build_command(
            Tool::Opencode,
            source,
            &[
                OsString::from("--pure"),
                OsString::from("export"),
                OsString::from(id),
            ],
        )
        .stdin(Stdio::null())
        // Bun can exit before a large piped stdout is fully flushed. A regular
        // file gives its exporter the durable sink it expects.
        .stdout(Stdio::from(output))
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("could not export opencode session {id}"))?;
        if !exported.status.success() {
            last_error = String::from_utf8_lossy(&exported.stderr).trim().to_owned();
            continue;
        }
        let contents =
            fs::read(path).with_context(|| format!("could not read {}", path.display()))?;
        match serde_json::from_slice::<serde_json::Value>(&contents) {
            Ok(_) => return Ok(()),
            Err(error) => last_error = error.to_string(),
        }
    }
    bail!("could not export opencode session {id}: {last_error}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{DEFAULT_PROFILE, Store};

    #[test]
    fn backfills_file_sessions_without_replacing_profile_history() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let store = Store::new(
            temporary.path().join("ditto"),
            temporary.path().join("home"),
        );
        let source = store.load_profile(DEFAULT_PROFILE)?;
        let target = store.create_profile("work")?;
        fs::create_dir_all(source.claude_home.join("projects/repo"))?;
        fs::create_dir_all(target.claude_home.join("projects/repo"))?;
        fs::write(source.claude_home.join("projects/repo/old.jsonl"), "base")?;
        fs::write(source.claude_home.join("projects/repo/same.jsonl"), "base")?;
        fs::write(
            target.claude_home.join("projects/repo/same.jsonl"),
            "profile",
        )?;

        let backfilled = backfill(&source, &target)?;
        let claude = backfilled
            .tools
            .iter()
            .find(|(tool, _)| *tool == Tool::Claude)
            .unwrap()
            .1;

        assert_eq!(claude, Counts { copied: 1, kept: 1 });
        assert_eq!(
            fs::read_to_string(target.claude_home.join("projects/repo/old.jsonl"))?,
            "base"
        );
        assert_eq!(
            fs::read_to_string(target.claude_home.join("projects/repo/same.jsonl"))?,
            "profile"
        );
        Ok(())
    }

    #[test]
    fn flattens_only_resumable_pi_sessions() -> Result<()> {
        let temporary = tempfile::tempdir()?;
        let store = Store::new(
            temporary.path().join("ditto"),
            temporary.path().join("home"),
        );
        let source = store.load_profile(DEFAULT_PROFILE)?;
        let target = store.create_profile("work")?;
        let source_sessions = source.pi_home.join("sessions/project");
        fs::create_dir_all(source_sessions.join("subagent"))?;
        fs::write(source_sessions.join("chat.jsonl"), "chat")?;
        fs::write(source_sessions.join("artifact.md"), "artifact")?;
        fs::write(source_sessions.join("subagent/session.jsonl"), "subagent")?;

        let backfilled = backfill(&source, &target)?;
        let pi = backfilled
            .tools
            .iter()
            .find(|(tool, _)| *tool == Tool::Pi)
            .unwrap()
            .1;
        let target_sessions = target.pi_home.join("sessions");

        assert_eq!(pi, Counts { copied: 1, kept: 0 });
        assert!(target_sessions.join("chat.jsonl").is_file());
        assert!(!target_sessions.join("artifact.md").exists());
        assert!(!target_sessions.join("session.jsonl").exists());
        Ok(())
    }
}
