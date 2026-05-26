# Shared Logic Refactor Plan

This plan tracks the remaining work to keep terminal, network, and browser
interfaces backed by the same game rules. The goal is not to make every UI share
the same renderer or event loop. The goal is to keep authoritative gameplay,
race lifecycle, item behavior, snapshots, and validation in browser-compatible
Rust modules, with terminal and web layers acting as adapters.

## Current Shared Foundation

These pieces already live in shared Rust code and are used by more than one
interface:

- `src/game/typing.rs`: deterministic character input, typo handling, word
  completion, and final-word finish behavior.
- `src/game/race.rs`: `RaceState`, `RaceParticipant`, result rows,
  `RaceLifecycleState`, and `RaceRuntimeState`.
- `src/game/bonus.rs`: bonus placement, choice availability, cooldowns, and
  claim resolution.
- `src/game/item_effects.rs`: shared item activation and per-player effect
  mutation for network and browser-hosted races.
- `src/game/items.rs`: item registry, weights, targeting helpers, and moddable
  item tuning.
- `src/game/track.rs` and `src/game/words.rs`: shared track generation and word
  set loading/validation.
- `crates/typekart-protocol`: relay and game message contract used by terminal
  and web clients.

## Design Rules

- Shared code must stay browser-compatible. Avoid filesystem, terminal,
  threads, sockets, and blocking sleeps inside shared gameplay modules.
- UI layers should translate input into shared commands and render shared
  snapshots or view models. They should not decide race rules.
- Terminal-only concerns stay in `src/ui` and `src/net/client.rs`.
- Native networking and relay infrastructure stay in `src/net`.
- Browser DOM, Leptos signals, WebSocket wrappers, and CSS stay in
  `web/typekart-web`.
- Shared APIs should accept game ids such as `RacePlayerId` at the core and
  convert protocol/UI ids at the boundary.

## Refactor Slices

### 1. Browser Web Module Split

Goal: make the browser code easier to reason about before deeper extraction.

Likely changes:

- Move browser connection/session loop code from `web/typekart-web/src/main.rs`
  into `session.rs`.
- Move browser join-client relay code into `client.rs`.
- Move browser host lobby/race authority code into `host.rs`.
- Move renderer helpers and Leptos components into `render.rs` or
  `components.rs`.
- Move browser setup/session UI into `setup.rs`, leaving `main.rs` as the app
  shell.
- Keep fixtures in `fixtures.rs`.

Validation:

- `cargo clippy --manifest-path web/typekart-web/Cargo.toml --locked --all-targets -- -D warnings`
- `cargo test --manifest-path web/typekart-web/Cargo.toml --locked`
- `cargo check --manifest-path web/typekart-web/Cargo.toml --locked --target wasm32-unknown-unknown`

Notes:

- This should be mostly movement, not behavior changes.
- Do this before adding more browser parity work.
- Progress: browser session/control primitives have been extracted to
  `web/typekart-web/src/session.rs`, browser join-client relay code has been
  extracted to `web/typekart-web/src/client.rs`, and browser host authority has
  been moved to `web/typekart-web/src/host.rs`. Host, client, and render tests
  now live beside their modules. Renderer components and layout helpers have
  been extracted to `web/typekart-web/src/render.rs`. Browser setup/session UI
  has been extracted to `web/typekart-web/src/setup.rs`, and session tests now
  live beside `web/typekart-web/src/session.rs`.

### 2. Shared Race Rule Modules And Host Facade

Goal: extract duplicated authoritative race rules into small browser-safe game
modules first, then grow `src/game/engine.rs` into a thin coordinator facade.
`engine.rs` should not become a large bucket of unrelated rules.

Intermediate module targets:

- `src/game/ai_driver.rs`: AI WPM adjustment, character budgets, next-key
  selection, pause checks, and AI typing events.
- `src/game/race_flow.rs`: lifecycle update helpers, finish timeout handling,
  result readiness, and runtime reset/rematch helpers.
- `src/game/input_rules.rs`: shared input pause policy, including stun and
  mushroom lockout rules.
- Keep future `bonus_flow.rs`, `snapshot.rs`, and `lobby.rs` as later slices
  unless the current extraction needs them.

Responsibilities to move behind the final facade:

- Apply human key input.
- Advance AI racers from WPM budget.
- Advance mushroom boosts.
- Update race lifecycle and result readiness.
- Reset runtime state for a new race.
- Emit structured host events for UI/logging layers.

Keep outside the facade:

- Terminal drawing.
- Browser DOM updates.
- TCP/WebSocket transport.
- Relay room management.
- Wall-clock scheduling primitives. Callers should pass `Instant`/elapsed time.

Implementation sequence:

1. Extract `ai_driver.rs` and use it from browser-hosted races first.
2. Wire the same AI driver into the terminal network host.
3. Extract `race_flow.rs` for lifecycle/result-ready behavior.
4. Keep `engine.rs` as the orchestration layer that calls these rule modules.

Progress:

- `src/game/ai_driver.rs` now owns shared AI WPM adjustment, character budgets,
  next-key selection, and AI typing advancement.
- `src/game/input_rules.rs` now owns shared input pause checks for stun and
  mushroom lockout.
- `src/game/race_flow.rs` now wraps shared lifecycle update and runtime reset
  helpers.
- Browser-hosted races and terminal network-hosted races both use the shared AI
  driver path.
- Local terminal AI racers use the shared AI WPM math and next-key selection.
- Browser and network race status updates now call `race_flow`.
- `src/game/engine.rs` remains a thin coordinator and delegates AI/lifecycle
  helpers instead of absorbing the rule implementations.

Validation:

- Existing network host tests should still pass while using the facade.
- Browser-hosted tests should use the same facade.
- Add focused rule-module tests that do not instantiate terminal or browser
  code.

### 3. Shared Bonus Attempt Flow

Goal: eliminate duplicated bonus typing state machines between
`src/net/server.rs` and `web/typekart-web/src/main.rs`.

Shared behavior:

- Determine when a player can start a bonus attempt.
- Infer bonus intent from the first typed character.
- Apply bonus attempt typing and backspace bailout.
- Force a player back to the main track when they lose a contested bonus.
- Require Space after completing a bonus word.
- Record spent bonus gap state.

Suggested API shape:

- `src/game/bonus_flow.rs` owns `BonusAttempt`, bonus key handling, claim
  resolution, spent-gap recording, and semantic bonus-flow events.
- Hosts pass `RaceState`, `BonusState`, runtime attempt/spent maps, player ids,
  key action, item roll context, item registry, current time, and RNG.
- Hosts remain responsible for protocol-specific feed messages, debug logs,
  snapshot broadcasting, and activating any item returned by the shared claim
  outcome.

Progress:

- Terminal network-hosted races and browser-hosted races now use
  `apply_bonus_key` for human bonus typing.
- Network AI bonus claims now use the shared claim helper after choosing an
  available bonus choice.
- Existing browser/network bonus tests remain at the adapter layer, with new
  focused rule tests in `src/game/bonus_flow.rs`.

Validation:

- Keep expanding focused shared tests as future contested/edge cases are found.
- Keep protocol-specific events/log strings at the adapter layer.

### 4. Shared Snapshot Builder

Goal: build protocol-shaped race snapshots from shared race state in one place.

Current issue:

- Terminal host and browser host both convert `RaceState`, bonus state, runtime
  item effects, and mod metadata into snapshots.
- Small divergence here can make the browser and terminal render different
  statuses for the same authoritative state.

Suggested extraction:

- `src/game/snapshot.rs` builds protocol-shaped race, delta, player, bonus,
  item cue, and impact cue snapshots from shared race state.
- Inputs are shared state plus adapter-provided phase, sequence, mod config,
  events, and player-kind mapping.
- Terminal and browser hosts still own when snapshots are emitted and how they
  are broadcast.

Progress:

- Terminal networking now uses the `typekart-protocol` crate directly instead
  of a separate local protocol module copy.
- Terminal-hosted and browser-hosted race snapshots now use the shared snapshot
  builder for core race, player, bonus, and effect fields.
- Relay room-code generation moved to a free helper because `RoomCode` now comes
  from the external protocol crate.

Validation:

- Existing protocol fixture tests.
- Existing network snapshot tests.
- Browser host snapshot tests.

### 5. Shared Lobby Policy

Goal: centralize lobby rules while keeping transport-specific state separate.

Shared rules:

- Unique player names.
- Host readiness defaults.
- Ready/unready behavior by phase.
- AI add/remove/tune limits.
- Kick rules.
- Which players are selected for the next race.

