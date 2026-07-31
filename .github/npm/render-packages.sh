#!/usr/bin/env bash

# Renders the npm packages for a released version into a directory, ready to be
# published. Takes the version, a directory holding the release archives, and
# the directory to write into.
#
#   render-packages.sh 0.3.2 archives npm-dist
#
# One release is five packages: a wrapper every platform installs, and a binary
# package per platform that npm skips unless `os` and `cpu` match. Publish the
# binary packages first — the wrapper names them as optional dependencies, and
# an install that reaches the wrapper before them finds nothing to run.
#
# Binaries are taken out of the archives the release published rather than
# built here, so what npm serves is byte for byte what the releases page and
# the Homebrew formula serve.

set -euo pipefail

usage="usage: render-packages.sh <version> <archives-directory> <output-directory>"
version="${1:?${usage}}"
archives="${2:?${usage}}"
output="${3:?${usage}}"

tag="v${version}"
here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "${here}/../.." && pwd)"
scope="@reyanshgupta"
repository="https://github.com/reyanshgupta/ditto-cli"
homepage="https://www.reyanshgupta.com/projects/ditto-cli"

# Read from the manifest rather than repeated here, so the crate and the npm
# packages cannot end up describing Ditto differently.
description="$(awk -F'"' '/^description = /{ print $2; exit }' "${root}/Cargo.toml")"
if [[ -z "${description}" ]]; then
  echo "render-packages: no description in ${root}/Cargo.toml" >&2
  exit 1
fi

# A missing archive has to stop the render. A binary package published without
# its binary installs cleanly and then fails at the first run, which is a long
# way from the release that went out incomplete.
render_platform() {
  local target="$1" platform="$2" arch="$3"
  local name="ditto-cli-${platform}-${arch}"
  local directory="${output}/${name}"
  local binary="ditto-cli"
  local archive="${archives}/ditto-cli-${tag}-${target}"
  local libc=""

  if [[ "${platform}" == "win32" ]]; then
    binary="ditto-cli.exe"
    archive="${archive}.zip"
  else
    archive="${archive}.tar.gz"
  fi

  # The Linux binary links glibc. npm passes over a package whose `libc` does
  # not match, so saying so is what makes an Alpine install report that there
  # is no binary for it rather than install one that cannot start.
  if [[ "${platform}" == "linux" ]]; then
    libc=$'\n  "libc": ["glibc"],'
  fi

  if [[ ! -f "${archive}" ]]; then
    echo "render-packages: no archive at ${archive}" >&2
    return 1
  fi

  mkdir -p "${directory}/bin"
  if [[ "${platform}" == "win32" ]]; then
    unzip -q -o "${archive}" -d "${directory}/bin"
  else
    tar -xzf "${archive}" -C "${directory}/bin"
  fi

  if [[ ! -f "${directory}/bin/${binary}" ]]; then
    echo "render-packages: ${archive} does not hold ${binary}" >&2
    return 1
  fi
  chmod +x "${directory}/bin/${binary}"

  cat > "${directory}/package.json" <<JSON
{
  "name": "${scope}/${name}",
  "version": "${version}",
  "description": "The ${platform}-${arch} binary for Ditto CLI",
  "license": "MIT",
  "author": "Reyansh Gupta",
  "homepage": "${homepage}",
  "repository": {
    "type": "git",
    "url": "git+${repository}.git"
  },
  "os": ["${platform}"],
  "cpu": ["${arch}"],${libc}
  "files": ["bin/${binary}"],
  "preferUnplugged": true
}
JSON
}

render_platform aarch64-apple-darwin darwin arm64
render_platform x86_64-apple-darwin darwin x64
render_platform x86_64-unknown-linux-gnu linux x64
render_platform x86_64-pc-windows-msvc win32 x64

wrapper="${output}/ditto-cli"
mkdir -p "${wrapper}/bin"
cp "${here}/launcher.js" "${wrapper}/bin/ditto-cli.js"
cp "${root}/README.md" "${wrapper}/README.md"
cp "${root}/LICENSE" "${wrapper}/LICENSE"

# The dependency versions are exact rather than caret ranges. A wrapper is only
# ever correct with the binaries cut from the same commit, and a range would
# let npm pair it with a later one.
cat > "${wrapper}/package.json" <<JSON
{
  "name": "${scope}/ditto-cli",
  "version": "${version}",
  "description": "${description}",
  "keywords": [
    "claude-code",
    "codex",
    "opencode",
    "omp",
    "profiles",
    "cli"
  ],
  "license": "MIT",
  "author": "Reyansh Gupta",
  "homepage": "${homepage}",
  "repository": {
    "type": "git",
    "url": "git+${repository}.git"
  },
  "bugs": {
    "url": "${repository}/issues"
  },
  "bin": {
    "ditto-cli": "bin/ditto-cli.js"
  },
  "files": [
    "bin/ditto-cli.js",
    "README.md",
    "LICENSE"
  ],
  "engines": {
    "node": ">=18"
  },
  "optionalDependencies": {
    "${scope}/ditto-cli-darwin-arm64": "${version}",
    "${scope}/ditto-cli-darwin-x64": "${version}",
    "${scope}/ditto-cli-linux-x64": "${version}",
    "${scope}/ditto-cli-win32-x64": "${version}"
  }
}
JSON

echo "Rendered ${scope}/ditto-cli ${version} and its four binary packages into ${output}"
