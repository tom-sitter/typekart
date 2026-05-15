//! Minimal TCP join client for Milestone 4.
//!
//! The current client performs the first protocol handshake and prints lobby
//! snapshots. Later slices will send key input and render race snapshots.

use std::{
    io::{BufRead, BufReader, Write},
    net::SocketAddr,
};

use anyhow::{Context, Result};

use super::protocol::{ClientMessage, ServerMessage, decode_server_message, encode_client_message};

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

    let mut line = String::new();
    let mut reader = BufReader::new(stream);
    reader
        .read_line(&mut line)
        .context("failed to read server welcome")?;

    match decode_server_message(line.trim_end()).context("failed to decode server response")? {
        ServerMessage::Welcome {
            player_id,
            assigned_color,
        } => {
            println!(
                "Joined TypeKart server as player {} ({assigned_color:?})",
                player_id.0
            );
        }
        ServerMessage::Error { message } => {
            println!("Server rejected join: {message}");
        }
        other => {
            println!("Unexpected server response: {other:?}");
        }
    }

    for line in reader.lines() {
        let line = line.context("failed to read server message")?;
        match decode_server_message(line.trim_end()).context("failed to decode server message")? {
            ServerMessage::LobbySnapshot { players, host_id } => {
                println!("Lobby:");
                for player in players {
                    println!(
                        "  {}: {} ({:?}){}",
                        player.id.0,
                        player.name,
                        player.color,
                        if player.id == host_id { " host" } else { "" }
                    );
                }
            }
            ServerMessage::RaceEvent { message } => println!("{message}"),
            ServerMessage::Error { message } => println!("Server error: {message}"),
            other => println!("Received: {other:?}"),
        }
    }

    println!("Disconnected from server");

    Ok(())
}
