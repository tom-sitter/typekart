# TypeKart

TypeKart is a terminal typing racer with kart-style item effects.

## Requirements

- Rust installed locally.
- Run commands from the repository root.

## Single Player

Start a local race:

```sh
cargo run -- play
```

Start a shorter test race:

```sh
cargo run -- play --words 20
```

Race against AI players:

```sh
cargo run -- play --ai-racers 6 --ai-difficulty easy
```

Use harder AI players:

```sh
cargo run -- play --ai-racers 6 --ai-difficulty hard
```

Use ASCII-safe item markers:

```sh
cargo run -- play --ascii
```

## Multiplayer On One Computer

Terminal 1, host the race:

```sh
cargo run -- host --name host --bind 127.0.0.1:4000
```

Terminal 2, join the race:

```sh
cargo run -- join --name player2 --server 127.0.0.1:4000
```

Additional local players can join from more terminals with unique names:

```sh
cargo run -- join --name player3 --server 127.0.0.1:4000
```

## Multiplayer On A Local Network

The host should bind to all local interfaces:

```sh
cargo run -- host --name host --bind 0.0.0.0:4000
```

Other computers join using the host computer's local IP address:

```sh
cargo run -- join --name player2 --server HOST_IP:4000
```

Example:

```sh
cargo run -- join --name player2 --server 192.168.1.25:4000
```

## Common Multiplayer Options

Set the track length:

```sh
cargo run -- host --name host --words 20 --bind 127.0.0.1:4000
```

Set the lobby size, including the host:

```sh
cargo run -- host --name host --max-players 6 --bind 127.0.0.1:4000
```

Add network AI racers for multiplayer testing:

```sh
cargo run -- host --name host --bind 127.0.0.1:4000 --ai-racers 4 --ai-difficulty easy
```

Use ASCII-safe item markers for host and joiners:

```sh
cargo run -- host --name host --bind 127.0.0.1:4000 --ascii
cargo run -- join --name player2 --server 127.0.0.1:4000 --ascii
```

Write debug logs:

```sh
cargo run -- host --name host --bind 127.0.0.1:4000 --debug-log host-debug.log
cargo run -- join --name player2 --server 127.0.0.1:4000 --debug-log player2-debug.log
```

## Word Sets And Item Packs

Use a custom newline-delimited word file:

```sh
cargo run -- play --word-set-file ./mods/words/custom.txt
```

Choose a random `.txt` word set from a directory:

```sh
cargo run -- play --word-set-dir ./mods/words
```

Use an item pack:

```sh
cargo run -- play --item-pack-file ./mods/items/classic-plus.json
```

Use mods while hosting:

```sh
cargo run -- host --name host --bind 127.0.0.1:4000 \
  --word-set-dir ./mods/words \
  --item-pack-file ./mods/items/classic-plus.json
```

## In-Game Flow

- In single player, press `Space` to start the countdown.
- In multiplayer, players ready up in the lobby, then the host starts the race.
- After a multiplayer race, the host can return to the lobby or start another race from the results screen.

For the full CLI reference:

```sh
cargo run -- --help
cargo run -- play --help
cargo run -- host --help
cargo run -- join --help
```

For structured multiplayer validation, see [docs/lan-validation-checklist.md](docs/lan-validation-checklist.md).
