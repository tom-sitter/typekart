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
            fog_multiplier_percent: 100,
        },
        now,
        Duration::from_secs(1),
    );

    assert!(advance.changed());
    assert_eq!(race.players[0].state.word_index, 1);
    assert!(driver.char_budget < 1.0);
}
