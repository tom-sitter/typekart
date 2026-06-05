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
    let focus = player.has_active_focus(now);
    apply_key_with_options(player, track, action, now, focus)
}

pub fn apply_key_with_options(
    player: &mut PlayerState,
    track: &Track,
    action: KeyAction,
    now: Instant,
    focus: bool,
) -> Vec<TypingEvent> {
    if player.is_finished() {
        return Vec::new();
    }

    let Some(base_target) = track.current_word(player.word_index) else {
        player.finished_at = Some(now);
        return vec![TypingEvent::RaceFinished];
    };
    let target = player
        .word_override(player.word_index)
        .unwrap_or(base_target)
        .to_string();

    match action {
        KeyAction::Char(ch) => apply_char(player, track, &target, ch, now, focus),
        KeyAction::Space => apply_space(player, track, &target, now, focus),
        KeyAction::Backspace => apply_backspace(player, &target),
    }
}

fn apply_char(
    player: &mut PlayerState,
    track: &Track,
    target: &str,
    ch: char,
    now: Instant,
    focus: bool,
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

    if focus && !is_correct {
        return vec![TypingEvent::InputChanged];
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
    focus: bool,
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

    apply_char(player, track, target, ' ', now, focus)
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
mod tests;
