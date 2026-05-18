# Distribution

TypeKart is distributed as a standalone terminal binary. The built-in word set is embedded at compile time, so release archives do not need to carry `words_alpha.txt`.

## Supported Packages

- macOS Apple Silicon: `typekart-aarch64-apple-darwin.tar.gz`
- macOS Intel: `typekart-x86_64-apple-darwin.tar.gz`
- Windows x64: `typekart-x86_64-pc-windows-msvc.zip`

The release workflow builds these archives whenever a `v*.*.*` tag is pushed and publishes a GitHub release with `typekart-checksums.txt`.

## Create A Release

Use the release script from a clean release tree:

```sh
scripts/release.sh 0.1.0
```

The script updates `Cargo.toml` when needed, creates release notes under `docs/releases/`, updates `CHANGELOG.md`, validates the build, commits release file changes, and creates an annotated `v0.1.0` tag. Push the release when ready:

```sh
git push origin main
git push origin main --tags
```

To push automatically after the local release commit and tag are created:

```sh
scripts/release.sh 0.1.0 --push
```

After the GitHub release finishes, publish the Homebrew tap update:

```sh
scripts/update-homebrew-tap.sh 0.1.0 --push
```

Then update the WinGet manifests from the published release checksums:

```sh
scripts/update-winget-manifests.sh 0.1.0
```

To regenerate notes without cutting a release:

```sh
scripts/generate-release-notes.sh 0.1.0 > docs/releases/v0.1.0.md
```

## Homebrew

The public tap is published at `tom-sitter/homebrew-tap`. Update it after each GitHub release with:

```sh
scripts/update-homebrew-tap.sh 0.1.0 --push
```

Users will install with:

```sh
brew tap tom-sitter/tap
brew install typekart
```

The update script fills:

- `REPLACE_WITH_ARM64_SHA256`
- `REPLACE_WITH_X86_64_SHA256`
- `REPLACE_WITH_VERSION`

Validate locally:

```sh
brew audit --strict --online tom-sitter/tap/typekart
brew style tom-sitter/tap/typekart
brew test tom-sitter/tap/typekart
```

## WinGet

Use the manifests under `packaging/winget/` as the starting point for a pull request to `microsoft/winget-pkgs`.
The package is not installable with `winget install tom-sitter.TypeKart` until
that pull request is merged and the public WinGet source has indexed it.

After each GitHub release, update the manifests with:

```sh
scripts/update-winget-manifests.sh 0.1.0
```

To copy the updated files into a local `microsoft/winget-pkgs` checkout and validate them:

```sh
scripts/update-winget-manifests.sh 0.1.0 --winget-pkgs-dir ../winget-pkgs --validate
```

The script fills:

- `PackageVersion`
- `InstallerUrl`
- `InstallerSha256`

## Direct Archive Install

macOS:

```sh
curl -LO https://github.com/tom-sitter/typekart/releases/download/v0.1.0/typekart-aarch64-apple-darwin.tar.gz
tar -xzf typekart-aarch64-apple-darwin.tar.gz
sudo install typekart /usr/local/bin/typekart
typekart --help
```

Windows PowerShell:

```powershell
Invoke-WebRequest https://github.com/tom-sitter/typekart/releases/download/v0.1.0/typekart-x86_64-pc-windows-msvc.zip -OutFile typekart.zip
Expand-Archive typekart.zip -DestinationPath typekart
.\typekart\typekart.exe --help
```
