//! Shared race lifecycle helpers.
//!
//! This module keeps finish/timeout/rematch policy out of host adapters. Hosts
//! still decide how to log, broadcast, and render lifecycle changes.

use std::time::{Duration, Instant};

use super::race::{
    RaceLifecycleState, RaceLifecycleStatus, RaceLifecycleUpdate, RacePlayerId, RaceRuntimeState,
    RaceState,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaceFinishedPlayer {
    pub player_id: RacePlayerId,
    pub placement: usize,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaceFinishedSummary {
    pub all_connected_finished: bool,
    pub all_connected_disconnected: bool,
    pub timeout_expired: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaceFlowOutcome {
    pub newly_finished: Vec<RaceFinishedPlayer>,
    pub finished: Option<RaceFinishedSummary>,
}

pub fn update_race_flow(
    lifecycle: &mut RaceLifecycleState,
    race: &RaceState,
    now: Instant,
    post_first_finish_timeout: Duration,
) -> RaceLifecycleUpdate {
    lifecycle.update(race, now, post_first_finish_timeout)
}

pub fn advance_race_flow(
    lifecycle: &mut RaceLifecycleState,
    race: &RaceState,
    now: Instant,
    post_first_finish_timeout: Duration,
) -> RaceFlowOutcome {
    let update = update_race_flow(lifecycle, race, now, post_first_finish_timeout);
    race_flow_outcome(race, lifecycle, update)
}

pub fn race_flow_is_finished(update: &RaceLifecycleUpdate) -> bool {
    matches!(update.status, RaceLifecycleStatus::Finished { .. })
}

pub fn reset_race_runtime<PlayerId, BonusAttempt>(
    runtime: &mut RaceRuntimeState<PlayerId, BonusAttempt>,
) {
    runtime.reset();
}

fn race_flow_outcome(
    race: &RaceState,
    lifecycle: &RaceLifecycleState,
    update: RaceLifecycleUpdate,
) -> RaceFlowOutcome {
    let newly_finished = update
        .newly_finished
        .into_iter()
        .map(|player_id| RaceFinishedPlayer {
            player_id,
            placement: lifecycle
                .placements
                .iter()
                .position(|placed| *placed == player_id)
                .map(|index| index + 1)
                .unwrap_or(lifecycle.placements.len()),
            name: race
                .player(player_id)
                .map(|player| player.name.clone())
                .unwrap_or_else(|| format!("player {}", player_id.0)),
        })
        .collect();

    let finished = match update.status {
        RaceLifecycleStatus::Running => None,
        RaceLifecycleStatus::Finished {
            all_connected_finished,
            all_connected_disconnected,
            timeout_expired,
        } => Some(RaceFinishedSummary {
            all_connected_finished,
            all_connected_disconnected,
            timeout_expired,
        }),
    };

    RaceFlowOutcome {
        newly_finished,
        finished,
    }
}

#[cfg(test)]
mod tests;
