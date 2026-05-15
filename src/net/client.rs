//! Minimal TCP join client for Milestone 4.
//!
//! The current client performs the first protocol handshake and prints lobby
//! snapshots. Later slices will send key input and render race snapshots.

use std::{
    io::{self, BufRead, BufReader, Write},
    net::SocketAddr,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use anyhow::{Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode},
};

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
    println!("After the race starts, typing is sent one key at a time.");

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

    let _raw_mode = RawModeGuard::enable()?;
    let mut sequence = 1;
    let mut lobby_command = String::new();
    print!("> ");
    io::stdout().flush().context("failed to flush prompt")?;

    loop {
        if !event::poll(Duration::from_millis(50)).context("failed to poll terminal input")? {
            continue;
        }

        let Event::Key(key_event) = event::read().context("failed to read terminal input")? else {
            continue;
        };

        if key_event.kind != KeyEventKind::Press {
            continue;
        }

        if should_leave(key_event) {
            send_client_message(&mut stream, &ClientMessage::Leave)?;
            break;
        }

        if *phase.lock().expect("client phase poisoned") == NetworkRacePhase::Racing {
            if send_race_key(&mut stream, key_event, &mut sequence)? {
                break;
            }
            continue;
        }

        if handle_lobby_key(&mut stream, key_event, &mut lobby_command)? {
            break;
        }
    }

    println!("Left server");

    Ok(())
}

struct RawModeGuard;

impl RawModeGuard {
    fn enable() -> Result<Self> {
        enable_raw_mode().context("failed to enable raw terminal mode")?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
    }
}

fn should_leave(key_event: KeyEvent) -> bool {
    key_event.code == KeyCode::Esc
        || (key_event.code == KeyCode::Char('c')
            && key_event.modifiers.contains(KeyModifiers::CONTROL))
}

fn handle_lobby_key(
    stream: &mut std::net::TcpStream,
    key_event: KeyEvent,
    lobby_command: &mut String,
) -> Result<bool> {
    if is_enter_key(key_event) {
        println!();
        let should_leave = send_lobby_command(stream, lobby_command.trim())?;
        lobby_command.clear();
        if !should_leave {
            print!("> ");
            io::stdout().flush().context("failed to flush prompt")?;
        }
        return Ok(should_leave);
    }

    match key_event.code {
        KeyCode::Backspace => {
            if lobby_command.pop().is_some() {
                print!("\u{8} \u{8}");
                io::stdout().flush().context("failed to flush prompt")?;
            }
            Ok(false)
        }
        KeyCode::Char(ch)
            if key_event.modifiers.is_empty() || key_event.modifiers == KeyModifiers::SHIFT =>
        {
            lobby_command.push(ch);
            print!("{ch}");
            io::stdout().flush().context("failed to flush prompt")?;
            Ok(false)
        }
        _ => Ok(false),
    }
}

fn is_enter_key(key_event: KeyEvent) -> bool {
    matches!(
        key_event.code,
        KeyCode::Enter | KeyCode::Char('\n') | KeyCode::Char('\r')
    ) || (matches!(key_event.code, KeyCode::Char('j') | KeyCode::Char('m'))
        && key_event.modifiers.contains(KeyModifiers::CONTROL))
}

fn send_lobby_command(stream: &mut std::net::TcpStream, command: &str) -> Result<bool> {
    let message = match command {
        "ready" => ClientMessage::SetReady { ready: true },
        "unready" => ClientMessage::SetReady { ready: false },
        "quit" | "leave" => ClientMessage::Leave,
        "" => return Ok(false),
        other => {
            println!("Unknown command: {other}");
            return Ok(false);
        }
    };

    send_client_message(stream, &message)?;

    Ok(matches!(message, ClientMessage::Leave))
}

fn send_race_key(
    stream: &mut std::net::TcpStream,
    key_event: KeyEvent,
    sequence: &mut u64,
) -> Result<bool> {
    let key = match key_event.code {
        KeyCode::Char(' ') => Some(ProtocolKey::Space),
        KeyCode::Char(ch)
            if key_event.modifiers.is_empty() || key_event.modifiers == KeyModifiers::SHIFT =>
        {
            Some(ProtocolKey::Char(ch))
        }
        KeyCode::Backspace => Some(ProtocolKey::Backspace),
        KeyCode::Enter => None,
        _ => None,
    };

    let Some(key) = key else {
        return Ok(false);
    };

    send_client_message(
        stream,
        &ClientMessage::KeyInput {
            sequence: ClientSequence(*sequence),
            key,
        },
    )?;
    *sequence += 1;

    Ok(false)
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
