# Web UI

TypeKart has an early browser UI in `web/typekart-web`. The web app is a Leptos
CSR application served by Trunk.

The current browser build can join and play in a CLI-hosted online room. It can
also create a browser-hosted relay room, manage the lobby, broadcast a
race-shell countdown, and advance racers through a fixed demo track.

## Current Capabilities

- Join an existing online room through the TypeKart relay.
- Create a browser-hosted relay room for lobby and race-shell testing.
- Disconnect from an active browser relay session and retry joining after a
  failure or closed room.
- Render lobby, race, and results messages from a CLI host.
- Ready and unready from the browser.
- Rename the browser player from the lobby.
- Show lobby and rematch controls only when they are valid for the browser
  player's current phase.
- Show host-only lobby management controls for adding AI racers, changing AI
  difficulty, and removing lobby players.
- Start a browser-hosted race shell that broadcasts `WaitingForHost`,
  `Countdown`, and `Racing` snapshots to connected clients.
- Type during a race with letter keys, Space, and Backspace without manually
  focusing the race panel.
- Render the browser player as the first race lane.
- Render track words, bonus words, racer markers, item/effect cues, minimap,
  and events.
- Switch static gallery scenarios between Unicode and ASCII item cues.

## Known Limitations

- Browser-hosted race gameplay is minimal: typing advances words on a fixed
  demo track, but bonuses, items, AI movement, results, and rematches are not
  implemented.
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

To create a browser-hosted lobby, leave `Room` blank and click `Create room`.
The relay assigns a room code and the browser becomes the host. Other browser
or terminal clients can join that room and appear in the lobby. The browser host
can rename itself, add AI racers, change AI difficulty, remove AI racers, and
kick non-host human players. Pressing `Start` broadcasts a race shell with
`WaitingForHost`, a `3 -> 2 -> 1` countdown, and a `Racing` snapshot. Typing
after that advances racers through the fixed demo track.

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

Browser-hosted lobby validation:

1. Start a local relay.
2. Open the web app and click `Create room`.
3. Confirm the generated room code is written into the room field.
4. Join the room from another browser tab or terminal client.
5. Rename the browser host and confirm the lobby broadcast updates joiners.
6. Add an AI racer.
7. Remove an AI racer.
8. Kick a non-host human player.
9. Change one AI racer's difficulty.
10. Change all AI racers to Easy or Hard.
11. Click `Start` and confirm the host and joiners render the race track.
12. Confirm the countdown advances `3 -> 2 -> 1` and then reaches `Racing`.
13. Type from a browser or terminal joiner and confirm that player's marker
    advances through the fixed demo track.

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

As a joiner, the browser joins the relay directly over WebSocket. It sends
`RelayClientMessage::JoinRoom`, receives relay envelopes, decodes host payloads
as `ServerMessage`, and sends browser commands back as
`RelayClientMessage::ClientToHost`.

As a lobby-only host, the browser sends `RelayClientMessage::CreateRoom`,
maintains a small authoritative lobby model in the browser, handles
`JoinForwarded` and `ClientToHost` relay envelopes, sends direct `Welcome`
messages, and broadcasts `LobbySnapshot` updates. It can also synthesize a
temporary `RaceSnapshot` sequence for the browser-hosted race shell. The race
shell uses fixed demo track words and no bonus words; it exists to validate
host-driven race phases and basic typing before the shared game engine runs in
the browser.

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
milestone plan. The near-term target is turning the browser-hosted race shell
into a playable browser-hosted race:

- Add a manual browser validation checklist.
- Improve focus and phase-aware controls.
- Validate bonus words and all item effects against browser players.
- Add automatic reconnect if manual retry feels insufficient.
- Move enough shared game/session logic into a browser-compatible crate for
  browser-hosted races.
- Continue renderer parity work.
