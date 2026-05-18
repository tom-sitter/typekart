//! Local development WebSocket relay.
//!
//! This relay routes TypeKart protocol envelopes by room. It deliberately does
//! not understand race rules; the host remains authoritative.

use std::{
    collections::HashMap,
    io::Write,
    net::{SocketAddr, TcpListener},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender},
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
    pub max_message_bytes: usize,
    pub room_idle_timeout: Duration,
}

impl Default for RelayLimits {
    fn default() -> Self {
        Self {
            max_rooms: 256,
            max_participants_per_room: 5,
            max_message_bytes: 256 * 1024,
            room_idle_timeout: Duration::from_secs(2 * 60 * 60),
        }
    }
}

#[derive(Debug, Default)]
struct RelayState {
    rooms: HashMap<RoomCode, RelayRoom>,
}

#[derive(Debug)]
struct RelayRoom {
    host_version: String,
    host: Sender<RelayServerMessage>,
    participants: HashMap<PlayerId, Sender<RelayServerMessage>>,
    next_pending_player_id: u64,
    last_activity: Instant,
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
        "TypeKart relay listening on ws://{local_addr} rooms={} participants_per_room={} max_message_bytes={} idle_timeout_secs={}",
        config.limits.max_rooms,
        config.limits.max_participants_per_room,
        config.limits.max_message_bytes,
        config.limits.room_idle_timeout.as_secs()
    );
    if let Some(ready_signal) = config.ready_signal {
        let _ = ready_signal.send(local_addr);
    }

    let state = Arc::new(Mutex::new(RelayState::default()));
    spawn_idle_room_sweeper(Arc::clone(&state), config.limits.room_idle_timeout);
    for stream in listener.incoming() {
        let stream = stream.context("failed to accept relay connection")?;
        let state = Arc::clone(&state);
        let limits = config.limits.clone();
        thread::spawn(move || {
            if let Err(error) = handle_connection(stream, state, limits) {
                eprintln!("Relay connection failed: {error:#}");
            }
        });
    }

    Ok(())
}

