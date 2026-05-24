//! Network-host lifecycle formatting helpers.
//!
//! Gameplay lifecycle policy lives in `src/game/race_flow.rs`. This module only
//! converts shared race-flow outcomes into network-host feed/log text and
//! protocol result messages.

use std::time::Instant;

use typekart_protocol::{PlayerId, RaceResultRow, ServerMessage};

use crate::game::{
    race::{RacePlayerId, RaceState},
    race_flow::{RaceFinishedPlayer, RaceFinishedSummary},
    snapshot::{build_placement_snapshots, build_race_result_snapshots},
};

#[derive(Debug, Clone)]
pub struct RaceResultsMessage {
    pub message: ServerMessage,
    pub placements: Vec<PlayerId>,
    pub row_count: usize,
}

pub fn finished_player_message(finished: &RaceFinishedPlayer) -> String {
    format!("{}. {} finished", finished.placement, finished.name)
}

pub fn finish_summary_log(summary: &RaceFinishedSummary) -> String {
    format!(
        "race finished all_connected_finished={} all_connected_disconnected={} timeout_expired={}",
        summary.all_connected_finished, summary.all_connected_disconnected, summary.timeout_expired
    )
}

pub fn build_race_result_rows(
    race: &RaceState,
    placements: &[RacePlayerId],
    now: Instant,
) -> Vec<RaceResultRow> {
    build_race_result_snapshots(race, placements, now)
}

pub fn build_race_results_message(
    race: &RaceState,
    placements: &[RacePlayerId],
    now: Instant,
) -> RaceResultsMessage {
    let rows = build_race_result_rows(race, placements, now);
    let row_count = rows.len();
    let placements = build_placement_snapshots(placements);
    let message = ServerMessage::RaceResults {
        placements: placements.clone(),
        rows,
    };

    RaceResultsMessage {
        message,
        placements,
        row_count,
    }
}

#[cfg(test)]
mod tests {
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
}
