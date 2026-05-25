//! Network-host console command adapter.
//!
//! This module owns the stdin command loop used by the embedded terminal host.
//! It translates host commands into lobby/race adapter calls while shared game
//! rules remain in `game` modules.

use std::{
    io::{self, BufRead},
    sync::{Arc, Mutex},
    thread,
    time::Instant,
};

use crate::{
    game::lobby::set_lobby_ready as shared_set_lobby_ready,
    net::{
        log::push_network_log,
        protocol::{NetworkRacePhase, PlayerId},
    },
};

use super::{
    HostState, broadcast_lobby_snapshot, broadcast_race_delta, broadcast_race_results_once,
    broadcast_race_snapshot,
    host_input::{self, NetworkInputOutcome},
    host_phase, print_lobby_snapshot, print_server_line, push_event,
};

pub(super) fn spawn_host_command_loop(state: Arc<Mutex<HostState>>) {
    thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            let Ok(command) = line else {
                break;
            };

            match command.trim() {
                "ready" => update_host_ready(&state, true),
                "unready" => update_host_ready(&state, false),
                "lobby" => {
                    let state = state.lock().expect("host state poisoned");
                    print_lobby_snapshot(&state.players);
                }
                "start" => host_phase::start_countdown(Arc::clone(&state)),
                "" if command == " " => host_phase::start_countdown(Arc::clone(&state)),
                "" => {}
                other => {
                    if host_phase::current_phase(&state) == NetworkRacePhase::Racing {
                        apply_line_input(&state, PlayerId(1), other);
                    } else {
                        let state = state.lock().expect("host state poisoned");
                        push_network_log(
                            &state.debug_log,
                            format!("unknown host command: {other}"),
                        );
                        print_server_line(format!("Unknown host command: {other}"));
                    }
                }
            }
        }
    });
}

pub(super) fn has_embedded_host_player(state: &Arc<Mutex<HostState>>) -> bool {
    state
        .lock()
        .expect("host state poisoned")
        .players
        .iter()
        .any(|player| player.id == PlayerId(1))
}

pub(super) fn update_host_ready(state: &Arc<Mutex<HostState>>, ready: bool) {
    let mut state = state.lock().expect("host state poisoned");
    match shared_set_lobby_ready(&mut state.players, PlayerId(1), ready) {
        Ok(outcome) => {
            print_server_line(format!(
                "{} is {}",
                outcome.name,
                if outcome.ready { "ready" } else { "not ready" }
            ));
            push_network_log(
                &state.debug_log,
                format!("{} ready={}", outcome.name, outcome.ready),
            );
        }
        Err(error) => push_event(&mut state, error.to_string()),
    }
    print_lobby_snapshot(&state.players);
    if let Err(error) = broadcast_lobby_snapshot(&mut state) {
        push_network_log(
            &state.debug_log,
            format!("failed to broadcast lobby snapshot: {error:#}"),
        );
    }
}

fn apply_line_input(state: &Arc<Mutex<HostState>>, player_id: PlayerId, line: &str) {
    let now = Instant::now();
    let mut state = state.lock().expect("host state poisoned");
    match host_input::apply_line_input_to_race(&mut state, player_id, line, now) {
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
            push_network_log(&state.debug_log, "race finished after host line input");
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
