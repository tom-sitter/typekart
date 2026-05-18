//! Loopback adapters for relay-backed online play.
//!
//! The current multiplayer game loop speaks the local TCP protocol. These
//! adapters keep that code path intact while carrying the same protocol through
//! the WebSocket relay. The relay still routes opaque messages; the host remains
//! authoritative for game rules.

use std::{
    collections::{HashMap, HashSet},
    io::{BufReader, ErrorKind},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use tungstenite::{
    Error as WebSocketError, Message, WebSocket, connect, error::ProtocolError,
    stream::MaybeTlsStream,
};

use super::{
    protocol::{ClientMessage, PlayerId, ServerMessage},
    relay::{RelayClientMessage, RelayServerMessage, RoomCode},
    transport::{
        read_client_message, read_server_message, write_client_message, write_server_message,
    },
};

type RelaySocket = WebSocket<MaybeTlsStream<TcpStream>>;
const RELAY_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(20);

#[derive(Debug, Default)]
struct HostBroadcastDedupe {
    race_snapshots: HashSet<u64>,
    race_deltas: HashSet<u64>,
    race_results_sent: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HostRelayDelivery {
    Broadcast,
    Direct,
    Skip,
}

#[derive(Debug, Clone)]
pub struct OnlineHostBridgeConfig {
    pub relay: String,
    pub local_server: SocketAddr,
    pub ready_signal: Option<Sender<RoomCode>>,
}

#[derive(Debug, Clone)]
pub struct OnlineJoinProxyConfig {
    pub relay: String,
    pub room: RoomCode,
    pub name: String,
    pub ready_signal: Sender<SocketAddr>,
}

pub fn run_online_host_bridge(config: OnlineHostBridgeConfig) -> Result<()> {
    let mut websocket = connect_relay(&config.relay)?;
    send_relay_message(
        &mut websocket,
        &RelayClientMessage::CreateRoom {
            host_version: env!("CARGO_PKG_VERSION").to_string(),
        },
    )?;
    let room = wait_for_created_room(&mut websocket)?;
    if let Some(ready_signal) = config.ready_signal {
        let _ = ready_signal.send(room.clone());
    } else {
        println!("Online room: {}", room.display());
        println!(
            "Join command: typekart join --name PLAYER --relay {} --room {}",
            config.relay,
            room.display()
        );
    }
    set_plain_nonblocking(&mut websocket)?;

    let (outbound_tx, outbound_rx) = mpsc::channel();
    let mut participants = HashMap::<PlayerId, TcpStream>::new();
    let broadcast_dedupe = Arc::new(Mutex::new(HostBroadcastDedupe::default()));
    let mut last_keepalive = Instant::now();

    loop {
        drain_relay_outbound(&mut websocket, &outbound_rx)?;
        send_keepalive_if_due(&mut websocket, &mut last_keepalive)?;
        match websocket.read() {
            Ok(Message::Text(text)) => {
                let message = decode_relay_message(&text)?;
                handle_host_relay_message(
                    message,
                    &room,
                    config.local_server,
                    &outbound_tx,
                    &mut participants,
                    &broadcast_dedupe,
                )?;
            }
            Ok(Message::Close(_)) | Err(WebSocketError::ConnectionClosed) => break,
            Ok(Message::Ping(payload)) => websocket.send(Message::Pong(payload))?,
            Ok(_) => {}
            Err(WebSocketError::Io(error)) if error.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) if websocket_disconnect_error(&error) => break,
            Err(error) => return Err(error).context("failed to read relay message"),
        }
    }

    Ok(())
}

pub fn run_online_join_proxy(config: OnlineJoinProxyConfig) -> Result<()> {
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .context("failed to bind online join loopback proxy")?;
    let local_addr = listener
        .local_addr()
        .context("failed to read online join proxy address")?;
    let _ = config.ready_signal.send(local_addr);

    let (mut local_stream, _) = listener
        .accept()
        .context("failed to accept local join client")?;
    let hello = read_local_hello(&local_stream)?;

    let mut websocket = connect_relay(&config.relay)?;
    send_relay_message(
        &mut websocket,
        &RelayClientMessage::JoinRoom {
            room: config.room.clone(),
            name: join_name(&hello).unwrap_or(&config.name).to_string(),
            client_version: join_version(&hello)
                .unwrap_or(env!("CARGO_PKG_VERSION"))
                .to_string(),
        },
    )?;

    let relay_player_id = wait_for_join_welcome(&mut websocket, &mut local_stream)?;
    set_plain_nonblocking(&mut websocket)?;

    let read_stream = local_stream
        .try_clone()
        .context("failed to clone local join stream for reading")?;
    let room_for_reader = config.room.clone();
    let (outbound_tx, outbound_rx) = mpsc::channel();
    let mut last_keepalive = Instant::now();
    thread::spawn(move || {
        forward_local_client_to_relay(read_stream, room_for_reader, relay_player_id, outbound_tx);
    });

    loop {
        drain_relay_outbound(&mut websocket, &outbound_rx)?;
        send_keepalive_if_due(&mut websocket, &mut last_keepalive)?;
        match websocket.read() {
            Ok(Message::Text(text)) => {
                let message = decode_relay_message(&text)?;
                match message {
                    RelayServerMessage::HostToClient { message, .. }
                    | RelayServerMessage::HostBroadcast { message, .. } => {
                        write_server_message(&mut local_stream, &message)
                            .context("failed to forward relay server message to local client")?;
                    }
                    RelayServerMessage::Error { message } => {
                        write_server_message(&mut local_stream, &ServerMessage::Error { message })?;
                    }
                    RelayServerMessage::RoomClosed { reason } => {
                        write_server_message(
                            &mut local_stream,
                            &ServerMessage::Error {
                                message: format!("Room closed: {reason}"),
                            },
                        )?;
                        break;
                    }
                    _ => {}
                }
            }
            Ok(Message::Close(_)) | Err(WebSocketError::ConnectionClosed) => break,
            Ok(Message::Ping(payload)) => websocket.send(Message::Pong(payload))?,
            Ok(_) => {}
            Err(WebSocketError::Io(error)) if error.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) if websocket_disconnect_error(&error) => break,
            Err(error) => return Err(error).context("failed to read relay message"),
        }
    }

    Ok(())
}