Keep outside shared policy:

- TCP client streams.
- Relay participant mapping.
- Browser signal updates.

Suggested API shape:

- Start with pure helpers in `src/game/lobby.rs` for name dedupe, player id
  assignment, color assignment, host-ready defaults, AI lobby player creation,
  and race participant selection.
- Later slices can introduce a fuller `LobbyState` command/event layer once the
  remaining host-specific roster mutations are smaller.

Progress:

- `src/game/lobby.rs` now owns shared lobby helper policy for terminal and
  browser hosts.
- Terminal-hosted and browser-hosted games use the same helpers for human
  lobby players, AI lobby players, unique names, player ids, color selection,
  and race participant conversion.
- Terminal-hosted and browser-hosted games now share lobby command policy for
  ready/unready, rename, add AI, remove/kick player, AI difficulty changes, and
  phase/roster validation.
- Transport-specific behavior remains in the adapters: TCP stream cleanup,
  relay participant mapping, browser signals, kick side effects, and event/feed
  wording.
- A fuller `LobbyState` command/event owner is intentionally deferred until a
  future cleanup needs to remove more host orchestration from the adapters.

Validation:

- Shared lobby helper tests cover name suffixes, player-id allocation, and
  ready participant selection.
- Shared lobby command-policy tests cover rename dedupe, roster limits, host
  removal rejection, readiness, and AI difficulty updates.
- Keep protocol broadcast tests in `src/net/server.rs`.

### 6. Shared Results And Rematch Flow

Goal: make finishing, results, and rematch readiness consistent for CLI-hosted
and browser-hosted games.

Shared behavior:

- First-place timeout.
- All-connected-finished completion.
- All-disconnected completion.
- Placement synthesis for unfinished racers.
- Result row construction.
- Runtime reset before rematch.

Most of this is already partially shared through `RaceLifecycleState`,
`RaceRuntimeState`, and result row helpers. The remaining work is to route both
hosts through one facade method rather than calling helpers manually.

Progress:

- `src/game/race_flow.rs` now returns semantic race-finish outcomes with
  newly finished player names/placements and finish summaries.
- `src/game/snapshot.rs` now owns protocol result-row and placement conversion.
- Terminal-hosted and browser-hosted games both use the shared finish outcome
  and protocol result conversion helpers.
- Rematch/runtime reset remains adapter-owned where track generation, lobby
  cleanup, browser signals, and transport broadcasts are still adapter-specific.

Validation:

- Shared race-flow tests cover finish outcome construction.
- Shared snapshot tests cover protocol result-row conversion.
- Existing terminal and browser host result/rematch tests should continue to
  cover adapter side effects.

### 7. Renderer View Model

Goal: share layout-relevant race display calculations without forcing terminal
and browser renderers to share the same UI implementation.

Candidates:

- Track window selection.
- Current-character marker position.
- Bonus column placement.
- Minimap positions.
- Player ordering from local perspective.
- Item/effect display tokens.

Keep separate:

- Ratatui widgets.
- HTML/CSS class names.
- Terminal-width emoji fallback behavior if browser layout does not need it.

Validation:

- Existing terminal renderer tests should move toward shared view-model tests.
- Browser renderer tests should assert against the same view-model output.

### 8. Local Session Shared Engine Migration

Goal: move single-player/local terminal rules onto the same shared race engine
used by network and browser hosts.

Current issue:

- `src/ui/session.rs` still owns bespoke local versions of several gameplay
  rules that now exist in shared modules.
- This makes local play the most likely place for old bugs to reappear after
  network/browser fixes.

Migration targets:

- Race lifecycle and timeout policy.
- AI driver state and WPM advancement.
- Bonus attempt state and claim resolution.
- Item effect mutation and impact cues.
- Result row construction.

Progress:

- Local race-end status now builds a temporary shared `RaceState` and advances
  lifecycle through `src/game/race_flow.rs`, so local play uses the same
  all-finished and post-first-finish timeout policy as network/browser races.
- Local countdown start, countdown-to-racing transition, race-finish event, and
  restart return/cancel intent now use shared `src/game/host_session.rs`
  outcomes while preserving local-only track regeneration and terminal UI
  behavior.
- Local terminal session, gallery, and renderer now carry shared
  `NetworkRacePhase` directly instead of a local-only `RacePhase` enum. Local
  countdown timing keeps a private deadline, but display and lifecycle decisions
  use the shared phase shape.
