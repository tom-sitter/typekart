use std::time::{Duration, Instant};

use super::{
    PlayerColorId, RaceLifecycleState, RaceLifecycleStatus, RaceParticipant, RacePlayerId,
    RaceResultStatus, RaceState, update_race_lifecycle,
};
use crate::game::{stats::TypingStats, track::Track, typing::KeyAction};

fn track(words: &[&str]) -> Track {
    Track::new(words.iter().map(|word| word.to_string()).collect())
}

#[test]
fn race_state_adds_players() {
    let now = Instant::now();
    let mut race = RaceState::new(track(&["one", "two"]));

    race.add_player(RacePlayerId(1), "tom", PlayerColorId::Cyan, now);

    let player = race.player(RacePlayerId(1)).unwrap();
    assert_eq!(player.name, "tom");
    assert_eq!(player.color, PlayerColorId::Cyan);
    assert!(player.connected);
    assert_eq!(player.state.word_index, 0);
}

#[test]
fn race_state_builds_from_participants() {
    let now = Instant::now();
    let race = RaceState::from_participants(
        track(&["one", "two"]),
        [
            RaceParticipant {
                id: RacePlayerId(1),
                name: "tom".to_string(),
                color: PlayerColorId::Cyan,
                connected: true,
            },
            RaceParticipant {
                id: RacePlayerId(2),
                name: "alex".to_string(),
                color: PlayerColorId::Red,
                connected: false,
            },
        ],
        now,
    );

    assert_eq!(race.players.len(), 2);
    assert_eq!(race.players[0].name, "tom");
    assert!(race.players[0].connected);
    assert_eq!(race.players[1].color, PlayerColorId::Red);
    assert!(!race.players[1].connected);
    assert_eq!(race.players[1].state.started_at, now);
}

#[test]
fn race_state_applies_key_input_to_selected_player() {
    let now = Instant::now();
    let mut race = RaceState::new(track(&["one", "two"]));
    race.add_player(RacePlayerId(1), "tom", PlayerColorId::Cyan, now);
    race.add_player(RacePlayerId(2), "alex", PlayerColorId::Red, now);

    race.apply_key_input(RacePlayerId(2), KeyAction::Char('o'), now)
        .unwrap();

    assert_eq!(race.player(RacePlayerId(1)).unwrap().state.input, "");
    assert_eq!(race.player(RacePlayerId(2)).unwrap().state.input, "o");
}

#[test]
fn race_state_returns_none_for_unknown_player_input() {
    let now = Instant::now();
    let mut race = RaceState::new(track(&["one", "two"]));

    let events = race.apply_key_input(RacePlayerId(99), KeyAction::Char('o'), now);

    assert_eq!(events, None);
}

#[test]
fn race_state_uses_existing_final_word_finish_rule() {
    let now = Instant::now();
    let mut race = RaceState::new(track(&["a"]));
    race.add_player(RacePlayerId(1), "tom", PlayerColorId::Cyan, now);

    race.apply_key_input(RacePlayerId(1), KeyAction::Char('a'), now)
        .unwrap();

    let player = race.player(RacePlayerId(1)).unwrap();
    assert!(player.state.is_finished());
    assert_eq!(player.state.stats.completed_words, 1);
}

#[test]
fn race_result_rows_order_finished_then_progress_and_include_stats() {
    let now = Instant::now();
    let mut race = RaceState::new(track(&["one", "two"]));
    race.add_player(RacePlayerId(1), "host", PlayerColorId::Cyan, now);
    race.add_player(RacePlayerId(2), "guest", PlayerColorId::Red, now);

    let guest = race
        .players
        .iter_mut()
        .find(|player| player.id == RacePlayerId(2))
        .unwrap();
    guest.state.finished_at = Some(now);
    guest.state.stats.completed_words = 2;

    let host = race
        .players
        .iter_mut()
        .find(|player| player.id == RacePlayerId(1))
        .unwrap();
    host.connected = false;
    host.state.word_index = 1;
    host.state.stats = TypingStats {
        typed_chars: 10,
        correct_chars: 8,
        typo_chars: 2,
        backspaces: 3,
        completed_words: 1,
    };

    let rows = super::build_race_result_rows(&race, &[RacePlayerId(2)], now);

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].player_id, RacePlayerId(2));
    assert_eq!(rows[0].status, RaceResultStatus::Finished);
    assert_eq!(rows[0].progress_words, 2);
    assert_eq!(rows[1].player_id, RacePlayerId(1));
    assert_eq!(rows[1].status, RaceResultStatus::Disconnected);
    assert_eq!(rows[1].progress_words, 1);
    assert_eq!(rows[1].accuracy_percent, 80);
    assert_eq!(rows[1].typo_chars, 2);
    assert_eq!(rows[1].backspaces, 3);
}

