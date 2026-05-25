//! Network-host join admission adapter.
//!
//! The accept loop owns incoming TCP streams and the initial handshake. This
//! module owns admission policy after `Hello`: version checks, lobby capacity,
//! player ids, colors, welcome responses, and lobby/race roster mutation.

use std::{net::TcpStream, time::Instant};

use anyhow::{Context, Result};

use crate::{
    game::{
        lobby::{
            connected_player_count, first_available_color, first_available_player_id,
            new_human_lobby_player, unique_lobby_name,
        },
        race::RacePlayerId,
    },
    net::{
        log::push_network_log,
        protocol::{
            AssignedColor, NetworkRacePhase, PlayerId, ServerMessage, version_mismatch_message,
        },
    },
};

use super::{
    ConnectedClient, HostState, host_handshake, print_lobby_snapshot, push_event,
    send_server_message,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AcceptedJoin {
    pub(super) player_id: PlayerId,
    pub(super) assigned_color: AssignedColor,
    pub(super) player_name: String,
}

pub(super) fn admit_joiner(
    state: &mut HostState,
    stream: &TcpStream,
    join_hello: host_handshake::JoinHello,
    next_player_id: u64,
) -> Result<Option<AcceptedJoin>> {
    let requested_player_name = join_hello.name;

    if join_hello.client_version != env!("CARGO_PKG_VERSION") {
        send_error(
            stream,
            version_mismatch_message(env!("CARGO_PKG_VERSION"), &join_hello.client_version),
        )?;
        push_network_log(
            &state.debug_log,
            format!(
                "join rejected: version mismatch name={} host_version={} client_version={}",
                requested_player_name,
                env!("CARGO_PKG_VERSION"),
                join_hello.client_version
            ),
        );
        return Ok(None);
    }

    let connected_players = connected_player_count(&state.players);
    if connected_players >= state.max_players {
        send_error(
            stream,
            format!(
                "Lobby is full: {connected_players}/{} connected players",
                state.max_players
            ),
        )?;
        push_network_log(
            &state.debug_log,
            format!(
                "join rejected: lobby full {connected_players}/{}",
                state.max_players
            ),
        );
        return Ok(None);
    }

    let player_name = unique_lobby_name(state.players.iter(), &requested_player_name);
    let player_id = first_available_player_id(&state.players, next_player_id);
    let Some(assigned_color) = first_available_color(&state.players) else {
        send_error(stream, "Lobby is full: no colors available".to_string())?;
        push_network_log(&state.debug_log, "join rejected: no colors available");
        return Ok(None);
    };

    let write_stream = host_handshake::welcome_joiner(stream, player_id, assigned_color)?;
    state.clients.push(ConnectedClient {
        player_id,
        stream: write_stream,
    });
    state.players.push(new_human_lobby_player(
        player_id,
        player_name.clone(),
        assigned_color,
    ));
    if matches!(
        state.phase,
        NetworkRacePhase::Lobby | NetworkRacePhase::WaitingForHost
    ) {
        state.race.add_player(
            RacePlayerId(player_id.0),
            player_name.clone(),
            assigned_color.into(),
            Instant::now(),
        );
    }
    push_event(state, format!("{player_name} joined"));
    push_network_log(
        &state.debug_log,
        format!(
            "{player_name} joined player={} color={assigned_color:?}",
            player_id.0
        ),
    );
    print_lobby_snapshot(&state.players);

    Ok(Some(AcceptedJoin {
        player_id,
        assigned_color,
        player_name,
    }))
}

fn send_error(stream: &TcpStream, message: String) -> Result<()> {
    send_server_message(
        stream
            .try_clone()
            .context("failed to clone client stream for join error")?,
        &ServerMessage::Error { message },
    )
}
