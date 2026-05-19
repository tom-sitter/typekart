//! Local development WebSocket relay.
//!
//! This relay routes TypeKart protocol envelopes by room. It deliberately does
//! not understand race rules; the host remains authoritative.

use std::{
    collections::HashMap,
    io::Write,
    net::{IpAddr, SocketAddr, TcpListener},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender, SyncSender, TrySendError},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use tungstenite::{Error as WebSocketError, Message, accept, error::ProtocolError};

use super::{
    protocol::{PlayerId, version_mismatch_message},
    relay::{RelayClientMessage, RelayServerMessage, RoomCode},
};

#[derive(Debug, Clone)]
pub struct RelayConfig {
    pub bind: SocketAddr,
    pub ready_signal: Option<Sender<SocketAddr>>,
    pub limits: RelayLimits,
}

#[derive(Debug, Clone)]
pub struct RelayLimits {
    pub max_rooms: usize,
    pub max_participants_per_room: usize,
    pub max_connections: usize,
    pub max_connections_per_ip: usize,
    pub max_message_bytes: usize,
    pub max_messages_per_second_per_ip: u32,
    pub max_room_creates_per_minute_per_ip: u32,
    pub max_room_joins_per_minute_per_ip: u32,
    pub outbound_queue_size: usize,
    pub handshake_timeout: Duration,
    pub room_idle_timeout: Duration,
}

impl Default for RelayLimits {
    fn default() -> Self {
        Self {
            max_rooms: 256,
            max_participants_per_room: 5,
            max_connections: 1024,
            max_connections_per_ip: 64,
            max_message_bytes: 256 * 1024,
            max_messages_per_second_per_ip: 120,
            max_room_creates_per_minute_per_ip: 20,
            max_room_joins_per_minute_per_ip: 120,
            outbound_queue_size: 256,
            handshake_timeout: Duration::from_secs(5),
            room_idle_timeout: Duration::from_secs(2 * 60 * 60),
        }
    }
}

#[derive(Debug, Default)]
struct RelayState {
    rooms: HashMap<RoomCode, RelayRoom>,
}

#[derive(Debug, Default)]
struct RelayRateState {
    total_connections: usize,
    connections_by_ip: HashMap<IpAddr, usize>,
    message_buckets_by_ip: HashMap<IpAddr, TokenBucket>,
    room_create_buckets_by_ip: HashMap<IpAddr, TokenBucket>,
    room_join_buckets_by_ip: HashMap<IpAddr, TokenBucket>,
}

#[derive(Debug)]
struct RelayShared {
    rooms: Mutex<RelayState>,
    rates: Mutex<RelayRateState>,
}

#[derive(Debug)]
struct RelayRoom {
    host_version: String,
    host: SyncSender<RelayServerMessage>,
    participants: HashMap<PlayerId, SyncSender<RelayServerMessage>>,
    next_pending_player_id: u64,
    last_activity: Instant,
}

#[derive(Debug, Clone, Copy)]
struct TokenBucket {
    tokens: f64,
    capacity: f64,
    refill_per_second: f64,
    last_refill: Instant,
}

impl TokenBucket {
    fn new(capacity: u32, refill_per_second: f64, now: Instant) -> Self {
        let capacity = capacity.max(1) as f64;
        Self {
            tokens: capacity,
            capacity,
            refill_per_second,
            last_refill: now,
        }
    }

