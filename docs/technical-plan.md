# TypeKart Technical Implementation Plan

## Purpose

This document describes the technical implementation plan for building TypeKart.

The project will start with Rust so the terminal UI, networking, and game-state model have room to become polished without the stack becoming the limiting factor. Some details remain open, but the implementation language and core terminal/networking direction are now decided.

## Product Constraints That Matter Technically

TypeKart is a real-time terminal multiplayer game. The implementation needs to support:

- Low-latency typing input.
- A continuously updating terminal display.
- Multiple players connected to one race.
- Server-authoritative race state.
- Shared bonus-word state and item effects.
- Per-player rendering differences, such as greyed-out bonus words when a player already has an item.
- Local network play first, with a path toward internet play later.

The most important technical risk is not raw performance. It is keeping typing responsive while synchronizing fair multiplayer state.

## Chosen Stack

### Baseline

Use Rust for the first implementation.

Current libraries:

- `ratatui` for terminal UI rendering.
- `crossterm` for terminal input, colors, and alternate-screen handling.
- `serde` for message serialization.
- `serde_json` for the first network protocol.
- Standard library TCP sockets and threads for the first local-network server/client.

Start with JSON messages for debuggability. If bandwidth or latency becomes a problem, switch to a compact binary format later.

`tokio` remains a reasonable future option, but the current Milestone 4 implementation intentionally uses a thread-per-client model. For 2 to 6 players this keeps the networking code easier to learn and debug.

### Why Rust Fits This Project

Rust is a strong fit because:

- Terminal input and rendering libraries are mature.
- Async networking is reliable.
- A single binary is easy to distribute.
- Strong types help keep game state, protocol messages, and item effects explicit.
- It can later support a server binary, client binary, or combined host-and-play binary.

The tradeoff is that Rust has a steeper learning curve than Python or JavaScript. For this project, that cost is probably worth paying because correctness and terminal control matter.

## Considered Stack Alternatives

These options were considered but are not the starting path.

### Python

Possible libraries:

- `asyncio` for networking.
- `textual` or `prompt_toolkit` for terminal UI.
- `websockets` or raw TCP for networking.

Pros:

- Fast to prototype.
- Easy to read and modify.
- Good for experimenting with game rules.

Cons:

- Terminal real-time rendering can become awkward.
- Packaging for other players can be less clean.
- Type and protocol mistakes are easier to introduce.

Python is reasonable if the main goal is learning and rapid iteration. It is less ideal if the goal is a polished multiplayer terminal game.

### TypeScript / Node.js

Possible libraries:

- `ink` or terminal-kit for UI.
- Node TCP sockets or WebSockets for networking.

Pros:

- Good async model.
- Familiar JSON-based networking.
- Easier later reuse if a web version is ever desired.

Cons:

- Terminal key handling and rendering can be less predictable across environments.
- Distribution requires a Node runtime or bundling.
- Terminal game loops can be more fragile than in Rust.

TypeScript is viable, especially if web play is likely later. For terminal-first gameplay, Rust is still the stronger default.

### Go

Possible libraries:

- `bubbletea` for terminal UI.
- Standard library networking.

Pros:

- Simple deployment.
- Good networking.
- Easier learning curve than Rust.

Cons:

- Real-time terminal rendering patterns may require more adaptation.
- Game-state modeling is less expressive than Rust.

Go is a solid fallback if Rust feels too heavy.

## Architecture Overview

Use a server-authoritative architecture.

```text
                 input events
Client A  ------------------------\
Client B  ------------------------- Server
Client C  ------------------------/

Client A  <------------------------ race snapshots
Client B  <------------------------ race snapshots
Client C  <------------------------ race snapshots
```

The server owns:

- Lobby state.
- Player list.
- Race phase.
- Track words.
- Bonus point choices and cooldowns.
- Player progress.
- Held items.
- Active effects.
- Attack warnings.
- Finish order.

Clients own:

- Local terminal input capture.
- Local rendering.
- A copy of the latest server snapshot.
- Optional local visual prediction for typed input.

For the first version, avoid complicated prediction. Send input events to the server and render from server snapshots. If typing feels laggy, add local prediction only for the local player's input buffer.

## Binary Layout

Start with one executable that supports subcommands:

```text
typekart host
typekart join 192.168.1.20:4000
```

Target architecture: the host process should run both:

- The authoritative game server.
- A local client connected to that server.

This matches the product decision that the host is always a player.

Current implementation: the host process is still the authoritative server and a player, but host input is read directly from the host terminal instead of going through a local client connection. This is a useful stepping stone, but we should still converge on a shared client path so host and joiner behavior do not drift.

Later, the code can expose separate server and client binaries if internet hosting needs it:

```text
typekart-server
typekart-client
```

## Network Transport

### First Version

Use TCP for local network play.

Why TCP:

- Reliable.
- Ordered.
- Simple to reason about.
- Good enough for a typing game prototype.
- Easier than designing reliability on top of UDP.

TCP can introduce head-of-line blocking, but TypeKart messages are small. For a first playable version, simplicity matters more.

### Later Options

If internet play or latency becomes more important:

