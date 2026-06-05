//! Network-host lifecycle formatting helpers.
//!
//! Gameplay lifecycle policy lives in `src/game/race_flow.rs`. This module only
//! converts shared race-flow outcomes into network-host feed/log text and
//! protocol result messages.

use std::time::Instant;

use typekart_protocol::{PlayerId, RaceResultRow, ServerMessage};

use crate::game::{
    host_session::{HostEvent, finalize_host_race_results},
    race::{RacePlayerId, RaceState},
    race_flow::{RaceFinishedPlayer, RaceFinishedSummary},
};

#[derive(Debug, Clone)]
pub struct RaceResultsMessage {
    pub message: ServerMessage,
    pub placements: Vec<PlayerId>,
    pub row_count: usize,
}

pub fn finished_player_message(finished: &RaceFinishedPlayer) -> String {
    HostEvent::PlayerFinished {
        placement: finished.placement,
        name: finished.name.clone(),
    }
    .message()
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
    finalize_host_race_results(race, placements, now).rows
}

pub fn build_race_results_message(
    race: &RaceState,
    placements: &[RacePlayerId],
    now: Instant,
) -> RaceResultsMessage {
    let results = finalize_host_race_results(race, placements, now);
    let rows = results.rows;
    let row_count = rows.len();
    let placements = results.placements;
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
mod tests;
