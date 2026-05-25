//! Network-host phase and ticking adapter.
//!
//! This module owns host-side countdown timing, race snapshot ticks, rematch
//! returns, and the bridge between periodic ticking and shared race/item/AI
//! rules.

use std::{
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use anyhow::Result;

use crate::net::{log::push_network_log, protocol::NetworkRacePhase};

use super::{
    HostState, RACE_SNAPSHOT_INTERVAL, broadcast_lobby_snapshot, broadcast_race_delta,
    broadcast_race_results_once, broadcast_race_snapshot, expire_bonus_cooldowns, host_ai,
    host_items, host_race, print_server_line, push_event,
};

pub(super) fn start_countdown(state: Arc<Mutex<HostState>>) {
    let should_start = {
        let mut state = state.lock().expect("host state poisoned");
        match state.phase {
            NetworkRacePhase::Lobby => {
                if let Err(error) = host_race::reset_race_from_lobby(&mut state) {
                    push_network_log(
                        &state.debug_log,
                        format!("failed to prepare race from lobby: {error:#}"),
                    );
                    return;
                }
            }
            NetworkRacePhase::WaitingForHost => {}
            NetworkRacePhase::Finished => {
                if let Err(error) = host_race::reset_race_from_lobby(&mut state) {
                    push_network_log(
                        &state.debug_log,
                        format!("failed to prepare rematch: {error:#}"),
                    );
                    return;
                }
            }
            NetworkRacePhase::Countdown { .. } | NetworkRacePhase::Racing => {
                push_network_log(&state.debug_log, "start ignored race already active");
                print_server_line("Race has already started");
                return;
            }
        }

        if host_race::current_race_connected_player_count(&state) < 1 {
            push_network_log(
                &state.debug_log,
                "start ignored no ready connected racers available",
            );
            print_server_line("Cannot start: at least one ready connected racer is required");
            return;
        }

        state.phase = NetworkRacePhase::Countdown {
            remaining_seconds: 3,
        };
        push_event(&mut state, "Countdown started".to_string());
        push_network_log(&state.debug_log, "countdown started remaining=3");
        print_server_line("Countdown: 3");
        if let Err(error) = broadcast_race_snapshot(&mut state) {
            push_network_log(
                &state.debug_log,
                format!("failed to broadcast race snapshot: {error:#}"),
            );
        }
        true
    };

    if should_start {
        thread::spawn(move || run_countdown(state));
    }
}

pub(super) fn return_to_lobby(state: &mut HostState) -> Result<()> {
    let event = match state.phase {
        NetworkRacePhase::Countdown { .. } | NetworkRacePhase::Racing => "Race cancelled",
        NetworkRacePhase::Finished => "Returned to lobby",
        NetworkRacePhase::Lobby | NetworkRacePhase::WaitingForHost => return Ok(()),
    };
    host_race::reset_race_from_lobby(state)?;
    push_event(state, event.to_string());
    push_network_log(&state.debug_log, event.to_ascii_lowercase());
    if let Err(error) = broadcast_race_snapshot(state) {
        push_network_log(
            &state.debug_log,
            format!("failed to broadcast lobby race snapshot: {error:#}"),
        );
    }
    if let Err(error) = broadcast_lobby_snapshot(state) {
        push_network_log(
            &state.debug_log,
            format!("failed to broadcast lobby snapshot: {error:#}"),
        );
    }

    Ok(())
}

pub(super) fn current_phase(state: &Arc<Mutex<HostState>>) -> NetworkRacePhase {
    state.lock().expect("host state poisoned").phase
}

pub(super) fn reconcile_phase_after_disconnect(state: &mut HostState, now: Instant) {
    match state.phase {
        NetworkRacePhase::Countdown { .. } => {
            if !countdown_has_any_connected_racer(state) {
                cancel_countdown(state);
            }
        }
        NetworkRacePhase::Racing => host_race::update_race_status(state, now),
        NetworkRacePhase::Finished => {}
        NetworkRacePhase::Lobby | NetworkRacePhase::WaitingForHost => {}
    }
}

pub(super) fn countdown_has_any_connected_racer(state: &HostState) -> bool {
    state
        .race
        .players
        .iter()
        .filter(|player| player.connected && !player.state.is_finished())
        .count()
        >= 1
}

pub(super) fn cancel_countdown(state: &mut HostState) {
    state.phase = NetworkRacePhase::WaitingForHost;
    push_event(state, "Countdown cancelled".to_string());
    push_network_log(&state.debug_log, "countdown cancelled no connected racers");
    print_server_line("Countdown cancelled");
}

fn run_countdown(state: Arc<Mutex<HostState>>) {
    for remaining_seconds in [2, 1] {
        thread::sleep(Duration::from_secs(1));
        let mut guard = state.lock().expect("host state poisoned");
        if !matches!(guard.phase, NetworkRacePhase::Countdown { .. }) {
            push_network_log(&guard.debug_log, "countdown stopped before next tick");
            return;
        }
        if !countdown_has_any_connected_racer(&guard) {
            cancel_countdown(&mut guard);
            broadcast_countdown_cancel(&mut guard);
            return;
        }

        guard.phase = NetworkRacePhase::Countdown { remaining_seconds };
        push_network_log(
            &guard.debug_log,
            format!("countdown tick remaining={remaining_seconds}"),
        );
        print_server_line(format!("Countdown: {remaining_seconds}"));
        if let Err(error) = broadcast_race_snapshot(&mut guard) {
            push_network_log(
                &guard.debug_log,
                format!("failed to broadcast race snapshot: {error:#}"),
            );
        }
    }

    thread::sleep(Duration::from_secs(1));
    let mut guard = state.lock().expect("host state poisoned");
    if !matches!(guard.phase, NetworkRacePhase::Countdown { .. }) {
        push_network_log(&guard.debug_log, "countdown stopped before race start");
        return;
    }
    if !countdown_has_any_connected_racer(&guard) {
        cancel_countdown(&mut guard);
        broadcast_countdown_cancel(&mut guard);
        return;
    }

    guard.phase = NetworkRacePhase::Racing;
    host_ai::reset_network_ai_timing(&mut guard, Instant::now());
    push_event(&mut guard, "Race started".to_string());
    push_network_log(&guard.debug_log, "race started");
    print_server_line("Race started");
    if let Err(error) = broadcast_race_snapshot(&mut guard) {
        push_network_log(
            &guard.debug_log,
            format!("failed to broadcast race snapshot: {error:#}"),
        );
    }
    drop(guard);
    spawn_race_snapshot_loop(state);
}

fn spawn_race_snapshot_loop(state: Arc<Mutex<HostState>>) {
    thread::spawn(move || {
        loop {
            thread::sleep(RACE_SNAPSHOT_INTERVAL);
            let mut state = state.lock().expect("host state poisoned");
            if state.phase != NetworkRacePhase::Racing {
                break;
            }

            let now = Instant::now();
            host_items::advance_network_mushrooms(&mut state, now);
            host_ai::advance_network_ai_racers(&mut state, now);
            host_race::update_race_status(&mut state, now);
            let expired_choices = expire_bonus_cooldowns(&mut state, now);
            if expired_choices > 0 {
                push_network_log(
                    &state.debug_log,
                    format!("bonus refreshed choices={expired_choices}"),
                );
            }

            if state.phase == NetworkRacePhase::Finished {
                if let Err(error) = broadcast_race_snapshot(&mut state) {
                    push_network_log(
                        &state.debug_log,
                        format!("failed to broadcast race snapshot: {error:#}"),
                    );
                }
                push_network_log(&state.debug_log, "race finished on snapshot tick");
                print_server_line("Race finished");
                if let Err(error) = broadcast_race_results_once(&mut state) {
                    push_network_log(
                        &state.debug_log,
                        format!("failed to broadcast race results: {error:#}"),
                    );
                }
                break;
            } else if let Err(error) = broadcast_race_delta(&mut state) {
                push_network_log(
                    &state.debug_log,
                    format!("failed to broadcast race delta: {error:#}"),
                );
            }
        }
    });
}

fn broadcast_countdown_cancel(state: &mut HostState) {
    if let Err(error) = broadcast_race_snapshot(state) {
        push_network_log(
            &state.debug_log,
            format!("failed to broadcast race snapshot: {error:#}"),
        );
    }
    if let Err(error) = broadcast_lobby_snapshot(state) {
        push_network_log(
            &state.debug_log,
            format!("failed to broadcast lobby snapshot: {error:#}"),
        );
    }
}
