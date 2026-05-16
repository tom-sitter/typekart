//! Deterministic typing rules.
//!
//! This module is deliberately independent from terminal input. The UI converts
//! terminal-specific key events into `KeyAction`, then this module mutates
//! `PlayerState` according to the game rules.

use std::time::Instant;

use super::{player::PlayerState, track::Track};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    Char(char),
    Space,
    Backspace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
) -> Vec<TypingEvent> {
    if player.is_finished() {
        return Vec::new();
    }

    let Some(target) = track.current_word(player.word_index) else {
        player.finished_at = Some(now);
        return vec![TypingEvent::RaceFinished];
    };

    match action {
        KeyAction::Char(ch) => apply_char(player, track, target, ch, now),
        KeyAction::Space => apply_space(player, track, target, now),
        KeyAction::Backspace => apply_backspace(player, target),
    }
}

fn apply_char(
    player: &mut PlayerState,
    track: &Track,
    target: &str,
    ch: char,
    now: Instant,
) -> Vec<TypingEvent> {
    let previous_typo = player.typo_index;
    let input_index = player.input.chars().count();
    // Once a typo exists, later characters are still recorded but cannot count
    // as progress until the typo span is backspaced away.
    let is_correct = previous_typo.is_none() && target.chars().nth(input_index) == Some(ch);

    player.stats.typed_chars += 1;
    if is_correct {
        player.stats.correct_chars += 1;
    } else {
        player.stats.typo_chars += 1;
    }

    player.input.push(ch);
    player.typo_index = first_typo_index(&player.input, target);

    let mut events = input_events(previous_typo, player.typo_index);
    if player.typo_index.is_none() && player.input == target && player.word_index + 1 >= track.len()
    {
        finish_current_word(player, now);
        events.push(TypingEvent::WordCompleted);
        events.push(TypingEvent::RaceFinished);
    }

    events
}

fn apply_space(
    player: &mut PlayerState,
    track: &Track,
    target: &str,
    now: Instant,
) -> Vec<TypingEvent> {
    if player.input == target {
        player.word_index += 1;
        player.input.clear();
        player.typo_index = None;
        player.stats.completed_words += 1;

        if player.word_index >= track.len() {
            player.finished_at = Some(now);
            return vec![TypingEvent::WordCompleted, TypingEvent::RaceFinished];
        }

        return vec![TypingEvent::WordCompleted];
    }

    apply_char(player, track, target, ' ', now)
}

fn finish_current_word(player: &mut PlayerState, now: Instant) {
    player.word_index += 1;
    player.input.clear();
    player.typo_index = None;
    player.stats.completed_words += 1;
    player.finished_at = Some(now);
}

fn apply_backspace(player: &mut PlayerState, target: &str) -> Vec<TypingEvent> {
    let previous_typo = player.typo_index;

    if player.input.pop().is_none() {
        return Vec::new();
    }

    player.stats.backspaces += 1;
    player.typo_index = first_typo_index(&player.input, target);

    input_events(previous_typo, player.typo_index)
}

pub fn first_typo_index(input: &str, target: &str) -> Option<usize> {
    for (index, input_char) in input.chars().enumerate() {
        if target.chars().nth(index) != Some(input_char) {
            return Some(index);
        }
    }

    None
}

