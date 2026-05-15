//! Minimal TCP join client for Milestone 4.
//!
//! The current client performs the first protocol handshake and prints lobby
//! snapshots. Later slices will send key input and render race snapshots.

use std::{
    collections::VecDeque,
    io::{self, BufRead, BufReader, Write},
    net::SocketAddr,
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
    widgets::{Block, Borders, Paragraph},
};

use super::protocol::{
    AssignedColor, ClientMessage, ClientSequence, LobbyPlayer, NetworkRacePhase, PlayerId,
    PlayerSnapshot, ProtocolKey, RaceSnapshot, ServerMessage, decode_server_message,
    encode_client_message,
};

#[derive(Debug, Clone)]
pub struct JoinConfig {
    pub server: SocketAddr,
    pub name: String,
}

pub fn run_join(config: JoinConfig) -> Result<()> {
    let mut stream = std::net::TcpStream::connect(config.server)
        .with_context(|| format!("failed to connect to {}", config.server))?;
    let hello = ClientMessage::Hello {
        name: config.name,
        client_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    let encoded = encode_client_message(&hello).context("failed to encode hello message")?;
    writeln!(stream, "{encoded}").context("failed to send hello message")?;
    stream.flush().context("failed to flush hello message")?;

    let read_stream = stream
        .try_clone()
        .context("failed to clone server stream for reading")?;
    let mut line = String::new();
    let mut reader = BufReader::new(read_stream);
    reader
        .read_line(&mut line)
        .context("failed to read server welcome")?;

    let player_id =
        match decode_server_message(line.trim_end()).context("failed to decode server response")? {
            ServerMessage::Welcome {
                player_id,
                assigned_color,
            } => {
                println!(
                    "Joined TypeKart server as player {} ({assigned_color:?})",
                    player_id.0
                );
                player_id
            }
            ServerMessage::Error { message } => {
                println!("Server rejected join: {message}");
                return Ok(());
            }
            other => {
                println!("Unexpected server response: {other:?}");
                return Ok(());
            }
        };

    println!("Lobby commands: ready, unready, quit");
    println!("After the race starts, typing is sent one key at a time.");

    let phase = Arc::new(Mutex::new(NetworkRacePhase::WaitingForHost));
    let reader_phase = Arc::clone(&phase);
    let view_state = Arc::new(Mutex::new(NetworkViewState::new(player_id)));
    let reader_view_state = Arc::clone(&view_state);

    thread::spawn(move || {
        for line in reader.lines() {
            let Ok(line) = line else {
                break;
            };
            match decode_server_message(line.trim_end()) {
                Ok(ServerMessage::LobbySnapshot { players, host_id }) => {
                    let mut state = reader_view_state.lock().expect("client view poisoned");
                    state.lobby_players = players;
                    state.host_id = Some(host_id);
                }
                Ok(ServerMessage::RaceEvent { message }) => {
                    reader_view_state
                        .lock()
                        .expect("client view poisoned")
                        .push_message(message);
                }
                Ok(ServerMessage::RaceSnapshot(snapshot)) => {
                    *reader_phase.lock().expect("client phase poisoned") = snapshot.phase;
                    reader_view_state
                        .lock()
                        .expect("client view poisoned")
                        .race_snapshot = Some(snapshot);
                }
                Ok(ServerMessage::Error { message }) => {
                    reader_view_state
                        .lock()
                        .expect("client view poisoned")
                        .push_message(format!("Server error: {message}"));
                }
                Ok(ServerMessage::RaceResults { placements }) => {
                    let mut state = reader_view_state.lock().expect("client view poisoned");
                    state.placements = placements;
                    state.push_message("Race results received".to_string());
                }
                Ok(other) => {
                    reader_view_state
                        .lock()
                        .expect("client view poisoned")
                        .push_message(format!("Received: {other:?}"));
                }
                Err(error) => {
                    reader_view_state
                        .lock()
                        .expect("client view poisoned")
                        .push_message(format!("Failed to decode server message: {error}"));
                }
            }
        }

        let mut state = reader_view_state.lock().expect("client view poisoned");
        state.disconnected = true;
        state.push_message("Disconnected from server".to_string());
    });

    let mut terminal = NetworkTerminal::setup()?;
    let mut sequence = 1;
    let mut lobby_command = String::new();

    loop {
        let state = view_state.lock().expect("client view poisoned").clone();
        terminal.draw(&state, &lobby_command)?;

        if !event::poll(Duration::from_millis(50)).context("failed to poll terminal input")? {
            continue;
        }

        let Event::Key(key_event) = event::read().context("failed to read terminal input")? else {
            continue;
        };

        if key_event.kind != KeyEventKind::Press {
            continue;
        }

        if should_leave(key_event) {
            send_client_message(&mut stream, &ClientMessage::Leave)?;
            break;
        }

        if *phase.lock().expect("client phase poisoned") == NetworkRacePhase::Racing {
            if send_race_key(&mut stream, key_event, &mut sequence)? {
                break;
            }
            continue;
        }

        if handle_lobby_key(&mut stream, key_event, &mut lobby_command, state.is_host())? {
            break;
        }
    }

    terminal.restore()?;
    println!("Left server");

    Ok(())
}

#[derive(Debug, Clone)]
struct NetworkViewState {
    player_id: PlayerId,
    lobby_players: Vec<LobbyPlayer>,
    host_id: Option<PlayerId>,
    race_snapshot: Option<RaceSnapshot>,
    placements: Vec<PlayerId>,
    messages: VecDeque<String>,
    disconnected: bool,
}

impl NetworkViewState {
    fn new(player_id: PlayerId) -> Self {
        Self {
            player_id,
            lobby_players: Vec::new(),
            host_id: None,
            race_snapshot: None,
            placements: Vec::new(),
            messages: VecDeque::new(),
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

    fn draw(&mut self, state: &NetworkViewState, lobby_command: &str) -> Result<()> {
        self.terminal
            .draw(|frame| render_network(frame, state, lobby_command))
            .context("failed to draw network screen")?;
        Ok(())
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

fn handle_lobby_key(
    stream: &mut std::net::TcpStream,
    key_event: KeyEvent,
    lobby_command: &mut String,
    is_host: bool,
) -> Result<bool> {
    if is_host && lobby_command.is_empty() && key_event.code == KeyCode::Char(' ') {
        send_client_message(stream, &ClientMessage::StartCountdown)?;
        return Ok(false);
    }

    if is_enter_key(key_event) {
        let should_leave = send_lobby_command(stream, lobby_command.trim(), is_host)?;
        lobby_command.clear();
        return Ok(should_leave);
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

fn is_enter_key(key_event: KeyEvent) -> bool {
    matches!(
        key_event.code,
        KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r')
    ) || (matches!(key_event.code, KeyCode::Char('j') | KeyCode::Char('m'))
        && key_event.modifiers.contains(KeyModifiers::CONTROL))
}

fn send_lobby_command(
    stream: &mut std::net::TcpStream,
    command: &str,
    is_host: bool,
) -> Result<bool> {
    let message = match command {
        "ready" => ClientMessage::SetReady { ready: true },
        "unready" => ClientMessage::SetReady { ready: false },
        "start" if is_host => ClientMessage::StartCountdown,
        "quit" | "leave" => ClientMessage::Leave,
        "" => return Ok(false),
        _ => return Ok(false),
    };

    send_client_message(stream, &message)?;

    Ok(matches!(message, ClientMessage::Leave))
}

fn send_race_key(
    stream: &mut std::net::TcpStream,
    key_event: KeyEvent,
    sequence: &mut u64,
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
    *sequence += 1;

    Ok(false)
}

fn send_client_message(stream: &mut std::net::TcpStream, message: &ClientMessage) -> Result<()> {
    let encoded = encode_client_message(message).context("failed to encode client message")?;
    writeln!(stream, "{encoded}").context("failed to send client message")?;
    stream.flush().context("failed to flush client message")
}

fn render_network(frame: &mut Frame<'_>, state: &NetworkViewState, lobby_command: &str) {
    let area = frame.size();
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
    frame.render_widget(network_footer(state, lobby_command), rows[2]);
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

    Paragraph::new(Line::from(vec![
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
    ]))
    .block(Block::default().borders(Borders::BOTTOM))
}

fn render_lobby(frame: &mut Frame<'_>, area: Rect, state: &NetworkViewState) {
    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(6)])
        .split(area);

    let players = state
        .lobby_players
        .iter()
        .map(|player| {
            Line::from(vec![
                Span::styled("● ", Style::default().fg(assigned_color(player.color))),
                Span::styled(
                    player.name.clone(),
                    Style::default()
                        .fg(assigned_color(player.color))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!(
                    "  {}{}{}",
                    if player.ready { "ready" } else { "not ready" },
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
    frame.render_widget(messages_view(state), body[1]);
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
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(columns[1]);

    frame.render_widget(track_view(state, snapshot), columns[0]);
    frame.render_widget(player_list_view(state, snapshot), right[0]);
    frame.render_widget(messages_and_events_view(state, snapshot), right[1]);
}

fn track_view<'a>(state: &NetworkViewState, snapshot: &'a RaceSnapshot) -> Paragraph<'a> {
    let local = snapshot
        .players
        .iter()
        .find(|player| player.id == state.player_id);
    let current_word_index = local.map(|player| player.word_index).unwrap_or(0);
    let input = local.map(|player| player.input.as_str()).unwrap_or("");

    let mut spans = Vec::new();
    for (index, word) in snapshot.track_words.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw(" "));
        }

        let style = if Some(index) == local.map(|player| player.word_index) {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else if index < current_word_index {
            Style::default().fg(Color::DarkGray)
        } else {
            Style::default()
        };
        spans.push(Span::styled(word.as_str(), style));
    }

    let mut lines = vec![Line::from(spans), Line::from("")];
    if let Some(local) = local {
        lines.push(Line::from(vec![
            Span::raw("Target: "),
            Span::styled(
                snapshot
                    .track_words
                    .get(local.word_index)
                    .map(String::as_str)
                    .unwrap_or("<finished>"),
                Style::default().fg(Color::Yellow),
            ),
            Span::raw("    Input: "),
            Span::styled(input.to_string(), input_style(local)),
        ]));
    }

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
                    player.word_index + 1,
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

fn messages_and_events_view<'a>(
    state: &'a NetworkViewState,
    snapshot: &'a RaceSnapshot,
) -> Paragraph<'a> {
    let mut lines = snapshot
        .events
        .iter()
        .rev()
        .take(5)
        .map(|event| Line::from(event.as_str()))
        .collect::<Vec<_>>();
    lines.extend(
        state
            .messages
            .iter()
            .rev()
            .take(4)
            .map(|message| Line::from(message.as_str())),
    );

    Paragraph::new(lines).block(Block::default().title("Events").borders(Borders::ALL))
}

fn messages_view<'a>(state: &'a NetworkViewState) -> Paragraph<'a> {
    let lines = state
        .messages
        .iter()
        .rev()
        .map(|message| Line::from(message.as_str()))
        .collect::<Vec<_>>();
    Paragraph::new(lines).block(Block::default().title("Events").borders(Borders::ALL))
}

fn network_footer<'a>(state: &NetworkViewState, lobby_command: &str) -> Paragraph<'a> {
    let text = match state
        .race_snapshot
        .as_ref()
        .map(|snapshot| snapshot.phase)
        .unwrap_or(NetworkRacePhase::WaitingForHost)
    {
        NetworkRacePhase::Racing => "Type letters, Space, and Backspace. Esc/Ctrl-C leaves.",
        _ => "",
    };
    let line = if text.is_empty() {
        let commands = if state.is_host() {
            "    ready | unready | start | Space starts | quit"
        } else {
            "    ready | unready | quit"
        };
        Line::from(vec![
            Span::raw("Command: "),
            Span::styled(
                lobby_command.to_string(),
                Style::default().fg(Color::Yellow),
            ),
            Span::raw(commands),
        ])
    } else {
        Line::from(text)
    };

    Paragraph::new(line).block(Block::default().borders(Borders::TOP))
}

fn input_style(player: &PlayerSnapshot) -> Style {
    match player.typo_index {
        Some(_) => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        None => Style::default().fg(Color::Green),
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
