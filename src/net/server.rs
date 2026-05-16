//! Minimal TCP host for Milestone 4.
//!
//! The host currently supports a persistent lobby connection. Joiners can stay
//! connected, receive lobby snapshots, and send simple readiness updates. Race
//! snapshots and key input will be layered on this socket structure next.

use std::{
    collections::HashMap,
    io::{self, BufRead, BufReader, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{bail, Context, Result};
use rand::thread_rng;

use crate::game::{
    bonus::{claim_bonus_choice, BonusChoiceStatus, BonusState},
    effects::ActiveEffect,
    items::{select_nearest_banana_target, HeldItem, ItemPickup, ItemRegistry, RacerPosition},
    mods::ActiveModConfig,
    race::{PlayerColorId, RacePlayerId, RaceState},
    track::{Track, WordList},
    typing::{first_typo_index, KeyAction},
};

use super::log::{push_network_log, SharedNetworkLog};
use super::protocol::{
    decode_client_message, encode_server_message, AssignedColor, AttackDirectionSnapshot,
    BonusChoiceSnapshot, BonusChoiceSnapshotStatus, BonusPointSnapshot, ClientMessage,
    ItemCueSnapshot, ItemCueSnapshotKind, LobbyPlayer, NetworkRacePhase, PlayerId, PlayerSnapshot,
    ProtocolKey, RaceSnapshot, ServerMessage,
};

const COLOR_ROTATION: [AssignedColor; 6] = [
    AssignedColor::Cyan,
    AssignedColor::Red,
    AssignedColor::Green,
    AssignedColor::Blue,
    AssignedColor::Yellow,
    AssignedColor::Magenta,
];
const POST_FIRST_FINISH_TIMEOUT: Duration = Duration::from_secs(30);
const RACE_SNAPSHOT_INTERVAL: Duration = Duration::from_millis(50);
const MUSHROOM_BOOST_WORDS: usize = 3;
const MUSHROOM_WPM: f64 = 180.0;
const BANANA_RANGE_WORDS: usize = 10;
const BANANA_STUN: Duration = Duration::from_secs(2);
const ITEM_IMPACT_BLINK: Duration = Duration::from_millis(1200);
const ITEM_CUE_DURATION: Duration = Duration::from_millis(1500);
const SHIELD_DURATION: Duration = Duration::from_secs(5);
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
    pub ready_signal: Option<Sender<SocketAddr>>,
    pub console_logging: bool,
    pub debug_log: Option<SharedNetworkLog>,
}

struct ConnectedClient {
    player_id: PlayerId,
    stream: TcpStream,
}

struct HostState {
    players: Vec<LobbyPlayer>,
    clients: Vec<ConnectedClient>,
    race: RaceState,
    bonuses: BonusState,
    item_registry: ItemRegistry,
    active_mod_config: ActiveModConfig,
    bonus_attempts: HashMap<PlayerId, NetworkBonusAttempt>,
    spent_bonus_gaps: HashMap<PlayerId, usize>,
    player_effects: HashMap<PlayerId, NetworkPlayerEffects>,
    phase: NetworkRacePhase,
    snapshot_sequence: u64,
    events: Vec<String>,
    placements: Vec<PlayerId>,
    first_finished_at: Option<Instant>,
    debug_log: Option<SharedNetworkLog>,
    race_results_sent: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NetworkBonusAttempt {
    point_index: usize,
    choice_index: usize,
}

#[derive(Debug, Clone, Copy, Default)]
struct NetworkPlayerEffects {
    stunned_until: Option<Instant>,
    impact_until: Option<Instant>,
    item_cue: Option<NetworkItemCue>,
}

#[derive(Debug, Clone, Copy)]
struct NetworkItemCue {
    kind: NetworkItemCueKind,
    until: Instant,
}

#[derive(Debug, Clone, Copy)]
enum NetworkItemCueKind {
    Banana { direction: AttackDirectionSnapshot },
}

pub fn run_host(config: HostConfig) -> Result<()> {
    SERVER_CONSOLE_LOGGING.store(config.console_logging, Ordering::Relaxed);

    if config.max_players == 0 || config.max_players > COLOR_ROTATION.len() {
        bail!("max players must be between 1 and {}", COLOR_ROTATION.len());
    }

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
            "server listening addr={local_addr} max_players={} words={}",
            config.max_players,
            config.track.len()
        ),
    );
    push_network_log(&config.debug_log, config.active_mod_config.log_summary());

    let bonuses = BonusState::generate(&config.track, &config.word_list);
    let mut race = RaceState::new(config.track);
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
            color: COLOR_ROTATION[0],
            ready: false,
            connected: true,
        });
        next_player_id = 2;
    }

    let state = Arc::new(Mutex::new(HostState {
        players,
        clients: Vec::new(),
        race,
        bonuses,
        item_registry: config.item_registry,
        active_mod_config: config.active_mod_config,
        bonus_attempts: HashMap::new(),
        spent_bonus_gaps: HashMap::new(),
        player_effects: HashMap::new(),
        phase: NetworkRacePhase::WaitingForHost,
        snapshot_sequence: 0,
        events: Vec::new(),
        placements: Vec::new(),
        first_finished_at: None,
        debug_log: config.debug_log,
        race_results_sent: false,
    }));

    server_println!("TypeKart host listening on {local_addr}");
    if has_embedded_host_player(&state) {
        server_println!("Host lobby commands: ready, unready, lobby, start");
        spawn_host_command_loop(Arc::clone(&state));
    }
    server_println!("Waiting for joiners. Press Ctrl-C to stop.");

    for stream in listener.incoming() {
        let stream = stream.context("failed to accept client connection")?;
        let peer = stream.peer_addr().ok();

        let player_name = match read_join_hello(&stream) {
            Ok(name) => name,
            Err(error) => {
                server_eprintln!("Rejected connection: {error:#}");
                continue;
            }
        };

        let (player_id, assigned_color) = {
            let mut state = state.lock().expect("host state poisoned");
            if connected_player_count(&state.players) >= config.max_players {
                send_server_message(
                    stream,
                    &ServerMessage::Error {
                        message: "Lobby is full".to_string(),
                    },
                )?;
                push_network_log(&state.debug_log, "join rejected: lobby full");
                continue;
            }

            if state.players.iter().any(|player| {
                player.connected && player.name.eq_ignore_ascii_case(player_name.trim())
            }) {
                send_server_message(
                    stream,
                    &ServerMessage::Error {
                        message: format!("Name '{player_name}' is already in use"),
                    },
                )?;
                push_network_log(
                    &state.debug_log,
                    format!("join rejected: duplicate name={player_name}"),
                );
                continue;
            }

            let player_id = PlayerId(next_player_id);
            let assigned_color = first_available_color(&state.players);
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
            state.players.push(LobbyPlayer {
                id: player_id,
                name: player_name.clone(),
                color: assigned_color,
                ready: false,
                connected: true,
            });
            state.race.add_player(
                RacePlayerId(player_id.0),
                player_name.clone(),
                assigned_color.into(),
                std::time::Instant::now(),
            );
            push_network_log(
                &state.debug_log,
                format!(
                    "{player_name} joined player={} color={assigned_color:?}",
                    player_id.0
                ),
            );
            print_lobby_snapshot(&state.players);
            broadcast_lobby_snapshot(&mut state)?;

            (player_id, assigned_color)
        };

        server_println!(
            "{} joined as player {} ({assigned_color:?}){}",
            player_name,
            player_id.0,
            peer.map(|addr| format!(" from {addr}")).unwrap_or_default()
        );

        let state_for_client = Arc::clone(&state);
        thread::spawn(move || handle_client_messages(player_id, stream, state_for_client));
        next_player_id += 1;
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
    if let Some(host) = state
        .players
        .iter_mut()
        .find(|player| player.id == PlayerId(1))
    {
        host.ready = ready;
        server_println!(
            "{} is {}",
            host.name,
            if ready { "ready" } else { "not ready" }
        );
    }
    print_lobby_snapshot(&state.players);
    if let Err(error) = broadcast_lobby_snapshot(&mut state) {
        server_eprintln!("Failed to broadcast lobby snapshot: {error:#}");
    }
}

