//! Minimal TCP join client for Milestone 4.
//!
//! The current client performs the first protocol handshake and prints lobby
//! snapshots. Later slices will send key input and render race snapshots.

use std::{
    io::{self, BufRead, BufReader, Write},
    net::SocketAddr,
    sync::{Arc, Mutex},
    thread,
};

use anyhow::{Context, Result};

use super::protocol::{
    ClientMessage, ClientSequence, NetworkRacePhase, ProtocolKey, ServerMessage,
    decode_server_message, encode_client_message,
};

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
            return Ok(());
        }
        other => {
            println!("Unexpected server response: {other:?}");
            return Ok(());
        }
    }

    println!("Lobby commands: ready, unready, quit");
    println!("After the race starts, submit typed text one line at a time.");

    let phase = Arc::new(Mutex::new(NetworkRacePhase::WaitingForHost));
    let reader_phase = Arc::clone(&phase);

    thread::spawn(move || {
        for line in reader.lines() {
            let Ok(line) = line else {
                break;
            };
            match decode_server_message(line.trim_end()) {
                Ok(ServerMessage::LobbySnapshot { players, host_id }) => {
                    println!("Lobby:");
                    for player in players {
                        println!(
                            "  {}: {} ({:?}){}{}{}",
                            player.id.0,
                            player.name,
                            player.color,
                            if player.ready { " ready" } else { "" },
                            if player.connected {
                                ""
                            } else {
                                " disconnected"
                            },
                            if player.id == host_id { " host" } else { "" }
                        );
                    }
                }
                Ok(ServerMessage::RaceEvent { message }) => println!("{message}"),
                Ok(ServerMessage::RaceSnapshot(snapshot)) => {
                    *reader_phase.lock().expect("client phase poisoned") = snapshot.phase;
                    println!("Race: {}", format_phase(snapshot.phase));
                    print_track(&snapshot.track_words);
                    print_progress(&snapshot.track_words, &snapshot.players);
                    for event in snapshot.events {
                        println!("  {event}");
                    }
                }
                Ok(ServerMessage::Error { message }) => println!("Server error: {message}"),
                Ok(other) => println!("Received: {other:?}"),
                Err(error) => println!("Failed to decode server message: {error}"),
            }
        }

        println!("Disconnected from server");
    });

    let mut sequence = 1;
    for line in io::stdin().lock().lines() {
        let command = line.context("failed to read lobby command")?;

        if *phase.lock().expect("client phase poisoned") == NetworkRacePhase::Racing {
            if matches!(command.trim(), "quit" | "leave") {
                send_client_message(&mut stream, &ClientMessage::Leave)?;
                break;
            }

            if command.trim() == "backspace" {
                send_client_message(
                    &mut stream,
                    &ClientMessage::KeyInput {
                        sequence: ClientSequence(sequence),
                        key: ProtocolKey::Backspace,
                    },
                )?;
                sequence += 1;
                continue;
            }

            for ch in command.chars() {
                let key = if ch == ' ' {
                    ProtocolKey::Space
                } else {
                    ProtocolKey::Char(ch)
                };
                send_client_message(
                    &mut stream,
                    &ClientMessage::KeyInput {
                        sequence: ClientSequence(sequence),
                        key,
                    },
                )?;
                sequence += 1;
            }
            send_client_message(
                &mut stream,
                &ClientMessage::KeyInput {
                    sequence: ClientSequence(sequence),
                    key: ProtocolKey::Space,
                },
            )?;
            sequence += 1;
            continue;
        }

        let message = match command.trim() {
            "ready" => ClientMessage::SetReady { ready: true },
            "unready" => ClientMessage::SetReady { ready: false },
            "quit" | "leave" => ClientMessage::Leave,
            "" => continue,
            other => {
                println!("Unknown command: {other}");
                continue;
            }
        };

        send_client_message(&mut stream, &message)?;

        if matches!(message, ClientMessage::Leave) {
            break;
        }
    }

    println!("Left server");

    Ok(())
}

fn send_client_message(stream: &mut std::net::TcpStream, message: &ClientMessage) -> Result<()> {
    let encoded = encode_client_message(message).context("failed to encode client message")?;
    writeln!(stream, "{encoded}").context("failed to send client message")?;
    stream.flush().context("failed to flush client message")
}

fn print_track(track_words: &[String]) {
    if !track_words.is_empty() {
        println!("  track: {}", track_words.join(" "));
    }
}

fn print_progress(track_words: &[String], players: &[super::protocol::PlayerSnapshot]) {
    for player in players {
        let target = track_words
            .get(player.word_index)
            .map(|word| format!(" target='{word}'"))
            .unwrap_or_default();
        println!(
            "  {}: word={}{} input='{}'{}{}",
            player.name,
            player.word_index + 1,
            target,
            player.input,
            player
                .typo_index
                .map(|index| format!(" typo@{index}"))
                .unwrap_or_default(),
            if player.finished { " finished" } else { "" }
        );
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
