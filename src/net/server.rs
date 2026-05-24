//! Authoritative TCP host for multiplayer races.

use std::{
    collections::HashMap,
    io::{self, BufRead, BufReader},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
    thread,
    time::{Duration, Instant},
};

use crate::game::{
    ai::AiDifficulty,
    bonus::BonusState,
    bonus_flow::BonusAttempt,
    input_rules::player_input_is_paused as shared_player_input_is_paused,
    items::ItemRegistry,
    lobby::{
        LOBBY_COLOR_ROTATION, connected_player_count, first_available_color,
        first_available_player_id, new_human_lobby_player, ready_connected_participants,
        set_lobby_ready as shared_set_lobby_ready, unique_lobby_name,
    },
    mods::ActiveModConfig,
    race::{PlayerColorId, RacePlayerId, RaceRuntimeState, RaceState},
    race_flow::advance_race_flow,
    track::{Track, WordList},
    typing::KeyAction,
};
use anyhow::{Context, Result, bail};

#[cfg(test)]
use super::host_lifecycle::build_race_result_rows as build_network_race_result_rows;
use super::host_lifecycle::{
    build_race_results_message, finish_summary_log, finished_player_message,
};
use super::log::{SharedNetworkLog, push_network_log};
#[cfg(test)]
use super::protocol::RaceResultRow;
use super::protocol::{
    AssignedColor, ClientMessage, LobbyPlayer, NetworkRacePhase, PlayerId, PlayerKind, ProtocolKey,
    ServerMessage, version_mismatch_message,
};
use super::transport::{read_client_message, write_server_message as write_framed_server_message};

mod host_ai;
mod host_bonus;
mod host_items;
mod host_lobby;
mod host_snapshots;
use host_ai::NetworkAiRacer;
#[cfg(test)]
use host_ai::set_lobby_ai_difficulty;
#[cfg(test)]
use host_ai::{add_lobby_ai_racer, add_network_ai_racers, advance_network_ai_racers};
#[cfg(test)]
use host_bonus::apply_network_key_input;
#[cfg(test)]
use host_items::activate_network_pickup;
#[cfg(test)]
use host_lobby::{cleanup_disconnected_waiting_players, remove_lobby_player, rename_lobby_player};
#[cfg(test)]
use host_snapshots::build_race_snapshot;

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

macro_rules! server_eprintln {
    ($($arg:tt)*) => {
        if SERVER_CONSOLE_LOGGING.load(std::sync::atomic::Ordering::Relaxed) {
            eprintln!($($arg)*);
        }
    };
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

struct ConnectedClient {
    player_id: PlayerId,
    stream: TcpStream,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct JoinHello {
    name: String,
    client_version: String,
}

struct HostState {
    players: Vec<LobbyPlayer>,
    clients: Vec<ConnectedClient>,
    race: RaceState,
    ai_racers: HashMap<PlayerId, NetworkAiRacer>,
    word_list: WordList,
    bonuses: BonusState,
    item_registry: ItemRegistry,
    active_mod_config: ActiveModConfig,
    max_players: usize,
    ai_difficulty: AiDifficulty,
    runtime: RaceRuntimeState<PlayerId, BonusAttempt>,
    phase: NetworkRacePhase,
    snapshot_sequence: u64,
    events: Vec<String>,
    debug_log: Option<SharedNetworkLog>,
    race_results_sent: bool,
}

pub fn run_host(config: HostConfig) -> Result<()> {
    SERVER_CONSOLE_LOGGING.store(config.console_logging, Ordering::Relaxed);

    validate_host_capacity(config.max_players, config.ai_racer_count)?;

    let listener = TcpListener::bind(config.bind)
        .with_context(|| format!("failed to bind host socket at {}", config.bind))?;
    let local_addr = listener
        .local_addr()
        .context("failed to read host address")?;
    if let Some(ready_signal) = config.ready_signal {
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

    let bonuses = BonusState::generate(&config.track, &config.word_list);
    let mut race = RaceState::new(config.track.clone());
    let mut players = Vec::new();
    let mut next_player_id = 1;
    if let Some(host_name) = config.host_name {
        race.add_player(
            RacePlayerId(1),
            host_name.clone(),
            PlayerColorId::Cyan,
            std::time::Instant::now(),
        );
        players.push(LobbyPlayer {
            id: PlayerId(1),
            name: host_name,
            kind: PlayerKind::Human,
            color: LOBBY_COLOR_ROTATION[0],
            ready: true,
            connected: true,
            ai_difficulty: None,
            ai_wpm: None,
        });
        next_player_id = 2;
    }
    let ai_racers = host_ai::add_network_ai_racers(
        &mut race,
        &mut players,
        config.ai_racer_count,
        config.ai_difficulty,
        std::time::Instant::now(),
    );

    let state = Arc::new(Mutex::new(HostState {
        players,
        clients: Vec::new(),
        race,
        ai_racers,
        word_list: config.word_list,
        bonuses,
        item_registry: config.item_registry,
        active_mod_config: config.active_mod_config,
        max_players: config.max_players,
        ai_difficulty: config.ai_difficulty,
        runtime: RaceRuntimeState::new(),
        phase: NetworkRacePhase::WaitingForHost,
        snapshot_sequence: 0,
        events: Vec::new(),
        debug_log: config.debug_log,
        race_results_sent: false,
    }));

    server_println!("TypeKart host listening on {local_addr}");
    if has_embedded_host_player(&state) {
        server_println!("Host lobby commands: start, lobby, ready, unready");
        spawn_host_command_loop(Arc::clone(&state));
    }
    server_println!("Waiting for joiners. Press Ctrl-C to stop.");

    for stream in listener.incoming() {
        let stream = stream.context("failed to accept client connection")?;
        let peer = stream.peer_addr().ok();

        let join_hello = match read_join_hello(&stream) {
            Ok(join_hello) => join_hello,
            Err(error) => {
                server_eprintln!("Rejected connection: {error:#}");
                continue;
            }
        };
        let requested_player_name = join_hello.name;

        let (player_id, assigned_color, player_name) = {
            let mut state = state.lock().expect("host state poisoned");
            if join_hello.client_version != env!("CARGO_PKG_VERSION") {
                send_server_message(
                    stream,
                    &ServerMessage::Error {
                        message: version_mismatch_message(
                            env!("CARGO_PKG_VERSION"),
                            &join_hello.client_version,
                        ),
                    },
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
                continue;
            }

            let connected_players = connected_player_count(&state.players);
            if connected_players >= config.max_players {
                send_server_message(
                    stream,
                    &ServerMessage::Error {
                        message: format!(
                            "Lobby is full: {connected_players}/{} connected players",
                            config.max_players
                        ),
                    },
                )?;
                push_network_log(
                    &state.debug_log,
                    format!(
                        "join rejected: lobby full {connected_players}/{}",
                        config.max_players
                    ),
                );
                continue;
            }

            let player_name = unique_lobby_name(state.players.iter(), &requested_player_name);
            let player_id = first_available_player_id(&state.players, next_player_id);
            let Some(assigned_color) = first_available_color(&state.players) else {
                send_server_message(
                    stream,
                    &ServerMessage::Error {
                        message: "Lobby is full: no colors available".to_string(),
                    },
                )?;
                push_network_log(&state.debug_log, "join rejected: no colors available");
                continue;
            };
            let write_stream = match welcome_joiner(&stream, player_id, assigned_color) {
                Ok(write_stream) => write_stream,
                Err(error) => {
                    server_eprintln!("Rejected connection: {error:#}");
                    continue;
                }
            };

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
                    std::time::Instant::now(),
                );
            }
            push_event(&mut state, format!("{player_name} joined"));
            push_network_log(
                &state.debug_log,
                format!(
                    "{player_name} joined player={} color={assigned_color:?}",
                    player_id.0
                ),
            );
            print_lobby_snapshot(&state.players);
            broadcast_lobby_snapshot(&mut state)?;

            (player_id, assigned_color, player_name)
        };

        server_println!(
            "{} joined as player {} ({assigned_color:?}){}",
            player_name,
            player_id.0,
            peer.map(|addr| format!(" from {addr}")).unwrap_or_default()
        );

        let state_for_client = Arc::clone(&state);
        thread::spawn(move || handle_client_messages(player_id, stream, state_for_client));
        next_player_id = next_player_id.max(player_id.0 + 1);
    }

    Ok(())
}

fn validate_host_capacity(max_players: usize, ai_racer_count: usize) -> Result<()> {
    if max_players == 0 || max_players > LOBBY_COLOR_ROTATION.len() {
        bail!(
            "max players must be between 1 and {}",
            LOBBY_COLOR_ROTATION.len()
        );
    }
    if ai_racer_count >= max_players {
        bail!("ai racers must be less than max players so the host has a slot");
    }
    Ok(())
}

fn spawn_host_command_loop(state: Arc<Mutex<HostState>>) {
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
                "start" => start_countdown(Arc::clone(&state)),
                "" if command == " " => start_countdown(Arc::clone(&state)),
                "" => {}
                other => {
                    if current_phase(&state) == NetworkRacePhase::Racing {
                        apply_line_input(&state, PlayerId(1), other);
                    } else {
                        server_println!("Unknown host command: {other}");
                    }
                }
            }
        }
    });
}

fn has_embedded_host_player(state: &Arc<Mutex<HostState>>) -> bool {
    state
        .lock()
        .expect("host state poisoned")
        .players
        .iter()
        .any(|player| player.id == PlayerId(1))
}

fn update_host_ready(state: &Arc<Mutex<HostState>>, ready: bool) {
    let mut state = state.lock().expect("host state poisoned");
    if let Ok(outcome) = shared_set_lobby_ready(&mut state.players, PlayerId(1), ready) {
        server_println!(
            "{} is {}",
            outcome.name,
            if outcome.ready { "ready" } else { "not ready" }
        );
    }
    print_lobby_snapshot(&state.players);
    if let Err(error) = broadcast_lobby_snapshot(&mut state) {
        server_eprintln!("Failed to broadcast lobby snapshot: {error:#}");
    }
}

