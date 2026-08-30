//! Keeps Ditto out of herdr's way, and says over herdr's own socket what
//! stepping aside stops the title from saying.
//!
//! herdr decides which agent is in a pane from the name of that pane's
//! foreground process. A pseudoterminal between the shell and the tool costs it
//! that answer twice over: the foreground process becomes `ditto-cli`, and the
//! tool moves to a session of its own, where herdr does not look. A pane
//! running Claude Code then reports no agent at all, which takes every
//! `herdr agent` command with it.
//!
//! Rewriting the title breaks the other half. herdr reads a tool's title
//! expecting the tool wrote it — Claude Code's spinner rule is anchored to the
//! first character — so a title Ditto has prefixed leaves the agent stuck in
//! whichever state it was last seen in.
//!
//! Neither is worth a profile name in a title bar herdr draws its own labels
//! over, so under herdr Ditto hands the terminal straight to the tool and
//! reports the profile to herdr instead.

use std::{
    ffi::OsString,
    process::{Command, Stdio},
};

use crate::profile::Profile;

/// Set by herdr in every pane it opens, and the only thing Ditto needs from it.
const PANE_VARIABLE: &str = "HERDR_PANE_ID";

/// Names Ditto as the reporter. herdr keeps what each source contributes apart,
/// so this is also what identifies the value to replace on the next launch
/// rather than something a reader has to match by hand.
const SOURCE: &str = "ditto";

/// herdr's own command is how Ditto reaches it. The socket protocol is
/// versioned and negotiated, and herdr's CLI is the part that tracks it, so
/// speaking to that rather than to the socket is what keeps this working
/// across a herdr upgrade.
const HERDR: &str = "herdr";

/// The pane Ditto is running in, if herdr opened it.
pub fn pane() -> Option<String> {
    pane_named(std::env::var_os(PANE_VARIABLE))
}

/// A variable set to nothing is not a pane. Only herdr sets this one, and it
/// never sets it empty, but a shell that exported it around a launch will.
fn pane_named(value: Option<OsString>) -> Option<String> {
    value
        .and_then(|pane| pane.into_string().ok())
        .filter(|pane| !pane.is_empty())
}

/// Puts the profile where somebody running under herdr will look for it, since
/// handing the terminal over means the title no longer names it.
///
/// Every failure is ignored, including herdr not being installed. herdr may
/// have stopped, its socket may be gone, and its CLI may not be on `PATH`;
/// none of that is a reason to refuse to launch a tool, because the label is
/// worth much less than the launch.
pub fn report_profile(profile: &Profile) {
    let Some(pane) = pane() else { return };

    let _ = report_command(&pane, profile)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Reports the profile as display-only metadata, which is the only kind herdr
/// takes from a source that is not the agent itself. A lifecycle report is
/// accepted and then ignored unless herdr detected the agent on its own, so
/// this says nothing about the agent and leaves that to the detection Ditto
/// has just stepped out of the way of.
fn report_command(pane: &str, profile: &Profile) -> Command {
    let mut command = Command::new(HERDR);
    // herdr takes the pane as a positional argument and its parser wants it
    // before the options, not after them.
    command
        .arg("pane")
        .arg("report-metadata")
        .arg(pane)
        .arg("--source")
        .arg(SOURCE)
        .arg("--token")
        .arg(format!("profile={}", profile.name));
    command
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    use crate::profile::OpencodeHome;

    fn profile() -> Profile {
        Profile {
            name: "work".to_owned(),
            claude_home: PathBuf::from("/profiles/work/claude"),
            codex_home: PathBuf::from("/profiles/work/codex"),
            fx_home: PathBuf::from("/profiles/work/fx-home"),
            omp_home: PathBuf::from("/omp/profiles/work/agent"),
            opencode: OpencodeHome {
                data: PathBuf::from("/profiles/work/opencode/data"),
                config: PathBuf::from("/profiles/work/opencode/config"),
                state: PathBuf::from("/profiles/work/opencode/state"),
            },
            pi_home: PathBuf::from("/profiles/work/pi"),
            prime_agent_home: PathBuf::from("/profiles/work/prime-agent"),
            generic: Vec::new(),
            managed: true,
        }
    }

    #[test]
    fn a_pane_is_only_a_pane_when_herdr_named_one() {
        assert_eq!(pane_named(None), None);
        assert_eq!(pane_named(Some(OsString::from(""))), None);
        assert_eq!(
            pane_named(Some(OsString::from("w1B:p3"))),
            Some("w1B:p3".to_owned())
        );
    }

    /// herdr's parser rejects the pane once options have started, and it does
    /// so by reporting the pane as an unknown option, which reads as a wrong
    /// pane rather than a wrong argument order.
    #[test]
    fn the_pane_comes_before_the_options_herdr_parses() {
        let command = report_command("w1B:p3", &profile());
        let arguments: Vec<_> = command.get_args().collect();

        assert_eq!(
            arguments,
            [
                "pane",
                "report-metadata",
                "w1B:p3",
                "--source",
                "ditto",
                "--token",
                "profile=work",
            ]
        );
    }
}