fn handle_host_relay_message(
    message: RelayServerMessage,
    room: &RoomCode,
    local_server: SocketAddr,
    outbound_tx: &Sender<RelayClientMessage>,
    participants: &mut HashMap<PlayerId, TcpStream>,
    broadcast_dedupe: &Arc<Mutex<HostBroadcastDedupe>>,
) -> Result<()> {
    match message {
        RelayServerMessage::JoinForwarded {
            room: joined_room,
            pending_player_id,
            name,
            client_version,
        } if &joined_room == room => {
            let mut local_stream = TcpStream::connect(local_server).with_context(|| {
                format!("failed to connect relay joiner to local host at {local_server}")
            })?;
            write_client_message(
                &mut local_stream,
                &ClientMessage::Hello {
                    name,
                    client_version,
                },
            )?;
            let mut local_reader = BufReader::new(
                local_stream
                    .try_clone()
                    .context("failed to clone local participant stream for welcome")?,
            );
            let Some(welcome) = read_server_message(&mut local_reader)? else {
                bail!("local host closed before welcoming relay participant");
            };
            send_relay_channel(
                outbound_tx,
                RelayClientMessage::HostToClient {
                    room: room.clone(),
                    player_id: pending_player_id,
                    message: welcome.clone(),
                },
            )?;

            if matches!(welcome, ServerMessage::Welcome { .. }) {
                let reader_stream = local_stream
                    .try_clone()
                    .context("failed to clone local participant stream for forwarding")?;
                let room_for_reader = room.clone();
                let outbound_tx = outbound_tx.clone();
                let broadcast_dedupe = Arc::clone(broadcast_dedupe);
                thread::spawn(move || {
                    forward_local_server_to_relay(
                        reader_stream,
                        room_for_reader,
                        pending_player_id,
                        outbound_tx,
                        broadcast_dedupe,
                    );
                });
                participants.insert(pending_player_id, local_stream);
            }
        }
        RelayServerMessage::ClientToHost {
            room: routed_room,
            player_id,
            message,
        } if &routed_room == room => {
            if let Some(local_stream) = participants.get_mut(&player_id)
                && let Err(error) = write_client_message(local_stream, &message)
            {
                eprintln!(
                    "Dropping relay participant {} after local host write failed: {error:#}",
                    player_id.0
                );
                participants.remove(&player_id);
            }
        }
        RelayServerMessage::ParticipantDisconnected {
            room: disconnected_room,
            player_id,
        } if &disconnected_room == room => {
            if let Some(mut local_stream) = participants.remove(&player_id) {
                let _ = write_client_message(&mut local_stream, &ClientMessage::Leave);
                let _ = local_stream.shutdown(Shutdown::Both);
            }
        }
        RelayServerMessage::Error { message } => {
            eprintln!("Relay error: {message}");
        }
        _ => {}
    }

    Ok(())
}

