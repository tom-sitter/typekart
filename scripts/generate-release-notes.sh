#!/usr/bin/env sh
set -eu

usage() {
  cat <<'USAGE'
Usage: scripts/generate-release-notes.sh VERSION [FROM_REF] [TO_REF]

Prints Markdown release notes for VERSION.

Arguments:
  VERSION   SemVer version without a leading "v", for example 0.1.0
  FROM_REF  Optional starting git ref. Defaults to the latest v* tag, or the first commit.
  TO_REF    Optional ending git ref. Defaults to HEAD.

Examples:
  scripts/generate-release-notes.sh 0.1.0
  scripts/generate-release-notes.sh 0.2.0 v0.1.0 HEAD
USAGE
}

die() {
  printf 'release-notes: %s\n' "$*" >&2
  exit 1
}

[ "${1:-}" != "-h" ] && [ "${1:-}" != "--help" ] || {
  usage
  exit 0
}

version=${1:-}
from_ref=${2:-}
to_ref=${3:-HEAD}

[ -n "$version" ] || {
  usage >&2
  exit 2
}

case "$version" in
  v*)
    die "pass the version without a leading v, for example 0.1.0"
    ;;
esac

printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$' \
  || die "version must be SemVer, for example 0.1.0"

command -v git >/dev/null 2>&1 || die "git is required"
git rev-parse --show-toplevel >/dev/null 2>&1 || die "run this from inside the git repository"

if [ -z "$from_ref" ]; then
  from_ref=$(git describe --tags --abbrev=0 --match 'v*' 2>/dev/null || true)
fi

if [ -n "$from_ref" ]; then
  range="$from_ref..$to_ref"
else
  root=$(git rev-list --max-parents=0 "$to_ref" | tail -n 1)
  range="$root..$to_ref"
fi

tag="v$version"
date_utc=$(date -u '+%Y-%m-%d')

cat <<EOF
# TypeKart $tag

Released: $date_utc

## Highlights

- Terminal typing races with kart-style item effects.
- Single-player races with optional AI racers.
- Local, LAN, and relay-backed online multiplayer.
- Unicode item indicators by default, with an ASCII fallback.
- Moddable word sets and configurable item packs.
- macOS and Windows release archives.

## Install

Download the archive for your platform from the GitHub release:

- macOS Apple Silicon: \`typekart-aarch64-apple-darwin.tar.gz\`
- macOS Intel: \`typekart-x86_64-apple-darwin.tar.gz\`
- Windows x64: \`typekart-x86_64-pc-windows-msvc.zip\`

## Changes

EOF

if git log --oneline "$range" >/dev/null 2>&1 && [ -n "$(git log --oneline "$range")" ]; then
  git log --reverse --pretty='- %s (%h)' "$range"
else
  printf '%s\n' '- Initial beta release.'
fi

cat <<'EOF'

## Known Beta Limitations

- Online play requires a relay that players can reach.
- The relay is in-memory; restarting it closes active rooms.
- WinGet manifests should be updated from this release's checksum file before submitting to microsoft/winget-pkgs.
EOF