- Keep TCP if it feels fine.
- Consider WebSockets for easier hosted or browser-adjacent infrastructure.
- Consider QUIC only if there is a clear need for lower-latency multiplexed transport.
- Avoid UDP until there is evidence TCP is the bottleneck.

## Protocol Shape

Use explicit client-to-server commands and server-to-client snapshots.

### Client To Server

Example message types:

```text
JoinLobby
SetReady
StartRace
KeyInput
ActivateItem
ActivateModifiedItem
LeaveRace
```

For typing, send key-level input events rather than full trusted progress updates. The server should be responsible for deciding whether the input advances the player.

Example key input payload:

```text
player_id
sequence_number
key
client_timestamp
```

### Server To Client

Example message types:

```text
LobbySnapshot
RaceSnapshot
RaceEvent
AttackWarning
RaceResults
Error
```

The server can broadcast snapshots at a fixed tick rate and send important events immediately.

Initial recommendation:

- Server tick rate: 20 ticks per second.
- Snapshot rate: 10 to 20 snapshots per second.
- Input events: sent immediately.

## Game Loop

The server loop should:

1. Accept input events from clients.
2. Apply valid input to authoritative player state.
3. Resolve bonus claims.
4. Resolve item activations.
5. Advance timers and cooldowns.
6. Resolve attack warnings.
7. Check finish conditions.
8. Broadcast updated state.

The client loop should:

1. Read terminal input.
2. Send input events to the server.
3. Receive snapshots and events.
4. Render the latest known state.

Keep game rules out of the terminal renderer. The renderer should display state, not decide state.

Current network prototype:

- The server accepts TCP clients and handles one reader thread per joiner.
- The server owns `RaceState`.
- `KeyInput` messages mutate server-owned player state.
- The server sends a `RaceSnapshot` immediately after accepted input.
- The join client uses raw terminal input during racing and sends character, Space, and Backspace events immediately.
- The join client prints snapshots instead of using the Ratatui race renderer.

This proves the protocol and authoritative input path. It is not the final client experience.

## Core State Model

Suggested Rust modules:

```text
src/
  main.rs
  app.rs
  protocol.rs
  server/
    mod.rs
    lobby.rs
    race.rs
    items.rs
  client/
    mod.rs
    input.rs
    render.rs
  game/
    mod.rs
    track.rs
    typing.rs
    bonus.rs
    player.rs
    effects.rs
```

Important data types:

```text
Race
PlayerState
Track
BonusPoint
BonusChoice
HeldItem
ActiveEffect
AttackWarning
RaceSnapshot
```

## Typing Engine

The typing engine should be deterministic and easy to test.

Inputs:

- Current player typing state.
- Current target word.
- Active effects.
- Key input.
- Bonus choices available at the player's position.

Outputs:

- Updated input buffer.
- Updated word progress.
- Typo state.
- Bonus claim attempt, if any.
- Finished-word event, if any.

Rules to support:

- Space is required between words.
- Early Space creates a typo.
- The final word finishes immediately when completed, without a trailing Space.
- First incorrect character creates a typo state.
- Progress is blocked while typo state is active.
- Backspace removes input and can clear typo state.
- Bonus intent is inferred from typed input.
- Bonus pickup is unavailable while holding an item.
- Bonus pickup is unavailable while typo state is active.
- Star Power can loosen typo rules without changing raw accuracy tracking.

This should be heavily unit tested before networking exists.

## Bonus Engine

The bonus engine should manage periodic bonus points.

Responsibilities:

- Generate bonus points along the track.
- Generate three active choices per bonus point.
- Track cooldown state per choice slot.
- Validate whether a player is in the bonus window.
- Validate whether a player can claim a bonus.
- Resolve simultaneous claims using server receive order.
- Force losing claim attempts onto the next main-track word.

This should also be unit tested independently.

## Item Engine

The item engine should treat items as direct player modifiers. Items are never persistent objects on the track.

Responsibilities:

- Roll items based on player position.
- Activate normal item behavior.
- Activate modified item behavior.
- Create attack warnings.
- Resolve warnings after the warning window.
- Apply active effects.
- Block effects with Shield.
- Expire timed effects.

Start with:

- Mushroom.
- Banana.
- Shield.

Then add:

- Star Power.
- Blue Shell.
- Better item weighting.

## Terminal Rendering

Use a retained state terminal UI rather than printing lines manually.

Recommended layout:

```text
Header: race phase, elapsed time, held item
Bonus layer: current bonus choices
Word layer: visible track words
Racer layer: three-cell colored racer markers
Player list: placement, name, progress
Event feed: recent important events
```

Rendering rules:

- Keep the word layer readable at all times.
- Use separate racer lanes aligned to the word layer.
- Use three-character colored racer markers.
- Render the local player lane immediately below the word layer.
- Render remote or AI racer lanes below the local player lane.
- Derive local racer marker placement from the current character, not only the current word.
- Pin the local racer marker to the first typo while typo recovery is required.
- Local player color wins on overlap.
- Remote overlaps blend colors when truecolor is available.
- Use a deterministic fallback when blending is unavailable.
- Highlight typed typo spans in red, including overflow across following words.
- Grey out bonus words when unavailable because the local player already holds an item.
- Show incoming attack warnings prominently.