fn forward_local_server_to_relay(
    stream: TcpStream,
    room: RoomCode,
    player_id: PlayerId,
    outbound_tx: Sender<RelayClientMessage>,
    broadcast_dedupe: Arc<Mutex<HostBroadcastDedupe>>,
) {
    let mut reader = BufReader::new(stream);
    loop {
        match read_server_message(&mut reader) {
            Ok(Some(message)) => {
                let relay_message = match host_relay_delivery(&message, &broadcast_dedupe) {
                    HostRelayDelivery::Broadcast => RelayClientMessage::HostBroadcast {
                        room: room.clone(),
                        message,
                    },
                    HostRelayDelivery::Direct => RelayClientMessage::HostToClient {
                        room: room.clone(),
                        player_id,
                        message,
                    },
                    HostRelayDelivery::Skip => continue,
                };
                if send_relay_channel(&outbound_tx, relay_message).is_err() {
                    break;
                }
            }
            Ok(None) => break,
            Err(_) => continue,
        }
    }
}

fn host_relay_delivery(
    message: &ServerMessage,
    broadcast_dedupe: &Arc<Mutex<HostBroadcastDedupe>>,
) -> HostRelayDelivery {
    let mut dedupe = broadcast_dedupe
        .lock()
        .expect("host broadcast dedupe poisoned");
    match message {
        ServerMessage::RaceSnapshot(snapshot)
            if dedupe.race_snapshots.insert(snapshot.sequence) =>
        {
            HostRelayDelivery::Broadcast
        }
        ServerMessage::RaceSnapshot(_) => HostRelayDelivery::Skip,
        ServerMessage::RaceDelta(delta) if dedupe.race_deltas.insert(delta.sequence) => {
            HostRelayDelivery::Broadcast
        }
        ServerMessage::RaceDelta(_) => HostRelayDelivery::Skip,
        ServerMessage::RaceResults { .. } if !dedupe.race_results_sent => {
            dedupe.race_results_sent = true;
            HostRelayDelivery::Broadcast
        }
        ServerMessage::RaceResults { .. } => HostRelayDelivery::Skip,
        _ => HostRelayDelivery::Direct,
    }
}

fn forward_local_client_to_relay(
    stream: TcpStream,
    room: RoomCode,
    player_id: PlayerId,
    outbound_tx: Sender<RelayClientMessage>,
) {
    let mut reader = BufReader::new(stream);
    loop {
        match read_client_message(&mut reader) {
            Ok(Some(message)) => {
                if send_relay_channel(
                    &outbound_tx,
                    RelayClientMessage::ClientToHost {
                        room: room.clone(),
                        player_id,
                        message,
                    },
                )
                .is_err()
                {
                    break;
                }
            }
            Ok(None) => {
                let _ = send_relay_channel(
                    &outbound_tx,
                    RelayClientMessage::ClientToHost {
                        room: room.clone(),
                        player_id,
                        message: ClientMessage::Leave,
                    },
                );
                break;
            }
            Err(_) => {
                let _ = send_relay_channel(
                    &outbound_tx,
                    RelayClientMessage::ClientToHost {
                        room: room.clone(),
                        player_id,
                        message: ClientMessage::Leave,
                    },
                );
                break;
            }
        }
    }
}