    fn allow(&mut self, now: Instant) -> bool {
        let elapsed = now
            .saturating_duration_since(self.last_refill)
            .as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_per_second).min(self.capacity);
        self.last_refill = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionRole {
    Host,
    Participant(PlayerId),
}

pub fn run_relay(config: RelayConfig) -> Result<()> {
    let listener = TcpListener::bind(config.bind)
        .with_context(|| format!("failed to bind relay at {}", config.bind))?;
    let local_addr = listener
        .local_addr()
        .context("failed to read relay address")?;
    println!(
        "TypeKart relay listening on ws://{local_addr} rooms={} participants_per_room={} connections={} connections_per_ip={} max_message_bytes={} messages_per_second_per_ip={} room_creates_per_minute_per_ip={} room_joins_per_minute_per_ip={} outbound_queue={} handshake_timeout_secs={} idle_timeout_secs={}",
        config.limits.max_rooms,
        config.limits.max_participants_per_room,
        config.limits.max_connections,
        config.limits.max_connections_per_ip,
        config.limits.max_message_bytes,
        config.limits.max_messages_per_second_per_ip,
        config.limits.max_room_creates_per_minute_per_ip,
        config.limits.max_room_joins_per_minute_per_ip,
        config.limits.outbound_queue_size,
        config.limits.handshake_timeout.as_secs(),
        config.limits.room_idle_timeout.as_secs()
    );
    if let Some(ready_signal) = config.ready_signal {
        let _ = ready_signal.send(local_addr);
    }

    let shared = Arc::new(RelayShared {
        rooms: Mutex::new(RelayState::default()),
        rates: Mutex::new(RelayRateState::default()),
    });
    spawn_idle_room_sweeper(Arc::clone(&shared), config.limits.room_idle_timeout);
    for stream in listener.incoming() {
        let stream = stream.context("failed to accept relay connection")?;
        let shared = Arc::clone(&shared);
        let limits = config.limits.clone();
        thread::spawn(move || {
            if let Err(error) = handle_connection(stream, shared, limits) {
                eprintln!("Relay connection failed: {error:#}");
            }
        });
    }

    Ok(())
}

fn handle_connection(
    mut stream: std::net::TcpStream,
    shared: Arc<RelayShared>,
    limits: RelayLimits,
) -> Result<()> {
    let peer = stream.peer_addr().ok();
    if handle_http_health_check(&mut stream)? {
        return Ok(());
    }
    let peer_ip = peer.map(|peer| peer.ip());
    let _connection_guard = if let Some(ip) = peer_ip {
        match try_register_connection(&shared, ip, &limits) {
            Ok(guard) => Some(guard),
            Err(message) => {
                write_http_rejection(&mut stream, &message)?;
                return Ok(());
            }
        }
    } else {
        None
    };

    let mut websocket = accept(stream).context("failed to accept websocket connection")?;
    websocket
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(10)))
        .context("failed to set relay socket read timeout")?;

    let (tx, rx) = mpsc::sync_channel::<RelayServerMessage>(limits.outbound_queue_size.max(1));
    let mut joined_room: Option<(RoomCode, ConnectionRole)> = None;
    let connected_at = Instant::now();

    loop {
        drain_outbound(&mut websocket, &rx)?;
        if joined_room.is_none() && connected_at.elapsed() > limits.handshake_timeout {
            queue_message(
                &tx,
                RelayServerMessage::Error {
                    message: "Relay handshake timed out".to_string(),
                },
            )?;
            drain_outbound(&mut websocket, &rx)?;
            break;
        }

        match websocket.read() {
            Ok(Message::Text(text)) => {
                if text.len() > limits.max_message_bytes {
                    let _ = queue_message(
                        &tx,
                        RelayServerMessage::Error {
                            message: format!(
                                "Message too large: {} bytes exceeds {} byte relay limit",
                                text.len(),
                                limits.max_message_bytes
                            ),
                        },
                    );
                    drain_outbound(&mut websocket, &rx)?;
                    break;
                }
                if let Some(ip) = peer_ip
                    && !allow_message_from_ip(&shared, ip, &limits)
                {
                    queue_message(
                        &tx,
                        RelayServerMessage::Error {
                            message: "Relay message rate limit exceeded".to_string(),
                        },
                    )?;
                    drain_outbound(&mut websocket, &rx)?;
                    break;
                }
                let message = serde_json::from_str::<RelayClientMessage>(&text)
                    .context("failed to decode relay message")?;
                if let Some(role) = handle_relay_message(message, &shared, &tx, peer_ip, &limits)? {
                    joined_room = Some(role);
                }
            }
            Ok(Message::Close(_)) => break,
            Ok(Message::Ping(payload)) => {
                websocket
                    .send(Message::Pong(payload))
                    .context("failed to send websocket pong")?;
            }
            Ok(_) => {}
            Err(WebSocketError::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                thread::sleep(Duration::from_millis(10));
            }
            Err(WebSocketError::ConnectionClosed | WebSocketError::AlreadyClosed) => break,
            Err(WebSocketError::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::ConnectionReset
                        | std::io::ErrorKind::ConnectionAborted
                        | std::io::ErrorKind::BrokenPipe
                        | std::io::ErrorKind::UnexpectedEof
                ) =>
            {
                break;
            }
            Err(WebSocketError::Protocol(ProtocolError::ResetWithoutClosingHandshake)) => break,
            Err(error) => return Err(error).context("websocket read failed"),
        }
    }

    if let Some((room, role)) = joined_room {
        cleanup_connection(&shared, &room, role);
    }
    if let Some(peer) = peer {
        println!("Relay client disconnected: {peer}");
    }
    Ok(())
}

struct ConnectionGuard {
    shared: Arc<RelayShared>,
    ip: IpAddr,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        let mut rates = self.shared.rates.lock().expect("relay rate state poisoned");
        rates.total_connections = rates.total_connections.saturating_sub(1);
        if let Some(count) = rates.connections_by_ip.get_mut(&self.ip) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                rates.connections_by_ip.remove(&self.ip);
            }
        }
    }
}

fn try_register_connection(
    shared: &Arc<RelayShared>,
    ip: IpAddr,
    limits: &RelayLimits,
) -> std::result::Result<ConnectionGuard, String> {
    let mut rates = shared.rates.lock().expect("relay rate state poisoned");
    if rates.total_connections >= limits.max_connections {
        return Err("Relay connection limit reached".to_string());
    }
    let ip_connections = rates.connections_by_ip.get(&ip).copied().unwrap_or(0);
    if ip_connections >= limits.max_connections_per_ip {
        return Err("Relay per-IP connection limit reached".to_string());
    }
    rates.total_connections += 1;
    *rates.connections_by_ip.entry(ip).or_insert(0) += 1;
    Ok(ConnectionGuard {
        shared: Arc::clone(shared),
        ip,
    })
}

