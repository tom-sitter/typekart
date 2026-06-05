use std::time::Instant;

use super::player_input_is_paused_or_finished;
use crate::game::{
    race::{PlayerColorId, RacePlayerId, RaceState},
    track::Track,
};

#[test]
fn missing_player_is_treated_as_paused_for_ai_input() {
    let race = RaceState::new(Track::new(vec!["go".to_string()]));
    assert!(player_input_is_paused_or_finished(
        &race,
        &Default::default(),
        RacePlayerId(99),
        Instant::now()
    ));
}

#[test]
fn active_player_without_effects_can_type() {
    let now = Instant::now();
    let mut race = RaceState::new(Track::new(vec!["go".to_string()]));
    race.add_player(RacePlayerId(1), "host", PlayerColorId::Cyan, now);

    assert!(!player_input_is_paused_or_finished(
        &race,
        &Default::default(),
        RacePlayerId(1),
        now
    ));
}
