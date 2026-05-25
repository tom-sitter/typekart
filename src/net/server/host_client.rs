//! Network-host per-client message adapter.
//!
//! The accept loop owns sockets and admission. Once a client is accepted, this
//! module owns its message loop and delegates gameplay/lobby mutations to the
//! smaller host adapter modules.

use std::{
    io::BufReader,
    net::TcpStream,
    sync::{Arc, Mutex},
    time::Instant,
};

use crate::{
    game::lobby::set_lobby_ready as shared_set_lobby_ready,
    net::{
        log::push_network_log,
        protocol::{ClientMessage, NetworkRacePhase, PlayerId},
        transport::read_client_message,
    },
};

use super::{
    HostState, broadcast_lobby_snapshot, broadcast_race_delta, broadcast_race_results_once,
    broadcast_race_snapshot, host_ai, host_disconnect,
    host_input::{self, NetworkInputOutcome},
    host_lobby, host_phase, print_lobby_snapshot, print_server_line, push_event,
};

pub(super) fn handle_client_messages(
    player_id: PlayerId,
    stream: TcpStream,
    state: Arc<Mutex<HostState>>,
) {
    let mut reader = BufReader::new(stream);
    loop {
        let message = match read_client_message(&mut reader) {
            Ok(Some(message)) => message,
            Ok(None) => break,
            Err(_) => continue,
        };

        match message {
            ClientMessage::Rename { name } => handle_rename(&state, player_id, &name),
            ClientMessage::SetReady { ready } => handle_ready(&state, player_id, ready),
            ClientMessage::StartCountdown if player_id == PlayerId(1) => {
                {
                    let state = state.lock().expect("host state poisoned");
                    push_network_log(&state.debug_log, "host requested countdown");
                }
                host_phase::start_countdown(Arc::clone(&state));
            }
            ClientMessage::StartCountdown => {
                print_server_line(format!(
                    "Ignoring start request from non-host player {}",
                    player_id.0
                ));
            }
            ClientMessage::AddAi if player_id == PlayerId(1) => handle_add_ai(&state),
            ClientMessage::RemoveLobbyPlayer { player_id: target } if player_id == PlayerId(1) => {
                handle_remove_lobby_player(&state, target);
            }
            ClientMessage::SetAiDifficulty {
                player_id: target,
                difficulty,
            } if player_id == PlayerId(1) => handle_set_ai_difficulty(&state, target, difficulty),
            ClientMessage::RestartRace if player_id == PlayerId(1) => {
                handle_return_to_lobby(&state)
            }
            ClientMessage::RestartRace => {
                print_server_line(format!(
                    "Ignoring rematch request from non-host player {}",
                    player_id.0
                ));
            }
            ClientMessage::KeyInput { key, .. } => handle_key_input(&state, player_id, key),
            ClientMessage::Leave => break,
            _ => {}
        }
    }

    handle_disconnect_after_loop(state, player_id);
}

fn handle_rename(state: &Arc<Mutex<HostState>>, player_id: PlayerId, name: &str) {
    let mut state = state.lock().expect("host state poisoned");
    if let Err(error) = host_lobby::rename_lobby_player(&mut state, player_id, name) {
        push_event(&mut state, error.to_string());
    }
    broadcast_lobby_update(&mut state);
}

fn handle_ready(state: &Arc<Mutex<HostState>>, player_id: PlayerId, ready: bool) {
    let mut state = state.lock().expect("host state poisoned");
    match shared_set_lobby_ready(&mut state.players, player_id, ready) {
        Ok(outcome) => {
            print_server_line(format!(
                "{} is {}",
                outcome.name,
                if outcome.ready { "ready" } else { "not ready" }
            ));
            push_event(
                &mut state,
                format!(
                    "{} {}",
                    outcome.name,
                    if outcome.ready { "ready" } else { "not ready" }
                ),
            );
            push_network_log(
                &state.debug_log,
                format!("{} ready={}", outcome.name, outcome.ready),
            );
        }
        Err(error) => push_event(&mut state, error.to_string()),
    }
    broadcast_lobby_update(&mut state);
}