fn write_http_rejection(stream: &mut std::net::TcpStream, message: &str) -> Result<()> {
    let body = format!("{message}\n");
    let response = format!(
        "HTTP/1.1 429 Too Many Requests\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    stream
        .write_all(response.as_bytes())
        .context("failed to write relay rejection response")
}

fn handle_http_health_check(stream: &mut std::net::TcpStream) -> Result<bool> {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .context("failed to set relay health-check read timeout")?;
    let mut buffer = [0_u8; 1024];
    let bytes = match stream.peek(&mut buffer) {
        Ok(bytes) => bytes,
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            stream
                .set_read_timeout(None)
                .context("failed to clear relay health-check read timeout")?;
            return Ok(false);
        }
        Err(error) => return Err(error).context("failed to inspect relay connection"),
    };
    stream
        .set_read_timeout(None)
        .context("failed to clear relay health-check read timeout")?;

    let request = String::from_utf8_lossy(&buffer[..bytes]);
    if is_health_check_request(&request) {
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK",
            )
            .context("failed to write relay health-check response")?;
        return Ok(true);
    }

    Ok(false)
}

fn is_health_check_request(request: &str) -> bool {
    let first_line = request.lines().next().unwrap_or_default();
    let is_health_path =
        first_line.starts_with("GET /healthz ") || first_line.starts_with("HEAD /healthz ");
    let is_websocket_upgrade = request
        .lines()
        .any(|line| line.eq_ignore_ascii_case("Upgrade: websocket"));
    is_health_path && !is_websocket_upgrade
}

fn drain_outbound(
    websocket: &mut tungstenite::WebSocket<std::net::TcpStream>,
    rx: &Receiver<RelayServerMessage>,
) -> Result<()> {
    while let Ok(message) = rx.try_recv() {
        let encoded = serde_json::to_string(&message).context("failed to encode relay message")?;
        if let Err(error) = websocket.send(Message::Text(encoded)) {
            if websocket_disconnect_error(&error) {
                break;
            }
            return Err(error).context("failed to send relay websocket message");
        }
    }
    Ok(())
}

fn queue_message(
    sender: &SyncSender<RelayServerMessage>,
    message: RelayServerMessage,
) -> Result<()> {
    match sender.try_send(message) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(_)) => anyhow::bail!("relay outbound queue is full"),
        Err(TrySendError::Disconnected(_)) => anyhow::bail!("relay outbound queue is closed"),
    }
}

fn websocket_disconnect_error(error: &WebSocketError) -> bool {
    match error {
        WebSocketError::ConnectionClosed | WebSocketError::AlreadyClosed => true,
        WebSocketError::Protocol(ProtocolError::ResetWithoutClosingHandshake) => true,
        WebSocketError::Io(error) => matches!(
            error.kind(),
            std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::ConnectionAborted
                | std::io::ErrorKind::BrokenPipe
                | std::io::ErrorKind::UnexpectedEof
        ),
        _ => false,
    }
}