fn read_local_hello(stream: &TcpStream) -> Result<ClientMessage> {
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .context("failed to clone local join stream for hello")?,
    );
    let Some(message) = read_client_message(&mut reader).context("failed to read local hello")?
    else {
        bail!("local join client disconnected before hello");
    };
    Ok(message)
}

fn join_name(message: &ClientMessage) -> Option<&str> {
    match message {
        ClientMessage::Hello { name, .. } if !name.trim().is_empty() => Some(name.trim()),
        _ => None,
    }
}

fn join_version(message: &ClientMessage) -> Option<&str> {
    match message {
        ClientMessage::Hello { client_version, .. } if !client_version.trim().is_empty() => {
            Some(client_version.trim())
        }
        _ => None,
    }
}

fn wait_for_created_room(websocket: &mut RelaySocket) -> Result<RoomCode> {
    loop {
        match websocket
            .read()
            .context("failed to read room creation response")?
        {
            Message::Text(text) => match decode_relay_message(&text)? {
                RelayServerMessage::RoomCreated { room } => return Ok(room),
                RelayServerMessage::Error { message } => {
                    bail!("relay rejected room creation: {message}")
                }
                other => bail!("unexpected relay response while creating room: {other:?}"),
            },
            Message::Ping(payload) => websocket.send(Message::Pong(payload))?,
            Message::Close(_) => bail!("relay closed while creating room"),
            _ => {}
        }
    }
}

fn wait_for_join_welcome(
    websocket: &mut RelaySocket,
    local_stream: &mut TcpStream,
) -> Result<PlayerId> {
    loop {
        match websocket.read().context("failed to read join response")? {
            Message::Text(text) => match decode_relay_message(&text)? {
                RelayServerMessage::HostToClient {
                    player_id, message, ..
                } => {
                    write_server_message(local_stream, &message)?;
                    return Ok(player_id);
                }
                RelayServerMessage::Error { message } => {
                    write_server_message(
                        local_stream,
                        &ServerMessage::Error {
                            message: message.clone(),
                        },
                    )?;
                    bail!("relay rejected join: {message}");
                }
                RelayServerMessage::RoomClosed { reason } => {
                    write_server_message(
                        local_stream,
                        &ServerMessage::Error {
                            message: format!("Room closed: {reason}"),
                        },
                    )?;
                    bail!("room closed while joining");
                }
                other => bail!("unexpected relay response while joining: {other:?}"),
            },
            Message::Ping(payload) => websocket.send(Message::Pong(payload))?,
            Message::Close(_) => bail!("relay closed while joining"),
            _ => {}
        }
    }
}

fn connect_relay(relay: &str) -> Result<RelaySocket> {
    let (websocket, _) =
        connect(relay).with_context(|| format!("failed to connect to relay {relay}"))?;
    Ok(websocket)
}

fn set_plain_nonblocking(websocket: &mut RelaySocket) -> Result<()> {
    match websocket.get_mut() {
        MaybeTlsStream::Plain(stream) => stream
            .set_nonblocking(true)
            .context("failed to set relay websocket nonblocking"),
        MaybeTlsStream::NativeTls(stream) => stream
            .get_ref()
            .set_nonblocking(true)
            .context("failed to set TLS relay websocket nonblocking"),
        _ => bail!("unsupported relay websocket stream type"),
    }
}

fn drain_relay_outbound(
    websocket: &mut RelaySocket,
    outbound_rx: &Receiver<RelayClientMessage>,
) -> Result<()> {
    while let Ok(message) = outbound_rx.try_recv() {
        if let Err(error) = send_relay_message(websocket, &message) {
            if let Some(websocket_error) = error.downcast_ref::<WebSocketError>()
                && websocket_disconnect_error(websocket_error)
            {
                break;
            }
            return Err(error);
        }
    }
    Ok(())
}

