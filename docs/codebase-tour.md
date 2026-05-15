# TypeKart Codebase Tour

## Purpose

This document explains how the current Rust code is organized and why it is shaped this way. It is meant to help you learn Rust while navigating the project.

The implementation is intentionally split into two broad areas:

- `game`: pure game rules and state.
- `ui`: terminal input and rendering.

That separation matters. The game rules should be testable without a terminal, networking, or real-time rendering. The UI should display state and translate key presses into game actions, not decide game rules.

## Project Entry Points

### `src/main.rs`

`main.rs` is the executable entry point.

It uses `clap` to parse command-line arguments:

```text
typekart play --words 40
```

Useful local-play flags include `--ai-racers`, `--ai-difficulty`, `--unicode-icons`, and `--debug-log <PATH>`.

The important Rust idea here is that `main` returns `anyhow::Result<()>`. That lets us use the `?` operator inside the call chain and let errors bubble up cleanly instead of manually matching every error.

Current flow:

```text
main
  parse CLI
  match command
  app::play(settings)
```

### `src/app.rs`

`app.rs` is the small coordinator between the CLI and the actual game.

It:

1. Loads `words_alpha.txt`.
2. Generates a `Track`.
3. Creates a `PlayerState`.
4. Starts the terminal session.

This file should stay thin. As the project grows, it can coordinate host/join/play commands, but it should not become the place where game rules live.

## Game Modules

### `src/game/mod.rs`

This file declares the game submodules:

```rust
pub mod player;
pub mod stats;
pub mod track;
pub mod typing;
pub mod bonus;
pub mod ai;
pub mod effects;
pub mod items;
```

In Rust, a module needs to be declared before other parts of the crate can use it. `pub mod` means the module is visible to parent modules such as `app` and `ui`.

### `src/game/track.rs`

This module owns word-list loading and generated race tracks.

Main types:

- `WordList`: the curated source list loaded from `words_alpha.txt`.
- `Track`: the generated race sequence for a single race.

Important Rust concepts:

- `impl Track` defines methods attached to the `Track` type.
- `Result<Self>` means a function either returns the type being implemented or an error.
- `impl AsRef<Path>` lets callers pass flexible path-like values, such as `&str` or `PathBuf`.
- `bail!` returns early with an error.

The word list is treated as curated data. Runtime code trims blank lines and samples from the list. Tests can validate that the curated file shape is sane, but normal gameplay should not silently filter words.

### `src/game/bonus.rs`

This module owns bonus points, bonus choices, cooldowns, and bonus item rolls.

Main types:

- `BonusState`: all bonus points for the current race.
- `BonusPoint`: one item-box gap after a specific track word.
- `BonusChoice`: one visible bonus word at a bonus point.
- `BonusChoiceStatus`: whether a choice is available or cooling down.

Important Rust concepts:

- Arrays like `[BonusChoice; 3]` are fixed-size collections known at compile time.
- `Duration` and `Instant` model cooldowns without relying on wall-clock dates.
- `Option<(usize, &BonusPoint)>` returns both the index and a borrowed point when a matching bonus gap exists.
- `#[cfg(test)]` helpers such as `with_points` exist only for tests and are not compiled into normal builds.

The bonus module does not decide how typed input is interpreted. It models the bonus state. `ui::session` coordinates whether a player is currently attempting a bonus word.

### `src/game/ai.rs`

This module defines local AI difficulty.

Main type:

- `AiDifficulty`: currently `Easy` or `Hard`.

Important Rust concepts:

- Small enums can own behavior through methods such as `wpm_range`.
- Keeping difficulty here avoids scattering magic WPM values through the UI.

AI movement itself lives in `ui::session` for now because these racers are local stand-ins for future remote clients. Each AI racer samples one WPM value from its difficulty range when the race starts.

### `src/game/items.rs`

This module defines item types and item-specific helper rules.

Main types:

- `HeldItem`: items that occupy the held-item slot, currently Mushroom or Banana.
- `ItemPickup`: either a held item or an immediate Shield pickup.
- `ItemUse`: normal or modified item activation.
- `RacerPosition`: a small helper for target-selection tests.

Important Rust concepts:

