//! Authoritative TCP host for multiplayer races.

use std::{
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
    time::{Duration, Instant},
};

#[cfg(test)]
use crate::game::lobby::{
    connected_player_count, first_available_color, new_human_lobby_player, unique_lobby_name,
};
#[cfg(test)]
use crate::game::race::{RacePlayerId, RaceState};
use crate::game::{
    ai::AiDifficulty,
    items::ItemRegistry,
    mods::ActiveModConfig,
    track::{Track, WordList},
};
use anyhow::{Context, Result};

#[cfg(test)]
use super::host_lifecycle::build_race_result_rows as build_network_race_result_rows;
use super::log::{SharedNetworkLog, push_network_log};
#[cfg(test)]
use super::protocol::AssignedColor;
#[cfg(test)]
use super::protocol::RaceResultRow;
use super::protocol::ServerMessage;
#[cfg(test)]
use super::protocol::{NetworkRacePhase, PlayerId};
use super::transport::write_server_message as write_framed_server_message;

mod host_accept;
mod host_ai;
mod host_bonus;
mod host_broadcast;
mod host_client;
mod host_commands;
mod host_disconnect;
mod host_handshake;
mod host_input;
mod host_items;
mod host_join;
mod host_lobby;
mod host_phase;
mod host_race;
mod host_snapshots;
mod host_state;
mod host_util;
#[cfg(test)]
use host_ai::NetworkAiRacer;
#[cfg(test)]
use host_ai::set_lobby_ai_difficulty;
#[cfg(test)]
use host_ai::{add_lobby_ai_racer, add_network_ai_racers, advance_network_ai_racers};
#[cfg(test)]
use host_bonus::apply_network_key_input;
use host_broadcast::{
    broadcast_lobby_snapshot, broadcast_race_delta, broadcast_race_results_once,
    broadcast_race_snapshot,
};
#[cfg(test)]
use host_client::handle_client_messages;
#[cfg(test)]
use host_commands::update_host_ready;
use host_commands::{has_embedded_host_player, spawn_host_command_loop};
#[cfg(test)]
use host_disconnect::handle_player_disconnect;
#[cfg(test)]
use host_handshake::{read_join_hello, welcome_joiner};
#[cfg(test)]
use host_items::activate_network_pickup;
#[cfg(test)]
use host_lobby::{cleanup_disconnected_waiting_players, remove_lobby_player, rename_lobby_player};
#[cfg(test)]
use host_phase::{reconcile_phase_after_disconnect, return_to_lobby};
#[cfg(test)]
use host_race::{reset_race_from_lobby, update_race_status};
#[cfg(test)]
use host_snapshots::build_race_snapshot;
use host_state::{ConnectedClient, HostState, build_initial_host_state};
use host_util::{print_lobby_snapshot, validate_host_capacity};

const POST_FIRST_FINISH_TIMEOUT: Duration = Duration::from_secs(30);
const RACE_SNAPSHOT_INTERVAL: Duration = Duration::from_millis(100);
static SERVER_CONSOLE_LOGGING: AtomicBool = AtomicBool::new(true);

macro_rules! server_println {
    ($($arg:tt)*) => {
        if SERVER_CONSOLE_LOGGING.load(std::sync::atomic::Ordering::Relaxed) {
            println!($($arg)*);
        }
    };
}

fn print_server_line(message: impl AsRef<str>) {
    server_println!("{}", message.as_ref());
}

#[derive(Debug, Clone)]
pub struct HostConfig {
    pub bind: SocketAddr,
    pub host_name: Option<String>,
    pub track: Track,
    pub word_list: WordList,
    pub item_registry: ItemRegistry,
    pub active_mod_config: ActiveModConfig,
    pub max_players: usize,
    pub ai_racer_count: usize,
    pub ai_difficulty: AiDifficulty,
    pub ready_signal: Option<Sender<SocketAddr>>,
    pub console_logging: bool,
    pub debug_log: Option<SharedNetworkLog>,
}

pub fn run_host(mut config: HostConfig) -> Result<()> {
    SERVER_CONSOLE_LOGGING.store(config.console_logging, Ordering::Relaxed);

    validate_host_capacity(config.max_players, config.ai_racer_count)?;

    let listener = TcpListener::bind(config.bind)
        .with_context(|| format!("failed to bind host socket at {}", config.bind))?;
    let local_addr = listener
        .local_addr()
        .context("failed to read host address")?;
    if let Some(ready_signal) = config.ready_signal.take() {
        let _ = ready_signal.send(local_addr);
    }
    push_network_log(
        &config.debug_log,
        format!(
            "server listening addr={local_addr} max_players={} words={} ai_racers={} ai_difficulty={}",
            config.max_players,
            config.track.len(),
            config.ai_racer_count,
            config.ai_difficulty.name()
        ),
    );
    push_network_log(&config.debug_log, config.active_mod_config.log_summary());

    let initial_state = build_initial_host_state(config);
    let state = Arc::new(Mutex::new(initial_state.state));

    server_println!("TypeKart host listening on {local_addr}");
    if has_embedded_host_player(&state) {
        server_println!("Host lobby commands: start, lobby, ready, unready");
        spawn_host_command_loop(Arc::clone(&state));
    }
    server_println!("Waiting for joiners. Press Ctrl-C to stop.");

    host_accept::run_accept_loop(listener, state, initial_state.next_player_id)
}

fn push_event(state: &mut HostState, event: String) {
    state.events.push(event);
    const EVENT_LIMIT: usize = 20;
    if state.events.len() > EVENT_LIMIT {
        let excess = state.events.len() - EVENT_LIMIT;
        state.events.drain(0..excess);
    }
}

fn expire_bonus_cooldowns(state: &mut HostState, now: Instant) -> usize {
    let track = &state.race.track;
    state.bonuses.expire_cooldowns(track, now)
}

#[cfg(test)]
fn client_is_in_current_race(race: &RaceState, player_id: PlayerId) -> bool {
    race.players
        .iter()
        .any(|player| player.id == RacePlayerId(player_id.0))
}

#[cfg(test)]
fn build_race_result_rows(state: &HostState, now: Instant) -> Vec<RaceResultRow> {
    build_network_race_result_rows(&state.race, &state.runtime.lifecycle.placements, now)
}

fn send_server_message(mut stream: TcpStream, message: &ServerMessage) -> Result<()> {
    write_server_message(&mut stream, message)
}

fn write_server_message(stream: &mut TcpStream, message: &ServerMessage) -> Result<()> {
    write_framed_server_message(stream, message)
}

#[cfg(test)]
mod tests;
