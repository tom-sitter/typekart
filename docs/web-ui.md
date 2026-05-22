# Web UI

TypeKart has an early browser UI in `web/typekart-web`. The web app is a Leptos
CSR application served by Trunk.

The current browser build can join and play in a CLI-hosted online room. It
cannot host a room yet.

## Current Capabilities

- Join an existing online room through the TypeKart relay.
- Disconnect from an active browser relay session and retry joining after a
  failure or closed room.
- Render lobby, race, and results messages from a CLI host.
- Ready and unready from the browser.
- Rename the browser player from the lobby.
- Show lobby and rematch controls only when they are valid for the browser
  player's current phase.
- Show host-only lobby management controls for adding AI racers, changing AI
  difficulty, and removing lobby players. These are mostly preparatory until
  browser hosting is implemented.
- Type during a race with letter keys, Space, and Backspace without manually
  focusing the race panel.
- Render the browser player as the first race lane.
- Render track words, bonus words, racer markers, item/effect cues, minimap,
  and events.
- Switch static gallery scenarios between Unicode and ASCII item cues.

## Known Limitations

- Browser hosting is not implemented.
- Host-only browser lobby controls are present but require a browser-hosted
  player to be useful.
- Automatic reconnect is not implemented.
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

Click `Join`, then `Ready`. During the race, type with letter keys, Space, and
Backspace.

The browser disables the connection fields while connected. Use `Disconnect` to
leave the current relay session, edit the connection fields, and join again.
If the host closes the room or the relay rejects the join, the browser clears
the live view and leaves the fields editable for another attempt.

## Browser Gameplay Validation

Use this checklist while browser parity is still in progress:

1. Start a CLI-hosted online room against the local relay.
2. Join from the browser and confirm the lobby shows the browser player.
3. Confirm only phase-valid controls are visible: `Ready` before readying,
   `Unready` after readying, and no host-only `Start` button for joiners.
4. Rename the browser player and confirm the lobby roster updates.
5. Start the race from the CLI host and confirm the browser sees the countdown.
6. Type from the browser during `Racing` without clicking the track first and
   confirm only the browser player's marker advances.
7. Finish the race and confirm results render with `Ready for rematch`.
8. Click `Disconnect` and confirm the live lobby/race view clears.
9. Close the CLI host and confirm the browser reports the closed room without
   leaving stale race UI behind.

Host-only lobby controls should be validated once browser hosting exists:

- Add AI racer.
- Remove AI racer.
- Kick a non-host human player.
- Change one AI racer's difficulty.
- Change all AI racers to Easy or Hard.

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
- Add automatic reconnect if manual retry feels insufficient.
- Implement browser hosting so host-only lobby controls can be exercised from
  the browser.
- Continue renderer parity work.
