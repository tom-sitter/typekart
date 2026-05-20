# Network Protocol

TypeKart uses JSON messages for online and LAN multiplayer. The relay protocol
wraps opaque game payloads so the relay can route rooms without knowing the game
schema. The game protocol is still host-authoritative: joiners send commands,
and the host sends lobby/race snapshots, race deltas, events, results, and
errors.

This contract is the shared boundary for the terminal client and the planned
browser client.

## Envelope Layer

Online play uses relay envelopes from `src/net/relay.rs`.

Client to relay:

- `create_room`: host asks the relay to create a room and includes
  `host_version`.
- `join_room`: joiner asks to join a room with `room`, `name`, and
  `client_version`.
- `client_to_host`: joiner sends an opaque game `message` to the host.
- `host_to_client`: host sends an opaque game `message` to one joiner.
- `host_broadcast`: host sends an opaque game `message` to every joiner.
- `leave_room`: participant leaves a room.

Relay to client:

- `room_created`: relay returns the generated room code.
- `join_forwarded`: relay forwards a join request to the host.
- `client_to_host`: relay forwards a joiner game message to the host.
- `host_to_client`: relay forwards a host game message to one joiner.
- `host_broadcast`: relay forwards a host game message to every joiner.
- `error`: relay rejected a request.
- `room_closed`: host room closed.
- `participant_disconnected`: a participant connection ended.

The relay should not need redeploying when normal game commands change. It only
needs to understand the envelope layer and room routing rules.

## Game Layer

Shared game messages live in `crates/typekart-protocol`. The terminal app
re-exports those types from `src/net/protocol.rs` and keeps game-specific
conversion impls there.

Joiners send `ClientMessage` values:

- `hello`: native LAN join handshake. Online joins use the relay-level
  `join_room` first.
- `set_ready`: ready or unready from the lobby.
- `rename`: rename the player before or during lobby phases.
- `start_countdown`: host starts a race or rematch countdown.
- `add_ai`: host adds an AI racer to the lobby.
- `remove_lobby_player`: host removes an AI or human lobby player.
- `set_ai_difficulty`: host changes one AI, or all future AIs, to easy/hard.
- `key_input`: race input with a monotonic client sequence.
- `restart_race`: host cancels a running race and returns everyone to lobby.
- `leave`: player leaves cleanly.

Hosts send `ServerMessage` values:

- `welcome`: accepted join response with assigned id and color.
- `lobby_snapshot`: full lobby state, active mods, and recent lobby events.
- `race_snapshot`: full race state, including track words and all players.
- `race_delta`: compact race update with current phase, bonuses, players, and
  events.
- `race_event`: standalone event text.
- `race_results`: final ranking rows for every racer.
- `error`: rejected command or incompatible client state.

## Browser Host Flow

1. Browser host opens a WebSocket to the relay.
2. Host sends `create_room`.
3. Relay returns `room_created`.
4. Browser joiner sends `join_room` with room, name, and app version.
5. Relay sends `join_forwarded` to the host.
6. Host validates version, mod compatibility, name, and capacity.
7. Host responds with `welcome` through `host_to_client`.
8. Host broadcasts `lobby_snapshot` and later `race_snapshot`/`race_delta`.

## Version And Mod Compatibility

Clients should use the same TypeKart version as the host. If the host rejects a
join because of version mismatch, the error should include both versions.

The host controls the active word set and item pack. Joiners render snapshots
from host data, but mod hashes are included so clients can show clear mismatch
diagnostics and future browser clients can preflight local assets.

## Fixture Coverage

The Rust tests include fixture-style assertions for browser-required JSON
shapes. When a protocol change is intentional, update this document and the
fixtures together.
