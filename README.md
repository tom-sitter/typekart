# TypeKart

TypeKart is a terminal typing racer with kart-style item effects.

## Install

### macOS With Homebrew

After the Homebrew tap is published:

```sh
brew tap OWNER/tap
brew install typekart
```

### Windows With Scoop

After the Scoop bucket is published:

```powershell
scoop bucket add typekart https://github.com/OWNER/scoop-bucket
scoop install typekart
```

### Windows With WinGet

After the WinGet package is accepted:

```powershell
winget install OWNER.TypeKart
```

### Direct Downloads

Download the archive for your system from the GitHub releases page:

- `typekart-aarch64-apple-darwin.tar.gz` for Apple Silicon Macs
- `typekart-x86_64-apple-darwin.tar.gz` for Intel Macs
- `typekart-x86_64-pc-windows-msvc.zip` for Windows x64

The built-in word set is embedded in the binary, so the release archive contains everything needed to run the default game.

Maintainer release and package-manager notes are in [docs/install/distribution.md](docs/install/distribution.md).

## Releases

Create release commits and tags with:

```sh
scripts/release.sh 0.1.0
```

The GitHub release workflow publishes macOS and Windows archives from pushed `v*.*.*` tags.

## Requirements

- Rust installed locally for development builds.
- Run development commands from the repository root.

Installed binaries can use `typekart` directly. Development examples below use `cargo run --`.

## Single Player

Start a local race:

```sh
cargo run -- play
```

Start a shorter test race:

```sh
cargo run -- play --words 20
```

Race against AI players:

```sh
cargo run -- play --ai-racers 6 --ai-difficulty easy
```

Use harder AI players:

```sh
cargo run -- play --ai-racers 6 --ai-difficulty hard
```

Use ASCII-safe item markers:

```sh
cargo run -- play --ascii
```

## Multiplayer On One Computer

Terminal 1, host the race:

```sh
cargo run -- host --name host --bind 127.0.0.1:4000
```

Terminal 2, join the race:

```sh
cargo run -- join --name player2 --server 127.0.0.1:4000
```

Additional local players can join from more terminals with unique names:

```sh
cargo run -- join --name player3 --server 127.0.0.1:4000
```

## Multiplayer On A Local Network

The host should bind to all local interfaces:

```sh
cargo run -- host --name host --bind 0.0.0.0:4000
```

Other computers join using the host computer's local IP address:

```sh
cargo run -- join --name player2 --server HOST_IP:4000
```

Example:

```sh
cargo run -- join --name player2 --server 192.168.1.25:4000
```

## Multiplayer Through A Relay

Terminal 1, start a local development relay:

```sh
cargo run -- relay --bind 127.0.0.1:8080
```

Terminal 2, host an online room through the relay:

```sh
cargo run -- host-online --name host --relay ws://127.0.0.1:8080
```

The host prints a room code. Terminal 3 can join with that room code:

```sh
cargo run -- join-online --name player2 --relay ws://127.0.0.1:8080 --room rocket-salad-tiger
```

For public internet play, run the relay behind a TLS reverse proxy and use a `wss://` relay URL. Deployment notes are in [docs/relay-deployment.md](docs/relay-deployment.md).

Build and run the relay container:

```sh
docker build -t typekart-relay .
docker run --rm -p 8080:8080 typekart-relay
```

Deployment configs are included for Fly.io (`fly.toml`) and Render (`render.yaml`).

## Common Multiplayer Options

Set the track length:

```sh
cargo run -- host --name host --words 20 --bind 127.0.0.1:4000
```

Set the lobby size, including the host:

```sh
cargo run -- host --name host --max-players 6 --bind 127.0.0.1:4000
```

Add network AI racers for multiplayer testing:

```sh
cargo run -- host --name host --bind 127.0.0.1:4000 --ai-racers 4 --ai-difficulty easy
```

Use ASCII-safe item markers for host and joiners:

```sh
cargo run -- host --name host --bind 127.0.0.1:4000 --ascii
cargo run -- join --name player2 --server 127.0.0.1:4000 --ascii
```

Write debug logs:

```sh
cargo run -- host --name host --bind 127.0.0.1:4000 --debug-log host-debug.log
cargo run -- join --name player2 --server 127.0.0.1:4000 --debug-log player2-debug.log
```

## Word Sets And Item Packs

Use a custom newline-delimited word file:

```sh
cargo run -- play --word-set-file ./mods/words/custom.txt
```

Choose a random `.txt` word set from a directory:

```sh
cargo run -- play --word-set-dir ./mods/words
```

Use an item pack:

```sh
cargo run -- play --item-pack-file ./mods/items/classic-plus.json
```

Use mods while hosting:

```sh
cargo run -- host --name host --bind 127.0.0.1:4000 \
  --word-set-dir ./mods/words \
  --item-pack-file ./mods/items/classic-plus.json
```

## In-Game Flow

- In single player, press `Space` to start the countdown.
- In multiplayer, joiners press `Enter` to ready up, then the host presses `Space` to start.
- After a multiplayer race, the host can return to the lobby or start another race from the results screen.

For the full CLI reference:

```sh
cargo run -- --help
cargo run -- play --help
cargo run -- host --help
cargo run -- join --help
```

For structured multiplayer validation, see [docs/lan-validation-checklist.md](docs/lan-validation-checklist.md).
