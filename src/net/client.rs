//! TCP join client and terminal UI for multiplayer races.

use std::{
    collections::VecDeque,
    io::{self, BufReader},
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use super::log::{NetworkLog, SharedNetworkLog, push_network_log, write_network_log};
use super::protocol::{
    AiDifficultySnapshot, AssignedColor, ClientMessage, ClientSequence, ImpactCueSnapshotKind,
    ItemCuePlacementSnapshot, LobbyPlayer, ModConfigSnapshot, NetworkRacePhase, PlayerId,
    PlayerKind, PlayerSnapshot, ProtocolKey, RaceDeltaSnapshot, RaceResultRow, RaceResultStatus,
    RaceSnapshot, ServerMessage,
};
use super::relay::RoomCode;
use super::transport::{read_server_message, write_client_message};
use crate::ui::render::IconMode;

const NETWORK_RACER_MARKER: &str = "███";
const NETWORK_FINISHED_EDGE_MARKER: &str = ">!";
const NETWORK_BOOST_PREFIX: &str = ">>>";

#[derive(Debug, Clone)]
pub struct JoinConfig {
    pub server: SocketAddr,
    pub name: String,
    pub icon_mode: IconMode,
    pub online_room: Option<OnlineRoomInfo>,
    pub debug_log: Option<PathBuf>,
    pub shared_log: Option<SharedNetworkLog>,
}

#[derive(Debug, Clone)]
pub struct OnlineRoomInfo {
    pub relay: String,
    pub room: RoomCode,
}

pub fn run_join(config: JoinConfig) -> Result<()> {
    let log = config.shared_log.clone().or_else(|| {
        config
            .debug_log
            .as_ref()
            .map(|_| NetworkLog::shared(std::time::Instant::now(), 2_000))
    });
    push_network_log(
        &log,
        format!(
            "client connecting server={} name={}",
            config.server, config.name
        ),
    );
    let mut stream = std::net::TcpStream::connect(config.server)
        .with_context(|| format!("failed to connect to {}", config.server))?;
    let hello = ClientMessage::Hello {
        name: config.name,
        client_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    write_client_message(&mut stream, &hello).context("failed to send hello message")?;
    push_network_log(&log, "client sent hello");

    let read_stream = stream
        .try_clone()
        .context("failed to clone server stream for reading")?;
    let mut reader = BufReader::new(read_stream);

    let player_id =
        match read_server_message(&mut reader).context("failed to read server response")? {
            Some(ServerMessage::Welcome {
                player_id,
                assigned_color,
            }) => {
                println!(
                    "Joined TypeKart server as player {} ({assigned_color:?})",
                    player_id.0
                );
                push_network_log(
                    &log,
                    format!(
                        "client welcomed player={} color={assigned_color:?}",
                        player_id.0
                    ),
                );
                player_id
            }
            Some(ServerMessage::Error { message }) => {
                println!("Could not join: {}", join_rejection_message(&message));
                push_network_log(&log, format!("client rejected: {message}"));
                if let (Some(path), Some(log)) = (config.debug_log.as_ref(), log.as_ref()) {
                    write_network_log(path, log)?;
                }
                return Ok(());
            }
            Some(other) => {
                println!("Unexpected server response: {other:?}");
                push_network_log(
                    &log,
                    format!("client unexpected welcome response: {other:?}"),
                );
                if let (Some(path), Some(log)) = (config.debug_log.as_ref(), log.as_ref()) {
                    write_network_log(path, log)?;
                }
                return Ok(());
            }
            None => {
                println!("Server closed connection before welcome");
                push_network_log(&log, "client disconnected before welcome");
                if let (Some(path), Some(log)) = (config.debug_log.as_ref(), log.as_ref()) {
                    write_network_log(path, log)?;
                }
                return Ok(());
            }
        };

    let phase = Arc::new(Mutex::new(NetworkRacePhase::WaitingForHost));
    let reader_phase = Arc::clone(&phase);
    let view_state = Arc::new(Mutex::new(NetworkViewState::new(
        player_id,
        config.icon_mode,
        config.online_room,
    )));
    let reader_view_state = Arc::clone(&view_state);
    let reader_log = log.clone();

    thread::spawn(move || {
        loop {
            match read_server_message(&mut reader) {
                Ok(Some(ServerMessage::LobbySnapshot {
                    players,
                    host_id,
                    mod_config,
                    events,
                })) => {
                    push_network_log(
                        &reader_log,
                        format!(
                            "client received lobby players={} host={}",
                            players.len(),
                            host_id.0
                        ),
                    );
                    let mut state = reader_view_state.lock().expect("client view poisoned");
                    state.lobby_players = players;
                    if state.selected_lobby_index >= state.lobby_players.len() {
                        state.selected_lobby_index = state.lobby_players.len().saturating_sub(1);
                    }
                    state.host_id = Some(host_id);
                    state.mod_config = Some(mod_config);
                    state.lobby_events = events;
                }
                Ok(Some(ServerMessage::RaceEvent { message })) => {
                    push_network_log(&reader_log, format!("client received event: {message}"));
                    reader_view_state
                        .lock()
                        .expect("client view poisoned")
                        .push_message(message);
                }
                Ok(Some(ServerMessage::RaceSnapshot(snapshot))) => {
                    log_client_snapshot(&reader_log, &snapshot);
                    *reader_phase.lock().expect("client phase poisoned") = snapshot.phase;
                    let mut state = reader_view_state.lock().expect("client view poisoned");
                    state.apply_race_snapshot(snapshot);
                }
                Ok(Some(ServerMessage::RaceDelta(delta))) => {
                    log_client_delta(&reader_log, &delta);
                    *reader_phase.lock().expect("client phase poisoned") = delta.phase;
                    let mut state = reader_view_state.lock().expect("client view poisoned");
                    state.apply_race_delta(delta);
                }
                Ok(Some(ServerMessage::Error { message })) => {
                    push_network_log(&reader_log, format!("client received error: {message}"));
                    let terminal_error = is_terminal_server_error(&message);
                    let mut state = reader_view_state.lock().expect("client view poisoned");
                    state.push_message(server_error_display_message(&message));
                    if terminal_error {
                        state.disconnected = true;
                        break;
                    }
                }
                Ok(Some(ServerMessage::RaceResults { placements, rows })) => {
                    push_network_log(
                        &reader_log,
                        format!(
                            "client received results placements={placements:?} rows={}",
                            rows.len()
                        ),
                    );
                    let mut state = reader_view_state.lock().expect("client view poisoned");
                    state.placements = placements;
                    state.result_rows = rows;
                    state.push_message("Race results received".to_string());
                }
                Ok(Some(other)) => {
                    push_network_log(&reader_log, format!("client received other: {other:?}"));
                    reader_view_state
                        .lock()
                        .expect("client view poisoned")
                        .push_message(format!("Received: {other:?}"));
                }
                Ok(None) => break,
                Err(error) => {
                    push_network_log(&reader_log, format!("client decode error: {error}"));
                    reader_view_state
                        .lock()
                        .expect("client view poisoned")
                        .push_message(format!("Failed to decode server message: {error}"));
                }
            }
        }

        let mut state = reader_view_state.lock().expect("client view poisoned");
        if !state.disconnected {
            state.disconnected = true;
            state.push_message("Disconnected from server".to_string());
        }
        push_network_log(&reader_log, "client disconnected from server");
    });

    let mut terminal = NetworkTerminal::setup()?;
    let mut sequence = 1;
    let mut lobby_command = String::new();
    let mut rename_input: Option<String> = None;

    loop {
        let state = view_state.lock().expect("client view poisoned").clone();
        terminal.draw(&state, &lobby_command, rename_input.as_deref())?;

        if !event::poll(Duration::from_millis(50)).context("failed to poll terminal input")? {
            continue;
        }

        let Event::Key(key_event) = event::read().context("failed to read terminal input")? else {
            continue;
        };

        if key_event.kind != KeyEventKind::Press {
            continue;
        }

        if rename_input.is_some()
            && key_event.code == KeyCode::Char('c')
            && key_event.modifiers.contains(KeyModifiers::CONTROL)
        {
            send_client_message(&mut stream, &ClientMessage::Leave)?;
            push_network_log(&log, "client sent leave");
            break;
        }

        if let Some(input) = rename_input.as_mut() {
            if handle_rename_key(&mut stream, key_event, input, &log)? {
                rename_input = None;
            }
            continue;
        }

        if should_leave(key_event) {
            send_client_message(&mut stream, &ClientMessage::Leave)?;
            push_network_log(&log, "client sent leave");
            break;
        }

        if key_event.code == KeyCode::Char('?') {
            let mut state = view_state.lock().expect("client view poisoned");
            state.show_help = !state.show_help;
            drop(state);
            terminal.clear()?;
            continue;
        }

        if host_cancel_key(state.is_host(), state.current_phase(), key_event) {
            send_client_message(&mut stream, &ClientMessage::RestartRace)?;
            push_network_log(&log, "client sent host cancel race");
            lobby_command.clear();
            continue;
        }

        if starts_rename_mode(state.current_phase(), &lobby_command, key_event) {
            rename_input = Some(rename_prefill(state.local_player_name().as_deref()));
            continue;
        }

        if *phase.lock().expect("client phase poisoned") == NetworkRacePhase::Racing {
            let is_racer = view_state
                .lock()
                .expect("client view poisoned")
                .is_local_player_in_current_race();
            if is_racer && send_race_key(&mut stream, key_event, &mut sequence, &log)? {
                break;
            }
            continue;
        }

        if matches!(key_event.code, KeyCode::Up | KeyCode::Down) {
            let mut state = view_state.lock().expect("client view poisoned");
            match key_event.code {
                KeyCode::Up => state.select_previous_lobby_player(),
                KeyCode::Down => state.select_next_lobby_player(),
                _ => {}
            }
            continue;
        }

        if handle_lobby_key(
            &mut stream,
            key_event,
            &mut lobby_command,
            state.is_host(),
            state.current_phase(),
            state.selected_lobby_player(),
            &log,
        )? {
            break;
        }
    }

    terminal.restore()?;
    println!("Left server");
    push_network_log(&log, "client left server");
    if let (Some(path), Some(log)) = (config.debug_log.as_ref(), log.as_ref()) {
        write_network_log(path, log)?;
    }

    Ok(())
}

fn join_rejection_message(message: &str) -> String {
    if message.contains("already in use") {
        if message.contains("--name") {
            message.to_string()
        } else {
            format!("{message}. Choose a different --name.")
        }
    } else if message.starts_with("Lobby is full") || message.contains(" is full:") {
        format!("{message}. Wait for the next race or ask the host to remove a racer.")
    } else if message.contains("was not found") {
        format!("{message}. Check the room code and make sure the host is still online.")
    } else {
        message.to_string()
    }
}

fn is_terminal_server_error(message: &str) -> bool {
    message.starts_with("Game closed:") || message.starts_with("Room closed:")
}

fn server_error_display_message(message: &str) -> String {
    if message.starts_with("Game closed:") {
        message.to_string()
    } else if let Some(reason) = message.strip_prefix("Room closed:") {
        format!("Game closed:{reason}")
    } else {
        format!("Server error: {message}")
    }
}

#[derive(Debug, Clone)]
struct NetworkViewState {
    player_id: PlayerId,
    icon_mode: IconMode,
    online_room: Option<OnlineRoomInfo>,
    lobby_players: Vec<LobbyPlayer>,
    host_id: Option<PlayerId>,
    mod_config: Option<ModConfigSnapshot>,
    race_snapshot: Option<RaceSnapshot>,
    placements: Vec<PlayerId>,
    result_rows: Vec<RaceResultRow>,
    lobby_events: Vec<String>,
    messages: VecDeque<String>,
    selected_lobby_index: usize,
    show_help: bool,
    disconnected: bool,
}

impl NetworkViewState {
    fn new(player_id: PlayerId, icon_mode: IconMode, online_room: Option<OnlineRoomInfo>) -> Self {
        Self {
            player_id,
            icon_mode,
            online_room,
            lobby_players: Vec::new(),
            host_id: None,
            mod_config: None,
            race_snapshot: None,
            placements: Vec::new(),
            result_rows: Vec::new(),
            lobby_events: Vec::new(),
            messages: VecDeque::new(),
            selected_lobby_index: 0,
            show_help: false,
            disconnected: false,
        }
    }

    fn push_message(&mut self, message: String) {
        const MESSAGE_LIMIT: usize = 8;
        self.messages.push_back(message);
        while self.messages.len() > MESSAGE_LIMIT {
            self.messages.pop_front();
        }
    }

    fn is_host(&self) -> bool {
        self.host_id == Some(self.player_id)
    }

    fn selected_lobby_player(&self) -> Option<&LobbyPlayer> {
        self.lobby_players.get(self.selected_lobby_index)
    }

    fn local_player_name(&self) -> Option<String> {
        self.lobby_players
            .iter()
            .find(|player| player.id == self.player_id)
            .map(|player| player.name.clone())
            .or_else(|| {
                self.race_snapshot.as_ref().and_then(|snapshot| {
                    snapshot
                        .players
                        .iter()
                        .find(|player| player.id == self.player_id)
                        .map(|player| player.name.clone())
                })
            })
    }

    fn select_previous_lobby_player(&mut self) {
        if self.lobby_players.is_empty() {
            self.selected_lobby_index = 0;
        } else {
            self.selected_lobby_index = self.selected_lobby_index.saturating_sub(1);
        }
    }

    fn select_next_lobby_player(&mut self) {
        if self.lobby_players.is_empty() {
            self.selected_lobby_index = 0;
        } else {
            self.selected_lobby_index =
                (self.selected_lobby_index + 1).min(self.lobby_players.len() - 1);
        }
    }

    fn current_phase(&self) -> NetworkRacePhase {
        self.race_snapshot
            .as_ref()
            .map(|snapshot| snapshot.phase)
            .unwrap_or(NetworkRacePhase::WaitingForHost)
    }

    fn is_local_player_in_current_race(&self) -> bool {
        self.race_snapshot.as_ref().is_some_and(|snapshot| {
            snapshot
                .players
                .iter()
                .any(|player| player.id == self.player_id)
        })
    }

    fn apply_race_snapshot(&mut self, snapshot: RaceSnapshot) {
        self.mod_config = Some(snapshot.mod_config.clone());
        match snapshot.phase {
            NetworkRacePhase::Lobby | NetworkRacePhase::WaitingForHost => {
                self.race_snapshot = None;
                self.placements.clear();
                self.result_rows.clear();
                if self.selected_lobby_index >= self.lobby_players.len() {
                    self.selected_lobby_index = self.lobby_players.len().saturating_sub(1);
                }
            }
            NetworkRacePhase::Countdown { .. }
            | NetworkRacePhase::Racing
            | NetworkRacePhase::Finished => {
                self.race_snapshot = Some(snapshot);
            }
        }
    }

    fn apply_race_delta(&mut self, delta: RaceDeltaSnapshot) {
        if let Some(snapshot) = &mut self.race_snapshot {
            snapshot.sequence = delta.sequence;
            snapshot.phase = delta.phase;
            snapshot.bonuses = delta.bonuses;
            snapshot.players = delta.players;
            snapshot.events = delta.events;
            return;
        }

        self.push_message("Skipped race update until full race snapshot arrives".to_string());
    }
}

type NetworkTerminalBackend = CrosstermBackend<io::Stdout>;

struct NetworkTerminal {
    terminal: Terminal<NetworkTerminalBackend>,
    restored: bool,
}

impl NetworkTerminal {
    fn setup() -> Result<Self> {
        enable_raw_mode().context("failed to enable raw terminal mode")?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen).context("failed to enter alternate screen")?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend).context("failed to create terminal")?;
        terminal.clear().context("failed to clear terminal")?;

        Ok(Self {
            terminal,
            restored: false,
        })
    }

    fn draw(
        &mut self,
        state: &NetworkViewState,
        lobby_command: &str,
        rename_input: Option<&str>,
    ) -> Result<()> {
        self.terminal
            .draw(|frame| render_network(frame, state, lobby_command, rename_input))
            .context("failed to draw network screen")?;
        Ok(())
    }

    fn clear(&mut self) -> Result<()> {
        self.terminal
            .clear()
            .context("failed to clear network terminal")
    }

    fn restore(&mut self) -> Result<()> {
        if self.restored {
            return Ok(());
        }

        disable_raw_mode().context("failed to disable raw terminal mode")?;
        execute!(self.terminal.backend_mut(), LeaveAlternateScreen)
            .context("failed to leave alternate screen")?;
        self.terminal
            .show_cursor()
            .context("failed to show terminal cursor")?;
        self.restored = true;
        Ok(())
    }
}

