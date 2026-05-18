# Contributing

Thanks for helping test and improve TypeKart. The project is in beta, so the most useful contributions are clear bug reports, focused gameplay feedback, terminal compatibility notes, and small patches that make the first-run experience better.

## Ways To Help

- Report bugs with exact commands, screenshots or terminal output, operating system, terminal app, and `typekart --version`.
- Share beta feedback about installation, first race onboarding, multiplayer setup, readability, item balance, and AI difficulty.
- Improve documentation when a command, rule, or setup step is unclear.
- Submit focused code changes with tests or a short manual test note.

## Development Setup

Install Rust, then run commands from the repository root.

```sh
cargo run -- play
```

Useful local commands:

```sh
cargo run -- --help
cargo run -- play --ai-racers 3 --ai-difficulty easy
cargo run -- host --help
cargo run -- join --help
cargo run -- gallery items
```

For more detail, read the [development guide](docs/development.md), [gameplay guide](docs/gameplay.md), and [multiplayer guide](docs/multiplayer.md).

## Checks

Before opening a pull request, run:

```sh
scripts/check.sh
```

That runs formatting, clippy, and tests:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

If your change affects multiplayer, also run at least one local manual multiplayer check:

```sh
cargo run -- host-lan --name host --bind 127.0.0.1:4000
cargo run -- join-lan --name player2 --server 127.0.0.1:4000
```

Use the [LAN validation checklist](docs/lan-validation-checklist.md) for broader multiplayer testing.

## Pull Requests

Keep pull requests focused. Include:

- What changed and why.
- How you tested it.
- Screenshots or terminal recordings for UI changes.
- Any follow-up work you intentionally left out.

Avoid mixing unrelated refactors with gameplay, networking, or documentation changes. Small patches are easier to review and safer to ship during beta.

## UI And Terminal Compatibility

TypeKart renders in real terminals, so UI changes should be checked in more than one terminal size when practical. If a visual issue depends on Unicode rendering, also try the ASCII fallback:

```sh
cargo run -- play --ascii
cargo run -- gallery items --ascii
```

The renderer gallery can preview item and effect states without playing a full race. See the [renderer gallery guide](docs/renderer-gallery.md).

## Releases

Maintainer release steps live in [distribution notes](docs/install/distribution.md). Normal contributions do not need to update release notes unless the maintainer asks for it.
