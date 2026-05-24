//! Network-host broadcast adapter.
//!
//! The server decides when messages should be broadcast. This module owns
//! building host-side protocol messages, writing them to connected TCP clients,
//! and pruning clients whose streams can no longer be written.

use std::time::Instant;

use anyhow::Result;

use crate::net::{
    host_lifecycle::build_race_results_message,
    log::push_network_log,
    protocol::{PlayerId, ServerMessage},
};

use super::{HostState, host_snapshots, write_server_message};

pub(super) fn broadcast_lobby_snapshot(state: &mut HostState) -> Result<()> {
    let snapshot = ServerMessage::LobbySnapshot {
        players: state.players.clone(),
        host_id: PlayerId(1),
        mod_config: (&state.active_mod_config).into(),
        events: state.events.clone(),
    };

    broadcast_server_message_to_clients(state, &snapshot, "lobby snapshot")
}

pub(super) fn broadcast_race_snapshot(state: &mut HostState) -> Result<()> {
    let snapshot = ServerMessage::RaceSnapshot(host_snapshots::build_race_snapshot(state));
    host_snapshots::log_race_snapshot(state);
    broadcast_server_message_to_clients(state, &snapshot, "race snapshot")
}

pub(super) fn broadcast_race_delta(state: &mut HostState) -> Result<()> {
    let delta = ServerMessage::RaceDelta(host_snapshots::build_race_delta_snapshot(state));
    host_snapshots::log_race_delta(state);
    broadcast_server_message_to_clients(state, &delta, "race delta")
}

pub(super) fn broadcast_race_results_once(state: &mut HostState) -> Result<()> {
    if state.race_results_sent {
        push_network_log(&state.debug_log, "skipped duplicate race results broadcast");
        return Ok(());
    }

    broadcast_race_results(state)?;
    state.race_results_sent = true;
    Ok(())
}

fn broadcast_race_results(state: &mut HostState) -> Result<()> {
    let results = build_race_results_message(
        &state.race,
        &state.runtime.lifecycle.placements,
        Instant::now(),
    );
    push_network_log(
        &state.debug_log,
        format!(
            "broadcast race results placements={:?} rows={}",
            results.placements, results.row_count
        ),
    );

    broadcast_server_message_to_clients(state, &results.message, "race results")
}

fn broadcast_server_message_to_clients(
    state: &mut HostState,
    message: &ServerMessage,
    label: &str,
) -> Result<()> {
    let mut failed_clients = Vec::new();
    for client in state.clients.iter_mut() {
        if let Err(error) = write_server_message(&mut client.stream, message) {
            push_network_log(
                &state.debug_log,
                format!(
                    "failed to send {label} to player {}: {error:#}",
                    client.player_id.0
                ),
            );
            failed_clients.push(client.player_id);
        }
    }

    state
        .clients
        .retain(|client| !failed_clients.contains(&client.player_id));
    Ok(())
}
