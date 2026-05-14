# Milestone 2 Implementation Plan: Race Renderer

## Goal

Replace the basic Milestone 1 typing screen with a more race-like terminal renderer.

Milestone 2 should make the single-player prototype look and feel closer to the intended multiplayer game without adding networking, bonus words, or items yet. The core deliverable is a responsive track window with a separate racer layer, a player list, and an event feed.

## Scope

Milestone 2 includes:

- A responsive visible track window.
- A separate racer layer aligned to the word layer.
- A three-character marker for the local player.
- A player list panel.
- An event feed panel.
- Cleaner layout behavior on narrow and wide terminals.
- Rendering-focused helper types and tests.
- Continued support for typo red highlighting and final-word instant finish.

Milestone 2 excludes:

- Real multiplayer.
- Remote player networking.
- Bonus words.
- Items.
- Attack warnings.
- Color blending for real overlapping remote players.
- Internet play.

## Design Intent

The renderer should move from a generic typing UI toward the eventual race UI.

The important design decision is that the word layer remains readable. Racer positions are shown on a separate layer aligned with the same track window.

Example target shape:

```text
TypeKart                    12/40 words       42 WPM

Track
quick brown fox jumps over the lazy driver into bright road
            ███

Input
fox
fo_

Players
1. you     12/40 words

Events
Race started
Completed brown
```

## Proposed File Changes

Milestone 2 should keep game logic mostly unchanged and focus on rendering structure.

Likely files:

```text
src/ui/render.rs
src/ui/terminal.rs
src/ui/layout.rs       # optional
src/ui/track_view.rs   # optional
src/ui/events.rs       # optional
src/game/player.rs
src/game/typing.rs
```

Recommended approach:

- Keep `render.rs` as the top-level renderer.
- Add small helper structs in `render.rs` first.
- Split into `layout.rs` or `track_view.rs` only if `render.rs` becomes hard to read.

Do not create abstractions until there is visible pressure. The current codebase is still small.

## New Rendering Concepts

### Track Window

The track window determines which words are visible.

Inputs:

- Full `Track`.
- Local `PlayerState`.
- Available terminal width.

Outputs:

- A list of visible word segments.
- The start and end word indices.
- The character column for the local player's current position.

The local player should usually be kept near the left-middle of the visible window, with enough upcoming words visible to feel like a track.

Initial recommendation:

- Keep 3 completed words visible behind the player when possible.
- Fill the remaining available width with upcoming words.
- Prefer whole words over cutting words mid-character.
- If a single word is too long for the available width, truncate only as a fallback.

### Word Segments

Represent visible words with metadata before rendering.

Possible helper type:

```rust
struct VisibleWord<'a> {
    index: usize,
    word: &'a str,
    start_col: usize,
    end_col: usize,
    state: WordRenderState,
}

enum WordRenderState {
    Completed,
    Current,
    Upcoming,
}
```

This makes it easier to align the racer layer and later bonus words.

### Racer Layer

Milestone 2 only has the local player, but implement the layer shape now.

Rules:

- The racer layer is a separate line below the word layer.
- The local racer marker is three adjacent cells wide.
- The marker is centered under the current word when possible.
- The marker uses the local player's color.
- If the marker would go past the edge, clamp it to the visible track width.

Initial local marker:

```text
███
```

Initial local color:

- Cyan or green.

Future multiplayer support can add:

- Multiple markers.
- Local-player visual priority.
- Remote overlap handling.
- Truecolor blending.

### Player List

Milestone 2 should add a player list even though there is only one player.

Example:

```text
Players
1. you     12/40 words
```

Purpose:

- Establish the layout needed for multiplayer.
- Make progress easy to scan.
- Give future remote players an obvious place to appear.

### Event Feed

Milestone 2 should add a small local event feed.

Initial events:

- Race started.
- Word completed.
- Typo started.
- Typo cleared.
- Race finished.

The event feed does not need to include every key press. It should show meaningful race events only.

Possible type:

```rust
pub struct EventLog {
    entries: VecDeque<String>,
    capacity: usize,
}
```

Keep only the latest few events, such as 5 to 8.

## Layout Plan

Use a two-column layout when the terminal is wide enough:

```text
+------------------------------------------------+----------------------+
| Header                                         | Stats                |
+------------------------------------------------+----------------------+
| Track                                          | Players              |
| word word word word                            | 1. you 12/40         |
|       ███                                      |                      |
+------------------------------------------------+----------------------+
| Input                                          | Events               |
| fo_                                            | Race started         |
|                                                | Completed brown      |
+------------------------------------------------+----------------------+
| Help                                                                  |
+-----------------------------------------------------------------------+
```

For narrow terminals, stack vertically:

```text
Header
Track
Input
Stats
Players
Events
Help
```

Initial breakpoint:

- Use two columns at width `>= 90`.
- Use stacked layout below that.

