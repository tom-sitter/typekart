# Web UI

TypeKart has an early browser UI in `web/typekart-web`. The web app is a Leptos
CSR application served by Trunk.

The current browser build can join and play in a CLI-hosted online room. It can
also create a browser-hosted relay room, manage the lobby, broadcast a
race-shell countdown, and advance racers through generated tracks.

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
- Generate browser-hosted race tracks through the shared `Track::generate`
  helper.
- Generate browser-hosted bonus points through the shared `BonusState`
  placement and word-choice helper.
- Claim browser-hosted bonus words through the shared bonus claim helper and
  put claimed choices on cooldown.
- Automatically activate claimed browser-hosted items and render shared
  item/effect cues in the race snapshot.
- Use the shared Rust typing engine for browser-hosted human and AI race
  progress.
- End browser-hosted races when all racers finish, or after the first-place
  timeout expires, and broadcast shared result-row rankings.
- Let browser hosts start another countdown from results after racers mark
  ready for rematch.
- Move browser-hosted AI racers during `Racing` based on their lobby WPM.
- Type during a race with letter keys, Space, and Backspace without manually
  focusing the race panel.
- Render the browser player as the first race lane.
- Render track words, bonus words, racer markers, item/effect cues, minimap,
  and events.
- Switch static gallery scenarios between Unicode and ASCII item cues.

## Known Limitations

- Browser-hosted race gameplay is still minimal: typing, AI word progress, race
  completion, track generation, bonus claiming, item activation, and result rows
  use shared game rules, but browser word-pack selection is not implemented.
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
after that advances racers through the generated track, and AI racers advance
automatically from their lobby WPM. When all racers finish, the browser host
broadcasts results. The host can press `Start` from results to begin another
countdown with connected ready racers.

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
    advances through the generated track.
14. Confirm AI racers advance without keyboard input.
15. Move to a bonus gap, type a visible bonus word, press Space, and confirm the
    claimed bonus word changes to cooldown.
16. Confirm the claimed item activates automatically and visible racer effects
    update on the track.

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
shell generates track words with `typekart::game::track::Track::generate` and
bonus points with `typekart::game::bonus::BonusState::generate`. Bonus claims
use `typekart::game::bonus::claim_bonus_choice`, which keeps choice cooldowns
aligned with terminal races. Claimed item effects are applied through
`typekart::game::item_effects`, a browser-compatible extraction from the
authoritative item-effect rules used by multiplayer hosts. Human input and AI
ticks now mutate a shared `typekart::game::race::RaceState`, then derive the
protocol `RaceSnapshot` from that state. Result rows are also derived from the
shared race result helper before being mapped into protocol messages. This keeps
basic typing behavior, typo handling, space handling, final-word finish
behavior, per-player typing state, bonus placement, bonus cooldowns, item state,
and result ranking aligned with terminal races.

The web crate depends on the root `typekart` library with default CLI features
disabled. The root library always exposes `game`, while terminal, relay, and
native UI modules remain behind the default `cli` feature so they are not pulled
into the browser build.

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
- Validate all browser-hosted item effects manually against browser and terminal
  players.
- Add automatic reconnect if manual retry feels insufficient.
- Move more shared session logic into a browser-compatible boundary, especially
  bonus claiming and items.
- Continue renderer parity work.
