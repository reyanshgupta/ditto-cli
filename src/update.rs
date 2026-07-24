use std::{
    cmp::Ordering,
    env,
    ffi::OsString,
    path::PathBuf,
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail};
use directories::BaseDirs;

const CRATE_NAME: &str = env!("CARGO_PKG_NAME");
const INSTALLED_VERSION: &str = env!("CARGO_PKG_VERSION");
const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

pub fn run(check: bool, from_git: bool) -> Result<()> {
    println!("Installed  {INSTALLED_VERSION}");

    if check {
        let published = published_version()?;
        println!("Published  {published}");
        match compare(INSTALLED_VERSION, &published) {
            Some(Ordering::Less) => {
                println!();
                println!("Run `ditto-cli update` to install {published}.");
            }
            Some(Ordering::Equal) => println!("\nDitto CLI is up to date."),
            Some(Ordering::Greater) => {
                println!("\nThis build is newer than the published release.")
            }
            None => println!("\nVersions could not be compared."),
        }
        return Ok(());
    }

    // crates.io is only consulted to skip needless work. A lookup failure,
    // typically no network, must not stop an explicit update.
    if !from_git {
        match published_version() {
            Ok(published) => {
                println!("Published  {published}");
                match compare(INSTALLED_VERSION, &published) {
                    Some(Ordering::Equal) => {
                        println!("\nDitto CLI is already up to date.");
                        return Ok(());
                    }
                    Some(Ordering::Greater) => {
                        println!("\nThis build is newer than the published release.");
                        println!("Nothing to install. Use --git to reinstall from source.");
                        return Ok(());
                    }
                    _ => {}
                }
            }
            Err(error) => eprintln!("ditto-cli: could not reach crates.io: {error:#}"),
        }
    }

    warn_about_unmanaged_copy();

    let source = if from_git { REPOSITORY } else { "crates.io" };
    println!("\nInstalling from {source} with cargo.");

    let mut command = Command::new(cargo());
    command.arg("install").arg("--force").arg("--locked");
    if from_git {
        command.arg("--git").arg(REPOSITORY);
    } else {
        command.arg(CRATE_NAME);
    }

    let status = command
        .status()
        .with_context(|| format!("could not run `{}`; is Rust installed?", display(cargo())))?;

    if !status.success() {
        if cfg!(windows) {
            bail!(
                "cargo install failed with {status}; Windows cannot overwrite a running \
                 program, so close every Ditto CLI window and run \
                 `cargo install {CRATE_NAME} --force --locked` directly"
            );
        }
        bail!("cargo install failed with {status}");
    }
    Ok(())
}

/// `cargo install` writes into Cargo's own bin directory. A binary taken from
/// a GitHub release lives elsewhere, and updating would shadow it rather than
/// replace it, so that is worth saying before anything is written.
fn warn_about_unmanaged_copy() {
    let Some(current) = env::current_exe()
        .ok()
        .and_then(|path| path.canonicalize().ok())
    else {
        return;
    };
    let cargo_bin = cargo_bin_directory().and_then(|path| path.canonicalize().ok());
    if cargo_bin.as_deref() == current.parent() {
        return;
    }

    eprintln!("ditto-cli: this copy is {}", current.display());
    match cargo_bin {
        Some(directory) => eprintln!(
            "ditto-cli: cargo installs into {}, so the copy above is left as it is",
            directory.display()
        ),
        None => eprintln!("ditto-cli: cargo installs elsewhere, so this copy is left as it is"),
    }
    eprintln!(
        "ditto-cli: replace it yourself, or download the new release from {REPOSITORY}/releases"
    );
}

fn cargo_bin_directory() -> Option<PathBuf> {
    let cargo_home = env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| BaseDirs::new().map(|dirs| dirs.home_dir().join(".cargo")))?;
    Some(cargo_home.join("bin"))
}

fn cargo() -> OsString {
    env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"))
}

fn display(value: OsString) -> String {
    value.to_string_lossy().into_owned()
}

/// Asks cargo for the newest release rather than talking to the registry
/// directly, which keeps Ditto CLI free of an HTTP stack.
fn published_version() -> Result<String> {
    let output = Command::new(cargo())
        .args(["search", CRATE_NAME, "--limit", "1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("could not run `{} search`", display(cargo())))?;

    if !output.status.success() {
        let reason = String::from_utf8_lossy(&output.stderr);
        let reason = reason.trim();
        if reason.is_empty() {
            bail!("`cargo search` failed with {}", output.status);
        }
        bail!("{reason}");
    }

    parse_search_output(&String::from_utf8_lossy(&output.stdout))
        .with_context(|| format!("`cargo search` did not report a version for {CRATE_NAME}"))
}

/// `cargo search` prints one `name = "version"  # description` line per hit.
fn parse_search_output(text: &str) -> Option<String> {
    text.lines()
        .filter_map(|line| line.split_once('='))
        .find(|(name, _)| name.trim() == CRATE_NAME)
        .and_then(|(_, rest)| rest.trim().strip_prefix('"'))
        .and_then(|rest| rest.split('"').next())
        .filter(|version| !version.is_empty())
        .map(str::to_owned)
}

/// Ditto CLI publishes plain `x.y.z` versions, so anything else is reported as
/// incomparable instead of being ordered incorrectly.
fn compare(installed: &str, published: &str) -> Option<Ordering> {
    Some(parse_version(installed)?.cmp(&parse_version(published)?))
}

fn parse_version(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    parts.next().is_none().then_some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_version_out_of_a_cargo_search_listing() {
        let listing = "ditto-cli = \"0.2.1\"    # A terminal profile switcher\n\
             note: to learn more about a package, run `cargo info <name>`\n";
        assert_eq!(parse_search_output(listing).as_deref(), Some("0.2.1"));
    }

    #[test]
    fn ignores_other_crates_in_a_cargo_search_listing() {
        let listing = "ditto = \"9.9.9\"    # something else\n\
             ditto-cli-extras = \"1.0.0\"    # not this one either\n";
        assert_eq!(parse_search_output(listing), None);
        assert_eq!(parse_search_output("no results"), None);
    }

    #[test]
    fn orders_released_versions() {
        assert_eq!(compare("0.2.0", "0.2.1"), Some(Ordering::Less));
        assert_eq!(compare("0.2.1", "0.2.1"), Some(Ordering::Equal));
        assert_eq!(compare("0.3.0", "0.2.9"), Some(Ordering::Greater));
        assert_eq!(compare("0.10.0", "0.9.0"), Some(Ordering::Greater));
    }

    #[test]
    fn refuses_to_order_versions_it_does_not_model() {
        assert_eq!(compare("0.2.0-rc.1", "0.2.0"), None);
        assert_eq!(compare("0.2", "0.2.0"), None);
        assert_eq!(compare("0.2.0.1", "0.2.0"), None);
    }
}
