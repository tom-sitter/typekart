//! Local development WebSocket relay.
//!
//! This relay routes TypeKart protocol envelopes by room. It deliberately does
//! not understand race rules; the host remains authoritative.

use std::{
    collections::HashMap,
    net::{SocketAddr, TcpListener},
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use anyhow::{Context, Result};
use tungstenite::{accept, Error as WebSocketError, Message};

use super::{
    protocol::PlayerId,
    relay::{RelayClientMessage, RelayServerMessage, RoomCode},
};

#[derive(Debug, Clone)]
pub struct RelayConfig {
    pub bind: SocketAddr,
    pub ready_signal: Option<Sender<SocketAddr>>,
}

#[derive(Debug, Default)]
struct RelayState {
    rooms: HashMap<RoomCode, RelayRoom>,
}

#[derive(Debug)]
struct RelayRoom {
    host: Sender<RelayServerMessage>,
    participants: HashMap<PlayerId, Sender<RelayServerMessage>>,
    next_pending_player_id: u64,
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
    println!("TypeKart relay listening on ws://{local_addr}");
    if let Some(ready_signal) = config.ready_signal {
        let _ = ready_signal.send(local_addr);
    }

    let state = Arc::new(Mutex::new(RelayState::default()));
    for stream in listener.incoming() {
        let stream = stream.context("failed to accept relay connection")?;
        let state = Arc::clone(&state);
        thread::spawn(move || {
            if let Err(error) = handle_connection(stream, state) {
                eprintln!("Relay connection ended: {error:#}");
            }
        });
    }

    Ok(())
}

fn handle_connection(stream: std::net::TcpStream, state: Arc<Mutex<RelayState>>) -> Result<()> {
    let peer = stream.peer_addr().ok();
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
                let message = serde_json::from_str::<RelayClientMessage>(&text)
                    .context("failed to decode relay message")?;
                if let Some(role) = handle_relay_message(message, &state, &tx)? {
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
            Err(WebSocketError::ConnectionClosed) => break,
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

fn drain_outbound(
    websocket: &mut tungstenite::WebSocket<std::net::TcpStream>,
    rx: &Receiver<RelayServerMessage>,
) -> Result<()> {
    while let Ok(message) = rx.try_recv() {
        let encoded = serde_json::to_string(&message).context("failed to encode relay message")?;
        websocket
            .send(Message::Text(encoded))
            .context("failed to send relay websocket message")?;
    }
    Ok(())
}

fn handle_relay_message(
    message: RelayClientMessage,
    state: &Arc<Mutex<RelayState>>,
    sender: &Sender<RelayServerMessage>,
) -> Result<Option<(RoomCode, ConnectionRole)>> {
    match message {
        RelayClientMessage::CreateRoom { .. } => {
            let room = create_room(state, sender.clone());
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
                            message: "Room not found".to_string(),
                        })
                        .ok();
                    return Ok(None);
                };
                let pending_player_id = PlayerId(room_state.next_pending_player_id);
                room_state.next_pending_player_id += 1;
                room_state
                    .participants
                    .insert(pending_player_id, sender.clone());
                room_state.host.send(RelayServerMessage::JoinForwarded {
                    room: room.clone(),
                    pending_player_id,
                    name,
                    client_version,
                })?;
                pending_player_id
            };
            Ok(Some((room, ConnectionRole::Participant(pending_player_id))))
        }
        RelayClientMessage::ClientToHost {
            room,
            player_id,
            message,
        } => {
            let state = state.lock().expect("relay state poisoned");
            if let Some(room_state) = state.rooms.get(&room) {
                room_state.host.send(RelayServerMessage::ClientToHost {
                    room,
                    player_id,
                    message,
                })?;
            }
            Ok(None)
        }
        RelayClientMessage::HostToClient {
            room,
            player_id,
            message,
        } => {
            let state = state.lock().expect("relay state poisoned");
            if let Some(room_state) = state.rooms.get(&room) {
                if let Some(participant) = room_state.participants.get(&player_id) {
                    participant.send(RelayServerMessage::HostToClient {
                        room,
                        player_id,
                        message,
                    })?;
                }
            }
            Ok(None)
        }
        RelayClientMessage::HostBroadcast { room, message } => {
            let state = state.lock().expect("relay state poisoned");
            if let Some(room_state) = state.rooms.get(&room) {
                for participant in room_state.participants.values() {
                    participant.send(RelayServerMessage::HostBroadcast {
                        room: room.clone(),
                        message: message.clone(),
                    })?;
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

fn create_room(state: &Arc<Mutex<RelayState>>, host: Sender<RelayServerMessage>) -> RoomCode {
    let mut state = state.lock().expect("relay state poisoned");
    loop {
        let room = RoomCode::generate();
        if !state.rooms.contains_key(&room) {
            state.rooms.insert(
                room.clone(),
                RelayRoom {
                    host,
                    participants: HashMap::new(),
                    next_pending_player_id: 2,
                },
            );
            return room;
        }
    }
}

fn cleanup_connection(state: &Arc<Mutex<RelayState>>, room: &RoomCode, role: ConnectionRole) {
    let mut state = state.lock().expect("relay state poisoned");
    match role {
        ConnectionRole::Host => close_room(&mut state, room),
        ConnectionRole::Participant(player_id) => {
            if let Some(room_state) = state.rooms.get_mut(room) {
                room_state.participants.remove(&player_id);
            }
        }
    }
}

fn close_room(state: &mut RelayState, room: &RoomCode) {
    let Some(room_state) = state.rooms.remove(room) else {
        return;
    };
    for participant in room_state.participants.values() {
        let _ = participant.send(RelayServerMessage::RoomClosed {
            reason: "Host disconnected".to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{mpsc, Arc, Mutex};

    use super::{cleanup_connection, handle_relay_message, ConnectionRole, RelayState};
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
        )
        .unwrap();

        cleanup_connection(&state, &room, ConnectionRole::Host);

        assert!(matches!(
            joiner_rx.recv().unwrap(),
            RelayServerMessage::RoomClosed { .. }
        ));
        assert!(state.lock().unwrap().rooms.get(&room).is_none());
    }
}