fn read_join_hello(stream: &TcpStream) -> Result<JoinHello> {
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .context("failed to clone client stream for reading")?,
    );
    let Some(message) = read_client_message(&mut reader).context("failed to read client hello")?
    else {
        bail!("client disconnected before hello");
    };
    let ClientMessage::Hello {
        name,
        client_version,
    } = message
    else {
        send_server_message(
            stream
                .try_clone()
                .context("failed to clone client stream for error response")?,
            &ServerMessage::Error {
                message: "Expected hello message".to_string(),
            },
        )?;
        bail!("client sent non-hello first message");
    };

    if name.trim().is_empty() {
        send_server_message(
            stream
                .try_clone()
                .context("failed to clone client stream for error response")?,
            &ServerMessage::Error {
                message: "Name cannot be empty".to_string(),
            },
        )?;
        bail!("client sent empty name");
    }

    if client_version.trim().is_empty() {
        send_server_message(
            stream
                .try_clone()
                .context("failed to clone client stream for error response")?,
            &ServerMessage::Error {
                message: "Client version cannot be empty".to_string(),
            },
        )?;
        bail!("client sent empty version");
    }

    Ok(JoinHello {
        name: name.trim().to_string(),
        client_version: client_version.trim().to_string(),
    })
}

fn welcome_joiner(
    stream: &TcpStream,
    player_id: PlayerId,
    assigned_color: AssignedColor,
) -> Result<TcpStream> {
    let mut write_stream = stream
        .try_clone()
        .context("failed to clone client stream for writing")?;
    write_server_message(
        &mut write_stream,
        &ServerMessage::Welcome {
            player_id,
            assigned_color,
        },
    )?;

    Ok(write_stream)
}

fn current_race_connected_player_count(state: &HostState) -> usize {
    state
        .race
        .players
        .iter()
        .filter(|player| player.connected)
        .count()
}

