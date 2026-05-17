# Milestone 5 Plan: Word Set Modding

## Goal

Allow hosts to bring custom word sets for themed or difficulty-specific races.

This should be implemented alongside the item-modding groundwork as one of the first two supported mod surfaces.

## Why Separate From Items

Word sets are content. Items are behavior.

Word set modding mainly needs:

- File loading.
- Validation.
- Selection by id/name.
- Track generation.
- Host metadata.

Item modding also needs targeting, effects, timers, rendering cues, server authority, and protocol compatibility. Keeping the plans separate lets us implement both surfaces without mixing their responsibilities.

## Current State

The app currently loads `words_alpha.txt` directly:

```text
WordList::load("words_alpha.txt")
Track::generate(&word_list, word_count)
```

The curated word list is assumed valid enough for gameplay. Runtime still trims blank lines and can validate shape in tests.

## Target User Experience

Possible future commands:

```sh
cargo run -- play --word-set classic --words 40
cargo run -- play --word-set-file ./mods/animals.txt --words 40
cargo run -- play --word-set-dir ./mods/word-packs --words 40
cargo run -- host-lan --word-set-file ./mods/technical.txt --words 60
cargo run -- host-lan --word-set-dir ./mods/word-packs --words 60
```

For local-network play, the host chooses the word set. Joiners do not need the source file because the server sends generated track words in race snapshots.

## Word Set Collections

The first collection format is a directory of `.txt` word sets:

```text
mods/
  word-packs/
    animals.txt
    programming.txt
    hard.txt
```

When the host starts with `--word-set-dir`, TypeKart loads and validates every `.txt` file in that directory, then randomly selects one for the race. This gives us the early version of "swap word packs between races" without needing lobby UI yet.

Longer term, this should become a manifest-backed word pack:

```json
{
  "id": "party-pack",
  "name": "Party Pack",
  "selection": "shuffle_bag",
  "word_sets": [
    { "id": "animals", "file": "animals.txt", "weight": 3 },
    { "id": "programming", "file": "programming.txt", "weight": 1 },
    { "id": "hard", "file": "hard.txt", "weight": 2 }
  ]
}
```

Useful future selection modes:

- Random each race.
- Weighted random.
- Rotate in order.
- Shuffle bag with no repeats until exhausted.
- Host selection from the lobby.
- Vote among a few candidates.

## Word Set Definition

Conceptual metadata:

```toml
id = "technical"
name = "Technical Terms"
language = "en"
description = "Programming and systems vocabulary"
min_typekart_version = "0.5.0"

[rules]
lowercase_ascii_only = true
allow_duplicates_in_track = true
min_word_length = 2
max_word_length = 12
```

The first external version can be simpler:

```text
one lowercase word per line
comments start with #
blank lines ignored
```

## Validation Rules

At load time:

- Reject empty word sets.
- Reject words with spaces.
- Reject punctuation unless the selected race mode allows it.
- Reject uppercase unless a future case-sensitive mode allows it.
- Enforce a minimum number of unique words.
- Enforce optional min/max word length.
- Report the first several invalid lines with line numbers.

Keep validation strict by default so typing rules stay predictable.

## Multiplayer Behavior

The host owns:

- Selected word set id/name/hash.
- Track generation.
- Final generated track words.

The client receives:

- Generated track words in snapshots.
- Word set metadata in lobby/race metadata.

Useful display text:

```text
Word set: Technical Terms (host custom)
Words: 60
```

## Proposed Modules

```text
src/game/words/
  mod.rs
  set.rs
  registry.rs
  validation.rs
```

Potential types:

```rust
pub struct WordSetId(String);

pub struct WordSetDefinition {
    pub id: WordSetId,
    pub name: String,
    pub source: WordSetSource,
    pub words: WordList,
    pub metadata: WordSetMetadata,
}

pub struct WordSetRegistry {
    pub sets: Vec<WordSetDefinition>,
}
```

This can eventually replace direct `WordList::load("words_alpha.txt")` calls in `app.rs`.

## Refactor Sequence

1. Done: Add shared mod groundwork with content-id validation helpers.
2. Done: Add `WordSetDefinition` around the current built-in `words_alpha.txt`.
3. Done: Add `WordSetRegistry::builtin()`.
4. Done: Change `app::play` and `app::host` setup to choose a word set through the registry.
5. Done: Add CLI flags for selecting a built-in word set.
6. Done: Add `--word-set-file` for local and host play.
7. Done: Add `--word-set-dir` for local and host play, backed by a `WordSetCollection`.
8. Done: Add validation error reporting with line numbers.
9. Done: Include selected word set id/name/hash in race snapshot mod metadata.
10. Add richer metadata-file support for named custom word sets and word-set collections.

## Tests

- Built-in default word set loads.
- Empty file is rejected.
- Invalid characters are rejected with line numbers.
- Word-set directories load `.txt` files only.
- Word-set directories reject empty collections.
- Track generation uses the selected set.
- Host lobby and race snapshots include selected word set id/hash.
- Joiners can race without the source word file.

## Non-Goals

- Punctuation race modes.
- Multi-language typing rules.
- Client-side word set authority.
- Runtime word set switching during a lobby UI.

## Recommendation

Implement word set modding alongside the item registry groundwork.

It is lower risk than item behavior and will validate the shared mod manifest/hash path with a second concrete content type.

## Implementation Status

The first word-set modding slice is implemented:

- `play` and `host` support `--word-set classic`.
- `play` and `host` support `--word-set-file ./path/to/words.txt`.
- `play` and `host` support `--word-set-dir ./path/to/word-packs`, which picks one `.txt` word set at random.
- Lobby and race snapshots include the selected word set id, name, and stable content hash.
- The network UI displays the selected word set before and during the race.
- Debug logs include the active mod summary, including the selected word set hash.
- The host remains authoritative in multiplayer; joiners do not need the source word-set file.
- Custom word files are validated as lowercase ASCII words with enough unique entries.
- The built-in `classic` word set keeps the existing trusted-load behavior so the refactor does not unexpectedly reject curated repository data.

The remaining work is optional metadata files for custom sets and manifest-backed collection selection modes.
