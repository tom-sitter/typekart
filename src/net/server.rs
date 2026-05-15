//! Minimal TCP host for Milestone 4.
//!
//! This is intentionally only a handshake/lobby skeleton. It proves that a
//! TypeKart host can accept TCP clients and speak the JSON protocol before we
//! wire the server to authoritative race snapshots.

use std::{
    io::{BufRead, BufReader, Write},
    net::{SocketAddr, TcpListener, TcpStream},
};

use anyhow::{Context, Result, bail};

use super::protocol::{
    AssignedColor, ClientMessage, LobbyPlayer, PlayerId, ServerMessage, decode_client_message,
    encode_server_message,
};

const COLOR_ROTATION: [AssignedColor; 6] = [
    AssignedColor::Cyan,
    AssignedColor::Red,
    AssignedColor::Green,
    AssignedColor::Blue,
    AssignedColor::Yellow,
    AssignedColor::Magenta,
];

#[derive(Debug, Clone)]
pub struct HostConfig {
    pub bind: SocketAddr,
    pub host_name: String,
    pub max_players: usize,
}

struct ConnectedClient {
    player_id: PlayerId,
    stream: TcpStream,
}

pub fn run_host(config: HostConfig) -> Result<()> {
    if config.max_players == 0 || config.max_players > COLOR_ROTATION.len() {
        bail!("max players must be between 1 and {}", COLOR_ROTATION.len());
    }

    let listener = TcpListener::bind(config.bind)
        .with_context(|| format!("failed to bind host socket at {}", config.bind))?;
    let local_addr = listener
        .local_addr()
        .context("failed to read host address")?;
    let mut players = vec![LobbyPlayer {
        id: PlayerId(1),
        name: config.host_name,
        color: COLOR_ROTATION[0],
        ready: false,
        connected: true,
    }];
    let mut clients = Vec::new();
    let mut next_player_id = 2;

    println!("TypeKart host listening on {local_addr}");
    println!("Waiting for joiners. Press Ctrl-C to stop.");

    for stream in listener.incoming() {
        let stream = stream.context("failed to accept client connection")?;
        let peer = stream.peer_addr().ok();

        if players.len() >= config.max_players {
            send_server_message(
                stream,
                &ServerMessage::Error {
                    message: "Lobby is full".to_string(),
                },
            )?;
            continue;
        }

        match handle_join_handshake(
            stream,
            PlayerId(next_player_id),
            COLOR_ROTATION[players.len()],
        ) {
            Ok((player, client_stream)) => {
                println!(
                    "{} joined as player {}{}",
                    player.name,
                    player.id.0,
                    peer.map(|addr| format!(" from {addr}")).unwrap_or_default()
                );
                clients.push(ConnectedClient {
                    player_id: player.id,
                    stream: client_stream,
                });
                players.push(player);
                print_lobby_snapshot(&players);
                broadcast_lobby_snapshot(&mut clients, &players)?;
                next_player_id += 1;
            }
            Err(error) => {
                eprintln!("Rejected connection: {error:#}");
            }
        }
    }

    Ok(())
}

fn handle_join_handshake(
    stream: TcpStream,
    player_id: PlayerId,
    assigned_color: AssignedColor,
) -> Result<(LobbyPlayer, TcpStream)> {
    let mut line = String::new();
    {
        let mut reader = BufReader::new(
            stream
                .try_clone()
                .context("failed to clone client stream for reading")?,
        );
        reader
            .read_line(&mut line)
            .context("failed to read client hello")?;
    }

    let message =
        decode_client_message(line.trim_end()).context("failed to decode client hello")?;
    let ClientMessage::Hello { name, .. } = message else {
        send_server_message(
            stream,
            &ServerMessage::Error {
                message: "Expected hello message".to_string(),
            },
        )?;
        bail!("client sent non-hello first message");
    };

    if name.trim().is_empty() {
        send_server_message(
            stream,
            &ServerMessage::Error {
                message: "Name cannot be empty".to_string(),
            },
        )?;
        bail!("client sent empty name");
    }

    let mut write_stream = stream
        .try_clone()
        .context("failed to clone client stream for writing")?;
    write_server_message(
        &mut write_stream,
        &ServerMessage::Welcome {
            player_id,
            assigned_color,
        },
    )?;

    Ok((
        LobbyPlayer {
            id: player_id,
            name,
            color: assigned_color,
            ready: false,
            connected: true,
        },
        write_stream,
    ))
}

