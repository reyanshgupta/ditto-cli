#!/usr/bin/env python3
"""Captures the picker for the README, so the screens there are what the
picker draws rather than what somebody remembered it drawing.

Runs a debug build in a pseudoterminal against a throwaway home, with stand-in
agents on PATH that answer their status commands the way the real ones do, and
prints two 80-column screens: the picker, and the picker with the launcher open.
Paste them into README.md over the blocks they replace.

    cargo build && .github/readme-picker.py
"""
import codecs, fcntl, os, pty, re, select, signal, struct, subprocess, tempfile, termios, time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
BIN = REPO / "target/debug/ditto-cli"
COLS, ROWS = 80, 36

# Each stand-in prints what its status command prints when signed in or out.
AGENTS = {
    "claude": 'echo \'{"loggedIn":true}\'',
    "codex": 'echo "Not logged in" >&2; exit 1',
    "fx": 'echo \'{"kind":"status","auth":"Codex subscription"}\'',
    "opencode": 'echo "0 credentials"',
    "omp": "exit 0",
    "prime-agent": "exit 0",
    "pi": "exit 0",
    "gemini": "exit 0",
    "grok": "exit 0",
}

tmp = Path(tempfile.mkdtemp(prefix="ditto-readme-"))
home = tmp / "home"
home.mkdir()
bin_dir = tmp / "bin"
bin_dir.mkdir()
for name, body in AGENTS.items():
    script = bin_dir / name
    script.write_text(f"#!/bin/sh\n{body}\n")
    script.chmod(0o755)
env = {"HOME": str(home), "PATH": str(bin_dir), "TERM": "xterm-256color"}

def ditto(*args):
    subprocess.run([str(BIN), *args], env=env, check=True, capture_output=True)

for name in ("personal", "work"):
    ditto("create", name, "--json")
ditto("default", "work", "--json")
work = home / ".ditto/profiles/work"
(work / "prime-agent/auth.json").write_text('{"anthropic": {}}')
(work / "pi/auth.json").write_text("{}")
(work / "gemini-home/.gemini/oauth_creds.json").write_text("{}")
subprocess.run(
    ["sqlite3", str(home / ".omp/profiles/work/agent/agent.db"),
     "CREATE TABLE auth_credentials(disabled_cause TEXT); INSERT INTO auth_credentials VALUES (NULL);"],
    check=True,
)

pid, fd = pty.fork()
if pid == 0:
    os.execvpe(str(BIN), [str(BIN)], env)
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))

screen = [[" "] * COLS for _ in range(ROWS)]
cursor = [0, 0]
decoder = codecs.getincrementaldecoder("utf-8")()
pending = ""

def read_for(seconds):
    end = time.time() + seconds
    while time.time() < end:
        ready, _, _ = select.select([fd], [], [], 0.1)
        if ready:
            try:
                paint(decoder.decode(os.read(fd, 65536)))
            except OSError:
                return

COMPLETE = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]|\x1b\][^\x07]*\x07|\x1b[()][A-Z0-9]|\x1b[=>]")

def paint(chunk):
    """Enough of a terminal to place text: cursor moves and clears, no styling.

    A read can end in the middle of an escape sequence, and the rest arrives
    with the next read, so an unfinished sequence at the end is held back
    rather than painted as text.
    """
    global pending
    text = pending + chunk
    pending = ""
    last = text.rfind("\x1b")
    if last != -1 and not COMPLETE.match(text, last):
        pending = text[last:]
        text = text[:last]
    i = 0
    while i < len(text):
        if text[i] == "\x1b":
            csi = re.match(r"\x1b\[([0-9;?]*)([A-Za-z])", text[i:])
            if csi:
                params, final = csi.groups()
                if final == "H":
                    row, _, col = params.partition(";")
                    cursor[:] = [max(0, int(row or 1) - 1), max(0, int(col or 1) - 1)]
                elif final == "J":
                    for line in screen:
                        line[:] = [" "] * COLS
                i += csi.end()
                continue
            other = re.match(r"\x1b\][^\x07]*\x07|\x1b[()][A-Z0-9]|\x1b[=>]", text[i:])
            i += other.end() if other else 1
            continue
        ch = text[i]
        if ch == "\r":
            cursor[1] = 0
        elif ch == "\n":
            cursor[0] = min(ROWS - 1, cursor[0] + 1)
        elif ch >= " ":
            row, col = cursor
            if row < ROWS and col < COLS:
                screen[row][col] = ch
            cursor[1] += 1
        i += 1

def frame():
    return "\n".join("".join(line).rstrip() for line in screen).rstrip("\n")

read_for(3.0)
os.write(fd, b"\x1b[B\x1b[B")
read_for(0.5)
picker = frame()
os.write(fd, b"\r")
read_for(1.5)
launcher = frame()
os.kill(pid, signal.SIGKILL)
os.waitpid(pid, 0)
print(picker)
print("\n=== launcher ===\n")
print(launcher)