fn input_events(previous_typo: Option<usize>, current_typo: Option<usize>) -> Vec<TypingEvent> {
    let mut events = vec![TypingEvent::InputChanged];

    match (previous_typo, current_typo) {
        (None, Some(index)) => events.push(TypingEvent::TypoStarted { index }),
        (Some(_), None) => events.push(TypingEvent::TypoCleared),
        _ => {}
    }

    events
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use pretty_assertions::assert_eq;

    use super::{apply_key, KeyAction, TypingEvent};
    use crate::game::{player::PlayerState, track::Track};

    fn track(words: &[&str]) -> Track {
        Track::new(words.iter().map(|word| word.to_string()).collect())
    }

    fn player() -> PlayerState {
        PlayerState::new(Instant::now())
    }

    fn type_chars(player: &mut PlayerState, track: &Track, input: &str) {
        type_chars_at(player, track, input, Instant::now());
    }

    fn type_chars_at(player: &mut PlayerState, track: &Track, input: &str, now: Instant) {
        for ch in input.chars() {
            apply_key(player, track, KeyAction::Char(ch), now);
        }
    }

    #[test]
    fn typing_a_correct_word_advances_only_after_space() {
        let track = track(&["fox", "road"]);
        let mut player = player();
        let now = Instant::now();

        type_chars(&mut player, &track, "fox");

        assert_eq!(player.word_index, 0);
        assert_eq!(player.input, "fox");

        let events = apply_key(&mut player, &track, KeyAction::Space, now);

        assert_eq!(player.word_index, 1);
        assert_eq!(player.input, "");
        assert_eq!(events, vec![TypingEvent::WordCompleted]);
    }

    #[test]
    fn typing_a_correct_prefix_does_not_advance_before_space() {
        let track = track(&["fox"]);
        let mut player = player();

        type_chars(&mut player, &track, "fo");

        assert_eq!(player.word_index, 0);
        assert_eq!(player.input, "fo");
        assert_eq!(player.typo_index, None);
    }

    #[test]
    fn pressing_space_early_creates_a_typo() {
        let track = track(&["fox"]);
        let mut player = player();

        type_chars(&mut player, &track, "fo");
        let events = apply_key(&mut player, &track, KeyAction::Space, Instant::now());

        assert_eq!(player.input, "fo ");
        assert_eq!(player.typo_index, Some(2));
        assert_eq!(
            events,
            vec![
                TypingEvent::InputChanged,
                TypingEvent::TypoStarted { index: 2 }
            ]
        );
    }

    #[test]
    fn wrong_letter_creates_typo_index() {
        let track = track(&["fox"]);
        let mut player = player();

        type_chars(&mut player, &track, "fax");

        assert_eq!(player.typo_index, Some(1));
    }

    #[test]
    fn progress_is_blocked_while_typo_exists() {
        let track = track(&["fox", "road"]);
        let mut player = player();

        type_chars(&mut player, &track, "fax");
        apply_key(&mut player, &track, KeyAction::Space, Instant::now());

        assert_eq!(player.word_index, 0);
        assert_eq!(player.typo_index, Some(1));
    }

    #[test]
    fn backspace_can_clear_a_typo() {
        let track = track(&["fox"]);
        let mut player = player();

        type_chars(&mut player, &track, "fa");
        assert_eq!(player.typo_index, Some(1));

        let events = apply_key(&mut player, &track, KeyAction::Backspace, Instant::now());

        assert_eq!(player.input, "f");
        assert_eq!(player.typo_index, None);
        assert_eq!(
            events,
            vec![TypingEvent::InputChanged, TypingEvent::TypoCleared]
        );
    }

    #[test]
    fn extra_typo_chars_must_be_backspaced_before_original_typo_clears() {
        let track = track(&["fox"]);
        let mut player = player();

        type_chars(&mut player, &track, "fabc");
        apply_key(&mut player, &track, KeyAction::Backspace, Instant::now());

        assert_eq!(player.input, "fab");
        assert_eq!(player.typo_index, Some(1));
    }

    #[test]
    fn completing_final_word_finishes_without_space() {
        let track = track(&["fox"]);
        let mut player = player();
        let now = Instant::now() + Duration::from_secs(10);

        type_chars_at(&mut player, &track, "fo", now);
        assert!(!player.is_finished());

        let events = apply_key(&mut player, &track, KeyAction::Char('x'), now);

        assert!(player.is_finished());
        assert_eq!(player.word_index, 1);
        assert_eq!(player.input, "");
        assert_eq!(
            events,
            vec![
                TypingEvent::InputChanged,
                TypingEvent::WordCompleted,
                TypingEvent::RaceFinished
            ]
        );
    }

    #[test]
    fn stats_update_for_accuracy_and_backspaces() {
        let track = track(&["fox"]);
        let mut player = player();

        type_chars(&mut player, &track, "fa");
        apply_key(&mut player, &track, KeyAction::Backspace, Instant::now());
        type_chars(&mut player, &track, "ox");

        assert_eq!(player.stats.typed_chars, 4);
        assert_eq!(player.stats.correct_chars, 3);
        assert_eq!(player.stats.typo_chars, 1);
        assert_eq!(player.stats.backspaces, 1);
        assert_eq!(player.stats.accuracy(), 75.0);
    }

    #[test]
    fn empty_input_backspace_is_harmless() {
        let track = track(&["fox"]);
        let mut player = player();

        let events = apply_key(&mut player, &track, KeyAction::Backspace, Instant::now());

        assert!(events.is_empty());
        assert_eq!(player.input, "");
        assert_eq!(player.stats.backspaces, 0);
    }

    #[test]
    fn typing_after_finish_does_nothing() {
        let track = track(&["fox"]);
        let mut player = player();
        let now = Instant::now();

        type_chars(&mut player, &track, "fox");
        apply_key(&mut player, &track, KeyAction::Space, now);
        let events = apply_key(&mut player, &track, KeyAction::Char('x'), now);

        assert!(events.is_empty());
        assert_eq!(player.input, "");
    }
}
