#!/usr/bin/env sh
set -eu

usage() {
  cat <<'USAGE'
Usage: scripts/release.sh VERSION [--push] [--skip-checks]

Creates a TypeKart release commit and annotated git tag.

Arguments:
  VERSION        SemVer version without a leading "v", for example 0.2.0

Options:
  --push         Push the release commit and tag after creating them
  --skip-checks  Skip cargo fmt, test, clippy, and release build checks
  -h, --help     Show this help text

Examples:
  scripts/release.sh 0.2.0
  scripts/release.sh 0.2.1 --push
USAGE
}

die() {
  printf 'release: %s\n' "$*" >&2
  exit 1
}

version=
push_release=0
skip_checks=0

while [ "$#" -gt 0 ]; do
  case "$1" in
    --push)
      push_release=1
      ;;
    --skip-checks)
      skip_checks=1
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

command -v git >/dev/null 2>&1 || die "git is required"
command -v cargo >/dev/null 2>&1 || die "cargo is required"

git rev-parse --show-toplevel >/dev/null 2>&1 || die "run this from inside the git repository"
repo_root=$(git rev-parse --show-toplevel)
cd "$repo_root"

tag="v$version"

git rev-parse --verify "$tag" >/dev/null 2>&1 && die "tag already exists: $tag"

dirty_release_files=$(git status --short -- Cargo.toml Cargo.lock packaging docs/install README.md .github scripts)
if [ -n "$dirty_release_files" ]; then
  printf '%s\n' "$dirty_release_files" >&2
  die "release files are dirty; commit or stash them before releasing"
fi

current_version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
[ "$current_version" != "$version" ] || die "Cargo.toml is already at version $version"

printf 'Updating Cargo package version: %s -> %s\n' "$current_version" "$version"
cargo metadata --no-deps --format-version 1 >/dev/null

if command -v cargo-set-version >/dev/null 2>&1; then
  cargo set-version "$version"
else
  command -v awk >/dev/null 2>&1 || die "awk is required when cargo-edit is not installed"
  tmp_file=$(mktemp)
  awk -v version="$version" '
    !updated && /^version = "/ {
      print "version = \"" version "\""
      updated = 1
      next
    }
    { print }
  ' Cargo.toml > "$tmp_file"
  cat "$tmp_file" > Cargo.toml
  rm "$tmp_file"
fi

cargo metadata --format-version 1 >/dev/null
cargo metadata --locked --format-version 1 >/dev/null

if [ "$skip_checks" -eq 0 ]; then
  cargo fmt --all -- --check
  cargo test --locked
  cargo clippy --locked --all-targets -- -D warnings
  cargo build --locked --release
fi

git add Cargo.toml Cargo.lock
git diff --cached --quiet && die "no version changes staged"

git commit -m "Release $tag"
git tag -a "$tag" -m "Release $tag"

if [ "$push_release" -eq 1 ]; then
  git push origin HEAD
  git push origin "$tag"
fi

cat <<EOF
Release $tag is ready.

Next steps:
  1. Push the release if you did not pass --push:
     git push origin HEAD
     git push origin $tag
  2. Wait for the GitHub release workflow to publish archives and checksums.
  3. Update Homebrew, Scoop, and WinGet manifests from typekart-checksums.txt.
EOF
