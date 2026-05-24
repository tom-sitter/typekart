//! Network-host lobby adapter.
//!
//! Shared lobby policy lives in `game::lobby`. This module owns the TCP host's
//! side effects around that policy: updating race roster mirrors, closing
//! kicked client sockets, clearing runtime state, and logging lobby changes.

use std::net::Shutdown;

use anyhow::Result;

use crate::game::{
    lobby::{
        remove_lobby_player as shared_remove_lobby_player,
        rename_lobby_player as shared_rename_lobby_player,
    },
    race::RacePlayerId,
};
use crate::net::{
    log::push_network_log,
    protocol::{PlayerId, PlayerKind, ServerMessage},
};

use super::{HostState, push_event, write_server_message};

pub(super) fn remove_lobby_player(state: &mut HostState, player_id: PlayerId) -> Result<()> {
    let removed = shared_remove_lobby_player(&mut state.players, state.phase, player_id)?;
    let name = removed.player.name;
    let kind = removed.player.kind;

    if kind == PlayerKind::Human {
        for client in state
            .clients
            .iter_mut()
            .filter(|client| client.player_id == player_id)
        {
            let _ = write_server_message(
                &mut client.stream,
                &ServerMessage::Error {
                    message: "Kicked by host".to_string(),
                },
            );
            let _ = client.stream.shutdown(Shutdown::Both);
        }
        state.clients.retain(|client| client.player_id != player_id);
    }

    state
        .race
        .players
        .retain(|player| player.id != RacePlayerId(player_id.0));
    state.ai_racers.remove(&player_id);
    state.runtime.bonus_attempts.remove(&player_id);
    state.runtime.spent_bonus_gaps.remove(&player_id);
    state
        .runtime
        .player_effects
        .remove(&RacePlayerId(player_id.0));
    push_event(
        state,
        match kind {
            PlayerKind::Human => format!("{name} kicked"),
            PlayerKind::Bot => format!("{name} removed"),
        },
    );
    push_network_log(
        &state.debug_log,
        format!(
            "lobby removed player={} name={name} kind={kind:?}",
            player_id.0
        ),
    );

    Ok(())
}

pub(super) fn rename_lobby_player(
    state: &mut HostState,
    player_id: PlayerId,
    requested_name: &str,
) -> Result<()> {
    let outcome =
        shared_rename_lobby_player(&mut state.players, state.phase, player_id, requested_name)?;
    if let Some(racer) = state
        .race
        .players
        .iter_mut()
        .find(|racer| racer.id == RacePlayerId(player_id.0))
    {
        racer.name = outcome.new_name.clone();
    }
    push_event(
        state,
        format!("{} renamed to {}", outcome.previous_name, outcome.new_name),
    );
    push_network_log(
        &state.debug_log,
        format!(
            "player={} renamed {} -> {}",
            outcome.player_id.0, outcome.previous_name, outcome.new_name
        ),
    );

    Ok(())
}

pub(super) fn cleanup_disconnected_waiting_players(state: &mut HostState) {
    if !matches!(
        state.phase,
        crate::net::protocol::NetworkRacePhase::Lobby
            | crate::net::protocol::NetworkRacePhase::WaitingForHost
            | crate::net::protocol::NetworkRacePhase::Finished
    ) {
        return;
    }

    let disconnected_ids = state
        .players
        .iter()
        .filter(|player| !player.connected)
        .map(|player| player.id)
        .collect::<Vec<_>>();
    if disconnected_ids.is_empty() {
        return;
    }

    state
        .players
        .retain(|player| !disconnected_ids.contains(&player.id));
    state
        .race
        .players
        .retain(|player| !disconnected_ids.contains(&PlayerId(player.id.0)));
    push_network_log(
        &state.debug_log,
        format!("cleaned up disconnected waiting players={disconnected_ids:?}"),
    );
}
