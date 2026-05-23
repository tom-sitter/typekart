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
- Move browser host lobby/race authority code into `host.rs`.
- Move renderer helpers and Leptos components into `render.rs` or
  `components.rs`.
- Keep fixtures in `fixtures.rs`.

Validation:

- `cargo clippy --manifest-path web/typekart-web/Cargo.toml --locked --all-targets -- -D warnings`
- `cargo test --manifest-path web/typekart-web/Cargo.toml --locked`
- `cargo check --manifest-path web/typekart-web/Cargo.toml --locked --target wasm32-unknown-unknown`

Notes:

- This should be mostly movement, not behavior changes.
- Do this before adding more browser parity work.
- Progress: browser session/control primitives have been extracted to
  `web/typekart-web/src/session.rs`, and browser host authority has been moved
  to `web/typekart-web/src/host.rs`. Host-specific tests now live beside the
  host module, so `main.rs` only keeps session, relay-join, and renderer tests.
  Renderer components and layout helpers have been extracted to
  `web/typekart-web/src/render.rs`.

### 2. Shared Race Host Facade

Goal: grow `src/game/engine.rs` from the current small `CoreHost` into the
single browser-safe authority for race ticking.

Responsibilities to move behind the facade:

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

Validation:

- Existing network host tests should still pass while using the facade.
- Browser-hosted tests should use the same facade.
- Add focused engine tests that do not instantiate terminal or browser code.

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

- `BonusAttemptState` in `src/game`.
- A function that accepts `RaceState`, `BonusState`, `RaceRuntimeState`, player
  id, key action, item registry, and current time.
- A small event enum such as `BonusAttemptStarted`, `BonusClaimed`,
  `BonusLost`, `BonusBailed`, and `BonusInputChanged`.

Validation:

- Port existing server and browser bonus tests to shared tests first.
- Keep protocol-specific events/log strings at the adapter layer.

### 4. Shared Snapshot Builder

Goal: build protocol-shaped race snapshots from shared race state in one place.

Current issue:

- Terminal host and browser host both convert `RaceState`, bonus state, runtime
  item effects, and mod metadata into snapshots.
- Small divergence here can make the browser and terminal render different
  statuses for the same authoritative state.

Suggested extraction:

- Create a snapshot builder near `src/net/protocol_types.rs` or a new
  browser-safe module such as `src/game/snapshot.rs`.
- Inputs should be shared state plus an adapter for player kind/color/mod
  metadata.
- Outputs should remain protocol types.

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

- `LobbyState` with game-level player records, host id, max players, and AI
  settings.
- Commands such as `SetReady`, `Rename`, `AddAi`, `RemovePlayer`,
  `SetAiDifficulty`, and `SelectRaceParticipants`.
- Events such as `PlayerRenamed`, `PlayerRemoved`, `AiAdded`, and
  `LobbyRejected`.

Validation:

- Move server lobby tests into shared tests where possible.
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

## Suggested Order

1. Split `web/typekart-web/src/main.rs` into smaller modules.
2. Expand `src/game/engine.rs` into the shared race host facade.
3. Move bonus attempt flow into shared gameplay code.
4. Extract shared snapshot building.
5. Extract shared lobby policy.
6. Consolidate rematch/result orchestration through the shared facade.
7. Extract renderer view-model helpers only after gameplay parity is stable.

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
