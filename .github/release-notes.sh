#!/usr/bin/env bash
# Writes the release notes for a tag to stdout.
#
# GitHub's own generated notes list merged pull requests, and this repository
# lands its commits on main directly, so those notes came out as nothing but
# the compare link. The commit messages here are written as prose, so the notes
# are the subject and body of every commit since the previous tag, with the
# version-bump commit and the trailers left out.
set -euo pipefail

tag=${1:?usage: release-notes.sh <tag>}
previous=$(git describe --tags --abbrev=0 "${tag}^" 2>/dev/null || true)
range=${previous:+${previous}..}${tag}
version=${tag#v}

echo "## What changed"
echo
git log --no-merges --reverse --format='%H' "${range}" | while read -r commit; do
  subject=$(git log -1 --format='%s' "${commit}")
  case "${subject}" in
    "Release ${version}") continue ;;
  esac
  echo "### ${subject}"
  echo
  git log -1 --format='%b' "${commit}" \
    | sed -E '/^(Co-Authored-By|Claude-Session|Signed-off-by|Reviewed-by|Co-authored-by): /d' \
    | sed -e :a -e '/^\n*$/{$d;N;ba' -e '}'
  echo
done

cat <<INSTALL
## Install

\`\`\`bash
brew install reyanshgupta/tap/ditto-cli        # or: brew upgrade ditto-cli
npm install -g @reyanshgupta/ditto-cli@${version}
cargo binstall ditto-cli                        # or: cargo install ditto-cli
\`\`\`

The archives below are the same binaries every channel installs; \`SHA256SUMS\` covers them.
INSTALL

if [ -n "${previous}" ]; then
  echo
  echo "**Full Changelog**: https://github.com/reyanshgupta/ditto-cli/compare/${previous}...${tag}"
fi
