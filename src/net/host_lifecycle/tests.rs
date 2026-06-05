use std::time::Instant;

use super::{build_race_results_message, finish_summary_log, finished_player_message};
use crate::game::{
    race::{PlayerColorId, RacePlayerId, RaceState},
    race_flow::{RaceFinishedPlayer, RaceFinishedSummary},
    track::Track,
};

#[test]
fn formats_finished_player_message() {
    let message = finished_player_message(&RaceFinishedPlayer {
        player_id: RacePlayerId(2),
        placement: 1,
        name: "alex".to_string(),
    });

    assert_eq!(message, "1. alex finished");
}

#[test]
fn formats_finish_summary_log() {
    let message = finish_summary_log(&RaceFinishedSummary {
        all_connected_finished: true,
        all_connected_disconnected: false,
        timeout_expired: false,
    });

    assert!(message.contains("all_connected_finished=true"));
    assert!(message.contains("timeout_expired=false"));
}

#[test]
fn builds_race_results_protocol_message() {
    let now = Instant::now();
    let mut race = RaceState::new(Track::new(vec!["go".to_string()]));
    race.add_player(RacePlayerId(1), "host", PlayerColorId::Cyan, now);
    race.players[0].state.finished_at = Some(now);

    let results = build_race_results_message(&race, &[RacePlayerId(1)], now);

    assert_eq!(results.placements, vec![typekart_protocol::PlayerId(1)]);
    assert_eq!(results.row_count, 1);
    assert!(matches!(
        results.message,
        typekart_protocol::ServerMessage::RaceResults { .. }
    ));
}
