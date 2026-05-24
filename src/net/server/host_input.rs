//! Network-host input adapter.
//!
//! Shared typing and bonus rules live in `game`. This module translates
//! protocol-level input into those shared rules and reports whether the host
//! should broadcast a race update or final results.

use std::time::Instant;

use crate::{
    game::{
        input_rules::player_input_is_paused as shared_player_input_is_paused, race::RacePlayerId,
        typing::KeyAction,
    },
    net::{
        log::push_network_log,
        protocol::{NetworkRacePhase, PlayerId, ProtocolKey},
    },
};

use super::{HostState, host_bonus, host_race};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NetworkInputOutcome {
    Ignored,
    Updated,
    Finished,
}

pub(super) fn apply_protocol_key_input(
    state: &mut HostState,
    player_id: PlayerId,
    key: ProtocolKey,
    now: Instant,
) -> NetworkInputOutcome {
    if state.phase != NetworkRacePhase::Racing {
        return NetworkInputOutcome::Ignored;
    }

    let action = protocol_key_to_action(key);
    push_network_log(
        &state.debug_log,
        format!("input player={} key={key:?}", player_id.0),
    );
    if !host_bonus::apply_network_key_input(state, player_id, action, now) {
        return NetworkInputOutcome::Ignored;
    }

    update_status_after_input(state, now)
}

pub(super) fn apply_line_input_to_race(
    state: &mut HostState,
    player_id: PlayerId,
    line: &str,
    now: Instant,
) -> NetworkInputOutcome {
    if state.phase != NetworkRacePhase::Racing {
        return NetworkInputOutcome::Ignored;
    }

    for ch in line.chars() {
        let action = if ch == ' ' {
            KeyAction::Space
        } else {
            KeyAction::Char(ch)
        };
        host_bonus::apply_network_key_input(state, player_id, action, now);
    }
    host_bonus::apply_network_key_input(state, player_id, KeyAction::Space, now);

    update_status_after_input(state, now)
}

pub(super) fn player_input_is_paused(state: &HostState, player_id: PlayerId, now: Instant) -> bool {
    shared_player_input_is_paused(
        &state.race,
        &state.runtime.player_effects,
        RacePlayerId(player_id.0),
        now,
    )
}

pub(super) fn player_label(state: &HostState, player_id: PlayerId) -> String {
    player_name(state, player_id).unwrap_or_else(|| format!("player {}", player_id.0))
}

pub(super) fn player_name(state: &HostState, id: PlayerId) -> Option<String> {
    state
        .race
        .players
        .iter()
        .find(|player| player.id == RacePlayerId(id.0))
        .map(|player| player.name.clone())
}

fn update_status_after_input(state: &mut HostState, now: Instant) -> NetworkInputOutcome {
    host_race::update_race_status(state, now);
    if state.phase == NetworkRacePhase::Finished {
        NetworkInputOutcome::Finished
    } else {
        NetworkInputOutcome::Updated
    }
}

fn protocol_key_to_action(key: ProtocolKey) -> KeyAction {
    match key {
        ProtocolKey::Char(' ') | ProtocolKey::Space => KeyAction::Space,
        ProtocolKey::Char(ch) => KeyAction::Char(ch),
        ProtocolKey::Backspace => KeyAction::Backspace,
    }
}
