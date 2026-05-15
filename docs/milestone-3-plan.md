# Milestone 3 Implementation Plan: Local Game Rules

## Goal

Add the first local version of TypeKart's game mechanics: bonus points, held items, item activation, and attack warning UI.

Milestone 3 should prove the rules locally before networking exists. The implementation should keep the game engine deterministic and testable, while the terminal UI should make bonus and item state visible enough to play with.

## Scope

Milestone 3 includes:

- Periodic bonus points along the track.
- Three visible bonus choices per bonus point.
- Inferred bonus-word intent.
- Bonus lockout while holding an item.
- Greyed-out bonus words while locked out.
- One held item at a time, except Shield, which activates immediately.
- Item activation key handling.
- Modified item activation key handling if terminal support is reliable enough.
- Mushroom.
- Shield.
- A local/simulated Banana path.
- Attack warning UI.
- Unit tests for bonus and item rules.

Milestone 3 excludes:

- Real multiplayer.
- Networked item targeting.
- Real remote racers.
- Blue Shell.
- Star Power.
- Full item weighting by race position.
- Internet play.

## Important Local-Only Constraint

Banana is an opponent-targeted item, but Milestone 3 does not have real opponents yet.

For this milestone, Banana should be implemented as a local rule path that can be tested and displayed, but it does not need to meaningfully hit a real remote player.

Recommended local behavior:

- If a simulated target exists in test code, Banana can create an attack warning against that target.
- In the playable single-player UI, activating Banana should log that no valid target is in range.

Initial recommendation:

- Consume Banana whenever it is used, whether or not a valid target exists.
- Log `No racer in range`.

This matches the intended multiplayer behavior: using Banana is a timing commitment, and a missed attack still spends the item.

## Proposed File Changes

Likely new files:

```text
src/game/bonus.rs
src/game/items.rs
src/game/effects.rs
```

Likely changed files:

```text
src/game/mod.rs
src/game/player.rs
src/game/typing.rs
src/game/track.rs
src/ui/session.rs
src/ui/render.rs
src/ui/terminal.rs
docs/codebase-tour.md
```

Recommended approach:

- Put core bonus and item rules under `game`.
- Keep UI-specific event text under `ui::session`.
- Keep rendering logic in `ui::render`.
- Avoid putting bonus or item rules directly in the terminal loop.

## Data Model

### BonusPoint

A bonus point is attached to a gap between two main-track words.

```rust
pub struct BonusPoint {
    pub after_word_index: usize,
    pub choices: [BonusChoice; 3],
}
```

Meaning:

- The bonus point becomes available after `after_word_index` is completed.
- It remains available before the next main-track word is begun.
- Example: if `after_word_index == 7`, the player can attempt the bonus after word 7 and before typing word 8.

### BonusChoice

```rust
pub struct BonusChoice {
    pub word: String,
    pub status: BonusChoiceStatus,
}

pub enum BonusChoiceStatus {
    Available,
    Cooldown { until: Instant },
}
```

Initial local implementation:

- Cooldowns can be time-based.
- Expired cooldowns replace the word with a new word from the word list.
- If implementation cost is high, use a fixed cooldown duration and check it once per terminal tick.

### BonusAttemptState

Bonus intent is inferred from typed input. The engine needs to know whether the player is currently attempting a bonus word.

Possible shape:

```rust
pub enum InputMode {
    MainWord,
    Bonus {
        point_index: usize,
        choice_index: usize,
    },
}
```

The existing `PlayerState.input` can still store the currently typed buffer.

Rules:

- If input starts matching exactly one available bonus choice during a bonus window, enter `InputMode::Bonus`.
- If input starts matching the main word, remain in `InputMode::MainWord`.
- If multiple choices share a prefix, stay unresolved until the input uniquely identifies one or becomes invalid.
- Backspace can clear a bonus attempt and return to no input.
- Once the next main word has begun, bonus choices for that point are no longer available to that player.

To avoid overcomplicating Milestone 3, we can require curated bonus choices at a point to have distinct first letters.

Initial recommendation:

- Generate three bonus choices with distinct first letters.
- This makes inferred intent easy and predictable.

### HeldItem

```rust
pub enum HeldItem {
    Mushroom,
    Banana,
}
```

