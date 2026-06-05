//! Network-host TCP accept loop adapter.
//!
//! Startup binds the listener and builds host state. This module owns the
//! steady-state incoming connection loop: read handshake, admit joiner, spawn
//! the per-client message loop, and advance the next human id hint.

use std::{
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
};

use anyhow::{Context, Result};

use super::{
    HostState, broadcast_lobby_snapshot, broadcast_race_snapshot, host_client, host_handshake,
    host_join, print_server_line,
};
use crate::net::protocol::NetworkRacePhase;

pub(super) fn run_accept_loop(
    listener: TcpListener,
    state: Arc<Mutex<HostState>>,
    mut next_player_id: u64,
) -> Result<()> {
    for stream in listener.incoming() {
        let stream = stream.context("failed to accept client connection")?;
        let peer = stream.peer_addr().ok();

        let join_hello = match host_handshake::read_join_hello(&stream) {
            Ok(join_hello) => join_hello,
            Err(error) => {
                print_server_line(format!("Rejected connection: {error:#}"));
                continue;
            }
        };
        let (player_id, assigned_color, player_name) = {
            let mut state = state.lock().expect("host state poisoned");
            let Some(accepted_join) =
                host_join::admit_joiner(&mut state, &stream, join_hello, next_player_id)?
            else {
                continue;
            };
            broadcast_lobby_snapshot(&mut state)?;
            if joiner_needs_active_race_snapshot(state.phase) {
                broadcast_race_snapshot(&mut state)?;
            }

            (
                accepted_join.player_id,
                accepted_join.assigned_color,
                accepted_join.player_name,
            )
        };

        print_server_line(format!(
            "{} joined as player {} ({assigned_color:?}){}",
            player_name,
            player_id.0,
            peer.map(|addr| format!(" from {addr}")).unwrap_or_default()
        ));

        let state_for_client = Arc::clone(&state);
        thread::spawn(move || {
            host_client::handle_client_messages(player_id, stream, state_for_client)
        });
        next_player_id = next_player_id.max(player_id.0 + 1);
    }

    Ok(())
}

fn joiner_needs_active_race_snapshot(phase: NetworkRacePhase) -> bool {
    matches!(
        phase,
        NetworkRacePhase::Countdown { .. } | NetworkRacePhase::Racing | NetworkRacePhase::Finished
    )
}

#[cfg(test)]
mod tests;
