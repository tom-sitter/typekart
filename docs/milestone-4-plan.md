# Milestone 4 Implementation Plan: Local Network Multiplayer

## Goal

Add local-network multiplayer while preserving the current single-machine game feel.

Milestone 4 should let one player host a race and other terminal clients join over TCP on the same network. The host remains a player. The server owns all authoritative race state, and clients send input commands and render server snapshots.

## Scope

Milestone 4 includes:

- `host` and `join` CLI flows.
- A local-network TCP server.
- Newline-delimited JSON protocol messages.
- Lobby join with unique display names.
- Host-as-player support.
- Up to 6 total human players.
- Server-authoritative typing input.
- Server-authoritative bonus claims, item rolls, item targeting, countdown, finish order, and race end.
- Client-side terminal rendering from snapshots.
- Basic disconnect handling.
- Debug logs for network messages and item resolution.

## Current Implementation Status

Milestone 4 is in progress.

Implemented:

- `host` and `join` CLI commands.
- TCP listener/client connection using newline-delimited JSON.
- Protocol message types and round-trip tests.
- Lobby join flow with capacity checks.
- Unique active display names.
- Host-as-player lobby entry.
- Up to 6 connected players.
- Color assignment with slot reuse after disconnects.
- Readiness commands for host and joiners.
- Basic disconnect handling.
- Host-started countdown.
- Server-broadcast `RaceSnapshot` messages for countdown and racing phases.
- Server-owned `RaceState` with generated track words.
- Server-authoritative key input for joined players and the host.
- Raw per-keystroke terminal input for `join` after the race starts.
- Race snapshots containing track words, player word index, current input, typo index, finished state, and connection state.

Not implemented yet:

- Ratatui race rendering for network snapshots.
- Fixed-rate race snapshot broadcast loop.
- Server-owned bonus state.
- Server-owned multiplayer item resolution.
- Finish order and race-end timeout.
- Network debug-log file support.
- Local host-as-client architecture. The current host process is also the player, but it reads host commands directly rather than connecting to itself as a normal client.

Milestone 4 excludes:

- Internet matchmaking or relay hosting.
- Encryption or authentication.
- Browser clients.
- Spectators.
- Reconnect/resume.
- Advanced latency compensation.
- Chat.
- New item types.

## Technical Direction

Use TCP for the first network version.

Use newline-delimited JSON for protocol messages:

```text
{"type":"key_input","sequence":42,"key":{"type":"char","value":"a"}}
{"type":"race_snapshot","sequence":120,"players":[...]}
```

This is not the most compact protocol, but it is inspectable, easy to log, and good enough for 2 to 6 terminal players on a local network.

Recommended new dependencies:

```toml
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

Avoid async networking for this milestone unless we hit a clear blocker. A thread-per-client server with channels is simpler to learn, test, and debug, and 6 players is small enough that this is practical.

## Proposed CLI

Current local command remains:

```sh
cargo run -- play
```

New host command:

```sh
cargo run -- host --name tom --bind 0.0.0.0:4000 --words 40 --max-players 6
```

New join command:

```sh
cargo run -- join --name alex --server 192.168.1.20:4000
```

Useful debug option on both:

```sh
--debug-log typekart-debug.log
```

`--debug-log` is planned but is not implemented for `host` or `join` yet.

Current transitional network controls:

```text
Host lobby: ready, unready, lobby, start
Join lobby: ready, unready, quit
After racing starts: character keys, Space, and Backspace are sent immediately
Leave during network racing: Esc or Ctrl-C
```

The join client still renders snapshots as plain text. The next client slice should switch network play to a Ratatui renderer so it feels like local `play`.

## Proposed File Changes

Likely new modules:

```text
src/net/mod.rs
src/net/protocol.rs
src/net/server.rs
src/net/client.rs
src/game/race.rs
```

Likely changed modules:

```text
src/main.rs
src/app.rs
src/game/mod.rs
src/game/player.rs
src/ui/session.rs
src/ui/terminal.rs
src/ui/render.rs
docs/codebase-tour.md
docs/game-design.md
docs/technical-plan.md
```

## Architecture

### Current Local Shape

`LocalSession` currently owns:

- The local player's `PlayerState`.
- AI racers.
- Track and bonus state.
- Item resolution.
- Countdown and finish status.
- UI-facing events.

That worked well for local development, but multiplayer needs these responsibilities split.

### Target Multiplayer Shape

The server owns authoritative race state:

- Track.
- Bonus state.
- Player states.
- Active effects.
- Item targeting.
- Attack warnings.
- Race phase.
- Results.

Each client owns:

- Terminal input.
- Latest received snapshot.
- Local display preferences such as Unicode icons.
- Connection state.

The renderer should not care whether a snapshot came from local play, AI simulation, or a real server.

## State Model

Introduce a reusable `RaceState` under `game`.

Initial shape:

```rust
pub struct RaceState {
    pub track: Track,
    pub bonuses: BonusState,
    pub players: Vec<RacePlayer>,
    pub phase: RacePhase,
    pub status: RaceStatus,
}