impl Drop for NetworkTerminal {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

fn should_leave(key_event: KeyEvent) -> bool {
    key_event.code == KeyCode::Esc
        || (key_event.code == KeyCode::Char('c')
            && key_event.modifiers.contains(KeyModifiers::CONTROL))
}

fn host_cancel_key(is_host: bool, phase: NetworkRacePhase, key_event: KeyEvent) -> bool {
    is_host
        && matches!(
            phase,
            NetworkRacePhase::Countdown { .. }
                | NetworkRacePhase::Racing
                | NetworkRacePhase::Finished
        )
        && matches!(key_event.code, KeyCode::Char('r') | KeyCode::Char('R'))
        && key_event.modifiers.contains(KeyModifiers::CONTROL)
}

fn starts_rename_mode(phase: NetworkRacePhase, lobby_command: &str, key_event: KeyEvent) -> bool {
    lobby_command.is_empty()
        && matches!(
            phase,
            NetworkRacePhase::Lobby | NetworkRacePhase::WaitingForHost
        )
        && matches!(key_event.code, KeyCode::Char('n') | KeyCode::Char('N'))
        && (key_event.modifiers.is_empty() || key_event.modifiers == KeyModifiers::SHIFT)
}

fn rename_prefill(current_name: Option<&str>) -> String {
    current_name
        .filter(|name| !is_default_anonymous_name(name))
        .unwrap_or_default()
        .to_string()
}

fn is_default_anonymous_name(name: &str) -> bool {
    let Some(suffix) = name.strip_prefix("anonymous") else {
        return false;
    };
    suffix.is_empty() || suffix.chars().all(|ch| ch.is_ascii_digit())
}

fn handle_rename_key(
    stream: &mut std::net::TcpStream,
    key_event: KeyEvent,
    input: &mut String,
    log: &Option<SharedNetworkLog>,
) -> Result<bool> {
    match key_event.code {
        KeyCode::Esc => Ok(true),
        KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r') => {
            let name = input.trim();
            if !name.is_empty() {
                send_client_message(
                    stream,
                    &ClientMessage::Rename {
                        name: name.to_string(),
                    },
                )?;
                push_network_log(log, format!("client sent rename name={name}"));
            }
            Ok(true)
        }
        KeyCode::Backspace => {
            input.pop();
            Ok(false)
        }
        KeyCode::Char(ch)
            if key_event.modifiers.is_empty() || key_event.modifiers == KeyModifiers::SHIFT =>
        {
            input.push(ch);
            Ok(false)
        }
        _ => Ok(false),
    }
}

fn handle_lobby_key(
    stream: &mut std::net::TcpStream,
    key_event: KeyEvent,
    lobby_command: &mut String,
    is_host: bool,
    phase: NetworkRacePhase,
    selected_player: Option<&LobbyPlayer>,
    log: &Option<SharedNetworkLog>,
) -> Result<bool> {
    if space_starts_countdown(is_host, phase)
        && lobby_command.is_empty()
        && key_event.code == KeyCode::Char(' ')
    {
        send_client_message(stream, &ClientMessage::StartCountdown)?;
        push_network_log(log, "client sent start countdown");
        return Ok(false);
    }

    if is_host
        && lobby_command.is_empty()
        && matches!(
            phase,
            NetworkRacePhase::Lobby | NetworkRacePhase::WaitingForHost
        )
    {
        match key_event.code {
            KeyCode::Char('a') | KeyCode::Char('A')
                if key_event.modifiers.is_empty() || key_event.modifiers == KeyModifiers::SHIFT =>
            {
                send_client_message(stream, &ClientMessage::AddAi)?;
                push_network_log(log, "client sent add ai");
                return Ok(false);
            }
            KeyCode::Char('x') | KeyCode::Char('X')
                if key_event.modifiers.is_empty() || key_event.modifiers == KeyModifiers::SHIFT =>
            {
                if let Some(player) = selected_player {
                    send_client_message(
                        stream,
                        &ClientMessage::RemoveLobbyPlayer {
                            player_id: player.id,
                        },
                    )?;
                    push_network_log(log, format!("client sent remove player={}", player.id.0));
                }
                return Ok(false);
            }
            KeyCode::Char('e') | KeyCode::Char('E')
                if key_event.modifiers.is_empty() || key_event.modifiers == KeyModifiers::SHIFT =>
            {
                send_ai_difficulty_command(
                    stream,
                    selected_player,
                    AiDifficultySnapshot::Easy,
                    log,
                )?;
                return Ok(false);
            }
            KeyCode::Char('h') | KeyCode::Char('H')
                if key_event.modifiers.is_empty() || key_event.modifiers == KeyModifiers::SHIFT =>
            {
                send_ai_difficulty_command(
                    stream,
                    selected_player,
                    AiDifficultySnapshot::Hard,
                    log,
                )?;
                return Ok(false);
            }
            _ => {}
        }
    }

    if is_enter_key(key_event) {
        if enter_sets_ready(!is_host, phase, lobby_command) {
            send_client_message(stream, &ClientMessage::SetReady { ready: true })?;
            push_network_log(log, "client sent ready from enter");
            return Ok(false);
        }
        let should_leave =
            send_lifecycle_command(stream, lobby_command.trim(), is_host, phase, log)?;
        lobby_command.clear();
        return Ok(should_leave);
    }

    if !phase_accepts_typed_commands(is_host, phase) {
        return Ok(false);
    }

    match key_event.code {
        KeyCode::Backspace => {
            lobby_command.pop();
            Ok(false)
        }
        KeyCode::Char(ch)
            if key_event.modifiers.is_empty() || key_event.modifiers == KeyModifiers::SHIFT =>
        {
            lobby_command.push(ch);
            Ok(false)
        }
        _ => Ok(false),
    }
}

