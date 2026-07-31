#!/usr/bin/env node
"use strict";

// npm has no way to publish one package carrying every platform's binary, so a
// release is a wrapper package — this one — depending on a binary package per
// platform. npm installs only the one whose `os` and `cpu` match and passes
// over the rest, because they are optional dependencies rather than required
// ones. All this file does is find the binary that was installed and hand the
// terminal to it.

const fs = require("node:fs");
const os = require("node:os");
const { spawnSync } = require("node:child_process");

const REPOSITORY = "https://github.com/reyanshgupta/ditto-cli";
const PACKAGES = {
  "darwin-arm64": "@reyanshgupta/ditto-cli-darwin-arm64",
  "darwin-x64": "@reyanshgupta/ditto-cli-darwin-x64",
  "linux-x64": "@reyanshgupta/ditto-cli-linux-x64",
  "win32-x64": "@reyanshgupta/ditto-cli-win32-x64",
};

const platform = `${process.platform}-${process.arch}`;
const packageName = PACKAGES[platform];

if (packageName === undefined) {
  fail(
    `ditto-cli: no prebuilt binary for ${platform}`,
    "ditto-cli: build one with `cargo install ditto-cli`, which needs Rust 1.85 or newer",
  );
}

const binaryName = process.platform === "win32" ? "ditto-cli.exe" : "ditto-cli";

let binary;
try {
  // The binary packages deliberately carry no `exports` field, which is what
  // lets a path inside one resolve at all. Adding one would have to list this
  // path or this line stops finding it.
  binary = require.resolve(`${packageName}/bin/${binaryName}`);
} catch {
  fail(
    `ditto-cli: ${packageName} is not installed`,
    "ditto-cli: it is an optional dependency, so an install run with `--no-optional` or",
    "ditto-cli: `--omit=optional` leaves the binary out; reinstall with neither, or take",
    `ditto-cli: an archive from ${REPOSITORY}/releases`,
  );
}

// Ditto draws over the whole terminal, and a launcher that exited first would
// hand the shell back a screen the process still running had not restored. So
// the signals a terminal sends to everything in the foreground are ignored
// here and left to the process actually drawing, which already handles them.
process.on("SIGINT", () => {});
process.on("SIGTERM", () => {});

// `ditto-cli update` installs with cargo, which would write a second copy into
// Cargo's bin directory rather than replace this one. Saying how this copy
// arrived is what lets it name the npm command instead.
const result = spawnSync(binary, process.argv.slice(2), {
  stdio: "inherit",
  env: { ...process.env, DITTO_INSTALL_SOURCE: "npm" },
});

if (result.error) {
  fail(`ditto-cli: could not run ${binary}: ${result.error.message}`);
}

// Ditto's launch commands exit with the status of the tool they ran, so the
// wrapper has to carry that status out rather than report its own success.
// A run that ended on a signal has no status, and 128 plus the signal number
// is what a shell would have reported for it.
if (result.signal !== null) {
  process.exit(128 + (os.constants.signals[result.signal] ?? 0));
}
process.exit(result.status ?? 1);

// Written straight to the file descriptor because `process.stderr.write` is
// not synchronous when stderr is a pipe, and the exit below would cut the
// message off before it left.
function fail(...lines) {
  fs.writeSync(2, `${lines.join("\n")}\n`);
  process.exit(1);
}
