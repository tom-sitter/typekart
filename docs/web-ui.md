# Web UI

TypeKart has an early browser UI in `web/typekart-web`. The web app is a Leptos
CSR application served by Trunk.

The current browser build can join and play in a CLI-hosted online room. It
cannot host a room yet.

## Current Capabilities

- Join an existing online room through the TypeKart relay.
- Render lobby, race, and results messages from a CLI host.
- Ready and unready from the browser.
- Type during a race with letter keys, Space, and Backspace.
- Render the browser player as the first race lane.
- Render track words, bonus words, racer markers, item/effect cues, minimap,
  and events.
- Switch static gallery scenarios between Unicode and ASCII item cues.

## Known Limitations

- Browser hosting is not implemented.
- Browser lobby controls are minimal.
- The race panel must be focused before typing.
- The `Start` button is wired, but a CLI host ignores start requests from
  non-host players.
- Reconnect and host-left messaging are basic.
- Renderer parity with the terminal UI is incomplete.
- Browser item/effect behavior needs structured manual validation.

## Local Development

Install the WASM target and Trunk once:

```sh
rustup target add wasm32-unknown-unknown
cargo install trunk
```

Run a local relay and CLI host:

```sh
cargo run -- relay --bind 127.0.0.1:8080
cargo run -- host --name host --relay ws://127.0.0.1:8080
```

Run the web app on a different port:

```sh
cd web/typekart-web
trunk serve --port 8081
```

Open the Trunk URL, choose `Join room`, enter:

```text
Relay: ws://127.0.0.1:8080
Room:  the room code printed by the CLI host
Name:  any player name
```

Click `Observe`, then `Ready`. During the race, click the race panel before
typing.

## Checks

Run these from the repository root:

```sh
cargo fmt --manifest-path web/typekart-web/Cargo.toml --all -- --check
cargo check --manifest-path web/typekart-web/Cargo.toml --locked
cargo test --manifest-path web/typekart-web/Cargo.toml --locked
cargo test --manifest-path crates/typekart-protocol/Cargo.toml --locked
```

Run the terminal app checks separately:

```sh
cargo check --locked
cargo test --locked
```

## Architecture Notes

The browser joins the relay directly over WebSocket. It sends
`RelayClientMessage::JoinRoom`, receives relay envelopes, decodes host payloads
as `ServerMessage`, and sends browser commands back as
`RelayClientMessage::ClientToHost`.

The browser stores two ids after joining:

- Relay participant id: the outer `HostToClient.player_id`, used for outbound
  relay routing.
- Game player id: the inner `Welcome.player_id`, used for local rendering
  perspective.

Keeping those ids separate is required because the relay route id and host game
id can differ.

The CLI host remains authoritative. The relay is still an opaque routing layer.

## Next Work

See [Web UI Implementation Plan](web-ui-implementation-plan.md) for the full
milestone plan. The near-term target is browser joiner parity before browser
hosting:

- Add a manual browser validation checklist.
- Improve focus and phase-aware controls.
- Validate bonus words and all item effects against browser players.
- Improve reconnect and room-closed UX.
- Continue renderer parity work.