#[test]
fn lifecycle_records_finish_order_and_finishes_when_all_connected_finish() {
    let now = Instant::now();
    let mut race = RaceState::new(track(&["one"]));
    race.add_player(RacePlayerId(1), "tom", PlayerColorId::Cyan, now);
    race.add_player(RacePlayerId(2), "alex", PlayerColorId::Red, now);
    let mut placements = Vec::new();
    let mut first_finished_at = None;

    race.players[1].state.finished_at = Some(now);
    let update = update_race_lifecycle(
        &race,
        &mut placements,
        &mut first_finished_at,
        now,
        Duration::from_secs(30),
    );

    assert_eq!(update.newly_finished, vec![RacePlayerId(2)]);
    assert_eq!(update.status, RaceLifecycleStatus::Running);
    assert_eq!(placements, vec![RacePlayerId(2)]);
    assert_eq!(first_finished_at, Some(now));

    race.players[0].state.finished_at = Some(now);
    let update = update_race_lifecycle(
        &race,
        &mut placements,
        &mut first_finished_at,
        now,
        Duration::from_secs(30),
    );

    assert_eq!(update.newly_finished, vec![RacePlayerId(1)]);
    assert_eq!(
        update.status,
        RaceLifecycleStatus::Finished {
            all_connected_finished: true,
            all_connected_disconnected: false,
            timeout_expired: false
        }
    );
    assert_eq!(placements, vec![RacePlayerId(2), RacePlayerId(1)]);
}

#[test]
fn lifecycle_state_updates_and_resets_shared_progress() {
    let now = Instant::now();
    let mut race = RaceState::new(track(&["one"]));
    race.add_player(RacePlayerId(1), "tom", PlayerColorId::Cyan, now);
    let mut lifecycle = RaceLifecycleState::new();
    race.players[0].state.finished_at = Some(now);

    lifecycle.update(&race, now, Duration::from_secs(30));

    assert_eq!(lifecycle.placements, vec![RacePlayerId(1)]);
    assert_eq!(lifecycle.first_finished_at, Some(now));

    lifecycle.reset();

    assert!(lifecycle.placements.is_empty());
    assert_eq!(lifecycle.first_finished_at, None);
}

#[test]
fn runtime_state_resets_transient_race_state() {
    let now = Instant::now();
    let mut runtime = super::RaceRuntimeState::<RacePlayerId, usize>::new();
    runtime.lifecycle.placements = vec![RacePlayerId(1)];
    runtime.lifecycle.first_finished_at = Some(now);
    runtime.bonus_attempts.insert(RacePlayerId(1), 0);
    runtime.spent_bonus_gaps.insert(RacePlayerId(1), 3);
    runtime
        .player_effects
        .insert(RacePlayerId(1), Default::default());

    runtime.reset();

    assert!(runtime.lifecycle.placements.is_empty());
    assert_eq!(runtime.lifecycle.first_finished_at, None);
    assert!(runtime.bonus_attempts.is_empty());
    assert!(runtime.spent_bonus_gaps.is_empty());
    assert!(runtime.player_effects.is_empty());
}

#[test]
fn lifecycle_timeout_places_unfinished_connected_racers_by_progress() {
    let now = Instant::now();
    let mut race = RaceState::new(track(&["one", "two"]));
    race.add_player(RacePlayerId(1), "tom", PlayerColorId::Cyan, now);
    race.add_player(RacePlayerId(2), "alex", PlayerColorId::Red, now);
    race.players[1].state.finished_at = Some(now);
    race.players[0].state.word_index = 1;
    let mut placements = vec![RacePlayerId(2)];
    let mut first_finished_at = Some(now);

    let update = update_race_lifecycle(
        &race,
        &mut placements,
        &mut first_finished_at,
        now + Duration::from_secs(30),
        Duration::from_secs(30),
    );

    assert_eq!(
        update.status,
        RaceLifecycleStatus::Finished {
            all_connected_finished: false,
            all_connected_disconnected: false,
            timeout_expired: true
        }
    );
    assert_eq!(placements, vec![RacePlayerId(2), RacePlayerId(1)]);
}