- Local session creation and restart now prepare local human/AI participants
  through `prepare_race_from_participants`, so local setup uses shared
  `RaceState` and `BonusState` construction while retaining local-only AI WPM
  and terminal session metadata.
- Local item activation now uses shared `game::item_effects` for all current
  items: banana, cyclone, squid ink, mushroom, shield, and focus. The old
  local-only banana warning, banana targeting, squid-ink mutation, mushroom
  stepping, shield activation, and focus activation paths were removed, so
  local and multiplayer item behavior are closer to parity.
- Local human bonus typing now uses shared `game::bonus_flow` for starting,
  editing, cancelling, and claiming bonus attempts. Local AI bonus pickup claim
  selection and resolution also use a shared random-available claim helper,
  matching the network AI path.

Validation:

- Local session finish/timeout tests.
- Full `ui::session` test module.

### 9. Network Host Module Split

Goal: shrink `src/net/server.rs` without moving rules back out of shared code.

Candidate extractions:

- `src/net/host_lifecycle.rs`: host-only broadcast sequencing around shared
  race-flow outcomes and result snapshots.
- `src/net/server/host_lobby.rs`: TCP-specific lobby side effects around
  shared lobby policy, including client kicks, rename side effects, and waiting
  roster cleanup.
- `src/net/server/host_snapshots.rs`: network-specific snapshot sequencing,
  bonus cooldown refresh, player-kind mapping, and snapshot broadcast logging
  around shared snapshot projection.
- `src/net/server/host_broadcast.rs`: network-specific client fanout for lobby
  snapshots, race snapshots/deltas, and one-shot race results.
- `src/net/server/host_disconnect.rs`: network-specific disconnect cleanup,
  host-left closure, and handoff into phase reconciliation.
- `src/net/server/host_phase.rs`: network-specific countdown, rematch/cancel
  phase changes, periodic race ticking, and race-finish broadcast sequencing.
- `src/net/server/host_commands.rs`: embedded terminal-host command loop,
  host ready/unready commands, and pasted line input handoff.
- `src/net/server/host_client.rs`: per-client TCP message loop and dispatch
  into lobby, phase, input, AI, and disconnect adapters.
- `src/net/server/host_join.rs`: post-handshake join admission policy,
  including version checks, capacity checks, names, colors, welcome responses,
  and lobby/race roster mutation.
- `src/net/server/host_util.rs`: small host utilities such as capacity
  validation, lobby console printing, and protocol/UI color conversions.
- `src/net/server/host_state.rs`: central network-host state container and
  initial state construction from `HostConfig`.
- `src/net/server/host_accept.rs`: TCP accept-loop shell after startup,
  including handshakes, join admission, accepted-client logging, and per-client
  thread spawning.
- `src/net/server/host_race.rs`: network-specific race lifecycle state
  mutations around shared `race_flow`, including rematch preparation and finish
  status updates.
- `src/net/server/host_handshake.rs`: TCP join handshake validation and welcome
  message handling.
- `src/net/server/host_input.rs`: TCP/protocol input translation and
  network-host race input outcomes around shared typing and bonus rules.
- `src/net/server/host_ai.rs`: network-specific AI timing map and debug
  logging around shared `ai_driver` and shared lobby AI policy.
- `src/net/server/host_bonus.rs`: network-specific event/log handling around
  shared `bonus_flow`.

Constraint:

- These modules should not own gameplay policy. They should orchestrate
  transport, logging, and adapter state around shared `src/game` helpers.

Progress:

- `src/net/host_lifecycle.rs` now owns network-host result message construction
  and race-finish feed/log formatting around shared `race_flow` and `snapshot`
  helpers.
- `src/net/server.rs` still owns socket writes, phase mutation, and
  `HostState`, but no longer constructs race-result protocol messages inline.
- `src/net/server/host_bonus.rs` now owns network-host bonus adapter flow around
  shared `bonus_flow`: human bonus input, AI bonus claims, bonus feed/log text,
  item roll context, and handoff to network item activation.
- `src/net/server/host_items.rs` now owns network-host item side effects around
  shared `item_effects`: item activation reports, interrupted bonus cleanup,
  AI typing budget resets, mushroom advancement, and item event logging.