pub struct RacePlayer {
    pub id: PlayerId,
    pub name: String,
    pub color: PlayerColor,
    pub state: PlayerState,
    pub impact_until: Option<Instant>,
    pub item_cue: Option<ItemCue>,
}
```

Important design choice: do not put terminal-specific color types in the authoritative game state. Use a small game-level color id, then map it to Ratatui colors in the renderer.

## Protocol

### Client To Server

```text
Hello { name, client_version }
SetReady { ready }
StartCountdown
KeyInput { sequence, key }
RestartRace
Leave
```

The `KeyInput` payload should reuse the existing `KeyAction` concepts:

```text
Char(char)
Space
Backspace
```

Manual item activation messages can stay in the protocol for future compatibility, but current automatic item activation means they do not need to do anything yet.

### Server To Client

```text
Welcome { player_id, assigned_color }
LobbySnapshot
RaceSnapshot
RaceEvent
RaceResults
Error
```

`RaceSnapshot` should include enough display state for each client to render without re-running rules locally:

- Race phase.
- Track words.
- Bonus choices and cooldown state.
- All player progress and display effects.
- Event feed entries.
- Results if the race ended.

For Milestone 4, every client can receive the same snapshot. Later, if bonus visibility or item information becomes player-specific, snapshots can gain per-recipient views.

## Host Flow

1. Host runs `typekart host`.
2. App starts a TCP server bound to the requested address.
3. Current implementation reads host lobby and race input directly from the host terminal.
4. Host appears in the lobby as a normal player.
5. Other players join.
6. Host marks ready.
7. Host runs `start` to start countdown.
8. Server broadcasts countdown snapshots.
9. Race starts simultaneously for connected clients.

Target later flow:

1. Host process also starts a local client connected to its own server.
2. Host presses Space to start countdown from the same raw terminal path used by joiners.

This keeps the host code path closer to every other client path, but we have not moved to that architecture yet.

## Current Network Race Flow

1. Host generates the race track from `words_alpha.txt`.
2. Server stores the track in `RaceState`.
3. When joiners connect, the server adds them to lobby state and race state.
4. When all connected players are ready, the host can start countdown.
5. During `Racing`, `KeyInput` messages mutate the authoritative `RaceState`.
6. The server immediately broadcasts a `RaceSnapshot` after accepted key input.
7. The join client captures raw key events during racing and prints text snapshots.

Current manual test shape:

```sh
cargo run -- host --name host --words 20 --bind 127.0.0.1:4000 --max-players 2
cargo run -- join --name alex --server 127.0.0.1:4000
```

Then:

```text
join> ready
host> ready
host> start
join racing input: type firstword, press Space, then keep typing
```

Each racing key press is converted into a `KeyInput` message immediately.

## Join Flow

1. Joiner runs `typekart join`.
2. Client opens a TCP connection.
3. Client sends `Hello`.
4. Server validates name uniqueness and capacity.
5. Server replies with `Welcome`.
6. Client renders lobby snapshots until countdown/race begins.

## Implementation Phases

### Phase 1: Protocol Types

Add serializable message types and tests.

Status: complete.

Deliverables:

- `ClientMessage`.
- `ServerMessage`.
- JSON encode/decode helpers.
- Tests for round-tripping common messages.

Validation:

```sh
cargo test protocol
```

### Phase 2: Extract Shared Race State

Move the authoritative local race rules out of `LocalSession` into a reusable game-level structure.

Status: partially complete.

Implemented:

- `RaceState`.
- `RacePlayer`.
- Server-style method for applying normal key input to one player.

Remaining:

- Shared bonus state.
- Shared item resolution.
- Shared finish order/race status.
- Adapting local `LocalSession` to the shared state, if we choose that path.

Deliverables:

- `RaceState`.
- `RacePlayer`.
- Server-style methods for applying key input.
- Server-style methods for countdown, bonus claim, item resolution, and finish order.
- `LocalSession` adapted to call the shared race state, or kept as a thin compatibility wrapper.

This is the highest-risk phase because it touches working gameplay. Keep changes small and preserve current tests.

Validation:

```sh
cargo test
cargo run -- play --ai-racers 6
```

### Phase 3: Server Skeleton

Add a TCP server that can accept clients and broadcast lobby snapshots.

Status: complete for the lobby skeleton.

Deliverables:

- `host` command starts the server.
- Server accepts up to 6 clients.
- Server assigns player ids and colors.
- Server rejects duplicate names.
- Server broadcasts lobby membership.

Validation:

- Start one host terminal.
- Start one join terminal.
- Confirm both see the lobby list.

### Phase 4: Client Skeleton

Add a network client loop that sends inputs and renders snapshots.

Status: partially complete.

Implemented:

- `join` connects and sends lobby commands.
- Client receives lobby snapshots and race snapshots.
- Client sends raw key-derived `KeyInput` after racing starts.
- Client prints text snapshots.

Remaining:

- Client-side Ratatui rendering from snapshots.
- Channels between input, network reader, and renderer.

Deliverables:

- `join` command connects to a host.
- Client reads terminal input.
- Client sends `KeyInput` and lobby/start commands.
- Client renders the latest `RaceSnapshot`.

Implementation note: use channels between the terminal loop and a network thread so terminal rendering stays responsive.

### Phase 5: Multiplayer Race Loop

Wire the server to the shared authoritative race state.

Status: started.

Implemented:

- Host-started countdown.
- Server applies client key input to `RaceState`.
- Server broadcasts immediate snapshots after key input.
- Final-word completion behavior comes from the shared typing engine.

Remaining:

- Fixed-rate snapshot loop.
- Finish order.
- Race end when all racers finish or timeout expires.
- Full two-terminal race UI.

Deliverables:

- Host Space starts countdown.
- Server applies client key inputs.
- Server broadcasts race snapshots 10 to 20 times per second.
- Finish order is server authoritative.
- Race ends when all players finish or the post-first-finish timeout expires.

Validation:

- Two local terminals can race each other.
- Typo behavior matches current local play.
- Final-word completion does not require a trailing Space.

### Phase 6: Multiplayer Items

Move current item behavior from AI/local simulation into player-vs-player server rules.

Deliverables:

- Bonus claims are serialized by the server.
- Bonus cooldowns are shared.
- Mushroom, Shield, and Banana work against human players.
- Banana ignores finished and already-stunned players.
- Shield blocks Banana and consumes the active shield.
- Item cues and impact blinks appear on all relevant clients.

Validation:

- Two clients can contest bonus points.
- A Banana hit visibly affects the target.
- A shielded target blocks Banana.
- Debug log can reconstruct item targeting.

### Phase 7: Diagnostics And Stabilization

Add enough logging and tests to make local network bugs diagnosable.

Deliverables:

- `--debug-log` works for host and join.
- Server log includes joins, disconnects, inputs, snapshots, item targeting, and finish events.
- Client log includes sent inputs, received snapshots, and connection errors.
- Basic handling for a disconnected client.

## Testing Strategy

Keep most behavior tests below the network layer.

Unit tests:

- Protocol encode/decode.
- Race state input application.
- Bonus claim serialization.
- Item targeting.
- Finish order.

Integration-style tests:

- Server accepts a client.
- Duplicate names are rejected.
- Server applies a key input and emits a snapshot.
- Race starts after host start command.

Manual tests:

- Host and join on `127.0.0.1`.
- Host and join from two machines on a LAN.
- Race with 2 players.
- Race with 6 players.
- Quit/disconnect mid-lobby.
- Quit/disconnect mid-race.

## Open Questions

- Should clients choose names via CLI only, or should the terminal UI include a lobby/name prompt later?
- Should host auto-ready, or is host readiness implicit?
- Should disconnected racers remain visible as inactive, or be removed immediately?
- Should `play` continue to use `LocalSession`, or should it become an in-process server/client path?
- How much per-client snapshot customization do we need before real multiplayer items become confusing?

## Recommended First Slice

Start with protocol types and shared race state extraction.

Do not write socket code first. The network layer should be thin and boring; the real risk is making sure the game rules can be driven by server-style commands without regressing the current local loop.

First implementation checklist:

1. Add `serde` and `serde_json`.
2. Add `src/net/protocol.rs`.
3. Define `ClientMessage`, `ServerMessage`, and serializable key actions.
4. Add JSON round-trip tests.
5. Add `src/game/race.rs` behind tests only or with minimal integration.
6. Move one simple rule first: applying normal typing input to one player in `RaceState`.
7. Keep `cargo test` green after every small extraction.