fn send_ai_difficulty_command(
    stream: &mut std::net::TcpStream,
    selected_player: Option<&LobbyPlayer>,
    difficulty: AiDifficultySnapshot,
    log: &Option<SharedNetworkLog>,
) -> Result<()> {
    let player_id = selected_player
        .filter(|player| player.kind == PlayerKind::Bot)
        .map(|player| player.id);
    send_client_message(
        stream,
        &ClientMessage::SetAiDifficulty {
            player_id,
            difficulty,
        },
    )?;
    push_network_log(
        log,
        format!("client sent ai difficulty={difficulty:?} player={player_id:?}"),
    );
    Ok(())
}

fn is_enter_key(key_event: KeyEvent) -> bool {
    matches!(
        key_event.code,
        KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r')
    ) || (matches!(key_event.code, KeyCode::Char('j') | KeyCode::Char('m'))
        && key_event.modifiers.contains(KeyModifiers::CONTROL))
}

fn send_lifecycle_command(
    stream: &mut std::net::TcpStream,
    command: &str,
    is_host: bool,
    phase: NetworkRacePhase,
    log: &Option<SharedNetworkLog>,
) -> Result<bool> {
    let Some(message) = lifecycle_command_message(command, is_host, phase) else {
        return Ok(false);
    };

    send_client_message(stream, &message)?;
    push_network_log(log, format!("client sent lobby command={command}"));

    Ok(matches!(message, ClientMessage::Leave))
}

fn lifecycle_command_message(
    command: &str,
    is_host: bool,
    phase: NetworkRacePhase,
) -> Option<ClientMessage> {
    match (command, phase) {
        ("ready", NetworkRacePhase::Lobby | NetworkRacePhase::WaitingForHost) => {
            Some(ClientMessage::SetReady { ready: true })
        }
        ("unready", NetworkRacePhase::Lobby | NetworkRacePhase::WaitingForHost) => {
            Some(ClientMessage::SetReady { ready: false })
        }
        ("start", NetworkRacePhase::Lobby | NetworkRacePhase::WaitingForHost) if is_host => {
            Some(ClientMessage::StartCountdown)
        }
        ("start", NetworkRacePhase::Finished) if is_host => Some(ClientMessage::StartCountdown),
        ("lobby" | "restart" | "rematch", NetworkRacePhase::Finished) if is_host => {
            Some(ClientMessage::RestartRace)
        }
        ("quit" | "leave", _) => Some(ClientMessage::Leave),
        ("", _) => None,
        _ => None,
    }
}

fn space_starts_countdown(is_host: bool, phase: NetworkRacePhase) -> bool {
    is_host
        && matches!(
            phase,
            NetworkRacePhase::Lobby | NetworkRacePhase::WaitingForHost | NetworkRacePhase::Finished
        )
}

fn enter_sets_ready(is_joiner: bool, phase: NetworkRacePhase, lobby_command: &str) -> bool {
    is_joiner
        && lobby_command.is_empty()
        && matches!(
            phase,
            NetworkRacePhase::Lobby | NetworkRacePhase::WaitingForHost
        )
}

fn phase_accepts_typed_commands(_is_host: bool, phase: NetworkRacePhase) -> bool {
    matches!(
        phase,
        NetworkRacePhase::Lobby | NetworkRacePhase::WaitingForHost
    ) || phase == NetworkRacePhase::Finished
}

fn send_race_key(
    stream: &mut std::net::TcpStream,
    key_event: KeyEvent,
    sequence: &mut u64,
    log: &Option<SharedNetworkLog>,
) -> Result<bool> {
    let key = match key_event.code {
        KeyCode::Char(' ') => Some(ProtocolKey::Space),
        KeyCode::Char(ch)
            if key_event.modifiers.is_empty() || key_event.modifiers == KeyModifiers::SHIFT =>
        {
            Some(ProtocolKey::Char(ch))
        }
        KeyCode::Backspace => Some(ProtocolKey::Backspace),
        KeyCode::Enter => None,
        _ => None,
    };

    let Some(key) = key else {
        return Ok(false);
    };

    send_client_message(
        stream,
        &ClientMessage::KeyInput {
            sequence: ClientSequence(*sequence),
            key,
        },
    )?;
    push_network_log(
        log,
        format!("client sent key_input seq={} key={key:?}", *sequence),
    );
    *sequence += 1;

    Ok(false)
}

fn send_client_message(stream: &mut std::net::TcpStream, message: &ClientMessage) -> Result<()> {
    write_client_message(stream, message)
}

fn log_client_snapshot(log: &Option<SharedNetworkLog>, snapshot: &RaceSnapshot) {
    match snapshot.phase {
        NetworkRacePhase::Countdown { remaining_seconds } => push_network_log(
            log,
            format!(
                "client received snapshot seq={} phase=countdown remaining={remaining_seconds}",
                snapshot.sequence
            ),
        ),
        NetworkRacePhase::Racing if snapshot.sequence.is_multiple_of(20) => push_network_log(
            log,
            format!(
                "client received snapshot seq={} phase=racing",
                snapshot.sequence
            ),
        ),
        NetworkRacePhase::Finished => push_network_log(
            log,
            format!(
                "client received snapshot seq={} phase=finished",
                snapshot.sequence
            ),
        ),
        _ => {}
    }
}

fn log_client_delta(log: &Option<SharedNetworkLog>, delta: &RaceDeltaSnapshot) {
    match delta.phase {
        NetworkRacePhase::Racing if delta.sequence.is_multiple_of(20) => push_network_log(
            log,
            format!("client received delta seq={} phase=racing", delta.sequence),
        ),
        NetworkRacePhase::Finished => push_network_log(
            log,
            format!(
                "client received delta seq={} phase=finished",
                delta.sequence
            ),
        ),
        _ => {}
    }
}

fn render_network(
    frame: &mut Frame<'_>,
    state: &NetworkViewState,
    lobby_command: &str,
    rename_input: Option<&str>,
) {
    let area = frame.size();
    frame.render_widget(Clear, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(area);

    frame.render_widget(network_header(state), rows[0]);
    match &state.race_snapshot {
        Some(snapshot) => render_race(frame, rows[1], state, snapshot),
        None => render_lobby(frame, rows[1], state),
    }
    frame.render_widget(network_footer(state, lobby_command, rename_input), rows[2]);
    if state.show_help {
        render_network_help_overlay(frame, area, state.is_host(), state.current_phase());
    }
}

fn network_header<'a>(state: &NetworkViewState) -> Paragraph<'a> {
    let phase = state
        .race_snapshot
        .as_ref()
        .map(|snapshot| format_phase(snapshot.phase))
        .unwrap_or_else(|| "lobby".to_string());
    let connection = if state.disconnected {
        "disconnected"
    } else {
        "connected"
    };

    let mut spans = vec![
        Span::styled(
            "TypeKart Network",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw("    "),
        Span::raw(format!("player {}", state.player_id.0)),
        Span::raw("    "),
        Span::raw(phase),
        Span::raw("    "),
        Span::styled(connection, connection_style(state.disconnected)),
    ];
    if let Some(room) = &state.online_room {
        spans.extend([
            Span::raw("    "),
            Span::styled(
                format!("room {} via {}", room.room.display(), room.relay),
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]);
    }

    Paragraph::new(Line::from(spans)).block(Block::default().borders(Borders::BOTTOM))
}

fn render_lobby(frame: &mut Frame<'_>, area: Rect, state: &NetworkViewState) {
    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(5),
            Constraint::Length(6),
        ])
        .split(area);

    let players = state
        .lobby_players
        .iter()
        .enumerate()
        .map(|(index, player)| {
            let selected = index == state.selected_lobby_index;
            let ai_details = match (player.ai_difficulty, player.ai_wpm) {
                (Some(difficulty), Some(wpm)) => {
                    format!(" {} {wpm}wpm", ai_difficulty_label(difficulty))
                }
                (Some(difficulty), None) => format!(" {}", ai_difficulty_label(difficulty)),
                _ => String::new(),
            };
            Line::from(vec![
                Span::raw(if selected { "> " } else { "  " }),
                Span::styled("● ", Style::default().fg(assigned_color(player.color))),
                Span::styled(
                    player.name.clone(),
                    Style::default()
                        .fg(assigned_color(player.color))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!(
                    "  {}{}{}{}",
                    if player.ready { "ready" } else { "not ready" },
                    ai_details,
                    if player.connected {
                        ""
                    } else {
                        " disconnected"
                    },
                    if Some(player.id) == state.host_id {
                        " host"
                    } else {
                        ""
                    }
                )),
            ])
        })
        .collect::<Vec<_>>();

    frame.render_widget(
        Paragraph::new(players).block(Block::default().title("Lobby").borders(Borders::ALL)),
        body[0],
    );
    frame.render_widget(mod_config_view(state.mod_config.as_ref()), body[1]);
    frame.render_widget(messages_view(state, body[2].height), body[2]);
}

fn ai_difficulty_label(difficulty: AiDifficultySnapshot) -> &'static str {
    match difficulty {
        AiDifficultySnapshot::Easy => "easy",
        AiDifficultySnapshot::Hard => "hard",
    }
}

fn render_race(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &NetworkViewState,
    snapshot: &RaceSnapshot,
) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
        .split(area);
    let right = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(45),
            Constraint::Length(5),
            Constraint::Percentage(55),
        ])
        .split(columns[1]);

    frame.render_widget(track_view(state, snapshot, columns[0].width), columns[0]);
    frame.render_widget(race_status_view(state, snapshot), right[0]);
    frame.render_widget(mod_config_view(Some(&snapshot.mod_config)), right[1]);
    frame.render_widget(
        messages_and_events_view(state, snapshot, right[2].height),
        right[2],
    );
}

