# Network AI Racers Plan

## Goal

Add server-owned AI racers to multiplayer so the host can manually test race behavior, item interactions, results, and rematches without needing many human joiners or many terminal windows.

Network AI racers should behave like normal race participants from the client's perspective: they appear in lobby/race/results, move over the network through server snapshots, can claim bonuses, can trigger items, can be targeted by humans, and can finish races.

## Design Direction

Implement bots inside the host process instead of launching fake TCP clients.

Why:

- Server-owned bots exercise the authoritative race logic directly.
- No process management or reconnect behavior is needed.
- Bots can be deterministic enough for tests while still feeling like racers.
- The same host can run a useful manual multiplayer test with one terminal client.

The protocol should still identify racer kind explicitly. Do not rely on name prefixes like `ai-1`.

## CLI Shape

Host-only flags:

```sh
cargo run -- host --name host --bind 127.0.0.1:4000 --ai-racers 4 --ai-difficulty easy
```

Harder bots:

```sh
cargo run -- host --name host --bind 127.0.0.1:4000 --ai-racers 4 --ai-difficulty hard
```

Validation-friendly short race:

```sh
cargo run -- host --name host --bind 127.0.0.1:4000 --words 20 --ai-racers 4
```

## Racer Capacity

For the first implementation, AI racers should count against the same total racer/color limit as human players.

Example:

- `--max-players 6 --ai-racers 4` leaves two human slots, one of which is the host if `--name` is present.
- If the host config requests too many AI racers for the current maximum, reject startup with a clear error.

This keeps rendering, color assignment, minimap markers, and result tables inside existing limits.

## Implementation Slices

### Slice 1: Protocol Racer Kind

Status: implemented.

Add explicit racer kind to network-facing player data.

Deliverables:

- Add `PlayerKind::{Human, Bot}` to `src/net/protocol.rs`.
- Add `kind: PlayerKind` to `LobbyPlayer`.
- Add `kind: PlayerKind` to `PlayerSnapshot`.
- Populate all current players as `Human`.
- Update protocol round-trip tests and fixture builders.
- No UI behavior change is required yet.

Validation:

- Existing multiplayer tests still pass.
- Serialized lobby and race snapshots include `kind`.

### Slice 2: Host Config And Bot Roster

Status: implemented for CLI/config and static bot roster. Bot movement begins in Slice 3.

Add host CLI/config support and create bot racers.

Deliverables:

- Add `--ai-racers` to `host`.
- Add `--ai-difficulty easy|hard` to `host`.
- Add AI fields to `HostConfig`.
- Create bots with stable names like `ai-1`, `ai-2`.
- Assign colors from the existing rotation.
- Add bots to lobby/race state with `kind: Bot`.
- Bots are always connected and ready.

Validation:

- Host with bots and no joiners can start when at least two racers exist.
- Host plus one joiner plus bots shows all racers in lobby.
- Full lobby rejection accounts for human plus bot capacity.

### Slice 3: Server AI Typing

Status: implemented for main-track typing. Bonus claiming and item use remain separate slices.

Move or reuse local AI typing behavior on the host.

Deliverables:

- Add server-internal AI state: WPM, char budget, last update time.
- Sample WPM from existing easy/hard ranges.
- On race ticks, apply AI key input through server-authoritative typing paths.
- Respect countdown/racing/finished states.
- Respect stun and Mushroom input pause.
- Keep main-track AI typing separate from bonus intent inference until Slice 4.

Validation:

- Bots advance during racing.
- Bots finish and appear in result order.
- Bot movement is visible to joiners through normal snapshots.

### Slice 4: Bonus Claiming

Let bots claim bonus words using the same server bonus state as humans.

Deliverables:

- Detect eligible bonus gaps for each bot.
- Pick an available choice when eligible.
- Apply bonus typing/claiming through existing server-owned bonus resolution.
- Obey active-effect lockouts.

Validation:

- Bots can claim bonuses.
- Contested bonus choices still resolve once.
- Bots cannot claim while shielded, boosted, stunned, or ineligible.

### Slice 5: Item Interactions

Wire bot item effects through existing server-owned item resolution.

Deliverables:

- Bot Mushroom advances through existing Mushroom effect.
- Bot Banana targets nearest valid unfinished racer, including humans.
- Human Banana can target bots.
- Bot Shield blocks Banana.
- Event feed and debug logs identify bot actors/targets clearly.

Validation:

- Bot hits human.
- Human hits bot.
- Shield blocks in both directions.
- Finished bots/humans are not valid item targets.

### Slice 6: Rematch And Cleanup

Make bots work across race lifecycle transitions.

Deliverables:

- Rebuild bots for each rematch/new race.
- Keep bot count stable unless the host restarts with different CLI options.
- Ensure bots do not occupy disconnected human cleanup paths.

Validation:

- Race completes with bots.
- Host returns to lobby.
- Host starts a new race with connected humans plus recreated bots.

## UI Notes

Initial UI can render bots exactly like humans because `kind` is protocol-visible.

Later polish options:

- Show `BOT` in lobby rows.
- Show bot labels in player lists/results.
- Add a host lobby hint showing human slots vs bot racers.

## Non-Goals

- Running bots as separate client processes.
- Internet matchmaking or hosted bot services.
- Bot chat/personality.
- Per-bot difficulty configuration.
- New AI item strategy beyond simple bonus claiming and automatic item activation.
