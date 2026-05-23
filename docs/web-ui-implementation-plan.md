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

## Current Status

The web port can join CLI-hosted online rooms and can create browser-hosted
relay rooms with a playable host-authoritative race loop. It is still a parity
workstream rather than a production web release.

Implemented:

- Standalone Leptos CSR app in `web/typekart-web`.
- Separate web `Cargo.lock` so Leptos dependencies do not enter the terminal
  release/package dependency graph.
- Shared protocol crate in `crates/typekart-protocol`.
- Static renderer gallery backed by protocol-shaped lobby, race, and results
  fixtures.
- Browser `Join room` mode that connects directly to the existing WebSocket
  relay.
- Browser can join a CLI-hosted online room, receive welcome/lobby/race/result
  messages, and render live snapshots/deltas.
- Browser can send ready/unready/start and key input messages through the relay.
- Browser typing works against a CLI host.
- Browser renderer anchors to the browser player's game id while using the
  relay participant id for outbound routing.
- Browser race lanes render the local browser player first.
- Track text, bonus words, racer lanes, markers, minimap, and event feed are
  aligned in a terminal-inspired layout.
- Browser can create a relay room, manage lobby racers, start countdowns, run
  browser-hosted races, process typing and AI progress, and publish results.

Known limitations:

- Browser-hosted games are playable, but still need structured manual item and
  cross-client validation.
- Browser word-pack and item-pack selection is not implemented.
- Browser renderer is close enough for development, but not yet full parity with
  the terminal renderer.
- Browser focus handling captures gameplay keys during active races, but still
  needs broader browser smoke testing.
- Reconnect, host-left, and game-closed UX are still minimal.
- No browser screenshots or cross-browser smoke tests are automated yet.

Recent manual validation:

- Browser can join a CLI-hosted room through a relay.
- Browser can ready up.
- Browser can see the race.
- Browser can type during the race.
- Browser input updates the authoritative race state and appears correctly in
  the terminal UI.
- Browser perspective now tracks the browser player's typed text and lane order.
- Browser-hosted rooms can run countdowns, accept browser and terminal joiners,
  advance racers, and produce results.

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

Initial slice:

- `docs/network-protocol.md` documents relay envelopes, game messages, browser
  host flow, and compatibility expectations.
- Protocol tests include fixture-style assertions for browser-required JSON
  shapes so accidental wire-format drift is caught in Rust tests.

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

Initial slice:

- `web/typekart-web` is a standalone Leptos CSR app that can be served with
  Trunk.
- The web app intentionally keeps its own `Cargo.lock` so Leptos dependencies
  do not enter the terminal app's release/package dependency graph.
- The web shell renders gallery, join, and create-room flows.
- Development docs include the WASM target, Trunk, serve, and check commands.

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

Initial slice:

- The web app now renders protocol-shaped fixture frames instead of a one-off
  static track:
  - lobby frames backed by shared `LobbyPlayer`/`ModConfigSnapshot` types
  - `RaceSnapshot`
  - results frames backed by shared `RaceResultRow` types
- `crates/typekart-protocol` contains the shared JSON wire contract used by the
  terminal app and browser app.
- Gallery scenarios cover lobby, countdown, Banana, Mushroom, Shield, Focus,
  Cyclone, Squid Ink, finish sprint, and results states.
- The browser renderer includes track words, stacked bonus choices, racer
  lanes, Unicode/ASCII cue toggle, minimap, and event feed.
- Fixture smoke tests assert that major states are represented, consumed bonus
  choices render, all current item cue/impact concepts are covered, Squid Ink
  masks only future words, results include timed-out racers, and finished
  players pin to the minimap finish edge.

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

Implemented:

- Shared relay envelope types (`RoomCode`, `RelayClientMessage`,
  `RelayServerMessage`) are now part of the shared protocol contract.
- The web app has a `Join room` mode with relay URL, room code, and display name
  inputs.
- Browser join mode opens a WebSocket relay connection, sends `join_room`,
  decodes host direct/broadcast payloads as `ServerMessage`, and renders lobby,
  full race snapshots, race deltas after a full snapshot, and race results.
- Gallery mode remains available for static renderer work.
- The live browser renderer uses a fixed track window, stacked bonus choices,
  per-racer lanes, local-player perspective, minimap, and event feed.

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

Implemented:

- Browser join mode now keeps a WebSocket write path open after joining a relay
  room.
- Ready, unready, and start controls send the same `ClientMessage` variants as
  the terminal client, wrapped in `RelayClientMessage::ClientToHost`.
- The live race panel is focusable and maps browser keydown events for letters,
  Space, and Backspace to protocol key input messages.
- Browser key input uses monotonically increasing client sequence numbers.
- Browser stores separate relay and game player ids:
  - relay participant id for outbound `ClientToHost` routing
  - game player id from `Welcome` for local rendering perspective
- Browser race lanes render the local player first.

Tasks:

- Complete lobby commands:
  - leave
  - rename, if the web flow keeps editable names after join
  - cancel/rematch once browser hosting exists
- Improve focus management so normal browser shortcuts do not interfere with
  race typing more than necessary.
- Verify typing semantics against the terminal client:
  - spaces are explicit input
  - backspace fixes typo state
  - bonus word typing works at the correct gap
  - input pauses during relevant effects
- Add browser-side affordances for race focus and connection state.

Deliverable:

- Browser joiner can race against a CLI host.

Validation:

- Manual race with CLI host and browser joiner passes.
- Browser input affects race state and is visible in the terminal host UI.
- Still needed: item/effect validation for browser players.
- Still needed: bonus word validation from the browser.

## Immediate Next Steps

The next practical work should finish browser joiner parity before browser
hosting. That keeps the feedback loop small because the CLI host remains the
authoritative reference implementation.

Recommended order:

1. Add a browser manual validation checklist covering join, ready, countdown,
   typing, spaces, backspace, typo recovery, bonus words, and each item effect.
2. Polish browser joiner UX:
   - clearer connected/player status
   - explicit "click race to type" state
   - phase-aware controls
   - leave/disconnect action
3. Bring the live browser renderer closer to terminal parity:
   - better countdown display
   - better impact blink/effect styling
   - results and lobby layout polish
   - responsive desktop-width constraints
4. Validate item effects against browser players:
   - Mushroom input pause and boost indicator
   - Shield/Focus marker overlays
   - Banana/Cyclone/Squid Ink impacts
   - bonus word cooldown/availability rendering
5. Decide whether browser joiners should remain compatible with CLI hosts long
   term or whether this is only a migration/testing bridge.

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

## Recommended Next Slice

Continue from the current browser-hosted race loop. The next useful work is
manual browser-hosted validation, renderer parity, mod selection, reconnect UX,
and deployment preparation.
