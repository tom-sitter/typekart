# Modding Architecture Plan

## Goal

Make TypeKart easier to extend without turning the game into an unsafe plugin host.

The near-term goal is item and word set modding. The architecture should also leave clean room for race rules, visual themes, AI profiles, and game modes.

## Guiding Principles

- The host owns the active mod configuration for a race.
- The server remains authoritative for rules, rolls, targeting, timers, and results.
- Clients render server snapshots and display mod metadata.
- Mods should be data-first, not arbitrary executable code.
- Built-in content should use the same loading/registry path as external content.
- Each mod surface should have explicit validation and compatibility checks.
- A race should be reproducible from track words, mod manifest metadata, item/rule configuration, and event logs.

## Mod Surfaces

TypeKart has several mod-worthy areas:

- Items: definitions, weights, effects, targeting rules, display cues.
- Word sets: curated word files, themed pools, difficulty tiers.
- Race rules: word count, bonus spacing, cooldowns, finish timeout, player caps.
- Bonus rules: number of choices, placement intervals, claim/cooldown behavior.
- Visual themes: colors, racer markers, minimap symbols, item cue glyphs.
- AI profiles: names, WPM ranges, item aggression, error behavior later.
- Game modes: no-items, sprint, endurance, hard words, chaos, catch-up-heavy items.
- Event text: item messages, race messages, results labels.

Do not make these moddable early:

- Network protocol semantics.
- Terminal key capture behavior.
- Arbitrary scripts or dynamic code.
- Client-side authoritative rule logic.
- Mid-race mod changes.

## Shared Mod Pack Shape

Use one manifest concept even if each mod surface is implemented separately.

Conceptual structure:

```toml
id = "classic"
name = "Classic TypeKart"
version = "1.0.0"
typekart_compat = ">=0.5.0"

[content]
items = "items.toml"
word_sets = ["words/basic.txt", "words/advanced.txt"]
rules = "rules.toml"
theme = "theme.toml"
ai_profiles = "ai.toml"
```

The first implementation can keep this in Rust structs without loading external files. The important part is establishing boundaries:

- `ModPackManifest`: metadata and enabled content modules.
- `ModPackId`: stable id for logs and compatibility.
- `ModPackHash`: deterministic hash of the effective config.
- `ContentRegistry`: validated built-ins plus optional host-provided content.

## Host And Client Responsibilities

Host:

- Loads and validates the selected mod pack.
- Builds registries for items, word sets, rules, themes, and AI profiles.
- Generates the race track.
- Owns authoritative item/rule resolution.
- Broadcasts mod metadata and compatibility hashes.

Client:

- Receives mod metadata.
- Receives generated track words through snapshots.
- Renders server-provided item/effect/race state.
- Warns if a future client-local display pack is missing or incompatible.

For local-network play, clients do not need the host's word files or item definitions to play if the server sends enough display metadata in snapshots.

## Proposed Shared Modules

Add a future `game::mods` area:

```text
src/game/mods/
  mod.rs
  manifest.rs
  registry.rs
  validation.rs
  hash.rs
```

Responsibilities:

- Represent active mod metadata.
- Validate ids, names, versions, and references.
- Produce a stable compatibility hash.
- Hold registries shared by item, word, rule, theme, and AI systems.

Do not make `game::mods` depend on Ratatui, sockets, local terminal sessions, or file watching.

## First Groundwork To Add During Item Modding

When implementing item and word set modding, build these pieces in a general way:

- Done: `ContentId` conventions shared by items, word sets, and future themes/rules.
- Done: `ActiveModConfig` metadata for the selected word set and effective item registry.
- Done: Stable hashes for selected word-set contents, item registry contents, and the combined active mod config.
- Done: Lobby and race snapshots expose mod metadata for connected clients.
- Done: Network UI displays the active word set, item pack, and short combined mod hash.
- Done: Local and network debug logs include the active mod summary.
- Later: `ModPackManifest` with built-in default metadata.
- `ContentRegistry` validation helpers for duplicate ids and unknown references.
- Later: richer lobby metadata and compatibility checks before race start.

This keeps future word packs and rules packs from needing a second compatibility mechanism.

## Compatibility Strategy

Use a layered compatibility model:

- Protocol version: can this client understand the server's snapshot format?
- Mod schema version: can this binary parse the mod configuration?
- Mod pack hash: is this the exact active host configuration?
- Display metadata version: can this client render the configured symbols/cues?

For now, the server can expose pack metadata in lobby/race snapshots as informational. Later, joiners can reject incompatible hosts or show a clear warning.

## File Loading Strategy

Suggested stages:

1. Built-in registries only.
2. Optional local files loaded by the host. First implemented examples are `--word-set-file` for custom word lists, `--word-set-dir` for random selection from a directory of word sets, and `--item-pack-file` for JSON item packs that tune built-in items.
3. Done for current built-ins/tuning: Host sends metadata to clients and the network UI displays it.
4. Optional client-side local display packs.

Avoid auto-downloading files or executing scripts.

## Documentation Set

- `docs/modding-architecture-plan.md`: shared architecture and compatibility model.
- `docs/milestone-5-item-modding-plan.md`: first-slice item refactor.
- `docs/milestone-5-word-set-modding-plan.md`: first-slice custom word set plan.
- `docs/milestone-5-rules-and-theme-modding-plan.md`: future rules, themes, AI profiles, and game modes.

## Recommendation

Implement the shared manifest/registry/hash groundwork during the item and word set refactors, and expose both as the first supported mod surfaces.

Leave rules, themes, AI profiles, game modes, and event text as documented extension points until the foundations are proven.
