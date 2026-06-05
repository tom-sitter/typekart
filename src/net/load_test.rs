//! Relay load testing tools.
//!
//! This module deliberately exercises the relay protocol directly rather than
//! driving the terminal UI. The result is a focused capacity test for room
//! creation, participant joins, client input forwarding, and host broadcasts.

use std::{
    io::ErrorKind,
    net::TcpStream,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use tungstenite::{Error as WebSocketError, Message, WebSocket, connect, stream::MaybeTlsStream};

use super::{
    protocol::{
        AssignedColor, ClientMessage, ClientSequence, NetworkRacePhase, PlayerId, PlayerKind,
        PlayerSnapshot, ProtocolKey, RaceDeltaSnapshot, ServerMessage,
    },
    relay::{RelayClientMessage, RelayServerMessage, RoomCode},
};

type RelaySocket = WebSocket<MaybeTlsStream<TcpStream>>;

#[derive(Debug, Clone)]
pub struct RelayLoadTestConfig {
    pub relay: String,
    pub start_games: usize,
    pub max_games: usize,
    pub step_games: usize,
    pub joiners_per_game: usize,
    pub duration: Duration,
    pub snapshot_interval: Duration,
    pub input_interval: Duration,
    pub settle_timeout: Duration,
    pub host_start_stagger: Duration,
    pub joiner_start_stagger: Duration,
    pub failure_samples: usize,
}

#[derive(Debug, Clone, Default)]
struct LoadMetrics {
    rooms_created: Arc<AtomicU64>,
    joiners_connected: Arc<AtomicU64>,
    host_messages_received: Arc<AtomicU64>,
    joiner_messages_received: Arc<AtomicU64>,
    broadcasts_sent: Arc<AtomicU64>,
    inputs_sent: Arc<AtomicU64>,
    errors: Arc<AtomicU64>,
    host_errors: Arc<AtomicU64>,
    joiner_errors: Arc<AtomicU64>,
    failure_samples: Arc<Mutex<Vec<String>>>,
}

#[derive(Debug, Clone)]
struct LoadRunSummary {
    games: usize,
    joiners_per_game: usize,
    elapsed: Duration,
    rooms_created: u64,
    joiners_connected: u64,
    host_messages_received: u64,
    joiner_messages_received: u64,
    broadcasts_sent: u64,
    inputs_sent: u64,
    errors: u64,
    host_errors: u64,
    joiner_errors: u64,
    failure_samples: Vec<String>,
}

impl LoadMetrics {
    fn summary(&self, games: usize, joiners_per_game: usize, elapsed: Duration) -> LoadRunSummary {
        LoadRunSummary {
            games,
            joiners_per_game,
            elapsed,
            rooms_created: self.rooms_created.load(Ordering::Relaxed),
            joiners_connected: self.joiners_connected.load(Ordering::Relaxed),
            host_messages_received: self.host_messages_received.load(Ordering::Relaxed),
            joiner_messages_received: self.joiner_messages_received.load(Ordering::Relaxed),
            broadcasts_sent: self.broadcasts_sent.load(Ordering::Relaxed),
            inputs_sent: self.inputs_sent.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
            host_errors: self.host_errors.load(Ordering::Relaxed),
            joiner_errors: self.joiner_errors.load(Ordering::Relaxed),
            failure_samples: self
                .failure_samples
                .lock()
                .expect("load failure samples poisoned")
                .clone(),
        }
    }

    fn record_host_error(&self, game_index: usize, error: anyhow::Error, sample_limit: usize) {
        self.errors.fetch_add(1, Ordering::Relaxed);
        self.host_errors.fetch_add(1, Ordering::Relaxed);
        self.record_failure_sample(format!("host game={game_index}: {error:#}"), sample_limit);
    }

    fn record_joiner_error(
        &self,
        game_index: usize,
        joiner_index: usize,
        room: &RoomCode,
        error: anyhow::Error,
        sample_limit: usize,
    ) {
        self.errors.fetch_add(1, Ordering::Relaxed);
        self.joiner_errors.fetch_add(1, Ordering::Relaxed);
        self.record_failure_sample(
            format!(
                "joiner game={game_index} joiner={joiner_index} room={}: {error:#}",
                room.display()
            ),
            sample_limit,
        );
    }

    fn record_failure_sample(&self, sample: String, sample_limit: usize) {
        if sample_limit == 0 {
            return;
        }
        let mut samples = self
            .failure_samples
            .lock()
            .expect("load failure samples poisoned");
        if samples.len() < sample_limit {
            samples.push(sample);
        }
    }
}

impl LoadRunSummary {
    fn expected_joiners(&self) -> u64 {
        (self.games * self.joiners_per_game) as u64
    }

    fn passed(&self) -> bool {
        self.rooms_created == self.games as u64
            && self.joiners_connected == self.expected_joiners()
            && self.errors == 0
    }

    fn print(&self) {
        let seconds = self.elapsed.as_secs_f64().max(0.001);
        println!(
            "{:>4} games | {:>4} sockets | rooms {:>4}/{:<4} | joiners {:>4}/{:<4} | broadcasts {:>7} ({:>6.1}/s) | inputs {:>7} ({:>6.1}/s) | received {:>7} | errors {:>3} | {}",
            self.games,
            self.games * (self.joiners_per_game + 1),
            self.rooms_created,
            self.games,
            self.joiners_connected,
            self.expected_joiners(),
            self.broadcasts_sent,
            self.broadcasts_sent as f64 / seconds,
            self.inputs_sent,
            self.inputs_sent as f64 / seconds,
            self.host_messages_received + self.joiner_messages_received,
            self.errors,
            if self.passed() { "ok" } else { "failed" }
        );
        if self.errors > 0 {
            println!(
                "     failures: hosts={} joiners={} samples={}",
                self.host_errors,
                self.joiner_errors,
                self.failure_samples.len()
            );
            for sample in &self.failure_samples {
                println!("       - {sample}");
            }
        }
    }
}

pub fn run_relay_load_test(config: RelayLoadTestConfig) -> Result<()> {
    if config.start_games == 0 {
        bail!("--start-games must be greater than zero");
    }
    if config.max_games < config.start_games {
        bail!("--max-games must be greater than or equal to --start-games");
    }
    if config.step_games == 0 {
        bail!("--step-games must be greater than zero");
    }
    if config.joiners_per_game == 0 {
        bail!("--joiners-per-game must be greater than zero");
    }

    println!(
        "Relay load test: relay={} joiners_per_game={} duration={}s snapshot_ms={} input_ms={}",
        config.relay,
        config.joiners_per_game,
        config.duration.as_secs(),
        config.snapshot_interval.as_millis(),
        config.input_interval.as_millis()
    );
    println!(
        "Each game uses {} relay sockets: one host plus {} joiners.",
        config.joiners_per_game + 1,
        config.joiners_per_game
    );
    if !config.host_start_stagger.is_zero() || !config.joiner_start_stagger.is_zero() {
        println!(
            "Staggering starts: host_start_ms={} joiner_start_ms={}",
            config.host_start_stagger.as_millis(),
            config.joiner_start_stagger.as_millis()
        );
    }

    let mut last_passed = None;
    let mut games = config.start_games;
    while games <= config.max_games {
        let summary = run_load_step(&config, games)?;
        summary.print();
        if summary.passed() {
            last_passed = Some(summary);
        } else {
            println!("Stopping after first failed step.");
            break;
        }
        games += config.step_games;
    }

    match last_passed {
        Some(summary) => {
            println!(
                "Estimated supported load: at least {} concurrent games ({} sockets) with this test profile.",
                summary.games,
                summary.games * (summary.joiners_per_game + 1)
            );
            Ok(())
        }
        None => bail!("load test did not pass the first step"),
    }
}

fn run_load_step(config: &RelayLoadTestConfig, games: usize) -> Result<LoadRunSummary> {
    let metrics = LoadMetrics::default();
    let stop_at = Instant::now() + config.duration;
    let (room_tx, room_rx) = mpsc::channel::<(usize, RoomCode)>();
    let mut handles = Vec::with_capacity(games * (config.joiners_per_game + 1));
    let started_at = Instant::now();

    for game_index in 0..games {
        let config = config.clone();
        let metrics = metrics.clone();
        let room_tx = room_tx.clone();
        handles.push(thread::spawn(move || {
            if !config.host_start_stagger.is_zero() {
                thread::sleep(config.host_start_stagger * game_index as u32);
            }
            let sample_limit = config.failure_samples;
            if let Err(error) =
                run_host_load_connection(game_index, config, metrics.clone(), room_tx, stop_at)
            {
                metrics.record_host_error(game_index, error, sample_limit);
            }
        }));
    }
    drop(room_tx);

    let mut rooms = Vec::with_capacity(games);
    let room_deadline = Instant::now() + config.settle_timeout;
    while rooms.len() < games && Instant::now() < room_deadline {
        match room_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(room) => rooms.push(room),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    for (game_index, room) in rooms {
        for joiner_index in 0..config.joiners_per_game {
            let config = config.clone();
            let metrics = metrics.clone();
            let room = room.clone();
            handles.push(thread::spawn(move || {
                if !config.joiner_start_stagger.is_zero() {
                    thread::sleep(config.joiner_start_stagger * joiner_index as u32);
                }
                let sample_limit = config.failure_samples;
                if let Err(error) = run_joiner_load_connection(
                    game_index,
                    joiner_index,
                    room.clone(),
                    config,
                    metrics.clone(),
                    stop_at,
                ) {
                    metrics.record_joiner_error(
                        game_index,
                        joiner_index,
                        &room,
                        error,
                        sample_limit,
                    );
                }
            }));
        }
    }

    for handle in handles {
        let _ = handle.join();
    }

    Ok(metrics.summary(games, config.joiners_per_game, started_at.elapsed()))
}

fn run_host_load_connection(
    game_index: usize,
    config: RelayLoadTestConfig,
    metrics: LoadMetrics,
    room_tx: mpsc::Sender<(usize, RoomCode)>,
    stop_at: Instant,
) -> Result<()> {
    let mut websocket = connect_load_socket(&config.relay)?;
    send_relay_message(
        &mut websocket,
        &RelayClientMessage::CreateRoom {
            host_version: env!("CARGO_PKG_VERSION").to_string(),
        },
    )?;
    let room = wait_for_room_created(&mut websocket)?;
    metrics.rooms_created.fetch_add(1, Ordering::Relaxed);
    room_tx
        .send((game_index, room.clone()))
        .context("failed to publish load-test room")?;
    set_load_socket_nonblocking(&mut websocket)?;

    let mut next_broadcast = Instant::now() + config.snapshot_interval;
    let mut sequence = 0;
    while Instant::now() < stop_at {
        drain_host_messages(&mut websocket, &metrics)?;
        if Instant::now() >= next_broadcast {
            sequence += 1;
            send_relay_message(
                &mut websocket,
                &RelayClientMessage::HostBroadcast {
                    room: room.clone(),
                    message: serde_json::to_value(ServerMessage::RaceDelta(load_delta(sequence)))
                        .context("failed to encode load-test race delta")?,
                },
            )?;
            metrics.broadcasts_sent.fetch_add(1, Ordering::Relaxed);
            next_broadcast += config.snapshot_interval;
        }
        thread::sleep(Duration::from_millis(2));
    }

    Ok(())
}

fn run_joiner_load_connection(
    game_index: usize,
    joiner_index: usize,
    room: RoomCode,
    config: RelayLoadTestConfig,
    metrics: LoadMetrics,
    stop_at: Instant,
) -> Result<()> {
    let mut websocket = connect_load_socket(&relay_join_url(&config.relay, &room))?;
    send_relay_message(
        &mut websocket,
        &RelayClientMessage::JoinRoom {
            room: room.clone(),
            name: format!("load-{game_index}-{joiner_index}"),
            client_version: env!("CARGO_PKG_VERSION").to_string(),
        },
    )?;
    metrics.joiners_connected.fetch_add(1, Ordering::Relaxed);
    set_load_socket_nonblocking(&mut websocket)?;

    let player_id = PlayerId((joiner_index + 2) as u64);
    let mut next_input = Instant::now() + config.input_interval;
    let mut sequence = 0;
    while Instant::now() < stop_at {
        drain_joiner_messages(&mut websocket, &metrics)?;
        if Instant::now() >= next_input {
            sequence += 1;
            send_relay_message(
                &mut websocket,
                &RelayClientMessage::ClientToHost {
                    room: room.clone(),
                    player_id,
                    message: serde_json::to_value(ClientMessage::KeyInput {
                        sequence: ClientSequence(sequence),
                        key: ProtocolKey::Char('a'),
                    })
                    .context("failed to encode load-test key input")?,
                },
            )?;
            metrics.inputs_sent.fetch_add(1, Ordering::Relaxed);
            next_input += config.input_interval;
        }
        thread::sleep(Duration::from_millis(2));
    }

    Ok(())
}

fn drain_host_messages(websocket: &mut RelaySocket, metrics: &LoadMetrics) -> Result<()> {
    loop {
        match websocket.read() {
            Ok(Message::Text(text)) => {
                let _message: RelayServerMessage =
                    serde_json::from_str(&text).context("failed to decode relay host message")?;
                metrics
                    .host_messages_received
                    .fetch_add(1, Ordering::Relaxed);
            }
            Ok(Message::Ping(payload)) => websocket.send(Message::Pong(payload))?,
            Ok(Message::Close(_)) | Err(WebSocketError::ConnectionClosed) => break,
            Ok(_) => {}
            Err(WebSocketError::Io(error)) if error.kind() == ErrorKind::WouldBlock => break,
            Err(error) if load_disconnect_error(&error) => break,
            Err(error) => return Err(error).context("failed to read relay host message"),
        }
    }
    Ok(())
}

fn drain_joiner_messages(websocket: &mut RelaySocket, metrics: &LoadMetrics) -> Result<()> {
    loop {
        match websocket.read() {
            Ok(Message::Text(text)) => {
                let _message: RelayServerMessage =
                    serde_json::from_str(&text).context("failed to decode relay joiner message")?;
                metrics
                    .joiner_messages_received
                    .fetch_add(1, Ordering::Relaxed);
            }
            Ok(Message::Ping(payload)) => websocket.send(Message::Pong(payload))?,
            Ok(Message::Close(_)) | Err(WebSocketError::ConnectionClosed) => break,
            Ok(_) => {}
            Err(WebSocketError::Io(error)) if error.kind() == ErrorKind::WouldBlock => break,
            Err(error) if load_disconnect_error(&error) => break,
            Err(error) => return Err(error).context("failed to read relay joiner message"),
        }
    }
    Ok(())
}

fn wait_for_room_created(websocket: &mut RelaySocket) -> Result<RoomCode> {
    loop {
        match websocket.read().context("failed to read room creation")? {
            Message::Text(text) => match serde_json::from_str::<RelayServerMessage>(&text)? {
                RelayServerMessage::RoomCreated { room } => return Ok(room),
                RelayServerMessage::Error { message } => bail!("relay rejected room: {message}"),
                other => bail!("unexpected room creation response: {other:?}"),
            },
            Message::Ping(payload) => websocket.send(Message::Pong(payload))?,
            Message::Close(_) => bail!("relay closed during room creation"),
            _ => {}
        }
    }
}

fn load_delta(sequence: u64) -> RaceDeltaSnapshot {
    RaceDeltaSnapshot {
        sequence,
        phase: NetworkRacePhase::Racing,
        bonuses: Vec::new(),
        players: vec![PlayerSnapshot {
            id: PlayerId(1),
            name: "host".to_string(),
            kind: PlayerKind::Human,
            color: AssignedColor::Cyan,
            word_index: sequence as usize % 40,
            input: "a".to_string(),
            typo_index: None,
            word_overrides: Vec::new(),
            finished: false,
            connected: true,
            shielded: false,
            focused: false,
            fogged: false,
            boosted: false,
            stunned: false,
            impact_remaining_ms: 0,
            impact_cue: None,
            item_cue: None,
        }],
        events: Vec::new(),
    }
}

fn connect_load_socket(relay: &str) -> Result<RelaySocket> {
    let (websocket, _) =
        connect(relay).with_context(|| format!("failed to connect to relay {relay}"))?;
    Ok(websocket)
}

fn relay_join_url(relay: &str, room: &RoomCode) -> String {
    let base = if relay_has_path_or_query(relay) {
        relay.to_string()
    } else {
        format!("{relay}/")
    };
    let separator = if base.contains('?') { '&' } else { '?' };
    format!("{base}{separator}typekart_room={}", room.as_str())
}

fn relay_has_path_or_query(relay: &str) -> bool {
    relay.contains('?')
        || relay
            .split_once("://")
            .is_none_or(|(_, rest)| rest.contains('/'))
}

fn send_relay_message(websocket: &mut RelaySocket, message: &RelayClientMessage) -> Result<()> {
    let encoded = serde_json::to_string(message).context("failed to encode relay message")?;
    websocket
        .send(Message::Text(encoded))
        .context("failed to send relay message")
}

fn set_load_socket_nonblocking(websocket: &mut RelaySocket) -> Result<()> {
    match websocket.get_mut() {
        MaybeTlsStream::Plain(stream) => stream
            .set_nonblocking(true)
            .context("failed to set load-test socket nonblocking"),
        MaybeTlsStream::NativeTls(stream) => stream
            .get_ref()
            .set_nonblocking(true)
            .context("failed to set load-test TLS socket nonblocking"),
        _ => bail!("unsupported relay websocket stream type"),
    }
}

fn load_disconnect_error(error: &WebSocketError) -> bool {
    matches!(
        error,
        WebSocketError::ConnectionClosed | WebSocketError::AlreadyClosed
    )
}

#[cfg(test)]
mod tests;
