# Milestone 1 Implementation Plan: Local Typing Prototype

## Goal

Build a single-player local typing prototype in Rust.

This milestone should prove the core typing feel before adding multiplayer, bonus words, items, or the full race renderer. The result should be a terminal program where one player races through a generated list of lowercase words, sees typo feedback, and receives basic stats when finished.

## Scope

Milestone 1 includes:

- Rust project setup.
- A generated lowercase word track.
- A deterministic typing engine.
- Strict typo behavior.
- Space-required word submission.
- Red highlighting from the first typo onward.
- A minimal terminal view.
- Basic race stats.
- Unit tests for typing behavior.

Milestone 1 excludes:

- Multiplayer.
- Hosting and joining.
- Bonus words.
- Items.
- Racer layer rendering.
- Attack warnings.
- Internet play.
- Persistent settings.

## Recommended First Deliverable

The first deliverable should be a CLI command:

```text
cargo run -- play
```

It should:

1. Load or generate a track.
2. Open a terminal typing session.
3. Let the player type words in order.
4. Highlight mistakes.
5. Finish immediately when the final word is completed.
6. Print a small results summary.

## Dependencies

Initial Rust dependencies:

```toml
[dependencies]
anyhow = "1"
clap = { version = "4", features = ["derive"] }
crossterm = "0.27"
rand = "0.8"
ratatui = "0.26"

[dev-dependencies]
pretty_assertions = "1"
```

Notes:

- `ratatui` handles structured terminal rendering.
- `crossterm` handles raw terminal mode and key input.
- `clap` gives a clean command structure for future `host` and `join` commands.
- `anyhow` keeps early application-level error handling simple.
- `rand` is enough for initial track generation.

Versions can be adjusted to current compatible releases when the project is scaffolded.

## Proposed File Structure

```text
Cargo.toml
src/
  main.rs
  app.rs
  game/
    mod.rs
    player.rs
    stats.rs
    track.rs
    typing.rs
  ui/
    mod.rs
    terminal.rs
    render.rs
words_alpha.txt
docs/
  game-design.md
  technical-plan.md
  milestone-1-plan.md
```

## Data Types

### Track

The track is the ordered list of words for the race.

```rust
pub struct Track {
    pub words: Vec<String>,
}
```

Initial behavior:

- Load curated words from `words_alpha.txt`.
- Treat the file as the playable word list.
- Randomly choose a fixed number of words for the race.

Initial recommendation:

- Default race length: 40 words.

### PlayerState

Player state tracks progress through the race.

```rust
pub struct PlayerState {
    pub word_index: usize,
    pub input: String,
    pub typo_index: Option<usize>,
    pub started_at: Option<Instant>,
    pub finished_at: Option<Instant>,
    pub stats: TypingStats,
}
```

Rules:

- `word_index` points at the current target word.
- `input` contains everything typed for the current word, including typo characters.
- `typo_index` is the first incorrect character in `input`, if any.
- Progress cannot advance while `typo_index` is present.

### TypingStats

Stats should track raw player behavior, not just successful progress.

```rust
pub struct TypingStats {
    pub typed_chars: usize,
    pub correct_chars: usize,
    pub typo_chars: usize,
    pub backspaces: usize,
    pub completed_words: usize,
}
```

Initial derived stats:

- Elapsed time.
- Words per minute.
- Raw accuracy.
- Completed word count.
- Backspace count.

## Typing Engine

The typing engine should be pure game logic with no terminal dependencies.

Suggested API:

```rust
pub enum KeyAction {
    Char(char),
    Space,
    Backspace,
}

pub enum TypingEvent {
    InputChanged,
    WordCompleted,
    RaceFinished,
    TypoStarted { index: usize },
    TypoCleared,
}

pub fn apply_key(
    player: &mut PlayerState,
    track: &Track,
    action: KeyAction,
    now: Instant,
) -> Vec<TypingEvent>;
```

This should remain independent from `crossterm`. The UI layer should convert terminal key events into `KeyAction`.

## Typing Rules

Implement these rules exactly:

- Letter keys append to the current input.
- The first incorrect character creates `typo_index`.
- While `typo_index` is present, more characters can be typed but progress is blocked.
- The typo and all later characters are rendered red.
- `Backspace` removes the last input character.
- If backspacing removes the typo span, `typo_index` is recalculated or cleared.
- `Space` submits the current word only when `input` exactly matches the target word.
- Pressing `Space` before the word is complete creates a typo.
- Pressing `Space` with incorrect input keeps the player on the same word.
- After a word is submitted, `input` clears and `word_index` advances.
- Completing the final word sets `finished_at` immediately without requiring a trailing `Space`.

Implementation detail:

- Treat early `Space` as an input character or as a special typo marker internally.
- The renderer must still show that the player needs to backspace to recover.

Recommended first approach:

- Store early Space in the input buffer as `' '`.
- Compare input characters directly against the target word.
- This naturally creates a typo at the first unexpected space.

## Terminal UI

Milestone 1 does not need the final race UI. It only needs enough UI to validate typing.

Recommended layout:

```text
TypeKart                         12/40 words      42 WPM

Track:
the quick brown fox jumps over the lazy driver

Current:
fox

Input:
fo_

Stats:
Accuracy 96%   Backspaces 3   Typos 4
```

Rendering requirements:

- Show a window of upcoming words around the current word.
- Highlight the current word.
- Show the current input.
- Render correct input normally.
- Render the first typo and all following input in red.
- Keep cursor behavior predictable.
- Restore the terminal on exit or panic where reasonable.

Terminal behavior:

- Enter alternate screen on start.
- Enable raw mode on start.
- Disable raw mode on exit.
- Leave alternate screen on exit.
- Support `Esc` or `Ctrl-C` to quit.

## Word Source

The repository has a curated `words_alpha.txt` with about 5,000 playable words.

For Milestone 1:

- Load the file at startup.
- Trim blank lines.
- Cache the word list in memory.
- Randomly sample words for each race.

Validation:

- The implementation does not need runtime filtering.
- A small test can validate that the curated file only contains lowercase ASCII alphabetic words.
- If validation fails, treat it as a word-list data problem rather than silently changing the list at runtime.

Future improvement:

- Split the curated list by difficulty or word length if race tuning needs it.
- Add themed word lists if custom race modes are added.

## Tests

Milestone 1 should include unit tests for the typing engine before adding much UI.

Required tests:

- Typing a correct word advances only after `Space`.
- Typing a correct prefix does not advance before `Space`.
- Pressing `Space` early creates a typo.
- A wrong letter creates `typo_index`.
- Progress is blocked while a typo exists.
- Backspace can clear a typo.
- Backspace after extra typo characters still leaves typo state until the original typo is removed.
- Correctly typing the final word finishes the race without requiring `Space`.
- Accuracy and backspace stats update.

Nice-to-have tests:

- Empty input plus Backspace is harmless.
- Typing after finish does nothing.
- Multiple words advance correctly.
- Typo index recalculates correctly after mixed input and backspaces.

## Implementation Steps

### Step 1: Scaffold Rust Project

Create:

- `Cargo.toml`
- `src/main.rs`
- Basic `clap` command with `play`.
- Basic module structure.

Acceptance criteria:

- `cargo test` passes.
- `cargo run -- play` starts and exits cleanly, even if it only prints a placeholder.

### Step 2: Implement Track Loading

Create `game::track`.

Acceptance criteria:

- Loads `words_alpha.txt`.
- Generates a `Track` of configurable length.
- Has tests for loading and sampling behavior.
- Optionally validates the curated word file in tests.

### Step 3: Implement Typing Engine

Create `game::typing` and `game::player`.

Acceptance criteria:

- All required typing rules are implemented.
- Typing engine has no terminal dependencies.
- Required unit tests pass.

### Step 4: Implement Stats

Create `game::stats`.

Acceptance criteria:

- Tracks typed characters, correct characters, typo characters, backspaces, and completed words.
- Computes elapsed time, WPM, and accuracy.
- Stats tests pass.

### Step 5: Build Minimal Terminal App

Create `ui::terminal` and `ui::render`.

Acceptance criteria:

- App enters raw mode and alternate screen.
- App reads key events.
- App converts key events into typing actions.
- App renders track, current word, input, and stats.
- Typo span renders red.
- `Esc` and `Ctrl-C` exit cleanly.

### Step 6: Finish Flow

Acceptance criteria:

- Completing the last word exits the active race view or shows a result screen.
- Results include elapsed time, WPM, accuracy, typo count, and backspaces.
- Terminal is restored after completion.

## Acceptance Criteria For Milestone 1

Milestone 1 is complete when:

- `cargo run -- play` starts a local single-player race.
- The player can complete a generated 40-word track.
- Non-final words only advance after correct input followed by `Space`.
- The final word finishes immediately when completed.
- Early `Space` behaves as a typo.
- Typos block progress until corrected with `Backspace`.
- Typo spans are highlighted red.
- Results are shown at the end.
- `cargo test` passes.

## Risks And Decisions To Watch

### Early Space Representation

Representing early Space as a literal input character is simple, but the visual display needs to make it clear. If it looks confusing, use a visible symbol such as `␠` in the input display while keeping the internal representation as space.

### Terminal Cleanup

Raw mode bugs are annoying. Build terminal setup and teardown carefully so crashes or exits do not leave the terminal in a broken state.

### UI Scope

Do not overbuild the UI in Milestone 1. The final racer layer and bonus display come later. This milestone is about typing correctness and feel.

## Suggested Next Step

After this plan is accepted, scaffold the Rust project and implement Step 1 through Step 3 before spending much time on rendering polish.
