#!/usr/bin/env bash

# Renders the Homebrew formula for a released version onto stdout.
#
# Checksums are read from the release's own SHA256SUMS rather than recomputed,
# so the formula cannot disagree with the archives that were actually
# published. Pass a local sums file as the second argument to render without a
# network connection.

set -euo pipefail

version="${1:?usage: render-formula.sh <version> [sha256sums-file]}"
tag="v${version}"
sums="${2:-}"

if [[ -z "${sums}" ]]; then
  sums="$(mktemp)"
  trap 'rm -f "${sums}"' EXIT
  curl --fail --silent --show-error --location --output "${sums}" \
    "https://github.com/reyanshgupta/ditto-cli/releases/download/${tag}/SHA256SUMS"
fi

# A missing target has to stop the render. A formula carrying an empty sha256
# installs nothing and reports it as a checksum mismatch on the user's machine,
# which is a long way from the release that actually went out incomplete.
sum_for() {
  local want="ditto-cli-${tag}-$1"
  local hash
  hash="$(awk -v want="${want}" '$2 == want { print $1 }' "${sums}")"
  if [[ -z "${hash}" ]]; then
    echo "render-formula: no checksum for ${want} in SHA256SUMS" >&2
    return 1
  fi
  printf '%s' "${hash}"
}

macos_arm="$(sum_for aarch64-apple-darwin.tar.gz)"
macos_intel="$(sum_for x86_64-apple-darwin.tar.gz)"
linux_intel="$(sum_for x86_64-unknown-linux-gnu.tar.gz)"

sed \
  -e "s|@VERSION@|${version}|g" \
  -e "s|@SHA_MACOS_ARM@|${macos_arm}|g" \
  -e "s|@SHA_MACOS_INTEL@|${macos_intel}|g" \
  -e "s|@SHA_LINUX_INTEL@|${linux_intel}|g" \
  "$(dirname "$0")/ditto-cli.rb.template"
