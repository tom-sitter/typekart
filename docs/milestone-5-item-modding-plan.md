# Milestone 5 Plan: Moddable Item System

## Goal

Refactor TypeKart items so new items can be added, removed, tuned, or disabled without touching the local session, network server, renderer, and bonus roller in several separate places.

The first version should support built-in items through data-backed definitions and shared effect handlers. In the same milestone, word sets should also become a moddable content surface. Later, the item shape can load user-provided item packs from configuration files.

Items and word sets are the first two modding surfaces planned for Milestone 5 implementation. The shared manifest/registry/hash groundwork should be generic enough for later rules, themes, AI profiles, and game modes. See `docs/modding-architecture-plan.md` and `docs/milestone-5-word-set-modding-plan.md`.

## Current Problem

Items are currently represented by Rust enums and hard-coded match statements:

- `HeldItem` contains `Mushroom` and `Banana`.
- `ItemPickup` special-cases `Shield`.
- Item roll tables are fixed arrays in `src/game/items.rs`.
- Local item behavior lives mostly in `src/ui/session.rs`.
- Network item behavior lives separately in `src/net/server.rs`.
- Render cues are represented separately in local and network renderers.

This makes every new item a cross-cutting change. Adding one item means changing the item enum, roll tables, pickup handling, local activation, network activation, snapshot fields, renderer cues, event text, and tests.

## Design Direction

Move from item-specific enums as the primary model to item ids plus definitions:

```text
ItemId("mushroom")
ItemId("banana")
ItemId("shield")
```

Each item should have a definition that describes:

- Display name.
- Roll weight.
- Whether it activates immediately or can be held.
- Targeting rule.
- Effect script/handler id.
- Timing values.
- Display cue.
- Blocking/defense tags.

Example conceptual definition:

```text
id: banana
name: Banana
activation: immediate
targeting: nearest_racer
range_words: 10
effect: stun_and_clear_input
stun_ms: 2000
impact_blink_ms: 1200
cue: banana_direction
blocked_by: shield
```

This does not need to be external TOML/JSON immediately. The first step can be Rust structs created in code, because that still removes most cross-module branching and gives us a stable API before exposing files to users.

## Proposed Modules

Add or reshape these modules:

```text
src/game/items/
  mod.rs
  definition.rs
  registry.rs
  roll.rs
  targeting.rs
  effects.rs
```

`definition.rs`

- Owns `ItemDefinition`, `ItemId`, `ActivationMode`, `TargetingRule`, `EffectKind`, and display metadata.
- Should not know about local UI, network sockets, or Ratatui.

`registry.rs`

- Owns `ItemRegistry`.
- Provides built-in item definitions.
- Later loads external item packs.

`roll.rs`

- Rolls an `ItemId` from the registry using weighted tables.
- Supports context-aware weights, such as nearby racers or player placement.

`targeting.rs`

- Converts an item targeting rule into target candidates.
- Replaces Banana-only helper naming with general target selection helpers.

`effects.rs`

- Applies effect handlers to shared race state.
- Handles Mushroom, Banana, Shield, and future effects through a common interface.

## Core Types

Target shape:

```rust
pub struct ItemDefinition {
    pub id: ItemId,
    pub name: String,
    pub activation: ActivationMode,
    pub targeting: TargetingRule,
    pub effect: EffectKind,
    pub timing: ItemTiming,
    pub cue: Option<ItemCueDefinition>,
    pub tags: Vec<ItemTag>,
}

pub enum ActivationMode {
    Immediate,
    Held,
}

pub enum TargetingRule {
    SelfOnly,
    NearestRacer { range_words: usize },
    FirstPlace,
    AllRacersInRange { range_words: usize },
}

pub enum EffectKind {
    MushroomBoost { words: usize, wpm: f64 },
    Shield { duration_ms: u64 },
    StunAndClearInput { stun_ms: u64, impact_blink_ms: u64 },
}
```

These enums are still Rust code, but they are less item-specific. Adding a new item can often reuse an existing `TargetingRule` and `EffectKind`.

## Shared Item Engine

Create one server-authoritative item engine that both local play and network play can call:

```text
ItemEngine::roll_pickup(context) -> ItemId
ItemEngine::activate_item(race_state, actor_id, item_id, now) -> ItemResolution
ItemEngine::tick(race_state, now) -> Vec<ItemEvent>
```

The engine should return structured events:

```text
ItemEvent::PickedUp { player, item }
ItemEvent::Activated { player, item }
ItemEvent::Missed { player, item }
ItemEvent::Hit { attacker, target, item }
ItemEvent::Blocked { attacker, target, item, blocker }
ItemEvent::EffectExpired { player, effect }
```

Local and network code should turn these events into logs, event-feed messages, and snapshots. The item engine should not write UI strings directly.

## Display Model

Move item display from item-specific renderer branches toward generic cues:

```text
EffectCue {
  icon_ascii: ">>>",
  icon_unicode: ">>🍄",
  placement: before_marker
}

AttackCue {
  icon_ascii_ahead: " ))>>",
  icon_unicode_ahead: " 🍌 >>",
  placement: before_or_after_marker
}
```

Snapshots should expose display state, not internal effect implementation:

```text
PlayerSnapshot {
  active_effects: [EffectSnapshot],
  item_cues: [ItemCueSnapshot],
}
```

