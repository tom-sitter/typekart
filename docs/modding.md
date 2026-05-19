# Modding Guide

TypeKart currently supports data-only modding for word sets and built-in item tuning. Mods are loaded by the host or local player. Multiplayer joiners do not need local copies because the host sends generated track words and authoritative race snapshots.

## Word Sets

A word-set file is a newline-delimited `.txt` file.

Rules:

- One word per line.
- Lowercase ASCII letters only.
- No punctuation, spaces, numbers, or capitalization.
- At least three unique words.
- Blank lines are ignored by the word-list loader.

Example:

```text
turbo
driver
corner
boost
banana
shield
```

Use one file:

```sh
typekart play --word-set-file ./mods/words/racing.txt
typekart host --name host --word-set-file ./mods/words/racing.txt
```

Use a directory of word packs. TypeKart loads every `.txt` file and picks one at random for the race:

```sh
typekart play --word-set-dir ./mods/words
typekart host --name host --word-set-dir ./mods/words
```

The file stem becomes the displayed word-set name and id.

## Item Packs

Item packs are JSON files that tune built-in items. The current mod surface does not add arbitrary new item behavior or execute scripts.

Supported item ids:

- `mushroom`
- `banana`
- `shield`
- `focus`
- `cyclone`
- `squid_ink`

Example:

```json
{
  "items": [
    {
      "id": "mushroom",
      "name": "Big Mushroom",
      "context_weights": {
        "standard": { "first": 2, "middle": 4, "trailing": 8 },
        "nearby_racer": { "first": 3, "middle": 5, "trailing": 10 }
      },
      "effect": {
        "boost_words": 4,
        "wpm": 220
      }
    },
    {
      "id": "banana",
      "effect": {
        "range_words": 12,
        "stun_ms": 2000,
        "impact_blink_ms": 1200,
        "cue_ms": 1500
      },
      "display": {
        "ascii_ahead": " ))>>",
        "ascii_behind": "((<< ",
        "unicode_ahead": " 🍌 >>",
        "unicode_behind": "<< 🍌 "
      }
    },
    {
      "id": "shield",
      "effect": {
        "duration_ms": 5000
      }
    },
    {
      "id": "focus",
      "effect": {
        "duration_ms": 10000,
        "focus_ai_wpm_boost": 10
      }
    },
    {
      "id": "cyclone",
      "effect": {
        "affected_words": 1,
        "cyclone_stun_ms": 2500
      }
    },
    {
      "id": "squid_ink",
      "effect": {
        "ink_range_words": 5,
        "ink_duration_ms": 5000,
        "ink_impact_blink_ms": 1200,
        "ink_cue_ms": 1500,
        "ink_ai_wpm_multiplier_percent": 70
      }
    }
  ]
}
```

Use an item pack:

```sh
typekart play --item-pack-file ./mods/items/classic-plus.json
typekart host --name host --item-pack-file ./mods/items/classic-plus.json
```

## Item Pack Fields

Top-level:

- `items`: array of item overrides.

Per item:

- `id`: required built-in item id.
- `name`: optional display name.
- `enabled`: optional boolean.
- `standard_weight`: optional flat roll weight for all race positions.
- `nearby_racer_weight`: optional flat roll weight when racers are nearby.
- `context_weights`: optional detailed weights by position and proximity.
- `effect`: optional effect tuning.
- `display`: optional display tuning for supported items.

Detailed `context_weights` shape:

```json
{
  "standard": { "first": 1, "middle": 2, "trailing": 3 },
  "nearby_racer": { "first": 2, "middle": 3, "trailing": 5 }
}
```

Effect fields are item-specific:

- Mushroom: `boost_words`, `wpm`.
- Banana: `range_words`, `stun_ms`, `impact_blink_ms`, `cue_ms`.
- Shield: `duration_ms`.
- Focus: `duration_ms`, `focus_ai_wpm_boost`.
- Cyclone: `affected_words`, `cyclone_stun_ms`.
- Squid Ink: `ink_range_words`, `ink_duration_ms`, `ink_impact_blink_ms`, `ink_cue_ms`, `ink_ai_wpm_multiplier_percent`.

Banana display fields:

- `ascii_ahead`
- `ascii_behind`
- `ascii_overlap`
- `unicode_ahead`
- `unicode_behind`
- `unicode_overlap`

## Validation

TypeKart rejects:

- Unknown item ids.
- Duplicate item ids in the effective registry.
- Empty item registries.
- Item registries with no enabled item that can be rolled.
- Effect fields applied to items that do not support them.
- Zero values for required positive effect fields.
- Word sets with unplayable words.

Use `--debug-log` while testing packs to capture selected mod metadata and item resolution details.

Copyable templates live in `mods/`. The item template mirrors the built-in defaults. The word template is the playable lowercase-only version of the bundled word list so it can pass external word-set validation.

- `mods/words/classic.txt`
- `mods/items/classic.json`
