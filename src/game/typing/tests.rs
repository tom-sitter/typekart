use std::time::{Duration, Instant};

use pretty_assertions::assert_eq;

use super::{KeyAction, TypingEvent, apply_key, apply_key_with_options};
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

#[test]
fn focus_counts_wrong_keys_without_adding_typo_input() {
    let track = track(&["fox"]);
    let mut player = player();
    let now = Instant::now();

    let events = apply_key_with_options(&mut player, &track, KeyAction::Char('x'), now, true);

    assert_eq!(events, vec![TypingEvent::InputChanged]);
    assert_eq!(player.input, "");
    assert_eq!(player.typo_index, None);
    assert_eq!(player.stats.typed_chars, 1);
    assert_eq!(player.stats.typo_chars, 1);

    apply_key_with_options(&mut player, &track, KeyAction::Char('f'), now, true);

    assert_eq!(player.input, "f");
    assert_eq!(player.stats.correct_chars, 1);
}

#[test]
fn word_override_becomes_target_word() {
    let track = track(&["drawer", "next"]);
    let mut player = player();
    player.word_overrides.insert(0, "reward".to_string());

    type_chars(&mut player, &track, "reward");

    assert_eq!(player.input, "reward");
    assert_eq!(player.typo_index, None);
}
