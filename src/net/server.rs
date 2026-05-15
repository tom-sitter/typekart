//! Minimal TCP host for Milestone 4.
//!
//! The host currently supports a persistent lobby connection. Joiners can stay
//! connected, receive lobby snapshots, and send simple readiness updates. Race
//! snapshots and key input will be layered on this socket structure next.

use std::{
    io::{self, BufRead, BufReader, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};

use crate::game::{
    race::{PlayerColorId, RacePlayerId, RaceState},
    track::Track,
    typing::KeyAction,
};

use super::protocol::{
    AssignedColor, ClientMessage, LobbyPlayer, NetworkRacePhase, PlayerId, PlayerSnapshot,
    ProtocolKey, RaceSnapshot, ServerMessage, decode_client_message, encode_server_message,
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
const RACE_STATUS_POLL_INTERVAL: Duration = Duration::from_millis(250);
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
    pub max_players: usize,
    pub ready_signal: Option<Sender<SocketAddr>>,
    pub console_logging: bool,
}

struct ConnectedClient {
    player_id: PlayerId,
    stream: TcpStream,
}

struct HostState {
    players: Vec<LobbyPlayer>,
    clients: Vec<ConnectedClient>,
    race: RaceState,
    phase: NetworkRacePhase,
    snapshot_sequence: u64,
    events: Vec<String>,
    placements: Vec<PlayerId>,
    first_finished_at: Option<Instant>,
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
        phase: NetworkRacePhase::WaitingForHost,
        snapshot_sequence: 0,
        events: Vec::new(),
        placements: Vec::new(),
        first_finished_at: None,
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
                    player.ready = ready;
                    server_println!(
                        "{} is {}",
                        player.name,
                        if ready { "ready" } else { "not ready" }
                    );
                }
                print_lobby_snapshot(&state.players);
                if let Err(error) = broadcast_lobby_snapshot(&mut state) {
                    server_eprintln!("Failed to broadcast lobby snapshot: {error:#}");
                }
            }
            ClientMessage::StartCountdown => {
                if player_id == PlayerId(1) {
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
                if state
                    .race
                    .apply_key_input(RacePlayerId(player_id.0), action, now)
                    .is_some()
                {
                    update_race_status(&mut state, now);
                    if let Err(error) = broadcast_race_snapshot(&mut state) {
                        server_eprintln!("Failed to broadcast race snapshot: {error:#}");
                    }
                    if state.phase == NetworkRacePhase::Finished {
                        server_println!("Race finished");
                        if let Err(error) = broadcast_race_results(&mut state) {
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
        player.connected = false;
        player.ready = false;
        server_println!("{} disconnected", player.name);
    }
    if let Some(player) = state
        .race
        .players
        .iter_mut()
        .find(|player| player.id == RacePlayerId(player_id.0))
    {
        player.connected = false;
    }
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
        if let Err(error) = broadcast_race_results(&mut state) {
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
        state
            .race
            .apply_key_input(RacePlayerId(player_id.0), action, now);
    }
    state
        .race
        .apply_key_input(RacePlayerId(player_id.0), KeyAction::Space, now);

    update_race_status(&mut state, now);
    if let Err(error) = broadcast_race_snapshot(&mut state) {
        server_eprintln!("Failed to broadcast race snapshot: {error:#}");
    }
    if state.phase == NetworkRacePhase::Finished {
        server_println!("Race finished");
        if let Err(error) = broadcast_race_results(&mut state) {
            server_eprintln!("Failed to broadcast race results: {error:#}");
        }
    }
}

fn run_countdown(state: Arc<Mutex<HostState>>) {
    for remaining_seconds in [2, 1] {
        thread::sleep(Duration::from_secs(1));
        let mut guard = state.lock().expect("host state poisoned");
        guard.phase = NetworkRacePhase::Countdown { remaining_seconds };
        server_println!("Countdown: {remaining_seconds}");
        if let Err(error) = broadcast_race_snapshot(&mut guard) {
            server_eprintln!("Failed to broadcast race snapshot: {error:#}");
        }
    }

    thread::sleep(Duration::from_secs(1));
    let mut guard = state.lock().expect("host state poisoned");
    guard.phase = NetworkRacePhase::Racing;
    push_event(&mut guard, "Race started".to_string());
    server_println!("Race started");
    if let Err(error) = broadcast_race_snapshot(&mut guard) {
        server_eprintln!("Failed to broadcast race snapshot: {error:#}");
    }
    drop(guard);
    spawn_race_status_loop(state);
}

fn spawn_race_status_loop(state: Arc<Mutex<HostState>>) {
    thread::spawn(move || {
        loop {
            thread::sleep(RACE_STATUS_POLL_INTERVAL);
            let mut state = state.lock().expect("host state poisoned");
            if state.phase != NetworkRacePhase::Racing {
                break;
            }

            update_race_status(&mut state, Instant::now());
            if state.phase != NetworkRacePhase::Finished {
                continue;
            }

            server_println!("Race finished");
            if let Err(error) = broadcast_race_snapshot(&mut state) {
                server_eprintln!("Failed to broadcast race snapshot: {error:#}");
            }
            if let Err(error) = broadcast_race_results(&mut state) {
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

fn broadcast_race_snapshot(state: &mut HostState) -> Result<()> {
    let snapshot = ServerMessage::RaceSnapshot(build_race_snapshot(state));

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

fn broadcast_race_results(state: &mut HostState) -> Result<()> {
    let results = ServerMessage::RaceResults {
        placements: state.placements.clone(),
    };

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
    state.snapshot_sequence += 1;
    RaceSnapshot {
        sequence: state.snapshot_sequence,
        phase: state.phase,
        track_words: state.race.track.words.clone(),
        players: state
            .race
            .players
            .iter()
            .map(|player| PlayerSnapshot {
                id: PlayerId(player.id.0),
                name: player.name.clone(),
                color: player.color.into(),
                word_index: player.state.word_index,
                input: player.state.input.clone(),
                typo_index: player.state.typo_index,
                finished: player.state.is_finished(),
                connected: player.connected,
            })
            .collect(),
        events: state.events.clone(),
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
    let encoded = encode_server_message(message).context("failed to encode server message")?;
    writeln!(stream, "{encoded}").context("failed to write server message")?;
    stream.flush().context("failed to flush server message")
}

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
        thread,
    };

    use super::{
        AssignedColor, ConnectedClient, HostState, NetworkRacePhase, POST_FIRST_FINISH_TIMEOUT,
        PlayerId, all_connected_players_ready, broadcast_lobby_snapshot, build_race_snapshot,
        connected_player_count, first_available_color, handle_client_messages, push_event,
        read_join_hello, update_host_ready, update_race_status, welcome_joiner,
    };
    use crate::game::{
        race::{PlayerColorId, RacePlayerId, RaceState},
        track::Track,
        typing::KeyAction,
    };
    use crate::net::protocol::{
        ClientMessage, LobbyPlayer, ServerMessage, decode_server_message, encode_client_message,
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
                phase: NetworkRacePhase::WaitingForHost,
                snapshot_sequence: 0,
                events: Vec::new(),
                placements: Vec::new(),
                first_finished_at: None,
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
                phase: NetworkRacePhase::WaitingForHost,
                snapshot_sequence: 0,
                events: Vec::new(),
                placements: Vec::new(),
                first_finished_at: None,
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

        assert!(
            state
                .players
                .iter()
                .any(|player| { player.id == PlayerId(2) && !player.ready && !player.connected })
        );
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
            phase: NetworkRacePhase::WaitingForHost,
            snapshot_sequence: 0,
            events: Vec::new(),
            placements: Vec::new(),
            first_finished_at: None,
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
            phase: NetworkRacePhase::Countdown {
                remaining_seconds: 3,
            },
            snapshot_sequence: 0,
            events: Vec::new(),
            placements: Vec::new(),
            first_finished_at: None,
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
        assert_eq!(snapshot.events, vec!["Countdown started"]);
    }

    #[test]
    fn race_snapshot_reflects_applied_key_input() {
        let mut state = HostState {
            clients: Vec::new(),
            players: test_players(true),
            race: test_race_state(),
            phase: NetworkRacePhase::Racing,
            snapshot_sequence: 0,
            events: Vec::new(),
            placements: Vec::new(),
            first_finished_at: None,
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
    fn race_status_records_finish_order_and_finishes_when_all_connected_finish() {
        let now = std::time::Instant::now();
        let mut state = HostState {
            clients: Vec::new(),
            players: test_players(true),
            race: test_race_state(),
            phase: NetworkRacePhase::Racing,
            snapshot_sequence: 0,
            events: Vec::new(),
            placements: Vec::new(),
            first_finished_at: None,
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
            phase: NetworkRacePhase::Racing,
            snapshot_sequence: 0,
            events: Vec::new(),
            placements: Vec::new(),
            first_finished_at: None,
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