- `src/net/server/host_ai.rs` now owns network-host AI roster and race ticking
  around shared `lobby` and `ai_driver`: initial bot creation, lobby add/remove
  difficulty tuning, countdown timing resets, bonus-claim attempts, typing
  budget advancement, and AI finish events.
- `src/net/server/host_lobby.rs` now owns network-host lobby side effects
  around shared `lobby`: player removal, rename propagation into the mirrored
  race roster, kicked-client socket closure, runtime cleanup for removed
  players, and disconnected waiting-roster cleanup.
- `src/net/server/host_snapshots.rs` now owns network-host snapshot adaptation
  around shared `snapshot`: sequence increments, cooldown expiry before
  projection, player-kind lookup, and snapshot/delta broadcast log messages.
- `src/net/server/host_broadcast.rs` now owns network-host client fanout:
  building lobby/race/result protocol messages, writing them to connected TCP
  clients, pruning failed client streams, and preserving one-shot result
  broadcast semantics.
- `src/net/server/host_disconnect.rs` now owns network-host disconnect
  handling: lobby/race disconnection flags, transient runtime cleanup, host-left
  game closure, client stream shutdown, and waiting-roster cleanup.
- `src/net/server/host_phase.rs` now owns network-host phase progression:
  preparing countdowns/rematches, returning to lobby, countdown ticks, active
  race snapshot ticks, item/AI periodic advancement, and phase reconciliation
  after disconnects.
- `src/net/server/host_commands.rs` now owns embedded terminal-host commands:
  ready/unready, lobby printing, race start, pasted host typing, and command
  feedback while delegating gameplay rules to shared adapters.
- `src/net/server/host_client.rs` now owns per-client message handling:
  reading framed client messages, dispatching lobby commands, start/rematch
  requests, AI controls, key input, leave handling, and post-loop disconnect
  broadcasts.
- `src/net/server/host_join.rs` now owns join admission after a valid handshake:
  version mismatch rejection, lobby-full/no-color rejection, unique name and id
  assignment, welcome delivery, client writer registration, lobby/race roster
  updates, and join feed/log events.
- `src/net/server/host_util.rs` now owns small host utility concerns: capacity
  validation, lobby console output, and `AssignedColor`/`PlayerColorId`
  conversions.
- `src/net/server/host_state.rs` now owns `HostState`, `ConnectedClient`, and
  initial host-state construction, including embedded host setup, AI racer
  creation, bonus generation, runtime initialization, and next-player-id setup.
- `src/net/server/host_accept.rs` now owns the post-startup TCP accept loop:
  accepting streams, reading handshakes, delegating join admission, broadcasting
  lobby updates, logging accepted joiners, and spawning per-client handlers.
- `src/net/server/host_race.rs` now owns network-host race lifecycle mutation
  around shared `race_flow`: connected-racer counts, rematch/race reset from
  lobby participants, runtime reset, race-finish status updates, and
  finish-summary event/log messages.
- `src/net/server/host_handshake.rs` now owns the TCP join handshake: requiring
  `Hello` as the first client message, validating non-empty names and versions,
  and sending `Welcome` to accepted joiners.
- `src/net/server/host_input.rs` now owns network-host input adaptation:
  translating protocol keys and line input into shared `KeyAction`s, invoking
  shared bonus/typing flow, applying race-status updates, and reporting whether
  the server should broadcast a delta or final results.

### 10. Shared Host Session Core

Goal: make terminal single-player, LAN-hosted, and browser-hosted races adapters
over one authoritative session core instead of three independently orchestrated
game loops.

Current issue:

- Network-hosted races now use many shared rule modules, but the network host,
  browser host, and local session still each coordinate race phases, ticking,
  input, lobby-to-race setup, item side effects, and snapshots in their own
  adapter code.
- This makes browser and single-player paths vulnerable to regressions for
  behavior already fixed in terminal multiplayer.

Shared session responsibilities:

- Build race participants from selected lobby players.
- Start countdowns and transition into active races.
- Apply human key input through shared typing, bonus, item, and race-flow
  helpers.
- Advance AI racers, mushroom boosts, item expiries, bonus cooldowns, and race
  lifecycle during periodic ticks.
- Cancel active races and return participants to the lobby.
- Produce semantic outcomes for adapters to render, log, or broadcast.

Keep outside the shared core:

- TCP sockets, relay room management, browser WebSocket wrappers, and terminal
  input loops.
- Ratatui rendering, Leptos signals, DOM updates, and CSS.
- Wall-clock scheduling primitives. Adapters should pass `Instant`/elapsed time
  into shared methods such as `start_countdown`, `apply_input`, and `tick`.

Suggested API shape:

- Add a browser-compatible `src/game/host_session.rs` or extend the existing
  `src/game/engine.rs` facade only if it can remain a small coordinator.
- Expose methods such as `prepare_race_from_lobby`, `start_countdown`,
  `apply_player_key`, `tick`, `cancel_race`, `return_to_lobby`, and
  `build_results`.
- Return structured outcomes rather than writing to UI, logs, sockets, or
  Leptos signals directly.

Migration sequence:

1. Extract one narrow shared session operation from the network host, where the
   behavior is already most mature and best covered.
2. Adopt that operation in the browser host.
3. Adopt the same operation in local single-player.
4. Repeat for the next operation instead of attempting a single large rewrite.

Validation:

- Each migrated operation should have shared `src/game` tests.
- Existing LAN, browser-host, and local session tests should continue to pass.
- Manual validation should compare the same scenario across local, LAN, and
  browser-hosted races.

Progress:

- `src/game/host_session.rs` now owns shared lobby-to-race preparation for
  host sessions, including ready/connected participant selection, `RaceState`
  construction, and bonus generation.
- Network-hosted rematches and browser-hosted countdown starts now both prepare
  races through the shared helper.
- Network countdown-connected-racer checks now use the shared host-session
  active-racer helper.
- `src/game/host_session.rs` now owns shared countdown phase policy for start
  eligibility, no-racer rejection, countdown ticks, transition into racing, and
  return-to-lobby decisions.
- Network-hosted and browser-hosted countdown starts now use the shared phase
  policy while keeping their own timers, broadcasts, and UI updates.
- `src/game/host_session.rs` now returns shared return-to-lobby outcomes with
  semantic event text, so network and browser adapters use the same lifecycle
  meaning while retaining adapter-specific cleanup and broadcasts.
- `src/game/host_session.rs` now owns a shared active-race tick outcome shape
  for choosing whether an adapter should ignore a tick, broadcast a delta, or
  publish final results after its local AI/item/lifecycle work.
- `src/game/host_session.rs` now owns active-race lifecycle advancement for
  host sessions, including the "only while racing" guard and the transition to
  the finished phase. Network and browser hosts both use this helper while
  keeping event feed, result snapshots, and broadcasts adapter-owned.
- `src/game/host_session.rs` now returns a shared start-race outcome from the
  countdown-to-racing transition, including the `Racing` phase and common race
  started event text used by network and browser hosts.
- `src/game/host_session.rs` now returns a shared countdown-cancel outcome,
  including the `WaitingForHost` phase and common cancellation event text used
  by network and browser hosts.
- `src/game/host_session.rs` now returns shared host runtime reset outcomes for
  preparing a waiting race and entering an active race. Network and browser
  hosts use these outcomes while applying their own concrete runtime, results,
  event, and AI timing fields.
- `src/game/host_session.rs` now includes shared race-finished event text in
  the host lifecycle outcome, so network and browser hosts use the same finish
  event while keeping result snapshots and logs adapter-owned.

### 11. Browser Host Authority Convergence

Goal: reduce browser-host-specific gameplay authority in
`web/typekart-web/src/host.rs` by moving rule decisions into shared game code.

Current issue:

- Browser host code still owns significant orchestration around lobby state,
  AI ticking, bonus state generation, race snapshot synchronization, and race
  lifecycle glue.
- Some browser state is legitimate adapter code, but browser-hosted races
  should not decide gameplay behavior differently from terminal-hosted races.

Targets:

- Replace browser-specific race setup and tick orchestration with calls into
  the shared host session core.
- Replace browser-specific lobby mutation paths with shared lobby/session
  outcomes where possible.
- Keep relay id mapping, browser signals, and DOM state updates in the browser
  adapter.

Validation:

- Browser host with browser joiner.
- Browser host with terminal joiner.
- AI countdown, item effects, bonus availability, cancel/rematch, and results
  parity against terminal multiplayer.

### 12. Local Single-Player Session Convergence

