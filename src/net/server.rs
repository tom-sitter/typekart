//! Authoritative TCP host for multiplayer races.

use std::{
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
    time::{Duration, Instant},
};

#[cfg(test)]
use crate::game::lobby::{
    connected_player_count, first_available_color, new_human_lobby_player, unique_lobby_name,
};
#[cfg(test)]
use crate::game::race::{RacePlayerId, RaceState};
use crate::game::{
    ai::AiDifficulty,
    items::ItemRegistry,
    mods::ActiveModConfig,
    track::{Track, WordList},
};
use anyhow::{Context, Result};

#[cfg(test)]
use super::host_lifecycle::build_race_result_rows as build_network_race_result_rows;
use super::log::{SharedNetworkLog, push_network_log};
#[cfg(test)]
use super::protocol::AssignedColor;
#[cfg(test)]
use super::protocol::RaceResultRow;
use super::protocol::ServerMessage;
#[cfg(test)]
use super::protocol::{NetworkRacePhase, PlayerId};
use super::transport::write_server_message as write_framed_server_message;

mod host_accept;
mod host_ai;
mod host_bonus;
mod host_broadcast;
mod host_client;
mod host_commands;
mod host_disconnect;
mod host_handshake;
mod host_input;
mod host_items;
mod host_join;
mod host_lobby;
mod host_phase;
mod host_race;
mod host_snapshots;
mod host_state;
mod host_util;
#[cfg(test)]
use host_ai::NetworkAiRacer;
#[cfg(test)]
use host_ai::set_lobby_ai_difficulty;
#[cfg(test)]
use host_ai::{add_lobby_ai_racer, add_network_ai_racers, advance_network_ai_racers};
#[cfg(test)]
use host_bonus::apply_network_key_input;
use host_broadcast::{
    broadcast_lobby_snapshot, broadcast_race_delta, broadcast_race_results_once,
    broadcast_race_snapshot,
};
#[cfg(test)]
use host_client::handle_client_messages;
#[cfg(test)]
use host_commands::update_host_ready;
use host_commands::{has_embedded_host_player, spawn_host_command_loop};
#[cfg(test)]
use host_disconnect::handle_player_disconnect;
#[cfg(test)]
use host_handshake::{read_join_hello, welcome_joiner};
#[cfg(test)]
use host_items::activate_network_pickup;
#[cfg(test)]
use host_lobby::{cleanup_disconnected_waiting_players, remove_lobby_player, rename_lobby_player};
#[cfg(test)]
use host_phase::{reconcile_phase_after_disconnect, return_to_lobby};
#[cfg(test)]
use host_race::{reset_race_from_lobby, update_race_status};
#[cfg(test)]
use host_snapshots::build_race_snapshot;
use host_state::{ConnectedClient, HostState, build_initial_host_state};
use host_util::{print_lobby_snapshot, validate_host_capacity};

const POST_FIRST_FINISH_TIMEOUT: Duration = Duration::from_secs(30);
const RACE_SNAPSHOT_INTERVAL: Duration = Duration::from_millis(100);
static SERVER_CONSOLE_LOGGING: AtomicBool = AtomicBool::new(true);

macro_rules! server_println {
    ($($arg:tt)*) => {
        if SERVER_CONSOLE_LOGGING.load(std::sync::atomic::Ordering::Relaxed) {
            println!($($arg)*);
        }
    };
}

fn print_server_line(message: impl AsRef<str>) {
    server_println!("{}", message.as_ref());
}

#[derive(Debug, Clone)]
pub struct HostConfig {
    pub bind: SocketAddr,
    pub host_name: Option<String>,
    pub track: Track,
    pub word_list: WordList,
    pub item_registry: ItemRegistry,
    pub active_mod_config: ActiveModConfig,
    pub max_players: usize,
    pub ai_racer_count: usize,
    pub ai_difficulty: AiDifficulty,
    pub ready_signal: Option<Sender<SocketAddr>>,
    pub console_logging: bool,
    pub debug_log: Option<SharedNetworkLog>,
}

pub fn run_host(mut config: HostConfig) -> Result<()> {
    SERVER_CONSOLE_LOGGING.store(config.console_logging, Ordering::Relaxed);

    validate_host_capacity(config.max_players, config.ai_racer_count)?;

    let listener = TcpListener::bind(config.bind)
        .with_context(|| format!("failed to bind host socket at {}", config.bind))?;
    let local_addr = listener
        .local_addr()
        .context("failed to read host address")?;
    if let Some(ready_signal) = config.ready_signal.take() {
        let _ = ready_signal.send(local_addr);
    }
    push_network_log(
        &config.debug_log,
        format!(
            "server listening addr={local_addr} max_players={} words={} ai_racers={} ai_difficulty={}",
            config.max_players,
            config.track.len(),
            config.ai_racer_count,
            config.ai_difficulty.name()
        ),
    );
    push_network_log(&config.debug_log, config.active_mod_config.log_summary());

    let initial_state = build_initial_host_state(config);
    let state = Arc::new(Mutex::new(initial_state.state));

    server_println!("TypeKart host listening on {local_addr}");
    if has_embedded_host_player(&state) {
        server_println!("Host lobby commands: start, lobby, ready, unready");
        spawn_host_command_loop(Arc::clone(&state));
    }
    server_println!("Waiting for joiners. Press Ctrl-C to stop.");

    host_accept::run_accept_loop(listener, state, initial_state.next_player_id)
}

fn push_event(state: &mut HostState, event: String) {
    state.events.push(event);
    const EVENT_LIMIT: usize = 20;
    if state.events.len() > EVENT_LIMIT {
        let excess = state.events.len() - EVENT_LIMIT;
        state.events.drain(0..excess);
    }
}

fn expire_bonus_cooldowns(state: &mut HostState, now: Instant) -> usize {
    let track = &state.race.track;
    state.bonuses.expire_cooldowns(track, now)
}

#[cfg(test)]
fn client_is_in_current_race(race: &RaceState, player_id: PlayerId) -> bool {
    race.players
        .iter()
        .any(|player| player.id == RacePlayerId(player_id.0))
}

#[cfg(test)]
fn build_race_result_rows(state: &HostState, now: Instant) -> Vec<RaceResultRow> {
    build_network_race_result_rows(&state.race, &state.runtime.lifecycle.placements, now)
}

fn send_server_message(mut stream: TcpStream, message: &ServerMessage) -> Result<()> {
    write_server_message(&mut stream, message)
}

