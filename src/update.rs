use std::{
    cmp::Ordering,
    env,
    ffi::OsString,
    io::{self, BufRead, BufReader, IsTerminal, Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use crossterm::{
    cursor::MoveToColumn,
    queue,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{Clear, ClearType},
};
use directories::BaseDirs;

const CRATE_NAME: &str = env!("CARGO_PKG_NAME");
const INSTALLED_VERSION: &str = env!("CARGO_PKG_VERSION");
const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

/// Set by the launcher npm installs in front of the binary, since nothing about
/// the running program otherwise says which channel it arrived through.
const INSTALL_SOURCE: &str = "DITTO_INSTALL_SOURCE";

/// Scoped because plain `ditto-cli` on npm was already an unrelated project's.
const NPM_PACKAGE: &str = "@reyanshgupta/ditto-cli";

pub fn run(check: bool, from_git: bool) -> Result<()> {
    println!("Installed  {INSTALLED_VERSION}");

    // `--git` asks for a source build in so many words, and that copy landing
    // in Cargo's bin directory is answered by the warning below, so only the
    // two commands that would otherwise reach for crates.io are intercepted.
    if !from_git && installed_by_npm() {
        report_npm_install();
        return Ok(());
    }

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

    let mut target_version = None;
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
                    _ => target_version = Some(published),
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

    install_with_progress(&mut command, from_git, target_version.as_deref())
}

/// Cargo's complete build log is useful when something breaks, but while an
/// ordinary source build succeeds it hides the only fact the user needs: that
/// the update is still moving. Independent readers drain both output streams
/// so a verbose compiler can never fill either pipe behind the animation.
fn install_with_progress(
    command: &mut Command,
    from_git: bool,
    target_version: Option<&str>,
) -> Result<()> {
    command
        .env("CARGO_TERM_COLOR", "never")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .with_context(|| format!("could not run `{}`; is Rust installed?", display(cargo())))?;
    let cargo_stdout = child.stdout.take().expect("cargo stdout was piped");
    let cargo_stderr = child.stderr.take().expect("cargo stderr was piped");

    let (phase_sender, phase_receiver) = mpsc::channel();
    let stderr_reader = thread::spawn(move || -> io::Result<Vec<u8>> {
        let mut reader = BufReader::new(cargo_stderr);
        let mut output = Vec::new();
        let mut line = Vec::new();
        let mut last_phase = None;
        loop {
            line.clear();
            if reader.read_until(b'\n', &mut line)? == 0 {
                break;
            }
            output.extend_from_slice(&line);
            if let Some(phase) = cargo_phase(&String::from_utf8_lossy(&line)) {
                if Some(phase) != last_phase {
                    // Only a handful of phase changes cross the channel, so cargo's
                    // pipe is drained continuously however quickly it writes.
                    let _ = phase_sender.send(phase);
                    last_phase = Some(phase);
                }
            }
        }
        Ok(output)
    });
    let stdout_reader = thread::spawn(move || -> io::Result<Vec<u8>> {
        let mut reader = BufReader::new(cargo_stdout);
        let mut output = Vec::new();
        reader.read_to_end(&mut output)?;
        Ok(output)
    });

    let mut progress = UpdateProgress::new();
    progress.start();
    let status = loop {
        for phase in phase_receiver.try_iter() {
            progress.set_phase(phase);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                progress.tick();
                thread::sleep(Duration::from_millis(80));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = stderr_reader.join();
                let _ = stdout_reader.join();
                return Err(error).context("could not wait for cargo install");
            }
        }
    };

    let stderr_output = stderr_reader
        .join()
        .map_err(|_| anyhow!("cargo stderr reader stopped unexpectedly"))?
        .context("could not read cargo's stderr")?;
    let stdout_output = stdout_reader
        .join()
        .map_err(|_| anyhow!("cargo stdout reader stopped unexpectedly"))?
        .context("could not read cargo's stdout")?;

    if status.success() {
        let message = if from_git {
            "Cargo installed Ditto CLI from Git.".to_owned()
        } else if let Some(version) = target_version {
            format!("Cargo installed Ditto CLI {version}.")
        } else {
            "Cargo installed Ditto CLI.".to_owned()
        };
        progress.finish(true, &message);
        return Ok(());
    }

    progress.finish(false, "Cargo could not install the update.");
    if !stderr_output.is_empty() || !stdout_output.is_empty() {
        let mut stderr = io::stderr().lock();
        let _ = writeln!(stderr, "\nCargo output:");
        for output in [&stderr_output, &stdout_output] {
            let _ = stderr.write_all(output);
            if !output.is_empty() && !output.ends_with(b"\n") {
                let _ = writeln!(stderr);
            }
        }
    }

    if cfg!(windows) {
        bail!(
            "cargo install failed with {status}; Windows cannot overwrite a running \
             program, so close every Ditto CLI window and run \
             `cargo install {CRATE_NAME} --force --locked` directly"
        );
    }
    bail!("cargo install failed with {status}");
}

fn cargo_phase(line: &str) -> Option<&'static str> {
    let line = line.trim_start();
    if line.starts_with("Updating crates.io index") {
        Some("Checking crates.io")
    } else if line.starts_with("Updating git repository") {
        Some("Fetching the source")
    } else if line.starts_with("Downloading ") || line.starts_with("Downloaded ") {
        Some("Downloading dependencies")
    } else if line.starts_with("Compiling ") && line.contains(CRATE_NAME) {
        Some("Building Ditto CLI")
    } else if line.starts_with("Compiling ") {
        Some("Compiling dependencies")
    } else if line.starts_with("Finished ") {
        Some("Finishing the build")
    } else if line.starts_with("Installing ") || line.starts_with("Replacing ") {
        Some("Installing the new binary")
    } else if line.starts_with("Replaced ") || line.starts_with("Installed package ") {
        Some("Finalizing the update")
    } else {
        None
    }
}

