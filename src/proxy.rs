//! Runs a tool inside a pseudoterminal so Ditto can name the profile in the
//! title.
//!
//! Every tool Ditto launches sets its own terminal title and keeps updating it,
//! so a title Ditto writes once is overwritten within moments of starting. Only
//! opencode can be told not to, which makes turning them off a dead end.
//!
//! Instead Ditto sits between the tool and the terminal and rewrites the title
//! sequences on their way out, turning `Codex — my-repo` into
//! `ditto:work — Codex — my-repo`. Everything else is forwarded byte for byte,
//! so what the tool draws is untouched and the tool keeps the title it wanted.

use std::io::Write;

/// Sits between the profile and whatever the tool called itself.
const SEPARATOR: &str = " — ";
const ESCAPE: u8 = 0x1b;
const BELL: u8 = 0x07;
/// Enough for any title worth reading. A sequence that never ends would
/// otherwise hold back the tool's output for as long as it kept arriving.
const MAX_SEQUENCE: usize = 8 * 1024;

/// Where the filter is in an escape sequence. Sequences arrive split across
/// reads, so this survives between chunks.
#[derive(Clone, Copy, Eq, PartialEq)]
enum State {
    /// Ordinary output, forwarded as it arrives.
    Text,
    /// An escape byte was seen and the sequence has not been identified yet.
    Escape,
    /// Inside an operating system command, collecting it to be rewritten.
    Command,
    /// Inside a command, on an escape that may end it.
    CommandEscape,
}

/// Rewrites the window title in a stream of terminal output, forwarding
/// everything else unchanged.
pub struct TitleFilter {
    /// What the profile is called in a rewritten title.
    label: String,
    state: State,
    /// The command being collected, without its introducer or terminator.
    pending: Vec<u8>,
}

impl TitleFilter {
    pub fn new(profile: &str) -> Self {
        Self {
            label: format!("ditto:{profile}"),
            state: State::Text,
            pending: Vec::new(),
        }
    }

    /// The title to set before the tool has said anything.
    pub fn initial_title(&self) -> Vec<u8> {
        let mut sequence = Vec::new();
        write_command(&mut sequence, b"0", Some(self.label.as_bytes()), BELL);
        sequence
    }

    pub fn push(&mut self, input: &[u8], output: &mut Vec<u8>) {
        for &byte in input {
            match self.state {
                State::Text => {
                    if byte == ESCAPE {
                        self.state = State::Escape;
                    } else {
                        output.push(byte);
                    }
                }
                State::Escape => match byte {
                    // A command is the only sequence worth collecting. Anything
                    // else is forwarded with the escape that introduced it.
                    b']' => {
                        self.state = State::Command;
                        self.pending.clear();
                    }
                    ESCAPE => output.push(ESCAPE),
                    _ => {
                        output.push(ESCAPE);
                        output.push(byte);
                        self.state = State::Text;
                    }
                },
                State::Command => match byte {
                    BELL => self.finish(output, BELL),
                    ESCAPE => self.state = State::CommandEscape,
                    _ => {
                        self.pending.push(byte);
                        // A sequence this long is not a title. Letting it go
                        // keeps the tool's output moving.
                        if self.pending.len() > MAX_SEQUENCE {
                            self.release(output);
                        }
                    }
                },
                State::CommandEscape => {
                    if byte == b'\\' {
                        self.finish(output, ESCAPE);
                    } else {
                        // The escape belonged to the command after all.
                        self.pending.push(ESCAPE);
                        self.pending.push(byte);
                        self.state = State::Command;
                    }
                }
            }
        }
    }

    /// Emits a collected command, rewritten when it names the title.
    fn finish(&mut self, output: &mut Vec<u8>, terminator: u8) {
        let (kind, text) = split_command(&self.pending);
        match (names_the_title(kind), text.map(std::str::from_utf8)) {
            (true, Some(Ok(text))) => {
                let rewritten = self.rewrite(text);
                write_command(output, kind, Some(rewritten.as_bytes()), terminator);
            }
            // A title command with no argument at all still means the empty
            // title, so it is rewritten like any other rather than forwarded.
            (true, None) => {
                let rewritten = self.rewrite("");
                write_command(output, kind, Some(rewritten.as_bytes()), terminator);
            }
            // Anything else is somebody else's sequence: hyperlinks, colours,
            // clipboard writes. It goes out exactly as it came in.
            _ => write_command(output, kind, text, terminator),
        }
        self.pending.clear();
        self.state = State::Text;
    }

    /// Puts the profile in front of the title the tool asked for. A title that
    /// already carries it is left alone, so a tool that reads its own title
    /// back and sets it again does not stack the prefix up.
    fn rewrite(&self, title: &str) -> String {
        if title == self.label || title.starts_with(&format!("{}{SEPARATOR}", self.label)) {
            return title.to_owned();
        }
        if title.is_empty() {
            return self.label.clone();
        }
        format!("{}{SEPARATOR}{title}", self.label)
    }