This avoids adding a new protocol field for every future item.

## External Modding Path

Do this in two stages.

### Stage 1: Built-In Registry

Keep item definitions in Rust code:

```rust
ItemRegistry::builtin()
```

This gives us the modding-shaped architecture while staying type-safe and easy to test.

### Stage 2: Configurable Item Packs

Load definitions from `items/*.toml`, `items/*.json`, or one item-pack file.

Only allow data-backed composition at first:

- Reuse known `TargetingRule` values.
- Reuse known `EffectKind` values.
- Reuse known cue layouts.
- Tune durations, ranges, weights, and names.

Do not load arbitrary code or scripts. Arbitrary code mods create security and distribution problems, especially for multiplayer. A host-loaded item pack is enough for early modding.

Example:

```json
{
  "items": [
    {
      "id": "banana",
      "enabled": false
    },
    {
      "id": "mushroom",
      "standard_weight": 6,
      "nearby_racer_weight": 8,
      "effect": {
        "boost_words": 4,
        "wpm": 220
      }
    },
    {
      "id": "banana",
      "effect": {
        "range_words": 8,
        "stun_ms": 1500,
        "impact_blink_ms": 900,
        "cue_ms": 1200
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
        "duration_ms": 3000
      },
      "context_weights": {
        "standard": { "first": 1, "middle": 1, "trailing": 1 },
        "nearby_racer": { "first": 4, "middle": 3, "trailing": 2 }
      }
    }
  ]
}
```

`standard_weight` and `nearby_racer_weight` are backwards-compatible shorthand for flat context tables. Full `context_weights` gives pack authors direct control over each first/middle/trailing race-position band in both normal and nearby-racer contexts.

The current `effect` fields tune existing built-in handlers only: Mushroom boost words/WPM, Banana range/stun/blink/cue timing, and Shield duration. Banana `display` fields can override the visible attack cue labels while leaving omitted labels at their built-in defaults.

## Multiplayer Compatibility

The host should own the active item registry for a race.

Joiners should receive either:

- The registry hash, if they only need built-in definitions.
- The full item display metadata, if custom item packs are active.

The server remains authoritative for:

- Item rolls.
- Target selection.
- Effect resolution.
- Timers.
- Blocks and misses.

Clients should only render snapshot state.

## Refactor Sequence

1. Done: Introduce shared mod groundwork with reusable content-id and metadata helpers.
2. Done: Introduce word set registry groundwork in parallel, so the mod shape has at least two real content surfaces.
3. Done: Introduce `ItemDefinition` and `ItemRegistry` while preserving current `HeldItem`/`ItemPickup` behavior.
4. Done: Replace hard-coded roll tables with registry-driven weighted rolls.
5. Move item activation into a shared item engine used by the network server first.
6. Migrate local `LocalSession` to call the same item engine.
7. Replace item-specific snapshot fields with generic effect/cue snapshots.
8. Replace renderer-specific item branches with generic cue rendering.
9. Done, first slice: Add optional JSON loading for host-defined item packs that tune/disable built-in items.
10. Partially done: Add active item registry hash metadata to race snapshots and debug logs. Compatibility checks are still future work.

## Testing Strategy

Unit tests:

- Registry rejects duplicate item ids.
- Roll table ignores disabled items.
- Roll table respects weights.
- Each targeting rule selects the intended targets.
- Each effect kind mutates shared race state correctly.
- Shield-like blocking works by tag, not by item name.

Integration tests:

- Built-in Mushroom, Banana, and Shield still behave exactly as they do now.
- Network snapshots expose generic effect/cue state.
- Local and network item activation produce the same structured events.
- A custom item pack can disable Banana.
- A custom item pack can add a one-word Mushroom variant without adding Rust branches.

## Non-Goals For First Modding Pass

- Arbitrary scripting.
- Client-side item authority.
- Downloading item packs from the internet.
- Mid-race item pack changes.
- User-created terminal rendering code.

## Recommendation

Start Milestone 5 with shared mod groundwork, a word set registry, and Stage 1 of the item system: a built-in registry and shared item engine.

That gives us most of the architectural win immediately, reduces local/network duplication, and validates the mod pack shape against both content and behavior. External TOML item packs should come after the built-in registry proves the model.

## Implementation Status

The first implementation slice is in place:

- `src/game/mods.rs` owns shared content ids and content metadata.
- `src/game/items.rs` now uses `ItemDefinition` and `ItemRegistry::builtin()` for Mushroom, Banana, and Shield weights.
- Existing public item enums remain in place so local play, network play, and render code keep their current behavior.
- `play` and `host` support `--item-pack-file ./path/to/items.json`.
- Custom item packs can currently change built-in item names, enabled flags, shorthand standard/nearby roll weights, full first/middle/trailing context weight tables, built-in effect parameters, and Banana attack cue labels.
- Lobby and race snapshots include the active item pack name and effective item registry hash.
- The network UI displays the active item pack before and during the race.
- Debug logs include the active item registry hash and combined mod hash.

This slice intentionally rejects unknown item ids. Adding new item effects still needs the larger behavior refactor: a shared item engine, generic effect/cue snapshots, richer external item-pack schema, and multiplayer compatibility checks.