Player state should gain:

```rust
pub held_item: Option<HeldItem>
pub active_effects: Vec<ActiveEffect>
```

### ActiveEffect

```rust
pub enum ActiveEffect {
    Shield { until: Instant },
}
```

Milestone 3 only needs Shield as a timed active effect.

### AttackWarning

```rust
pub struct AttackWarning {
    pub attack: PendingAttack,
    pub target_player_id: LocalPlayerId,
    pub resolves_at: Instant,
}
```

For Milestone 3 there is only one playable local player, so `LocalPlayerId` may be unnecessary. If keeping it local, use:

```rust
pub struct AttackWarning {
    pub attack: PendingAttack,
    pub resolves_at: Instant,
}
```

Possible pending attacks:

```rust
pub enum PendingAttack {
    BananaWordSwap,
}
```

## Bonus Rules

Implement these rules:

- Bonus points appear periodically along the generated track.
- Each bonus point has three choices.
- Bonus choices are visible to the player when their track window includes the relevant gap.
- A bonus point is claimable only after the preceding word is completed and before the next word is begun.
- Players cannot claim bonuses while holding an item.
- Players cannot claim bonuses while a typo is present.
- If a player starts typing a bonus word, Backspace can bail out.
- Completing a bonus word grants an item.
- The claimed bonus choice enters cooldown.
- When cooldown expires, the choice is replaced.

Local simplification:

- There are no simultaneous claim races until multiplayer exists.
- Still model choice cooldown now so the later server can own the same state.

## Bonus Placement

Initial recommendation:

- One bonus point every 8 words.
- Do not place a bonus point after the final word.
- For very short races, allow no bonus points.
- Generate three choices per point from `words_alpha.txt`.
- Use 4 to 8 letter bonus words if the curated list supports that easily.
- Avoid duplicate active choices at the same bonus point.
- Prefer distinct first letters for inference.

## Item Rules

### Item Rolling

Milestone 3 can use mostly equal item odds:

- Mushroom.
- Banana.
- Shield.

Shield should become more likely when one or more racers are within 5 words ahead or behind. In the local prototype this can be modeled with a helper that accepts `has_nearby_racer`, even though there are no real nearby opponents yet.

### Mushroom

Rules:

- Activating Mushroom rapidly advances three main-track words, one word at a time.
- If fewer than three words remain, finish the race.
- Clear current input and typo state.
- Do not grant or skip bonus claims while jumping.
- Consume the held item.
- Initial boost speed should be approximately equivalent to 150 WPM.
- The boost speed should be tunable after playtesting.

Open implementation choice:

- Whether Mushroom completion should increment `completed_words` by the number boosted.

Recommendation:

- Yes. `completed_words` should reflect progress through the track.
- Raw typing stats should remain separate, so Mushroom does not add `typed_chars` or `correct_chars`.

### Shield

Rules:

- Picking up Shield immediately creates `ActiveEffect::Shield` for 5 seconds.
- Shield is not stored as a held item.
- Shield cannot be saved for later.
- If a blockable attack resolves while Shield is active, block the attack and consume the active shield.
- Expire Shield after 5 seconds if unused.

### Banana

Rules:

- Normal use targets nearest racer behind.
- Modified use targets nearest racer ahead.
- Target must be within 10 words.
- If no target exists, log `No racer in range`.
- Banana is consumed whenever it is used, whether or not a target exists.

Milestone 3 local recommendation:

- Add test-only helper logic for target selection so multiplayer can reuse it later.
- Add `PendingAttack::BananaWordSwap` and attack-warning rendering even if normal local play rarely creates it.

## Controls

Milestone 3 should validate item activation keys in practice.

Initial recommendation:

- Normal item use: `Enter`.
- Modified item use: attempt `Shift+Enter` if `crossterm` reports it distinctly.
- If `Shift+Enter` cannot be detected reliably, use `Ctrl+K` for modified item use.

Implementation recommendation:

- Add a small debug command or log entry to confirm what key event is received for `Shift+Enter`.
- Keep key mapping isolated in `ui::terminal::key_action`.

Possible expanded input enum:

```rust
pub enum LocalAction {
    Typing(KeyAction),
    ActivateItem,
    ActivateModifiedItem,
}
```

