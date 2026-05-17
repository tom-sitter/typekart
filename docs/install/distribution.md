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

After the GitHub release finishes, copy the SHA-256 values from `typekart-checksums.txt` into the package manifests under `packaging/`.

To regenerate notes without cutting a release:

```sh
scripts/generate-release-notes.sh 0.1.0 > docs/releases/v0.1.0.md
```

## Homebrew

Create a tap repository named `homebrew-tap`, then add `packaging/homebrew/typekart.rb` as `Formula/typekart.rb`.

Users will install with:

```sh
brew tap tom-sitter/tap
brew install typekart
```

Before publishing the tap, replace:

- `REPLACE_WITH_ARM64_SHA256`
- `REPLACE_WITH_X86_64_SHA256`

Validate locally:

```sh
brew install --build-from-source ./Formula/typekart.rb
brew test typekart
brew audit --strict typekart
```

## Scoop

Create a Scoop bucket repository, then add `packaging/scoop/typekart.json` as `bucket/typekart.json`.

Users will install with:

```powershell
scoop bucket add typekart https://github.com/tom-sitter/scoop-bucket
scoop install typekart
```

Before publishing the bucket, replace:

- `REPLACE_WITH_WINDOWS_SHA256`

## WinGet

Use the manifests under `packaging/winget/` as the starting point for a pull request to `microsoft/winget-pkgs`.

Before submitting, replace:

- `REPLACE_WITH_WINDOWS_SHA256_UPPERCASE`
- publisher details if the package should use an organization or personal name

Validate with:

```powershell
winget validate .\packaging\winget
```

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
