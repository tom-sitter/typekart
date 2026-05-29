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

use crate::game::host_session::{
    CountdownAdvanceRejection, CountdownRacePreparation, CountdownStartRejection,
    HostRaceTickAction, advance_active_race_tick, begin_countdown_phase, cancel_countdown_outcome,
    countdown_should_cancel, countdown_start_plan, countdown_tick_phase,
    has_connected_active_racer, return_to_lobby_outcome, start_active_race_runtime_outcome,
    start_race_from_countdown,
};
use crate::net::{log::push_network_log, protocol::NetworkRacePhase};

use super::{
    HostState, RACE_SNAPSHOT_INTERVAL, broadcast_lobby_snapshot, broadcast_race_delta,
    broadcast_race_results_once, broadcast_race_snapshot, host_ai, host_items, host_race,
    print_server_line, push_event,
};

pub(super) fn start_countdown(state: Arc<Mutex<HostState>>) {
    let should_start = {
        let mut state = state.lock().expect("host state poisoned");
        match countdown_start_plan(state.phase) {
            Ok(CountdownRacePreparation::PrepareRace) => {
                if let Err(error) = host_race::reset_race_from_lobby(&mut state) {
                    push_network_log(
                        &state.debug_log,
                        format!("failed to prepare race from lobby: {error:#}"),
                    );
                    return;
                }
            }
            Ok(CountdownRacePreparation::UseCurrentRace) => {}
            Err(CountdownStartRejection::RaceAlreadyActive) => {
                push_network_log(&state.debug_log, "start ignored race already active");
                print_server_line("Race has already started");
                return;
            }
            Err(CountdownStartRejection::NoConnectedRacers) => {
                unreachable!("countdown phase planning does not inspect racer count")
            }
        }

        let phase =
            match begin_countdown_phase(host_race::current_race_connected_player_count(&state)) {
                Ok(phase) => phase,
                Err(CountdownStartRejection::NoConnectedRacers) => {
                    push_network_log(
                        &state.debug_log,
                        "start ignored no ready connected racers available",
                    );
                    print_server_line(
                        "Cannot start: at least one ready connected racer is required",
                    );
                    return;
                }
                Err(CountdownStartRejection::RaceAlreadyActive) => {
                    unreachable!("countdown begin only validates connected racer availability")
                }
            };

        state.phase = phase;
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
    let Some(outcome) = return_to_lobby_outcome(state.phase) else {
        return Ok(());
    };
    host_race::reset_race_from_lobby(state)?;
    push_event(state, outcome.event.to_string());
    push_network_log(&state.debug_log, outcome.event.to_ascii_lowercase());
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
    has_connected_active_racer(&state.race)
}

pub(super) fn cancel_countdown(state: &mut HostState) {
    let outcome = cancel_countdown_outcome();
    state.phase = outcome.phase;
    push_event(state, outcome.event.to_string());
    push_network_log(&state.debug_log, outcome.event.to_ascii_lowercase());
    print_server_line(outcome.event);
}

fn run_countdown(state: Arc<Mutex<HostState>>) {
    for remaining_seconds in [2, 1] {
        thread::sleep(Duration::from_secs(1));
        let mut guard = state.lock().expect("host state poisoned");
        if !matches!(guard.phase, NetworkRacePhase::Countdown { .. }) {
            push_network_log(&guard.debug_log, "countdown stopped before next tick");
            return;
        }
        if countdown_should_cancel(&guard.race) {
            cancel_countdown(&mut guard);
            broadcast_countdown_cancel(&mut guard);
            return;
        }

        guard.phase = countdown_tick_phase(remaining_seconds);
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
    let start_outcome =
        match start_race_from_countdown(guard.phase, has_connected_active_racer(&guard.race)) {
            Ok(outcome) => outcome,
            Err(CountdownAdvanceRejection::NoConnectedRacers) => {
                cancel_countdown(&mut guard);
                broadcast_countdown_cancel(&mut guard);
                return;
            }
            Err(CountdownAdvanceRejection::NotCountingDown) => {
                push_network_log(&guard.debug_log, "countdown stopped before race start");
                return;
            }
        };
    guard.phase = start_outcome.phase;
    let runtime_outcome = start_active_race_runtime_outcome();
    if runtime_outcome.set_ai_timing_now {
        host_ai::reset_network_ai_timing(&mut guard, Instant::now());
    }
    push_event(&mut guard, start_outcome.event.to_string());
    push_network_log(&guard.debug_log, start_outcome.event.to_ascii_lowercase());
    print_server_line(start_outcome.event);
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
            let phase = state.phase;
            let tick = {
                let HostState {
                    runtime,
                    race,
                    bonuses,
                    ..
                } = &mut *state;
                advance_active_race_tick(
                    &mut runtime.lifecycle,
                    race,
                    bonuses,
                    phase,
                    now,
                    super::POST_FIRST_FINISH_TIMEOUT,
                    true,
                )
            };
            state.phase = tick.lifecycle.phase;
            let tick_outcome = tick.tick;
            let refreshed_choices = tick_outcome.bonus_choices_refreshed;
            host_race::apply_race_lifecycle_outcome(&mut state, tick.lifecycle);
            if refreshed_choices > 0 {
                push_network_log(
                    &state.debug_log,
                    format!("bonus refreshed choices={refreshed_choices}"),
                );
            }

            match tick_outcome.action {
                HostRaceTickAction::BroadcastResults => {
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
                }
                HostRaceTickAction::BroadcastDelta => {
                    if let Err(error) = broadcast_race_delta(&mut state) {
                        push_network_log(
                            &state.debug_log,
                            format!("failed to broadcast race delta: {error:#}"),
                        );
                    }
                }
                HostRaceTickAction::Ignore => {}
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