fn handle_client_messages(player_id: PlayerId, stream: TcpStream, state: Arc<Mutex<HostState>>) {
    let mut reader = BufReader::new(stream);
    loop {
        let message = match read_client_message(&mut reader) {
            Ok(Some(message)) => message,
            Ok(None) => break,
            Err(_) => continue,
        };

        match message {
            ClientMessage::Rename { name } => {
                let mut state = state.lock().expect("host state poisoned");
                if let Err(error) = host_lobby::rename_lobby_player(&mut state, player_id, &name) {
                    push_event(&mut state, error.to_string());
                }
                print_lobby_snapshot(&state.players);
                if let Err(error) = broadcast_lobby_snapshot(&mut state) {
                    server_eprintln!("Failed to broadcast lobby snapshot: {error:#}");
                }
            }
            ClientMessage::SetReady { ready } => {
                let mut state = state.lock().expect("host state poisoned");
                match shared_set_lobby_ready(&mut state.players, player_id, ready) {
                    Ok(outcome) => {
                        server_println!(
                            "{} is {}",
                            outcome.name,
                            if outcome.ready { "ready" } else { "not ready" }
                        );
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
                print_lobby_snapshot(&state.players);
                if let Err(error) = broadcast_lobby_snapshot(&mut state) {
                    server_eprintln!("Failed to broadcast lobby snapshot: {error:#}");
                }
            }
            ClientMessage::StartCountdown => {
                if player_id == PlayerId(1) {
                    {
                        let state = state.lock().expect("host state poisoned");
                        push_network_log(&state.debug_log, "host requested countdown");
                    }
                    start_countdown(Arc::clone(&state));
                } else {
                    server_println!(
                        "Ignoring start request from non-host player {}",
                        player_id.0
                    );
                }
            }
            ClientMessage::AddAi if player_id == PlayerId(1) => {
                let mut state = state.lock().expect("host state poisoned");
                if let Err(error) = host_ai::add_lobby_ai_racer(&mut state) {
                    push_event(&mut state, error.to_string());
                }
                print_lobby_snapshot(&state.players);
                if let Err(error) = broadcast_lobby_snapshot(&mut state) {
                    server_eprintln!("Failed to broadcast lobby snapshot: {error:#}");
                }
            }
            ClientMessage::RemoveLobbyPlayer { player_id: target } if player_id == PlayerId(1) => {
                let mut state = state.lock().expect("host state poisoned");
                if let Err(error) = host_lobby::remove_lobby_player(&mut state, target) {
                    push_event(&mut state, error.to_string());
                }
                print_lobby_snapshot(&state.players);
                if let Err(error) = broadcast_lobby_snapshot(&mut state) {
                    server_eprintln!("Failed to broadcast lobby snapshot: {error:#}");
                }
            }
            ClientMessage::SetAiDifficulty {
                player_id: target,
                difficulty,
            } if player_id == PlayerId(1) => {
                let mut state = state.lock().expect("host state poisoned");
                if let Err(error) =
                    host_ai::set_lobby_ai_difficulty(&mut state, target, difficulty.into())
                {
                    push_event(&mut state, error.to_string());
                }
                print_lobby_snapshot(&state.players);
                if let Err(error) = broadcast_lobby_snapshot(&mut state) {
                    server_eprintln!("Failed to broadcast lobby snapshot: {error:#}");
                }
            }
            ClientMessage::RestartRace => {
                if player_id == PlayerId(1) {
                    let mut state = state.lock().expect("host state poisoned");
                    if let Err(error) = return_to_lobby(&mut state) {
                        server_eprintln!("Failed to return to lobby: {error:#}");
                        push_network_log(
                            &state.debug_log,
                            format!("failed to return to lobby: {error:#}"),
                        );
                    }
                } else {
                    server_println!(
                        "Ignoring rematch request from non-host player {}",
                        player_id.0
                    );
                }
            }
            ClientMessage::KeyInput { key, .. } => {
                let now = std::time::Instant::now();
                let mut state = state.lock().expect("host state poisoned");
                if state.phase != NetworkRacePhase::Racing {
                    continue;
                }
                let action = protocol_key_to_action(key);
                push_network_log(
                    &state.debug_log,
                    format!("input player={} key={key:?}", player_id.0),
                );
                if host_bonus::apply_network_key_input(&mut state, player_id, action, now) {
                    update_race_status(&mut state, now);
                    if state.phase == NetworkRacePhase::Finished {
                        if let Err(error) = broadcast_race_snapshot(&mut state) {
                            server_eprintln!("Failed to broadcast race snapshot: {error:#}");
                        }
                        server_println!("Race finished");
                        if let Err(error) = broadcast_race_results_once(&mut state) {
                            server_eprintln!("Failed to broadcast race results: {error:#}");
                        }
                    } else if let Err(error) = broadcast_race_delta(&mut state) {
                        server_eprintln!("Failed to broadcast race delta: {error:#}");
                    }
                }
            }
            ClientMessage::Leave => break,
            _ => {}
        }
    }

    let mut state = state.lock().expect("host state poisoned");
    let was_race_screen_phase = matches!(
        state.phase,
        NetworkRacePhase::Countdown { .. } | NetworkRacePhase::Racing | NetworkRacePhase::Finished
    );
    if handle_player_disconnect(&mut state, player_id, std::time::Instant::now()) {
        return;
    }
    print_lobby_snapshot(&state.players);
    if let Err(error) = broadcast_lobby_snapshot(&mut state) {
        server_eprintln!("Failed to broadcast lobby snapshot: {error:#}");
    }
    if was_race_screen_phase && let Err(error) = broadcast_race_snapshot(&mut state) {
        server_eprintln!("Failed to broadcast race snapshot: {error:#}");
    }
    if state.phase == NetworkRacePhase::Finished
        && let Err(error) = broadcast_race_results_once(&mut state)
    {
        server_eprintln!("Failed to broadcast race results: {error:#}");
    }
}

fn handle_player_disconnect(state: &mut HostState, player_id: PlayerId, now: Instant) -> bool {
    if let Some(player) = state
        .players
        .iter_mut()
        .find(|player| player.id == player_id)
    {
        let name = player.name.clone();
        player.connected = false;
        player.ready = false;
        server_println!("{name} disconnected");
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

    reconcile_phase_after_disconnect(state, now);
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
            server_eprintln!(
                "Failed to send close message to player {}: {error:#}",
                client.player_id.0
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

fn print_lobby_snapshot(players: &[LobbyPlayer]) {
    server_println!("Lobby:");
    for player in players {
        server_println!(
            "  {}: {} ({:?}){}{}{}",
            player.id.0,
            player.name,
            player.color,
            if player.ready { " ready" } else { "" },
            if player.connected {
                ""
            } else {
                " disconnected"
            },
            if player.id == PlayerId(1) {
                " host"
            } else {
                ""
            }
        );
    }
}

fn broadcast_lobby_snapshot(state: &mut HostState) -> Result<()> {
    let snapshot = ServerMessage::LobbySnapshot {
        players: state.players.clone(),
        host_id: PlayerId(1),
        mod_config: (&state.active_mod_config).into(),
        events: state.events.clone(),
    };

    let mut failed_clients = Vec::new();
    for client in state.clients.iter_mut() {
        if let Err(error) = write_server_message(&mut client.stream, &snapshot) {
            server_eprintln!(
                "Failed to send lobby snapshot to player {}: {error:#}",
                client.player_id.0
            );
            failed_clients.push(client.player_id);
        }
    }

    state
        .clients
        .retain(|client| !failed_clients.contains(&client.player_id));
    Ok(())
}

fn start_countdown(state: Arc<Mutex<HostState>>) {
    let should_start = {
        let mut state = state.lock().expect("host state poisoned");
        match state.phase {
            NetworkRacePhase::WaitingForHost
            | NetworkRacePhase::Lobby
            | NetworkRacePhase::Finished => {
                if let Err(error) = reset_race_from_lobby(&mut state) {
                    server_eprintln!("Failed to prepare race: {error:#}");
                    push_network_log(
                        &state.debug_log,
                        format!("failed to prepare race: {error:#}"),
                    );
                    return;
                }
            }
            NetworkRacePhase::Countdown { .. } | NetworkRacePhase::Racing => {
                server_println!("Race has already started");
                return;
            }
        }

        if current_race_connected_player_count(&state) < 1 {
            server_println!("Cannot start: at least one ready connected racer is required");
            return;
        }

        state.phase = NetworkRacePhase::Countdown {
            remaining_seconds: 3,
        };
        push_event(&mut state, "Countdown started".to_string());
        push_network_log(&state.debug_log, "countdown started remaining=3");
        server_println!("Countdown: 3");
        if let Err(error) = broadcast_race_snapshot(&mut state) {
            server_eprintln!("Failed to broadcast race snapshot: {error:#}");
        }
        true
    };

    if should_start {
        thread::spawn(move || run_countdown(state));
    }
}

fn reset_race_from_lobby(state: &mut HostState) -> Result<()> {
    host_lobby::cleanup_disconnected_waiting_players(state);

    let word_count = state.race.track.len();
    let track = Track::generate(&state.word_list, word_count)
        .context("failed to generate rematch track")?;
    let now = Instant::now();
    let participants = ready_connected_participants(&state.players);
    state.race = RaceState::from_participants(track, participants, now);
    host_ai::reset_network_ai_timing(state, now);

    state.bonuses = BonusState::generate(&state.race.track, &state.word_list);
    state.runtime.reset();
    state.race_results_sent = false;
    state.events.clear();
    state.phase = NetworkRacePhase::WaitingForHost;
    push_network_log(
        &state.debug_log,
        format!("prepared rematch racers={}", state.race.players.len()),
    );

    Ok(())
}

fn return_to_lobby(state: &mut HostState) -> Result<()> {
    let event = match state.phase {
        NetworkRacePhase::Countdown { .. } | NetworkRacePhase::Racing => "Race cancelled",
        NetworkRacePhase::Finished => "Returned to lobby",
        NetworkRacePhase::Lobby | NetworkRacePhase::WaitingForHost => return Ok(()),
    };
    reset_race_from_lobby(state)?;
    push_event(state, event.to_string());
    push_network_log(&state.debug_log, event.to_ascii_lowercase());
    if let Err(error) = broadcast_race_snapshot(state) {
        server_eprintln!("Failed to broadcast lobby race snapshot: {error:#}");
    }
    if let Err(error) = broadcast_lobby_snapshot(state) {
        server_eprintln!("Failed to broadcast lobby snapshot: {error:#}");
    }

    Ok(())
}

fn current_phase(state: &Arc<Mutex<HostState>>) -> NetworkRacePhase {
    state.lock().expect("host state poisoned").phase
}

fn apply_line_input(state: &Arc<Mutex<HostState>>, player_id: PlayerId, line: &str) {
    let now = std::time::Instant::now();
    let mut state = state.lock().expect("host state poisoned");
    if state.phase != NetworkRacePhase::Racing {
        return;
    }

    for ch in line.chars() {
        let action = if ch == ' ' {
            KeyAction::Space
        } else {
            KeyAction::Char(ch)
        };
        host_bonus::apply_network_key_input(&mut state, player_id, action, now);
    }
    host_bonus::apply_network_key_input(&mut state, player_id, KeyAction::Space, now);

    update_race_status(&mut state, now);
    if state.phase == NetworkRacePhase::Finished {
        if let Err(error) = broadcast_race_snapshot(&mut state) {
            server_eprintln!("Failed to broadcast race snapshot: {error:#}");
        }
        server_println!("Race finished");
        push_network_log(&state.debug_log, "race finished after host line input");
        if let Err(error) = broadcast_race_results_once(&mut state) {
            server_eprintln!("Failed to broadcast race results: {error:#}");
        }
    } else if let Err(error) = broadcast_race_delta(&mut state) {
        server_eprintln!("Failed to broadcast race delta: {error:#}");
    }
}

fn player_input_is_paused(state: &HostState, player_id: PlayerId, now: Instant) -> bool {
    shared_player_input_is_paused(
        &state.race,
        &state.runtime.player_effects,
        RacePlayerId(player_id.0),
        now,
    )
}

fn player_label(state: &HostState, player_id: PlayerId) -> String {
    player_name(state, player_id).unwrap_or_else(|| format!("player {}", player_id.0))
}

fn player_name(state: &HostState, id: PlayerId) -> Option<String> {
    state
        .race
        .players
        .iter()
        .find(|player| player.id == RacePlayerId(id.0))
        .map(|player| player.name.clone())
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
            if let Err(error) = broadcast_race_snapshot(&mut guard) {
                server_eprintln!("Failed to broadcast race snapshot: {error:#}");
            }
            if let Err(error) = broadcast_lobby_snapshot(&mut guard) {
                server_eprintln!("Failed to broadcast lobby snapshot: {error:#}");
            }
            return;
        }

        guard.phase = NetworkRacePhase::Countdown { remaining_seconds };
        push_network_log(
            &guard.debug_log,
            format!("countdown tick remaining={remaining_seconds}"),
        );
        server_println!("Countdown: {remaining_seconds}");
        if let Err(error) = broadcast_race_snapshot(&mut guard) {
            server_eprintln!("Failed to broadcast race snapshot: {error:#}");
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
        if let Err(error) = broadcast_race_snapshot(&mut guard) {
            server_eprintln!("Failed to broadcast race snapshot: {error:#}");
        }
        if let Err(error) = broadcast_lobby_snapshot(&mut guard) {
            server_eprintln!("Failed to broadcast lobby snapshot: {error:#}");
        }
        return;
    }

    guard.phase = NetworkRacePhase::Racing;
    host_ai::reset_network_ai_timing(&mut guard, Instant::now());
    push_event(&mut guard, "Race started".to_string());
    push_network_log(&guard.debug_log, "race started");
    server_println!("Race started");
    if let Err(error) = broadcast_race_snapshot(&mut guard) {
        server_eprintln!("Failed to broadcast race snapshot: {error:#}");
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
            update_race_status(&mut state, now);
            let expired_choices = expire_bonus_cooldowns(&mut state, now);
            if expired_choices > 0 {
                push_network_log(
                    &state.debug_log,
                    format!("bonus refreshed choices={expired_choices}"),
                );
            }

            if state.phase == NetworkRacePhase::Finished {
                if let Err(error) = broadcast_race_snapshot(&mut state) {
                    server_eprintln!("Failed to broadcast race snapshot: {error:#}");
                }
                server_println!("Race finished");
                push_network_log(&state.debug_log, "race finished on snapshot tick");
                if let Err(error) = broadcast_race_results_once(&mut state) {
                    server_eprintln!("Failed to broadcast race results: {error:#}");
                }
                break;
            } else if let Err(error) = broadcast_race_delta(&mut state) {
                server_eprintln!("Failed to broadcast race delta: {error:#}");
            }
        }
    });
}

fn reconcile_phase_after_disconnect(state: &mut HostState, now: Instant) {
    match state.phase {
        NetworkRacePhase::Countdown { .. } => {
            if !countdown_has_any_connected_racer(state) {
                cancel_countdown(state);
            }
        }
        NetworkRacePhase::Racing => update_race_status(state, now),
        NetworkRacePhase::Finished => {}
        NetworkRacePhase::Lobby | NetworkRacePhase::WaitingForHost => {}
    }
}

fn countdown_has_any_connected_racer(state: &HostState) -> bool {
    state
        .race
        .players
        .iter()
        .filter(|player| player.connected && !player.state.is_finished())
        .count()
        >= 1
}

fn cancel_countdown(state: &mut HostState) {
    state.phase = NetworkRacePhase::WaitingForHost;
    push_event(state, "Countdown cancelled".to_string());
    push_network_log(&state.debug_log, "countdown cancelled no connected racers");
    server_println!("Countdown cancelled");
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

fn broadcast_race_snapshot(state: &mut HostState) -> Result<()> {
    let snapshot = ServerMessage::RaceSnapshot(host_snapshots::build_race_snapshot(state));
    host_snapshots::log_race_snapshot(state);
    broadcast_server_message_to_clients(state, &snapshot)
}

fn broadcast_race_delta(state: &mut HostState) -> Result<()> {
    let delta = ServerMessage::RaceDelta(host_snapshots::build_race_delta_snapshot(state));
    host_snapshots::log_race_delta(state);
    broadcast_server_message_to_clients(state, &delta)
}

fn broadcast_server_message_to_clients(
    state: &mut HostState,
    message: &ServerMessage,
) -> Result<()> {
    let mut failed_clients = Vec::new();
    for client in state.clients.iter_mut() {
        if let Err(error) = write_server_message(&mut client.stream, message) {
            server_eprintln!(
                "Failed to send server message to player {}: {error:#}",
                client.player_id.0
            );
            failed_clients.push(client.player_id);
        }
    }

    state
        .clients
        .retain(|client| !failed_clients.contains(&client.player_id));
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

    let mut failed_clients = Vec::new();
    for client in state.clients.iter_mut() {
        if let Err(error) = write_server_message(&mut client.stream, &results.message) {
            server_eprintln!(
                "Failed to send race results to player {}: {error:#}",
                client.player_id.0
            );
            failed_clients.push(client.player_id);
        }
    }

    state
        .clients
        .retain(|client| !failed_clients.contains(&client.player_id));
    Ok(())
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

fn broadcast_race_results_once(state: &mut HostState) -> Result<()> {
    if state.race_results_sent {
        push_network_log(&state.debug_log, "skipped duplicate race results broadcast");
        return Ok(());
    }

    broadcast_race_results(state)?;
    state.race_results_sent = true;
    Ok(())
}

fn update_race_status(state: &mut HostState, now: Instant) {
    if state.phase != NetworkRacePhase::Racing {
        return;
    }

    let outcome = advance_race_flow(
        &mut state.runtime.lifecycle,
        &state.race,
        now,
        POST_FIRST_FINISH_TIMEOUT,
    );

    for finished in outcome.newly_finished {
        let message = finished_player_message(&finished);
        push_event(state, message.clone());
        push_network_log(&state.debug_log, message);
    }

    if let Some(summary) = outcome.finished {
        state.phase = NetworkRacePhase::Finished;
        push_event(state, "Race finished".to_string());
        push_network_log(&state.debug_log, finish_summary_log(&summary));
    }
}

fn protocol_key_to_action(key: ProtocolKey) -> KeyAction {
    match key {
        ProtocolKey::Char(' ') | ProtocolKey::Space => KeyAction::Space,
        ProtocolKey::Char(ch) => KeyAction::Char(ch),
        ProtocolKey::Backspace => KeyAction::Backspace,
    }
}

impl From<AssignedColor> for PlayerColorId {
    fn from(value: AssignedColor) -> Self {
        match value {
            AssignedColor::Cyan => Self::Cyan,
            AssignedColor::Red => Self::Red,
            AssignedColor::Green => Self::Green,
            AssignedColor::Blue => Self::Blue,
            AssignedColor::Yellow => Self::Yellow,
            AssignedColor::Magenta => Self::Magenta,
        }
    }
}

impl From<PlayerColorId> for AssignedColor {
    fn from(value: PlayerColorId) -> Self {
        match value {
            PlayerColorId::Cyan => Self::Cyan,
            PlayerColorId::Red => Self::Red,
            PlayerColorId::Green => Self::Green,
            PlayerColorId::Blue => Self::Blue,
            PlayerColorId::Yellow => Self::Yellow,
            PlayerColorId::Magenta => Self::Magenta,
        }
    }
}

fn send_server_message(mut stream: TcpStream, message: &ServerMessage) -> Result<()> {
    write_server_message(&mut stream, message)
}

fn write_server_message(stream: &mut TcpStream, message: &ServerMessage) -> Result<()> {
    write_framed_server_message(stream, message)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
        thread,
        time::{Duration, Instant},
    };

    use super::{
        AssignedColor, ConnectedClient, HostState, NetworkAiRacer, NetworkRacePhase,
        POST_FIRST_FINISH_TIMEOUT, PlayerId, activate_network_pickup, add_lobby_ai_racer,
        add_network_ai_racers, advance_network_ai_racers, apply_network_key_input,
        broadcast_lobby_snapshot, broadcast_race_results_once, broadcast_race_snapshot,
        build_race_result_rows, build_race_snapshot, cleanup_disconnected_waiting_players,
        client_is_in_current_race, connected_player_count, first_available_color,
        handle_client_messages, handle_player_disconnect, new_human_lobby_player, push_event,
        read_join_hello, reconcile_phase_after_disconnect, remove_lobby_player,
        rename_lobby_player, reset_race_from_lobby, return_to_lobby, set_lobby_ai_difficulty,
        unique_lobby_name, update_host_ready, update_race_status, validate_host_capacity,
        welcome_joiner,
    };
    use crate::game::{
        ai::AiDifficulty,
        bonus::{BonusChoice, BonusChoiceStatus, BonusPoint, BonusState},
        effects::ActiveEffect,
        item_effects::{RaceImpactCueKind, RaceItemEffectState},
        items::{HeldItem, ItemActivation, ItemDefinition, ItemPickup, ItemRegistry},
        mods::{ActiveModConfig, ContentMetadata},
        race::{PlayerColorId, RacePlayerId, RaceRuntimeState, RaceState},
        stats::TypingStats,
        track::{Track, WordList},
        typing::KeyAction,
        words::WordSetDefinition,
    };
    use crate::net::protocol::{
        ClientMessage, LobbyPlayer, PlayerKind, RaceResultStatus, ServerMessage,
        decode_server_message, encode_client_message,
    };

    #[test]
    fn host_handshake_accepts_hello() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();

        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let hello = read_join_hello(&stream).unwrap();
            welcome_joiner(&stream, PlayerId(2), AssignedColor::Red).unwrap();
            LobbyPlayer {
                id: PlayerId(2),
                name: hello.name,
                kind: PlayerKind::Human,
                color: AssignedColor::Red,
                ready: false,
                connected: true,
                ai_difficulty: None,
                ai_wpm: None,
            }
        });

        let mut client = std::net::TcpStream::connect(address).unwrap();
        let hello = encode_client_message(&ClientMessage::Hello {
            name: "alex".to_string(),
            client_version: env!("CARGO_PKG_VERSION").to_string(),
        })
        .unwrap();
        writeln!(client, "{hello}").unwrap();

        let player = server.join().unwrap();

        assert_eq!(player.id, PlayerId(2));
        assert_eq!(player.name, "alex");
        assert_eq!(player.color, AssignedColor::Red);
    }

    #[test]
    fn host_handshake_rejects_empty_client_version() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();

        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            read_join_hello(&stream).unwrap_err();
        });

        let mut client = std::net::TcpStream::connect(address).unwrap();
        let hello = encode_client_message(&ClientMessage::Hello {
            name: "alex".to_string(),
            client_version: "".to_string(),
        })
        .unwrap();
        writeln!(client, "{hello}").unwrap();

        let mut reader = BufReader::new(client);
        let mut error_line = String::new();
        reader.read_line(&mut error_line).unwrap();

        assert!(matches!(
            decode_server_message(error_line.trim_end()).unwrap(),
            ServerMessage::Error { ref message } if message == "Client version cannot be empty"
        ));
        server.join().unwrap();
    }

    #[test]
    fn duplicate_human_names_get_numbered_suffixes() {
        let players = [
            lobby_player(PlayerId(1), "tom", PlayerKind::Human, true),
            lobby_player(PlayerId(2), "Tom2", PlayerKind::Human, true),
            lobby_player(PlayerId(3), "tom3", PlayerKind::Human, false),
        ];

        assert_eq!(unique_lobby_name(players.iter(), "tom"), "tom3");
        assert_eq!(unique_lobby_name(players.iter(), "alex"), "alex");
    }

    #[test]
    fn lobby_player_can_rename_with_unique_suffix() {
        let mut state = test_host_state(NetworkRacePhase::WaitingForHost);

        rename_lobby_player(&mut state, PlayerId(2), "host").unwrap();

        assert_eq!(state.players[1].name, "host2");
        assert_eq!(state.race.players[1].name, "host2");
        assert!(
            state
                .events
                .iter()
                .any(|event| event == "alex renamed to host2")
        );
    }

    #[test]
    fn lobby_player_cannot_rename_during_active_race() {
        let mut state = test_host_state(NetworkRacePhase::Racing);

        assert!(rename_lobby_player(&mut state, PlayerId(2), "alex").is_err());
    }

    #[test]
    fn first_human_joiner_is_host_and_starts_ready() {
        assert!(new_human_lobby_player(PlayerId(1), "host", AssignedColor::Cyan).ready);
        assert!(!new_human_lobby_player(PlayerId(2), "joiner", AssignedColor::Red).ready);
    }

    #[test]
    fn lobby_snapshot_broadcasts_to_connected_clients() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();

        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            read_join_hello(&stream).unwrap();
            let client_stream = welcome_joiner(&stream, PlayerId(2), AssignedColor::Red).unwrap();
            let mut state = HostState {
                clients: vec![ConnectedClient {
                    player_id: PlayerId(2),
                    stream: client_stream,
                }],
                players: test_players(false),
                race: test_race_state(),
                ai_racers: HashMap::new(),
                word_list: test_word_list(),
                bonuses: test_bonus_state(),
                item_registry: ItemRegistry::builtin(),
                active_mod_config: test_active_mod_config(),
                max_players: 6,
                ai_difficulty: AiDifficulty::Easy,
                runtime: RaceRuntimeState::new(),
                phase: NetworkRacePhase::WaitingForHost,
                snapshot_sequence: 0,
                events: Vec::new(),
                debug_log: None,
                race_results_sent: false,
            };

            broadcast_lobby_snapshot(&mut state).unwrap();
        });

        let mut client = std::net::TcpStream::connect(address).unwrap();
        send_hello(&mut client);

        let mut reader = BufReader::new(client);
        let mut welcome_line = String::new();
        reader.read_line(&mut welcome_line).unwrap();
        let mut snapshot_line = String::new();
        reader.read_line(&mut snapshot_line).unwrap();

        assert!(matches!(
            decode_server_message(welcome_line.trim_end()).unwrap(),
            ServerMessage::Welcome { .. }
        ));
        assert!(matches!(
            decode_server_message(snapshot_line.trim_end()).unwrap(),
            ServerMessage::LobbySnapshot { ref players, .. } if players.len() == 2
        ));
        server.join().unwrap();
    }

    #[test]
    fn set_ready_updates_lobby_and_broadcasts_snapshot() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();

        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            read_join_hello(&stream).unwrap();
            let client_stream = welcome_joiner(&stream, PlayerId(2), AssignedColor::Red).unwrap();
            let read_stream = stream;
            let state = Arc::new(Mutex::new(HostState {
                clients: vec![ConnectedClient {
                    player_id: PlayerId(2),
                    stream: client_stream,
                }],
                players: test_players(false),
                race: test_race_state(),
                ai_racers: HashMap::new(),
                word_list: test_word_list(),
                bonuses: test_bonus_state(),
                item_registry: ItemRegistry::builtin(),
                active_mod_config: test_active_mod_config(),
                max_players: 6,
                ai_difficulty: AiDifficulty::Easy,
                runtime: RaceRuntimeState::new(),
                phase: NetworkRacePhase::WaitingForHost,
                snapshot_sequence: 0,
                events: Vec::new(),
                debug_log: None,
                race_results_sent: false,
            }));
            handle_client_messages(PlayerId(2), read_stream, Arc::clone(&state));
            state
        });

        let mut client = std::net::TcpStream::connect(address).unwrap();
        send_hello(&mut client);
        let mut reader = BufReader::new(client.try_clone().unwrap());
        let mut welcome_line = String::new();
        reader.read_line(&mut welcome_line).unwrap();
        let ready = encode_client_message(&ClientMessage::SetReady { ready: true }).unwrap();
        writeln!(client, "{ready}").unwrap();
        let mut snapshot_line = String::new();
        reader.read_line(&mut snapshot_line).unwrap();
        drop(client);
        drop(reader);

        let state = server.join().unwrap();
        let state = state.lock().unwrap();

        assert!(state.players.iter().all(|player| player.id != PlayerId(2)));
        assert!(
            state
                .race
                .players
                .iter()
                .all(|player| player.id != RacePlayerId(2))
        );
        assert!(matches!(
            decode_server_message(snapshot_line.trim_end()).unwrap(),
            ServerMessage::LobbySnapshot { ref players, ref events, .. }
                if players.iter().any(|player| player.id == PlayerId(2) && player.ready)
                    && events.iter().any(|event| event == "alex ready")
        ));
    }

    #[test]
    fn disconnected_players_do_not_count_against_capacity_or_color_assignment() {
        let players = vec![
            LobbyPlayer {
                id: PlayerId(1),
                name: "host".to_string(),
                kind: PlayerKind::Human,
                color: AssignedColor::Cyan,
                ready: true,
                connected: true,
                ai_difficulty: None,
                ai_wpm: None,
            },
            LobbyPlayer {
                id: PlayerId(2),
                name: "alex".to_string(),
                kind: PlayerKind::Human,
                color: AssignedColor::Red,
                ready: false,
                connected: false,
                ai_difficulty: None,
                ai_wpm: None,
            },
        ];

        assert_eq!(connected_player_count(&players), 1);
        assert_eq!(first_available_color(&players), Some(AssignedColor::Red));
    }

    #[test]
    fn network_ai_racers_are_added_as_ready_bots() {
        let now = Instant::now();
        let mut race = RaceState::new(Track::new(vec!["one".to_string(), "two".to_string()]));
        let mut players = Vec::new();

        let ai_racers = add_network_ai_racers(&mut race, &mut players, 2, AiDifficulty::Easy, now);

        assert_eq!(players.len(), 2);
        assert_eq!(race.players.len(), 2);
        assert_eq!(ai_racers.len(), 2);
        assert_eq!(players[0].id, PlayerId(2));
        assert_eq!(players[0].name, "ai-1");
        assert_eq!(players[0].kind, PlayerKind::Bot);
        assert_eq!(players[0].color, AssignedColor::Red);
        assert!(players[0].ready);
        assert!(players[0].connected);
        assert_eq!(players[1].id, PlayerId(3));
        assert_eq!(players[1].color, AssignedColor::Green);
    }

    #[test]
    fn network_ai_racers_reserve_human_host_slot() {
        assert!(validate_host_capacity(6, 5).is_ok());
        assert!(validate_host_capacity(6, 6).is_err());
        assert!(validate_host_capacity(0, 0).is_err());
        assert!(validate_host_capacity(7, 0).is_err());
    }

    #[test]
    fn network_ai_racer_advances_from_wpm_budget() {
        let now = Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.players[1].kind = PlayerKind::Bot;
        state.ai_racers.insert(
            PlayerId(2),
            NetworkAiRacer {
                difficulty: AiDifficulty::Easy,
                words_per_minute: 60.0,
                char_budget: 0.0,
                last_update: now,
            },
        );

        advance_network_ai_racers(&mut state, now + Duration::from_secs(1));

        let ai = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(ai.state.word_index, 1);
        assert_eq!(ai.state.input, "t");
    }

    #[test]
    fn network_inked_ai_racer_hesitates_from_reduced_wpm_budget() {
        let now = Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.players[1].kind = PlayerKind::Bot;
        state.race.players[1].state.inked_word_index = Some(0);
        state.race.players[1].state.inked_until = Some(now + Duration::from_secs(5));
        state.ai_racers.insert(
            PlayerId(2),
            NetworkAiRacer {
                difficulty: AiDifficulty::Easy,
                words_per_minute: 60.0,
                char_budget: 0.0,
                last_update: now,
            },
        );

        advance_network_ai_racers(&mut state, now + Duration::from_secs(1));

        let ai = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(ai.state.word_index, 0);
        assert_eq!(ai.state.input, "one");
    }

    #[test]
    fn network_ai_racer_does_not_type_while_stunned() {
        let now = Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.players[1].kind = PlayerKind::Bot;
        state.ai_racers.insert(
            PlayerId(2),
            NetworkAiRacer {
                difficulty: AiDifficulty::Easy,
                words_per_minute: 120.0,
                char_budget: 0.0,
                last_update: now,
            },
        );
        state.runtime.player_effects.insert(
            RacePlayerId(2),
            RaceItemEffectState {
                stunned_until: Some(now + Duration::from_secs(1)),
                ..Default::default()
            },
        );

        advance_network_ai_racers(&mut state, now + Duration::from_millis(500));

        let ai = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(ai.state.word_index, 0);
        assert_eq!(ai.state.input, "");
        assert_eq!(state.ai_racers.get(&PlayerId(2)).unwrap().char_budget, 0.0);
    }

    #[test]
    fn network_ai_racer_does_not_advance_or_accrue_budget_during_countdown() {
        let now = Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Countdown {
            remaining_seconds: 3,
        });
        state.players[1].kind = PlayerKind::Bot;
        state.ai_racers.insert(
            PlayerId(2),
            NetworkAiRacer {
                difficulty: AiDifficulty::Easy,
                words_per_minute: 120.0,
                char_budget: 4.0,
                last_update: now - Duration::from_secs(10),
            },
        );

        advance_network_ai_racers(&mut state, now);

        let bot = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(bot.state.word_index, 0);
        assert_eq!(bot.state.input, "");
        let ai = state.ai_racers.get(&PlayerId(2)).unwrap();
        assert_eq!(ai.char_budget, 0.0);
        assert_eq!(ai.last_update, now);

        state.phase = NetworkRacePhase::Racing;
        advance_network_ai_racers(&mut state, now + Duration::from_millis(50));

        let bot = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(bot.state.word_index, 0);
        assert_eq!(bot.state.input, "");
        assert!(state.ai_racers.get(&PlayerId(2)).unwrap().char_budget < 1.0);
    }

    #[test]
    fn network_ai_racer_can_claim_bonus_and_activate_shield() {
        let now = Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.players[1].kind = PlayerKind::Bot;
        state.item_registry = test_single_item_registry(ItemPickup::Shield);
        state.ai_racers.insert(
            PlayerId(2),
            NetworkAiRacer {
                difficulty: AiDifficulty::Easy,
                words_per_minute: 60.0,
                char_budget: 0.0,
                last_update: now,
            },
        );
        state.race.players[1].state.word_index = 1;

        advance_network_ai_racers(&mut state, now);

        let bot = state.race.player(RacePlayerId(2)).unwrap();
        assert!(bot.state.has_active_shield(now));
        assert!(
            state.bonuses.points[0]
                .choices
                .iter()
                .any(|choice| matches!(choice.status, BonusChoiceStatus::Cooldown { .. }))
        );
        assert_eq!(state.runtime.spent_bonus_gaps.get(&PlayerId(2)), Some(&0));
        assert!(state.events.iter().any(|event| event == "alex got Shield"));
    }

    #[test]
    fn network_ai_racer_can_claim_bonus_and_reset_human_with_banana() {
        let now = Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.players[1].kind = PlayerKind::Bot;
        state.item_registry = test_single_item_registry(ItemPickup::Held(HeldItem::Banana));
        state.ai_racers.insert(
            PlayerId(2),
            NetworkAiRacer {
                difficulty: AiDifficulty::Easy,
                words_per_minute: 60.0,
                char_budget: 0.0,
                last_update: now,
            },
        );
        state.race.players[0].state.word_index = 1;
        state.race.players[1].state.word_index = 1;

        advance_network_ai_racers(&mut state, now);

        assert!(
            state
                .runtime
                .player_effects
                .get(&RacePlayerId(1))
                .and_then(|effects| effects.stunned_until)
                .is_none()
        );
        assert!(
            state
                .runtime
                .player_effects
                .get(&RacePlayerId(1))
                .and_then(|effects| effects.impact_cue)
                .is_some_and(|cue| cue.kind == RaceImpactCueKind::Banana && cue.until > now)
        );
        assert!(
            state
                .runtime
                .player_effects
                .get(&RacePlayerId(2))
                .and_then(|effects| effects.item_cue.as_ref())
                .is_some_and(|cue| cue.until > now)
        );
        assert!(state.events.iter().any(|event| event == "alex hit host"));
    }

    #[test]
    fn human_banana_resets_network_ai_typing_budget() {
        let now = Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.players[1].kind = PlayerKind::Bot;
        state.ai_racers.insert(
            PlayerId(2),
            NetworkAiRacer {
                difficulty: AiDifficulty::Easy,
                words_per_minute: 60.0,
                char_budget: 3.0,
                last_update: now,
            },
        );

        activate_network_pickup(
            &mut state,
            PlayerId(1),
            ItemPickup::Held(HeldItem::Banana),
            now,
        );

        assert_eq!(state.ai_racers.get(&PlayerId(2)).unwrap().char_budget, 0.0);
        assert!(
            state
                .runtime
                .player_effects
                .get(&RacePlayerId(2))
                .and_then(|effects| effects.stunned_until)
                .is_some_and(|until| until > now)
        );
    }

    #[test]
    fn waiting_cleanup_removes_disconnected_players_from_next_race_roster() {
        let mut state = test_host_state(NetworkRacePhase::WaitingForHost);
        state.players[1].connected = false;
        state.race.players[1].connected = false;

        cleanup_disconnected_waiting_players(&mut state);

        assert_eq!(state.players.len(), 1);
        assert_eq!(state.race.players.len(), 1);
        assert!(state.players.iter().all(|player| player.id != PlayerId(2)));
        assert!(
            state
                .race
                .players
                .iter()
                .all(|player| player.id != RacePlayerId(2))
        );
    }

    #[test]
    fn waiting_cleanup_keeps_disconnected_players_during_active_race() {
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.players[1].connected = false;
        state.race.players[1].connected = false;

        cleanup_disconnected_waiting_players(&mut state);

        assert_eq!(state.players.len(), 2);
        assert_eq!(state.race.players.len(), 2);
    }

    #[test]
    fn joiner_disconnect_is_removed_from_waiting_lobby_with_event() {
        let mut state = test_host_state(NetworkRacePhase::WaitingForHost);

        let closed_game =
            handle_player_disconnect(&mut state, PlayerId(2), std::time::Instant::now());

        assert!(!closed_game);
        assert_eq!(state.players.len(), 1);
        assert_eq!(state.race.players.len(), 1);
        assert!(state.players.iter().all(|player| player.id != PlayerId(2)));
        assert!(
            state
                .events
                .iter()
                .any(|event| event == "alex disconnected")
        );
    }

    #[test]
    fn host_disconnect_sends_game_closed_to_joiners() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let mut remote_client = std::net::TcpStream::connect(address).unwrap();
        let (server_stream, _) = listener.accept().unwrap();
        let mut state = test_host_state(NetworkRacePhase::WaitingForHost);
        state.clients.push(ConnectedClient {
            player_id: PlayerId(2),
            stream: server_stream,
        });

        let closed_game =
            handle_player_disconnect(&mut state, PlayerId(1), std::time::Instant::now());

        let mut reader = BufReader::new(&mut remote_client);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();

        assert!(closed_game);
        assert!(state.clients.is_empty());
        assert!(matches!(
            decode_server_message(line.trim_end()).unwrap(),
            ServerMessage::Error { ref message } if message == "Game closed: host left"
        ));
        assert!(state.players.iter().all(|player| !player.connected));
    }

    #[test]
    fn rematch_rebuilds_race_from_connected_lobby_players() {
        let mut state = test_host_state(NetworkRacePhase::Finished);
        state.runtime.lifecycle.placements = vec![RacePlayerId(2), RacePlayerId(1)];
        state.race_results_sent = true;
        state.players.push(LobbyPlayer {
            id: PlayerId(3),
            name: "casey".to_string(),
            kind: PlayerKind::Human,
            color: AssignedColor::Green,
            ready: true,
            connected: true,
            ai_difficulty: None,
            ai_wpm: None,
        });

        reset_race_from_lobby(&mut state).unwrap();

        assert_eq!(state.phase, NetworkRacePhase::WaitingForHost);
        assert_eq!(state.race.players.len(), 3);
        assert!(
            state
                .race
                .players
                .iter()
                .any(|player| player.id == RacePlayerId(3))
        );
        assert!(state.runtime.lifecycle.placements.is_empty());
        assert!(!state.race_results_sent);
        assert!(state.events.is_empty());
    }

    #[test]
    fn race_rebuild_excludes_unready_lobby_players() {
        let mut state = test_host_state(NetworkRacePhase::WaitingForHost);
        state.players[0].ready = true;
        state.players[1].ready = false;

        reset_race_from_lobby(&mut state).unwrap();

        assert!(client_is_in_current_race(&state.race, PlayerId(1)));
        assert!(!client_is_in_current_race(&state.race, PlayerId(2)));
        assert!(state.players.iter().any(|player| player.id == PlayerId(2)));
    }

    #[test]
    fn late_joiner_is_not_part_of_current_race_until_rematch() {
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.players.push(LobbyPlayer {
            id: PlayerId(3),
            name: "casey".to_string(),
            kind: PlayerKind::Human,
            color: AssignedColor::Green,
            ready: true,
            connected: true,
            ai_difficulty: None,
            ai_wpm: None,
        });

        assert!(!client_is_in_current_race(&state.race, PlayerId(3)));

        state.phase = NetworkRacePhase::Finished;
        reset_race_from_lobby(&mut state).unwrap();

        assert!(client_is_in_current_race(&state.race, PlayerId(3)));
    }

    #[test]
    fn return_to_lobby_resets_finished_race() {
        let mut state = test_host_state(NetworkRacePhase::Finished);
        state.runtime.lifecycle.placements = vec![RacePlayerId(2), RacePlayerId(1)];
        state.race_results_sent = true;

        return_to_lobby(&mut state).unwrap();

        assert_eq!(state.phase, NetworkRacePhase::WaitingForHost);
        assert!(state.runtime.lifecycle.placements.is_empty());
        assert!(!state.race_results_sent);
        assert_eq!(state.race.players.len(), 2);
    }

    #[test]
    fn return_to_lobby_cancels_active_race() {
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.snapshot_sequence = 7;
        state.runtime.lifecycle.placements = vec![RacePlayerId(1)];
        state
            .runtime
            .player_effects
            .insert(RacePlayerId(1), Default::default());

        return_to_lobby(&mut state).unwrap();

        assert_eq!(state.snapshot_sequence, 8);
        assert_eq!(state.phase, NetworkRacePhase::WaitingForHost);
        assert!(state.runtime.lifecycle.placements.is_empty());
        assert!(state.runtime.player_effects.is_empty());
        assert!(state.events.iter().any(|event| event == "Race cancelled"));
        assert_eq!(state.race.players.len(), 2);
    }

    #[test]
    fn rematch_keeps_network_ai_racers_and_resets_ai_timing_state() {
        let now = Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Finished);
        state.players[1].kind = PlayerKind::Bot;
        state.ai_racers.insert(
            PlayerId(2),
            NetworkAiRacer {
                difficulty: AiDifficulty::Easy,
                words_per_minute: 60.0,
                char_budget: 5.0,
                last_update: now - Duration::from_secs(5),
            },
        );
        finish_player(&mut state, RacePlayerId(2), now);

        reset_race_from_lobby(&mut state).unwrap();

        assert!(client_is_in_current_race(&state.race, PlayerId(2)));
        assert_eq!(
            state.race.player(RacePlayerId(2)).unwrap().state.word_index,
            0
        );
        let ai = state.ai_racers.get(&PlayerId(2)).unwrap();
        assert_eq!(ai.char_budget, 0.0);
        assert!(ai.last_update >= now);
    }

    #[test]
    fn host_can_add_remove_and_retune_ai_in_lobby() {
        let mut state = test_host_state(NetworkRacePhase::WaitingForHost);

        add_lobby_ai_racer(&mut state).unwrap();

        let ai_player = state
            .players
            .iter()
            .find(|player| player.kind == PlayerKind::Bot)
            .unwrap()
            .clone();
        assert!(state.ai_racers.contains_key(&ai_player.id));
        assert!(client_is_in_current_race(&state.race, ai_player.id));

        set_lobby_ai_difficulty(&mut state, Some(ai_player.id), AiDifficulty::Hard).unwrap();

        let ai_player = state
            .players
            .iter()
            .find(|player| player.id == ai_player.id)
            .unwrap();
        assert_eq!(ai_player.ai_difficulty, Some(AiDifficulty::Hard.into()));
        assert!(state.ai_racers.get(&ai_player.id).unwrap().words_per_minute >= 55.0);

        let ai_player_id = ai_player.id;
        remove_lobby_player(&mut state, ai_player_id).unwrap();

        assert!(!state.ai_racers.contains_key(&ai_player_id));
        assert!(!state.players.iter().any(|player| player.id == ai_player_id));
        assert!(!client_is_in_current_race(&state.race, ai_player_id));
    }

    #[test]
    fn host_can_kick_joiner_in_lobby_but_not_self() {
        let mut state = test_host_state(NetworkRacePhase::WaitingForHost);

        assert!(remove_lobby_player(&mut state, PlayerId(1)).is_err());
        remove_lobby_player(&mut state, PlayerId(2)).unwrap();

        assert!(!state.players.iter().any(|player| player.id == PlayerId(2)));
        assert!(!client_is_in_current_race(&state.race, PlayerId(2)));
    }

    #[test]
    fn host_ready_command_updates_host_player() {
        let state = Arc::new(Mutex::new(HostState {
            clients: Vec::new(),
            players: vec![LobbyPlayer {
                id: PlayerId(1),
                name: "host".to_string(),
                kind: PlayerKind::Human,
                color: AssignedColor::Cyan,
                ready: true,
                connected: true,
                ai_difficulty: None,
                ai_wpm: None,
            }],
            race: test_race_state(),
            ai_racers: HashMap::new(),
            word_list: test_word_list(),
            bonuses: test_bonus_state(),
            item_registry: ItemRegistry::builtin(),
            active_mod_config: test_active_mod_config(),
            max_players: 6,
            ai_difficulty: AiDifficulty::Easy,
            runtime: RaceRuntimeState::new(),
            phase: NetworkRacePhase::WaitingForHost,
            snapshot_sequence: 0,
            events: Vec::new(),
            debug_log: None,
            race_results_sent: false,
        }));

        update_host_ready(&state, true);

        let state = state.lock().unwrap();
        assert!(state.players[0].ready);
    }

    #[test]
    fn race_snapshot_includes_phase_players_and_recent_events() {
        let mut state = HostState {
            clients: Vec::new(),
            players: test_players(true),
            race: test_race_state(),
            ai_racers: HashMap::new(),
            word_list: test_word_list(),
            bonuses: test_bonus_state(),
            item_registry: ItemRegistry::builtin(),
            active_mod_config: test_active_mod_config(),
            max_players: 6,
            ai_difficulty: AiDifficulty::Easy,
            runtime: RaceRuntimeState::new(),
            phase: NetworkRacePhase::Countdown {
                remaining_seconds: 3,
            },
            snapshot_sequence: 0,
            events: Vec::new(),
            debug_log: None,
            race_results_sent: false,
        };
        push_event(&mut state, "Countdown started".to_string());

        let snapshot = build_race_snapshot(&mut state);

        assert_eq!(
            snapshot.phase,
            NetworkRacePhase::Countdown {
                remaining_seconds: 3
            }
        );
        assert_eq!(snapshot.sequence, 1);
        assert_eq!(snapshot.players.len(), 2);
        assert_eq!(snapshot.track_words, vec!["one", "two"]);
        assert_eq!(snapshot.bonuses.len(), 1);
        assert_eq!(snapshot.bonuses[0].after_word_index, 0);
        assert_eq!(snapshot.bonuses[0].choices[0].word, "dash");
        assert_eq!(snapshot.events, vec!["Countdown started"]);
    }

    #[test]
    fn race_snapshot_reflects_applied_key_input() {
        let mut state = HostState {
            clients: Vec::new(),
            players: test_players(true),
            race: test_race_state(),
            ai_racers: HashMap::new(),
            word_list: test_word_list(),
            bonuses: test_bonus_state(),
            item_registry: ItemRegistry::builtin(),
            active_mod_config: test_active_mod_config(),
            max_players: 6,
            ai_difficulty: AiDifficulty::Easy,
            runtime: RaceRuntimeState::new(),
            phase: NetworkRacePhase::Racing,
            snapshot_sequence: 0,
            events: Vec::new(),
            debug_log: None,
            race_results_sent: false,
        };

        state
            .race
            .apply_key_input(
                RacePlayerId(2),
                KeyAction::Char('o'),
                std::time::Instant::now(),
            )
            .unwrap();

        let snapshot = build_race_snapshot(&mut state);
        let alex = snapshot
            .players
            .iter()
            .find(|player| player.id == PlayerId(2))
            .unwrap();

        assert_eq!(alex.word_index, 0);
        assert_eq!(alex.input, "o");
        assert_eq!(alex.typo_index, None);
    }

    #[test]
    fn race_snapshots_are_broadcast_to_lobby_observers() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();

        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let observer_stream = stream.try_clone().unwrap();
            let mut state = test_host_state(NetworkRacePhase::Racing);
            state.clients.push(ConnectedClient {
                player_id: PlayerId(9),
                stream: observer_stream,
            });
            broadcast_race_snapshot(&mut state).unwrap();
        });

        let client = std::net::TcpStream::connect(address).unwrap();
        let mut reader = BufReader::new(client);
        let mut snapshot_line = String::new();
        reader.read_line(&mut snapshot_line).unwrap();

        assert!(matches!(
            decode_server_message(snapshot_line.trim_end()).unwrap(),
            ServerMessage::RaceSnapshot(_)
        ));
        server.join().unwrap();
    }

    #[test]
    fn network_key_input_can_start_bonus_attempt_at_gap() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.race.players[1].state.word_index = 1;

        apply_network_key_input(&mut state, PlayerId(2), KeyAction::Char('d'), now);

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(alex.state.word_index, 1);
        assert_eq!(alex.state.input, "d");
        assert_eq!(alex.state.typo_index, None);
        assert!(state.runtime.bonus_attempts.contains_key(&PlayerId(2)));
    }

    #[test]
    fn network_bonus_claim_places_choice_on_cooldown() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.race.players[1].state.word_index = 1;

        for action in [
            KeyAction::Char('d'),
            KeyAction::Char('a'),
            KeyAction::Char('s'),
            KeyAction::Char('h'),
            KeyAction::Space,
        ] {
            apply_network_key_input(&mut state, PlayerId(2), action, now);
        }

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(alex.state.input, "");
        assert!(!state.runtime.bonus_attempts.contains_key(&PlayerId(2)));
        assert_eq!(state.runtime.spent_bonus_gaps.get(&PlayerId(2)), Some(&0));
        assert!(matches!(
            state.bonuses.points[0].choices[0].status,
            BonusChoiceStatus::Cooldown { .. }
        ));
        assert!(
            state
                .events
                .iter()
                .any(|event| event.starts_with("alex got "))
        );
    }

    #[test]
    fn losing_contested_network_bonus_forces_player_to_main_word() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.race.players[0].state.word_index = 1;
        state.race.players[1].state.word_index = 1;

        apply_network_key_input(&mut state, PlayerId(1), KeyAction::Char('d'), now);
        apply_network_key_input(&mut state, PlayerId(2), KeyAction::Char('d'), now);
        state.bonuses.points[0].choices[0].status = BonusChoiceStatus::Cooldown {
            until: now + Duration::from_secs(4),
        };
        for action in [
            KeyAction::Char('a'),
            KeyAction::Char('s'),
            KeyAction::Char('h'),
            KeyAction::Space,
        ] {
            apply_network_key_input(&mut state, PlayerId(2), action, now);
        }

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(alex.state.word_index, 1);
        assert_eq!(alex.state.input, "");
        assert_eq!(state.runtime.spent_bonus_gaps.get(&PlayerId(2)), Some(&0));

        apply_network_key_input(&mut state, PlayerId(2), KeyAction::Char('s'), now);

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(alex.state.input, "s");
        assert_eq!(alex.state.typo_index, Some(0));
        assert!(!state.runtime.bonus_attempts.contains_key(&PlayerId(2)));
    }

    #[test]
    fn network_mushroom_boost_advances_one_word_immediately() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);

        activate_network_pickup(
            &mut state,
            PlayerId(2),
            ItemPickup::Held(HeldItem::Mushroom),
            now,
        );

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(alex.state.word_index, 1);
        assert!(alex.state.active_effects.iter().any(|effect| {
            matches!(
                effect,
                ActiveEffect::Mushroom {
                    remaining_words: 2,
                    ..
                }
            )
        }));
    }

    #[test]
    fn network_banana_resets_human_target_to_word_start_without_stun() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.race.players[0].state.word_index = 1;
        state.race.players[1].state.word_index = 0;
        state.race.players[1].state.input = "tw".to_string();

        activate_network_pickup(
            &mut state,
            PlayerId(1),
            ItemPickup::Held(HeldItem::Banana),
            now,
        );

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(alex.state.input, "");
        assert_eq!(alex.state.typo_index, None);
        assert!(
            state
                .runtime
                .player_effects
                .get(&RacePlayerId(2))
                .and_then(|effects| effects.stunned_until)
                .is_none()
        );
        assert!(
            state
                .runtime
                .player_effects
                .get(&RacePlayerId(2))
                .and_then(|effects| effects.impact_cue)
                .is_some_and(|cue| cue.until > now && cue.kind == RaceImpactCueKind::Banana)
        );
        assert!(
            state
                .runtime
                .player_effects
                .get(&RacePlayerId(1))
                .and_then(|effects| effects.item_cue.clone())
                .is_some()
        );
    }

    #[test]
    fn network_shield_blocks_banana_and_is_consumed() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.race.players[0].state.word_index = 1;
        state.race.players[1].state.word_index = 0;

        activate_network_pickup(&mut state, PlayerId(2), ItemPickup::Shield, now);
        activate_network_pickup(
            &mut state,
            PlayerId(1),
            ItemPickup::Held(HeldItem::Banana),
            now,
        );

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert!(!alex.state.has_active_shield(now));
        assert_eq!(
            state
                .runtime
                .player_effects
                .get(&RacePlayerId(2))
                .and_then(|effects| effects.impact_cue)
                .map(|cue| cue.kind),
            Some(RaceImpactCueKind::ShieldBlock)
        );
    }

    #[test]
    fn network_focus_pickup_marks_snapshot_as_focused() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);

        activate_network_pickup(
            &mut state,
            PlayerId(1),
            ItemPickup::Held(HeldItem::Focus),
            now,
        );
        let snapshot = build_race_snapshot(&mut state);

        assert!(snapshot.players[0].focused);
    }

    #[test]
    fn network_focused_ai_racer_gets_small_wpm_boost() {
        assert_eq!(
            crate::game::ai_driver::ai_effective_wpm(60.0, true, false, 10, 70),
            70.0
        );
        assert_eq!(
            crate::game::ai_driver::ai_effective_wpm(60.0, false, false, 10, 70),
            60.0
        );
    }

    #[test]
    fn network_cyclone_reverses_first_place_target_word() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.race.players[1].state.word_index = 1;

        activate_network_pickup(
            &mut state,
            PlayerId(1),
            ItemPickup::Held(HeldItem::Cyclone),
            now,
        );

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(alex.state.word_override(1), Some("owt"));
        assert!(
            state
                .runtime
                .player_effects
                .get(&RacePlayerId(2))
                .and_then(|effects| effects.stunned_until)
                .is_some_and(|until| until > now)
        );
        assert!(
            state
                .events
                .iter()
                .any(|event| event == "host hit alex with Cyclone")
        );
    }

    #[test]
    fn network_cyclone_misses_when_attacker_is_first_place() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.race.players[0].state.word_index = 1;
        state.race.players[1].state.word_index = 0;

        activate_network_pickup(
            &mut state,
            PlayerId(1),
            ItemPickup::Held(HeldItem::Cyclone),
            now,
        );

        let host = state.race.player(RacePlayerId(1)).unwrap();
        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(host.state.word_override(1), None);
        assert_eq!(alex.state.word_override(0), None);
        assert!(
            state
                .events
                .iter()
                .any(|event| event == "host missed Cyclone")
        );
    }

    #[test]
    fn network_cyclone_targets_first_place_ai() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.players[1].kind = PlayerKind::Bot;
        state.race.players[0].state.word_index = 0;
        state.race.players[1].state.word_index = 1;
        state.ai_racers.insert(
            PlayerId(2),
            NetworkAiRacer {
                difficulty: AiDifficulty::Easy,
                words_per_minute: 60.0,
                char_budget: 4.0,
                last_update: now,
            },
        );

        activate_network_pickup(
            &mut state,
            PlayerId(1),
            ItemPickup::Held(HeldItem::Cyclone),
            now,
        );

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(alex.state.word_override(1), Some("owt"));
        assert_eq!(state.ai_racers.get(&PlayerId(2)).unwrap().char_budget, 0.0);
        assert!(
            state
                .events
                .iter()
                .any(|event| event == "host hit alex with Cyclone")
        );
    }

    #[test]
    fn network_cyclone_is_blocked_by_shield_and_consumes_shield() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.race.players[1].state.word_index = 1;
        state.race.players[1]
            .state
            .active_effects
            .push(ActiveEffect::Shield {
                until: now + Duration::from_secs(5),
            });

        activate_network_pickup(
            &mut state,
            PlayerId(1),
            ItemPickup::Held(HeldItem::Cyclone),
            now,
        );

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(alex.state.word_override(1), None);
        assert!(!alex.state.has_active_shield(now));
        assert!(
            state
                .events
                .iter()
                .any(|event| event == "alex blocked Cyclone")
        );
    }

    #[test]
    fn network_squid_ink_marks_all_targets_in_range() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.race.players[0].state.word_index = 1;
        state.race.players[1].state.word_index = 3;

        activate_network_pickup(
            &mut state,
            PlayerId(1),
            ItemPickup::Held(HeldItem::SquidInk),
            now,
        );

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert!(alex.state.is_inked_at(now));
        assert_eq!(
            state
                .runtime
                .player_effects
                .get(&RacePlayerId(2))
                .and_then(|effects| effects.impact_cue)
                .map(|cue| cue.kind),
            Some(RaceImpactCueKind::SquidInk)
        );

        let snapshot = build_race_snapshot(&mut state);
        assert!(snapshot.players[1].inked);
    }

    #[test]
    fn network_squid_ink_is_blocked_by_shield() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.race.players[1]
            .state
            .active_effects
            .push(ActiveEffect::Shield {
                until: now + Duration::from_secs(5),
            });

        activate_network_pickup(
            &mut state,
            PlayerId(1),
            ItemPickup::Held(HeldItem::SquidInk),
            now,
        );

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert!(!alex.state.is_inked_at(now));
        assert!(!alex.state.has_active_shield(now));
        assert!(
            state
                .events
                .iter()
                .any(|event| event == "alex blocked Squid Ink")
        );
    }

    #[test]
    fn race_status_records_finish_order_and_finishes_when_all_connected_finish() {
        let now = std::time::Instant::now();
        let mut state = HostState {
            clients: Vec::new(),
            players: test_players(true),
            race: test_race_state(),
            ai_racers: HashMap::new(),
            word_list: test_word_list(),
            bonuses: test_bonus_state(),
            item_registry: ItemRegistry::builtin(),
            active_mod_config: test_active_mod_config(),
            max_players: 6,
            ai_difficulty: AiDifficulty::Easy,
            runtime: RaceRuntimeState::new(),
            phase: NetworkRacePhase::Racing,
            snapshot_sequence: 0,
            events: Vec::new(),
            debug_log: None,
            race_results_sent: false,
        };

        finish_player(&mut state, RacePlayerId(2), now);
        update_race_status(&mut state, now);

        assert_eq!(state.phase, NetworkRacePhase::Racing);
        assert_eq!(state.runtime.lifecycle.placements, vec![RacePlayerId(2)]);

        finish_player(&mut state, RacePlayerId(1), now);
        update_race_status(&mut state, now);

        assert_eq!(state.phase, NetworkRacePhase::Finished);
        assert_eq!(
            state.runtime.lifecycle.placements,
            vec![RacePlayerId(2), RacePlayerId(1)]
        );
        assert!(state.events.iter().any(|event| event == "Race finished"));
    }

    #[test]
    fn race_status_timeout_places_unfinished_connected_racers_by_progress() {
        let now = std::time::Instant::now();
        let mut state = HostState {
            clients: Vec::new(),
            players: test_players(true),
            race: test_race_state(),
            ai_racers: HashMap::new(),
            word_list: test_word_list(),
            bonuses: test_bonus_state(),
            item_registry: ItemRegistry::builtin(),
            active_mod_config: test_active_mod_config(),
            max_players: 6,
            ai_difficulty: AiDifficulty::Easy,
            runtime: RaceRuntimeState::new(),
            phase: NetworkRacePhase::Racing,
            snapshot_sequence: 0,
            events: Vec::new(),
            debug_log: None,
            race_results_sent: false,
        };
        finish_player(&mut state, RacePlayerId(2), now);
        state
            .race
            .apply_key_input(RacePlayerId(1), KeyAction::Char('o'), now)
            .unwrap();

        update_race_status(&mut state, now);
        update_race_status(&mut state, now + POST_FIRST_FINISH_TIMEOUT);

        assert_eq!(state.phase, NetworkRacePhase::Finished);
        assert_eq!(
            state.runtime.lifecycle.placements,
            vec![RacePlayerId(2), RacePlayerId(1)]
        );
    }

    #[test]
    fn race_status_finishes_when_all_racers_disconnect() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        for player in &mut state.players {
            player.connected = false;
        }
        for player in &mut state.race.players {
            player.connected = false;
        }

        update_race_status(&mut state, now);

        assert_eq!(state.phase, NetworkRacePhase::Finished);
        assert!(state.runtime.lifecycle.placements.is_empty());
        assert!(state.events.iter().any(|event| event == "Race finished"));
    }

    #[test]
    fn countdown_continues_when_one_racer_remains_connected() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Countdown {
            remaining_seconds: 2,
        });
        state.players[1].connected = false;
        state.race.players[1].connected = false;

        reconcile_phase_after_disconnect(&mut state, now);

        assert_eq!(
            state.phase,
            NetworkRacePhase::Countdown {
                remaining_seconds: 2
            }
        );
    }

    #[test]
    fn countdown_cancels_when_no_racers_remain_connected() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Countdown {
            remaining_seconds: 2,
        });
        for player in &mut state.players {
            player.connected = false;
        }
        for player in &mut state.race.players {
            player.connected = false;
        }

        reconcile_phase_after_disconnect(&mut state, now);

        assert_eq!(state.phase, NetworkRacePhase::WaitingForHost);
        assert!(
            state
                .events
                .iter()
                .any(|event| event == "Countdown cancelled")
        );
    }

    #[test]
    fn countdown_continues_when_multiple_racers_remain_connected() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Countdown {
            remaining_seconds: 2,
        });

        reconcile_phase_after_disconnect(&mut state, now);

        assert_eq!(
            state.phase,
            NetworkRacePhase::Countdown {
                remaining_seconds: 2
            }
        );
    }

    #[test]
    fn race_results_are_broadcast_only_once() {
        let mut state = HostState {
            clients: Vec::new(),
            players: test_players(true),
            race: test_race_state(),
            ai_racers: HashMap::new(),
            word_list: test_word_list(),
            bonuses: test_bonus_state(),
            item_registry: ItemRegistry::builtin(),
            active_mod_config: test_active_mod_config(),
            max_players: 6,
            ai_difficulty: AiDifficulty::Easy,
            runtime: RaceRuntimeState::new(),
            phase: NetworkRacePhase::Finished,
            snapshot_sequence: 0,
            events: Vec::new(),
            debug_log: None,
            race_results_sent: false,
        };

        broadcast_race_results_once(&mut state).unwrap();
        assert!(state.race_results_sent);

        broadcast_race_results_once(&mut state).unwrap();
        assert!(state.race_results_sent);
    }

    #[test]
    fn race_result_rows_include_stats_and_status_for_every_racer() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Finished);
        finish_player(&mut state, RacePlayerId(2), now);
        state.runtime.lifecycle.placements = vec![RacePlayerId(2)];

        let host = state
            .race
            .players
            .iter_mut()
            .find(|player| player.id == RacePlayerId(1))
            .unwrap();
        host.connected = false;
        host.state.word_index = 1;
        host.state.stats = TypingStats {
            typed_chars: 10,
            correct_chars: 8,
            typo_chars: 2,
            backspaces: 3,
            completed_words: 1,
        };

        let rows = build_race_result_rows(&state, now + Duration::from_secs(30));

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].player_id, PlayerId(2));
        assert_eq!(rows[0].status, RaceResultStatus::Finished);
        assert_eq!(rows[0].progress_words, 2);
        assert_eq!(rows[1].player_id, PlayerId(1));
        assert_eq!(rows[1].status, RaceResultStatus::Disconnected);
        assert_eq!(rows[1].progress_words, 1);
        assert_eq!(rows[1].accuracy_percent, 80);
        assert_eq!(rows[1].typo_chars, 2);
        assert_eq!(rows[1].backspaces, 3);
    }

    fn send_hello(client: &mut std::net::TcpStream) {
        let hello = encode_client_message(&ClientMessage::Hello {
            name: "alex".to_string(),
            client_version: env!("CARGO_PKG_VERSION").to_string(),
        })
        .unwrap();
        writeln!(client, "{hello}").unwrap();
    }

    fn lobby_player(id: PlayerId, name: &str, kind: PlayerKind, connected: bool) -> LobbyPlayer {
        LobbyPlayer {
            id,
            name: name.to_string(),
            kind,
            color: AssignedColor::Cyan,
            ready: false,
            connected,
            ai_difficulty: None,
            ai_wpm: None,
        }
    }

    fn test_players(joiner_ready: bool) -> Vec<LobbyPlayer> {
        vec![
            LobbyPlayer {
                id: PlayerId(1),
                name: "host".to_string(),
                kind: PlayerKind::Human,
                color: AssignedColor::Cyan,
                ready: true,
                connected: true,
                ai_difficulty: None,
                ai_wpm: None,
            },
            LobbyPlayer {
                id: PlayerId(2),
                name: "alex".to_string(),
                kind: PlayerKind::Human,
                color: AssignedColor::Red,
                ready: joiner_ready,
                connected: true,
                ai_difficulty: None,
                ai_wpm: None,
            },
        ]
    }

    fn test_race_state() -> RaceState {
        let now = std::time::Instant::now();
        let mut race = RaceState::new(Track::new(vec!["one".to_string(), "two".to_string()]));
        race.add_player(RacePlayerId(1), "host", PlayerColorId::Cyan, now);
        race.add_player(RacePlayerId(2), "alex", PlayerColorId::Red, now);
        race
    }

    fn test_bonus_state() -> BonusState {
        BonusState::with_points(
            vec![BonusPoint::new(
                0,
                [
                    BonusChoice::available("dash"),
                    BonusChoice::available("drift"),
                    BonusChoice::available("spark"),
                ],
            )],
            vec!["dash".to_string(), "drift".to_string(), "spark".to_string()],
        )
    }

    fn test_word_list() -> WordList {
        WordList {
            words: vec![
                "one".to_string(),
                "two".to_string(),
                "three".to_string(),
                "four".to_string(),
            ],
        }
    }

    fn test_active_mod_config() -> ActiveModConfig {
        let item_registry = ItemRegistry::builtin();
        ActiveModConfig::new(
            &WordSetDefinition {
                metadata: ContentMetadata::built_in("classic", "Classic"),
                words: test_word_list(),
            },
            &item_registry,
            None,
        )
    }

    fn test_single_item_registry(pickup: ItemPickup) -> ItemRegistry {
        let (id, name, activation) = match pickup {
            ItemPickup::Held(HeldItem::Mushroom) => ("mushroom", "Mushroom", ItemActivation::Held),
            ItemPickup::Held(HeldItem::Banana) => ("banana", "Banana", ItemActivation::Held),
            ItemPickup::Held(HeldItem::Focus) => ("focus", "Focus", ItemActivation::Held),
            ItemPickup::Held(HeldItem::Cyclone) => ("cyclone", "Cyclone", ItemActivation::Held),
            ItemPickup::Held(HeldItem::SquidInk) => {
                ("squid_ink", "Squid Ink", ItemActivation::Held)
            }
            ItemPickup::Shield => ("shield", "Shield", ItemActivation::Immediate),
        };
        ItemRegistry::new(vec![ItemDefinition::built_in(
            id, name, pickup, activation, 1, 1,
        )])
        .unwrap()
    }

    fn test_host_state(phase: NetworkRacePhase) -> HostState {
        HostState {
            clients: Vec::new(),
            players: test_players(true),
            race: test_race_state(),
            ai_racers: HashMap::new(),
            word_list: test_word_list(),
            bonuses: test_bonus_state(),
            item_registry: ItemRegistry::builtin(),
            active_mod_config: test_active_mod_config(),
            max_players: 6,
            ai_difficulty: AiDifficulty::Easy,
            runtime: RaceRuntimeState::new(),
            phase,
            snapshot_sequence: 0,
            events: Vec::new(),
            debug_log: None,
            race_results_sent: false,
        }
    }

    fn finish_player(state: &mut HostState, id: RacePlayerId, now: std::time::Instant) {
        let player = state
            .race
            .players
            .iter_mut()
            .find(|player| player.id == id)
            .unwrap();
        player.state.word_index = state.race.track.len();
        player.state.finished_at = Some(now);
    }
}