fn send_keepalive_if_due(websocket: &mut RelaySocket, last_keepalive: &mut Instant) -> Result<()> {
    if last_keepalive.elapsed() >= RELAY_KEEPALIVE_INTERVAL {
        websocket
            .send(Message::Ping(Vec::new()))
            .context("failed to send relay keepalive ping")?;
        *last_keepalive = Instant::now();
    }
    Ok(())
}

fn send_relay_channel(
    outbound_tx: &Sender<RelayClientMessage>,
    message: RelayClientMessage,
) -> Result<()> {
    outbound_tx
        .send(message)
        .context("failed to queue relay message")
}

fn send_relay_message(websocket: &mut RelaySocket, message: &RelayClientMessage) -> Result<()> {
    let encoded = serde_json::to_string(message).context("failed to encode relay message")?;
    websocket
        .send(Message::Text(encoded))
        .context("failed to send relay message")
}

fn websocket_disconnect_error(error: &WebSocketError) -> bool {
    match error {
        WebSocketError::ConnectionClosed | WebSocketError::AlreadyClosed => true,
        WebSocketError::Protocol(ProtocolError::ResetWithoutClosingHandshake) => true,
        WebSocketError::Io(error) => matches!(
            error.kind(),
            ErrorKind::ConnectionReset
                | ErrorKind::ConnectionAborted
                | ErrorKind::BrokenPipe
                | ErrorKind::UnexpectedEof
        ),
        _ => false,
    }
}