fn handle_relay_message(
    message: RelayClientMessage,
    shared: &Arc<RelayShared>,
    sender: &SyncSender<RelayServerMessage>,
    peer_ip: Option<IpAddr>,
    limits: &RelayLimits,
) -> Result<Option<(RoomCode, ConnectionRole)>> {
    cleanup_stale_rooms(shared, limits.room_idle_timeout);

    match message {
        RelayClientMessage::CreateRoom { host_version } => {
            if let Some(ip) = peer_ip
                && !allow_room_create_from_ip(shared, ip, limits)
            {
                queue_message(
                    sender,
                    RelayServerMessage::Error {
                        message: "Relay room creation rate limit exceeded".to_string(),
                    },
                )?;
                return Ok(None);
            }
            let room = match create_room(shared, sender.clone(), host_version, limits) {
                Ok(room) => room,
                Err(message) => {
                    queue_message(sender, RelayServerMessage::Error { message })?;
                    return Ok(None);
                }
            };
            println!("Relay room created: {}", room.display());
            queue_message(
                sender,
                RelayServerMessage::RoomCreated { room: room.clone() },
            )
            .context("failed to send room created")?;
            Ok(Some((room, ConnectionRole::Host)))
        }
        RelayClientMessage::JoinRoom {
            room,
            name,
            client_version,
        } => {
            if let Some(ip) = peer_ip
                && !allow_room_join_from_ip(shared, ip, limits)
            {
                queue_message(
                    sender,
                    RelayServerMessage::Error {
                        message: "Relay room join rate limit exceeded".to_string(),
                    },
                )?;
                return Ok(None);
            }
            let pending_player_id = {
                let mut state = shared.rooms.lock().expect("relay state poisoned");
                let Some(room_state) = state.rooms.get_mut(&room) else {
                    queue_message(
                        sender,
                        RelayServerMessage::Error {
                            message: format!("Room {} was not found", room.display()),
                        },
                    )
                    .ok();
                    return Ok(None);
                };
                if room_state.participants.len() >= limits.max_participants_per_room {
                    queue_message(
                        sender,
                        RelayServerMessage::Error {
                            message: format!(
                                "Room {} is full: {}/{} joiners connected",
                                room.display(),
                                room_state.participants.len(),
                                limits.max_participants_per_room
                            ),
                        },
                    )
                    .ok();
                    return Ok(None);
                }
                if client_version != room_state.host_version {
                    queue_message(
                        sender,
                        RelayServerMessage::Error {
                            message: version_mismatch_message(
                                &room_state.host_version,
                                &client_version,
                            ),
                        },
                    )
                    .ok();
                    return Ok(None);
                }
                let pending_player_id = PlayerId(room_state.next_pending_player_id);
                room_state.next_pending_player_id += 1;
                room_state.last_activity = Instant::now();
                room_state
                    .participants
                    .insert(pending_player_id, sender.clone());
                println!(
                    "Relay join forwarded: room={} pending_player={} name={}",
                    room.display(),
                    pending_player_id.0,
                    name
                );
                if room_state
                    .host
                    .try_send(RelayServerMessage::JoinForwarded {
                        room: room.clone(),
                        pending_player_id,
                        name,
                        client_version,
                    })
                    .is_err()
                {
                    room_state.participants.remove(&pending_player_id);
                    let _ = queue_message(
                        sender,
                        RelayServerMessage::Error {
                            message: "Host is not accepting relay messages".to_string(),
                        },
                    );
                    return Ok(None);
                }
                pending_player_id
            };
            Ok(Some((room, ConnectionRole::Participant(pending_player_id))))
        }
        RelayClientMessage::ClientToHost {
            room,
            player_id,
            message,
        } => {
            let mut state = shared.rooms.lock().expect("relay state poisoned");
            if let Some(room_state) = state.rooms.get_mut(&room) {
                room_state.last_activity = Instant::now();
                if room_state
                    .host
                    .try_send(RelayServerMessage::ClientToHost {
                        room: room.clone(),
                        player_id,
                        message,
                    })
                    .is_err()
                {
                    close_room(&mut state, &room, "Host relay queue full");
                }
            }
            Ok(None)
        }
        RelayClientMessage::HostToClient {
            room,
            player_id,
            message,
        } => {
            let mut state = shared.rooms.lock().expect("relay state poisoned");
            if let Some(room_state) = state.rooms.get_mut(&room) {
                room_state.last_activity = Instant::now();
                if let Some(participant) = room_state.participants.get(&player_id)
                    && participant
                        .try_send(RelayServerMessage::HostToClient {
                            room: room.clone(),
                            player_id,
                            message,
                        })
                        .is_err()
                {
                    room_state.participants.remove(&player_id);
                    let _ = room_state
                        .host
                        .try_send(RelayServerMessage::ParticipantDisconnected { room, player_id });
                }
            }
            Ok(None)
        }
        RelayClientMessage::HostBroadcast { room, message } => {
            let mut state = shared.rooms.lock().expect("relay state poisoned");
            if let Some(room_state) = state.rooms.get_mut(&room) {
                room_state.last_activity = Instant::now();
                let mut disconnected = Vec::new();
                for (player_id, participant) in &room_state.participants {
                    if participant
                        .try_send(RelayServerMessage::HostBroadcast {
                            room: room.clone(),
                            message: message.clone(),
                        })
                        .is_err()
                    {
                        disconnected.push(*player_id);
                    }
                }
                for player_id in disconnected {
                    room_state.participants.remove(&player_id);
                    let _ = room_state
                        .host
                        .try_send(RelayServerMessage::ParticipantDisconnected {
                            room: room.clone(),
                            player_id,
                        });
                }
            }
            Ok(None)
        }
        // The connection cleanup path owns removal because it knows whether this
        // socket is the host or a participant. A leave message can close the
        // socket in a later online client implementation without needing to
        // compare channel identities.
        RelayClientMessage::LeaveRoom { .. } => Ok(None),
    }
}

fn create_room(
    shared: &Arc<RelayShared>,
    host: SyncSender<RelayServerMessage>,
    host_version: String,
    limits: &RelayLimits,
) -> std::result::Result<RoomCode, String> {
    let mut state = shared.rooms.lock().expect("relay state poisoned");
    if state.rooms.len() >= limits.max_rooms {
        return Err(format!(
            "Relay is full: {}/{} rooms active",
            state.rooms.len(),
            limits.max_rooms
        ));
    }

    loop {
        let room = RoomCode::generate();
        if !state.rooms.contains_key(&room) {
            state.rooms.insert(
                room.clone(),
                RelayRoom {
                    host_version: host_version.clone(),
                    host: host.clone(),
                    participants: HashMap::new(),
                    next_pending_player_id: 2,
                    last_activity: Instant::now(),
                },
            );
            return Ok(room);
        }
    }
}

