//! Network-host disconnect adapter.
//!
//! This module owns TCP-host cleanup after a player connection ends: marking
//! lobby/race participants disconnected, clearing transient runtime state,
//! notifying joiners when the host leaves, and reconciling active race phases.

use std::{net::Shutdown, time::Instant};

use crate::{
    game::race::RacePlayerId,
    net::{
        log::push_network_log,
        protocol::{PlayerId, PlayerKind, ServerMessage},
    },
};

use super::{
    HostState, host_lobby, host_phase, print_server_line, push_event, write_server_message,
};

pub(super) fn handle_player_disconnect(
    state: &mut HostState,
    player_id: PlayerId,
    now: Instant,
) -> bool {
    if let Some(player) = state
        .players
        .iter_mut()
        .find(|player| player.id == player_id)
    {
        let name = player.name.clone();
        player.connected = false;
        player.ready = false;
        print_server_line(format!("{name} disconnected"));
        push_event(state, format!("{name} disconnected"));
        push_network_log(&state.debug_log, format!("{name} disconnected"));
    }
    if let Some(player) = state
        .race
        .players
        .iter_mut()
        .find(|player| player.id == RacePlayerId(player_id.0))
    {
        player.connected = false;
    }
    state.runtime.bonus_attempts.remove(&player_id);
    state.runtime.spent_bonus_gaps.remove(&player_id);
    state
        .runtime
        .player_effects
        .remove(&RacePlayerId(player_id.0));
    state.clients.retain(|client| client.player_id != player_id);

    if player_id == PlayerId(1) {
        close_game_for_joiners(state, "Game closed: host left");
        return true;
    }

    host_phase::reconcile_phase_after_disconnect(state, now);
    host_lobby::cleanup_disconnected_waiting_players(state);
    false
}

fn close_game_for_joiners(state: &mut HostState, message: &str) {
    push_event(state, message.to_string());
    push_network_log(&state.debug_log, message);

    let message = ServerMessage::Error {
        message: message.to_string(),
    };
    for client in &mut state.clients {
        if let Err(error) = write_server_message(&mut client.stream, &message) {
            push_network_log(
                &state.debug_log,
                format!(
                    "failed to send close message to player {}: {error:#}",
                    client.player_id.0
                ),
            );
        }
        let _ = client.stream.shutdown(Shutdown::Both);
    }
    state.clients.clear();

    for player in &mut state.players {
        if player.kind == PlayerKind::Human {
            player.connected = false;
            player.ready = false;
        }
    }
    for player in &mut state.race.players {
        player.connected = false;
    }
}