fn print_lobby_snapshot(players: &[LobbyPlayer]) {
    println!("Lobby:");
    for player in players {
        println!(
            "  {}: {} ({:?}){}",
            player.id.0,
            player.name,
            player.color,
            if player.id == PlayerId(1) {
                " host"
            } else {
                ""
            }
        );
    }
}

fn broadcast_lobby_snapshot(
    clients: &mut Vec<ConnectedClient>,
    players: &[LobbyPlayer],
) -> Result<()> {
    let snapshot = ServerMessage::LobbySnapshot {
        players: players.to_vec(),
        host_id: PlayerId(1),
    };

    let mut failed_clients = Vec::new();
    for client in clients.iter_mut() {
        if let Err(error) = write_server_message(&mut client.stream, &snapshot) {
            eprintln!(
                "Failed to send lobby snapshot to player {}: {error:#}",
                client.player_id.0
            );
            failed_clients.push(client.player_id);
        }
    }

    clients.retain(|client| !failed_clients.contains(&client.player_id));
    Ok(())
}

fn send_server_message(mut stream: TcpStream, message: &ServerMessage) -> Result<()> {
    write_server_message(&mut stream, message)
}

fn write_server_message(stream: &mut TcpStream, message: &ServerMessage) -> Result<()> {
    let encoded = encode_server_message(message).context("failed to encode server message")?;
    writeln!(stream, "{encoded}").context("failed to write server message")?;
    stream.flush().context("failed to flush server message")
}

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        thread,
    };

    use super::{
        AssignedColor, ConnectedClient, PlayerId, broadcast_lobby_snapshot, handle_join_handshake,
    };
    use crate::net::protocol::{
        ClientMessage, LobbyPlayer, ServerMessage, decode_server_message, encode_client_message,
    };

    #[test]
    fn host_handshake_accepts_hello() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();

        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle_join_handshake(stream, PlayerId(2), AssignedColor::Red)
                .unwrap()
                .0
        });

        let mut client = std::net::TcpStream::connect(address).unwrap();
        let hello = encode_client_message(&ClientMessage::Hello {
            name: "alex".to_string(),
            client_version: env!("CARGO_PKG_VERSION").to_string(),
        })
        .unwrap();
        writeln!(client, "{hello}").unwrap();

        let player = server.join().unwrap();

        assert_eq!(player.id, PlayerId(2));
        assert_eq!(player.name, "alex");
        assert_eq!(player.color, AssignedColor::Red);
    }

    #[test]
    fn lobby_snapshot_broadcasts_to_connected_clients() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();

        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let (_, client_stream) =
                handle_join_handshake(stream, PlayerId(2), AssignedColor::Red).unwrap();
            let mut clients = vec![ConnectedClient {
                player_id: PlayerId(2),
                stream: client_stream,
            }];
            let players = vec![
                LobbyPlayer {
                    id: PlayerId(1),
                    name: "host".to_string(),
                    color: AssignedColor::Cyan,
                    ready: false,
                    connected: true,
                },
                LobbyPlayer {
                    id: PlayerId(2),
                    name: "alex".to_string(),
                    color: AssignedColor::Red,
                    ready: false,
                    connected: true,
                },
            ];

            broadcast_lobby_snapshot(&mut clients, &players).unwrap();
        });

        let mut client = std::net::TcpStream::connect(address).unwrap();
        let hello = encode_client_message(&ClientMessage::Hello {
            name: "alex".to_string(),
            client_version: env!("CARGO_PKG_VERSION").to_string(),
        })
        .unwrap();
        writeln!(client, "{hello}").unwrap();

        let mut reader = BufReader::new(client);
        let mut welcome_line = String::new();
        reader.read_line(&mut welcome_line).unwrap();
        let mut snapshot_line = String::new();
        reader.read_line(&mut snapshot_line).unwrap();

        assert!(matches!(
            decode_server_message(welcome_line.trim_end()).unwrap(),
            ServerMessage::Welcome { .. }
        ));
        assert!(matches!(
            decode_server_message(snapshot_line.trim_end()).unwrap(),
            ServerMessage::LobbySnapshot { ref players, .. } if players.len() == 2
        ));
        server.join().unwrap();
    }
}
