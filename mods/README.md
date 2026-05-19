# TypeKart Mod Templates

This directory contains copyable templates for TypeKart's current data-only mod support.

## Word Sets

Word sets are newline-delimited `.txt` files:

```sh
typekart play --word-set-file ./mods/words/classic.txt
typekart host --word-set-file ./mods/words/classic.txt
```

Use a directory to let the host pick one word pack at random for each race:

```sh
typekart play --word-set-dir ./mods/words
typekart host --word-set-dir ./mods/words
```

## Item Packs

Item packs are JSON files that tune built-in items:

```sh
typekart play --item-pack-file ./mods/items/classic.json
typekart host --item-pack-file ./mods/items/classic.json
```

The included item template matches the built-in defaults. The included word template is the playable lowercase-only version of the bundled word list.