fn decode_relay_message(text: &str) -> Result<RelayServerMessage> {
    serde_json::from_str(text).context("failed to decode relay message")
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        io::BufReader,
        net::{SocketAddr, TcpStream},
        sync::{Arc, Mutex, mpsc},
        thread,
        time::Duration,
    };

    use super::{
        HostBroadcastDedupe, HostRelayDelivery, OnlineHostBridgeConfig, OnlineJoinProxyConfig,
        handle_host_relay_message, host_relay_delivery, join_version, run_online_host_bridge,
        run_online_join_proxy,
    };
    use crate::{
        game::{
            ai::AiDifficulty, items::ItemRegistry, mods::ActiveModConfig, track::Track,
            words::WordSetDefinition,
        },
        net::{
            protocol::{ClientMessage, PlayerId, ServerMessage},
            relay::{RelayServerMessage, RoomCode},
            relay_server::{RelayConfig, run_relay},
            server::{HostConfig, run_host},
            transport::{read_client_message, read_server_message, write_client_message},
        },
    };

    #[test]
    fn join_version_uses_local_client_hello_version() {
        let message = ClientMessage::Hello {
            name: "alex".to_string(),
            client_version: "9.9.9".to_string(),
        };

        assert_eq!(join_version(&message), Some("9.9.9"));
    }

    #[test]
    fn online_bridge_allows_joiner_to_receive_host_welcome_through_relay() {
        let relay_addr = spawn_test_relay();
        let local_host = spawn_test_host();
        let (room_tx, room_rx) = mpsc::channel();
        let relay_url = format!("ws://{relay_addr}");

        {
            let relay_url = relay_url.clone();
            thread::spawn(move || {
                let _ = run_online_host_bridge(OnlineHostBridgeConfig {
                    relay: relay_url,
                    local_server: local_host,
                    ready_signal: Some(room_tx),
                });
            });
        }
        let room = room_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("online host bridge should create a room");

        let (proxy_tx, proxy_rx) = mpsc::channel();
        {
            let relay_url = relay_url.clone();
            thread::spawn(move || {
                let _ = run_online_join_proxy(OnlineJoinProxyConfig {
                    relay: relay_url,
                    room,
                    name: "alex".to_string(),
                    ready_signal: proxy_tx,
                });
            });
        }
        let proxy_addr = proxy_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("online join proxy should start");

        let mut client =
            TcpStream::connect(proxy_addr).expect("test client should connect to join proxy");
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("read timeout should be configurable");
        write_client_message(
            &mut client,
            &ClientMessage::Hello {
                name: "alex".to_string(),
                client_version: env!("CARGO_PKG_VERSION").to_string(),
            },
        )
        .expect("hello should be forwarded to relay");

        let mut reader = std::io::BufReader::new(client.try_clone().unwrap());
        let welcome = read_server_message(&mut reader)
            .expect("welcome should decode")
            .expect("welcome should arrive");
        assert!(matches!(welcome, ServerMessage::Welcome { .. }));

        write_client_message(&mut client, &ClientMessage::Leave).unwrap();
    }

    #[test]
    fn host_bridge_turns_relay_participant_disconnect_into_local_leave() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let participant_stream = TcpStream::connect(address).unwrap();
        let (host_side_stream, _) = listener.accept().unwrap();
        let room = RoomCode::parse("rocket-salad-tiger").unwrap();
        let player_id = PlayerId(2);
        let (outbound_tx, _outbound_rx) = mpsc::channel();
        let mut participants = HashMap::from([(player_id, participant_stream)]);
        let broadcast_dedupe = Arc::new(Mutex::new(HostBroadcastDedupe::default()));

        handle_host_relay_message(
            RelayServerMessage::ParticipantDisconnected {
                room: room.clone(),
                player_id,
            },
            &room,
            address,
            &outbound_tx,
            &mut participants,
            &broadcast_dedupe,
        )
        .unwrap();

        let mut reader = BufReader::new(host_side_stream);
        assert_eq!(
            read_client_message(&mut reader).unwrap(),
            Some(ClientMessage::Leave)
        );
        assert!(!participants.contains_key(&player_id));
    }

    #[test]
    fn host_bridge_broadcast_dedupe_skips_duplicate_shared_race_update() {
        let dedupe = Arc::new(Mutex::new(HostBroadcastDedupe::default()));
        let message = ServerMessage::RaceDelta(crate::net::protocol::RaceDeltaSnapshot {
            sequence: 9,
            phase: crate::net::protocol::NetworkRacePhase::Racing,
            bonuses: Vec::new(),
            players: Vec::new(),
            events: Vec::new(),
        });

        assert_eq!(
            host_relay_delivery(&message, &dedupe),
            HostRelayDelivery::Broadcast
        );
        assert_eq!(
            host_relay_delivery(&message, &dedupe),
            HostRelayDelivery::Skip
        );
    }

    #[test]
    fn online_bridge_treats_common_websocket_disconnects_as_normal_close() {
        assert!(super::websocket_disconnect_error(
            &tungstenite::Error::ConnectionClosed
        ));
        assert!(super::websocket_disconnect_error(
            &tungstenite::Error::Protocol(
                tungstenite::error::ProtocolError::ResetWithoutClosingHandshake
            )
        ));
        assert!(super::websocket_disconnect_error(&tungstenite::Error::Io(
            std::io::Error::new(std::io::ErrorKind::ConnectionReset, "reset")
        )));
    }

    fn spawn_test_relay() -> SocketAddr {
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = run_relay(RelayConfig {
                bind: SocketAddr::from(([127, 0, 0, 1], 0)),
                ready_signal: Some(tx),
                limits: Default::default(),
            });
        });
        rx.recv_timeout(Duration::from_secs(2))
            .expect("relay should start")
    }

    fn spawn_test_host() -> SocketAddr {
        let word_set = WordSetDefinition::load_builtin_default().unwrap();
        let item_registry = ItemRegistry::builtin();
        let active_mod_config = ActiveModConfig::new(&word_set, &item_registry, None);
        let track = Track::generate(&word_set.words, 6).unwrap();
        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            let _ = run_host(HostConfig {
                bind: SocketAddr::from(([127, 0, 0, 1], 0)),
                host_name: None,
                track,
                word_list: word_set.words,
                item_registry,
                active_mod_config,
                max_players: 6,
                ai_racer_count: 0,
                ai_difficulty: AiDifficulty::Easy,
                ready_signal: Some(tx),
                console_logging: false,
                debug_log: None,
            });
        });

        rx.recv_timeout(Duration::from_secs(2))
            .expect("host should start")
    }
}