    /// Gives up on a sequence that ran too long and forwards it as it stands.
    fn release(&mut self, output: &mut Vec<u8>) {
        output.push(ESCAPE);
        output.push(b']');
        output.extend_from_slice(&self.pending);
        self.pending.clear();
        self.state = State::Text;
    }
}

/// Splits a command into the number that says what it does and its argument.
///
/// A command with no `;` has no argument, which is not the same as an empty
/// one: `OSC 104` resets the whole palette while `OSC 104;` names no colour to
/// reset. Putting the separator back into a sequence that never had one would
/// change what it asks the terminal for, so the difference is carried.
fn split_command(command: &[u8]) -> (&[u8], Option<&[u8]>) {
    match command.iter().position(|&byte| byte == b';') {
        Some(separator) => (&command[..separator], Some(&command[separator + 1..])),
        None => (command, None),
    }
}

/// Commands 0, 1 and 2 set the window title, the icon name, or both. Every
/// other command means something unrelated and must not be touched.
fn names_the_title(kind: &[u8]) -> bool {
    matches!(kind, b"0" | b"1" | b"2")
}

fn write_command(output: &mut Vec<u8>, kind: &[u8], text: Option<&[u8]>, terminator: u8) {
    output.push(ESCAPE);
    output.push(b']');
    output.extend_from_slice(kind);
    if let Some(text) = text {
        output.push(b';');
        output.extend_from_slice(text);
    }
    if terminator == ESCAPE {
        output.push(ESCAPE);
        output.push(b'\\');
    } else {
        output.push(BELL);
    }
}

/// Sends the title Ditto starts with, before the tool has drawn anything.
pub fn announce(writer: &mut impl Write, filter: &TitleFilter) {
    let _ = writer.write_all(&filter.initial_title());
    let _ = writer.flush();
}

#[cfg(unix)]
pub use unix::{exit_code, run};

#[cfg(unix)]
mod unix {
    use std::{
        ffi::{CString, OsString},
        fs::File,
        io::{self, ErrorKind, Read, Write},
        os::{
            fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
            unix::process::{CommandExt, ExitStatusExt},
        },
        process::ExitStatus,
        ptr,
        sync::atomic::{AtomicBool, Ordering},
        thread,
    };

    use anyhow::{Context, Result, bail};
    use crossterm::terminal::{disable_raw_mode, enable_raw_mode, size};

    use super::{TitleFilter, announce};
    use crate::{launch, profile::Profile};

    /// Set by the resize handler and read by the forwarding loop. A signal
    /// handler can do almost nothing safely, so it only leaves a note.
    static RESIZED: AtomicBool = AtomicBool::new(false);

    extern "C" fn on_resize(_signal: libc::c_int) {
        RESIZED.store(true, Ordering::Relaxed);
    }

    /// Puts the terminal back the way it was found, including when the
    /// forwarding loop fails part way through.
    struct RawMode;

    impl RawMode {
        fn enter() -> Result<Self> {
            enable_raw_mode().context("could not put the terminal into raw mode")?;
            Ok(Self)
        }
    }

    impl Drop for RawMode {
        fn drop(&mut self) {
            let _ = disable_raw_mode();
        }
    }

    /// Runs the tool in a pseudoterminal and forwards it, rewriting the title
    /// on the way out. Returns how the tool exited so Ditto can exit the same
    /// way.
    pub fn run(tool: launch::Tool, profile: &Profile, args: &[OsString]) -> Result<ExitStatus> {
        let (controller, device) = open_pair()?;

        let mut command = launch::build_command(tool, profile, args);
        command
            .stdin(device.try_clone().context("could not attach the input")?)
            .stdout(device.try_clone().context("could not attach the output")?)
            .stderr(
                device
                    .try_clone()
                    .context("could not attach the error output")?,
            );

        // The tool needs a session of its own with the pseudoterminal as its
        // controlling terminal, or job control and Ctrl-C do not reach it.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(io::Error::last_os_error());
                }
                if libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let mut child = command
            .spawn()
            .with_context(|| format!("could not launch {}", tool.label()))?;
        // Only the tool holds the device end now. Keeping it open here would
        // stop the reader below from ever seeing the tool exit.
        drop(device);

        let started = (|| {
            let raw = RawMode::enter()?;
            install_resize_handler();
            resize(controller.as_raw_fd());

            let status = forward(&controller, &profile.name, &mut child);
            drop(raw);
            status
        })();