- Enums are a good fit for closed sets of game states.
- `match` over an enum makes each item behavior explicit.
- Small pure helper functions, such as `select_nearest_banana_target`, are easy to unit test before multiplayer exists.

Banana target selection is already represented as reusable game logic. With automatic activation, it chooses the nearest valid racer within range rather than asking for a direction.

Shield is intentionally not a `HeldItem`. When rolled from a bonus word, it activates immediately as an `ActiveEffect`.

### `src/game/effects.rs`

This module defines timed active effects and pending attacks.

Main types:

- `ActiveEffect`: currently only Shield.
- `PendingAttack`: currently only the planned Banana word swap.
- `AttackWarning`: an attack plus the time when it resolves.

Shield is represented as an active effect with an expiration `Instant`. The UI can ask `PlayerState` whether Shield is active and render the racer marker as `[███]` in ASCII mode or as `█🛡` in Unicode icon mode. When the effect expires, it is consumed automatically.

### `src/game/player.rs`

This module defines the local player's race state.

Main type:

```rust
pub struct PlayerState {
    pub word_index: usize,
    pub input: String,
    pub typo_index: Option<usize>,
    pub started_at: Instant,
    pub finished_at: Option<Instant>,
    pub stats: TypingStats,
}
```

Important Rust concepts:

- `usize` is the standard index type for vectors and strings.
- `Option<T>` means a value may or may not exist.
- `Option<usize>` is used for `typo_index` because there may be no typo.
- `Instant` is used for elapsed-time measurement.

The current code stores fields as `pub` for simplicity while the model is small. Later, we may hide fields behind methods if invariants become harder to protect.

### `src/game/stats.rs`

This module tracks raw typing stats.

Main type:

```rust
pub struct TypingStats {
    pub typed_chars: usize,
    pub correct_chars: usize,
    pub typo_chars: usize,
    pub backspaces: usize,
    pub completed_words: usize,
}
```

Important Rust concepts:

- `#[derive(Debug, Clone, Default, PartialEq, Eq)]` asks the compiler to generate common trait implementations.
- `Default` lets us create empty stats with `TypingStats::default()`.
- Methods that only read state take `&self`.

WPM uses the common typing convention of five characters per word.

### `src/game/typing.rs`

This is the most important module in Milestone 1. It contains the deterministic typing engine.

Main types:

- `KeyAction`: the game-level input actions the engine understands.
- `TypingEvent`: events emitted by the engine after applying input.

The key design choice is that `typing.rs` does not know about `crossterm`, terminals, colors, or rendering. It only knows about player state, tracks, and game actions.

Current flow:

```text
apply_key
  ignore input if player is already finished
  find current target word
  dispatch to:
    apply_char
    apply_space
    apply_backspace
```

The function signature is:

```rust
pub fn apply_key(
    player: &mut PlayerState,
    track: &Track,
    action: KeyAction,
    now: Instant,
) -> Vec<TypingEvent>
```

Important Rust concepts:

- `&mut PlayerState` means the function can mutate the player.
- `&Track` means the function can read the track but cannot mutate it.
- `Vec<TypingEvent>` returns zero or more resulting events.
- Passing `now: Instant` from the caller makes the function easier to test than calling `Instant::now()` internally everywhere.

Typo behavior:

- The first incorrect character sets `typo_index`.
- While `typo_index` exists, typed characters are counted but progress is blocked.
- Backspace recalculates the first typo.
- Early Space is stored as a literal space in the input buffer.
- The renderer displays that space as `␠` so the mistake is visible.
- The final word is a special case: once its last correct character is typed,
  the race finishes immediately without requiring a trailing Space.

Tests live in the same file under:

```rust
#[cfg(test)]
mod tests { ... }
```

This is common in Rust. Unit tests can access private helper functions in the same module, which makes it practical to test implementation details when useful.

## UI Modules

### `src/ui/mod.rs`

This declares the UI submodules:

```rust
pub mod render;
pub mod session;
pub mod terminal;
```

### `src/ui/session.rs`

This module owns local session state for the terminal prototype.

Main types:

- `LocalSession`: holds the current `Track`, local `PlayerState`, AI racers, `BonusState`, optional bonus attempt, optional attack warning, and `EventLog`.
- `AiRacer`: local simulated racer state used to pressure-test multiplayer display and item behavior.
- `LocalAction`: terminal-level actions such as typing, normal item use, and modified item use.
- `BonusAttempt`: the bonus point and choice currently being typed.
- `EventLog`: stores recent display-facing race events.

Important Rust concepts:

- `VecDeque<String>` is a double-ended queue. We use it so old event entries can be removed from the front while new entries are pushed onto the back.
- `impl Into<String>` lets `EventLog::push` accept either `&str` or `String`.
- `impl DoubleEndedIterator<Item = &str>` returns an iterator without exposing the internal `VecDeque`.

`LocalSession::apply_action` is now the bridge from input to game state:

```text
LocalAction
  typing may become a bonus attempt or main-track input
  item actions activate held items
  returned TypingEvent values become EventLog messages
```

`LocalSession::tick` handles time-based behavior:

```text
tick
  refresh expired bonus cooldowns
  advance AI typing budgets
  let AI racers claim and use items
  advance active Mushroom boosts
  expire active Shield
  resolve pending attack warnings
```

This keeps event display text and local coordination in the UI layer while preserving reusable game-rule helpers under `game`.

`LocalSession::restart` rebuilds the race in place:

```text
restart
  generate a fresh Track from the stored WordList
  create a new PlayerState
  regenerate bonus points for the new track
  clear transient item, warning, input, and event state
  wait for host Space before starting the countdown
```

The session stores the loaded word list and original word count so the terminal loop can restart without returning to `app::play`.

### `src/ui/terminal.rs`

This module owns terminal setup, teardown, input polling, and the app loop.

Responsibilities:

- Enable raw mode.
- Enter the alternate screen.
- Poll for keyboard events.
- Convert terminal key events into `LocalAction`.
- Apply actions through `LocalSession`.
- Ask the renderer to draw the current state.
- Restore the terminal on exit.

`Ctrl-R` maps to `LocalAction::Restart`. It is a control chord rather than a plain `r` so normal typing still works for words containing that letter.

Important Rust concepts:

- `type AppTerminal = Terminal<CrosstermBackend<Stdout>>` creates a local type alias for a long concrete type.
- `?` propagates errors from terminal setup, drawing, and event reading.
- `let Event::Key(key_event) = event::read()? else { continue; };` is pattern matching. It says: if the event is a key event, bind it; otherwise continue the loop.

Terminal raw mode is important because normal terminals buffer input until Enter. Raw mode lets the app receive individual key presses.

The alternate screen is the full-screen terminal buffer many terminal apps use. When the app exits, the terminal returns to the previous shell screen.

### `src/ui/render.rs`

This module translates game state into terminal widgets.

Responsibilities:

- Lay out the screen.
- Render the responsive track window.
- Render separate racer lanes.
- Render stats.
- Render bonus choices and item state.
- Render the player list.
- Render the event feed.
- Render help and final results.

Important Rust concepts:

- `TypingScreen<'a>` contains borrowed references to `Track`, `PlayerState`, `BonusState`, and `EventLog`.
- The `'a` lifetime means the screen cannot outlive the data it references.
- `Frame<'_>` means the frame has a lifetime, but the function does not need to name it.
- Ratatui widgets are values. We build them and pass them to `frame.render_widget`.
- `TrackWindow<'a>` and `VisibleWord<'a>` borrow word strings from the existing `Track` instead of cloning them.

The track renderer first computes a `TrackWindow`, which records visible words and their terminal columns. That metadata is then used to draw the word layer and each racer's lane. When Shield is active, the marker is rendered in bracketed form as `[███]` in ASCII mode or as `█🛡` in Unicode icon mode. The Unicode shield marker reserves the normal three-column kart footprint but does not draw a right-side block, because the shield emoji commonly occupies two terminal columns. When Mushroom is active, the marker gains a `>>>` prefix in ASCII mode or a `>>🍄` prefix in Unicode icon mode. Banana attacks briefly show a direction cue beside the attacker marker. Item impacts briefly blink the impacted racer's marker.