fn track_view<'a>(
    state: &NetworkViewState,
    snapshot: &'a RaceSnapshot,
    area_width: u16,
) -> Paragraph<'a> {
    let local = snapshot
        .players
        .iter()
        .find(|player| player.id == state.player_id);
    let width = usize::from(area_width.saturating_sub(2)).max(1);
    let current_word_index = local
        .map(|player| player.word_index)
        .or_else(|| snapshot.players.first().map(|player| player.word_index))
        .unwrap_or(0);
    let window = NetworkTrackWindow::new(&snapshot.track_words, current_word_index, width);
    let mut lines = network_bonus_lines(&window, snapshot, local);
    lines.push(network_track_word_line(&window, local));
    lines.extend(network_racer_lines(&window, state, snapshot));
    lines.push(network_minimap_line(state, snapshot, width));

    Paragraph::new(lines).block(Block::default().title("Track").borders(Borders::ALL))
}

fn player_list_view<'a>(state: &NetworkViewState, snapshot: &'a RaceSnapshot) -> Paragraph<'a> {
    let mut players = snapshot.players.clone();
    players.sort_by_key(|player| player_sort_key(state, player));

    let lines = players
        .iter()
        .enumerate()
        .map(|(index, player)| {
            let marker = if player.id == state.player_id {
                "@"
            } else {
                "●"
            };
            Line::from(vec![
                Span::raw(format!("{:>2}. ", index + 1)),
                Span::styled(
                    marker,
                    Style::default()
                        .fg(assigned_color(player.color))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::styled(
                    player.name.clone(),
                    Style::default()
                        .fg(assigned_color(player.color))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!(
                    "  word {}{}{}{}",
                    display_word_number(player, snapshot.track_words.len()),
                    placement_label(state, player.id),
                    if player.finished { " finished" } else { "" },
                    if player.connected {
                        ""
                    } else {
                        " disconnected"
                    }
                )),
            ])
        })
        .collect::<Vec<_>>();

    Paragraph::new(lines).block(Block::default().title("Players").borders(Borders::ALL))
}

fn race_status_view<'a>(state: &NetworkViewState, snapshot: &'a RaceSnapshot) -> Paragraph<'a> {
    if !state.result_rows.is_empty() || snapshot.phase == NetworkRacePhase::Finished {
        results_view(state, snapshot)
    } else {
        player_list_view(state, snapshot)
    }
}

fn results_view<'a>(state: &NetworkViewState, snapshot: &'a RaceSnapshot) -> Paragraph<'a> {
    if state.result_rows.is_empty() {
        return player_list_view(state, snapshot);
    }

    let lines = state
        .result_rows
        .iter()
        .map(|row| {
            Line::from(vec![
                Span::raw(format!("{:>2}. ", row.placement)),
                Span::styled(
                    result_status_label(row.status),
                    result_status_style(row.status),
                ),
                Span::raw(" "),
                Span::styled(
                    row.name.clone(),
                    Style::default()
                        .fg(assigned_color(row.color))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!(
                    " {}/{} {}wpm {}% e{} b{}",
                    row.progress_words,
                    row.track_words,
                    row.wpm,
                    row.accuracy_percent,
                    row.typo_chars,
                    row.backspaces
                )),
            ])
        })
        .collect::<Vec<_>>();

    Paragraph::new(lines).block(Block::default().title("Results").borders(Borders::ALL))
}

fn result_status_label(status: RaceResultStatus) -> &'static str {
    match status {
        RaceResultStatus::Finished => "done",
        RaceResultStatus::TimedOut => "time",
        RaceResultStatus::Disconnected => "disc",
    }
}

fn result_status_style(status: RaceResultStatus) -> Style {
    match status {
        RaceResultStatus::Finished => Style::default().fg(Color::Green),
        RaceResultStatus::TimedOut => Style::default().fg(Color::Yellow),
        RaceResultStatus::Disconnected => Style::default().fg(Color::Red),
    }
}

fn display_word_number(player: &PlayerSnapshot, track_len: usize) -> usize {
    if track_len == 0 {
        return 0;
    }

    if player.finished {
        track_len
    } else {
        (player.word_index + 1).min(track_len)
    }
}

fn player_sort_key(
    state: &NetworkViewState,
    player: &PlayerSnapshot,
) -> (
    usize,
    std::cmp::Reverse<usize>,
    std::cmp::Reverse<usize>,
    u64,
) {
    let placement_rank = state
        .placements
        .iter()
        .position(|id| *id == player.id)
        .unwrap_or(usize::MAX);

    (
        placement_rank,
        std::cmp::Reverse(player.word_index),
        std::cmp::Reverse(player.input.chars().count()),
        player.id.0,
    )
}

fn placement_label(state: &NetworkViewState, id: PlayerId) -> String {
    state
        .placements
        .iter()
        .position(|placement_id| *placement_id == id)
        .map(|index| format!(" place={}", index + 1))
        .unwrap_or_default()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NetworkVisibleWord<'a> {
    index: usize,
    word: &'a str,
    start_col: usize,
    end_col: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NetworkTrackWindow<'a> {
    words: Vec<NetworkVisibleWord<'a>>,
    width: usize,
    track_len: usize,
}

impl<'a> NetworkTrackWindow<'a> {
    fn new(words: &'a [String], current_index: usize, width: usize) -> Self {
        let width = width.max(1);
        let track_len = words.len();
        if words.is_empty() {
            return Self {
                words: Vec::new(),
                width,
                track_len,
            };
        }

        let current_index = current_index.min(words.len().saturating_sub(1));
        let mut first_index = current_index.saturating_sub(3);
        let mut visible = build_network_visible_words(words, first_index, width);
        while !visible.iter().any(|word| word.index == current_index) && first_index < current_index
        {
            first_index += 1;
            visible = build_network_visible_words(words, first_index, width);
        }

        Self {
            words: visible,
            width,
            track_len,
        }
    }

    fn marker_for_player(&self, player: &PlayerSnapshot) -> NetworkMarkerPosition {
        let Some(first_visible) = self.words.first().map(|word| word.index) else {
            return NetworkMarkerPosition::Visible { start: 0 };
        };
        let Some(last_visible) = self.words.last().map(|word| word.index) else {
            return NetworkMarkerPosition::Visible { start: 0 };
        };

        if player.word_index < first_visible {
            return NetworkMarkerPosition::Behind;
        }

        if player.finished
            && self
                .words
                .iter()
                .any(|word| word.index + 1 == self.track_len)
        {
            return NetworkMarkerPosition::Visible {
                start: self
                    .column_for_player(player)
                    .saturating_sub(NETWORK_RACER_MARKER.chars().count() / 2)
                    .min(
                        self.width
                            .saturating_sub(NETWORK_RACER_MARKER.chars().count()),
                    ),
            };
        }

        if player.finished {
            return NetworkMarkerPosition::FinishedAhead;
        }

        if player.word_index > last_visible {
            return NetworkMarkerPosition::Ahead;
        }

        NetworkMarkerPosition::Visible {
            start: self
                .column_for_player(player)
                .saturating_sub(NETWORK_RACER_MARKER.chars().count() / 2)
                .min(
                    self.width
                        .saturating_sub(NETWORK_RACER_MARKER.chars().count()),
                ),
        }
    }

    fn column_for_player(&self, player: &PlayerSnapshot) -> usize {
        let target_stream_index = player
            .typo_index
            .unwrap_or_else(|| player.input.chars().count());

        self.column_for_stream_index(player.word_index, target_stream_index)
            .or_else(|| {
                self.words
                    .iter()
                    .find(|word| word.index == player.word_index)
                    .map(|word| word.start_col)
            })
            .unwrap_or(0)
    }

    fn column_for_stream_index(
        &self,
        current_word_index: usize,
        target_stream_index: usize,
    ) -> Option<usize> {
        let mut stream_index = 0;
        let mut previous_visible_word_index = None;

        for visible in self
            .words
            .iter()
            .filter(|word| word.index >= current_word_index)
        {
            if previous_visible_word_index.is_some() {
                if target_stream_index == stream_index {
                    return Some(visible.start_col.saturating_sub(1));
                }
                stream_index += 1;
            }

            let word_width = visible.word.chars().count();
            if target_stream_index < stream_index + word_width {
                return Some(visible.start_col + target_stream_index - stream_index);
            }

            stream_index += word_width;
            previous_visible_word_index = Some(visible.index);
        }

        self.words
            .last()
            .map(|word| word.end_col.min(self.width.saturating_sub(1)))
    }
}

fn build_network_visible_words<'a>(
    words: &'a [String],
    first_index: usize,
    width: usize,
) -> Vec<NetworkVisibleWord<'a>> {
    let mut visible = Vec::new();
    let mut col = 0;
    for (index, word) in words.iter().enumerate().skip(first_index) {
        let word_width = word.chars().count();
        let separator_width = usize::from(!visible.is_empty());
        if !visible.is_empty() && col + separator_width + word_width > width {
            break;
        }
        if visible.is_empty() && word_width > width {
            visible.push(NetworkVisibleWord {
                index,
                word,
                start_col: 0,
                end_col: width,
            });
            break;
        }
        if !visible.is_empty() {
            col += separator_width;
        }

        let start_col = col;
        let available_width = width.saturating_sub(start_col);
        let rendered_width = word_width.min(available_width);
        visible.push(NetworkVisibleWord {
            index,
            word,
            start_col,
            end_col: start_col + rendered_width,
        });
        col += rendered_width;

        if col >= width {
            break;
        }
    }

    visible
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NetworkMarkerPosition {
    Visible { start: usize },
    Behind,
    Ahead,
    FinishedAhead,
}

#[derive(Debug, Clone, Copy)]
struct NetworkTrackCell {
    ch: char,
    style: Style,
}

impl Default for NetworkTrackCell {
    fn default() -> Self {
        Self {
            ch: ' ',
            style: Style::default(),
        }
    }
}

fn network_track_word_line<'a>(
    window: &NetworkTrackWindow<'a>,
    local: Option<&PlayerSnapshot>,
) -> Line<'a> {
    let mut cells = vec![NetworkTrackCell::default(); window.width];
    for word in &window.words {
        let display_word = local
            .and_then(|player| network_word_override(player, word.index))
            .unwrap_or(word.word);
        for (char_index, ch) in display_word.chars().enumerate() {
            let column = word.start_col + char_index;
            if let Some(cell) = cells.get_mut(column) {
                *cell = NetworkTrackCell {
                    ch: network_visible_word_char(local, word.index, ch),
                    style: network_word_char_style(window, word, char_index, local),
                };
            }
        }
    }

    if let Some(local) = local {
        overlay_network_local_input(&mut cells, window, local);
    }

    Line::from(
        cells
            .into_iter()
            .map(|cell| Span::styled(cell.ch.to_string(), cell.style))
            .collect::<Vec<_>>(),
    )
}