        // The caller answers a failure here by handing the terminal to the tool
        // itself, so a tool this one already started has to be gone before it
        // does: two copies of the same tool against one profile write over each
        // other's session, and the one nothing is attached to would be
        // invisible. Everything that can fail past this point has left the tool
        // with a terminal nobody is reading, so there is nothing to keep.
        if started.is_err() {
            let _ = child.kill();
            let _ = child.wait();
        }
        started
    }

    /// Copies between the terminal and the tool until the tool exits.
    fn forward(
        controller: &OwnedFd,
        profile: &str,
        child: &mut std::process::Child,
    ) -> Result<ExitStatus> {
        let mut reader = File::from(
            controller
                .try_clone()
                .context("could not read from the tool")?,
        );
        let mut writer = File::from(
            controller
                .try_clone()
                .context("could not write to the tool")?,
        );

        // Input is forwarded on its own thread: both directions block, and the
        // tool cannot be waited on while a read of the keyboard is in the way.
        // The thread ends with the process, which is why it is never joined.
        thread::spawn(move || {
            let mut keyboard = io::stdin();
            let mut buffer = [0u8; 4096];
            loop {
                match keyboard.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => {
                        if writer.write_all(&buffer[..read]).is_err() {
                            break;
                        }
                    }
                    Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        });

        let mut filter = TitleFilter::new(profile);
        let mut screen = io::stdout();
        announce(&mut screen, &filter);

        let mut buffer = [0u8; 8192];
        let mut rewritten = Vec::with_capacity(buffer.len());
        loop {
            if RESIZED.swap(false, Ordering::Relaxed) {
                resize(controller.as_raw_fd());
            }

            match reader.read(&mut buffer) {
                // A tool that has exited leaves nothing on its end. Both of
                // these mean the same thing, and which one arrives depends on
                // the platform.
                Ok(0) => break,
                Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
                // The read was cut short by the window changing size, which the
                // top of the loop handles.
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) => return Err(error).context("could not read from the tool"),
                Ok(read) => {
                    rewritten.clear();
                    filter.push(&buffer[..read], &mut rewritten);
                    screen
                        .write_all(&rewritten)
                        .and_then(|()| screen.flush())
                        .context("could not write to the terminal")?;
                }
            }
        }

        child.wait().context("could not wait for the tool to exit")
    }

    /// Opens a pseudoterminal, returning the end Ditto keeps and the end the
    /// tool is given.
    fn open_pair() -> Result<(OwnedFd, OwnedFd)> {
        unsafe {
            let controller = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY);
            if controller < 0 {
                bail!(
                    "could not open a pseudoterminal: {}",
                    io::Error::last_os_error()
                );
            }
            let controller = OwnedFd::from_raw_fd(controller);

            if libc::grantpt(controller.as_raw_fd()) < 0
                || libc::unlockpt(controller.as_raw_fd()) < 0
            {
                bail!(
                    "could not prepare a pseudoterminal: {}",
                    io::Error::last_os_error()
                );
            }

            let name = libc::ptsname(controller.as_raw_fd());
            if name.is_null() {
                bail!("could not name the pseudoterminal");
            }
            let name = CString::from(std::ffi::CStr::from_ptr(name));

            let device = libc::open(name.as_ptr(), libc::O_RDWR | libc::O_NOCTTY);
            if device < 0 {
                bail!(
                    "could not open the pseudoterminal: {}",
                    io::Error::last_os_error()
                );
            }
            Ok((controller, OwnedFd::from_raw_fd(device)))
        }
    }

    /// Asks to be told when the window changes size. The handler is installed
    /// without restarting interrupted reads, so the forwarding loop notices.
    fn install_resize_handler() {
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = on_resize as *const () as libc::sighandler_t;
            libc::sigemptyset(&mut action.sa_mask);
            action.sa_flags = 0;
            libc::sigaction(libc::SIGWINCH, &action, ptr::null_mut());
        }
    }

    /// Matches the tool's terminal to the real one. Setting the size also tells
    /// the tool to redraw, so nothing else has to be forwarded.
    fn resize(controller: RawFd) {
        let Ok((columns, rows)) = size() else {
            return;
        };
        let size = libc::winsize {
            ws_row: rows,
            ws_col: columns,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        unsafe {
            libc::ioctl(controller, libc::TIOCSWINSZ as _, &size);
        }
    }

    /// Turns a tool's exit into Ditto's own, so a caller sees what it would
    /// have seen had Ditto stepped out of the way.
    pub fn exit_code(status: ExitStatus) -> i32 {
        status
            .code()
            .unwrap_or_else(|| status.signal().map_or(1, |signal| 128 + signal))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filtered(chunks: &[&[u8]]) -> Vec<u8> {
        let mut filter = TitleFilter::new("work");
        let mut output = Vec::new();
        for chunk in chunks {
            filter.push(chunk, &mut output);
        }
        output
    }

    fn text(chunks: &[&[u8]]) -> String {
        String::from_utf8(filtered(chunks)).unwrap()
    }

    #[test]
    fn names_the_profile_in_a_title_the_tool_sets() {
        assert_eq!(
            text(&[b"\x1b]0;Codex\x07"]),
            "\x1b]0;ditto:work — Codex\x07"
        );
        // The other title commands mean the same thing to a terminal.
        assert_eq!(
            text(&[b"\x1b]2;Codex\x07"]),
            "\x1b]2;ditto:work — Codex\x07"
        );
        assert_eq!(text(&[b"\x1b]1;omp\x07"]), "\x1b]1;ditto:work — omp\x07");
    }

    /// `OSC 104` resets the whole palette and `OSC 112` the cursor colour, and
    /// neither carries an argument. Adding the separator they left out would
    /// turn each into a request naming nothing, which the terminal ignores.
    #[test]
    fn forwards_a_command_that_carries_no_argument_as_it_stands() {
        assert_eq!(filtered(&[b"\x1b]112\x07"]), b"\x1b]112\x07");
        assert_eq!(filtered(&[b"\x1b]104\x1b\\"]), b"\x1b]104\x1b\\");
    }

    #[test]
    fn keeps_the_terminator_the_tool_used() {
        // A string terminator is as valid as a bell and has to come back out
        // the same way, or the terminal keeps reading the rest as a title.
        assert_eq!(
            text(&[b"\x1b]0;Codex\x1b\\"]),
            "\x1b]0;ditto:work — Codex\x1b\\"
        );
    }

    #[test]
    fn rebuilds_a_title_split_across_reads() {
        // Output arrives in whatever sizes the tool happens to write, so a
        // title can land in pieces and still has to come out whole and once.
        assert_eq!(
            text(&[b"\x1b]0;Co", b"dex \xe2\x80\x94 rep", b"o\x07"]),
            "\x1b]0;ditto:work — Codex — repo\x07"
        );
        assert_eq!(
            text(&[b"\x1b", b"]0;omp\x07"]),
            "\x1b]0;ditto:work — omp\x07"
        );
        assert_eq!(
            text(&[b"\x1b]0;omp\x1b", b"\\"]),
            "\x1b]0;ditto:work — omp\x1b\\"
        );
    }

    #[test]
    fn leaves_a_title_that_already_names_the_profile() {
        // A tool that reads its title back and writes it again must not stack
        // the profile up every time it does.
        let once = text(&[b"\x1b]0;Codex\x07"]);
        let twice = text(&[once.as_bytes()]);
        assert_eq!(twice, once);
        assert_eq!(text(&[b"\x1b]0;ditto:work\x07"]), "\x1b]0;ditto:work\x07");
    }

    #[test]
    fn names_the_profile_alone_when_the_tool_clears_the_title() {
        assert_eq!(text(&[b"\x1b]0;\x07"]), "\x1b]0;ditto:work\x07");
    }

    #[test]
    fn forwards_everything_that_is_not_a_title() {
        // Hyperlinks, colours and clipboard writes are commands too, and
        // rewriting any of them would break the thing they do.
        for sequence in [
            "\x1b]8;;https://example.com\x07link\x1b]8;;\x07",
            "\x1b]11;rgb:0000/0000/0000\x07",
            "\x1b]52;c;aGk=\x07",
            "\x1b]133;A\x07",
        ] {
            assert_eq!(text(&[sequence.as_bytes()]), sequence, "{sequence:?}");
        }
    }

    #[test]
    fn forwards_drawing_untouched() {
        // Everything a tool paints has to arrive exactly as it was sent.
        let screen = "\x1b[2J\x1b[H\x1b[38;2;1;2;3mhello\x1b[0m\r\n\x1b[?1049h";
        assert_eq!(text(&[screen.as_bytes()]), screen);
        assert_eq!(text(&[b"plain output"]), "plain output");
        // A lone escape is not the start of anything and still has to survive.
        assert_eq!(filtered(&[b"\x1b\x1bZ"]), b"\x1b\x1bZ");
    }

    #[test]
    fn gives_up_on_a_sequence_that_never_ends() {
        // Holding output back forever would freeze the tool on screen, so an
        // implausible sequence is forwarded rather than collected.
        let mut runaway = b"\x1b]0;".to_vec();
        runaway.extend(std::iter::repeat_n(b'x', MAX_SEQUENCE + 1));
        let output = filtered(&[&runaway]);

        assert!(output.starts_with(b"\x1b]0;"));
        assert_eq!(output.len(), runaway.len());
    }

    #[test]
    fn starts_by_naming_the_profile() {
        let filter = TitleFilter::new("work");
        assert_eq!(filter.initial_title(), b"\x1b]0;ditto:work\x07");
    }
}