## Terminal Key Handling

Key handling needs early validation.

Questions to test before committing controls:

- Can the terminal library distinguish `Enter` from `Shift+Enter`?
- Can it distinguish useful control combinations such as `Ctrl+J` and `Ctrl+K`?
- Do these combinations behave consistently across macOS Terminal, iTerm2, and common Linux terminals?
- Do any candidate keys interfere with normal typing, Backspace, Space, or Ctrl-C?

Recommended spike:

- Build a tiny key-inspector command.
- Print every key event and modifier detected by the terminal library.
- Test candidate item keys in the terminal environments we care about.

Until that spike is done, keep item activation configurable internally.

## Local Network Play

For the first version:

- Host opens a TCP listener on a configurable port.
- Joiners connect by IP address and port.
- Host also runs a local client.
- No lobby discovery.
- No NAT traversal.
- No internet relay.

This is enough for same-Wi-Fi testing and keeps the first networking milestone realistic.

## Internet Play Options

Internet play requires a separate decision after local play works.

Options:

| Option | Description | Pros | Cons |
| --- | --- | --- | --- |
| Direct port forwarding | Host opens a port on their router. | Simple code. | Bad user experience; often fails. |
| Relay server | Clients connect to a public relay that forwards traffic. | Works behind NAT. | Requires hosted infrastructure and added latency. |
| Hosted authoritative server | Public server owns game state. | Best fairness and reliability. | Most infrastructure and operations work. |
| WebSocket service | Hosted server or relay uses WebSockets. | Easy to deploy through common hosting. | Requires protocol adaptation. |

Initial recommendation:

- Build local network first.
- Design protocol messages so they can later travel over TCP, WebSockets, or a relay.
- Revisit internet play after the local prototype proves the game loop.

## Testing Strategy

Prioritize unit tests for pure game logic.

High-value tests:

- Typing correct words.
- Typing mistakes and backspacing.
- Early Space typo behavior.
- Star Power typo forgiveness.
- Bonus window availability.
- Bonus claim conflicts.
- Held-item bonus lockout.
- Mushroom three-word speedboost progression.
- Shield activation and expiry.
- Attack warning followed by block.
- Banana target selection.
- Blue Shell capitalization generation.
- Finish order.

Integration tests:

- One host and one joiner can enter a lobby.
- Countdown starts both players.
- Input from two clients updates server state.
- Race completes and results are broadcast.

Manual tests:

- Terminal rendering on narrow and wide terminals.
- Color readability.
- Overlapping racer markers.
- Key handling across terminals.
- Local network connection between two machines.

## Development Milestones

### Milestone 1: Local Typing Prototype

- Single-player terminal view.
- Generated word track.
- Typing engine.
- Typo red highlighting.
- Space-between-words behavior.
- Basic stats.

### Milestone 2: Race Renderer

- Visible track window.
- Separate racer layer.
- Player list.
- Event feed.
- Responsive terminal resizing.

### Milestone 3: Local Game Rules

- Bonus points.
- Three shared bonus choices.
- Held-item lockout with greyed-out bonus words.
- Mushroom.
- Banana.
- Shield.
- Attack warning UI.

### Milestone 4: Local Network Multiplayer

- Host command.
- Join command.
- Lobby.
- Readiness.
- Countdown.
- Server-authoritative input.
- Race snapshots.
- Finish order.

See `docs/milestone-4-plan.md` for the detailed implementation plan.

Current status: Milestone 4 has host/join, lobby readiness, countdown snapshots, server-owned race state, and server-authoritative raw key input for joiners during racing. Remaining major work is rendering snapshots through the real UI, fixed-rate snapshots, finish order, race end, bonuses, items, and network diagnostics.

### Milestone 5: Multiplayer Polish

- Better disconnect handling.
- Better event feed.
- Item weighting.
- Star Power.
- Blue Shell.
- Stats screen.
- More robust terminal controls.

### Milestone 6: Internet Play Decision

- Measure local network behavior.
- Decide whether to use relay, direct port forwarding, or hosted authoritative servers.
- Adjust protocol transport if needed.

## First Implementation Slice

The first code we should write is not networking. It should be the pure game logic.

Recommended first slice:

1. Define `Track`, `PlayerState`, and typing state.
2. Implement typing one word correctly.
3. Implement typo state and Backspace recovery.
4. Implement Space submission.
5. Add unit tests for the typing engine.

This creates a stable foundation. The UI and networking can then call into the same tested game rules instead of reimplementing them.

## Current Open Technical Questions

- How much local prediction is needed for typing feel?
- What snapshot rate feels smooth without overcomplicating networking?
- Should the host be refactored next into an in-process client, or should we first finish joiner raw input/rendering?
- Should disconnected racers remain visible through the whole race or disappear after a timeout?
- After local network play works, should internet play use direct port forwarding, a relay server, or hosted authoritative servers?

Item activation keys are lower priority while item pickup remains automatic. If manual activation returns, we should run the key-inspector spike described above before committing to `Enter`, `Shift+Enter`, or control-key combinations.
