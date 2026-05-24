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

Validation:

- Local session finish/timeout tests.
- Full `ui::session` test module.

### 9. Network Host Module Split

Goal: shrink `src/net/server.rs` without moving rules back out of shared code.

Candidate extractions:

- `src/net/host_lifecycle.rs`: host-only broadcast sequencing around shared
  race-flow outcomes and result snapshots.
- `src/net/host_lobby.rs`: TCP-specific lobby side effects around shared lobby
  policy, including client kicks and lobby snapshot broadcasts.
- `src/net/host_ai.rs`: network-specific AI timing map and debug logging around
  shared `ai_driver`.
- `src/net/host_bonus.rs`: network-specific event/log handling around shared
  `bonus_flow`.

Constraint:

- These modules should not own gameplay policy. They should orchestrate
  transport, logging, and adapter state around shared `src/game` helpers.

Progress:

- `src/net/host_lifecycle.rs` now owns network-host result message construction
  and race-finish feed/log formatting around shared `race_flow` and `snapshot`
  helpers.
- `src/net/server.rs` still owns socket writes, phase mutation, and
  `HostState`, but no longer constructs race-result protocol messages inline.

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