fn write_server_message(stream: &mut TcpStream, message: &ServerMessage) -> Result<()> {
    write_framed_server_message(stream, message)
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
        thread,
        time::{Duration, Instant},
    };

    use super::{
        AssignedColor, ConnectedClient, HostState, NetworkAiRacer, NetworkRacePhase,
        POST_FIRST_FINISH_TIMEOUT, PlayerId, activate_network_pickup, add_lobby_ai_racer,
        add_network_ai_racers, advance_network_ai_racers, apply_network_key_input,
        broadcast_lobby_snapshot, broadcast_race_results_once, broadcast_race_snapshot,
        build_race_result_rows, build_race_snapshot, cleanup_disconnected_waiting_players,
        client_is_in_current_race, connected_player_count, first_available_color,
        handle_client_messages, handle_player_disconnect, new_human_lobby_player, push_event,
        read_join_hello, reconcile_phase_after_disconnect, remove_lobby_player,
        rename_lobby_player, reset_race_from_lobby, return_to_lobby, set_lobby_ai_difficulty,
        unique_lobby_name, update_host_ready, update_race_status, validate_host_capacity,
        welcome_joiner,
    };
    use crate::game::{
        ai::AiDifficulty,
        bonus::{BonusChoice, BonusChoiceStatus, BonusPoint, BonusState},
        effects::ActiveEffect,
        item_effects::{RaceImpactCueKind, RaceItemEffectState},
        items::{HeldItem, ItemActivation, ItemDefinition, ItemPickup, ItemRegistry},
        mods::{ActiveModConfig, ContentMetadata},
        race::{PlayerColorId, RacePlayerId, RaceRuntimeState, RaceState},
        stats::TypingStats,
        track::{Track, WordList},
        typing::KeyAction,
        words::WordSetDefinition,
    };
    use crate::net::protocol::{
        ClientMessage, LobbyPlayer, PlayerKind, RaceResultStatus, ServerMessage,
        decode_server_message, encode_client_message,
    };

    #[test]
    fn host_handshake_accepts_hello() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();

        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let hello = read_join_hello(&stream).unwrap();
            welcome_joiner(&stream, PlayerId(2), AssignedColor::Red).unwrap();
            LobbyPlayer {
                id: PlayerId(2),
                name: hello.name,
                kind: PlayerKind::Human,
                color: AssignedColor::Red,
                ready: false,
                connected: true,
                ai_difficulty: None,
                ai_wpm: None,
            }
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
    fn host_handshake_rejects_empty_client_version() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();

        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            read_join_hello(&stream).unwrap_err();
        });

        let mut client = std::net::TcpStream::connect(address).unwrap();
        let hello = encode_client_message(&ClientMessage::Hello {
            name: "alex".to_string(),
            client_version: "".to_string(),
        })
        .unwrap();
        writeln!(client, "{hello}").unwrap();

        let mut reader = BufReader::new(client);
        let mut error_line = String::new();
        reader.read_line(&mut error_line).unwrap();

        assert!(matches!(
            decode_server_message(error_line.trim_end()).unwrap(),
            ServerMessage::Error { ref message } if message == "Client version cannot be empty"
        ));
        server.join().unwrap();
    }

    #[test]
    fn duplicate_human_names_get_numbered_suffixes() {
        let players = [
            lobby_player(PlayerId(1), "tom", PlayerKind::Human, true),
            lobby_player(PlayerId(2), "Tom2", PlayerKind::Human, true),
            lobby_player(PlayerId(3), "tom3", PlayerKind::Human, false),
        ];

        assert_eq!(unique_lobby_name(players.iter(), "tom"), "tom3");
        assert_eq!(unique_lobby_name(players.iter(), "alex"), "alex");
    }

    #[test]
    fn lobby_player_can_rename_with_unique_suffix() {
        let mut state = test_host_state(NetworkRacePhase::WaitingForHost);

        rename_lobby_player(&mut state, PlayerId(2), "host").unwrap();

        assert_eq!(state.players[1].name, "host2");
        assert_eq!(state.race.players[1].name, "host2");
        assert!(
            state
                .events
                .iter()
                .any(|event| event == "alex renamed to host2")
        );
    }

    #[test]
    fn lobby_player_cannot_rename_during_active_race() {
        let mut state = test_host_state(NetworkRacePhase::Racing);

        assert!(rename_lobby_player(&mut state, PlayerId(2), "alex").is_err());
    }

    #[test]
    fn first_human_joiner_is_host_and_starts_ready() {
        assert!(new_human_lobby_player(PlayerId(1), "host", AssignedColor::Cyan).ready);
        assert!(!new_human_lobby_player(PlayerId(2), "joiner", AssignedColor::Red).ready);
    }

    #[test]
    fn lobby_snapshot_broadcasts_to_connected_clients() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();

        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            read_join_hello(&stream).unwrap();
            let client_stream = welcome_joiner(&stream, PlayerId(2), AssignedColor::Red).unwrap();
            let mut state = HostState {
                clients: vec![ConnectedClient {
                    player_id: PlayerId(2),
                    stream: client_stream,
                }],
                players: test_players(false),
                race: test_race_state(),
                ai_racers: HashMap::new(),
                word_list: test_word_list(),
                bonuses: test_bonus_state(),
                item_registry: ItemRegistry::builtin(),
                active_mod_config: test_active_mod_config(),
                max_players: 6,
                ai_difficulty: AiDifficulty::Easy,
                runtime: RaceRuntimeState::new(),
                phase: NetworkRacePhase::WaitingForHost,
                snapshot_sequence: 0,
                events: Vec::new(),
                debug_log: None,
                race_results_sent: false,
            };

            broadcast_lobby_snapshot(&mut state).unwrap();
        });

        let mut client = std::net::TcpStream::connect(address).unwrap();
        send_hello(&mut client);

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

    #[test]
    fn set_ready_updates_lobby_and_broadcasts_snapshot() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();

        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            read_join_hello(&stream).unwrap();
            let client_stream = welcome_joiner(&stream, PlayerId(2), AssignedColor::Red).unwrap();
            let read_stream = stream;
            let state = Arc::new(Mutex::new(HostState {
                clients: vec![ConnectedClient {
                    player_id: PlayerId(2),
                    stream: client_stream,
                }],
                players: test_players(false),
                race: test_race_state(),
                ai_racers: HashMap::new(),
                word_list: test_word_list(),
                bonuses: test_bonus_state(),
                item_registry: ItemRegistry::builtin(),
                active_mod_config: test_active_mod_config(),
                max_players: 6,
                ai_difficulty: AiDifficulty::Easy,
                runtime: RaceRuntimeState::new(),
                phase: NetworkRacePhase::WaitingForHost,
                snapshot_sequence: 0,
                events: Vec::new(),
                debug_log: None,
                race_results_sent: false,
            }));
            handle_client_messages(PlayerId(2), read_stream, Arc::clone(&state));
            state
        });

        let mut client = std::net::TcpStream::connect(address).unwrap();
        send_hello(&mut client);
        let mut reader = BufReader::new(client.try_clone().unwrap());
        let mut welcome_line = String::new();
        reader.read_line(&mut welcome_line).unwrap();
        let ready = encode_client_message(&ClientMessage::SetReady { ready: true }).unwrap();
        writeln!(client, "{ready}").unwrap();
        let mut snapshot_line = String::new();
        reader.read_line(&mut snapshot_line).unwrap();
        drop(client);
        drop(reader);

        let state = server.join().unwrap();
        let state = state.lock().unwrap();

        assert!(state.players.iter().all(|player| player.id != PlayerId(2)));
        assert!(
            state
                .race
                .players
                .iter()
                .all(|player| player.id != RacePlayerId(2))
        );
        assert!(matches!(
            decode_server_message(snapshot_line.trim_end()).unwrap(),
            ServerMessage::LobbySnapshot { ref players, ref events, .. }
                if players.iter().any(|player| player.id == PlayerId(2) && player.ready)
                    && events.iter().any(|event| event == "alex ready")
        ));
    }

    #[test]
    fn disconnected_players_do_not_count_against_capacity_or_color_assignment() {
        let players = vec![
            LobbyPlayer {
                id: PlayerId(1),
                name: "host".to_string(),
                kind: PlayerKind::Human,
                color: AssignedColor::Cyan,
                ready: true,
                connected: true,
                ai_difficulty: None,
                ai_wpm: None,
            },
            LobbyPlayer {
                id: PlayerId(2),
                name: "alex".to_string(),
                kind: PlayerKind::Human,
                color: AssignedColor::Red,
                ready: false,
                connected: false,
                ai_difficulty: None,
                ai_wpm: None,
            },
        ];

        assert_eq!(connected_player_count(&players), 1);
        assert_eq!(first_available_color(&players), Some(AssignedColor::Red));
    }

    #[test]
    fn network_ai_racers_are_added_as_ready_bots() {
        let now = Instant::now();
        let mut race = RaceState::new(Track::new(vec!["one".to_string(), "two".to_string()]));
        let mut players = Vec::new();

        let ai_racers = add_network_ai_racers(&mut race, &mut players, 2, AiDifficulty::Easy, now);

        assert_eq!(players.len(), 2);
        assert_eq!(race.players.len(), 2);
        assert_eq!(ai_racers.len(), 2);
        assert_eq!(players[0].id, PlayerId(2));
        assert_eq!(players[0].name, "ai-1");
        assert_eq!(players[0].kind, PlayerKind::Bot);
        assert_eq!(players[0].color, AssignedColor::Red);
        assert!(players[0].ready);
        assert!(players[0].connected);
        assert_eq!(players[1].id, PlayerId(3));
        assert_eq!(players[1].color, AssignedColor::Green);
    }

    #[test]
    fn network_ai_racers_reserve_human_host_slot() {
        assert!(validate_host_capacity(6, 5).is_ok());
        assert!(validate_host_capacity(6, 6).is_err());
        assert!(validate_host_capacity(0, 0).is_err());
        assert!(validate_host_capacity(7, 0).is_err());
    }

    #[test]
    fn network_ai_racer_advances_from_wpm_budget() {
        let now = Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.players[1].kind = PlayerKind::Bot;
        state.ai_racers.insert(
            PlayerId(2),
            NetworkAiRacer {
                difficulty: AiDifficulty::Easy,
                words_per_minute: 60.0,
                char_budget: 0.0,
                last_update: now,
            },
        );

        advance_network_ai_racers(&mut state, now + Duration::from_secs(1));

        let ai = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(ai.state.word_index, 1);
        assert_eq!(ai.state.input, "t");
    }

    #[test]
    fn network_fogged_ai_racer_hesitates_from_reduced_wpm_budget() {
        let now = Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.players[1].kind = PlayerKind::Bot;
        state.race.players[1].state.fogged_word_index = Some(0);
        state.race.players[1].state.fogged_until = Some(now + Duration::from_secs(5));
        state.ai_racers.insert(
            PlayerId(2),
            NetworkAiRacer {
                difficulty: AiDifficulty::Easy,
                words_per_minute: 60.0,
                char_budget: 0.0,
                last_update: now,
            },
        );

        advance_network_ai_racers(&mut state, now + Duration::from_secs(1));

        let ai = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(ai.state.word_index, 0);
        assert_eq!(ai.state.input, "one");
    }

    #[test]
    fn network_ai_racer_does_not_type_while_stunned() {
        let now = Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.players[1].kind = PlayerKind::Bot;
        state.ai_racers.insert(
            PlayerId(2),
            NetworkAiRacer {
                difficulty: AiDifficulty::Easy,
                words_per_minute: 120.0,
                char_budget: 0.0,
                last_update: now,
            },
        );
        state.runtime.player_effects.insert(
            RacePlayerId(2),
            RaceItemEffectState {
                stunned_until: Some(now + Duration::from_secs(1)),
                ..Default::default()
            },
        );

        advance_network_ai_racers(&mut state, now + Duration::from_millis(500));

        let ai = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(ai.state.word_index, 0);
        assert_eq!(ai.state.input, "");
        assert_eq!(state.ai_racers.get(&PlayerId(2)).unwrap().char_budget, 0.0);
    }

    #[test]
    fn network_ai_racer_does_not_advance_or_accrue_budget_during_countdown() {
        let now = Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Countdown {
            remaining_seconds: 3,
        });
        state.players[1].kind = PlayerKind::Bot;
        state.ai_racers.insert(
            PlayerId(2),
            NetworkAiRacer {
                difficulty: AiDifficulty::Easy,
                words_per_minute: 120.0,
                char_budget: 4.0,
                last_update: now - Duration::from_secs(10),
            },
        );

        advance_network_ai_racers(&mut state, now);

        let bot = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(bot.state.word_index, 0);
        assert_eq!(bot.state.input, "");
        let ai = state.ai_racers.get(&PlayerId(2)).unwrap();
        assert_eq!(ai.char_budget, 0.0);
        assert_eq!(ai.last_update, now);

        state.phase = NetworkRacePhase::Racing;
        advance_network_ai_racers(&mut state, now + Duration::from_millis(50));

        let bot = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(bot.state.word_index, 0);
        assert_eq!(bot.state.input, "");
        assert!(state.ai_racers.get(&PlayerId(2)).unwrap().char_budget < 1.0);
    }

    #[test]
    fn network_ai_racer_can_claim_bonus_and_activate_shield() {
        let now = Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.players[1].kind = PlayerKind::Bot;
        state.item_registry = test_single_item_registry(ItemPickup::Shield);
        state.ai_racers.insert(
            PlayerId(2),
            NetworkAiRacer {
                difficulty: AiDifficulty::Easy,
                words_per_minute: 60.0,
                char_budget: 0.0,
                last_update: now,
            },
        );
        state.race.players[1].state.word_index = 1;

        advance_network_ai_racers(&mut state, now);

        let bot = state.race.player(RacePlayerId(2)).unwrap();
        assert!(bot.state.has_active_shield(now));
        assert!(
            state.bonuses.points[0]
                .choices
                .iter()
                .any(|choice| matches!(choice.status, BonusChoiceStatus::Cooldown { .. }))
        );
        assert_eq!(state.runtime.spent_bonus_gaps.get(&PlayerId(2)), Some(&0));
        assert!(state.events.iter().any(|event| event == "alex got Shield"));
    }

    #[test]
    fn network_ai_racer_can_claim_bonus_and_reset_human_with_banana() {
        let now = Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.players[1].kind = PlayerKind::Bot;
        state.item_registry = test_single_item_registry(ItemPickup::Held(HeldItem::Banana));
        state.ai_racers.insert(
            PlayerId(2),
            NetworkAiRacer {
                difficulty: AiDifficulty::Easy,
                words_per_minute: 60.0,
                char_budget: 0.0,
                last_update: now,
            },
        );
        state.race.players[0].state.word_index = 1;
        state.race.players[1].state.word_index = 1;

        advance_network_ai_racers(&mut state, now);

        assert!(
            state
                .runtime
                .player_effects
                .get(&RacePlayerId(1))
                .and_then(|effects| effects.stunned_until)
                .is_none()
        );
        assert!(
            state
                .runtime
                .player_effects
                .get(&RacePlayerId(1))
                .and_then(|effects| effects.impact_cue)
                .is_some_and(|cue| cue.kind == RaceImpactCueKind::Banana && cue.until > now)
        );
        assert!(
            state
                .runtime
                .player_effects
                .get(&RacePlayerId(2))
                .and_then(|effects| effects.item_cue.as_ref())
                .is_some_and(|cue| cue.until > now)
        );
        assert!(state.events.iter().any(|event| event == "alex hit host"));
    }

    #[test]
    fn human_banana_resets_network_ai_typing_budget() {
        let now = Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.players[1].kind = PlayerKind::Bot;
        state.ai_racers.insert(
            PlayerId(2),
            NetworkAiRacer {
                difficulty: AiDifficulty::Easy,
                words_per_minute: 60.0,
                char_budget: 3.0,
                last_update: now,
            },
        );

        activate_network_pickup(
            &mut state,
            PlayerId(1),
            ItemPickup::Held(HeldItem::Banana),
            now,
        );

        assert_eq!(state.ai_racers.get(&PlayerId(2)).unwrap().char_budget, 0.0);
        assert!(
            state
                .runtime
                .player_effects
                .get(&RacePlayerId(2))
                .and_then(|effects| effects.stunned_until)
                .is_some_and(|until| until > now)
        );
    }

    #[test]
    fn waiting_cleanup_removes_disconnected_players_from_next_race_roster() {
        let mut state = test_host_state(NetworkRacePhase::WaitingForHost);
        state.players[1].connected = false;
        state.race.players[1].connected = false;

        cleanup_disconnected_waiting_players(&mut state);

        assert_eq!(state.players.len(), 1);
        assert_eq!(state.race.players.len(), 1);
        assert!(state.players.iter().all(|player| player.id != PlayerId(2)));
        assert!(
            state
                .race
                .players
                .iter()
                .all(|player| player.id != RacePlayerId(2))
        );
    }

    #[test]
    fn waiting_cleanup_keeps_disconnected_players_during_active_race() {
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.players[1].connected = false;
        state.race.players[1].connected = false;

        cleanup_disconnected_waiting_players(&mut state);

        assert_eq!(state.players.len(), 2);
        assert_eq!(state.race.players.len(), 2);
    }

    #[test]
    fn joiner_disconnect_is_removed_from_waiting_lobby_with_event() {
        let mut state = test_host_state(NetworkRacePhase::WaitingForHost);

        let closed_game =
            handle_player_disconnect(&mut state, PlayerId(2), std::time::Instant::now());

        assert!(!closed_game);
        assert_eq!(state.players.len(), 1);
        assert_eq!(state.race.players.len(), 1);
        assert!(state.players.iter().all(|player| player.id != PlayerId(2)));
        assert!(
            state
                .events
                .iter()
                .any(|event| event == "alex disconnected")
        );
    }

    #[test]
    fn host_disconnect_sends_game_closed_to_joiners() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let mut remote_client = std::net::TcpStream::connect(address).unwrap();
        let (server_stream, _) = listener.accept().unwrap();
        let mut state = test_host_state(NetworkRacePhase::WaitingForHost);
        state.clients.push(ConnectedClient {
            player_id: PlayerId(2),
            stream: server_stream,
        });

        let closed_game =
            handle_player_disconnect(&mut state, PlayerId(1), std::time::Instant::now());

        let mut reader = BufReader::new(&mut remote_client);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();

        assert!(closed_game);
        assert!(state.clients.is_empty());
        assert!(matches!(
            decode_server_message(line.trim_end()).unwrap(),
            ServerMessage::Error { ref message } if message == "Game closed: host left"
        ));
        assert!(state.players.iter().all(|player| !player.connected));
    }

    #[test]
    fn rematch_rebuilds_race_from_connected_lobby_players() {
        let mut state = test_host_state(NetworkRacePhase::Finished);
        state.runtime.lifecycle.placements = vec![RacePlayerId(2), RacePlayerId(1)];
        state.race_results_sent = true;
        state.players.push(LobbyPlayer {
            id: PlayerId(3),
            name: "casey".to_string(),
            kind: PlayerKind::Human,
            color: AssignedColor::Green,
            ready: true,
            connected: true,
            ai_difficulty: None,
            ai_wpm: None,
        });

        reset_race_from_lobby(&mut state).unwrap();

        assert_eq!(state.phase, NetworkRacePhase::WaitingForHost);
        assert_eq!(state.race.players.len(), 3);
        assert!(
            state
                .race
                .players
                .iter()
                .any(|player| player.id == RacePlayerId(3))
        );
        assert!(state.runtime.lifecycle.placements.is_empty());
        assert!(!state.race_results_sent);
        assert!(state.events.is_empty());
    }

    #[test]
    fn race_rebuild_excludes_unready_lobby_players() {
        let mut state = test_host_state(NetworkRacePhase::WaitingForHost);
        state.players[0].ready = true;
        state.players[1].ready = false;

        reset_race_from_lobby(&mut state).unwrap();

        assert!(client_is_in_current_race(&state.race, PlayerId(1)));
        assert!(!client_is_in_current_race(&state.race, PlayerId(2)));
        assert!(state.players.iter().any(|player| player.id == PlayerId(2)));
    }

    #[test]
    fn late_joiner_is_not_part_of_current_race_until_rematch() {
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.players.push(LobbyPlayer {
            id: PlayerId(3),
            name: "casey".to_string(),
            kind: PlayerKind::Human,
            color: AssignedColor::Green,
            ready: true,
            connected: true,
            ai_difficulty: None,
            ai_wpm: None,
        });

        assert!(!client_is_in_current_race(&state.race, PlayerId(3)));

        state.phase = NetworkRacePhase::Finished;
        reset_race_from_lobby(&mut state).unwrap();

        assert!(client_is_in_current_race(&state.race, PlayerId(3)));
    }

    #[test]
    fn return_to_lobby_resets_finished_race() {
        let mut state = test_host_state(NetworkRacePhase::Finished);
        state.runtime.lifecycle.placements = vec![RacePlayerId(2), RacePlayerId(1)];
        state.race_results_sent = true;

        return_to_lobby(&mut state).unwrap();

        assert_eq!(state.phase, NetworkRacePhase::WaitingForHost);
        assert!(state.runtime.lifecycle.placements.is_empty());
        assert!(!state.race_results_sent);
        assert_eq!(state.race.players.len(), 2);
    }

    #[test]
    fn return_to_lobby_cancels_active_race() {
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.snapshot_sequence = 7;
        state.runtime.lifecycle.placements = vec![RacePlayerId(1)];
        state
            .runtime
            .player_effects
            .insert(RacePlayerId(1), Default::default());

        return_to_lobby(&mut state).unwrap();

        assert_eq!(state.snapshot_sequence, 8);
        assert_eq!(state.phase, NetworkRacePhase::WaitingForHost);
        assert!(state.runtime.lifecycle.placements.is_empty());
        assert!(state.runtime.player_effects.is_empty());
        assert!(state.events.iter().any(|event| event == "Race cancelled"));
        assert_eq!(state.race.players.len(), 2);
    }

    #[test]
    fn rematch_keeps_network_ai_racers_and_resets_ai_timing_state() {
        let now = Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Finished);
        state.players[1].kind = PlayerKind::Bot;
        state.ai_racers.insert(
            PlayerId(2),
            NetworkAiRacer {
                difficulty: AiDifficulty::Easy,
                words_per_minute: 60.0,
                char_budget: 5.0,
                last_update: now - Duration::from_secs(5),
            },
        );
        finish_player(&mut state, RacePlayerId(2), now);

        reset_race_from_lobby(&mut state).unwrap();

        assert!(client_is_in_current_race(&state.race, PlayerId(2)));
        assert_eq!(
            state.race.player(RacePlayerId(2)).unwrap().state.word_index,
            0
        );
        let ai = state.ai_racers.get(&PlayerId(2)).unwrap();
        assert_eq!(ai.char_budget, 0.0);
        assert!(ai.last_update >= now);
    }

    #[test]
    fn host_can_add_remove_and_retune_ai_in_lobby() {
        let mut state = test_host_state(NetworkRacePhase::WaitingForHost);

        add_lobby_ai_racer(&mut state).unwrap();

        let ai_player = state
            .players
            .iter()
            .find(|player| player.kind == PlayerKind::Bot)
            .unwrap()
            .clone();
        assert!(state.ai_racers.contains_key(&ai_player.id));
        assert!(client_is_in_current_race(&state.race, ai_player.id));

        set_lobby_ai_difficulty(&mut state, Some(ai_player.id), AiDifficulty::Hard).unwrap();

        let ai_player = state
            .players
            .iter()
            .find(|player| player.id == ai_player.id)
            .unwrap();
        assert_eq!(ai_player.ai_difficulty, Some(AiDifficulty::Hard.into()));
        assert!(state.ai_racers.get(&ai_player.id).unwrap().words_per_minute >= 55.0);

        let ai_player_id = ai_player.id;
        remove_lobby_player(&mut state, ai_player_id).unwrap();

        assert!(!state.ai_racers.contains_key(&ai_player_id));
        assert!(!state.players.iter().any(|player| player.id == ai_player_id));
        assert!(!client_is_in_current_race(&state.race, ai_player_id));
    }

    #[test]
    fn host_can_kick_joiner_in_lobby_but_not_self() {
        let mut state = test_host_state(NetworkRacePhase::WaitingForHost);

        assert!(remove_lobby_player(&mut state, PlayerId(1)).is_err());
        remove_lobby_player(&mut state, PlayerId(2)).unwrap();

        assert!(!state.players.iter().any(|player| player.id == PlayerId(2)));
        assert!(!client_is_in_current_race(&state.race, PlayerId(2)));
    }

    #[test]
    fn host_ready_command_updates_host_player() {
        let state = Arc::new(Mutex::new(HostState {
            clients: Vec::new(),
            players: vec![LobbyPlayer {
                id: PlayerId(1),
                name: "host".to_string(),
                kind: PlayerKind::Human,
                color: AssignedColor::Cyan,
                ready: true,
                connected: true,
                ai_difficulty: None,
                ai_wpm: None,
            }],
            race: test_race_state(),
            ai_racers: HashMap::new(),
            word_list: test_word_list(),
            bonuses: test_bonus_state(),
            item_registry: ItemRegistry::builtin(),
            active_mod_config: test_active_mod_config(),
            max_players: 6,
            ai_difficulty: AiDifficulty::Easy,
            runtime: RaceRuntimeState::new(),
            phase: NetworkRacePhase::WaitingForHost,
            snapshot_sequence: 0,
            events: Vec::new(),
            debug_log: None,
            race_results_sent: false,
        }));

        update_host_ready(&state, true);

        let state = state.lock().unwrap();
        assert!(state.players[0].ready);
    }

    #[test]
    fn race_snapshot_includes_phase_players_and_recent_events() {
        let mut state = HostState {
            clients: Vec::new(),
            players: test_players(true),
            race: test_race_state(),
            ai_racers: HashMap::new(),
            word_list: test_word_list(),
            bonuses: test_bonus_state(),
            item_registry: ItemRegistry::builtin(),
            active_mod_config: test_active_mod_config(),
            max_players: 6,
            ai_difficulty: AiDifficulty::Easy,
            runtime: RaceRuntimeState::new(),
            phase: NetworkRacePhase::Countdown {
                remaining_seconds: 3,
            },
            snapshot_sequence: 0,
            events: Vec::new(),
            debug_log: None,
            race_results_sent: false,
        };
        push_event(&mut state, "Countdown started".to_string());

        let snapshot = build_race_snapshot(&mut state);

        assert_eq!(
            snapshot.phase,
            NetworkRacePhase::Countdown {
                remaining_seconds: 3
            }
        );
        assert_eq!(snapshot.sequence, 1);
        assert_eq!(snapshot.players.len(), 2);
        assert_eq!(snapshot.track_words, vec!["one", "two"]);
        assert_eq!(snapshot.bonuses.len(), 1);
        assert_eq!(snapshot.bonuses[0].after_word_index, 0);
        assert_eq!(snapshot.bonuses[0].choices[0].word, "dash");
        assert_eq!(snapshot.events, vec!["Countdown started"]);
    }

    #[test]
    fn race_snapshot_reflects_applied_key_input() {
        let mut state = HostState {
            clients: Vec::new(),
            players: test_players(true),
            race: test_race_state(),
            ai_racers: HashMap::new(),
            word_list: test_word_list(),
            bonuses: test_bonus_state(),
            item_registry: ItemRegistry::builtin(),
            active_mod_config: test_active_mod_config(),
            max_players: 6,
            ai_difficulty: AiDifficulty::Easy,
            runtime: RaceRuntimeState::new(),
            phase: NetworkRacePhase::Racing,
            snapshot_sequence: 0,
            events: Vec::new(),
            debug_log: None,
            race_results_sent: false,
        };

        state
            .race
            .apply_key_input(
                RacePlayerId(2),
                KeyAction::Char('o'),
                std::time::Instant::now(),
            )
            .unwrap();

        let snapshot = build_race_snapshot(&mut state);
        let alex = snapshot
            .players
            .iter()
            .find(|player| player.id == PlayerId(2))
            .unwrap();

        assert_eq!(alex.word_index, 0);
        assert_eq!(alex.input, "o");
        assert_eq!(alex.typo_index, None);
    }

    #[test]
    fn race_snapshots_are_broadcast_to_lobby_observers() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();

        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let observer_stream = stream.try_clone().unwrap();
            let mut state = test_host_state(NetworkRacePhase::Racing);
            state.clients.push(ConnectedClient {
                player_id: PlayerId(9),
                stream: observer_stream,
            });
            broadcast_race_snapshot(&mut state).unwrap();
        });

        let client = std::net::TcpStream::connect(address).unwrap();
        let mut reader = BufReader::new(client);
        let mut snapshot_line = String::new();
        reader.read_line(&mut snapshot_line).unwrap();

        assert!(matches!(
            decode_server_message(snapshot_line.trim_end()).unwrap(),
            ServerMessage::RaceSnapshot(_)
        ));
        server.join().unwrap();
    }

    #[test]
    fn network_key_input_can_start_bonus_attempt_at_gap() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.race.players[1].state.word_index = 1;

        apply_network_key_input(&mut state, PlayerId(2), KeyAction::Char('d'), now);

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(alex.state.word_index, 1);
        assert_eq!(alex.state.input, "d");
        assert_eq!(alex.state.typo_index, None);
        assert!(state.runtime.bonus_attempts.contains_key(&PlayerId(2)));
    }

    #[test]
    fn network_bonus_claim_places_choice_on_cooldown() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.race.players[1].state.word_index = 1;

        for action in [
            KeyAction::Char('d'),
            KeyAction::Char('a'),
            KeyAction::Char('s'),
            KeyAction::Char('h'),
            KeyAction::Space,
        ] {
            apply_network_key_input(&mut state, PlayerId(2), action, now);
        }

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(alex.state.input, "");
        assert!(!state.runtime.bonus_attempts.contains_key(&PlayerId(2)));
        assert_eq!(state.runtime.spent_bonus_gaps.get(&PlayerId(2)), Some(&0));
        assert!(matches!(
            state.bonuses.points[0].choices[0].status,
            BonusChoiceStatus::Cooldown { .. }
        ));
        assert!(
            state
                .events
                .iter()
                .any(|event| event.starts_with("alex got "))
        );
    }

    #[test]
    fn losing_contested_network_bonus_forces_player_to_main_word() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.race.players[0].state.word_index = 1;
        state.race.players[1].state.word_index = 1;

        apply_network_key_input(&mut state, PlayerId(1), KeyAction::Char('d'), now);
        apply_network_key_input(&mut state, PlayerId(2), KeyAction::Char('d'), now);
        state.bonuses.points[0].choices[0].status = BonusChoiceStatus::Cooldown {
            until: now + Duration::from_secs(4),
        };
        for action in [
            KeyAction::Char('a'),
            KeyAction::Char('s'),
            KeyAction::Char('h'),
            KeyAction::Space,
        ] {
            apply_network_key_input(&mut state, PlayerId(2), action, now);
        }

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(alex.state.word_index, 1);
        assert_eq!(alex.state.input, "");
        assert_eq!(state.runtime.spent_bonus_gaps.get(&PlayerId(2)), Some(&0));

        apply_network_key_input(&mut state, PlayerId(2), KeyAction::Char('s'), now);

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(alex.state.input, "s");
        assert_eq!(alex.state.typo_index, Some(0));
        assert!(!state.runtime.bonus_attempts.contains_key(&PlayerId(2)));
    }

    #[test]
    fn network_mushroom_boost_advances_one_word_immediately() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);

        activate_network_pickup(
            &mut state,
            PlayerId(2),
            ItemPickup::Held(HeldItem::Mushroom),
            now,
        );

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(alex.state.word_index, 1);
        assert!(alex.state.active_effects.iter().any(|effect| {
            matches!(
                effect,
                ActiveEffect::Mushroom {
                    remaining_words: 2,
                    ..
                }
            )
        }));
    }

    #[test]
    fn network_banana_resets_human_target_to_word_start_without_stun() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.race.players[0].state.word_index = 1;
        state.race.players[1].state.word_index = 0;
        state.race.players[1].state.input = "tw".to_string();

        activate_network_pickup(
            &mut state,
            PlayerId(1),
            ItemPickup::Held(HeldItem::Banana),
            now,
        );

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(alex.state.input, "");
        assert_eq!(alex.state.typo_index, None);
        assert!(
            state
                .runtime
                .player_effects
                .get(&RacePlayerId(2))
                .and_then(|effects| effects.stunned_until)
                .is_none()
        );
        assert!(
            state
                .runtime
                .player_effects
                .get(&RacePlayerId(2))
                .and_then(|effects| effects.impact_cue)
                .is_some_and(|cue| cue.until > now && cue.kind == RaceImpactCueKind::Banana)
        );
        assert!(
            state
                .runtime
                .player_effects
                .get(&RacePlayerId(1))
                .and_then(|effects| effects.item_cue.clone())
                .is_some()
        );
    }

    #[test]
    fn network_shield_blocks_banana_and_is_consumed() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.race.players[0].state.word_index = 1;
        state.race.players[1].state.word_index = 0;

        activate_network_pickup(&mut state, PlayerId(2), ItemPickup::Shield, now);
        activate_network_pickup(
            &mut state,
            PlayerId(1),
            ItemPickup::Held(HeldItem::Banana),
            now,
        );

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert!(!alex.state.has_active_shield(now));
        assert_eq!(
            state
                .runtime
                .player_effects
                .get(&RacePlayerId(2))
                .and_then(|effects| effects.impact_cue)
                .map(|cue| cue.kind),
            Some(RaceImpactCueKind::ShieldBlock)
        );
    }

    #[test]
    fn network_focus_pickup_marks_snapshot_as_focused() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);

        activate_network_pickup(
            &mut state,
            PlayerId(1),
            ItemPickup::Held(HeldItem::Focus),
            now,
        );
        let snapshot = build_race_snapshot(&mut state);

        assert!(snapshot.players[0].focused);
    }

    #[test]
    fn network_focused_ai_racer_gets_small_wpm_boost() {
        assert_eq!(
            crate::game::ai_driver::ai_effective_wpm(60.0, true, false, 10, 70),
            70.0
        );
        assert_eq!(
            crate::game::ai_driver::ai_effective_wpm(60.0, false, false, 10, 70),
            60.0
        );
    }

    #[test]
    fn network_cyclone_reverses_first_place_target_word() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.race.players[1].state.word_index = 1;

        activate_network_pickup(
            &mut state,
            PlayerId(1),
            ItemPickup::Held(HeldItem::Cyclone),
            now,
        );

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(alex.state.word_override(1), Some("owt"));
        assert!(
            state
                .runtime
                .player_effects
                .get(&RacePlayerId(2))
                .and_then(|effects| effects.stunned_until)
                .is_some_and(|until| until > now)
        );
        assert!(
            state
                .events
                .iter()
                .any(|event| event == "host hit alex with Cyclone")
        );
    }

    #[test]
    fn network_cyclone_misses_when_attacker_is_first_place() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.race.players[0].state.word_index = 1;
        state.race.players[1].state.word_index = 0;

        activate_network_pickup(
            &mut state,
            PlayerId(1),
            ItemPickup::Held(HeldItem::Cyclone),
            now,
        );

        let host = state.race.player(RacePlayerId(1)).unwrap();
        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(host.state.word_override(1), None);
        assert_eq!(alex.state.word_override(0), None);
        assert!(
            state
                .events
                .iter()
                .any(|event| event == "host missed Cyclone")
        );
    }

    #[test]
    fn network_cyclone_targets_first_place_ai() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.players[1].kind = PlayerKind::Bot;
        state.race.players[0].state.word_index = 0;
        state.race.players[1].state.word_index = 1;
        state.ai_racers.insert(
            PlayerId(2),
            NetworkAiRacer {
                difficulty: AiDifficulty::Easy,
                words_per_minute: 60.0,
                char_budget: 4.0,
                last_update: now,
            },
        );

        activate_network_pickup(
            &mut state,
            PlayerId(1),
            ItemPickup::Held(HeldItem::Cyclone),
            now,
        );

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(alex.state.word_override(1), Some("owt"));
        assert_eq!(state.ai_racers.get(&PlayerId(2)).unwrap().char_budget, 0.0);
        assert!(
            state
                .events
                .iter()
                .any(|event| event == "host hit alex with Cyclone")
        );
    }

    #[test]
    fn network_cyclone_is_blocked_by_shield_and_consumes_shield() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.race.players[1].state.word_index = 1;
        state.race.players[1]
            .state
            .active_effects
            .push(ActiveEffect::Shield {
                until: now + Duration::from_secs(5),
            });

        activate_network_pickup(
            &mut state,
            PlayerId(1),
            ItemPickup::Held(HeldItem::Cyclone),
            now,
        );

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(alex.state.word_override(1), None);
        assert!(!alex.state.has_active_shield(now));
        assert!(
            state
                .events
                .iter()
                .any(|event| event == "alex blocked Cyclone")
        );
    }

    #[test]
    fn network_fog_marks_all_targets_in_range() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.race.players[0].state.word_index = 1;
        state.race.players[1].state.word_index = 3;

        activate_network_pickup(
            &mut state,
            PlayerId(1),
            ItemPickup::Held(HeldItem::Fog),
            now,
        );

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert!(alex.state.is_fogged_at(now));
        assert_eq!(
            state
                .runtime
                .player_effects
                .get(&RacePlayerId(2))
                .and_then(|effects| effects.impact_cue)
                .map(|cue| cue.kind),
            Some(RaceImpactCueKind::Fog)
        );

        let snapshot = build_race_snapshot(&mut state);
        assert!(snapshot.players[1].fogged);
    }

    #[test]
    fn network_fog_is_blocked_by_shield() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.race.players[1]
            .state
            .active_effects
            .push(ActiveEffect::Shield {
                until: now + Duration::from_secs(5),
            });

        activate_network_pickup(
            &mut state,
            PlayerId(1),
            ItemPickup::Held(HeldItem::Fog),
            now,
        );

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert!(!alex.state.is_fogged_at(now));
        assert!(!alex.state.has_active_shield(now));
        assert!(state.events.iter().any(|event| event == "alex blocked Fog"));
    }

    #[test]
    fn race_status_records_finish_order_and_finishes_when_all_connected_finish() {
        let now = std::time::Instant::now();
        let mut state = HostState {
            clients: Vec::new(),
            players: test_players(true),
            race: test_race_state(),
            ai_racers: HashMap::new(),
            word_list: test_word_list(),
            bonuses: test_bonus_state(),
            item_registry: ItemRegistry::builtin(),
            active_mod_config: test_active_mod_config(),
            max_players: 6,
            ai_difficulty: AiDifficulty::Easy,
            runtime: RaceRuntimeState::new(),
            phase: NetworkRacePhase::Racing,
            snapshot_sequence: 0,
            events: Vec::new(),
            debug_log: None,
            race_results_sent: false,
        };

        finish_player(&mut state, RacePlayerId(2), now);
        update_race_status(&mut state, now);

        assert_eq!(state.phase, NetworkRacePhase::Racing);
        assert_eq!(state.runtime.lifecycle.placements, vec![RacePlayerId(2)]);

        finish_player(&mut state, RacePlayerId(1), now);
        update_race_status(&mut state, now);

        assert_eq!(state.phase, NetworkRacePhase::Finished);
        assert_eq!(
            state.runtime.lifecycle.placements,
            vec![RacePlayerId(2), RacePlayerId(1)]
        );
        assert!(state.events.iter().any(|event| event == "Race finished"));
    }

    #[test]
    fn race_status_timeout_places_unfinished_connected_racers_by_progress() {
        let now = std::time::Instant::now();
        let mut state = HostState {
            clients: Vec::new(),
            players: test_players(true),
            race: test_race_state(),
            ai_racers: HashMap::new(),
            word_list: test_word_list(),
            bonuses: test_bonus_state(),
            item_registry: ItemRegistry::builtin(),
            active_mod_config: test_active_mod_config(),
            max_players: 6,
            ai_difficulty: AiDifficulty::Easy,
            runtime: RaceRuntimeState::new(),
            phase: NetworkRacePhase::Racing,
            snapshot_sequence: 0,
            events: Vec::new(),
            debug_log: None,
            race_results_sent: false,
        };
        finish_player(&mut state, RacePlayerId(2), now);
        state
            .race
            .apply_key_input(RacePlayerId(1), KeyAction::Char('o'), now)
            .unwrap();

        update_race_status(&mut state, now);
        update_race_status(&mut state, now + POST_FIRST_FINISH_TIMEOUT);

        assert_eq!(state.phase, NetworkRacePhase::Finished);
        assert_eq!(
            state.runtime.lifecycle.placements,
            vec![RacePlayerId(2), RacePlayerId(1)]
        );
    }

    #[test]
    fn race_status_finishes_when_all_racers_disconnect() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        for player in &mut state.players {
            player.connected = false;
        }
        for player in &mut state.race.players {
            player.connected = false;
        }

        update_race_status(&mut state, now);

        assert_eq!(state.phase, NetworkRacePhase::Finished);
        assert!(state.runtime.lifecycle.placements.is_empty());
        assert!(state.events.iter().any(|event| event == "Race finished"));
    }

    #[test]
    fn countdown_continues_when_one_racer_remains_connected() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Countdown {
            remaining_seconds: 2,
        });
        state.players[1].connected = false;
        state.race.players[1].connected = false;

        reconcile_phase_after_disconnect(&mut state, now);

        assert_eq!(
            state.phase,
            NetworkRacePhase::Countdown {
                remaining_seconds: 2
            }
        );
    }

    #[test]
    fn countdown_cancels_when_no_racers_remain_connected() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Countdown {
            remaining_seconds: 2,
        });
        for player in &mut state.players {
            player.connected = false;
        }
        for player in &mut state.race.players {
            player.connected = false;
        }

        reconcile_phase_after_disconnect(&mut state, now);

        assert_eq!(state.phase, NetworkRacePhase::WaitingForHost);
        assert!(
            state
                .events
                .iter()
                .any(|event| event == "Countdown cancelled")
        );
    }

    #[test]
    fn countdown_continues_when_multiple_racers_remain_connected() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Countdown {
            remaining_seconds: 2,
        });

        reconcile_phase_after_disconnect(&mut state, now);

        assert_eq!(
            state.phase,
            NetworkRacePhase::Countdown {
                remaining_seconds: 2
            }
        );
    }

    #[test]
    fn race_results_are_broadcast_only_once() {
        let mut state = HostState {
            clients: Vec::new(),
            players: test_players(true),
            race: test_race_state(),
            ai_racers: HashMap::new(),
            word_list: test_word_list(),
            bonuses: test_bonus_state(),
            item_registry: ItemRegistry::builtin(),
            active_mod_config: test_active_mod_config(),
            max_players: 6,
            ai_difficulty: AiDifficulty::Easy,
            runtime: RaceRuntimeState::new(),
            phase: NetworkRacePhase::Finished,
            snapshot_sequence: 0,
            events: Vec::new(),
            debug_log: None,
            race_results_sent: false,
        };

        broadcast_race_results_once(&mut state).unwrap();
        assert!(state.race_results_sent);

        broadcast_race_results_once(&mut state).unwrap();
        assert!(state.race_results_sent);
    }

    #[test]
    fn race_result_rows_include_stats_and_status_for_every_racer() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Finished);
        finish_player(&mut state, RacePlayerId(2), now);
        state.runtime.lifecycle.placements = vec![RacePlayerId(2)];

        let host = state
            .race
            .players
            .iter_mut()
            .find(|player| player.id == RacePlayerId(1))
            .unwrap();
        host.connected = false;
        host.state.word_index = 1;
        host.state.stats = TypingStats {
            typed_chars: 10,
            correct_chars: 8,
            typo_chars: 2,
            backspaces: 3,
            completed_words: 1,
        };

        let rows = build_race_result_rows(&state, now + Duration::from_secs(30));

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].player_id, PlayerId(2));
        assert_eq!(rows[0].status, RaceResultStatus::Finished);
        assert_eq!(rows[0].progress_words, 2);
        assert_eq!(rows[1].player_id, PlayerId(1));
        assert_eq!(rows[1].status, RaceResultStatus::Disconnected);
        assert_eq!(rows[1].progress_words, 1);
        assert_eq!(rows[1].accuracy_percent, 80);
        assert_eq!(rows[1].typo_chars, 2);
        assert_eq!(rows[1].backspaces, 3);
    }

    fn send_hello(client: &mut std::net::TcpStream) {
        let hello = encode_client_message(&ClientMessage::Hello {
            name: "alex".to_string(),
            client_version: env!("CARGO_PKG_VERSION").to_string(),
        })
        .unwrap();
        writeln!(client, "{hello}").unwrap();
    }

    fn lobby_player(id: PlayerId, name: &str, kind: PlayerKind, connected: bool) -> LobbyPlayer {
        LobbyPlayer {
            id,
            name: name.to_string(),
            kind,
            color: AssignedColor::Cyan,
            ready: false,
            connected,
            ai_difficulty: None,
            ai_wpm: None,
        }
    }

    fn test_players(joiner_ready: bool) -> Vec<LobbyPlayer> {
        vec![
            LobbyPlayer {
                id: PlayerId(1),
                name: "host".to_string(),
                kind: PlayerKind::Human,
                color: AssignedColor::Cyan,
                ready: true,
                connected: true,
                ai_difficulty: None,
                ai_wpm: None,
            },
            LobbyPlayer {
                id: PlayerId(2),
                name: "alex".to_string(),
                kind: PlayerKind::Human,
                color: AssignedColor::Red,
                ready: joiner_ready,
                connected: true,
                ai_difficulty: None,
                ai_wpm: None,
            },
        ]
    }

    fn test_race_state() -> RaceState {
        let now = std::time::Instant::now();
        let mut race = RaceState::new(Track::new(vec!["one".to_string(), "two".to_string()]));
        race.add_player(RacePlayerId(1), "host", PlayerColorId::Cyan, now);
        race.add_player(RacePlayerId(2), "alex", PlayerColorId::Red, now);
        race
    }

    fn test_bonus_state() -> BonusState {
        BonusState::with_points(
            vec![BonusPoint::new(
                0,
                [
                    BonusChoice::available("dash"),
                    BonusChoice::available("drift"),
                    BonusChoice::available("spark"),
                ],
            )],
            vec!["dash".to_string(), "drift".to_string(), "spark".to_string()],
        )
    }

    fn test_word_list() -> WordList {
        WordList {
            words: vec![
                "one".to_string(),
                "two".to_string(),
                "three".to_string(),
                "four".to_string(),
            ],
        }
    }

    fn test_active_mod_config() -> ActiveModConfig {
        let item_registry = ItemRegistry::builtin();
        ActiveModConfig::new(
            &WordSetDefinition {
                metadata: ContentMetadata::built_in("classic", "Classic"),
                words: test_word_list(),
            },
            &item_registry,
            None,
        )
    }

    fn test_single_item_registry(pickup: ItemPickup) -> ItemRegistry {
        let (id, name, activation) = match pickup {
            ItemPickup::Held(HeldItem::Mushroom) => ("mushroom", "Mushroom", ItemActivation::Held),
            ItemPickup::Held(HeldItem::Banana) => ("banana", "Banana", ItemActivation::Held),
            ItemPickup::Held(HeldItem::Focus) => ("focus", "Focus", ItemActivation::Held),
            ItemPickup::Held(HeldItem::Cyclone) => ("cyclone", "Cyclone", ItemActivation::Held),
            ItemPickup::Held(HeldItem::Fog) => ("fog", "Fog", ItemActivation::Held),
            ItemPickup::Shield => ("shield", "Shield", ItemActivation::Immediate),
        };
        ItemRegistry::new(vec![ItemDefinition::built_in(
            id, name, pickup, activation, 1, 1,
        )])
        .unwrap()
    }

    fn test_host_state(phase: NetworkRacePhase) -> HostState {
        HostState {
            clients: Vec::new(),
            players: test_players(true),
            race: test_race_state(),
            ai_racers: HashMap::new(),
            word_list: test_word_list(),
            bonuses: test_bonus_state(),
            item_registry: ItemRegistry::builtin(),
            active_mod_config: test_active_mod_config(),
            max_players: 6,
            ai_difficulty: AiDifficulty::Easy,
            runtime: RaceRuntimeState::new(),
            phase,
            snapshot_sequence: 0,
            events: Vec::new(),
            debug_log: None,
            race_results_sent: false,
        }
    }

    fn finish_player(state: &mut HostState, id: RacePlayerId, now: std::time::Instant) {
        let player = state
            .race
            .players
            .iter_mut()
            .find(|player| player.id == id)
            .unwrap();
        player.state.word_index = state.race.track.len();
        player.state.finished_at = Some(now);
    }
}