fn read_join_hello(stream: &TcpStream) -> Result<String> {
    let mut line = String::new();
    {
        let mut reader = BufReader::new(
            stream
                .try_clone()
                .context("failed to clone client stream for reading")?,
        );
        reader
            .read_line(&mut line)
            .context("failed to read client hello")?;
    }

    let message =
        decode_client_message(line.trim_end()).context("failed to decode client hello")?;
    let ClientMessage::Hello { name, .. } = message else {
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

    Ok(name.trim().to_string())
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

fn connected_player_count(players: &[LobbyPlayer]) -> usize {
    players.iter().filter(|player| player.connected).count()
}

fn first_available_color(players: &[LobbyPlayer]) -> AssignedColor {
    COLOR_ROTATION
        .iter()
        .copied()
        .find(|color| {
            !players
                .iter()
                .any(|player| player.connected && player.color == *color)
        })
        .expect("color rotation covers the configured player limit")
}

fn handle_client_messages(player_id: PlayerId, stream: TcpStream, state: Arc<Mutex<HostState>>) {
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(line) = line else {
            break;
        };
        let Ok(message) = decode_client_message(line.trim_end()) else {
            continue;
        };

        match message {
            ClientMessage::SetReady { ready } => {
                let mut state = state.lock().expect("host state poisoned");
                if let Some(player) = state
                    .players
                    .iter_mut()
                    .find(|player| player.id == player_id)
                {
                    let name = player.name.clone();
                    player.ready = ready;
                    server_println!("{} is {}", name, if ready { "ready" } else { "not ready" });
                    push_network_log(&state.debug_log, format!("{name} ready={ready}"));
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
                if apply_network_key_input(&mut state, player_id, action, now) {
                    update_race_status(&mut state, now);
                    if let Err(error) = broadcast_race_snapshot(&mut state) {
                        server_eprintln!("Failed to broadcast race snapshot: {error:#}");
                    }
                    if state.phase == NetworkRacePhase::Finished {
                        server_println!("Race finished");
                        if let Err(error) = broadcast_race_results_once(&mut state) {
                            server_eprintln!("Failed to broadcast race results: {error:#}");
                        }
                    }
                }
            }
            ClientMessage::Leave => break,
            _ => {}
        }
    }

    let mut state = state.lock().expect("host state poisoned");
    if let Some(player) = state
        .players
        .iter_mut()
        .find(|player| player.id == player_id)
    {
        let name = player.name.clone();
        player.connected = false;
        player.ready = false;
        server_println!("{name} disconnected");
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
    state.bonus_attempts.remove(&player_id);
    state.spent_bonus_gaps.remove(&player_id);
    state.player_effects.remove(&player_id);
    state.clients.retain(|client| client.player_id != player_id);
    update_race_status(&mut state, std::time::Instant::now());
    print_lobby_snapshot(&state.players);
    if let Err(error) = broadcast_lobby_snapshot(&mut state) {
        server_eprintln!("Failed to broadcast lobby snapshot: {error:#}");
    }
    if state.phase == NetworkRacePhase::Finished {
        if let Err(error) = broadcast_race_snapshot(&mut state) {
            server_eprintln!("Failed to broadcast race snapshot: {error:#}");
        }
        if let Err(error) = broadcast_race_results_once(&mut state) {
            server_eprintln!("Failed to broadcast race results: {error:#}");
        }
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
            NetworkRacePhase::WaitingForHost | NetworkRacePhase::Lobby => {}
            NetworkRacePhase::Countdown { .. }
            | NetworkRacePhase::Racing
            | NetworkRacePhase::Finished => {
                server_println!("Race has already started");
                return;
            }
        }

        if !all_connected_players_ready(&state.players) {
            server_println!("Cannot start: all connected players must be ready");
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
        apply_network_key_input(&mut state, player_id, action, now);
    }
    apply_network_key_input(&mut state, player_id, KeyAction::Space, now);

    update_race_status(&mut state, now);
    if let Err(error) = broadcast_race_snapshot(&mut state) {
        server_eprintln!("Failed to broadcast race snapshot: {error:#}");
    }
    if state.phase == NetworkRacePhase::Finished {
        server_println!("Race finished");
        push_network_log(&state.debug_log, "race finished after host line input");
        if let Err(error) = broadcast_race_results_once(&mut state) {
            server_eprintln!("Failed to broadcast race results: {error:#}");
        }
    }
}

fn apply_network_key_input(
    state: &mut HostState,
    player_id: PlayerId,
    action: KeyAction,
    now: Instant,
) -> bool {
    if player_input_is_paused(state, player_id, now) {
        return true;
    }

    if state.bonus_attempts.contains_key(&player_id) {
        apply_network_bonus_typing_action(state, player_id, action, now);
        return true;
    }

    if let KeyAction::Char(ch) = action {
        if let Some(attempt) = network_bonus_start(state, player_id, ch, now) {
            state.bonus_attempts.insert(player_id, attempt);
            apply_network_bonus_char(state, player_id, ch);
            return true;
        }
    }

    state
        .race
        .apply_key_input(RacePlayerId(player_id.0), action, now)
        .is_some()
}

fn apply_network_bonus_typing_action(
    state: &mut HostState,
    player_id: PlayerId,
    action: KeyAction,
    now: Instant,
) {
    match action {
        KeyAction::Char(ch) => apply_network_bonus_char(state, player_id, ch),
        KeyAction::Backspace => {
            let Some(player) = state
                .race
                .players
                .iter_mut()
                .find(|player| player.id == RacePlayerId(player_id.0))
            else {
                state.bonus_attempts.remove(&player_id);
                return;
            };

            let previous_typo = player.state.typo_index;
            if player.state.input.pop().is_some() {
                player.state.stats.backspaces += 1;
            }

            let input_is_empty = player.state.input.is_empty();
            let input = player.state.input.clone();
            let _ = player;

            recalculate_network_bonus_typo(state, player_id, &input);

            let typo_cleared = previous_typo.is_some()
                && state
                    .race
                    .player(RacePlayerId(player_id.0))
                    .is_some_and(|player| player.state.typo_index.is_none());
            if typo_cleared {
                push_event_for_player(state, player_id, "Typo cleared");
            }
            if input_is_empty {
                state.bonus_attempts.remove(&player_id);
                push_event_for_player(state, player_id, "Bonus attempt cancelled");
            }
        }
        KeyAction::Space => {
            if network_bonus_completed_without_typo(state, player_id) {
                claim_network_bonus(state, player_id, now);
            } else {
                apply_network_bonus_char(state, player_id, ' ');
            }
        }
    }
}

fn network_bonus_start(
    state: &HostState,
    player_id: PlayerId,
    ch: char,
    now: Instant,
) -> Option<NetworkBonusAttempt> {
    let player = state.race.player(RacePlayerId(player_id.0))?;
    if player.state.held_item.is_some()
        || player.state.has_active_shield(now)
        || player.state.typo_index.is_some()
        || !player.state.input.is_empty()
        || player.state.is_finished()
    {
        return None;
    }

    let (point_index, point) = state.bonuses.point_for_gap(player.state.word_index)?;
    if state
        .spent_bonus_gaps
        .get(&player_id)
        .is_some_and(|after_word_index| *after_word_index == point.after_word_index)
    {
        return None;
    }

    point
        .available_choice_starting_with(ch, now)
        .map(|(choice_index, _)| NetworkBonusAttempt {
            point_index,
            choice_index,
        })
}

fn apply_network_bonus_char(state: &mut HostState, player_id: PlayerId, ch: char) {
    let Some(attempt) = state.bonus_attempts.get(&player_id).copied() else {
        return;
    };
    let Some(target) = network_bonus_target(state, attempt).map(str::to_owned) else {
        state.bonus_attempts.remove(&player_id);
        return;
    };
    let Some(player) = state
        .race
        .players
        .iter_mut()
        .find(|player| player.id == RacePlayerId(player_id.0))
    else {
        state.bonus_attempts.remove(&player_id);
        return;
    };

    let previous_typo = player.state.typo_index;
    let input_index = player.state.input.chars().count();
    let is_correct = previous_typo.is_none() && target.chars().nth(input_index) == Some(ch);

    player.state.stats.typed_chars += 1;
    if is_correct {
        player.state.stats.correct_chars += 1;
    } else {
        player.state.stats.typo_chars += 1;
    }

    player.state.input.push(ch);
    player.state.typo_index = first_typo_index(&player.state.input, &target);
    if previous_typo.is_none() && player.state.typo_index.is_some() {
        push_event_for_player(state, player_id, "Typo started");
    }
}

fn recalculate_network_bonus_typo(state: &mut HostState, player_id: PlayerId, input: &str) {
    let Some(attempt) = state.bonus_attempts.get(&player_id).copied() else {
        return;
    };
    let target = network_bonus_target(state, attempt).map(str::to_owned);
    let Some(player) = state
        .race
        .players
        .iter_mut()
        .find(|player| player.id == RacePlayerId(player_id.0))
    else {
        state.bonus_attempts.remove(&player_id);
        return;
    };

    player.state.typo_index = target
        .as_deref()
        .and_then(|target| first_typo_index(input, target));
}

fn network_bonus_completed_without_typo(state: &HostState, player_id: PlayerId) -> bool {
    let Some(attempt) = state.bonus_attempts.get(&player_id).copied() else {
        return false;
    };
    let Some(target) = network_bonus_target(state, attempt) else {
        return false;
    };
    let Some(player) = state.race.player(RacePlayerId(player_id.0)) else {
        return false;
    };

    player.state.typo_index.is_none() && player.state.input == target
}

fn claim_network_bonus(state: &mut HostState, player_id: PlayerId, now: Instant) {
    let Some(attempt) = state.bonus_attempts.remove(&player_id) else {
        return;
    };

    let after_word_index = state
        .bonuses
        .points
        .get(attempt.point_index)
        .map(|point| point.after_word_index);
    let has_nearby_racer = player_has_nearby_racer(state, player_id, 5);
    let item_registry = state.item_registry.clone();
    let mut rng = thread_rng();
    let pickup = claim_bonus_choice(
        &mut state.bonuses,
        attempt.point_index,
        attempt.choice_index,
        now,
        has_nearby_racer,
        &item_registry,
        &mut rng,
    );

    if let Some(player) = state
        .race
        .players
        .iter_mut()
        .find(|player| player.id == RacePlayerId(player_id.0))
    {
        player.state.input.clear();
        player.state.typo_index = None;
    }

    if let Some(after_word_index) = after_word_index {
        state.spent_bonus_gaps.insert(player_id, after_word_index);
    }

    let name = player_name(state, player_id).unwrap_or_else(|| format!("player {}", player_id.0));
    match pickup {
        Some(item) => {
            let item_name = item_pickup_name(item);
            push_event(state, format!("{name} picked up {item_name}"));
            push_network_log(
                &state.debug_log,
                format!("{name} picked up {item_name} from network bonus"),
            );
            activate_network_pickup(state, player_id, item, now);
        }
        None => {
            push_event(state, format!("{name} missed the bonus"));
            push_network_log(
                &state.debug_log,
                format!("{name} missed network bonus; choice was unavailable"),
            );
        }
    }
}

fn network_bonus_target(state: &HostState, attempt: NetworkBonusAttempt) -> Option<&str> {
    state
        .bonuses
        .points
        .get(attempt.point_index)?
        .choices
        .get(attempt.choice_index)
        .map(|choice| choice.word.as_str())
}

fn player_has_nearby_racer(
    state: &HostState,
    player_id: PlayerId,
    max_distance_words: usize,
) -> bool {
    let Some(player) = state.race.player(RacePlayerId(player_id.0)) else {
        return false;
    };

    state.race.players.iter().any(|other| {
        other.id != player.id
            && other.connected
            && !other.state.is_finished()
            && player.state.word_index.abs_diff(other.state.word_index) <= max_distance_words
    })
}

fn player_input_is_paused(state: &HostState, player_id: PlayerId, now: Instant) -> bool {
    if state
        .player_effects
        .get(&player_id)
        .and_then(|effects| effects.stunned_until)
        .is_some_and(|until| until > now)
    {
        return true;
    }

    state
        .race
        .player(RacePlayerId(player_id.0))
        .is_some_and(|player| player_has_active_mushroom_effect(player, now))
}

fn activate_network_pickup(
    state: &mut HostState,
    player_id: PlayerId,
    item: ItemPickup,
    now: Instant,
) {
    match item {
        ItemPickup::Held(HeldItem::Mushroom) => activate_network_mushroom(state, player_id, now),
        ItemPickup::Held(HeldItem::Banana) => activate_network_banana(state, player_id, now),
        ItemPickup::Shield => activate_network_shield(state, player_id, now),
    }
}

fn activate_network_shield(state: &mut HostState, player_id: PlayerId, now: Instant) {
    let Some(player) = state
        .race
        .players
        .iter_mut()
        .find(|player| player.id == RacePlayerId(player_id.0))
    else {
        return;
    };

    player.state.active_effects.push(ActiveEffect::Shield {
        until: now + SHIELD_DURATION,
    });
    let name = player.name.clone();
    push_event(state, format!("{name} shielded"));
}

fn activate_network_mushroom(state: &mut HostState, player_id: PlayerId, now: Instant) {
    state.bonus_attempts.remove(&player_id);
    let Some(player) = state
        .race
        .players
        .iter_mut()
        .find(|player| player.id == RacePlayerId(player_id.0))
    else {
        return;
    };

    player.state.input.clear();
    player.state.typo_index = None;
    player.state.active_effects.push(ActiveEffect::Mushroom {
        remaining_words: MUSHROOM_BOOST_WORDS,
        next_step_at: now,
        step_interval: mushroom_step_interval(),
    });
    let name = player.name.clone();
    push_event(state, format!("{name} used Mushroom"));
    advance_network_mushrooms(state, now);
}

fn activate_network_banana(state: &mut HostState, player_id: PlayerId, now: Instant) {
    let Some(attacker) = state.race.player(RacePlayerId(player_id.0)) else {
        return;
    };
    let attacker_word_index = attacker.state.word_index;
    let attacker_name = attacker.name.clone();
    let candidates = state
        .race
        .players
        .iter()
        .filter(|player| player.id != attacker.id)
        .filter(|player| player.connected)
        .filter(|player| !player.state.is_finished())
        .filter(|player| !player_is_stunned(state, PlayerId(player.id.0), now))
        .map(|player| RacerPosition {
            id: player.id.0 as usize,
            word_index: player.state.word_index,
        })
        .collect::<Vec<_>>();

    push_network_log(
        &state.debug_log,
        format!(
            "{attacker_name} banana fired from word={attacker_word_index}; candidates={}",
            network_racer_positions_summary(state, &candidates, now)
        ),
    );

    let Some(target) =
        select_nearest_banana_target(attacker_word_index, &candidates, BANANA_RANGE_WORDS)
    else {
        push_event(state, format!("{attacker_name} missed Banana"));
        push_network_log(&state.debug_log, format!("{attacker_name} banana missed"));
        return;
    };

    let target_id = PlayerId(target.id as u64);
    let direction = attack_direction(attacker_word_index, target.word_index);
    let distance = attacker_word_index.abs_diff(target.word_index);
    push_network_log(
        &state.debug_log,
        format!(
            "{attacker_name} banana target={} target_word={} direction={direction:?} distance_words={distance}",
            player_name(state, target_id).unwrap_or_else(|| format!("player {}", target_id.0)),
            target.word_index
        ),
    );
    state.player_effects.entry(player_id).or_default().item_cue = Some(NetworkItemCue {
        kind: NetworkItemCueKind::Banana { direction },
        until: now + ITEM_CUE_DURATION,
    });

    match apply_network_banana_to_player(state, target_id, now) {
        Some(BananaResolution::SpunOut) => {
            let target_name =
                player_name(state, target_id).unwrap_or_else(|| format!("player {}", target_id.0));
            push_event(state, format!("{attacker_name} hit {target_name}"));
        }
        Some(BananaResolution::Blocked) | None => {}
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BananaResolution {
    SpunOut,
    Blocked,
}

fn apply_network_banana_to_player(
    state: &mut HostState,
    target_id: PlayerId,
    now: Instant,
) -> Option<BananaResolution> {
    let target_index = state
        .race
        .players
        .iter()
        .position(|player| player.id == RacePlayerId(target_id.0))?;
    let target_name = state.race.players[target_index].name.clone();
    let word_index = state.race.players[target_index].state.word_index;

    if state.race.players[target_index]
        .state
        .has_active_shield(now)
    {
        state.race.players[target_index]
            .state
            .active_effects
            .retain(|effect| !matches!(effect, ActiveEffect::Shield { .. }));
        push_event(state, format!("{target_name} blocked Banana"));
        push_network_log(
            &state.debug_log,
            format!("{target_name} blocked Banana at word={word_index}; shield consumed"),
        );
        return Some(BananaResolution::Blocked);
    }

    state.bonus_attempts.remove(&target_id);
    let target = &mut state.race.players[target_index];
    target.state.input.clear();
    target.state.typo_index = None;
    let effects = state.player_effects.entry(target_id).or_default();
    effects.stunned_until = Some(now + BANANA_STUN);
    effects.impact_until = Some(now + ITEM_IMPACT_BLINK);
    push_event(state, format!("{target_name} spun out"));
    push_network_log(
        &state.debug_log,
        format!(
            "{target_name} spun out at word={word_index}; stun_ms={} impact_blink_ms={}",
            BANANA_STUN.as_millis(),
            ITEM_IMPACT_BLINK.as_millis()
        ),
    );
    Some(BananaResolution::SpunOut)
}

fn advance_network_mushrooms(state: &mut HostState, now: Instant) {
    let player_ids = state
        .race
        .players
        .iter()
        .map(|player| PlayerId(player.id.0))
        .collect::<Vec<_>>();

    for player_id in player_ids {
        loop {
            if !advance_network_mushroom_one_word(state, player_id, now) {
                break;
            }
            if state
                .race
                .player(RacePlayerId(player_id.0))
                .is_some_and(|player| player.state.is_finished())
            {
                break;
            }
        }
    }
}

fn advance_network_mushroom_one_word(
    state: &mut HostState,
    player_id: PlayerId,
    now: Instant,
) -> bool {
    let Some(player_index) = state
        .race
        .players
        .iter()
        .position(|player| player.id == RacePlayerId(player_id.0))
    else {
        return false;
    };
    let Some(effect_index) = state.race.players[player_index]
        .state
        .active_effects
        .iter()
        .position(|effect| {
            matches!(
                effect,
                ActiveEffect::Mushroom {
                    remaining_words,
                    next_step_at,
                    ..
                } if *remaining_words > 0 && *next_step_at <= now
            )
        })
    else {
        return false;
    };

    let remaining = state
        .race
        .track
        .len()
        .saturating_sub(state.race.players[player_index].state.word_index);
    if remaining == 0 {
        state.race.players[player_index]
            .state
            .active_effects
            .remove(effect_index);
        return false;
    }

    let player = &mut state.race.players[player_index];
    player.state.word_index += 1;
    player.state.stats.completed_words += 1;
    player.state.input.clear();
    player.state.typo_index = None;
    state.bonus_attempts.remove(&player_id);

    if player.state.word_index >= state.race.track.len() {
        player.state.finished_at = Some(now);
        player.state.active_effects.remove(effect_index);
        return false;
    }

    if let Some(ActiveEffect::Mushroom {
        remaining_words,
        next_step_at,
        step_interval,
    }) = player.state.active_effects.get_mut(effect_index)
    {
        *remaining_words -= 1;
        if *remaining_words == 0 {
            player.state.active_effects.remove(effect_index);
        } else {
            *next_step_at += *step_interval;
        }
    }

    true
}

fn player_has_active_mushroom_effect(
    player: &crate::game::race::RacePlayer,
    _now: Instant,
) -> bool {
    player.state.active_effects.iter().any(|effect| {
        matches!(
            effect,
            ActiveEffect::Mushroom {
                remaining_words,
                ..
            } if *remaining_words > 0
        )
    })
}

fn player_is_stunned(state: &HostState, player_id: PlayerId, now: Instant) -> bool {
    state
        .player_effects
        .get(&player_id)
        .and_then(|effects| effects.stunned_until)
        .is_some_and(|until| until > now)
}

fn attack_direction(
    attacker_word_index: usize,
    target_word_index: usize,
) -> AttackDirectionSnapshot {
    match target_word_index.cmp(&attacker_word_index) {
        std::cmp::Ordering::Greater => AttackDirectionSnapshot::Ahead,
        std::cmp::Ordering::Less => AttackDirectionSnapshot::Behind,
        std::cmp::Ordering::Equal => AttackDirectionSnapshot::Overlap,
    }
}

fn mushroom_step_interval() -> Duration {
    Duration::from_secs_f64(60.0 / MUSHROOM_WPM)
}

fn network_racer_positions_summary(
    state: &HostState,
    racers: &[RacerPosition],
    now: Instant,
) -> String {
    if racers.is_empty() {
        return "none".to_string();
    }

    racers
        .iter()
        .map(|racer| {
            let player_id = PlayerId(racer.id as u64);
            let shield = state
                .race
                .player(RacePlayerId(player_id.0))
                .is_some_and(|player| player.state.has_active_shield(now));
            let stunned = player_is_stunned(state, player_id, now);
            let finished = state
                .race
                .player(RacePlayerId(player_id.0))
                .is_some_and(|player| player.state.is_finished());
            format!(
                "{}@{} shield={shield} stunned={stunned} finished={finished}",
                player_name(state, player_id).unwrap_or_else(|| format!("player {}", player_id.0)),
                racer.word_index
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn item_pickup_name(item: ItemPickup) -> &'static str {
    match item {
        ItemPickup::Held(held_item) => held_item.name(),
        ItemPickup::Shield => "Shield",
    }
}

fn push_event_for_player(state: &mut HostState, player_id: PlayerId, message: &str) {
    let name = player_name(state, player_id).unwrap_or_else(|| format!("player {}", player_id.0));
    push_event(state, format!("{name}: {message}"));
}

fn run_countdown(state: Arc<Mutex<HostState>>) {
    for remaining_seconds in [2, 1] {
        thread::sleep(Duration::from_secs(1));
        let mut guard = state.lock().expect("host state poisoned");
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
    guard.phase = NetworkRacePhase::Racing;
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
    thread::spawn(move || loop {
        thread::sleep(RACE_SNAPSHOT_INTERVAL);
        let mut state = state.lock().expect("host state poisoned");
        if state.phase != NetworkRacePhase::Racing {
            break;
        }

        let now = Instant::now();
        update_race_status(&mut state, now);
        advance_network_mushrooms(&mut state, now);
        let expired_choices = expire_bonus_cooldowns(&mut state, now);
        if expired_choices > 0 {
            push_network_log(
                &state.debug_log,
                format!("bonus refreshed choices={expired_choices}"),
            );
        }

        if let Err(error) = broadcast_race_snapshot(&mut state) {
            server_eprintln!("Failed to broadcast race snapshot: {error:#}");
        }

        if state.phase == NetworkRacePhase::Finished {
            server_println!("Race finished");
            push_network_log(&state.debug_log, "race finished on snapshot tick");
            if let Err(error) = broadcast_race_results_once(&mut state) {
                server_eprintln!("Failed to broadcast race results: {error:#}");
            }
            break;
        }
    });
}

fn all_connected_players_ready(players: &[LobbyPlayer]) -> bool {
    players
        .iter()
        .filter(|player| player.connected)
        .all(|player| player.ready)
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
    let expired = state.bonuses.expire_cooldowns(track, now);
    if expired > 0 {
        push_event(state, "Bonus refreshed".to_string());
    }
    expired
}

fn broadcast_race_snapshot(state: &mut HostState) -> Result<()> {
    let snapshot = ServerMessage::RaceSnapshot(build_race_snapshot(state));
    log_race_snapshot(state);

    let mut failed_clients = Vec::new();
    for client in state.clients.iter_mut() {
        if let Err(error) = write_server_message(&mut client.stream, &snapshot) {
            server_eprintln!(
                "Failed to send race snapshot to player {}: {error:#}",
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

fn log_race_snapshot(state: &HostState) {
    match state.phase {
        NetworkRacePhase::Countdown { remaining_seconds } => push_network_log(
            &state.debug_log,
            format!(
                "broadcast snapshot seq={} phase=countdown remaining={remaining_seconds}",
                state.snapshot_sequence
            ),
        ),
        NetworkRacePhase::Racing if state.snapshot_sequence % 20 == 0 => push_network_log(
            &state.debug_log,
            format!(
                "broadcast snapshot seq={} phase=racing",
                state.snapshot_sequence
            ),
        ),
        NetworkRacePhase::Finished => push_network_log(
            &state.debug_log,
            format!(
                "broadcast snapshot seq={} phase=finished",
                state.snapshot_sequence
            ),
        ),
        _ => {}
    }
}

fn broadcast_race_results(state: &mut HostState) -> Result<()> {
    let results = ServerMessage::RaceResults {
        placements: state.placements.clone(),
    };
    push_network_log(
        &state.debug_log,
        format!("broadcast race results placements={:?}", state.placements),
    );

    let mut failed_clients = Vec::new();
    for client in state.clients.iter_mut() {
        if let Err(error) = write_server_message(&mut client.stream, &results) {
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

    let finished_ids = state
        .race
        .players
        .iter()
        .filter(|player| player.connected && player.state.is_finished())
        .map(|player| PlayerId(player.id.0))
        .collect::<Vec<_>>();

    for id in finished_ids {
        if !state.placements.contains(&id) {
            state.placements.push(id);
            let name = player_name(state, id).unwrap_or_else(|| format!("player {}", id.0));
            push_event(
                state,
                format!("{}. {name} finished", state.placements.len()),
            );
            push_network_log(
                &state.debug_log,
                format!("{}. {name} finished", state.placements.len()),
            );
        }
    }

    if state.first_finished_at.is_none() && !state.placements.is_empty() {
        state.first_finished_at = Some(now);
    }

    let connected_racers = state
        .race
        .players
        .iter()
        .filter(|player| player.connected)
        .count();
    let connected_finished = state
        .race
        .players
        .iter()
        .filter(|player| player.connected && player.state.is_finished())
        .count();
    let all_connected_finished = connected_racers > 0 && connected_finished == connected_racers;
    let timeout_expired = state.first_finished_at.is_some_and(|first_finished_at| {
        now.duration_since(first_finished_at) >= POST_FIRST_FINISH_TIMEOUT
    });

    if all_connected_finished || timeout_expired {
        append_unfinished_connected_placements(state);
        state.phase = NetworkRacePhase::Finished;
        push_event(state, "Race finished".to_string());
        push_network_log(
            &state.debug_log,
            format!(
                "race finished all_connected_finished={all_connected_finished} timeout_expired={timeout_expired}"
            ),
        );
    }
}

fn append_unfinished_connected_placements(state: &mut HostState) {
    let mut remaining = state
        .race
        .players
        .iter()
        .filter(|player| player.connected)
        .map(|player| {
            (
                PlayerId(player.id.0),
                player.state.word_index,
                player.state.input.chars().count(),
            )
        })
        .filter(|(id, _, _)| !state.placements.contains(id))
        .collect::<Vec<_>>();

    remaining.sort_by_key(|(_, word_index, input_len)| {
        (
            std::cmp::Reverse(*word_index),
            std::cmp::Reverse(*input_len),
        )
    });

    state
        .placements
        .extend(remaining.into_iter().map(|(id, _, _)| id));
}

fn player_name(state: &HostState, id: PlayerId) -> Option<String> {
    state
        .race
        .players
        .iter()
        .find(|player| player.id == RacePlayerId(id.0))
        .map(|player| player.name.clone())
}

fn build_race_snapshot(state: &mut HostState) -> RaceSnapshot {
    let now = Instant::now();
    expire_bonus_cooldowns(state, now);

    state.snapshot_sequence += 1;
    RaceSnapshot {
        sequence: state.snapshot_sequence,
        phase: state.phase,
        mod_config: (&state.active_mod_config).into(),
        track_words: state.race.track.words.clone(),
        bonuses: build_bonus_snapshots(&state.bonuses, now),
        players: state
            .race
            .players
            .iter()
            .map(|player| {
                let player_id = PlayerId(player.id.0);
                let effects = state
                    .player_effects
                    .get(&player_id)
                    .copied()
                    .unwrap_or_default();
                PlayerSnapshot {
                    id: player_id,
                    name: player.name.clone(),
                    color: player.color.into(),
                    word_index: player.state.word_index,
                    input: player.state.input.clone(),
                    typo_index: player.state.typo_index,
                    finished: player.state.is_finished(),
                    connected: player.connected,
                    shielded: player.state.has_active_shield(now),
                    boosted: player_has_active_mushroom_effect(player, now),
                    stunned: effects.stunned_until.is_some_and(|until| until > now),
                    impact_remaining_ms: remaining_ms(effects.impact_until, now),
                    item_cue: build_item_cue_snapshot(effects.item_cue, now),
                }
            })
            .collect(),
        events: state.events.clone(),
    }
}

fn remaining_ms(until: Option<Instant>, now: Instant) -> u64 {
    until
        .filter(|until| *until > now)
        .map(|until| until.saturating_duration_since(now).as_millis() as u64)
        .unwrap_or(0)
}

fn build_item_cue_snapshot(cue: Option<NetworkItemCue>, now: Instant) -> Option<ItemCueSnapshot> {
    let cue = cue.filter(|cue| cue.until > now)?;
    Some(ItemCueSnapshot {
        kind: match cue.kind {
            NetworkItemCueKind::Banana { direction } => ItemCueSnapshotKind::Banana { direction },
        },
        remaining_ms: cue.until.saturating_duration_since(now).as_millis() as u64,
    })
}

fn build_bonus_snapshots(bonuses: &BonusState, now: Instant) -> Vec<BonusPointSnapshot> {
    bonuses
        .points
        .iter()
        .map(|point| BonusPointSnapshot {
            after_word_index: point.after_word_index,
            choices: point
                .choices
                .iter()
                .map(|choice| BonusChoiceSnapshot {
                    word: choice.word.clone(),
                    status: match choice.status {
                        BonusChoiceStatus::Available => BonusChoiceSnapshotStatus::Available,
                        BonusChoiceStatus::Cooldown { until } if until <= now => {
                            BonusChoiceSnapshotStatus::Available
                        }
                        BonusChoiceStatus::Cooldown { until } => {
                            BonusChoiceSnapshotStatus::Cooldown {
                                remaining_ms: until.saturating_duration_since(now).as_millis()
                                    as u64,
                            }
                        }
                    },
                })
                .collect(),
        })
        .collect()
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
    let encoded = encode_server_message(message).context("failed to encode server message")?;
    writeln!(stream, "{encoded}").context("failed to write server message")?;
    stream.flush().context("failed to flush server message")
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
        thread,
        time::Duration,
    };

    use super::{
        activate_network_pickup, all_connected_players_ready, apply_network_banana_to_player,
        apply_network_key_input, broadcast_lobby_snapshot, broadcast_race_results_once,
        build_race_snapshot, connected_player_count, first_available_color, handle_client_messages,
        push_event, read_join_hello, update_host_ready, update_race_status, welcome_joiner,
        AssignedColor, ConnectedClient, HostState, NetworkRacePhase, PlayerId,
        POST_FIRST_FINISH_TIMEOUT,
    };
    use crate::game::{
        bonus::{BonusChoice, BonusChoiceStatus, BonusPoint, BonusState},
        effects::ActiveEffect,
        items::{HeldItem, ItemPickup, ItemRegistry},
        mods::{ActiveModConfig, ContentMetadata},
        race::{PlayerColorId, RacePlayerId, RaceState},
        track::{Track, WordList},
        typing::KeyAction,
        words::WordSetDefinition,
    };
    use crate::net::protocol::{
        decode_server_message, encode_client_message, ClientMessage, LobbyPlayer, ServerMessage,
    };

    #[test]
    fn host_handshake_accepts_hello() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();

        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let name = read_join_hello(&stream).unwrap();
            welcome_joiner(&stream, PlayerId(2), AssignedColor::Red).unwrap();
            LobbyPlayer {
                id: PlayerId(2),
                name,
                color: AssignedColor::Red,
                ready: false,
                connected: true,
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
                bonuses: test_bonus_state(),
                item_registry: ItemRegistry::builtin(),
                active_mod_config: test_active_mod_config(),
                bonus_attempts: HashMap::new(),
                spent_bonus_gaps: HashMap::new(),
                player_effects: HashMap::new(),
                phase: NetworkRacePhase::WaitingForHost,
                snapshot_sequence: 0,
                events: Vec::new(),
                placements: Vec::new(),
                first_finished_at: None,
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
                bonuses: test_bonus_state(),
                item_registry: ItemRegistry::builtin(),
                active_mod_config: test_active_mod_config(),
                bonus_attempts: HashMap::new(),
                spent_bonus_gaps: HashMap::new(),
                player_effects: HashMap::new(),
                phase: NetworkRacePhase::WaitingForHost,
                snapshot_sequence: 0,
                events: Vec::new(),
                placements: Vec::new(),
                first_finished_at: None,
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

        assert!(state
            .players
            .iter()
            .any(|player| { player.id == PlayerId(2) && !player.ready && !player.connected }));
        assert!(matches!(
            decode_server_message(snapshot_line.trim_end()).unwrap(),
            ServerMessage::LobbySnapshot { ref players, .. }
                if players.iter().any(|player| player.id == PlayerId(2) && player.ready)
        ));
    }

    #[test]
    fn disconnected_players_do_not_count_against_capacity_or_color_assignment() {
        let players = vec![
            LobbyPlayer {
                id: PlayerId(1),
                name: "host".to_string(),
                color: AssignedColor::Cyan,
                ready: false,
                connected: true,
            },
            LobbyPlayer {
                id: PlayerId(2),
                name: "alex".to_string(),
                color: AssignedColor::Red,
                ready: false,
                connected: false,
            },
        ];

        assert_eq!(connected_player_count(&players), 1);
        assert_eq!(first_available_color(&players), AssignedColor::Red);
    }

    #[test]
    fn host_ready_command_updates_host_player() {
        let state = Arc::new(Mutex::new(HostState {
            clients: Vec::new(),
            players: vec![LobbyPlayer {
                id: PlayerId(1),
                name: "host".to_string(),
                color: AssignedColor::Cyan,
                ready: false,
                connected: true,
            }],
            race: test_race_state(),
            bonuses: test_bonus_state(),
            item_registry: ItemRegistry::builtin(),
            active_mod_config: test_active_mod_config(),
            bonus_attempts: HashMap::new(),
            spent_bonus_gaps: HashMap::new(),
            player_effects: HashMap::new(),
            phase: NetworkRacePhase::WaitingForHost,
            snapshot_sequence: 0,
            events: Vec::new(),
            placements: Vec::new(),
            first_finished_at: None,
            debug_log: None,
            race_results_sent: false,
        }));

        update_host_ready(&state, true);

        let state = state.lock().unwrap();
        assert!(state.players[0].ready);
    }

    #[test]
    fn connected_players_must_all_be_ready_before_countdown() {
        let mut players = test_players(false);
        players[0].ready = true;

        assert!(!all_connected_players_ready(&players));

        players[1].connected = false;
        assert!(all_connected_players_ready(&players));
    }

    #[test]
    fn race_snapshot_includes_phase_players_and_recent_events() {
        let mut state = HostState {
            clients: Vec::new(),
            players: test_players(true),
            race: test_race_state(),
            bonuses: test_bonus_state(),
            item_registry: ItemRegistry::builtin(),
            active_mod_config: test_active_mod_config(),
            bonus_attempts: HashMap::new(),
            spent_bonus_gaps: HashMap::new(),
            player_effects: HashMap::new(),
            phase: NetworkRacePhase::Countdown {
                remaining_seconds: 3,
            },
            snapshot_sequence: 0,
            events: Vec::new(),
            placements: Vec::new(),
            first_finished_at: None,
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
            bonuses: test_bonus_state(),
            item_registry: ItemRegistry::builtin(),
            active_mod_config: test_active_mod_config(),
            bonus_attempts: HashMap::new(),
            spent_bonus_gaps: HashMap::new(),
            player_effects: HashMap::new(),
            phase: NetworkRacePhase::Racing,
            snapshot_sequence: 0,
            events: Vec::new(),
            placements: Vec::new(),
            first_finished_at: None,
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
    fn network_key_input_can_start_bonus_attempt_at_gap() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.race.players[1].state.word_index = 1;

        apply_network_key_input(&mut state, PlayerId(2), KeyAction::Char('d'), now);

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(alex.state.word_index, 1);
        assert_eq!(alex.state.input, "d");
        assert_eq!(alex.state.typo_index, None);
        assert!(state.bonus_attempts.contains_key(&PlayerId(2)));
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
        assert!(!state.bonus_attempts.contains_key(&PlayerId(2)));
        assert_eq!(state.spent_bonus_gaps.get(&PlayerId(2)), Some(&0));
        assert!(matches!(
            state.bonuses.points[0].choices[0].status,
            BonusChoiceStatus::Cooldown { .. }
        ));
        assert!(state
            .events
            .iter()
            .any(|event| event.starts_with("alex picked up ")));
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
        assert_eq!(state.spent_bonus_gaps.get(&PlayerId(2)), Some(&0));

        apply_network_key_input(&mut state, PlayerId(2), KeyAction::Char('s'), now);

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(alex.state.input, "s");
        assert_eq!(alex.state.typo_index, Some(0));
        assert!(!state.bonus_attempts.contains_key(&PlayerId(2)));
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
    fn network_banana_stuns_nearest_target_and_clears_input() {
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
        assert!(state
            .player_effects
            .get(&PlayerId(2))
            .and_then(|effects| effects.stunned_until)
            .is_some_and(|until| until > now));
        assert!(state
            .player_effects
            .get(&PlayerId(1))
            .and_then(|effects| effects.item_cue)
            .is_some());
    }

    #[test]
    fn network_shield_blocks_banana_and_is_consumed() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.race.players[0].state.word_index = 1;
        state.race.players[1].state.word_index = 0;

        activate_network_pickup(&mut state, PlayerId(2), ItemPickup::Shield, now);
        let result = apply_network_banana_to_player(&mut state, PlayerId(2), now);

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(result, Some(super::BananaResolution::Blocked));
        assert!(!alex.state.has_active_shield(now));
        assert!(!state.player_effects.contains_key(&PlayerId(2)));
    }

    #[test]
    fn race_status_records_finish_order_and_finishes_when_all_connected_finish() {
        let now = std::time::Instant::now();
        let mut state = HostState {
            clients: Vec::new(),
            players: test_players(true),
            race: test_race_state(),
            bonuses: test_bonus_state(),
            item_registry: ItemRegistry::builtin(),
            active_mod_config: test_active_mod_config(),
            bonus_attempts: HashMap::new(),
            spent_bonus_gaps: HashMap::new(),
            player_effects: HashMap::new(),
            phase: NetworkRacePhase::Racing,
            snapshot_sequence: 0,
            events: Vec::new(),
            placements: Vec::new(),
            first_finished_at: None,
            debug_log: None,
            race_results_sent: false,
        };

        finish_player(&mut state, RacePlayerId(2), now);
        update_race_status(&mut state, now);

        assert_eq!(state.phase, NetworkRacePhase::Racing);
        assert_eq!(state.placements, vec![PlayerId(2)]);

        finish_player(&mut state, RacePlayerId(1), now);
        update_race_status(&mut state, now);

        assert_eq!(state.phase, NetworkRacePhase::Finished);
        assert_eq!(state.placements, vec![PlayerId(2), PlayerId(1)]);
        assert!(state.events.iter().any(|event| event == "Race finished"));
    }

    #[test]
    fn race_status_timeout_places_unfinished_connected_racers_by_progress() {
        let now = std::time::Instant::now();
        let mut state = HostState {
            clients: Vec::new(),
            players: test_players(true),
            race: test_race_state(),
            bonuses: test_bonus_state(),
            item_registry: ItemRegistry::builtin(),
            active_mod_config: test_active_mod_config(),
            bonus_attempts: HashMap::new(),
            spent_bonus_gaps: HashMap::new(),
            player_effects: HashMap::new(),
            phase: NetworkRacePhase::Racing,
            snapshot_sequence: 0,
            events: Vec::new(),
            placements: Vec::new(),
            first_finished_at: None,
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
        assert_eq!(state.placements, vec![PlayerId(2), PlayerId(1)]);
    }

    #[test]
    fn race_results_are_broadcast_only_once() {
        let mut state = HostState {
            clients: Vec::new(),
            players: test_players(true),
            race: test_race_state(),
            bonuses: test_bonus_state(),
            item_registry: ItemRegistry::builtin(),
            active_mod_config: test_active_mod_config(),
            bonus_attempts: HashMap::new(),
            spent_bonus_gaps: HashMap::new(),
            player_effects: HashMap::new(),
            phase: NetworkRacePhase::Finished,
            snapshot_sequence: 0,
            events: Vec::new(),
            placements: vec![PlayerId(2), PlayerId(1)],
            first_finished_at: None,
            debug_log: None,
            race_results_sent: false,
        };

        broadcast_race_results_once(&mut state).unwrap();
        assert!(state.race_results_sent);

        broadcast_race_results_once(&mut state).unwrap();
        assert!(state.race_results_sent);
    }

    fn send_hello(client: &mut std::net::TcpStream) {
        let hello = encode_client_message(&ClientMessage::Hello {
            name: "alex".to_string(),
            client_version: env!("CARGO_PKG_VERSION").to_string(),
        })
        .unwrap();
        writeln!(client, "{hello}").unwrap();
    }

    fn test_players(joiner_ready: bool) -> Vec<LobbyPlayer> {
        vec![
            LobbyPlayer {
                id: PlayerId(1),
                name: "host".to_string(),
                color: AssignedColor::Cyan,
                ready: false,
                connected: true,
            },
            LobbyPlayer {
                id: PlayerId(2),
                name: "alex".to_string(),
                color: AssignedColor::Red,
                ready: joiner_ready,
                connected: true,
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

    fn test_active_mod_config() -> ActiveModConfig {
        let item_registry = ItemRegistry::builtin();
        ActiveModConfig::new(
            &WordSetDefinition {
                metadata: ContentMetadata::built_in("classic", "Classic"),
                words: WordList {
                    words: vec!["one".to_string(), "two".to_string()],
                },
            },
            &item_registry,
            None,
        )
    }

    fn test_host_state(phase: NetworkRacePhase) -> HostState {
        HostState {
            clients: Vec::new(),
            players: test_players(true),
            race: test_race_state(),
            bonuses: test_bonus_state(),
            item_registry: ItemRegistry::builtin(),
            active_mod_config: test_active_mod_config(),
            bonus_attempts: HashMap::new(),
            spent_bonus_gaps: HashMap::new(),
            player_effects: HashMap::new(),
            phase,
            snapshot_sequence: 0,
            events: Vec::new(),
            placements: Vec::new(),
            first_finished_at: None,
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