fn network_visible_word_char(local: Option<&PlayerSnapshot>, word_index: usize, ch: char) -> char {
    if local.is_some_and(|player| player.fogged && word_index > player.word_index) {
        '█'
    } else {
        ch
    }
}

fn network_word_override(player: &PlayerSnapshot, word_index: usize) -> Option<&str> {
    player
        .word_overrides
        .iter()
        .find(|override_word| override_word.word_index == word_index)
        .map(|override_word| override_word.word.as_str())
}

fn overlay_network_local_input(
    cells: &mut [NetworkTrackCell],
    window: &NetworkTrackWindow<'_>,
    local: &PlayerSnapshot,
) {
    if local.finished {
        return;
    }

    for (stream_index, typed_ch) in local.input.chars().enumerate() {
        let Some(column) = window.column_for_stream_index(local.word_index, stream_index) else {
            break;
        };
        let Some(cell) = cells.get_mut(column) else {
            break;
        };

        cell.ch = display_network_input_char(typed_ch);
        cell.style = network_typed_char_style(stream_index, local.typo_index);
    }
}

fn display_network_input_char(ch: char) -> char {
    if ch == ' ' { '␠' } else { ch }
}

fn network_typed_char_style(stream_index: usize, typo_index: Option<usize>) -> Style {
    if typo_index.is_some_and(|typo| stream_index >= typo) {
        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD)
    }
}

fn network_bonus_lines<'a>(
    window: &NetworkTrackWindow<'_>,
    snapshot: &'a RaceSnapshot,
    local: Option<&PlayerSnapshot>,
) -> Vec<Line<'a>> {
    let Some(point) = visible_network_bonus_point(window, snapshot) else {
        return vec![Line::from(""), Line::from(""), Line::from("")];
    };

    (0..3)
        .map(|choice_index| {
            let Some(choice) = point.choices.get(choice_index) else {
                return Line::from("");
            };
            let text = match choice.status {
                super::protocol::BonusChoiceSnapshotStatus::Available => choice.word.clone(),
                super::protocol::BonusChoiceSnapshotStatus::Cooldown { .. } => "---".to_string(),
            };
            let start = network_bonus_column(window, point.after_word_index, text.chars().count());
            let available_to_local = local.is_some_and(|player| {
                network_bonus_choice_available_to_player(player, point, choice)
            });
            Line::from(vec![
                Span::raw(" ".repeat(start)),
                Span::styled(
                    text,
                    if available_to_local {
                        Style::default().fg(Color::Magenta)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    },
                ),
            ])
        })
        .collect()
}

fn network_bonus_choice_available_to_player(
    player: &PlayerSnapshot,
    point: &super::protocol::BonusPointSnapshot,
    choice: &super::protocol::BonusChoiceSnapshot,
) -> bool {
    matches!(
        choice.status,
        super::protocol::BonusChoiceSnapshotStatus::Available
    ) && player.word_index == point.after_word_index.saturating_add(1)
        && !player.finished
        && !player.shielded
        && !player.focused
        && !player.boosted
        && !player.stunned
        && player.typo_index.is_none()
        && (player.input.is_empty() || choice.word.starts_with(&player.input))
}

fn visible_network_bonus_point<'a>(
    window: &NetworkTrackWindow<'_>,
    snapshot: &'a RaceSnapshot,
) -> Option<&'a super::protocol::BonusPointSnapshot> {
    let first_visible = window.words.first()?.index;
    let last_visible = window.words.last()?.index;

    snapshot
        .bonuses
        .iter()
        .filter(|point| point.after_word_index >= first_visible)
        .filter(|point| point.after_word_index.saturating_add(1) <= last_visible)
        .min_by_key(|point| point.after_word_index)
}

fn network_bonus_column(
    window: &NetworkTrackWindow<'_>,
    after_word_index: usize,
    word_width: usize,
) -> usize {
    let Some(before_word) = window
        .words
        .iter()
        .find(|word| word.index == after_word_index)
    else {
        return 0;
    };
    let Some(after_word) = window
        .words
        .iter()
        .find(|word| word.index == after_word_index + 1)
    else {
        return 0;
    };

    let center = (before_word.end_col + after_word.start_col) / 2;
    center
        .saturating_sub(word_width / 2)
        .min(window.width.saturating_sub(word_width))
}

fn network_word_char_style(
    window: &NetworkTrackWindow<'_>,
    word: &NetworkVisibleWord<'_>,
    char_index: usize,
    local: Option<&PlayerSnapshot>,
) -> Style {
    let Some(local) = local else {
        return Style::default();
    };

    if word.index < local.word_index || local.finished {
        return Style::default().fg(Color::DarkGray);
    }

    let Some(stream_index) = stream_index_for_word_char(window, local.word_index, word, char_index)
    else {
        return Style::default();
    };
    let input_len = local.input.chars().count();

    if local
        .typo_index
        .is_some_and(|typo_index| stream_index >= typo_index)
        && stream_index < input_len
    {
        return Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
    }

    if stream_index < input_len {
        return Style::default().fg(Color::Green);
    }

    if word.index == local.word_index {
        if local.focused {
            Style::default()
                .fg(Color::LightMagenta)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        }
    } else if local.focused {
        Style::default().fg(Color::LightMagenta)
    } else {
        Style::default()
    }
}

fn stream_index_for_word_char(
    window: &NetworkTrackWindow<'_>,
    current_word_index: usize,
    target_word: &NetworkVisibleWord<'_>,
    target_char_index: usize,
) -> Option<usize> {
    if target_word.index < current_word_index {
        return None;
    }

    let mut stream_index = 0;
    let mut previous_visible_word_index = None;
    for visible in window
        .words
        .iter()
        .filter(|word| word.index >= current_word_index)
    {
        if previous_visible_word_index.is_some() {
            stream_index += 1;
        }

        if visible.index == target_word.index {
            return Some(stream_index + target_char_index);
        }

        stream_index += visible.word.chars().count();
        previous_visible_word_index = Some(visible.index);
    }

    None
}

fn network_racer_lines<'a>(
    window: &NetworkTrackWindow<'_>,
    state: &NetworkViewState,
    snapshot: &'a RaceSnapshot,
) -> Vec<Line<'a>> {
    let mut players = snapshot.players.clone();
    players.sort_by_key(|player| {
        if player.id == state.player_id {
            (0, 0)
        } else {
            (1, player.id.0)
        }
    });

    players
        .iter()
        .map(|player| {
            network_racer_line(
                window,
                player,
                player.id == state.player_id,
                snapshot.phase,
                state.icon_mode,
            )
        })
        .collect()
}

fn network_racer_line<'a>(
    window: &NetworkTrackWindow<'_>,
    player: &PlayerSnapshot,
    is_local: bool,
    phase: NetworkRacePhase,
    icon_mode: IconMode,
) -> Line<'a> {
    let mut cells = vec![NetworkTrackCell::default(); window.width];
    let color = assigned_color(player.color);
    let style = network_marker_style(player, color);

    let (start, marker) = match window.marker_for_player(player) {
        NetworkMarkerPosition::Visible { start } => (start, NETWORK_RACER_MARKER),
        NetworkMarkerPosition::Behind => (0, "<"),
        NetworkMarkerPosition::Ahead => (window.width.saturating_sub(1), ">"),
        NetworkMarkerPosition::FinishedAhead => (
            window
                .width
                .saturating_sub(NETWORK_FINISHED_EDGE_MARKER.chars().count()),
            NETWORK_FINISHED_EDGE_MARKER,
        ),
    };

    let mut prefix = String::new();
    if player.boosted {
        prefix.push_str(network_boost_prefix(icon_mode));
    }
    if player.focused && player.shielded {
        prefix.push_str(network_focus_prefix(icon_mode));
    }
    if !prefix.is_empty() {
        let prefix_start = start.saturating_sub(prefix.chars().count());
        write_network_marker(&mut cells, prefix_start, &prefix, style);
    }
    let mut after_marker_width = 0;
    if let Some((cue, placement)) = network_item_cue(player, icon_mode) {
        match placement {
            NetworkCuePlacement::Before => {
                let cue_start = start.saturating_sub(cue.chars().count());
                write_network_marker(&mut cells, cue_start, cue, style);
            }
            NetworkCuePlacement::After => {
                write_network_marker(&mut cells, start + marker.chars().count(), cue, style);
                after_marker_width = cue.chars().count();
            }
        }
    }

    let marker = if let Some(marker) = network_impact_marker(marker, player, icon_mode) {
        marker
    } else if player.shielded && matches!(marker, NETWORK_RACER_MARKER) {
        network_shield_marker(icon_mode)
    } else if player.shielded && marker == "<" {
        network_edge_shield_marker('<', icon_mode)
    } else if player.shielded && marker == ">" {
        network_edge_shield_marker('>', icon_mode)
    } else if player.focused && matches!(marker, NETWORK_RACER_MARKER) {
        network_focus_marker(icon_mode)
    } else if player.focused && marker == "<" {
        network_edge_focus_marker('<', icon_mode)
    } else if player.focused && marker == ">" {
        network_edge_focus_marker('>', icon_mode)
    } else {
        marker
    };
    write_network_marker(&mut cells, start, marker, style);
    let label = network_racer_label(is_local, phase);
    if let Some(label) = label {
        write_network_marker(
            &mut cells,
            start + marker.chars().count() + after_marker_width,
            label.as_str(),
            style,
        );
    }

    Line::from(
        cells
            .into_iter()
            .map(|cell| Span::styled(cell.ch.to_string(), cell.style))
            .collect::<Vec<_>>(),
    )
}

fn network_racer_label(is_local: bool, phase: NetworkRacePhase) -> Option<String> {
    if !is_local {
        return None;
    }

    match phase {
        NetworkRacePhase::WaitingForHost => Some(" Space".to_string()),
        NetworkRacePhase::Countdown { remaining_seconds } => Some(format!(" {remaining_seconds}")),
        NetworkRacePhase::Lobby | NetworkRacePhase::Racing | NetworkRacePhase::Finished => {
            Some(" you".to_string())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NetworkCuePlacement {
    Before,
    After,
}

fn network_item_cue(
    player: &PlayerSnapshot,
    icon_mode: IconMode,
) -> Option<(&str, NetworkCuePlacement)> {
    let cue = player.item_cue.as_ref()?;
    let label = match icon_mode {
        IconMode::Ascii => cue.ascii_label.as_str(),
        IconMode::Unicode => cue.unicode_label.as_str(),
    };
    let placement = match cue.placement {
        ItemCuePlacementSnapshot::Before => NetworkCuePlacement::Before,
        ItemCuePlacementSnapshot::After => NetworkCuePlacement::After,
    };
    Some((label, placement))
}

fn network_boost_prefix(icon_mode: IconMode) -> &'static str {
    match icon_mode {
        IconMode::Ascii => NETWORK_BOOST_PREFIX,
        IconMode::Unicode => ">>🍄",
    }
}

fn network_focus_prefix(icon_mode: IconMode) -> &'static str {
    match icon_mode {
        IconMode::Ascii => "*",
        IconMode::Unicode => "★",
    }
}

fn network_shield_marker(icon_mode: IconMode) -> &'static str {
    match icon_mode {
        IconMode::Ascii => "[███]",
        IconMode::Unicode => "█🛡",
    }
}

