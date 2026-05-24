//! Shared AI racer typing rules.
//!
//! Hosts still decide when to tick AIs and how to broadcast/log the result.
//! This module owns the browser-safe rule details for WPM, focus/ink modifiers,
//! character budgets, pause checks, and selecting the next typing key.

use std::time::{Duration, Instant};

use super::{
    input_rules::player_input_is_paused_or_finished,
    player::PlayerState,
    race::{RacePlayerId, RaceState},
    track::Track,
    typing::{KeyAction, TypingEvent},
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AiDriverConfig {
    pub base_wpm: f64,
    pub focus_boost_wpm: u32,
    pub ink_multiplier_percent: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct AiDriverState {
    pub char_budget: f64,
    pub last_update: Option<Instant>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiDriverAdvance {
    pub typed_actions: Vec<KeyAction>,
    pub typing_events: Vec<TypingEvent>,
}

impl AiDriverAdvance {
    pub fn changed(&self) -> bool {
        !self.typed_actions.is_empty() || !self.typing_events.is_empty()
    }

    pub fn finished(&self) -> bool {
        self.typing_events
            .iter()
            .any(|event| matches!(event, TypingEvent::RaceFinished))
    }
}

pub fn reset_ai_driver_timing(state: &mut AiDriverState, now: Instant) {
    state.char_budget = 0.0;
    state.last_update = Some(now);
}

pub fn advance_ai_driver(
    race: &mut RaceState,
    player_id: RacePlayerId,
    driver: &mut AiDriverState,
    config: AiDriverConfig,
    now: Instant,
    elapsed: Duration,
) -> AiDriverAdvance {
    let mut advance = AiDriverAdvance {
        typed_actions: Vec::new(),
        typing_events: Vec::new(),
    };

    if ai_input_is_paused(race, player_id, now) {
        return advance;
    }

    let Some(player) = race.player(player_id) else {
        return advance;
    };
    let effective_wpm = ai_effective_wpm(
        config.base_wpm,
        player.state.has_active_focus(now),
        player.state.is_inked_at(now),
        config.focus_boost_wpm,
        config.ink_multiplier_percent,
    );
    driver.char_budget += ai_chars_for_elapsed(effective_wpm, elapsed);

    while driver.char_budget >= 1.0 {
        let Some(action) = next_ai_key(race, player_id) else {
            break;
        };
        let events = race
            .apply_key_input(player_id, action, now)
            .unwrap_or_default();
        driver.char_budget -= 1.0;
        advance.typed_actions.push(action);
        let finished = events
            .iter()
            .any(|event| matches!(event, TypingEvent::RaceFinished));
        advance.typing_events.extend(events);
        if finished {
            break;
        }
    }

    advance
}

pub fn ai_input_is_paused(race: &RaceState, player_id: RacePlayerId, now: Instant) -> bool {
    player_input_is_paused_or_finished(race, &Default::default(), player_id, now)
}

pub fn next_ai_key(race: &RaceState, player_id: RacePlayerId) -> Option<KeyAction> {
    let player = race.player(player_id)?;
    next_ai_key_for_player(&player.state, &race.track)
}

pub fn next_ai_key_for_player(player: &PlayerState, track: &Track) -> Option<KeyAction> {
    if player.is_finished() {
        return None;
    }
    let target = player
        .word_override(player.word_index)
        .or_else(|| track.current_word(player.word_index))?;
    if player.input == target {
        return Some(KeyAction::Space);
    }

    target
        .chars()
        .nth(player.input.chars().count())
        .map(KeyAction::Char)
}

pub fn ai_effective_wpm(
    base_wpm: f64,
    is_focused: bool,
    is_inked: bool,
    focus_boost_wpm: u32,
    ink_multiplier_percent: u32,
) -> f64 {
    let focused_wpm = if is_focused {
        base_wpm + f64::from(focus_boost_wpm)
    } else {
        base_wpm
    };
    if is_inked {
        focused_wpm * f64::from(ink_multiplier_percent) / 100.0
    } else {
        focused_wpm
    }
}

pub fn ai_chars_for_elapsed(words_per_minute: f64, elapsed: Duration) -> f64 {
    elapsed.as_secs_f64() * ai_chars_per_second(words_per_minute)
}

pub fn ai_chars_per_second(words_per_minute: f64) -> f64 {
    words_per_minute * 5.0 / 60.0
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{AiDriverConfig, AiDriverState, advance_ai_driver, next_ai_key};
    use crate::game::{
        race::{PlayerColorId, RacePlayerId, RaceState},
        track::Track,
        typing::KeyAction,
    };

    fn race(words: &[&str], now: Instant) -> RaceState {
        let mut race = RaceState::new(Track::new(
            words.iter().map(|word| word.to_string()).collect(),
        ));
        race.add_player(RacePlayerId(1), "ai", PlayerColorId::Cyan, now);
        race
    }

    #[test]
    fn next_ai_key_uses_space_after_current_word_is_complete() {
        let now = Instant::now();
        let mut race = race(&["go", "fast"], now);
        let player = race.players.first_mut().unwrap();
        player.state.input = "go".to_string();

        assert_eq!(next_ai_key(&race, RacePlayerId(1)), Some(KeyAction::Space));
    }

    #[test]
    fn advance_ai_driver_consumes_budget_and_types() {
        let now = Instant::now();
        let mut race = race(&["go", "fast"], now);
        let mut driver = AiDriverState::default();
        let advance = advance_ai_driver(
            &mut race,
            RacePlayerId(1),
            &mut driver,
            AiDriverConfig {
                base_wpm: 60.0,
                focus_boost_wpm: 0,
                ink_multiplier_percent: 100,
            },
            now,
            Duration::from_secs(1),
        );

        assert!(advance.changed());
        assert_eq!(race.players[0].state.word_index, 1);
        assert!(driver.char_budget < 1.0);
    }
}
