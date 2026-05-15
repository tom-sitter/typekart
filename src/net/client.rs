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

const NETWORK_RACER_MARKER: &str = "███";
const NETWORK_FINISHED_EDGE_MARKER: &str = ">!";

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

    frame.render_widget(track_view(state, snapshot, columns[0].width), columns[0]);
    frame.render_widget(player_list_view(state, snapshot), right[0]);
    frame.render_widget(messages_and_events_view(state, snapshot), right[1]);
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
    let current_word_index = local.map(|player| player.word_index).unwrap_or(0);
    let window = NetworkTrackWindow::new(&snapshot.track_words, current_word_index, width);
    let mut lines = vec![network_track_word_line(&window, local)];
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
        let mut first_index = current_index;
        let mut used_width = words[current_index].chars().count().min(width);

        while first_index > 0 {
            let candidate_width = words[first_index - 1].chars().count() + 1;
            if used_width + candidate_width > width {
                break;
            }
            first_index -= 1;
            used_width += candidate_width;
        }

        let mut visible = Vec::new();
        let mut col = 0;
        for (index, word) in words.iter().enumerate().skip(first_index) {
            let word_width = word.chars().count();
            let separator_width = usize::from(!visible.is_empty());
            if !visible.is_empty() && col + separator_width + word_width > width {
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
    let mut spans = Vec::new();

    for word in &window.words {
        if word.start_col > spans_width(&spans) {
            spans.push(Span::raw(" ".repeat(word.start_col - spans_width(&spans))));
        }

        for (char_index, ch) in word.word.chars().enumerate() {
            spans.push(Span::styled(
                ch.to_string(),
                network_word_char_style(window, word, char_index, local),
            ));
        }
    }

    Line::from(spans)
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
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
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

fn spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|span| span.content.chars().count()).sum()
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
        .map(|player| network_racer_line(window, player, player.id == state.player_id))
        .collect()
}

fn network_racer_line<'a>(
    window: &NetworkTrackWindow<'_>,
    player: &PlayerSnapshot,
    is_local: bool,
) -> Line<'a> {
    let mut cells = vec![NetworkTrackCell::default(); window.width];
    let color = assigned_color(player.color);
    let style = Style::default().fg(color).add_modifier(Modifier::BOLD);

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

    write_network_marker(&mut cells, start, marker, style);
    let label = if is_local { " you" } else { "" };
    if !label.is_empty() {
        write_network_marker(&mut cells, start + marker.chars().count(), label, style);
    }

    Line::from(
        cells
            .into_iter()
            .map(|cell| Span::styled(cell.ch.to_string(), cell.style))
            .collect::<Vec<_>>(),
    )
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
        AssignedColor, NetworkMarkerPosition, NetworkTrackWindow, PlayerId, PlayerSnapshot,
        display_word_number, network_minimap_column, stream_index_for_word_char,
    };

    #[test]
    fn network_track_window_keeps_current_word_visible() {
        let words = words(["zero", "one", "two", "three", "four"]);
        let window = NetworkTrackWindow::new(&words, 3, 13);

        assert!(window.words.iter().any(|word| word.index == 3));
        assert!(window.words.iter().all(|word| word.end_col <= window.width));
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
    fn display_word_number_clamps_finished_player_to_track_length() {
        let player = player(PlayerId(1), 3, "", None, true);

        assert_eq!(display_word_number(&player, 3), 3);
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
            color: AssignedColor::Cyan,
            word_index,
            input: input.to_string(),
            typo_index,
            finished,
            connected: true,
        }
    }
}
