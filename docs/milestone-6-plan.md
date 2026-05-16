# Milestone 6 Plan: Multiplayer Polish

## Goal

Polish the local-network multiplayer experience now that the core host/join loop and modding foundation exist.

Milestone 6 should make multiplayer easier to test, easier to understand during a race, and less brittle when players disconnect or finish. It should avoid broad new modding work and avoid internet-play infrastructure.

## Scope

Milestone 6 includes:

- Manual LAN validation and fixes from real multi-machine testing.
- Better lobby and race UI clarity.
- Better event feed behavior.
- Better disconnect handling.
- More useful multiplayer results and stats.
- Returning from results into a fresh race.
- Item weighting and balance improvements.
- Additional item effects only if they build cleanly on existing mechanics.
- More robust terminal controls and help text.

Milestone 6 excludes:

- Internet matchmaking, relay servers, NAT traversal, or hosted authoritative servers.
- Arbitrary mod scripts.
- Manifest-backed mod-pack selection UI.
- Large renderer rewrites.
- Replacing TCP or the newline-delimited JSON protocol unless LAN testing exposes a real blocker.

## Current Starting Point

Implemented before Milestone 6:

- `host` and `join` commands.
- Host-as-local-client architecture.
- Lobby readiness and host-started countdown.
- Server-authoritative typing input.
- Server-owned bonus choices, bonus claims, cooldowns, and item rolls.
- Server-owned Mushroom, Banana, and Shield behavior.
- Race snapshots at 20 snapshots per second while racing.
- Results broadcast with placement order and structured result rows.
- Network UI with track window, racer lanes, typo coloring, bonus lanes, minimap, events, and mod metadata.
- Debug logs for host and join.
- Word-set and item-pack mod metadata in lobby/race snapshots.

Known rough edges:

- Event feed is functional but can be noisy or underspecified.
- Disconnect/rematch behavior has automated coverage for lobby cleanup, capacity/color reuse, countdown cancellation, rematch roster rebuilds, and no-connected-racer race completion, but still needs manual LAN validation.
- LAN behavior has mainly been tested on loopback.
- Terminal controls are implicit and not discoverable enough.
- Item pool is still Mushroom, Banana, and Shield.

## Proposed Implementation Order

### Slice 1: LAN Validation Checklist And Debug Ergonomics

Create a repeatable validation checklist and make logs easier to compare between host and joiners.

Deliverables:

- Document exact host/join commands for same-machine and LAN testing.
- Add a short checklist for lobby, countdown, typing, bonus claim, item hit/block, finish, results, disconnect, and rematch/restart behavior.
- Ensure host and join debug logs include enough shared context to line up events by time, player id, and snapshot sequence.

Validation:

- Run two clients on loopback.
- Run at least one real LAN test with two machines when available.

### Slice 2: Results And Stats

Status: implemented.

Make end-of-race multiplayer results useful, not just ordered.

Deliverables:

- Extend race result data with racer names, finish/timeout status, progress, WPM, accuracy, typo count, and backspaces.
- Render a results table for all players in the network UI.
- Keep `RaceResults` placement order internally and send structured result rows beside it.

Validation:

- Finished racers rank by finish order.
- Timed-out racers rank by progress.
- Disconnected racers are represented clearly.
- Results survive late or duplicate result broadcasts.

### Slice 3: Disconnect Handling

Status: implemented for server-side phase handling; still needs manual LAN validation.

Clarify what happens when players leave during lobby, countdown, racing, and results.

Target behavior:

- Lobby disconnects free capacity and color slots.
- Disconnected lobby/waiting players are removed from the next race roster.
- Countdown disconnects should not leave the race stuck; if fewer than two connected racers remain, the countdown is cancelled back to waiting.
- Racing disconnects mark the racer disconnected but keep them visible for context.
- Disconnected unfinished racers can be ranked after active racers at timeout.
- If all remaining active racers finish or all racers disconnect, the race ends cleanly.
- After results, the host can start a fresh race using the currently connected lobby players, including players who joined while the previous race was active.
- After results, the host can use `lobby`, `restart`, or `rematch` to return everyone to the lobby before starting again, or use `start` / `Space` to start the rematch countdown directly.

Validation:

- Disconnect before ready.
- Disconnect after ready.
- Disconnect during countdown.
- Disconnect mid-race.
- Disconnect after finishing.

### Slice 4: Event Feed Polish

Make the event feed useful for racers without overloading it.

Deliverables:

- Normalize event wording between local and network play.
- Keep high-signal events: joins, disconnects, race start, bonus pickup, item hit, item block, miss, finish.
- Avoid repeated low-signal snapshot or cooldown events in player-facing UI.
- Keep detailed causal information in debug logs.

Validation:

- Race with several item events remains readable.
- Debug logs still reconstruct item cause and effect.

### Slice 5: Item Weighting And Balance

Improve item rolls without adding a major new effect engine.

Deliverables:

- Expand item roll context beyond nearby-racer detection.
- Consider placement/progress-aware weights.
- Keep item registry/mod-pack tuning compatible with the new weighting model.
- Add tests for first-place, middle, trailing, and nearby-racer contexts.

Validation:

- First-place racers receive fewer disruptive attacks.
- Trailing racers receive more comeback help.
- Shield remains more likely when racers are nearby.

### Slice 6: Terminal Control Polish

Status: partially implemented for phase-aware network command display and parsing.

Make lobby and race controls easier to discover and less fragile.

Deliverables:

- Improve footer/help text.
- Drive available commands from the current network phase so irrelevant commands are hidden and ignored.
- Decide whether a lightweight help overlay is worthwhile.
- Validate key handling on common terminal environments.
- Keep manual item activation reserved until held items return.

Validation:

- Host can start countdown by `Space` and `start`.
- Joiners can ready/unready/quit.
- Racing input ignores non-game keys without producing confusing behavior.

## Deferred From Milestone 6

These remain useful but should wait unless they become blockers:

- Star Power.
- Blue Shell.
- Full shared item engine.
- Generic item effect/cue snapshots.
- Manifest-backed word-pack collections.
- Internet play.

Star Power and Blue Shell are gameplay polish, but both are more expensive than they look because they introduce new typing-rule and display-rule surfaces. They should follow after results, disconnects, and event clarity are solid.

## Recommended First Slice

Start with Slice 2: multiplayer results and stats.

Reasoning:

- It directly improves every test run.
- It reveals whether network snapshots/results contain enough durable player data.
- It gives us a better end-state before deeper disconnect and LAN testing.
- It is less risky than adding new item effects.
