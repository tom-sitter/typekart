//! Loopback adapters for relay-backed online play.
//!
//! The current multiplayer game loop speaks the local TCP protocol. These
//! adapters keep that code path intact while carrying the same protocol through
//! the WebSocket relay. The relay still routes opaque messages; the host remains
//! authoritative for game rules.

use std::{
    collections::HashMap,
    io::{BufReader, ErrorKind},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

use anyhow::{bail, Context, Result};
use tungstenite::{connect, stream::MaybeTlsStream, Error as WebSocketError, Message, WebSocket};

use super::{
    protocol::{ClientMessage, PlayerId, ServerMessage},
    relay::{RelayClientMessage, RelayServerMessage, RoomCode},
    transport::{
        read_client_message, read_server_message, write_client_message, write_server_message,
    },
};

type RelaySocket = WebSocket<MaybeTlsStream<TcpStream>>;

#[derive(Debug, Clone)]
pub struct OnlineHostBridgeConfig {
    pub relay: String,
    pub local_server: SocketAddr,
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
    println!("Online room: {}", room.display());
    println!(
        "Join command: cargo run -- join-online --name PLAYER --relay {} --room {}",
        config.relay,
        room.display()
    );
    set_plain_nonblocking(&mut websocket)?;

    let (outbound_tx, outbound_rx) = mpsc::channel();
    let mut participants = HashMap::<PlayerId, TcpStream>::new();

    loop {
        drain_relay_outbound(&mut websocket, &outbound_rx)?;
        match websocket.read() {
            Ok(Message::Text(text)) => {
                let message = decode_relay_message(&text)?;
                handle_host_relay_message(
                    message,
                    &room,
                    config.local_server,
                    &outbound_tx,
                    &mut participants,
                )?;
            }
            Ok(Message::Close(_)) | Err(WebSocketError::ConnectionClosed) => break,
            Ok(Message::Ping(payload)) => websocket.send(Message::Pong(payload))?,
            Ok(_) => {}
            Err(WebSocketError::Io(error)) if error.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
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
            client_version: env!("CARGO_PKG_VERSION").to_string(),
        },
    )?;

    let relay_player_id = wait_for_join_welcome(&mut websocket, &mut local_stream)?;
    set_plain_nonblocking(&mut websocket)?;

    let read_stream = local_stream
        .try_clone()
        .context("failed to clone local join stream for reading")?;
    let room_for_reader = config.room.clone();
    let (outbound_tx, outbound_rx) = mpsc::channel();
    thread::spawn(move || {
        forward_local_client_to_relay(read_stream, room_for_reader, relay_player_id, outbound_tx);
    });

    loop {
        drain_relay_outbound(&mut websocket, &outbound_rx)?;
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
) -> Result<()> {
    match message {
        RelayServerMessage::JoinForwarded {
            room: joined_room,
            pending_player_id,
            name,
            ..
        } if &joined_room == room => {
            let mut local_stream = TcpStream::connect(local_server).with_context(|| {
                format!("failed to connect relay joiner to local host at {local_server}")
            })?;
            write_client_message(
                &mut local_stream,
                &ClientMessage::Hello {
                    name,
                    client_version: env!("CARGO_PKG_VERSION").to_string(),
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
                thread::spawn(move || {
                    forward_local_server_to_relay(
                        reader_stream,
                        room_for_reader,
                        pending_player_id,
                        outbound_tx,
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
            if let Some(local_stream) = participants.get_mut(&player_id) {
                write_client_message(local_stream, &message)
                    .context("failed to forward relay client message to local host")?;
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
) {
    let mut reader = BufReader::new(stream);
    loop {
        match read_server_message(&mut reader) {
            Ok(Some(message)) => {
                if send_relay_channel(
                    &outbound_tx,
                    RelayClientMessage::HostToClient {
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
            Ok(None) => break,
            Err(_) => continue,
        }
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
            Ok(None) => break,
            Err(_) => continue,
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
                    write_server_message(local_stream, &ServerMessage::Error { message })?;
                    bail!("relay rejected join");
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
        _ => bail!("online relay currently requires a plain ws:// relay URL"),
    }
}

fn drain_relay_outbound(
    websocket: &mut RelaySocket,
    outbound_rx: &Receiver<RelayClientMessage>,
) -> Result<()> {
    while let Ok(message) = outbound_rx.try_recv() {
        send_relay_message(websocket, &message)?;
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

fn decode_relay_message(text: &str) -> Result<RelayServerMessage> {
    serde_json::from_str(text).context("failed to decode relay message")
}
