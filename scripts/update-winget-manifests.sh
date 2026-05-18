#!/usr/bin/env sh
set -eu

usage() {
  cat <<'USAGE'
Usage: scripts/update-winget-manifests.sh VERSION [--winget-pkgs-dir PATH] [--validate]

Updates the TypeKart WinGet manifests for an already-published GitHub release.

Arguments:
  VERSION                SemVer version without a leading "v", for example 0.2.0

Options:
  --winget-pkgs-dir PATH Optional microsoft/winget-pkgs checkout. When provided,
                         the updated manifests are copied to the expected
                         manifests/t/tom-sitter/TypeKart/VERSION directory.
  --validate             Run "winget validate" on the updated manifest directory
                         when the winget command is available.
  -h, --help             Show this help text

Examples:
  scripts/update-winget-manifests.sh 0.2.0
  scripts/update-winget-manifests.sh 0.2.0 --winget-pkgs-dir ../winget-pkgs --validate
USAGE
}

die() {
  printf 'winget: %s\n' "$*" >&2
  exit 1
}

version=
winget_pkgs_dir=
validate=0

while [ "$#" -gt 0 ]; do
  case "$1" in
    --winget-pkgs-dir)
      shift
      [ "${1:-}" ] || die "--winget-pkgs-dir requires a path"
      winget_pkgs_dir=$1
      ;;
    --validate)
      validate=1
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
command -v tr >/dev/null 2>&1 || die "tr is required"

git rev-parse --show-toplevel >/dev/null 2>&1 || die "run this from inside the typekart repository"
repo_root=$(git rev-parse --show-toplevel)
cd "$repo_root"

manifest_dir="packaging/winget"
version_manifest="$manifest_dir/typekart.yaml"
installer_manifest="$manifest_dir/typekart.installer.yaml"
locale_manifest="$manifest_dir/typekart.locale.en-US.yaml"

[ -f "$version_manifest" ] || die "missing manifest: $version_manifest"
[ -f "$installer_manifest" ] || die "missing manifest: $installer_manifest"
[ -f "$locale_manifest" ] || die "missing manifest: $locale_manifest"

tag="v$version"
windows_archive="typekart-x86_64-pc-windows-msvc.zip"
windows_url="https://github.com/tom-sitter/typekart/releases/download/$tag/$windows_archive"
checksums_url="https://github.com/tom-sitter/typekart/releases/download/$tag/typekart-checksums.txt"
checksums_file=$(mktemp)
cleanup() {
  rm -f "$checksums_file"
}
trap cleanup EXIT INT TERM

printf 'Fetching release checksums: %s\n' "$checksums_url"
curl --fail --location --silent --show-error "$checksums_url" > "$checksums_file" \
  || die "failed to fetch checksums; wait for the GitHub release workflow to finish"

windows_sha=$(awk -v archive="$windows_archive" '$2 == archive { print $1 }' "$checksums_file" | tr '[:lower:]' '[:upper:]')
[ -n "$windows_sha" ] || die "missing Windows checksum in typekart-checksums.txt"

update_manifest() {
  input=$1
  tmp_file=$(mktemp)
  awk \
    -v version="$version" \
    -v windows_url="$windows_url" \
    -v windows_sha="$windows_sha" '
      /^PackageVersion:/ {
        print "PackageVersion: " version
        next
      }
      /^    InstallerUrl:/ {
        print "    InstallerUrl: " windows_url
        next
      }
      /^    InstallerSha256:/ {
        print "    InstallerSha256: " windows_sha
        next
      }
      { print }
    ' "$input" > "$tmp_file"
  cat "$tmp_file" > "$input"
  rm "$tmp_file"
}

update_manifest "$version_manifest"
update_manifest "$installer_manifest"
update_manifest "$locale_manifest"

target_dir=$manifest_dir
if [ -n "$winget_pkgs_dir" ]; then
  [ -d "$winget_pkgs_dir/.git" ] || die "winget-pkgs directory is not a git checkout: $winget_pkgs_dir"
  target_dir="$winget_pkgs_dir/manifests/t/tom-sitter/TypeKart/$version"
  mkdir -p "$target_dir"
  cp "$version_manifest" "$target_dir/tom-sitter.TypeKart.yaml"
  cp "$installer_manifest" "$target_dir/tom-sitter.TypeKart.installer.yaml"
  cp "$locale_manifest" "$target_dir/tom-sitter.TypeKart.locale.en-US.yaml"
fi

if [ "$validate" -eq 1 ]; then
  if command -v winget >/dev/null 2>&1; then
    winget validate "$target_dir"
  else
    printf 'winget command not found; skipping validation\n' >&2
  fi
fi

cat <<EOF
WinGet manifests updated for $tag.

Manifest source: $manifest_dir
Manifest target: $target_dir
Windows SHA256:  $windows_sha

Next steps:
  1. Review the manifest diff.
  2. Submit the manifests to microsoft/winget-pkgs when ready.
EOF