fn network_focus_marker(icon_mode: IconMode) -> &'static str {
    match icon_mode {
        IconMode::Ascii => "[*]",
        IconMode::Unicode => "█★█",
    }
}

fn network_edge_shield_marker(direction: char, icon_mode: IconMode) -> &'static str {
    match (direction, icon_mode) {
        ('<', IconMode::Ascii) => "[<]",
        ('>', IconMode::Ascii) => "[>]",
        ('<', IconMode::Unicode) => "<🛡",
        ('>', IconMode::Unicode) => ">🛡",
        _ => "",
    }
}

fn network_edge_focus_marker(direction: char, icon_mode: IconMode) -> &'static str {
    match (direction, icon_mode) {
        ('<', IconMode::Ascii) => "*<*",
        ('>', IconMode::Ascii) => "*>*",
        ('<', IconMode::Unicode) => "<★",
        ('>', IconMode::Unicode) => ">★",
        _ => "*",
    }
}

fn network_impact_marker(
    marker: &str,
    player: &PlayerSnapshot,
    icon_mode: IconMode,
) -> Option<&'static str> {
    let symbol = network_impact_marker_symbol(player, icon_mode)?;
    match (marker, symbol, icon_mode) {
        (NETWORK_RACER_MARKER, "🍌", IconMode::Unicode) => Some("█🍌"),
        (NETWORK_RACER_MARKER, "🌀", IconMode::Unicode) => Some("█🌀"),
        (NETWORK_RACER_MARKER, "🌫", IconMode::Unicode) => Some("█🌫"),
        (NETWORK_RACER_MARKER, "🛡", IconMode::Unicode) => Some("█🛡"),
        ("<", "🍌", IconMode::Unicode) => Some("<🍌"),
        ("<", "🌀", IconMode::Unicode) => Some("<🌀"),
        ("<", "🌫", IconMode::Unicode) => Some("<🌫"),
        ("<", "🛡", IconMode::Unicode) => Some("<🛡"),
        (">", "🍌", IconMode::Unicode) => Some(">🍌"),
        (">", "🌀", IconMode::Unicode) => Some(">🌀"),
        (">", "🌫", IconMode::Unicode) => Some(">🌫"),
        (">", "🛡", IconMode::Unicode) => Some(">🛡"),
        (NETWORK_RACER_MARKER | "<" | ">", "B", IconMode::Ascii) => Some("[B]"),
        (NETWORK_RACER_MARKER | "<" | ">", "C", IconMode::Ascii) => Some("[C]"),
        (NETWORK_RACER_MARKER | "<" | ">", "I", IconMode::Ascii) => Some("[I]"),
        (NETWORK_RACER_MARKER | "<" | ">", "S", IconMode::Ascii) => Some("[S]"),
        _ => None,
    }
}

fn network_impact_marker_symbol(
    player: &PlayerSnapshot,
    icon_mode: IconMode,
) -> Option<&'static str> {
    let cue = player
        .impact_cue
        .as_ref()
        .filter(|cue| cue.remaining_ms > 0)?;
    match (cue.kind, icon_mode) {
        (ImpactCueSnapshotKind::Banana, IconMode::Unicode) => Some("🍌"),
        (ImpactCueSnapshotKind::Banana, IconMode::Ascii) => Some("B"),
        (ImpactCueSnapshotKind::Cyclone, IconMode::Unicode) => Some("🌀"),
        (ImpactCueSnapshotKind::Cyclone, IconMode::Ascii) => Some("C"),
        (ImpactCueSnapshotKind::Fog, IconMode::Unicode) => Some("🌫"),
        (ImpactCueSnapshotKind::Fog, IconMode::Ascii) => Some("F"),
        (ImpactCueSnapshotKind::ShieldBlock, IconMode::Unicode) => Some("🛡"),
        (ImpactCueSnapshotKind::ShieldBlock, IconMode::Ascii) => Some("S"),
    }
}

fn network_marker_style(player: &PlayerSnapshot, color: Color) -> Style {
    let base = Style::default().fg(color).add_modifier(Modifier::BOLD);
    if let Some(cue) = player.impact_cue.filter(|cue| cue.remaining_ms > 0) {
        match cue.kind {
            ImpactCueSnapshotKind::Banana => base.bg(Color::Yellow).fg(Color::Black),
            ImpactCueSnapshotKind::Cyclone => base.bg(Color::Blue).fg(Color::White),
            ImpactCueSnapshotKind::Fog => base.bg(Color::Gray).fg(Color::Black),
            ImpactCueSnapshotKind::ShieldBlock => base.bg(Color::Cyan).fg(Color::Black),
        }
    } else if player.impact_remaining_ms > 0 {
        base.bg(Color::Yellow).fg(Color::Black)
    } else if player.stunned {
        base.bg(Color::Red).fg(Color::White)
    } else {
        base
    }
}

fn write_network_marker(cells: &mut [NetworkTrackCell], start: usize, marker: &str, style: Style) {
    for (offset, ch) in marker.chars().enumerate() {
        if let Some(cell) = cells.get_mut(start + offset) {
            *cell = NetworkTrackCell { ch, style };
        }
    }
}

fn network_minimap_line<'a>(
    state: &NetworkViewState,
    snapshot: &'a RaceSnapshot,
    width: usize,
) -> Line<'a> {
    let label = "Map  ";
    let label_width = label.chars().count();
    if width <= label_width {
        return Line::from(label.chars().take(width).collect::<String>());
    }

    let map_width = width - label_width;
    if map_width < 2 {
        return Line::from(format!("{label}{}", "|".repeat(map_width)));
    }

    let mut cells = vec![
        NetworkTrackCell {
            ch: '-',
            style: Style::default().fg(Color::DarkGray),
        };
        map_width
    ];
    cells[0].ch = '|';
    cells[map_width - 1].ch = '|';

    let usable_width = map_width.saturating_sub(2);
    for player in &snapshot.players {
        let col = network_minimap_column(snapshot.track_words.len(), usable_width, player);
        let marker = if player.id == state.player_id {
            '@'
        } else if cells[col].ch != '-' && cells[col].ch != '|' {
            '*'
        } else {
            network_player_marker_char(player)
        };
        cells[col] = NetworkTrackCell {
            ch: marker,
            style: Style::default()
                .fg(if player.id == state.player_id {
                    Color::Cyan
                } else {
                    assigned_color(player.color)
                })
                .add_modifier(Modifier::BOLD),
        };
    }

    let mut spans = label
        .chars()
        .map(|ch| Span::raw(ch.to_string()))
        .collect::<Vec<_>>();
    spans.extend(
        cells
            .into_iter()
            .map(|cell| Span::styled(cell.ch.to_string(), cell.style)),
    );
    Line::from(spans)
}

fn network_minimap_column(track_len: usize, usable_width: usize, player: &PlayerSnapshot) -> usize {
    let finish_col = usable_width + 1;
    if player.finished {
        return finish_col;
    }

    if track_len <= 1 {
        return 1;
    }

    let final_word_index = track_len - 1;
    let word_index = player.word_index.min(final_word_index);
    1 + ((word_index * usable_width) + (final_word_index / 2)) / final_word_index
}

fn network_player_marker_char(player: &PlayerSnapshot) -> char {
    char::from_digit((player.id.0 % 10) as u32, 10).unwrap_or('*')
}

fn messages_and_events_view<'a>(
    state: &'a NetworkViewState,
    snapshot: &'a RaceSnapshot,
    area_height: u16,
) -> Paragraph<'a> {
    let max_lines = block_inner_height(area_height);
    let mut lines = snapshot
        .events
        .iter()
        .rev()
        .take(max_lines)
        .map(|event| Line::from(event.as_str()))
        .collect::<Vec<_>>();
    let remaining = max_lines.saturating_sub(lines.len());
    lines.extend(
        state
            .messages
            .iter()
            .rev()
            .take(remaining)
            .map(|message| Line::from(message.as_str())),
    );

    Paragraph::new(lines).block(Block::default().title("Events").borders(Borders::ALL))
}

fn messages_view<'a>(state: &'a NetworkViewState, area_height: u16) -> Paragraph<'a> {
    let max_lines = block_inner_height(area_height);
    let mut lines = state
        .lobby_events
        .iter()
        .rev()
        .take(max_lines)
        .map(|event| Line::from(event.as_str()))
        .collect::<Vec<_>>();
    let remaining = max_lines.saturating_sub(lines.len());
    lines.extend(
        state
            .messages
            .iter()
            .rev()
            .take(remaining)
            .map(|message| Line::from(message.as_str())),
    );
    Paragraph::new(lines).block(Block::default().title("Events").borders(Borders::ALL))
}

fn block_inner_height(area_height: u16) -> usize {
    area_height.saturating_sub(2) as usize
}

fn mod_config_view<'a>(mod_config: Option<&'a ModConfigSnapshot>) -> Paragraph<'a> {
    let lines = if let Some(config) = mod_config {
        vec![
            Line::from(vec![
                Span::styled("Words ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(format!("{} ({})", config.word_set_name, config.word_set_id)),
            ]),
            Line::from(vec![
                Span::styled("Items ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(config.item_pack_name.clone()),
            ]),
            Line::from(vec![
                Span::styled("Mod ", Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(short_hash(&config.combined_hash)),
            ]),
        ]
    } else {
        vec![Line::from("Waiting for host settings")]
    };

    Paragraph::new(lines).block(Block::default().title("Mods").borders(Borders::ALL))
}

fn short_hash(hash: &str) -> String {
    hash.chars().take(8).collect()
}

fn network_footer<'a>(
    state: &NetworkViewState,
    lobby_command: &str,
    rename_input: Option<&str>,
) -> Paragraph<'a> {
    let phase = state.current_phase();
    let line = if let Some(rename_input) = rename_input {
        Line::from(vec![
            Span::raw("Rename: "),
            Span::styled(rename_input.to_string(), Style::default().fg(Color::Yellow)),
            Span::raw("    Enter save | Esc cancel"),
        ])
    } else if state.show_help {
        Line::from("? hide help | Esc/Ctrl-C leaves")
    } else if phase_accepts_typed_commands(state.is_host(), phase) {
        let commands = primary_command_help(state.is_host(), phase);
        Line::from(vec![
            Span::raw("Command: "),
            Span::styled(
                lobby_command.to_string(),
                Style::default().fg(Color::Yellow),
            ),
            Span::raw(commands),
        ])
    } else {
        Line::from(primary_command_help(state.is_host(), phase))
    };

    Paragraph::new(line).block(Block::default().borders(Borders::TOP))
}