The word layer is rendered through fixed-width track cells. Correctly typed characters are green, the next character has a cursor-like highlight, and `typo_index` makes typed characters from the first typo onward red. Since the renderer maps `PlayerState.input` over the visible track stream, typo overflow can continue across following words and spaces while `word_index` remains blocked at the real race position.

The local racer marker is also derived from the character stream. It follows the next character while input is valid, and pins to the first typo while typo recovery is required. The local racer lane is rendered immediately below the word layer; AI racer lanes are rendered below it.

The planned minimap should also live in the track renderer, below all racer lanes. See `docs/minimap-plan.md` for the current implementation plan.

The bonus renderer reads the next visible bonus point from `BonusState`. Choices are stacked vertically so players can scan them before reaching the claim window. They stay grey while merely upcoming, then turn magenta once the player reaches the claim window. They also render grey when unavailable because the player has a held item, has a typo, has an active Shield, or the choice is cooling down.

## Data Flow

The current local game loop looks like this:

```text
keyboard event
  ui::terminal converts it to LocalAction
  ui::session applies it to LocalSession
  game::typing::apply_key mutates PlayerState for main-track input
  game::bonus handles bonus state and cooldowns
  game::items handles item helper rules
  ui::session logs meaningful TypingEvents
  ui::render draws Track + PlayerState + BonusState + EventLog
```

The important boundary:

```text
crossterm KeyEvent  ->  KeyAction  ->  game rules
```

This keeps terminal-specific code out of the game engine.

## Borrowing And Ownership In This Code

Some useful examples:

### Owned Values

`Track` owns its words:

```rust
pub struct Track {
    pub words: Vec<String>,
}
```

The track owns the `Vec`, and the vector owns each `String`.

### Shared Borrows

Rendering borrows state:

```rust
pub struct TypingScreen<'a> {
    pub track: &'a Track,
    pub player: &'a PlayerState,
}
```

The renderer should not mutate game state, so it gets shared references.

### Mutable Borrows

The typing engine mutates player state:

```rust
apply_key(&mut player, &track, action, Instant::now());
```

`&mut player` means there can be only one active mutable borrow of that player at a time. Rust enforces this to prevent accidental concurrent mutation.

## Error Handling

Application code mostly returns `anyhow::Result<()>`.

This is pragmatic for binaries because it keeps error handling lightweight:

```rust
pub fn play(...) -> Result<()> {
    let word_list = WordList::load("words_alpha.txt")?;
    ...
}
```

For library-style game rules, we should prefer explicit types and avoid unnecessary `anyhow`. For app setup, file loading, and terminal control, `anyhow` is fine.

## How To Run The Current Project

Run tests:

```sh
cargo test
```

Run the local typing prototype:

```sh
cargo run -- play
```

Run a short race:

```sh
cargo run -- play --words 10
```

Run with AI racers and write detailed item diagnostics after quitting:

```sh
cargo run -- play --ai-racers 6 --debug-log typekart-debug.log
```

Format code:

```sh
cargo fmt
```

## How To Read This Code

Recommended order:

1. Start with `src/main.rs` to see the CLI entry point.
2. Read `src/app.rs` to see how the first race is assembled.
3. Read `src/game/player.rs` and `src/game/track.rs` for the core data.
4. Read `src/game/typing.rs` slowly. This is where the main rules live.
5. Read the tests in `typing.rs`; they explain expected behavior very directly.
6. Read `src/ui/session.rs` to see how local state and display events are bundled.
7. Read `src/ui/terminal.rs` to see the runtime loop.
8. Read `src/ui/render.rs` to understand how state becomes terminal output.

## Things That Are Intentionally Simple For Now

- The UI is functional, not final.
- There is no multiplayer yet.
- The racer layer only shows the local player.
- The player list and event feed are local placeholders for future multiplayer.
- Bonus and item behavior is local-only; there are no real remote targets yet.
- The typing engine still only owns main-track typing. Bonus attempts are coordinated by `LocalSession`.
- Track generation samples with replacement, so repeated words can appear.
- Most state fields are public to keep early iteration straightforward.

These choices are acceptable for the current local prototype. We should tighten them only when the next milestone creates real pressure to do so.
