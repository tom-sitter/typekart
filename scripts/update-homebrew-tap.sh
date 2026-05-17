#!/usr/bin/env sh
set -eu

usage() {
  cat <<'USAGE'
Usage: scripts/update-homebrew-tap.sh VERSION [--tap-dir PATH] [--push]

Updates the TypeKart Homebrew tap formula for an already-published GitHub release.

Arguments:
  VERSION         SemVer version without a leading "v", for example 0.2.0

Options:
  --tap-dir PATH  Existing homebrew-tap checkout to update. If omitted, the script
                  clones tom-sitter/homebrew-tap into a temporary directory.
  --push          Push the tap commit after creating it
  -h, --help      Show this help text

Examples:
  scripts/update-homebrew-tap.sh 0.2.0 --tap-dir ../homebrew-tap
  scripts/update-homebrew-tap.sh 0.2.0 --push
USAGE
}

die() {
  printf 'homebrew-tap: %s\n' "$*" >&2
  exit 1
}

version=
tap_dir=
push_tap=0

while [ "$#" -gt 0 ]; do
  case "$1" in
    --tap-dir)
      shift
      [ "${1:-}" ] || die "--tap-dir requires a path"
      tap_dir=$1
      ;;
    --push)
      push_tap=1
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    -*)
      die "unknown option: $1"
      ;;
    *)
      if [ -n "$version" ]; then
        die "unexpected extra argument: $1"
      fi
      version=$1
      ;;
  esac
  shift
done

[ -n "$version" ] || {
  usage >&2
  exit 2
}

case "$version" in
  v*)
    die "pass the version without a leading v, for example 0.2.0"
    ;;
esac

printf '%s\n' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$' \
  || die "version must be SemVer, for example 0.2.0"

command -v awk >/dev/null 2>&1 || die "awk is required"
command -v curl >/dev/null 2>&1 || die "curl is required"
command -v git >/dev/null 2>&1 || die "git is required"

git rev-parse --show-toplevel >/dev/null 2>&1 || die "run this from inside the typekart repository"
repo_root=$(git rev-parse --show-toplevel)
cd "$repo_root"

formula_template="packaging/homebrew/typekart.rb"
[ -f "$formula_template" ] || die "missing formula template: $formula_template"

tag="v$version"
checksums_url="https://github.com/tom-sitter/typekart/releases/download/$tag/typekart-checksums.txt"
checksums_file=$(mktemp)
formula_file=$(mktemp)
cleanup() {
  rm -f "$checksums_file" "$formula_file"
}
trap cleanup EXIT INT TERM

printf 'Fetching release checksums: %s\n' "$checksums_url"
curl --fail --location --silent --show-error "$checksums_url" > "$checksums_file" \
  || die "failed to fetch checksums; wait for the GitHub release workflow to finish"

arm64_sha=$(awk '$2 == "typekart-aarch64-apple-darwin.tar.gz" { print $1 }' "$checksums_file")
x86_64_sha=$(awk '$2 == "typekart-x86_64-apple-darwin.tar.gz" { print $1 }' "$checksums_file")

[ -n "$arm64_sha" ] || die "missing Apple Silicon checksum in typekart-checksums.txt"
[ -n "$x86_64_sha" ] || die "missing Intel macOS checksum in typekart-checksums.txt"

awk \
  -v version="$version" \
  -v arm64_sha="$arm64_sha" \
  -v x86_64_sha="$x86_64_sha" '
    {
      gsub(/vREPLACE_WITH_VERSION/, "v" version)
      gsub(/REPLACE_WITH_ARM64_SHA256/, arm64_sha)
      gsub(/REPLACE_WITH_X86_64_SHA256/, x86_64_sha)
      print
    }
  ' "$formula_template" > "$formula_file"

if [ -z "$tap_dir" ]; then
  tap_dir=$(mktemp -d)
  printf 'Cloning Homebrew tap into temporary directory: %s\n' "$tap_dir"
  git clone git@github-tom-sitter:tom-sitter/homebrew-tap.git "$tap_dir"
fi

[ -d "$tap_dir/.git" ] || die "tap directory is not a git checkout: $tap_dir"

tap_dirty=$(git -C "$tap_dir" status --short)
if [ -n "$tap_dirty" ]; then
  printf '%s\n' "$tap_dirty" >&2
  die "tap checkout is dirty; commit or stash it before updating"
fi

mkdir -p "$tap_dir/Formula"
cp "$formula_file" "$tap_dir/Formula/typekart.rb"

git -C "$tap_dir" add Formula/typekart.rb
if git -C "$tap_dir" diff --cached --quiet; then
  printf 'Homebrew tap already matches %s; no commit needed.\n' "$tag"
else
  git -C "$tap_dir" commit -m "Update TypeKart to $tag"
fi

if [ "$push_tap" -eq 1 ]; then
  git -C "$tap_dir" push origin HEAD
fi

cat <<EOF
Homebrew tap update for $tag is ready.

Tap checkout: $tap_dir
Formula:      $tap_dir/Formula/typekart.rb

Next steps:
  1. Push the tap commit if you did not pass --push:
     git -C "$tap_dir" push origin HEAD
  2. Verify install:
     brew update
     brew install tom-sitter/tap/typekart
EOF
