//! Network-host race snapshot adapter.
//!
//! Shared snapshot projection lives in `game::snapshot`. This module owns the
//! network host's sequencing, cooldown refresh, player-kind mapping, and
//! snapshot broadcast logging.

use std::time::Instant;

use crate::game::snapshot::{
    RaceDeltaSnapshotInput, RaceSnapshotInput,
    build_race_delta_snapshot as build_shared_race_delta_snapshot,
    build_race_snapshot as build_shared_race_snapshot,
};
use crate::net::{
    log::push_network_log,
    protocol::{NetworkRacePhase, PlayerId, PlayerKind, RaceDeltaSnapshot, RaceSnapshot},
};

use super::{HostState, expire_bonus_cooldowns};

pub(super) fn build_race_snapshot(state: &mut HostState) -> RaceSnapshot {
    let now = Instant::now();
    expire_bonus_cooldowns(state, now);

    state.snapshot_sequence += 1;
    build_shared_race_snapshot(
        RaceSnapshotInput {
            sequence: state.snapshot_sequence,
            phase: state.phase,
            mod_config: (&state.active_mod_config).into(),
            race: &state.race,
            bonuses: &state.bonuses,
            player_effects: &state.runtime.player_effects,
            events: state.events.clone(),
            now,
        },
        |player_id| player_kind(state, player_id),
    )
}

pub(super) fn build_race_delta_snapshot(state: &mut HostState) -> RaceDeltaSnapshot {
    let now = Instant::now();
    expire_bonus_cooldowns(state, now);

    state.snapshot_sequence += 1;
    build_shared_race_delta_snapshot(
        RaceDeltaSnapshotInput {
            sequence: state.snapshot_sequence,
            phase: state.phase,
            race: &state.race,
            bonuses: &state.bonuses,
            player_effects: &state.runtime.player_effects,
            events: state.events.clone(),
            now,
        },
        |player_id| player_kind(state, player_id),
    )
}

pub(super) fn log_race_snapshot(state: &HostState) {
    match state.phase {
        NetworkRacePhase::Countdown { remaining_seconds } => push_network_log(
            &state.debug_log,
            format!(
                "broadcast snapshot seq={} phase=countdown remaining={remaining_seconds}",
                state.snapshot_sequence
            ),
        ),
        NetworkRacePhase::Racing if state.snapshot_sequence.is_multiple_of(20) => push_network_log(
            &state.debug_log,
            format!(
                "broadcast snapshot seq={} phase=racing",
                state.snapshot_sequence
            ),
        ),
        NetworkRacePhase::Finished => push_network_log(
            &state.debug_log,
            format!(
                "broadcast snapshot seq={} phase=finished",
                state.snapshot_sequence
            ),
        ),
        _ => {}
    }
}

pub(super) fn log_race_delta(state: &HostState) {
    match state.phase {
        NetworkRacePhase::Racing if state.snapshot_sequence.is_multiple_of(20) => push_network_log(
            &state.debug_log,
            format!(
                "broadcast delta seq={} phase=racing",
                state.snapshot_sequence
            ),
        ),
        NetworkRacePhase::Finished => push_network_log(
            &state.debug_log,
            format!(
                "broadcast delta seq={} phase=finished",
                state.snapshot_sequence
            ),
        ),
        _ => {}
    }
}

fn player_kind(state: &HostState, player_id: PlayerId) -> PlayerKind {
    state
        .players
        .iter()
        .find(|player| player.id == player_id)
        .map(|player| player.kind)
        .unwrap_or(PlayerKind::Human)
}
