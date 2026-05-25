//! Network-host race lifecycle adapter.
//!
//! Shared race status rules live in `game::race_flow`. This module owns the
//! network host's state mutations around those rules: preparing rematches,
//! resetting runtime state, and turning lifecycle outcomes into host events.

use std::time::Instant;

use anyhow::{Context, Result};

use crate::game::{
    host_session::{connected_racer_count, prepare_race_from_lobby},
    race_flow::advance_race_flow,
    track::Track,
};
use crate::net::{
    host_lifecycle::{finish_summary_log, finished_player_message},
    log::push_network_log,
    protocol::NetworkRacePhase,
};

use super::{HostState, POST_FIRST_FINISH_TIMEOUT, host_ai, host_lobby, push_event};

pub(super) fn current_race_connected_player_count(state: &HostState) -> usize {
    connected_racer_count(&state.race)
}

pub(super) fn reset_race_from_lobby(state: &mut HostState) -> Result<()> {
    host_lobby::cleanup_disconnected_waiting_players(state);

    let word_count = state.race.track.len();
    let track = Track::generate(&state.word_list, word_count)
        .context("failed to generate rematch track")?;
    let now = Instant::now();
    let prepared = prepare_race_from_lobby(&state.players, track, &state.word_list, now);
    state.race = prepared.race;
    host_ai::reset_network_ai_timing(state, now);

    state.bonuses = prepared.bonuses;
    state.runtime.reset();
    state.race_results_sent = false;
    state.events.clear();
    state.phase = NetworkRacePhase::WaitingForHost;
    push_network_log(
        &state.debug_log,
        format!("prepared rematch racers={}", state.race.players.len()),
    );

    Ok(())
}

pub(super) fn update_race_status(state: &mut HostState, now: Instant) {
    if state.phase != NetworkRacePhase::Racing {
        return;
    }

    let outcome = advance_race_flow(
        &mut state.runtime.lifecycle,
        &state.race,
        now,
        POST_FIRST_FINISH_TIMEOUT,
    );

    for finished in outcome.newly_finished {
        let message = finished_player_message(&finished);
        push_event(state, message.clone());
        push_network_log(&state.debug_log, message);
    }

    if let Some(summary) = outcome.finished {
        state.phase = NetworkRacePhase::Finished;
        push_event(state, "Race finished".to_string());
        push_network_log(&state.debug_log, finish_summary_log(&summary));
    }
}