fn primary_command_help(is_host: bool, phase: NetworkRacePhase) -> &'static str {
    match phase {
        NetworkRacePhase::Lobby | NetworkRacePhase::WaitingForHost if is_host => {
            "    Space start | N rename | ? help | quit"
        }
        NetworkRacePhase::Lobby | NetworkRacePhase::WaitingForHost => {
            "    Enter ready | N rename | ? help | quit"
        }
        NetworkRacePhase::Finished if is_host => "    Space rematch | Ctrl-R lobby | ? help | quit",
        NetworkRacePhase::Finished => "    ? help | quit",
        NetworkRacePhase::Countdown { .. } if is_host => {
            "Countdown active | Ctrl-R lobby | ? help | Esc/Ctrl-C leaves"
        }
        NetworkRacePhase::Countdown { .. } => "Countdown active | ? help | Esc/Ctrl-C leaves",
        NetworkRacePhase::Racing if is_host => {
            "Type words | Ctrl-R lobby | ? help | Esc/Ctrl-C leaves"
        }
        NetworkRacePhase::Racing => "Type words | Backspace fixes | ? help | Esc/Ctrl-C leaves",
    }
}

fn render_network_help_overlay(
    frame: &mut Frame<'_>,
    area: Rect,
    is_host: bool,
    phase: NetworkRacePhase,
) {
    let overlay = centered_rect(area, 74, 14);
    let lines = network_help_lines(is_host, phase);
    frame.render_widget(Clear, overlay);
    frame.render_widget(
        Paragraph::new(lines).block(Block::default().title("Commands").borders(Borders::ALL)),
        overlay,
    );
}

fn network_help_lines(is_host: bool, phase: NetworkRacePhase) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(vec![
        Span::styled(
            "Key / Command",
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw("       "),
        Span::styled("Action", Style::default().add_modifier(Modifier::BOLD)),
    ])];

    match phase {
        NetworkRacePhase::Lobby | NetworkRacePhase::WaitingForHost if is_host => {
            lines.extend([
                Line::from("Space / start       Start countdown"),
                Line::from("N                   Rename yourself"),
                Line::from("Up / Down           Select lobby racer"),
                Line::from("A                   Add AI racer"),
                Line::from("X                   Remove selected AI or kick human"),
                Line::from("E / H               Set selected AI to Easy / Hard"),
                Line::from("?                   Hide this help"),
                Line::from("Esc / quit          Leave"),
            ]);
        }
        NetworkRacePhase::Lobby | NetworkRacePhase::WaitingForHost => {
            lines.extend([
                Line::from("Enter / ready       Ready up"),
                Line::from("unready             Mark yourself not ready"),
                Line::from("N                   Rename yourself"),
                Line::from("?                   Hide this help"),
                Line::from("Esc / quit          Leave"),
            ]);
        }
        NetworkRacePhase::Finished if is_host => {
            lines.extend([
                Line::from("Space / start       Start rematch countdown"),
                Line::from("Ctrl-R              Return racers to lobby"),
                Line::from("lobby               Return racers to lobby"),
                Line::from("rematch / restart   Return racers to lobby"),
                Line::from("?                   Hide this help"),
                Line::from("Esc / quit          Leave"),
            ]);
        }
        NetworkRacePhase::Finished => {
            lines.extend([
                Line::from("?                   Hide this help"),
                Line::from("Esc / quit          Leave"),
            ]);
        }
        NetworkRacePhase::Countdown { .. } => {
            lines.push(Line::from("Input locked        Wait for countdown"));
            if is_host {
                lines.push(Line::from(
                    "Ctrl-R              Cancel race and return to lobby",
                ));
            }
            lines.extend([
                Line::from("?                   Hide this help"),
                Line::from("Esc / Ctrl-C        Leave"),
            ]);
        }
        NetworkRacePhase::Racing => {
            lines.extend([
                Line::from("Letters             Type current word"),
                Line::from("Space               Submit completed word"),
                Line::from("Backspace           Fix typos"),
            ]);
            if is_host {
                lines.push(Line::from(
                    "Ctrl-R              Cancel race and return to lobby",
                ));
            }
            lines.extend([
                Line::from("?                   Hide this help"),
                Line::from("Esc / Ctrl-C        Leave"),
            ]);
        }
    }

    lines
}