fn cleanup_connection(shared: &Arc<RelayShared>, room: &RoomCode, role: ConnectionRole) {
    let mut state = shared.rooms.lock().expect("relay state poisoned");
    match role {
        ConnectionRole::Host => close_room(&mut state, room, "Host disconnected"),
        ConnectionRole::Participant(player_id) => {
            if let Some(room_state) = state.rooms.get_mut(room) {
                room_state.participants.remove(&player_id);
                room_state.last_activity = Instant::now();
                let _ = room_state
                    .host
                    .try_send(RelayServerMessage::ParticipantDisconnected {
                        room: room.clone(),
                        player_id,
                    });
                println!(
                    "Relay participant disconnected: room={} player={}",
                    room.display(),
                    player_id.0
                );
            }
        }
    }
}

fn close_room(state: &mut RelayState, room: &RoomCode, reason: &str) {
    let Some(room_state) = state.rooms.remove(room) else {
        return;
    };
    println!("Relay room closed: {} ({reason})", room.display());
    for participant in room_state.participants.values() {
        let _ = participant.try_send(RelayServerMessage::RoomClosed {
            reason: reason.to_string(),
        });
    }
}

fn cleanup_stale_rooms(shared: &Arc<RelayShared>, idle_timeout: Duration) {
    if idle_timeout.is_zero() {
        return;
    }

    let mut state = shared.rooms.lock().expect("relay state poisoned");
    let now = Instant::now();
    let stale_rooms = state
        .rooms
        .iter()
        .filter_map(|(room, room_state)| {
            if now.duration_since(room_state.last_activity) > idle_timeout {
                Some(room.clone())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    for room in stale_rooms {
        close_room(&mut state, &room, "Room idle timeout");
    }
}

fn spawn_idle_room_sweeper(shared: Arc<RelayShared>, idle_timeout: Duration) {
    if idle_timeout.is_zero() {
        return;
    }

    thread::spawn(move || {
        loop {
            thread::sleep(idle_sweep_interval(idle_timeout));
            cleanup_stale_rooms(&shared, idle_timeout);
        }
    });
}

fn allow_message_from_ip(shared: &Arc<RelayShared>, ip: IpAddr, limits: &RelayLimits) -> bool {
    allow_bucket(
        &mut shared.rates.lock().expect("relay rate state poisoned"),
        BucketKind::Message,
        ip,
        limits.max_messages_per_second_per_ip,
        limits.max_messages_per_second_per_ip as f64,
    )
}

fn allow_room_create_from_ip(shared: &Arc<RelayShared>, ip: IpAddr, limits: &RelayLimits) -> bool {
    allow_bucket(
        &mut shared.rates.lock().expect("relay rate state poisoned"),
        BucketKind::RoomCreate,
        ip,
        limits.max_room_creates_per_minute_per_ip,
        limits.max_room_creates_per_minute_per_ip as f64 / 60.0,
    )
}

fn allow_room_join_from_ip(shared: &Arc<RelayShared>, ip: IpAddr, limits: &RelayLimits) -> bool {
    allow_bucket(
        &mut shared.rates.lock().expect("relay rate state poisoned"),
        BucketKind::RoomJoin,
        ip,
        limits.max_room_joins_per_minute_per_ip,
        limits.max_room_joins_per_minute_per_ip as f64 / 60.0,
    )
}

#[derive(Clone, Copy)]
enum BucketKind {
    Message,
    RoomCreate,
    RoomJoin,
}

fn allow_bucket(
    rates: &mut RelayRateState,
    kind: BucketKind,
    ip: IpAddr,
    capacity: u32,
    refill_per_second: f64,
) -> bool {
    let now = Instant::now();
    let buckets = match kind {
        BucketKind::Message => &mut rates.message_buckets_by_ip,
        BucketKind::RoomCreate => &mut rates.room_create_buckets_by_ip,
        BucketKind::RoomJoin => &mut rates.room_join_buckets_by_ip,
    };
    buckets
        .entry(ip)
        .or_insert_with(|| TokenBucket::new(capacity, refill_per_second, now))
        .allow(now)
}

fn idle_sweep_interval(idle_timeout: Duration) -> Duration {
    #[cfg(test)]
    if idle_timeout < Duration::from_secs(5) {
        return idle_timeout;
    }

    const MIN_SWEEP_INTERVAL: Duration = Duration::from_secs(5);
    const MAX_SWEEP_INTERVAL: Duration = Duration::from_secs(60);

    let interval = idle_timeout / 4;
    interval.clamp(MIN_SWEEP_INTERVAL, MAX_SWEEP_INTERVAL)
}

#[cfg(test)]
mod tests {
    use std::{
        net::{IpAddr, Ipv4Addr},
        sync::{Arc, mpsc},
        thread,
        time::{Duration, Instant},
    };

    use super::{
        ConnectionRole, RelayLimits, RelayShared, allow_message_from_ip, allow_room_create_from_ip,
        allow_room_join_from_ip, cleanup_connection, cleanup_stale_rooms, handle_relay_message,
        idle_sweep_interval, spawn_idle_room_sweeper,
    };
    use crate::net::{
        protocol::PlayerId,
        relay::{RelayClientMessage, RelayServerMessage},
    };

    fn relay_shared() -> Arc<RelayShared> {
        Arc::new(RelayShared {
            rooms: Default::default(),
            rates: Default::default(),
        })
    }

    fn localhost() -> IpAddr {
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    }

    #[test]
    fn relay_creates_room_and_forwards_join_to_host() {
        let state = relay_shared();
        let (host_tx, host_rx) = mpsc::sync_channel(32);
        let (joiner_tx, _joiner_rx) = mpsc::sync_channel(32);

        let Some((room, ConnectionRole::Host)) = handle_relay_message(
            RelayClientMessage::CreateRoom {
                host_version: "test".to_string(),
            },
            &state,
            &host_tx,
            None,
            &RelayLimits::default(),
        )
        .unwrap() else {
            panic!("host should create a room");
        };
        assert!(matches!(
            host_rx.recv().unwrap(),
            RelayServerMessage::RoomCreated { .. }
        ));

        let joined = handle_relay_message(
            RelayClientMessage::JoinRoom {
                room: room.clone(),
                name: "joiner".to_string(),
                client_version: "test".to_string(),
            },
            &state,
            &joiner_tx,
            None,
            &RelayLimits::default(),
        )
        .unwrap();

        assert_eq!(
            joined,
            Some((room.clone(), ConnectionRole::Participant(PlayerId(2))))
        );
        assert_eq!(
            host_rx.recv().unwrap(),
            RelayServerMessage::JoinForwarded {
                room,
                pending_player_id: PlayerId(2),
                name: "joiner".to_string(),
                client_version: "test".to_string(),
            }
        );
    }

    #[test]
    fn relay_tracks_connection_limits_by_ip() {
        let state = relay_shared();
        let limits = RelayLimits {
            max_connections: 2,
            max_connections_per_ip: 1,
            ..RelayLimits::default()
        };

        let first = super::try_register_connection(&state, localhost(), &limits).unwrap();
        assert!(super::try_register_connection(&state, localhost(), &limits).is_err());
        drop(first);
        assert!(super::try_register_connection(&state, localhost(), &limits).is_ok());
    }

    #[test]
    fn relay_rate_limits_room_creation_by_ip() {
        let state = relay_shared();
        let limits = RelayLimits {
            max_room_creates_per_minute_per_ip: 1,
            ..RelayLimits::default()
        };

        assert!(allow_room_create_from_ip(&state, localhost(), &limits));
        assert!(!allow_room_create_from_ip(&state, localhost(), &limits));
    }

    #[test]
    fn relay_rate_limits_joins_by_ip() {
        let state = relay_shared();
        let limits = RelayLimits {
            max_room_joins_per_minute_per_ip: 1,
            ..RelayLimits::default()
        };

        assert!(allow_room_join_from_ip(&state, localhost(), &limits));
        assert!(!allow_room_join_from_ip(&state, localhost(), &limits));
    }

    #[test]
    fn relay_rate_limits_messages_by_ip() {
        let state = relay_shared();
        let limits = RelayLimits {
            max_messages_per_second_per_ip: 1,
            ..RelayLimits::default()
        };

        assert!(allow_message_from_ip(&state, localhost(), &limits));
        assert!(!allow_message_from_ip(&state, localhost(), &limits));
    }

    #[test]
    fn relay_routes_client_messages_to_host() {
        let state = relay_shared();
        let (host_tx, host_rx) = mpsc::sync_channel(32);
        let (joiner_tx, _joiner_rx) = mpsc::sync_channel(32);
        let Some((room, _)) = handle_relay_message(
            RelayClientMessage::CreateRoom {
                host_version: "test".to_string(),
            },
            &state,
            &host_tx,
            None,
            &RelayLimits::default(),
        )
        .unwrap() else {
            panic!("host should create a room");
        };
        let _ = host_rx.recv().unwrap();
        handle_relay_message(
            RelayClientMessage::JoinRoom {
                room: room.clone(),
                name: "joiner".to_string(),
                client_version: "test".to_string(),
            },
            &state,
            &joiner_tx,
            None,
            &RelayLimits::default(),
        )
        .unwrap();
        let _ = host_rx.recv().unwrap();

        let message = serde_json::json!({
            "type": "future_client_command",
            "payload": { "sequence": 7 }
        });
        handle_relay_message(
            RelayClientMessage::ClientToHost {
                room: room.clone(),
                player_id: PlayerId(2),
                message: message.clone(),
            },
            &state,
            &joiner_tx,
            None,
            &RelayLimits::default(),
        )
        .unwrap();

        assert_eq!(
            host_rx.recv().unwrap(),
            RelayServerMessage::ClientToHost {
                room,
                player_id: PlayerId(2),
                message,
            }
        );
    }

    #[test]
    fn relay_rejects_join_when_client_version_differs_from_host() {
        let state = relay_shared();
        let (host_tx, host_rx) = mpsc::sync_channel(32);
        let (joiner_tx, joiner_rx) = mpsc::sync_channel(32);

        let Some((room, ConnectionRole::Host)) = handle_relay_message(
            RelayClientMessage::CreateRoom {
                host_version: "1.2.3".to_string(),
            },
            &state,
            &host_tx,
            None,
            &RelayLimits::default(),
        )
        .unwrap() else {
            panic!("host should create a room");
        };
        let _ = host_rx.recv().unwrap();

        let joined = handle_relay_message(
            RelayClientMessage::JoinRoom {
                room,
                name: "joiner".to_string(),
                client_version: "1.2.4".to_string(),
            },
            &state,
            &joiner_tx,
            None,
            &RelayLimits::default(),
        )
        .unwrap();

        assert_eq!(joined, None);
        assert!(matches!(
            joiner_rx.recv().unwrap(),
            RelayServerMessage::Error { message } if message.contains("Version mismatch")
                && message.contains("1.2.3")
                && message.contains("1.2.4")
        ));
        assert!(host_rx.try_recv().is_err());
    }

    #[test]
    fn relay_broadcasts_host_messages_once_per_participant() {
        let state = relay_shared();
        let (host_tx, host_rx) = mpsc::sync_channel(32);
        let (joiner_tx, joiner_rx) = mpsc::sync_channel(32);
        let Some((room, _)) = handle_relay_message(
            RelayClientMessage::CreateRoom {
                host_version: "test".to_string(),
            },
            &state,
            &host_tx,
            None,
            &RelayLimits::default(),
        )
        .unwrap() else {
            panic!("host should create a room");
        };
        let _ = host_rx.recv().unwrap();
        handle_relay_message(
            RelayClientMessage::JoinRoom {
                room: room.clone(),
                name: "joiner".to_string(),
                client_version: "test".to_string(),
            },
            &state,
            &joiner_tx,
            None,
            &RelayLimits::default(),
        )
        .unwrap();

        let message = serde_json::json!({
            "type": "future_server_command",
            "payload": { "message": "test" }
        });
        handle_relay_message(
            RelayClientMessage::HostBroadcast {
                room: room.clone(),
                message: message.clone(),
            },
            &state,
            &host_tx,
            None,
            &RelayLimits::default(),
        )
        .unwrap();

        assert_eq!(
            joiner_rx.try_recv().unwrap(),
            RelayServerMessage::HostBroadcast { room, message }
        );
        assert!(joiner_rx.try_recv().is_err());
    }

    #[test]
    fn relay_closes_room_when_host_disconnects() {
        let state = relay_shared();
        let (host_tx, host_rx) = mpsc::sync_channel(32);
        let (joiner_tx, joiner_rx) = mpsc::sync_channel(32);
        let Some((room, _)) = handle_relay_message(
            RelayClientMessage::CreateRoom {
                host_version: "test".to_string(),
            },
            &state,
            &host_tx,
            None,
            &RelayLimits::default(),
        )
        .unwrap() else {
            panic!("host should create a room");
        };
        let _ = host_rx.recv().unwrap();
        handle_relay_message(
            RelayClientMessage::JoinRoom {
                room: room.clone(),
                name: "joiner".to_string(),
                client_version: "test".to_string(),
            },
            &state,
            &joiner_tx,
            None,
            &RelayLimits::default(),
        )
        .unwrap();

        cleanup_connection(&state, &room, ConnectionRole::Host);

        assert!(matches!(
            joiner_rx.recv().unwrap(),
            RelayServerMessage::RoomClosed { .. }
        ));
        assert!(!state.rooms.lock().unwrap().rooms.contains_key(&room));
    }

    #[test]
    fn relay_notifies_host_when_participant_disconnects() {
        let state = relay_shared();
        let (host_tx, host_rx) = mpsc::sync_channel(32);
        let (joiner_tx, _joiner_rx) = mpsc::sync_channel(32);
        let Some((room, _)) = handle_relay_message(
            RelayClientMessage::CreateRoom {
                host_version: "test".to_string(),
            },
            &state,
            &host_tx,
            None,
            &RelayLimits::default(),
        )
        .unwrap() else {
            panic!("host should create a room");
        };
        let _ = host_rx.recv().unwrap();
        let Some((_, ConnectionRole::Participant(player_id))) = handle_relay_message(
            RelayClientMessage::JoinRoom {
                room: room.clone(),
                name: "joiner".to_string(),
                client_version: "test".to_string(),
            },
            &state,
            &joiner_tx,
            None,
            &RelayLimits::default(),
        )
        .unwrap() else {
            panic!("joiner should enter room");
        };
        let _ = host_rx.recv().unwrap();

        cleanup_connection(&state, &room, ConnectionRole::Participant(player_id));

        assert!(matches!(
            host_rx.recv().unwrap(),
            RelayServerMessage::ParticipantDisconnected {
                room: disconnected_room,
                player_id: disconnected_player,
            } if disconnected_room == room && disconnected_player == player_id
        ));
        assert!(
            !state
                .rooms
                .lock()
                .unwrap()
                .rooms
                .get(&room)
                .unwrap()
                .participants
                .contains_key(&player_id)
        );
    }

    #[test]
    fn relay_rejects_room_creation_when_room_limit_is_reached() {
        let state = relay_shared();
        let (host_tx, host_rx) = mpsc::sync_channel(32);
        let limits = RelayLimits {
            max_rooms: 1,
            ..RelayLimits::default()
        };

        let Some((_, ConnectionRole::Host)) = handle_relay_message(
            RelayClientMessage::CreateRoom {
                host_version: "test".to_string(),
            },
            &state,
            &host_tx,
            None,
            &limits,
        )
        .unwrap() else {
            panic!("first host should create a room");
        };
        let _ = host_rx.recv().unwrap();

        assert_eq!(
            handle_relay_message(
                RelayClientMessage::CreateRoom {
                    host_version: "test".to_string(),
                },
                &state,
                &host_tx,
                None,
                &limits,
            )
            .unwrap(),
            None
        );
        assert!(matches!(
            host_rx.recv().unwrap(),
            RelayServerMessage::Error { message } if message.contains("Relay is full")
        ));
    }

    #[test]
    fn relay_rejects_join_when_room_is_full() {
        let state = relay_shared();
        let (host_tx, host_rx) = mpsc::sync_channel(32);
        let (joiner_tx, _joiner_rx) = mpsc::sync_channel(32);
        let (extra_tx, extra_rx) = mpsc::sync_channel(32);
        let limits = RelayLimits {
            max_participants_per_room: 1,
            ..RelayLimits::default()
        };

        let Some((room, _)) = handle_relay_message(
            RelayClientMessage::CreateRoom {
                host_version: "test".to_string(),
            },
            &state,
            &host_tx,
            None,
            &limits,
        )
        .unwrap() else {
            panic!("host should create a room");
        };
        let _ = host_rx.recv().unwrap();
        handle_relay_message(
            RelayClientMessage::JoinRoom {
                room: room.clone(),
                name: "joiner".to_string(),
                client_version: "test".to_string(),
            },
            &state,
            &joiner_tx,
            None,
            &limits,
        )
        .unwrap();
        let _ = host_rx.recv().unwrap();

        assert_eq!(
            handle_relay_message(
                RelayClientMessage::JoinRoom {
                    room,
                    name: "extra".to_string(),
                    client_version: "test".to_string(),
                },
                &state,
                &extra_tx,
                None,
                &limits,
            )
            .unwrap(),
            None
        );
        assert!(matches!(
            extra_rx.recv().unwrap(),
            RelayServerMessage::Error { message } if message.contains("is full")
        ));
    }

    #[test]
    fn relay_cleans_up_idle_rooms() {
        let state = relay_shared();
        let (host_tx, host_rx) = mpsc::sync_channel(32);
        let limits = RelayLimits::default();
        let Some((room, _)) = handle_relay_message(
            RelayClientMessage::CreateRoom {
                host_version: "test".to_string(),
            },
            &state,
            &host_tx,
            None,
            &limits,
        )
        .unwrap() else {
            panic!("host should create a room");
        };
        let _ = host_rx.recv().unwrap();

        state
            .rooms
            .lock()
            .unwrap()
            .rooms
            .get_mut(&room)
            .unwrap()
            .last_activity = Instant::now() - Duration::from_secs(5);

        cleanup_stale_rooms(&state, Duration::from_secs(1));

        assert!(!state.rooms.lock().unwrap().rooms.contains_key(&room));
    }

    #[test]
    fn idle_sweep_interval_is_bounded() {
        assert_eq!(
            idle_sweep_interval(Duration::from_secs(1)),
            Duration::from_secs(1)
        );
        assert_eq!(
            idle_sweep_interval(Duration::from_secs(80)),
            Duration::from_secs(20)
        );
        assert_eq!(
            idle_sweep_interval(Duration::from_secs(1000)),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn health_check_request_matches_only_plain_health_http() {
        assert!(super::is_health_check_request(
            "GET /healthz HTTP/1.1\r\nHost: relay\r\n\r\n"
        ));
        assert!(super::is_health_check_request(
            "HEAD /healthz HTTP/1.1\r\nHost: relay\r\n\r\n"
        ));
        assert!(!super::is_health_check_request(
            "GET / HTTP/1.1\r\nHost: relay\r\n\r\n"
        ));
        assert!(!super::is_health_check_request(
            "GET /healthz HTTP/1.1\r\nHost: relay\r\nUpgrade: websocket\r\n\r\n"
        ));
    }

    #[test]
    fn idle_room_sweeper_removes_stale_rooms_without_new_messages() {
        let state = relay_shared();
        let (host_tx, host_rx) = mpsc::sync_channel(32);
        let limits = RelayLimits::default();
        let Some((room, _)) = handle_relay_message(
            RelayClientMessage::CreateRoom {
                host_version: "test".to_string(),
            },
            &state,
            &host_tx,
            None,
            &limits,
        )
        .unwrap() else {
            panic!("host should create a room");
        };
        let _ = host_rx.recv().unwrap();

        state
            .rooms
            .lock()
            .unwrap()
            .rooms
            .get_mut(&room)
            .unwrap()
            .last_activity = Instant::now() - Duration::from_secs(10);

        spawn_idle_room_sweeper(Arc::clone(&state), Duration::from_millis(1));
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            if !state.rooms.lock().unwrap().rooms.contains_key(&room) {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }

        panic!("idle sweeper did not remove stale room");
    }
}
