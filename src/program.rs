//! Finds the program behind a tool's name.
//!
//! Unix hands the search to `execvp`, which walks `PATH` and needs nothing from
//! Ditto. Windows only ever appends `.exe`, so the `claude.cmd` shims npm
//! writes are invisible to it and a tool sitting plainly on `PATH` looks as
//! though it were never installed. Ditto walks `PATH` itself there, honouring
//! `PATHEXT` the way a command prompt does, and hands the standard library the
//! full path it found. Running a `.cmd` through the command prompt with its
//! arguments escaped is then the standard library's own business.
//!
//! The search is built on every platform even though only Windows calls it.
//! Ditto is developed on Unix, and a rule that can only run where nobody can
//! exercise it is a rule nobody checks.

use std::ffi::{OsStr, OsString};

#[cfg(any(windows, test))]
use std::{
    env,
    path::{Path, PathBuf},
};

/// The extensions a command prompt tries when a name carries none, used only
/// when the environment does not say. Windows always sets `PATHEXT`, so this
/// covers a stripped environment rather than a normal one.
#[cfg(windows)]
const DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";

#[cfg(not(windows))]
pub fn resolve(name: &OsStr) -> OsString {
    name.to_owned()
}

/// Whether a command would start, without starting it. A tool that is not
/// installed has no sign-in state worth asking for, and asking by running it
/// would print the shell's own complaint into a status report.
pub fn installed(name: &OsStr) -> bool {
    let path = std::path::Path::new(name);
    if path.components().count() > 1 {
        return path.is_file();
    }
    #[cfg(windows)]
    {
        std::path::Path::new(&resolve(name)).is_file()
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;
        std::env::var_os("PATH").is_some_and(|paths| {
            std::env::split_paths(&paths).any(|directory| {
                std::fs::metadata(directory.join(name))
                    .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
            })
        })
    }
}

#[cfg(windows)]
pub fn resolve(name: &OsStr) -> OsString {
    let path = env::var_os("PATH").unwrap_or_default();
    let pathext = env::var_os("PATHEXT").unwrap_or_else(|| OsString::from(DEFAULT_PATHEXT));
    search(name, &path, &pathext)
}

/// The full path to the program, or the name unchanged when nothing matches.
/// Handing the name back lets the standard library report a missing program in
/// its own words rather than inventing a second kind of "not found".
#[cfg(any(windows, test))]
fn search(name: &OsStr, path: &OsStr, pathext: &OsStr) -> OsString {
    if name.is_empty() {
        return name.to_owned();
    }

    let found = if names_a_path(name) {
        executable(Path::new(name), pathext)
    } else {
        // Deliberately only `PATH`. `CreateProcess` would also search the
        // current directory, which is how a repository that happens to contain
        // a `claude.bat` gets to run in place of the real one.
        env::split_paths(path)
            .filter(|directory| !directory.as_os_str().is_empty())
            .find_map(|directory| executable(&directory.join(name), pathext))
    };
    found.map_or_else(|| name.to_owned(), PathBuf::into_os_string)
}

/// Whether the name says where to look rather than what to look for. Anything
/// with a directory in front of it, and anything drive relative, is a location;
/// a bare `claude` is not.
#[cfg(any(windows, test))]
fn names_a_path(name: &OsStr) -> bool {
    Path::new(name)
        .parent()
        .is_some_and(|parent| !parent.as_os_str().is_empty())
}

