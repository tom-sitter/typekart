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
use serde::Serialize;
use serde_json::Value;
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
            "Join command: typekart join --relay {} --room {}",
            config.relay,
            room.display()
        );
    }
    set_relay_read_timeout(&mut websocket)?;

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
            Err(WebSocketError::Io(error))
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
            {
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

    let mut websocket = connect_relay(&relay_join_url(&config.relay, &config.room))?;
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
    set_relay_read_timeout(&mut websocket)?;

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
                        let message = decode_server_payload(message)?;
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
            Err(WebSocketError::Io(error))
                if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
            {
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
                    message: encode_payload(&welcome)?,
                },
            )?;

            if matches!(welcome, ServerMessage::Welcome { .. }) {
                let room_for_reader = room.clone();
                let outbound_tx = outbound_tx.clone();
                let broadcast_dedupe = Arc::clone(broadcast_dedupe);
                thread::spawn(move || {
                    forward_local_server_to_relay(
                        local_reader,
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
            let message = decode_client_payload(message)?;
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
    mut reader: BufReader<TcpStream>,
    room: RoomCode,
    player_id: PlayerId,
    outbound_tx: Sender<RelayClientMessage>,
    broadcast_dedupe: Arc<Mutex<HostBroadcastDedupe>>,
) {
    loop {
        match read_server_message(&mut reader) {
            Ok(Some(message)) => {
                let relay_message = match host_relay_delivery(&message, &broadcast_dedupe) {
                    HostRelayDelivery::Broadcast => RelayClientMessage::HostBroadcast {
                        room: room.clone(),
                        message: match encode_payload(&message) {
                            Ok(message) => message,
                            Err(_) => continue,
                        },
                    },
                    HostRelayDelivery::Direct => RelayClientMessage::HostToClient {
                        room: room.clone(),
                        player_id,
                        message: match encode_payload(&message) {
                            Ok(message) => message,
                            Err(_) => continue,
                        },
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
                        message: match encode_payload(&message) {
                            Ok(message) => message,
                            Err(_) => break,
                        },
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
                        message: match encode_payload(&ClientMessage::Leave) {
                            Ok(message) => message,
                            Err(_) => break,
                        },
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
                        message: match encode_payload(&ClientMessage::Leave) {
                            Ok(message) => message,
                            Err(_) => break,
                        },
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
    let mut pending_messages = Vec::<ServerMessage>::new();
    loop {
        match websocket.read().context("failed to read join response")? {
            Message::Text(text) => match decode_relay_message(&text)? {
                RelayServerMessage::HostToClient {
                    player_id, message, ..
                } => {
                    let message = decode_server_payload(message)?;
                    if matches!(message, ServerMessage::Welcome { .. }) {
                        write_server_message(local_stream, &message)?;
                        for pending_message in pending_messages {
                            write_server_message(local_stream, &pending_message)?;
                        }
                        return Ok(player_id);
                    } else {
                        pending_messages.push(message);
                    }
                }
                RelayServerMessage::HostBroadcast { message, .. } => {
                    pending_messages.push(decode_server_payload(message)?);
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

fn set_relay_read_timeout(websocket: &mut RelaySocket) -> Result<()> {
    const RELAY_READ_TIMEOUT: Duration = Duration::from_millis(10);
    match websocket.get_mut() {
        MaybeTlsStream::Plain(stream) => stream
            .set_read_timeout(Some(RELAY_READ_TIMEOUT))
            .context("failed to set relay websocket read timeout"),
        MaybeTlsStream::NativeTls(stream) => stream
            .get_ref()
            .set_read_timeout(Some(RELAY_READ_TIMEOUT))
            .context("failed to set TLS relay websocket read timeout"),
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

fn encode_payload(message: &impl Serialize) -> Result<Value> {
    serde_json::to_value(message).context("failed to encode relay payload")
}

fn decode_client_payload(message: Value) -> Result<ClientMessage> {
    serde_json::from_value(message).context("failed to decode client relay payload")
}

fn decode_server_payload(message: Value) -> Result<ServerMessage> {
    serde_json::from_value(message).context("failed to decode server relay payload")
}

#[cfg(test)]
mod tests;
