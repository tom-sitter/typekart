# Web UI Implementation Plan

This plan adds a fully browser-based TypeKart experience while preserving the
current host-authoritative game model. The browser host will run the same Rust
game engine compiled to WebAssembly, and browser joiners will connect through
the existing WebSocket relay.

## Goals

- Let a player create and host an online game entirely from the browser.
- Let other players join and race entirely from the browser.
- Reuse the existing Rust game rules, item behavior, AI behavior, mod
  validation, and race protocol as much as practical.
- Keep the relay as a routing layer, not an authoritative game server.
- Preserve the terminal app as a supported client.
- Leave a path open for future server-authoritative hosted rooms.

## Non-Goals

- Public matchmaking.
- Accounts, ranking, or anti-cheat.
- Host migration when the browser host closes.
- Mobile-first gameplay. The layout should be responsive, but keyboard play is
  the first target.
- Rewriting the game engine in TypeScript.

## Target Architecture

```text
host browser
  Leptos UI
  Rust/WASM host game loop
  shared protocol messages
        |
        | WebSocket
        v
TypeKart relay
  room creation
  room routing
  opaque message forwarding
        |
        | WebSocket
        v
joiner browsers
  Leptos UI
  shared protocol messages
  render snapshots/deltas
  send input/lobby commands
```

The browser host is authoritative for the room. It owns track generation, bonus
state, item rolls, AI racers, race phase, results, and all snapshots/deltas.
Joiners send commands and render the host's state.

This is appropriate for casual games with friends. It is not cheat-resistant:
the host browser can be modified by the person running it. If competitive public
rooms become a goal, add a server-authoritative host process later.

## Technology Choices

- Rust/WASM for shared game logic.
- Leptos for browser UI.
- WebSocket transport through the existing relay.
- DOM/CSS Grid renderer first; avoid canvas until the UI needs animation that
  the DOM cannot handle cleanly.
- TypeScript bindings generated from Rust protocol types once the protocol
  boundary is stable.

WebRTC is not the first transport target. It adds signaling, STUN/TURN, and
reconnection complexity while providing little benefit for this text racing
game. The relay-backed WebSocket path is simpler and matches the current online
architecture.

## Milestone 1: Core Boundary

Goal: make the reusable game engine explicit.

Initial slice:

- `src/lib.rs` exposes the reusable application modules as a library surface.
- `game::engine::CoreHost` provides a terminal-free and network-free host tick
  API that can apply queued commands to `RaceState`.
- The first core tests drive a race to completion without Ratatui, Crossterm,
  TCP, or relay code.

Tasks:

- Create a `typekart-core` library boundary inside the repo or as a workspace
  crate.
- Move pure game logic behind that boundary:
  - track and word-set selection
  - typing rules
  - player state
  - bonus words
  - item registry and effects
  - AI typing and item behavior
  - race status/results
- Keep terminal-only concerns outside the core:
  - Ratatui rendering
  - Crossterm input
  - native TCP listeners/streams
  - filesystem-only loading APIs
- Add a browser-safe host tick API that can advance the authoritative race from
  elapsed time plus queued client commands.
- Keep existing terminal tests passing.

Deliverable:

- A Rust core API that can be called from the terminal app and later from WASM.

Validation:

- Existing unit tests pass.
- A small test can drive a race from commands without terminal or TCP code.

## Milestone 2: Protocol Contract

Goal: turn the current network protocol into a browser-friendly contract.

Tasks:

- Audit `ClientMessage`, `ServerMessage`, `RaceSnapshot`, `RaceDelta`,
  `LobbySnapshot`, and relay envelopes.
- Remove or isolate terminal-specific assumptions from protocol payloads.
- Decide which messages browser hosts send to joiners and which messages
  joiners send to hosts.
- Add protocol documentation for:
  - room creation
  - join flow
  - lobby commands
  - race input
  - snapshots and deltas
  - results
  - disconnects
  - version/mod mismatch
- Add TypeScript type generation or a checked hand-written TypeScript contract.

Deliverable:

- Documented JSON protocol that browser code can consume safely.

Validation:

- Round-trip tests for all browser-required messages.
- TypeScript compile check against protocol fixtures.

## Milestone 3: Web Workspace Scaffold

Goal: add a minimal browser app without changing gameplay yet.

Tasks:

- Add a web app package/workspace using Leptos.
- Add build/dev commands to `docs/development.md`.
- Add a simple browser shell:
  - home screen
  - create room button placeholder
  - join room form placeholder
  - renderer gallery route placeholder
- Configure WASM build tooling.
- Add basic CI checks for the web app once the toolchain is stable.

Deliverable:

- `typekart-web` runs locally and renders a static shell.

Validation:

- Browser dev server starts.
- WASM bundle builds.
- Existing CLI build still passes.

## Milestone 4: Web Renderer Gallery

Goal: build the browser race renderer before wiring live multiplayer.

Tasks:

- Convert existing gallery scenarios into protocol-level fixture snapshots.
- Render:
  - track words
  - local racer lane
  - other racer lanes
  - bonus words, including unavailable/cooldown choices
  - item cues
  - impact cues
  - minimap
  - events
  - results
- Support Unicode and ASCII display modes.
- Add browser screenshots or visual smoke tests for representative scenarios.

Deliverable:

- Browser gallery can preview the same important race states as the terminal
  gallery.

Validation:

- Static fixtures render without overlap on desktop widths.
- Key item states have visible cues: Banana, Mushroom, Shield, Focus, Cyclone,
  and Squid Ink.

## Milestone 5: Browser Joiner Read-Only Mode

Goal: connect a browser to a real relay room as an observer/joiner renderer.

Tasks:

- Implement browser WebSocket relay connection.
- Join an existing room through the relay.
- Receive lobby snapshots, race snapshots, and deltas.
- Render live lobby and race state.
- Handle game-closed and disconnect messages.
- Add basic reconnect/error messaging.

Deliverable:

- Browser can observe a CLI-hosted online game through the existing relay.

Validation:

- CLI host + browser observer works locally against a local relay.
- CLI host + browser observer works against the public relay.

## Milestone 6: Browser Joiner Play

Goal: make browser joiners playable clients.

Tasks:

- Map browser keyboard events to existing race input messages.
- Implement lobby commands:
  - ready
  - unready, if still supported
  - rename
  - leave
- Preserve typing semantics:
  - spaces are explicit input
  - backspace fixes typo state
  - bonus word typing works at the correct gap
  - input pauses during relevant effects
- Add focus management so normal browser shortcuts do not interfere with race
  typing more than necessary.

Deliverable:

- Browser joiner can race against a CLI host.

Validation:

- Manual race with CLI host and browser joiner.
- Item effects from the CLI host affect the browser player correctly.
- Browser input affects race state identically to terminal input.

## Milestone 7: Browser Host Core Loop

Goal: run the authoritative host in the browser.

Tasks:

- Compile the core host game loop to WASM.
- Implement browser host state:
  - lobby roster
  - ready state
  - AI management
  - countdown
  - race tick
  - snapshot/delta emission
  - results
- Implement browser host handling for joiner commands.
- Send host broadcasts through the relay using the existing room model.
- Keep host-side AI behavior identical to CLI host behavior.

Deliverable:

- Browser can create a room and host a race for browser joiners.

Validation:

- Browser host + browser joiner race works against local relay.
- Browser host + CLI joiner compatibility is evaluated and either supported or
  explicitly blocked.

## Milestone 8: Browser Lobby Management

Goal: bring browser host controls up to parity with the terminal lobby.

Tasks:

- Add host controls:
  - add/remove AI
  - set AI difficulty
  - kick human joiner
  - start race
  - cancel active race
  - return to lobby after results
- Show mod/word/item pack metadata.
- Show room code and join link.
- Support copy-to-clipboard for room links.

Deliverable:

- Browser host can manage a full casual room without using the CLI.

Validation:

- Host can run repeated browser-hosted races with changing lobby rosters.

## Milestone 9: Browser Mods

Goal: make browser-hosted games configurable without filesystem assumptions.

Tasks:

- Support built-in word sets and item packs in the web bundle.
- Let a browser host upload local word-set and item-pack files.
- Validate uploaded files with shared Rust logic.
- Include mod hashes in lobby/race snapshots.
- Ensure joiners see the host's selected mod metadata.
- Decide whether browser joiners need the full mod files or only snapshots from
  the host.

Deliverable:

- Browser host can start a game with built-in or uploaded mods.

Validation:

- Valid mods load and appear in the lobby.
- Invalid mods show actionable validation errors.
- Joiners cannot enter a race with incompatible protocol/core versions.

## Milestone 10: Deployment

Goal: publish a usable web app.

Tasks:

- Choose hosting target for static web assets.
- Configure production build.
- Add cache/version strategy so clients refresh when protocol changes.
- Add relay URL configuration.
- Add basic privacy/security headers.
- Document deployment and rollback.

Deliverable:

- Public web UI URL that can create and join browser-hosted races.

Validation:

- Production web app can host and join through the public relay.
- CLI online play continues to work.

## Milestone 11: Hardening And Polish

Goal: make the browser experience credible for early public use.

Tasks:

- Improve reconnect handling.
- Add clear host-left/game-closed UX.
- Add latency/connection status indicators.
- Add keyboard shortcut help.
- Add accessibility pass for color and focus states.
- Add mobile/tablet unsupported or limited-support messaging if needed.
- Extend relay load tests for browser-style traffic.
- Add browser renderer regression tests.

Deliverable:

- Web UI is stable enough to link from the README as an early-access path.

Validation:

- Manual browser validation checklist.
- Relay load test updated for browser-hosted room patterns.
- Cross-browser smoke test in current Chrome, Firefox, Safari, and Edge.

## Open Questions

- Should CLI clients be allowed to join browser-hosted rooms in the first web
  release?
- Should browser clients be allowed to join CLI-hosted rooms long term, or only
  during the migration?
- How much host trust do we want to communicate in the UI?
- Do uploaded mods need to be shared with joiners, or are host-generated
  snapshots sufficient for the first browser release?
- Should the public relay impose different rate limits for browser-hosted rooms?
- Do we need host pause/resume behavior when the host tab is backgrounded?

## Recommended First Slice

Start with Milestone 1 and Milestone 2. The web UI will be much easier if the
core game loop and protocol boundary are clean before adding Leptos. The first
visible browser work should be the renderer gallery, because it can move quickly
without depending on live networking.