Goal: make local terminal play a local adapter around the same authoritative
session logic used by LAN and browser hosts.

Current issue:

- `src/ui/session.rs` still owns bespoke local versions of gameplay behavior,
  even though some lifecycle and AI helpers have already moved into shared
  modules.
- Local play should not be a special rule implementation. It should be a host
  session with one human, optional AIs, and no transport.

Targets:

- Route local race setup, input, AI advancement, item effects, bonus flow,
  lifecycle, and result construction through shared session operations.
- Preserve terminal-only rendering, local command handling, and gallery/debug
  helpers in `src/ui`.

Validation:

- Existing `ui::session` tests.
- Local single-player manual race with AIs, bonus pickup, each item effect,
  race cancel/restart, and result display.

### 13. Shared Host Events And Snapshot Frames

Goal: give adapters structured state changes from shared gameplay code instead
of duplicating event strings and snapshot timing logic.

Targets:

- Emit structured host events such as item pickup, player hit, shield block,
  bonus claim, countdown start, race start, finish, cancel, and return-to-lobby.
- Keep adapter-specific wording, debug logs, event-feed truncation, and network
  broadcasts outside shared gameplay code.
- Expand shared snapshot/frame helpers so terminal, LAN, and browser adapters
  project the same authoritative state into race, lobby, and result views.

Validation:

- Shared event tests should assert semantic event data, not UI strings.
- Existing terminal and browser event-feed tests should cover adapter wording.

### 14. Clock And Tick Boundary

Goal: keep shared gameplay deterministic and browser-compatible while allowing
terminal, LAN, and browser adapters to schedule work differently.

Rules:

- Shared game modules may accept `Instant`, elapsed milliseconds, or tick
  context, but should not sleep, spawn threads, block, or own timers.
- Terminal/LAN code may use native threads and blocking socket loops.
- Browser code may use browser timers and async WebSocket callbacks.
- Local terminal code may use its event loop cadence.

Validation:

- Shared tick tests should advance fake times deterministically.
- Adapter tests should verify that each interface calls the shared tick methods
  at the expected phase boundaries.

## Suggested Order

1. Split `web/typekart-web/src/main.rs` into smaller modules.
2. Expand `src/game/engine.rs` into the shared race host facade.
3. Move bonus attempt flow into shared gameplay code.
4. Extract shared snapshot building.
5. Extract shared lobby policy.
6. Consolidate rematch/result orchestration through the shared facade.
7. Extract renderer view-model helpers only after gameplay parity is stable.
8. Migrate local terminal session rules to shared gameplay modules.
9. Split network host adapter modules after shared ownership boundaries are
   clear.
10. Define and migrate toward the shared host session core one operation at a
    time.
11. Converge browser host authority onto shared session operations.
12. Converge local single-player onto shared session operations.
13. Replace duplicated host event strings and frame projection with structured
    shared events/snapshots.
14. Keep clock/tick scheduling adapter-owned while testing shared tick behavior
    with deterministic fake times.
15. Split large inline tests into focused sibling or integration test modules
    after the shared-logic boundaries settle.

## Deferred Cleanup

- Split large inline Rust test modules into sibling `tests.rs` modules or
  focused integration tests after the shared-logic refactors stabilize. Rust
  commonly keeps unit tests beside production code, but the current large
  modules in files like `src/net/server.rs`, `src/ui/session.rs`, and
  `src/game/item_effects.rs` are making navigation harder. Deferring this
  avoids moving tests repeatedly while production code is still being split.

## Out Of Scope For This Refactor

- Replacing the terminal renderer with a browser renderer or vice versa.
- Replacing TCP/LAN networking with browser WebSockets.
- Public matchmaking, accounts, rankings, or anti-cheat.
- Deployment of the web UI.

## Health Checks

Run these after each slice:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo clippy --manifest-path web/typekart-web/Cargo.toml --locked --all-targets -- -D warnings
cargo test --manifest-path web/typekart-web/Cargo.toml --locked
cargo check --locked --no-default-features
cargo check --manifest-path web/typekart-web/Cargo.toml --locked --target wasm32-unknown-unknown
```

Manual validation should cover at least:

- CLI host with terminal joiner.
- CLI host with browser joiner.
- Browser host with browser joiner.
- Browser host with terminal joiner.
- Bonus pickup and each item effect in browser-hosted and CLI-hosted races.
