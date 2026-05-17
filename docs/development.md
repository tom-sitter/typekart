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
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

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

## Repository Layout

- `src/main.rs`: CLI parsing and command dispatch.
- `src/app.rs`: thin coordinator between CLI, game setup, UI, and networking.
- `src/game`: pure game rules, typing state, items, bonus words, words, mods, and AI rules.
- `src/ui`: terminal rendering and local session loop.
- `src/net`: LAN protocol, host server, client, relay protocol, relay server, and online loopback adapters.
- `docs`: user, operator, and maintainer documentation.
- `packaging`: package-manager templates.
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

Maintainer packaging details are in [Distribution Notes](install/distribution.md).