The terminal layer maps key events to `LocalAction`, and the session decides what to do.

## Rendering Requirements

### Bonus Layer

Render bonus choices above the track word layer when a bonus point is visible in the track window.

The three choices should be stacked vertically and visible before the player reaches the bonus point, so the player can plan whether to attempt a bonus.

Ahead-of-time visibility is not the same as claimability. Bonus choices should render inactive or greyed out until the player has completed the preceding main-track word and has not begun the next one.

States:

- Available: normal/highlighted.
- Upcoming but not claimable yet: greyed out.
- Unavailable because holding item: greyed out.
- Unavailable because typo exists: greyed out.
- Cooldown: dimmed or replaced with a cooldown marker.

Initial display example:

```text
Bonus       turbo
Bonus       spark
Bonus       drift
Track   quick brown fox jumps over road
Racer             ███
```

If there is no nearby bonus point, the bonus layer can be blank.

### Shield Marker

When Shield is active, the racer marker should be encapsulated in brackets.

Example:

```text
Track   quick brown fox jumps over road
Racer             [███]
```

Rules:

- The bracketed marker is only shown while the Shield effect is active.
- The marker should keep the racer's unique color.
- The brackets should make the protection state visible even if the player is focused on the track rather than the item panel.
- Alignment should account for the bracketed marker being wider than the normal three-character marker.

### Item Panel

Show:

- Held item, or `None`.
- Active effects and remaining time.
- Attack warning if present.

Example:

```text
Item
Held: Mushroom
Shield: 3.2s
Warning: Banana incoming
```

### Event Feed

Add events for:

- Bonus claimed.
- Item received.
- Item used.
- No target in range.
- Shield activated.
- Shield expired.
- Attack blocked.
- Attack landed.

## Session Flow

`LocalSession` should become the local game coordinator.

On typing action:

1. Check whether input should apply to a bonus attempt or main typing.
2. Apply typing rules.
3. Resolve bonus claim if completed.
4. Add events.

On item activation:

1. Check held item.
2. Apply item behavior.
3. Consume item if appropriate.
4. Add events.

On tick:

1. Expire bonus cooldowns and replace choices.
2. Expire active effects.
3. Resolve attack warnings.
4. Add events.

This is the first milestone where the app needs meaningful time-based updates even without key input.

## Testing Strategy

Prioritize unit tests for game logic.

Bonus tests:

- Bonus points are generated periodically.
- Each bonus point has three choices.
- Choices at a point are unique.
- Choices at a point have distinct first letters.
- Bonus is available only in the correct word gap.
- Bonus is unavailable while holding an item.
- Bonus is unavailable while typo state exists.
- Bonus is unavailable while Shield is active.
- Backspace can bail out of a bonus attempt.
- Completing a bonus grants an item.
- Claimed choice enters cooldown.
- Expired cooldown replaces the choice.

Item tests:

- Item roll returns only Milestone 3 items.
- Item roll increases Shield probability when racers are nearby.
- Mushroom advances three words.
- Mushroom advances one word at a time rather than teleporting.
- Mushroom finishes if fewer than three words remain.
- Mushroom clears input and typo state.
- Mushroom consumes held item.
- Shield pickup immediately creates a 5 second effect.
- Shield is not placed in the held item slot.
- Shield expires.
- Shield blocks a pending attack.
- Blocking consumes active Shield.
- Banana with no local target logs or returns no target.
- Banana with no local target still consumes the held item.
- Banana target selection chooses nearest valid racer in the requested direction.

Renderer/helper tests:

- Bonus choices render as available when claimable.
- Bonus choices render as greyed out while holding an item.
- Bonus choices render as greyed out while typo state exists.
- Bonus choices render as greyed out while Shield is active.
- Item panel text changes when held item changes.
- Racer marker changes to bracketed shield form while Shield is active.

Manual tests:

- Claim a bonus.
- Confirm held item appears.
- Confirm bonus choices grey out while holding item.
- Confirm bonus choices grey out while Shield is active.
- Use Mushroom.
- Pick up Shield and confirm it activates immediately.
- Try Banana with no target.
- Confirm event feed stays understandable.
- Confirm final-word instant finish still works.

