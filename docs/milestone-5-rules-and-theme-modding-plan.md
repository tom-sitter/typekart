# Milestone 5 Plan: Rules, Themes, AI, And Game Mode Modding

## Goal

Plan future mod surfaces that are not part of the immediate item and word set refactors, while shaping the shared mod groundwork so these features fit later.

## Scope Groups

This document covers:

- Race rules.
- Bonus rules.
- Visual themes.
- AI profiles.
- Game modes.
- Event text.

These should not be implemented in the first modding slice unless a field is needed to support item definitions, word set metadata, or shared mod pack compatibility cleanly.

## Race Rule Mods

Potential fields:

```toml
[race]
default_words = 40
max_players = 6
post_first_finish_timeout_ms = 30000
countdown_seconds = 3
```

Rules should be host-owned in multiplayer. Clients can display them but should not enforce them.

Implementation plan:

1. Introduce `RaceRules` with current constants as defaults.
2. Replace scattered constants with reads from `RaceRules`.
3. Include `RaceRules` in active mod metadata/hash.
4. Add CLI override flags only after the struct exists.
5. Add external config loading later.

## Bonus Rule Mods

Potential fields:

```toml
[bonus]
interval_words = 8
choice_count = 3
cooldown_ms = 4000
visible_ahead = true
```

Implementation plan:

1. Add `BonusRules` around interval, choice count, cooldown, and availability behavior.
2. Update `BonusState::generate` and cooldown handling to read rules.
3. Keep `choice_count = 3` fixed until rendering and protocol are generalized.
4. Later allow different choice counts after snapshot/UI support is ready.

## Theme Mods

Potential fields:

```toml
[theme]
local_marker_ascii = "███"
boost_ascii = ">>>"
boost_unicode = ">>🍄"
banana_ahead_ascii = " ))>>"
banana_ahead_unicode = " 🍌 >>"
minimap_empty = "-"
minimap_overlap = "*"
```

Themes should only affect display. They should not change game rules.

Implementation plan:

1. Define a `ThemeDefinition` with current symbols/colors as built-ins.
2. Make local and network renderers read cue labels from shared display metadata.
3. Keep color choices internal at first if terminal portability becomes complicated.
4. Add external theme files after item and word set mod loading exists.

## AI Profile Mods

Potential fields:

```toml
[[ai_profiles]]
id = "easy"
name_pool = ["ai-1", "ai-2"]
wpm_min = 25
wpm_max = 55
item_aggression = 0.4
bonus_interest = 0.6
```

Implementation plan:

1. Move WPM ranges from `AiDifficulty` into `AiProfile`.
2. Keep `Easy` and `Hard` as built-in profile ids.
3. Add optional name pools.
4. Later add behavior tuning when AI behavior has more knobs.

## Game Mode Mods

Game modes should compose existing rule groups rather than inventing a second rule system.

Examples:

- `classic`: default items and bonus rules.
- `no_items`: disables bonus/item systems.
- `sprint`: short track and shorter finish timeout.
- `chaos`: more frequent bonus points and higher item weights.
- `hard_words`: selects a harder word set and longer track.

Implementation plan:

1. Define `GameModeDefinition` as references to rules, item table, word set, and optional theme.
2. Keep built-in modes only.
3. Add CLI selection after rules/word registries exist.

## Event Text Mods

Potential fields:

```toml
[events]
banana_hit = "{attacker} hit {target}"
banana_blocked = "{target} blocked {item}"
race_finished = "Race finished"
```

Implementation plan:

1. Stop emitting final display strings from item/race engines.
2. Emit structured events.
3. Format events at the UI/log boundary.
4. Add event text overrides later.

## Shared Groundwork Needed Now

The item refactor should avoid item-only names for shared infrastructure:

- Prefer `ModPackManifest` over `ItemPackManifest`.
- Prefer `ContentRegistry` over `ItemRegistryRoot`.
- Prefer `ContentId` patterns that work for items, word sets, themes, rules, and profiles.
- Include active pack metadata in networking in a general field.

## Non-Goals For Milestone 5 First Slice

- External rules files.
- External theme files.
- External AI behavior packs.
- User scripts.
- Client-side rule enforcement.

## Recommendation

Design the shared manifest and registry types now, implement item and word set modding first, and leave these other surfaces as documented extension points.

That gives future modding a clear path without making the first modding slice cover every configurable part of the game.
