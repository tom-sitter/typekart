# Development Guide

## Setup

Install Rust, then run commands from the repository root.

```sh
cargo run -- play
```

Useful development commands:

```sh
cargo run -- --help
cargo run -- host --help
cargo run -- join --help
cargo run -- host-lan --help
cargo run -- join-lan --help
```

## Checks

Run the same checks used before pushing:

```sh
scripts/check.sh
```

The script runs:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

## Web UI Development

The browser UI is a Leptos CSR app served by Trunk. Install the browser build
tools once:

```sh
rustup target add wasm32-unknown-unknown
cargo install trunk
```

Run the web shell locally:

```sh
cd web/typekart-web
trunk serve
```

Check the web package from the repository root:

```sh
cargo check --manifest-path web/typekart-web/Cargo.toml
```

Check the shared browser/terminal protocol crate:

```sh
cargo test --manifest-path crates/typekart-protocol/Cargo.toml
```

## Relay Load Testing

The hidden `relay-load-test` command opens synthetic relay rooms and joiners,
then sends realistic host broadcasts and joiner key-input messages. It tests the
relay path directly without terminal rendering or local game-loop overhead.

Run a short local relay test:

```sh
cargo run -- relay --bind 127.0.0.1:8080
cargo run -- relay-load-test --relay ws://127.0.0.1:8080 --start-games 5 --max-games 20 --step-games 5 --duration-secs 10
```

Run against the public relay carefully:

```sh
cargo run -- relay-load-test --start-games 10 --max-games 100 --step-games 10 --duration-secs 30
```

Each synthetic game uses one host WebSocket plus `--joiners-per-game` joiner
WebSockets. With the default `--joiners-per-game 5`, `30` games means `180`
relay connections.

## Local Multiplayer Testing

Internet path through a local relay:

```sh
cargo run -- relay --bind 127.0.0.1:8080
cargo run -- host --name host --relay ws://127.0.0.1:8080
cargo run -- join --name player2 --relay ws://127.0.0.1:8080 --room rocket-salad-tiger
```

LAN path on one machine:

```sh
cargo run -- host-lan --name host --bind 127.0.0.1:4000
cargo run -- join-lan --name player2 --server 127.0.0.1:4000
```

Use [LAN Validation Checklist](lan-validation-checklist.md) for structured manual testing.

Preview item and effect UI states without playing a full race:

```sh
cargo run -- gallery items
cargo run -- gallery items --scenario banana-hit-pack
```

See [Renderer Gallery](renderer-gallery.md) for controls and covered scenarios.

## Repository Layout

- `src/lib.rs`: reusable library surface for the CLI today and future browser/WASM code.
- `src/main.rs`: CLI parsing and command dispatch.
- `src/app.rs`: thin coordinator between CLI, game setup, UI, and networking.
- `src/game`: pure game rules, typing state, items, bonus words, words, mods, and AI rules.
- `src/ui`: terminal rendering and local session loop.
- `src/net`: LAN protocol, host server, client, relay protocol, relay server, and online loopback adapters.
- `crates/typekart-protocol`: shared JSON protocol contract used by the terminal app and browser app.
- `web/typekart-web`: Leptos browser UI shell.
- `docs`: user, operator, and maintainer documentation.
- `packaging`: Homebrew and WinGet package templates.
- `scripts`: release and packaging helpers.

For a deeper walkthrough, read [Codebase Tour](codebase-tour.md).

## Release Flow

Create release notes, commit, and tag with:

```sh
scripts/release.sh 0.1.0
```

Pushing a `v*.*.*` tag starts the GitHub release workflow for macOS and Windows archives.

After a GitHub release exists, update the Homebrew tap with:

```sh
scripts/update-homebrew-tap.sh 0.1.0
```

Update WinGet manifests with:

```sh
scripts/update-winget-manifests.sh 0.1.0
```

Maintainer packaging details are in [Distribution Notes](install/distribution.md).