/// The runnable file at this path, extension included. A name that already
/// carries an extension is taken at its word first, the way a command prompt
/// does; failing that every `PATHEXT` extension is tried in turn, so the order
/// the environment gives decides between an executable and a shim of the same
/// name.
#[cfg(any(windows, test))]
fn executable(candidate: &Path, pathext: &OsStr) -> Option<PathBuf> {
    if candidate.extension().is_some() && candidate.is_file() {
        return Some(candidate.to_path_buf());
    }

    pathext
        .to_string_lossy()
        .split(';')
        .map(str::trim)
        .filter(|extension| extension.len() > 1)
        .find_map(|extension| {
            let mut extended = candidate.as_os_str().to_owned();
            extended.push(extension);
            let extended = PathBuf::from(extended);
            extended.is_file().then_some(extended)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lowercased so the search runs the same on a case-sensitive filesystem as
    /// it does on the case-insensitive one Windows would supply. What is under
    /// test is the order and the fallback, not the filesystem's own matching.
    const PATHEXT: &str = ".com;.exe;.bat;.cmd";

    fn program(directory: &Path, name: &str) -> PathBuf {
        let path = directory.join(name);
        std::fs::write(&path, "program").unwrap();
        path
    }

    fn resolved(name: &str, path: &Path) -> OsString {
        search(OsStr::new(name), path.as_os_str(), OsStr::new(PATHEXT))
    }

    #[test]
    fn finds_the_command_shim_npm_installs() {
        let temporary = tempfile::tempdir().unwrap();
        let shim = program(temporary.path(), "claude.cmd");

        // `CreateProcess` only ever looks for `claude.exe`, so without the
        // extension search an installed tool reads as missing.
        assert_eq!(resolved("claude", temporary.path()), shim.as_os_str());
    }

    #[test]
    fn prefers_a_real_executable_to_a_shim() {
        let temporary = tempfile::tempdir().unwrap();
        let executable = program(temporary.path(), "codex.exe");
        program(temporary.path(), "codex.cmd");

        // `PATHEXT` puts `.exe` ahead of `.cmd`, and a native build beats a
        // shim whose only job is to start one.
        assert_eq!(resolved("codex", temporary.path()), executable.as_os_str());
    }

    #[test]
    fn searches_path_in_order() {
        let temporary = tempfile::tempdir().unwrap();
        let first = temporary.path().join("first");
        let second = temporary.path().join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let winner = program(&first, "omp.exe");
        program(&second, "omp.exe");

        let path = env::join_paths([&first, &second]).unwrap();
        assert_eq!(
            search(OsStr::new("omp"), &path, OsStr::new(PATHEXT)),
            winner.as_os_str()
        );
    }

    #[test]
    fn ignores_a_directory_of_the_same_name() {
        let temporary = tempfile::tempdir().unwrap();
        // A `claude` directory on `PATH` is not something to try to run, and
        // moving past it has to leave the real program still findable.
        std::fs::create_dir_all(temporary.path().join("claude.exe")).unwrap();
        let shim = program(temporary.path(), "claude.cmd");

        assert_eq!(resolved("claude", temporary.path()), shim.as_os_str());
    }

    #[test]
    fn takes_an_explicit_path_as_given() {
        let temporary = tempfile::tempdir().unwrap();
        let executable = program(temporary.path(), "omp.exe");
        let nothing = OsStr::new("");

        assert_eq!(
            search(executable.as_os_str(), nothing, OsStr::new(PATHEXT)),
            executable.as_os_str()
        );
        // An override that names the program without its extension, which is
        // how the same setting would be written on any other platform, still
        // resolves.
        assert_eq!(
            search(
                temporary.path().join("omp").as_os_str(),
                nothing,
                OsStr::new(PATHEXT)
            ),
            executable.as_os_str()
        );
    }

    #[test]
    fn hands_back_a_name_nothing_matches() {
        let temporary = tempfile::tempdir().unwrap();

        assert_eq!(
            resolved("opencode", temporary.path()),
            OsStr::new("opencode")
        );
        assert_eq!(resolved("", temporary.path()), OsStr::new(""));
    }

    /// Unix resolves through `execvp`, which reads `PATH` at the moment of the
    /// call. Rewriting the name there would take that away for no gain.
    #[cfg(not(windows))]
    #[test]
    fn leaves_the_name_to_the_operating_system_off_windows() {
        let temporary = tempfile::tempdir().unwrap();
        program(temporary.path(), "claude");

        assert_eq!(resolve(OsStr::new("claude")), OsStr::new("claude"));
    }
}
