# CLI Reference

Installed binaries use `typekart`. Development examples can be run from the repository with `cargo run --`.

## Common Commands

Solo race:

```sh
typekart play
```

Solo race with AI racers:

```sh
typekart play --ai-racers 3 --ai-difficulty easy
```

Host an internet race through the default public relay:

```sh
typekart host --name host
```

Join an internet race:

```sh
typekart join --name player2 --room rocket-salad-tiger
```

Host a LAN race:

```sh
typekart host-lan --name host --bind 0.0.0.0:4000
```

Join a LAN race:

```sh
typekart join-lan --name player2 --server HOST_IP:4000
```

## Internet Relay Options

`host` and `join` default to the public TypeKart relay:

```text
wss://typekart-relay.fly.dev
```

Use `--relay` for a local or private relay:

```sh
typekart host --name host --relay ws://127.0.0.1:8080
typekart join --name player2 --relay ws://127.0.0.1:8080 --room rocket-salad-tiger
```

Start a local relay:

```sh
typekart relay --bind 127.0.0.1:8080
```

## Race Options

Set the track length:

```sh
typekart play --words 20
typekart host --name host --words 20
typekart host-lan --name host --words 20
```

Set the maximum multiplayer lobby size, including the host:

```sh
typekart host --name host --max-players 6
typekart host-lan --name host --max-players 6
```

Add server-owned AI racers to multiplayer:

```sh
typekart host --name host --ai-racers 4 --ai-difficulty easy
typekart host-lan --name host --ai-racers 4 --ai-difficulty hard
```

The host can also add, remove, and retune AI racers from the lobby before starting:

```text
Press ? in the lobby for: ↑/↓ select | A add AI | X remove/kick | E easy | H hard
```

Use ASCII-safe item markers:

```sh
typekart play --ascii
typekart host --name host --ascii
typekart join --name player2 --room rocket-salad-tiger --ascii
```

Write a debug log:

```sh
typekart play --debug-log local-debug.log
typekart host --name host --debug-log host-debug.log
typekart join --name player2 --room rocket-salad-tiger --debug-log player2-debug.log
```

## Mod Options

Use a custom word-set file:

```sh
typekart play --word-set-file ./mods/words/custom.txt
```

Choose a random `.txt` word set from a directory:

```sh
typekart host --name host --word-set-dir ./mods/words
```

Use an item pack:

```sh
typekart play --item-pack-file ./mods/items/classic-plus.json
typekart host --name host --item-pack-file ./mods/items/classic-plus.json
```

## Help

```sh
typekart --help
typekart play --help
typekart host --help
typekart join --help
typekart host-lan --help
typekart join-lan --help
typekart relay --help
```