Implementation detail:

- Ratatui provides `Rect` values from layouts.
- The renderer should compute layout from `frame.size()` every draw.
- No resize-specific event handling is needed yet; redraws can adapt to the latest frame size.

## Event Flow

Milestone 1 currently ignores returned `TypingEvent` values.

Milestone 2 should keep and display them.

Suggested runtime state:

```rust
pub struct LocalSession {
    pub track: Track,
    pub player: PlayerState,
    pub events: EventLog,
}
```

`terminal.rs` can own `LocalSession` instead of separate `track` and `player` variables.

Flow:

```text
key event
  convert to KeyAction
  apply_key returns Vec<TypingEvent>
  append meaningful events to EventLog
  render Track + PlayerState + EventLog
```

This keeps event generation connected to the typing engine while leaving display text in the UI layer.

## Testing Strategy

Rendering itself is hard to test visually, so test the helper logic.

Recommended unit tests:

- Track window includes current word.
- Track window includes several upcoming words when width allows.
- Track window keeps completed words behind the player when possible.
- Track window does not exceed requested display width.
- Visible word metadata has correct start columns.
- Local racer marker is centered under the current word.
- Local racer marker clamps at the left edge.
- Local racer marker clamps at the right edge.
- Event log keeps only its configured capacity.
- Event log appends human-readable messages for key `TypingEvent` values.

Manual tests:

- Run `cargo run -- play --words 10`.
- Try a normal race.
- Try typos and backspacing.
- Resize the terminal while racing.
- Try a narrow terminal.
- Try a wide terminal.
- Complete the final word and verify the results view still appears immediately.

## Implementation Steps

### Step 1: Add Local Session State

Create a local session model for the terminal app.

Acceptance criteria:

- `terminal.rs` owns `LocalSession`.
- `LocalSession` contains `Track`, `PlayerState`, and `EventLog`.
- Existing typing behavior still works.
- `cargo test` passes.

### Step 2: Add Event Log

Add an event log and translate `TypingEvent` values into display entries.

Acceptance criteria:

- Race started event appears.
- Word completed events appear.
- Typo started and typo cleared events appear.
- Race finished event appears.
- Event log capacity is enforced.
- Event log logic has unit tests.

### Step 3: Build Track Window Helper

Create helper logic that maps words to visible columns.

Acceptance criteria:

- Track window helper has unit tests.
- It respects available width.
- It marks completed/current/upcoming words.
- It records start columns for visible words.

### Step 4: Render Separate Racer Layer

Use the track window metadata to render a local racer marker.

Acceptance criteria:

- Racer layer appears below the word layer.
- Local marker is three cells wide.
- Marker aligns with the current word.
- Marker remains visible near edges.
- Word layer remains readable.

### Step 5: Add Player List And Event Feed Panels

Add panels that match the future multiplayer layout.

Acceptance criteria:

- Player list shows `you` and progress.
- Event feed shows recent meaningful events.
- Panels fit in both wide and narrow layouts.

### Step 6: Improve Responsive Layout

Make layout adapt to terminal width.

Acceptance criteria:

- Wide terminals use two columns.
- Narrow terminals stack panels.
- Text does not obviously overlap or disappear in normal terminal sizes.
- Help/results content remains visible.

### Step 7: Update Docs

Update learning and implementation docs.

Acceptance criteria:

- `docs/codebase-tour.md` explains the new renderer/session/event-log structure.
- `docs/technical-plan.md` Milestone 2 notes remain accurate.
- Any changed controls or display behavior are reflected in docs.

## Acceptance Criteria For Milestone 2

Milestone 2 is complete when:

- `cargo run -- play` shows a race-like renderer.
- The visible track window adapts to terminal width.
- The word layer and racer layer are separate.
- The local racer is represented by a three-character colored marker.
- The player list is visible.
- The event feed is visible and updates during the race.
- Typo red highlighting still works.
- Final-word instant finish still works.
- The terminal restores cleanly on exit.
- `cargo fmt` passes.
- `cargo test` passes.

## Risks And Decisions To Watch

### Layout Complexity

Ratatui layouts can become hard to read if everything stays in one function. Start simple, but split helpers once layout code gets noisy.

### Unicode Width

The marker uses block characters such as `█`. These should normally be single-width in modern terminals, but terminal width behavior can vary. If alignment looks wrong, switch to ASCII markers such as `===` for Milestone 2.

### String Width

Rust string length in bytes is not the same as terminal display width. The current curated word list should be ASCII, so simple character counts are acceptable for now. Revisit this if non-ASCII words are introduced.

### Overbuilding Multiplayer Early

Milestone 2 should prepare the display shape for multiplayer but not implement multiplayer. Use one local player and simple placeholder structures where useful.

## Suggested Next Step

Implement Step 1 and Step 2 first. Once the session owns an event log, the renderer can display meaningful state while the track-window helper is built.
