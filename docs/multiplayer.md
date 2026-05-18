# Multiplayer Guide

TypeKart has two multiplayer paths:

- Internet play through a WebSocket relay: `host` and `join`.
- Direct LAN play through a TCP socket: `host-lan` and `join-lan`.

The host is authoritative in both modes. Joiners send input commands and render snapshots from the host.

## Internet Play

The easiest path is the public relay:

```sh
typekart host --name host
```

The host prints a room code and a join command:

```text
Online room: rocket-salad-tiger
Join command: typekart join --name PLAYER --relay wss://typekart-relay.fly.dev --room rocket-salad-tiger
```

Other racers join with:

```sh
typekart join --name player2 --room rocket-salad-tiger
```

Use `--relay` only for a local, private, or self-hosted relay.

## LAN Play

On one computer:

```sh
cargo run -- host-lan --name host --bind 127.0.0.1:4000
cargo run -- join-lan --name player2 --server 127.0.0.1:4000
```

On a local network:

```sh
typekart host-lan --name host --bind 0.0.0.0:4000
typekart join-lan --name player2 --server HOST_IP:4000
```

If joiners cannot connect, check that the host firewall allows inbound traffic on the chosen port and that joiners are using the host machine's LAN IP address.

## Lobby Lifecycle

- The host starts ready.
- Joiners press `Enter` to ready or unready.
- The host can use `Up` and `Down` to select lobby rows.
- The host can press `A` to add an AI racer.
- The host can press `X` to remove the selected AI racer or kick the selected human joiner.
- The host can press `E` or `H` to set Easy or Hard AI difficulty for the selected AI. If a human is selected, this changes the default difficulty for newly added AIs.
- The host presses `Space` to start with ready connected racers. Unready players stay in the lobby and can ready for the next race.
- During countdown, input is locked and the track text is grey.
- During countdown or racing, the host can press `Ctrl-R` to cancel the race and return everyone to the lobby.
- If too few connected racers remain during countdown, the game returns to the lobby.
- Players who join or ready up while a race is active can watch the current race from the lobby and join the next race.
- After results, the host can return to the lobby or start a rematch with connected lobby players.

The footer shows the primary phase command plus `? help`. Press `?` to show or hide a centered command overlay with lobby management controls and typed command fallbacks.

## Version Compatibility

All multiplayer racers must run the same TypeKart version.

- LAN joins are rejected by the host if the client version differs.
- Relay joins are rejected before being forwarded to the host if the client version differs from the room host.

When a join fails unexpectedly, compare the release archive names, Homebrew formula version, or git checkout on every machine. A dedicated `typekart --version` flag is not exposed yet.

## AI Racers

The host can add server-owned AI racers:

```sh
typekart host --name host --ai-racers 4 --ai-difficulty easy
```

The host can also add, remove, and retune AI racers from the lobby before the race starts. AI racers participate in the authoritative host simulation, can collect items, and appear to all clients.

## Kicking Players

The host can kick a human joiner from the lobby by selecting that player and pressing `X`.

- Duplicate connected player names receive a numeric suffix.
- Disconnected players do not reserve their old name.
- The host cannot kick themselves.
- Kicks are only available before the countdown or race starts.

## Diagnostics

Use debug logs when investigating multiplayer issues:

```sh
typekart host --name host --debug-log host-debug.log
typekart join --name player2 --room rocket-salad-tiger --debug-log player2-debug.log
```

For LAN-specific manual checks, use [LAN Validation Checklist](lan-validation-checklist.md).