fn handle_add_ai(state: &Arc<Mutex<HostState>>) {
    let mut state = state.lock().expect("host state poisoned");
    if let Err(error) = host_ai::add_lobby_ai_racer(&mut state) {
        push_event(&mut state, error.to_string());
    }
    broadcast_lobby_update(&mut state);
}

fn handle_remove_lobby_player(state: &Arc<Mutex<HostState>>, target: PlayerId) {
    let mut state = state.lock().expect("host state poisoned");
    if let Err(error) = host_lobby::remove_lobby_player(&mut state, target) {
        push_event(&mut state, error.to_string());
    }
    broadcast_lobby_update(&mut state);
}

fn handle_set_ai_difficulty(
    state: &Arc<Mutex<HostState>>,
    target: Option<PlayerId>,
    difficulty: crate::net::protocol::AiDifficultySnapshot,
) {
    let mut state = state.lock().expect("host state poisoned");
    if let Err(error) = host_ai::set_lobby_ai_difficulty(&mut state, target, difficulty.into()) {
        push_event(&mut state, error.to_string());
    }
    broadcast_lobby_update(&mut state);
}

fn handle_return_to_lobby(state: &Arc<Mutex<HostState>>) {
    let mut state = state.lock().expect("host state poisoned");
    if let Err(error) = host_phase::return_to_lobby(&mut state) {
        push_network_log(
            &state.debug_log,
            format!("failed to return to lobby: {error:#}"),
        );
    }
}

fn handle_key_input(
    state: &Arc<Mutex<HostState>>,
    player_id: PlayerId,
    key: crate::net::protocol::ProtocolKey,
) {
    let now = Instant::now();
    let mut state = state.lock().expect("host state poisoned");
    match host_input::apply_protocol_key_input(&mut state, player_id, key, now) {
        NetworkInputOutcome::Ignored => {}
        NetworkInputOutcome::Updated => {
            if let Err(error) = broadcast_race_delta(&mut state) {
                push_network_log(
                    &state.debug_log,
                    format!("failed to broadcast race delta: {error:#}"),
                );
            }
        }
        NetworkInputOutcome::Finished => {
            if let Err(error) = broadcast_race_snapshot(&mut state) {
                push_network_log(
                    &state.debug_log,
                    format!("failed to broadcast race snapshot: {error:#}"),
                );
            }
            print_server_line("Race finished");
            if let Err(error) = broadcast_race_results_once(&mut state) {
                push_network_log(
                    &state.debug_log,
                    format!("failed to broadcast race results: {error:#}"),
                );
            }
        }
    }
}

fn handle_disconnect_after_loop(state: Arc<Mutex<HostState>>, player_id: PlayerId) {
    let mut state = state.lock().expect("host state poisoned");
    let was_race_screen_phase = matches!(
        state.phase,
        NetworkRacePhase::Countdown { .. } | NetworkRacePhase::Racing | NetworkRacePhase::Finished
    );
    if host_disconnect::handle_player_disconnect(&mut state, player_id, Instant::now()) {
        return;
    }
    broadcast_lobby_update(&mut state);
    if was_race_screen_phase && let Err(error) = broadcast_race_snapshot(&mut state) {
        push_network_log(
            &state.debug_log,
            format!("failed to broadcast race snapshot: {error:#}"),
        );
    }
    if state.phase == NetworkRacePhase::Finished
        && let Err(error) = broadcast_race_results_once(&mut state)
    {
        push_network_log(
            &state.debug_log,
            format!("failed to broadcast race results: {error:#}"),
        );
    }
}

fn broadcast_lobby_update(state: &mut HostState) {
    print_lobby_snapshot(&state.players);
    if let Err(error) = broadcast_lobby_snapshot(state) {
        push_network_log(
            &state.debug_log,
            format!("failed to broadcast lobby snapshot: {error:#}"),
        );
    }
}