fn handle_connection(
    mut stream: std::net::TcpStream,
    state: Arc<Mutex<RelayState>>,
    limits: RelayLimits,
) -> Result<()> {
    let peer = stream.peer_addr().ok();
    if handle_http_health_check(&mut stream)? {
        return Ok(());
    }

    let mut websocket = accept(stream).context("failed to accept websocket connection")?;
    websocket
        .get_mut()
        .set_nonblocking(true)
        .context("failed to set relay socket nonblocking")?;

    let (tx, rx) = mpsc::channel::<RelayServerMessage>();
    let mut joined_room: Option<(RoomCode, ConnectionRole)> = None;

    loop {
        drain_outbound(&mut websocket, &rx)?;

        match websocket.read() {
            Ok(Message::Text(text)) => {
                if text.len() > limits.max_message_bytes {
                    let _ = tx.send(RelayServerMessage::Error {
                        message: format!(
                            "Message too large: {} bytes exceeds {} byte relay limit",
                            text.len(),
                            limits.max_message_bytes
                        ),
                    });
                    drain_outbound(&mut websocket, &rx)?;
                    break;
                }
                let message = serde_json::from_str::<RelayClientMessage>(&text)
                    .context("failed to decode relay message")?;
                if let Some(role) = handle_relay_message(message, &state, &tx, &limits)? {
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
            Err(WebSocketError::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock => {
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
        cleanup_connection(&state, &room, role);
    }
    if let Some(peer) = peer {
        println!("Relay client disconnected: {peer}");
    }
    Ok(())
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
    state: &Arc<Mutex<RelayState>>,
    sender: &Sender<RelayServerMessage>,
    limits: &RelayLimits,
) -> Result<Option<(RoomCode, ConnectionRole)>> {
    cleanup_stale_rooms(state, limits.room_idle_timeout);

    match message {
        RelayClientMessage::CreateRoom { host_version } => {
            let room = match create_room(state, sender.clone(), host_version, limits) {
                Ok(room) => room,
                Err(message) => {
                    sender.send(RelayServerMessage::Error { message })?;
                    return Ok(None);
                }
            };
            println!("Relay room created: {}", room.display());
            sender
                .send(RelayServerMessage::RoomCreated { room: room.clone() })
                .context("failed to send room created")?;
            Ok(Some((room, ConnectionRole::Host)))
        }
        RelayClientMessage::JoinRoom {
            room,
            name,
            client_version,
        } => {
            let pending_player_id = {
                let mut state = state.lock().expect("relay state poisoned");
                let Some(room_state) = state.rooms.get_mut(&room) else {
                    sender
                        .send(RelayServerMessage::Error {
                            message: format!("Room {} was not found", room.display()),
                        })
                        .ok();
                    return Ok(None);
                };
                if room_state.participants.len() >= limits.max_participants_per_room {
                    sender
                        .send(RelayServerMessage::Error {
                            message: format!(
                                "Room {} is full: {}/{} joiners connected",
                                room.display(),
                                room_state.participants.len(),
                                limits.max_participants_per_room
                            ),
                        })
                        .ok();
                    return Ok(None);
                }
                if client_version != room_state.host_version {
                    sender
                        .send(RelayServerMessage::Error {
                            message: version_mismatch_message(
                                &room_state.host_version,
                                &client_version,
                            ),
                        })
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
                    .send(RelayServerMessage::JoinForwarded {
                        room: room.clone(),
                        pending_player_id,
                        name,
                        client_version,
                    })
                    .is_err()
                {
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
            let mut state = state.lock().expect("relay state poisoned");
            if let Some(room_state) = state.rooms.get_mut(&room) {
                room_state.last_activity = Instant::now();
                let _ = room_state.host.send(RelayServerMessage::ClientToHost {
                    room,
                    player_id,
                    message,
                });
            }
            Ok(None)
        }
        RelayClientMessage::HostToClient {
            room,
            player_id,
            message,
        } => {
            let mut state = state.lock().expect("relay state poisoned");
            if let Some(room_state) = state.rooms.get_mut(&room) {
                room_state.last_activity = Instant::now();
                if let Some(participant) = room_state.participants.get(&player_id) {
                    let _ = participant.send(RelayServerMessage::HostToClient {
                        room,
                        player_id,
                        message,
                    });
                }
            }
            Ok(None)
        }
        RelayClientMessage::HostBroadcast { room, message } => {
            let mut state = state.lock().expect("relay state poisoned");
            if let Some(room_state) = state.rooms.get_mut(&room) {
                room_state.last_activity = Instant::now();
                for participant in room_state.participants.values() {
                    let _ = participant.send(RelayServerMessage::HostBroadcast {
                        room: room.clone(),
                        message: message.clone(),
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
    state: &Arc<Mutex<RelayState>>,
    host: Sender<RelayServerMessage>,
    host_version: String,
    limits: &RelayLimits,
) -> std::result::Result<RoomCode, String> {
    let mut state = state.lock().expect("relay state poisoned");
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

fn cleanup_connection(state: &Arc<Mutex<RelayState>>, room: &RoomCode, role: ConnectionRole) {
    let mut state = state.lock().expect("relay state poisoned");
    match role {
        ConnectionRole::Host => close_room(&mut state, room, "Host disconnected"),
        ConnectionRole::Participant(player_id) => {
            if let Some(room_state) = state.rooms.get_mut(room) {
                room_state.participants.remove(&player_id);
                room_state.last_activity = Instant::now();
                let _ = room_state
                    .host
                    .send(RelayServerMessage::ParticipantDisconnected {
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
        let _ = participant.send(RelayServerMessage::RoomClosed {
            reason: reason.to_string(),
        });
    }
}

fn cleanup_stale_rooms(state: &Arc<Mutex<RelayState>>, idle_timeout: Duration) {
    if idle_timeout.is_zero() {
        return;
    }

    let mut state = state.lock().expect("relay state poisoned");
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

fn spawn_idle_room_sweeper(state: Arc<Mutex<RelayState>>, idle_timeout: Duration) {
    if idle_timeout.is_zero() {
        return;
    }

    thread::spawn(move || {
        loop {
            thread::sleep(idle_sweep_interval(idle_timeout));
            cleanup_stale_rooms(&state, idle_timeout);
        }
    });
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
        sync::{Arc, Mutex, mpsc},
        thread,
        time::{Duration, Instant},
    };

    use super::{
        ConnectionRole, RelayLimits, RelayState, cleanup_connection, cleanup_stale_rooms,
        handle_relay_message, idle_sweep_interval, spawn_idle_room_sweeper,
    };
    use crate::net::{
        protocol::{ClientMessage, ClientSequence, PlayerId, ProtocolKey, ServerMessage},
        relay::{RelayClientMessage, RelayServerMessage},
    };

    #[test]
    fn relay_creates_room_and_forwards_join_to_host() {
        let state = Arc::new(Mutex::new(RelayState::default()));
        let (host_tx, host_rx) = mpsc::channel();
        let (joiner_tx, _joiner_rx) = mpsc::channel();

        let Some((room, ConnectionRole::Host)) = handle_relay_message(
            RelayClientMessage::CreateRoom {
                host_version: "test".to_string(),
            },
            &state,
            &host_tx,
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
    fn relay_routes_client_messages_to_host() {
        let state = Arc::new(Mutex::new(RelayState::default()));
        let (host_tx, host_rx) = mpsc::channel();
        let (joiner_tx, _joiner_rx) = mpsc::channel();
        let Some((room, _)) = handle_relay_message(
            RelayClientMessage::CreateRoom {
                host_version: "test".to_string(),
            },
            &state,
            &host_tx,
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
            &RelayLimits::default(),
        )
        .unwrap();
        let _ = host_rx.recv().unwrap();

        let message = ClientMessage::KeyInput {
            sequence: ClientSequence(7),
            key: ProtocolKey::Char('a'),
        };
        handle_relay_message(
            RelayClientMessage::ClientToHost {
                room: room.clone(),
                player_id: PlayerId(2),
                message: message.clone(),
            },
            &state,
            &joiner_tx,
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
        let state = Arc::new(Mutex::new(RelayState::default()));
        let (host_tx, host_rx) = mpsc::channel();
        let (joiner_tx, joiner_rx) = mpsc::channel();

        let Some((room, ConnectionRole::Host)) = handle_relay_message(
            RelayClientMessage::CreateRoom {
                host_version: "1.2.3".to_string(),
            },
            &state,
            &host_tx,
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
        let state = Arc::new(Mutex::new(RelayState::default()));
        let (host_tx, host_rx) = mpsc::channel();
        let (joiner_tx, joiner_rx) = mpsc::channel();
        let Some((room, _)) = handle_relay_message(
            RelayClientMessage::CreateRoom {
                host_version: "test".to_string(),
            },
            &state,
            &host_tx,
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
            &RelayLimits::default(),
        )
        .unwrap();

        let message = ServerMessage::Error {
            message: "test".to_string(),
        };
        handle_relay_message(
            RelayClientMessage::HostBroadcast {
                room: room.clone(),
                message: message.clone(),
            },
            &state,
            &host_tx,
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
        let state = Arc::new(Mutex::new(RelayState::default()));
        let (host_tx, host_rx) = mpsc::channel();
        let (joiner_tx, joiner_rx) = mpsc::channel();
        let Some((room, _)) = handle_relay_message(
            RelayClientMessage::CreateRoom {
                host_version: "test".to_string(),
            },
            &state,
            &host_tx,
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
            &RelayLimits::default(),
        )
        .unwrap();

        cleanup_connection(&state, &room, ConnectionRole::Host);

        assert!(matches!(
            joiner_rx.recv().unwrap(),
            RelayServerMessage::RoomClosed { .. }
        ));
        assert!(!state.lock().unwrap().rooms.contains_key(&room));
    }

    #[test]
    fn relay_notifies_host_when_participant_disconnects() {
        let state = Arc::new(Mutex::new(RelayState::default()));
        let (host_tx, host_rx) = mpsc::channel();
        let (joiner_tx, _joiner_rx) = mpsc::channel();
        let Some((room, _)) = handle_relay_message(
            RelayClientMessage::CreateRoom {
                host_version: "test".to_string(),
            },
            &state,
            &host_tx,
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
        let state = Arc::new(Mutex::new(RelayState::default()));
        let (host_tx, host_rx) = mpsc::channel();
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
        let state = Arc::new(Mutex::new(RelayState::default()));
        let (host_tx, host_rx) = mpsc::channel();
        let (joiner_tx, _joiner_rx) = mpsc::channel();
        let (extra_tx, extra_rx) = mpsc::channel();
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
        let state = Arc::new(Mutex::new(RelayState::default()));
        let (host_tx, host_rx) = mpsc::channel();
        let limits = RelayLimits::default();
        let Some((room, _)) = handle_relay_message(
            RelayClientMessage::CreateRoom {
                host_version: "test".to_string(),
            },
            &state,
            &host_tx,
            &limits,
        )
        .unwrap() else {
            panic!("host should create a room");
        };
        let _ = host_rx.recv().unwrap();

        state
            .lock()
            .unwrap()
            .rooms
            .get_mut(&room)
            .unwrap()
            .last_activity = Instant::now() - Duration::from_secs(5);

        cleanup_stale_rooms(&state, Duration::from_secs(1));

        assert!(!state.lock().unwrap().rooms.contains_key(&room));
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
    fn idle_room_sweeper_removes_stale_rooms_without_new_messages() {
        let state = Arc::new(Mutex::new(RelayState::default()));
        let (host_tx, host_rx) = mpsc::channel();
        let limits = RelayLimits::default();
        let Some((room, _)) = handle_relay_message(
            RelayClientMessage::CreateRoom {
                host_version: "test".to_string(),
            },
            &state,
            &host_tx,
            &limits,
        )
        .unwrap() else {
            panic!("host should create a room");
        };
        let _ = host_rx.recv().unwrap();

        state
            .lock()
            .unwrap()
            .rooms
            .get_mut(&room)
            .unwrap()
            .last_activity = Instant::now() - Duration::from_secs(10);

        spawn_idle_room_sweeper(Arc::clone(&state), Duration::from_millis(1));
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            if !state.lock().unwrap().rooms.contains_key(&room) {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }

        panic!("idle sweeper did not remove stale room");
    }
}
