# LAN Validation Checklist

Use this checklist when validating local-network multiplayer behavior on loopback or on two machines.

## Setup

Same computer:

```sh
cargo run -- host-lan --name host --bind 127.0.0.1:4000 --debug-log host-debug.log
cargo run -- join-lan --name player2 --server 127.0.0.1:4000 --debug-log player2-debug.log
```

Local network:

```sh
cargo run -- host-lan --name host --bind 0.0.0.0:4000 --debug-log host-debug.log
cargo run -- join-lan --name player2 --server HOST_IP:4000 --debug-log player2-debug.log
```

Useful shorter race:

```sh
cargo run -- host-lan --name host --bind 0.0.0.0:4000 --words 20 --debug-log host-debug.log
```

Use the same `--ascii` setting on host and joiners when validating ASCII fallback rendering.

## Lobby

- Host appears in the lobby.
- Joiners appear with unique colors.
- Duplicate names are rejected.
- Full lobby join attempts show a clear rejection.
- Joiners can press `Enter` to ready up in lobby/waiting phases.
- Typed `ready` and `unready` remain supported as fallback lobby commands.
- Host can start with `Space` or `start` once connected players are ready.
- Footer commands change by phase and do not show irrelevant race/result commands.
- Lobby Events show joins, ready changes, disconnects, countdown start/cancel, and rematch transitions.

## Countdown

- Race text is grey before input is accepted.
- Countdown appears next to the local racer marker.
- Disconnecting a joiner during countdown cancels the countdown if fewer than two connected racers remain.
- After countdown cancellation, the host returns to a lobby/waiting state and can accept new players.

## Racing

- Typing advances the local racer with the same track-centering behavior as single player.
- Typos are shown on the track, but typo start/clear events do not spam the Events panel.
- Bonus choices are visible before reaching them.
- Bonus choices can only be claimed at the correct gap.
- Losing a contested bonus forces the player back to the main track word.
- Bonus refreshes update the track but do not spam the Events panel.

## Items

- Mushroom boost advances one word at a time and shows the boost marker.
- Banana targets the nearest valid unfinished racer.
- Banana misses are shown once in Events.
- Banana hits show one concise attacker/target event and the target blink.
- Shield blocks Banana and consumes the shield.
- Star Power shows the star racer marker and discards incorrect keys without forcing backspace recovery.
- Blue Shell shows the turtle attack cue, reverses the target's next word, and makes the target blink blue.
- Shield blocks Blue Shell and consumes the shield.
- Finished racers are not valid item targets.
- Debug logs include detailed item cause and effect, including target candidates and distances.

## Results And Rematch

- Results show all race participants.
- Finished racers rank by finish order.
- Timed-out unfinished racers rank by progress.
- Disconnected racers are represented clearly.
- Host can return to lobby after results.
- Host can start a fresh race from connected lobby players, including players who joined during the previous race.
- Result footer shows host-only rematch/lobby commands only to the host; joiners only see quit.

## Disconnects

- Lobby disconnect frees capacity and color assignment.
- Waiting disconnected players are cleaned from the next race roster.
- Racing disconnect keeps that racer visible for context.
- If all active racers finish or disconnect, the race ends cleanly.
- Host debug log records disconnect cleanup and race-end cause.

## Debug Log Review

Compare host and joiner logs after each validation run:

- Session start settings: word count, mod hashes, address.
- Countdown start/cancel/start race timing.
- Snapshot sequence around any visible issue.
- Bonus pickup and item resolution sequence.
- Race finish cause and result broadcast.

Prefer fixing missing debug context before fixing a hard-to-reproduce UI issue.