## Implementation Steps

### Step 1: Add Item Types To Player State

Acceptance criteria:

- `HeldItem` exists with Mushroom and Banana.
- Shield exists as an immediate pickup/effect.
- `PlayerState` has `held_item`.
- `PlayerState` has `active_effects`.
- Renderer shows held item.
- Tests pass.

### Step 2: Add Local Action Mapping

Acceptance criteria:

- Terminal input maps to `LocalAction`.
- Typing behavior still works.
- `Enter` triggers normal item activation.
- A candidate modified-use key is mapped.
- Key mapping is isolated and testable where practical.

### Step 3: Add Bonus Model And Generation

Acceptance criteria:

- Bonus points are generated with the track.
- Each point has three choices.
- Choices are unique and preferably distinct by first letter.
- Bonus generation has unit tests.

### Step 4: Add Bonus Claiming

Acceptance criteria:

- Bonus intent is inferred from typed input during the bonus window.
- Backspace can bail out.
- Bonus claim grants a held item or immediately activates Shield.
- Bonus claim is blocked while typo state exists.
- Bonus claim is blocked while holding an item.
- Bonus claim is blocked while Shield is active.
- Claimed choice enters cooldown.
- Unit tests cover claim and lockout behavior.

### Step 5: Render Bonus Layer

Acceptance criteria:

- Nearby/current bonus choices appear above the track.
- Greyed-out state appears while holding an item.
- Greyed-out state appears while typo state exists.
- Greyed-out state appears while Shield is active.
- Cooldown state is visible.

### Step 6: Implement Mushroom

Acceptance criteria:

- Mushroom advances three words.
- Mushroom advances at a visible speedboost pace.
- Mushroom can finish the race.
- Mushroom clears input and typo state.
- Mushroom consumes held item.
- Events are logged.
- Tests pass.

### Step 7: Implement Shield And Attack Warning Model

Acceptance criteria:

- Shield activates for 5 seconds when picked up.
- Shield appears in the item panel.
- Active Shield changes the racer marker to bracketed form.
- Shield expires on tick.
- Attack warning can be represented and rendered.
- Shield can block a pending attack in tests.
- Tests pass.

### Step 8: Implement Banana Local Path

Acceptance criteria:

- Banana normal and modified activation paths exist.
- Target selection helper supports ahead/behind within 10 words.
- Single-player activation logs `No racer in range`.
- Held Banana is consumed when used, even if no target exists.
- Tests pass.

### Step 9: Update Docs

Acceptance criteria:

- `docs/codebase-tour.md` explains the new bonus/item modules.
- `docs/technical-plan.md` remains accurate.
- Any key mapping decision is documented.

## Acceptance Criteria For Milestone 3

Milestone 3 is complete when:

- Bonus points appear during a local race.
- Each bonus point shows three choices.
- Bonus intent is inferred from typing.
- Bonus words are unavailable while holding an item.
- Bonus words are unavailable while typo state exists.
- Bonus words are unavailable while Shield is active.
- Unavailable bonus words are greyed out.
- Completing a bonus grants a held item or immediately activates Shield.
- Held item is visible.
- Mushroom works.
- Shield works.
- Banana has a local no-target path and reusable target-selection logic.
- Attack warnings can be represented and rendered.
- Event feed reports bonus and item events.
- Existing typing rules still work.
- Final-word instant finish still works.
- `cargo fmt` passes.
- `cargo test` passes.

## Risks And Decisions To Watch

### Bonus Intent Ambiguity

Inferred bonus intent can become confusing if bonus choices share prefixes with each other or the main word. Start with distinct first letters at each bonus point.

### UI Crowding

Bonus choices, item state, event feed, and track state can crowd the terminal. Prefer clear minimal text over showing every detail at once.

### Item Key Reliability

`Shift+Enter` may not be distinguishable in some terminals. Validate it before depending on it. Use `Ctrl+K` if needed.

### Scope Creep

Do not add Blue Shell, Star Power, or real multiplayer in this milestone. The goal is to prove the local game-rule architecture.

## Suggested Next Step

Start with Step 1 and Step 2: add item fields and local action mapping. That gives the UI a place to show held-item state before the more complex bonus engine arrives.