const SPINNER_FRAMES: [&str; 8] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠇"];

struct UpdateProgress {
    animated: bool,
    active: bool,
    frame: usize,
    phase: &'static str,
    started: Instant,
}

impl UpdateProgress {
    fn new() -> Self {
        let dumb_terminal = env::var_os("TERM").is_some_and(|term| term == "dumb");
        Self {
            animated: io::stderr().is_terminal() && !dumb_terminal,
            active: false,
            frame: 0,
            phase: "Preparing the update",
            started: Instant::now(),
        }
    }

    fn start(&mut self) {
        if self.animated {
            self.active = true;
            self.render();
        }
    }

    fn set_phase(&mut self, phase: &'static str) {
        self.phase = phase;
    }

    fn tick(&mut self) {
        if !self.animated {
            return;
        }
        self.frame = (self.frame + 1) % SPINNER_FRAMES.len();
        self.render();
    }

    fn render(&self) {
        let elapsed = self.started.elapsed().as_secs();
        let elapsed = if elapsed > 0 {
            format!("  {elapsed}s")
        } else {
            String::new()
        };
        let mut stderr = io::stderr().lock();
        let _ = queue!(
            stderr,
            MoveToColumn(0),
            Clear(ClearType::CurrentLine),
            SetForegroundColor(Color::Cyan),
            Print(SPINNER_FRAMES[self.frame]),
            ResetColor,
            Print(format!(" {}{elapsed}", self.phase)),
        );
        let _ = stderr.flush();
    }

    fn finish(&mut self, success: bool, message: &str) {
        if !self.animated {
            eprintln!("{message}");
            return;
        }

        let color = if success { Color::Green } else { Color::Red };
        let symbol = if success { "✓" } else { "✗" };
        let elapsed = self.started.elapsed().as_secs_f32();
        let mut stderr = io::stderr().lock();
        let _ = queue!(
            stderr,
            MoveToColumn(0),
            Clear(ClearType::CurrentLine),
            SetForegroundColor(color),
            Print(symbol),
            ResetColor,
            Print(format!(" {message} ({elapsed:.1}s)\n")),
        );
        let _ = stderr.flush();
        self.active = false;
    }

    fn clear(&self) {
        let mut stderr = io::stderr().lock();
        let _ = queue!(stderr, MoveToColumn(0), Clear(ClearType::CurrentLine));
        let _ = stderr.flush();
    }
}

impl Drop for UpdateProgress {
    fn drop(&mut self) {
        if self.active {
            self.clear();
        }
    }
}

fn installed_by_npm() -> bool {
    env::var_os(INSTALL_SOURCE).is_some_and(|source| source == "npm")
}

/// npm keeps its copy inside its own tree, so `cargo install` would add a
/// second one to Cargo's bin directory and leave which of them runs to
/// whichever comes first on `PATH`. The npm command is named rather than run:
/// it replaces the program that would be running it, which Windows refuses
/// outright, and npm is the thing that knows how to stand itself back up.
fn report_npm_install() {
    println!();
    println!("This copy was installed with npm, so cargo cannot replace it.");
    println!("Check for a newer one with `npm view {NPM_PACKAGE} version`.");
    println!("Install it with `npm install -g {NPM_PACKAGE}@latest`.");
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
    fn turns_cargo_build_messages_into_update_phases() {
        assert_eq!(
            cargo_phase("    Updating crates.io index\n"),
            Some("Checking crates.io")
        );
        assert_eq!(
            cargo_phase("   Compiling serde v1.0.0\n"),
            Some("Compiling dependencies")
        );
        assert_eq!(
            cargo_phase("   Compiling ditto-cli v0.3.5\n"),
            Some("Building Ditto CLI")
        );
        assert_eq!(cargo_phase("warning: harmless"), None);
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