fn centered_rect(area: Rect, max_width: u16, height: u16) -> Rect {
    let width = max_width.min(area.width.saturating_sub(2)).max(20);
    let height = height.min(area.height.saturating_sub(2)).max(6);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

fn connection_style(disconnected: bool) -> Style {
    if disconnected {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(Color::Green)
    }
}

fn assigned_color(color: AssignedColor) -> Color {
    match color {
        AssignedColor::Cyan => Color::Cyan,
        AssignedColor::Red => Color::Red,
        AssignedColor::Green => Color::Green,
        AssignedColor::Blue => Color::Blue,
        AssignedColor::Yellow => Color::Yellow,
        AssignedColor::Magenta => Color::Magenta,
    }
}

fn format_phase(phase: NetworkRacePhase) -> String {
    match phase {
        NetworkRacePhase::Lobby => "lobby".to_string(),
        NetworkRacePhase::WaitingForHost => "waiting for host".to_string(),
        NetworkRacePhase::Countdown { remaining_seconds } => {
            format!("countdown {remaining_seconds}")
        }
        NetworkRacePhase::Racing => "racing".to_string(),
        NetworkRacePhase::Finished => "finished".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AssignedColor, NetworkMarkerPosition, NetworkTrackWindow, NetworkViewState, PlayerId,
        PlayerSnapshot, display_word_number, enter_sets_ready, host_cancel_key,
        join_rejection_message, lifecycle_command_message, network_bonus_column,
        network_bonus_lines, network_help_lines, network_minimap_column, network_racer_label,
        network_racer_line, network_track_word_line, phase_accepts_typed_commands,
        primary_command_help, rename_prefill, space_starts_countdown, starts_rename_mode,
        stream_index_for_word_char, visible_network_bonus_point,
    };
    use crate::net::protocol::{
        BonusChoiceSnapshot, BonusChoiceSnapshotStatus, BonusPointSnapshot, ClientMessage,
        ImpactCueSnapshot, ImpactCueSnapshotKind, ModConfigSnapshot, NetworkRacePhase, PlayerKind,
        RaceSnapshot,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::style::{Color, Modifier};

    #[test]
    fn network_track_window_keeps_current_word_visible() {
        let words = words(["zero", "one", "two", "three", "four"]);
        let window = NetworkTrackWindow::new(&words, 3, 13);

        assert!(window.words.iter().any(|word| word.index == 3));
        assert!(window.words.iter().all(|word| word.end_col <= window.width));
    }

    #[test]
    fn network_track_window_keeps_three_completed_words_behind_player() {
        let words = words([
            "zero", "one", "two", "three", "four", "five", "six", "seven",
        ]);
        let window = NetworkTrackWindow::new(&words, 5, 80);

        assert_eq!(window.words.first().map(|word| word.index), Some(2));
    }

    #[test]
    fn network_marker_tracks_current_character_position() {
        let words = words(["one", "two", "three"]);
        let window = NetworkTrackWindow::new(&words, 1, 20);
        let player = player(PlayerId(1), 1, "tw", None, false);

        assert_eq!(window.column_for_player(&player), 6);
    }

    #[test]
    fn network_marker_pins_to_first_typo() {
        let words = words(["one", "two", "three"]);
        let window = NetworkTrackWindow::new(&words, 1, 20);
        let player = player(PlayerId(1), 1, "txxx", Some(1), false);

        assert_eq!(window.column_for_player(&player), 5);
    }

    #[test]
    fn network_marker_uses_finished_edge_marker_when_finish_is_offscreen() {
        let words = words(["one", "two", "three", "four"]);
        let window = NetworkTrackWindow::new(&words, 1, 7);
        let player = player(PlayerId(2), 3, "", None, true);

        assert_eq!(
            window.marker_for_player(&player),
            NetworkMarkerPosition::FinishedAhead
        );
    }

    #[test]
    fn network_minimap_pins_finished_player_to_finish_edge() {
        let player = player(PlayerId(1), 2, "", None, true);

        assert_eq!(network_minimap_column(4, 10, &player), 11);
    }

    #[test]
    fn network_stream_index_counts_spaces_between_words() {
        let words = words(["one", "two", "three"]);
        let window = NetworkTrackWindow::new(&words, 0, 20);
        let target = window.words.iter().find(|word| word.index == 1).unwrap();

        assert_eq!(stream_index_for_word_char(&window, 0, target, 0), Some(4));
    }

    #[test]
    fn network_word_line_renders_space_position_typos_red() {
        let words = words(["one", "two", "three"]);
        let window = NetworkTrackWindow::new(&words, 0, 20);
        let player = player(PlayerId(1), 0, "onex", Some(3), false);

        let line = network_track_word_line(&window, Some(&player));

        assert_eq!(line.spans[3].content.as_ref(), "x");
        assert_eq!(line.spans[3].style.fg, Some(Color::Red));
        assert!(line.spans[3].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn network_word_line_tints_focused_track_words() {
        let words = words(["one", "two", "three"]);
        let window = NetworkTrackWindow::new(&words, 0, 20);
        let mut player = player(PlayerId(1), 0, "", None, false);
        player.focused = true;

        let line = network_track_word_line(&window, Some(&player));

        assert_eq!(line.spans[0].style.fg, Some(Color::LightMagenta));
        assert!(line.spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(line.spans[4].style.fg, Some(Color::LightMagenta));
    }

    #[test]
    fn network_bonus_point_is_visible_when_gap_words_are_visible() {
        let words = words(["one", "two", "three"]);
        let window = NetworkTrackWindow::new(&words, 0, 20);
        let snapshot = snapshot_with_bonus(0);

        let point = visible_network_bonus_point(&window, &snapshot).unwrap();

        assert_eq!(point.after_word_index, 0);
    }

    #[test]
    fn network_bonus_column_aligns_between_gap_words() {
        let words = words(["one", "two", "three"]);
        let window = NetworkTrackWindow::new(&words, 0, 20);

        assert_eq!(network_bonus_column(&window, 0, 4), 1);
    }

    #[test]
    fn network_bonus_words_are_magenta_only_when_local_player_can_claim_them() {
        let words = words(["one", "two", "three"]);
        let window = NetworkTrackWindow::new(&words, 0, 20);
        let snapshot = snapshot_with_bonus(0);

        let before_gap = player(PlayerId(1), 0, "", None, false);
        let claimable = player(PlayerId(1), 1, "", None, false);
        let bonus_attempt = player(PlayerId(1), 1, "d", None, false);
        let next_word_started = player(PlayerId(1), 1, "t", None, false);
        let after_gap = player(PlayerId(1), 2, "", None, false);

        assert_eq!(
            network_bonus_lines(&window, &snapshot, Some(&claimable))[0].spans[1]
                .style
                .fg,
            Some(Color::Magenta)
        );
        assert_eq!(
            network_bonus_lines(&window, &snapshot, Some(&bonus_attempt))[0].spans[1]
                .style
                .fg,
            Some(Color::Magenta)
        );
        assert_eq!(
            network_bonus_lines(&window, &snapshot, Some(&before_gap))[0].spans[1]
                .style
                .fg,
            Some(Color::DarkGray)
        );
        assert_eq!(
            network_bonus_lines(&window, &snapshot, Some(&next_word_started))[0].spans[1]
                .style
                .fg,
            Some(Color::DarkGray)
        );
        assert_eq!(
            network_bonus_lines(&window, &snapshot, Some(&after_gap))[0].spans[1]
                .style
                .fg,
            Some(Color::DarkGray)
        );
    }

    #[test]
    fn display_word_number_clamps_finished_player_to_track_length() {
        let player = player(PlayerId(1), 3, "", None, true);

        assert_eq!(display_word_number(&player, 3), 3);
    }

    #[test]
    fn network_local_racer_label_shows_countdown() {
        assert_eq!(
            network_racer_label(
                true,
                NetworkRacePhase::Countdown {
                    remaining_seconds: 3
                }
            ),
            Some(" 3".to_string())
        );
        assert_eq!(
            network_racer_label(
                false,
                NetworkRacePhase::Countdown {
                    remaining_seconds: 3
                }
            ),
            None
        );
    }

    #[test]
    fn waiting_snapshot_returns_network_view_to_lobby() {
        let mut state =
            NetworkViewState::new(PlayerId(1), crate::ui::render::IconMode::Ascii, None);
        let mut snapshot = snapshot_with_bonus(0);
        state.apply_race_snapshot(snapshot.clone());
        assert!(state.race_snapshot.is_some());

        snapshot.phase = NetworkRacePhase::WaitingForHost;
        state.apply_race_snapshot(snapshot);

        assert!(state.race_snapshot.is_none());
        assert!(state.placements.is_empty());
        assert!(state.result_rows.is_empty());
    }

    #[test]
    fn lobby_observer_can_hold_race_snapshot_without_being_racer() {
        let mut state =
            NetworkViewState::new(PlayerId(9), crate::ui::render::IconMode::Ascii, None);

        state.apply_race_snapshot(snapshot_with_bonus(0));

        assert!(state.race_snapshot.is_some());
        assert!(!state.is_local_player_in_current_race());
    }

    #[test]
    fn lifecycle_commands_are_phase_aware() {
        assert_eq!(
            lifecycle_command_message("ready", true, NetworkRacePhase::WaitingForHost),
            Some(ClientMessage::SetReady { ready: true })
        );
        assert_eq!(
            lifecycle_command_message("ready", true, NetworkRacePhase::Racing),
            None
        );
        assert_eq!(
            lifecycle_command_message("lobby", true, NetworkRacePhase::Finished),
            Some(ClientMessage::RestartRace)
        );
        assert_eq!(
            lifecycle_command_message("rename speedy", false, NetworkRacePhase::WaitingForHost),
            None
        );
        assert_eq!(
            lifecycle_command_message("lobby", false, NetworkRacePhase::Finished),
            None
        );
    }

    #[test]
    fn n_starts_rename_mode_only_from_lobby_without_typed_command() {
        let key = KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE);

        assert!(starts_rename_mode(
            NetworkRacePhase::WaitingForHost,
            "",
            key
        ));
        assert!(!starts_rename_mode(
            NetworkRacePhase::WaitingForHost,
            "ready",
            key
        ));
        assert!(!starts_rename_mode(NetworkRacePhase::Racing, "", key));
    }

    #[test]
    fn rename_prefill_starts_blank_for_default_anonymous_names() {
        assert_eq!(rename_prefill(Some("anonymous")), "");
        assert_eq!(rename_prefill(Some("anonymous2")), "");
        assert_eq!(rename_prefill(Some("anonymous42")), "");
        assert_eq!(rename_prefill(Some("anonymous-racer")), "anonymous-racer");
        assert_eq!(rename_prefill(Some("tom")), "tom");
    }

    #[test]
    fn ctrl_r_is_host_only_cancel_during_active_network_races() {
        let key = KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL);

        assert!(host_cancel_key(
            true,
            NetworkRacePhase::Countdown {
                remaining_seconds: 2
            },
            key
        ));
        assert!(host_cancel_key(true, NetworkRacePhase::Racing, key));
        assert!(host_cancel_key(true, NetworkRacePhase::Finished, key));
        assert!(!host_cancel_key(false, NetworkRacePhase::Racing, key));
        assert!(!host_cancel_key(
            true,
            NetworkRacePhase::WaitingForHost,
            key
        ));
    }

    #[test]
    fn lifecycle_help_hides_irrelevant_commands() {
        assert!(primary_command_help(true, NetworkRacePhase::WaitingForHost).contains("Space"));
        assert!(primary_command_help(false, NetworkRacePhase::WaitingForHost).contains("Enter"));
        assert!(!primary_command_help(true, NetworkRacePhase::Finished).contains("ready"));
        let finished_host_help =
            format!("{:?}", network_help_lines(true, NetworkRacePhase::Finished));
        assert!(finished_host_help.contains("lobby"));
        assert_eq!(
            primary_command_help(false, NetworkRacePhase::Finished),
            "    ? help | quit"
        );
        assert!(!phase_accepts_typed_commands(
            true,
            NetworkRacePhase::Countdown {
                remaining_seconds: 2
            }
        ));
        assert!(space_starts_countdown(true, NetworkRacePhase::Finished));
        assert!(phase_accepts_typed_commands(
            false,
            NetworkRacePhase::Finished
        ));
        assert!(enter_sets_ready(true, NetworkRacePhase::WaitingForHost, ""));
        assert!(!enter_sets_ready(
            true,
            NetworkRacePhase::WaitingForHost,
            "quit"
        ));
        assert!(!enter_sets_ready(
            false,
            NetworkRacePhase::WaitingForHost,
            ""
        ));
    }

    #[test]
    fn join_rejection_messages_add_user_guidance() {
        assert_eq!(
            join_rejection_message("Name 'tom' is already in use"),
            "Name 'tom' is already in use. Choose a different --name."
        );
        assert!(
            join_rejection_message("Lobby is full: 6/6 connected players")
                .contains("ask the host to remove a racer")
        );
        assert!(
            join_rejection_message("Room vivid-grape-lemon was not found")
                .contains("make sure the host is still online")
        );
        assert!(
            join_rejection_message(
                "Version mismatch: this room is running TypeKart 0.1.0, but you are running TypeKart 0.2.0. Install or launch the same TypeKart version as the room host."
            )
            .contains("room is running TypeKart 0.1.0")
        );
    }

    #[test]
    fn network_racer_marker_shows_banana_impact_icon() {
        let words = words(["one", "two", "three"]);
        let window = NetworkTrackWindow::new(&words, 0, 40);
        let mut racer = player(PlayerId(1), 0, "", None, false);
        racer.impact_cue = Some(ImpactCueSnapshot {
            kind: ImpactCueSnapshotKind::Banana,
            remaining_ms: 500,
        });

        let line = network_racer_line(
            &window,
            &racer,
            true,
            NetworkRacePhase::Racing,
            super::IconMode::Unicode,
        );

        assert_eq!(line.spans[0].content.as_ref(), "█");
        assert_eq!(line.spans[1].content.as_ref(), "🍌");
    }

    #[test]
    fn network_racer_marker_shows_cyclone_impact_icon() {
        let words = words(["one", "two", "three"]);
        let window = NetworkTrackWindow::new(&words, 0, 40);
        let mut racer = player(PlayerId(1), 0, "", None, false);
        racer.impact_cue = Some(ImpactCueSnapshot {
            kind: ImpactCueSnapshotKind::Cyclone,
            remaining_ms: 500,
        });

        let line = network_racer_line(
            &window,
            &racer,
            true,
            NetworkRacePhase::Racing,
            super::IconMode::Unicode,
        );

        assert_eq!(line.spans[0].content.as_ref(), "█");
        assert_eq!(line.spans[1].content.as_ref(), "🌀");
    }

    fn words<const N: usize>(words: [&str; N]) -> Vec<String> {
        words.into_iter().map(str::to_string).collect()
    }

    fn player(
        id: PlayerId,
        word_index: usize,
        input: &str,
        typo_index: Option<usize>,
        finished: bool,
    ) -> PlayerSnapshot {
        PlayerSnapshot {
            id,
            name: format!("player-{}", id.0),
            kind: PlayerKind::Human,
            color: AssignedColor::Cyan,
            word_index,
            input: input.to_string(),
            typo_index,
            word_overrides: Vec::new(),
            finished,
            connected: true,
            shielded: false,
            focused: false,
            fogged: false,
            boosted: false,
            stunned: false,
            impact_remaining_ms: 0,
            impact_cue: None,
            item_cue: None,
        }
    }

    fn snapshot_with_bonus(after_word_index: usize) -> RaceSnapshot {
        RaceSnapshot {
            sequence: 1,
            phase: NetworkRacePhase::Racing,
            mod_config: test_mod_config(),
            track_words: words(["one", "two", "three"]),
            bonuses: vec![BonusPointSnapshot {
                after_word_index,
                choices: vec![BonusChoiceSnapshot {
                    word: "dash".to_string(),
                    status: BonusChoiceSnapshotStatus::Available,
                }],
            }],
            players: Vec::new(),
            events: Vec::new(),
        }
    }

    fn test_mod_config() -> ModConfigSnapshot {
        ModConfigSnapshot {
            word_set_id: "classic".to_string(),
            word_set_name: "Classic".to_string(),
            word_set_hash: "0000000000000001".to_string(),
            item_pack_name: "classic".to_string(),
            item_registry_hash: "0000000000000002".to_string(),
            combined_hash: "0000000000000003".to_string(),
        }
    }
}
