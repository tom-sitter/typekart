use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

use futures_channel::mpsc::UnboundedReceiver;
use futures_util::{SinkExt, StreamExt, select};
use gloo_net::websocket::{Message, futures::WebSocket};
use gloo_timers::future::{IntervalStream, TimeoutFuture};
use leptos::prelude::*;
use rand::thread_rng;
use typekart::game::{
    ai_driver::{AiDriverConfig, AiDriverState, advance_ai_driver},
    bonus::BonusState,
    bonus_flow::{BonusAttempt, BonusClaimRoll, BonusFlowEvent, BonusFlowState, apply_bonus_key},
    host_session::{
        CountdownAdvanceRejection, CountdownRacePreparation, CountdownStartRejection,
        PreparedHostRace, advance_countdown_to_racing, begin_countdown_phase,
        countdown_start_plan, countdown_tick_phase, prepare_race_from_selected_lobby_players,
        return_to_lobby_outcome,
    },
    item_effects::{RaceItemEffectState, activate_item_pickup, advance_mushrooms, player_is_stunned},
    input_rules::player_input_is_paused,
    items::{ItemPickup, ItemRegistry, ItemRollContext, RacePositionBand},
    lobby::{
        add_ai_lobby_player as shared_add_ai_lobby_player, color_for_lobby_slot,
        first_available_player_id, lobby_name_or_default, new_human_lobby_player,
        remove_lobby_player as shared_remove_lobby_player,
        rename_lobby_player as shared_rename_lobby_player,
        set_lobby_ai_difficulty as shared_set_lobby_ai_difficulty,
        set_lobby_ready as shared_set_lobby_ready, unique_lobby_name,
    },
    player::PlayerState,
    race::{PlayerColorId, RacePlayer, RacePlayerId, RaceState, RaceRuntimeState},
    race_flow::{advance_race_flow, update_race_flow},
    snapshot::{
        RaceSnapshotInput, build_bonus_snapshots,
        build_placement_snapshots as build_shared_placement_snapshots, build_player_snapshots,
        build_race_result_snapshots as build_shared_race_result_snapshots, build_race_snapshot,
        player_color_id,
    },
    track::{Track, WordList},
    typing::KeyAction,
};
use typekart_protocol::{
    AiDifficultySnapshot, AssignedColor, ClientMessage, LobbyPlayer, ModConfigSnapshot,
    NetworkRacePhase, PlayerId, PlayerKind, ProtocolKey, RaceSnapshot, RelayClientMessage,
    RelayServerMessage, RoomCode, ServerMessage,
};

use crate::fixtures::{GalleryFrame, LobbyFrame, ResultsFrame};
use crate::session::{BrowserHostSignals, BrowserOutboundMessage, ConnectionState};

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const BROWSER_HOST_TRACK_WORD_COUNT: usize = 16;
const BROWSER_HOST_AI_TICK_MS: u32 = 250;
const BROWSER_HOST_POST_FIRST_FINISH_TIMEOUT: Duration = Duration::from_secs(30);
const BROWSER_HOST_MAX_PLAYERS: usize = 6;

pub(crate) async fn host_browser_lobby(
    relay_url: String,
    host_name: String,
    outbound: UnboundedReceiver<BrowserOutboundMessage>,
    signals: BrowserHostSignals,
) -> Result<(), String> {
    let websocket = WebSocket::open(&relay_url)
        .map_err(|error| format!("failed to open relay websocket: {error:?}"))?;
    let (mut writer, reader) = websocket.split();
    let create = RelayClientMessage::CreateRoom {
        host_version: APP_VERSION.to_string(),
    };
    writer
        .send(Message::Text(serde_json::to_string(&create).map_err(
            |error| format!("failed to encode room create request: {error}"),
        )?))
        .await
        .map_err(|error| format!("failed to send room create request: {error:?}"))?;

    signals.session.set_connection.set(ConnectionState::Connected {
        message: "Creating room...".to_string(),
    });

    let mut state: Option<BrowserHostLobby> = None;
    let mut reader = reader.fuse();
    let mut outbound = outbound.fuse();
    let mut ai_ticks = IntervalStream::new(BROWSER_HOST_AI_TICK_MS).fuse();
    loop {
        select! {
            message = reader.next() => {
                let Some(message) = message else {
                    break;
                };
                let Message::Text(text) =
                    message.map_err(|error| format!("failed to read relay message: {error:?}"))?
                else {
                    continue;
                };
                let relay_message = serde_json::from_str::<RelayServerMessage>(&text)
                    .map_err(|error| format!("failed to decode relay message: {error}"))?;
                let keep_running = handle_browser_host_relay_message(
                    relay_message,
                    &mut state,
                    &host_name,
                    &mut writer,
                    signals,
                )
                .await?;
                if !keep_running {
                    break;
                }
            },
            outbound_message = outbound.next() => {
                let Some(outbound_message) = outbound_message else {
                    continue;
                };
                if matches!(outbound_message, BrowserOutboundMessage::Disconnect) {
                    if let Some(state) = &state {
                        let leave = RelayClientMessage::LeaveRoom {
                            room: state.room.clone(),
                        };
                        writer
                            .send(Message::Text(
                                serde_json::to_string(&leave)
                                    .map_err(|error| format!("failed to encode leave message: {error}"))?,
                            ))
                            .await
                            .map_err(|error| format!("failed to send leave message: {error:?}"))?;
                    }
                    let _ = writer.close().await;
                    break;
                }

                let Some(state) = state.as_mut() else {
                    continue;
                };
                if let BrowserOutboundMessage::Client { player_id, message } = outbound_message {
                    handle_browser_host_client_message(
                        state,
                        player_id,
                        message,
                        &mut writer,
                        signals.session.set_connection,
                        signals.session.set_live_frame,
                    )
                    .await?;
                }
            },
            _ = ai_ticks.next() => {
                let Some(state) = state.as_mut() else {
                    continue;
                };
                if apply_browser_host_ai_tick(state, BROWSER_HOST_AI_TICK_MS) {
                    publish_browser_host_state(state, &mut writer, signals.session.set_live_frame).await?;
                }
            },
        }
    }

    Ok(())
}

struct BrowserHostLobby {
    room: RoomCode,
    players: Vec<LobbyPlayer>,
    relay_players: HashMap<PlayerId, PlayerId>,
    next_player_id: u64,
    race_sequence: u64,
    next_track_words: Vec<String>,
    bonuses: BonusState,
    item_registry: ItemRegistry,
    active_race: Option<RaceSnapshot>,
    active_results: Option<ResultsFrame>,
    core_race: Option<RaceState>,
    runtime: RaceRuntimeState<PlayerId, BonusAttempt>,
    ai_char_budget: HashMap<PlayerId, AiDriverState>,
    ai_last_tick_ms: Option<f64>,
    events: Vec<String>,
    mod_config: ModConfigSnapshot,
}

impl BrowserHostLobby {
    fn new(room: RoomCode, host_name: String) -> Self {
        let next_track_words = browser_generate_track_words();
        let bonuses = browser_generate_bonus_state(&next_track_words);
        Self {
            room,
            players: vec![LobbyPlayer {
                id: PlayerId(1),
                name: lobby_name_or_default(&host_name, "host"),
                kind: PlayerKind::Human,
                color: AssignedColor::Cyan,
                ready: true,
                connected: true,
                ai_difficulty: None,
                ai_wpm: None,
            }],
            relay_players: HashMap::new(),
            next_player_id: 2,
            race_sequence: 0,
            next_track_words,
            bonuses,
            item_registry: ItemRegistry::builtin(),
            active_race: None,
            active_results: None,
            core_race: None,
            runtime: RaceRuntimeState::new(),
            ai_char_budget: HashMap::new(),
            ai_last_tick_ms: None,
            events: vec!["host created room".to_string()],
            mod_config: browser_default_mod_config(),
        }
    }

    fn game_player_id_for_relay(&self, relay_player_id: PlayerId) -> Option<PlayerId> {
        if relay_player_id == PlayerId(1) {
            return Some(PlayerId(1));
        }
        self.relay_players.get(&relay_player_id).copied()
    }

    fn frame(&self) -> LobbyFrame {
        LobbyFrame {
            host_id: PlayerId(1),
            players: self.players.clone(),
            mod_config: self.mod_config.clone(),
            events: self.events.clone(),
        }
    }

    fn phase(&self) -> NetworkRacePhase {
        if let Some(snapshot) = &self.active_race {
            snapshot.phase
        } else if self.active_results.is_some() {
            NetworkRacePhase::Finished
        } else {
            NetworkRacePhase::Lobby
        }
    }

    fn push_event(&mut self, event: impl Into<String>) {
        self.events.push(event.into());
        const MAX_EVENTS: usize = 8;
        if self.events.len() > MAX_EVENTS {
            self.events.drain(0..self.events.len() - MAX_EVENTS);
        }
    }

    fn next_race_sequence(&mut self) -> u64 {
        self.race_sequence += 1;
        self.race_sequence
    }
}

async fn handle_browser_host_relay_message(
    relay_message: RelayServerMessage,
    state: &mut Option<BrowserHostLobby>,
    host_name: &str,
    writer: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    signals: BrowserHostSignals,
) -> Result<bool, String> {
    match relay_message {
        RelayServerMessage::RoomCreated { room } => {
            signals.set_room_code.set(room.display());
            signals.session.set_relay_player_id.set(Some(PlayerId(1)));
            signals.session.set_game_player_id.set(Some(PlayerId(1)));
            signals.session.set_connection.set(ConnectionState::Connected {
                message: format!("Hosting room {}", room.display()),
            });
            let lobby = BrowserHostLobby::new(room, host_name.to_string());
            *state = Some(lobby);
            if let Some(state) = state {
                publish_browser_host_state(state, writer, signals.session.set_live_frame).await?;
            }
        }
        RelayServerMessage::JoinForwarded {
            pending_player_id,
            name,
            ..
        } => {
            let Some(state) = state else {
                return Ok(true);
            };
            let assigned = add_browser_lobby_human(state, pending_player_id, &name);
            let welcome = ServerMessage::Welcome {
                player_id: assigned.id,
                assigned_color: assigned.color,
            };
            send_browser_host_direct(state, writer, pending_player_id, welcome).await?;
            state.push_event(format!("{} joined", assigned.name));
            publish_browser_host_state(state, writer, signals.session.set_live_frame).await?;
        }
        RelayServerMessage::ClientToHost {
            player_id, message, ..
        } => {
            let Some(state) = state else {
                return Ok(true);
            };
            let Some(game_player_id) = state.game_player_id_for_relay(player_id) else {
                state.push_event(format!("unknown relay player {} ignored", player_id.0));
                publish_browser_host_state(state, writer, signals.session.set_live_frame).await?;
                return Ok(true);
            };
            let message = serde_json::from_value::<ClientMessage>(message)
                .map_err(|error| format!("failed to decode client message: {error}"))?;
            handle_browser_host_client_message(
                state,
                game_player_id,
                message,
                writer,
                signals.session.set_connection,
                signals.session.set_live_frame,
            )
            .await?;
        }
        RelayServerMessage::ParticipantDisconnected { player_id, .. } => {
            if let Some(state) = state {
                let Some(game_player_id) = state.relay_players.remove(&player_id) else {
                    return Ok(true);
                };
                if let Some(player) = state
                    .players
                    .iter_mut()
                    .find(|player| player.id == game_player_id)
                {
                    player.connected = false;
                    let name = player.name.clone();
                    state.push_event(format!("{name} disconnected"));
                    publish_browser_host_state(state, writer, signals.session.set_live_frame).await?;
                }
            }
        }
        RelayServerMessage::Error { message } => {
            signals.session.set_connection.set(ConnectionState::Failed { message });
            signals.session.set_live_frame.set(None);
            return Ok(false);
        }
        RelayServerMessage::RoomClosed { reason } => {
            signals.session.set_connection.set(ConnectionState::Closed { reason });
            signals.session.set_live_frame.set(None);
            return Ok(false);
        }
        RelayServerMessage::HostToClient { .. } | RelayServerMessage::HostBroadcast { .. } => {}
    }
    Ok(true)
}

async fn handle_browser_host_client_message(
    state: &mut BrowserHostLobby,
    player_id: PlayerId,
    message: ClientMessage,
    writer: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    set_connection: WriteSignal<ConnectionState>,
    set_live_frame: WriteSignal<Option<GalleryFrame>>,
) -> Result<(), String> {
    if matches!(message, ClientMessage::StartCountdown) && player_id == PlayerId(1) {
        run_browser_host_countdown(state, writer, set_connection, set_live_frame).await?;
        return Ok(());
    }

    process_browser_host_client_message(state, player_id, message, set_connection);
    publish_browser_host_state(state, writer, set_live_frame).await
}

fn process_browser_host_client_message(
    state: &mut BrowserHostLobby,
    player_id: PlayerId,
    message: ClientMessage,
    set_connection: WriteSignal<ConnectionState>,
) {
    match message {
        ClientMessage::Rename { name } => {
            let phase = state.phase();
            match shared_rename_lobby_player(
                &mut state.players,
                phase,
                player_id,
                &name,
            ) {
                Ok(outcome) => {
                    state.push_event(format!(
                        "{} renamed to {}",
                        outcome.previous_name, outcome.new_name
                    ));
                }
                Err(error) => state.push_event(error.to_string()),
            }
        }
        ClientMessage::SetReady { ready } => {
            match shared_set_lobby_ready(&mut state.players, player_id, ready) {
                Ok(outcome) => {
                    state.push_event(format!(
                        "{} {}",
                        outcome.name,
                        if outcome.ready { "ready" } else { "not ready" }
                    ));
                }
                Err(error) => state.push_event(error.to_string()),
            }
        }
        ClientMessage::AddAi if player_id == PlayerId(1) => {
            let phase = state.phase();
            match shared_add_ai_lobby_player(
                &mut state.players,
                phase,
                BROWSER_HOST_MAX_PLAYERS,
                AiDifficultySnapshot::Easy,
                browser_ai_wpm(AiDifficultySnapshot::Easy),
            ) {
                Ok(outcome) => state.push_event(format!("{} added", outcome.player.name)),
                Err(error) => state.push_event(error.to_string()),
            }
        }
        ClientMessage::RemoveLobbyPlayer { player_id: target } if player_id == PlayerId(1) => {
            let phase = state.phase();
            match shared_remove_lobby_player(&mut state.players, phase, target) {
                Ok(outcome) => state.push_event(match outcome.player.kind {
                    PlayerKind::Human => format!("{} kicked", outcome.player.name),
                    PlayerKind::Bot => format!("{} removed", outcome.player.name),
                }),
                Err(error) => state.push_event(error.to_string()),
            }
        }
        ClientMessage::SetAiDifficulty {
            player_id: target,
            difficulty,
        } if player_id == PlayerId(1) => {
            let phase = state.phase();
            match shared_set_lobby_ai_difficulty(
                &mut state.players,
                phase,
                target,
                difficulty,
                browser_ai_wpm(difficulty),
            ) {
                Ok(typekart::game::lobby::LobbyAiDifficultyOutcome::DefaultChanged { .. }) => {
                    state.push_event("updated default AI racer difficulty");
                }
                Ok(typekart::game::lobby::LobbyAiDifficultyOutcome::PlayerChanged {
                    name,
                    ..
                }) => {
                    state.push_event(format!("{} set to {}", name, browser_ai_label(difficulty)));
                }
                Err(error) => state.push_event(error.to_string()),
            }
        }
        ClientMessage::StartCountdown if player_id == PlayerId(1) => {}
        ClientMessage::Leave => {
            if let Some(player) = state
                .players
                .iter_mut()
                .find(|player| player.id == player_id)
            {
                player.connected = false;
                let name = player.name.clone();
                state.push_event(format!("{name} left"));
            }
        }
        ClientMessage::KeyInput { key, .. } => {
            apply_browser_host_race_key_input(state, player_id, key, set_connection);
        }
        ClientMessage::RestartRace if player_id == PlayerId(1) => {
            browser_return_host_to_lobby(state);
        }
        _ => {}
    }
}

fn browser_return_host_to_lobby(state: &mut BrowserHostLobby) {
    let Some(outcome) = return_to_lobby_outcome(state.phase()) else {
        return;
    };
    state.active_race = None;
    state.active_results = None;
    state.core_race = None;
    state.runtime.reset();
    state.ai_char_budget.clear();
    state.ai_last_tick_ms = None;
    state.push_event(outcome.event);
}

async fn publish_browser_host_state(
    state: &BrowserHostLobby,
    writer: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    set_live_frame: WriteSignal<Option<GalleryFrame>>,
) -> Result<(), String> {
    if let Some(results) = &state.active_results {
        set_live_frame.set(Some(GalleryFrame::Results(results.clone())));
        return send_browser_host_broadcast(
            state,
            writer,
            ServerMessage::RaceResults {
                placements: results.placements.clone(),
                rows: results.rows.clone(),
            },
        )
        .await;
    }

    if let Some(snapshot) = &state.active_race {
        set_live_frame.set(Some(GalleryFrame::Race(snapshot.clone())));
        return send_browser_host_broadcast(
            state,
            writer,
            ServerMessage::RaceSnapshot(snapshot.clone()),
        )
        .await;
    }

    publish_browser_host_lobby(state, writer, set_live_frame).await
}

async fn publish_browser_host_lobby(
    state: &BrowserHostLobby,
    writer: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    set_live_frame: WriteSignal<Option<GalleryFrame>>,
) -> Result<(), String> {
    let frame = state.frame();
    set_live_frame.set(Some(GalleryFrame::Lobby(frame.clone())));
    let message = ServerMessage::LobbySnapshot {
        players: frame.players,
        host_id: frame.host_id,
        mod_config: frame.mod_config,
        events: frame.events,
    };
    send_browser_host_broadcast(state, writer, message).await
}

async fn run_browser_host_countdown(
    state: &mut BrowserHostLobby,
    writer: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    set_connection: WriteSignal<ConnectionState>,
    set_live_frame: WriteSignal<Option<GalleryFrame>>,
) -> Result<(), String> {
    match countdown_start_plan(state.phase()) {
        Ok(CountdownRacePreparation::PrepareRace | CountdownRacePreparation::UseCurrentRace) => {}
        Err(CountdownStartRejection::RaceAlreadyActive) => {
            state.push_event("race already started");
            publish_browser_host_state(state, writer, set_live_frame).await?;
            return Ok(());
        }
        Err(CountdownStartRejection::NoConnectedRacers) => unreachable!(
            "countdown phase planning does not inspect racer count"
        ),
    }

    let racers = state
        .players
        .iter()
        .filter(|player| player.connected && player.ready)
        .cloned()
        .collect::<Vec<_>>();
    if racers.is_empty() {
        state.push_event("cannot start without ready racers");
        publish_browser_host_lobby(state, writer, set_live_frame).await?;
        return Ok(());
    }
    let countdown_phase = match begin_countdown_phase(racers.len()) {
        Ok(phase) => phase,
        Err(CountdownStartRejection::NoConnectedRacers) => {
            state.push_event("cannot start without ready racers");
            publish_browser_host_lobby(state, writer, set_live_frame).await?;
            return Ok(());
        }
        Err(CountdownStartRejection::RaceAlreadyActive) => unreachable!(
            "countdown begin only validates connected racer availability"
        ),
    };

    set_connection.set(ConnectionState::Connected {
        message: "Starting browser-hosted race shell".to_string(),
    });
    let race_track_words = state.next_track_words.clone();
    let prepared_race = browser_prepare_race_from_lobby(&racers, race_track_words.clone());
    state.bonuses = prepared_race.bonuses.clone();

    state.active_race = Some(browser_host_race_snapshot_with_track(
        state.next_race_sequence(),
        NetworkRacePhase::WaitingForHost,
        &state.mod_config,
        &racers,
        &race_track_words,
        &state.bonuses,
        vec!["browser host preparing race".to_string()],
    ));
    state.active_results = None;
    state.core_race = None;
    state.runtime.reset();
    state.ai_char_budget.clear();
    state.ai_last_tick_ms = None;
    publish_browser_host_state(state, writer, set_live_frame).await?;

    for remaining_seconds in [3, 2, 1] {
        let phase = if remaining_seconds == 3 {
            countdown_phase
        } else {
            countdown_tick_phase(remaining_seconds)
        };
        state.active_race = Some(browser_host_race_snapshot_with_track(
            state.next_race_sequence(),
            phase,
            &state.mod_config,
            &racers,
            &race_track_words,
            &state.bonuses,
            vec![format!("countdown {remaining_seconds}")],
        ));
        publish_browser_host_state(state, writer, set_live_frame).await?;
        TimeoutFuture::new(1000).await;
    }
    let racing_phase = match advance_countdown_to_racing(countdown_tick_phase(1), !racers.is_empty())
    {
        Ok(phase) => phase,
        Err(CountdownAdvanceRejection::NoConnectedRacers) => {
            state.push_event("countdown cancelled");
            publish_browser_host_lobby(state, writer, set_live_frame).await?;
            return Ok(());
        }
        Err(CountdownAdvanceRejection::NotCountingDown) => unreachable!(
            "browser countdown loop advances from countdown phase"
        ),
    };

    state.active_race = Some(browser_host_race_snapshot_with_track(
        state.next_race_sequence(),
        racing_phase,
        &state.mod_config,
        &racers,
        &race_track_words,
        &state.bonuses,
        vec!["browser-hosted race started".to_string()],
    ));
    state.active_results = None;
    state.core_race = Some(prepared_race.race);
    state.next_track_words = browser_generate_track_words();
    state.runtime.reset();
    if let (Some(snapshot), Some(core_race)) = (&mut state.active_race, &state.core_race) {
        browser_sync_snapshot_from_core(
            snapshot,
            core_race,
            &state.players,
            &state.runtime.player_effects,
        );
    }
    state.ai_char_budget.clear();
    state.ai_last_tick_ms = Some(browser_now_ms());
    publish_browser_host_state(state, writer, set_live_frame).await
}

#[cfg(test)]
fn browser_host_race_snapshot(
    sequence: u64,
    phase: NetworkRacePhase,
    mod_config: &ModConfigSnapshot,
    racers: &[LobbyPlayer],
    events: Vec<String>,
) -> RaceSnapshot {
    browser_host_race_snapshot_with_track(
        sequence,
        phase,
        mod_config,
        racers,
        &browser_demo_track_words(),
        &browser_generate_bonus_state(&browser_demo_track_words()),
        events,
    )
}

fn browser_host_race_snapshot_with_track(
    sequence: u64,
    phase: NetworkRacePhase,
    mod_config: &ModConfigSnapshot,
    racers: &[LobbyPlayer],
    track_words: &[String],
    bonuses: &BonusState,
    events: Vec<String>,
) -> RaceSnapshot {
    let race = browser_prepare_race_from_lobby(racers, track_words.to_vec()).race;
    let player_effects = HashMap::new();
    build_race_snapshot(
        RaceSnapshotInput {
            sequence,
            phase,
            mod_config: mod_config.clone(),
            race: &race,
            bonuses,
            player_effects: &player_effects,
            events,
            now: Instant::now(),
        },
        |player_id| {
            racers
                .iter()
                .find(|racer| racer.id == player_id)
                .map(|racer| racer.kind)
                .unwrap_or(PlayerKind::Human)
        },
    )
}

#[cfg(test)]
fn browser_host_core_race(racers: &[LobbyPlayer], track_words: Vec<String>) -> RaceState {
    browser_prepare_race_from_lobby(racers, track_words).race
}

fn browser_prepare_race_from_lobby(
    racers: &[LobbyPlayer],
    track_words: Vec<String>,
) -> PreparedHostRace {
    prepare_race_from_selected_lobby_players(
        racers,
        Track::new(track_words),
        &WordList::from_static(BROWSER_HOST_WORDS),
        Instant::now(),
    )
}

fn browser_ensure_core_race(state: &mut BrowserHostLobby) {
    if state.core_race.is_some() {
        return;
    }
    let Some(snapshot) = &state.active_race else {
        return;
    };
    state.core_race = Some(browser_core_race_from_snapshot(snapshot));
}

fn browser_core_race_from_snapshot(snapshot: &RaceSnapshot) -> RaceState {
    let now = Instant::now();
    RaceState {
        track: Track::new(snapshot.track_words.clone()),
        players: snapshot
            .players
            .iter()
            .map(|player| {
                let mut state = PlayerState::new(now);
                state.word_index = player.word_index;
                state.input = player.input.clone();
                state.typo_index = player.typo_index;
                state.word_overrides = player
                    .word_overrides
                    .iter()
                    .map(|override_word| (override_word.word_index, override_word.word.clone()))
                    .collect();
                if player.finished {
                    state.finished_at = Some(now);
                }

                RacePlayer {
                    id: RacePlayerId(player.id.0),
                    name: player.name.clone(),
                    color: browser_player_color_id(player.color),
                    state,
                    connected: player.connected,
                }
            })
            .collect(),
    }
}

fn apply_browser_host_race_key_input(
    state: &mut BrowserHostLobby,
    player_id: PlayerId,
    key: ProtocolKey,
    set_connection: WriteSignal<ConnectionState>,
) {
    browser_ensure_core_race(state);
    let Some(race) = &state.active_race else {
        return;
    };
    if race.phase != NetworkRacePhase::Racing {
        return;
    }
    let Some(player_name) = race
        .players
        .iter()
        .find(|player| player.id == player_id)
        .map(|player| player.name.clone())
    else {
        return;
    };
    if race
        .players
        .iter()
        .find(|player| player.id == player_id)
        .is_some_and(|player| player.finished)
    {
        return;
    }
    if browser_player_input_is_paused(state, player_id, Instant::now()) {
        return;
    }

    let action = browser_protocol_key_to_action(key);
    let now = Instant::now();
    let item_context = browser_item_roll_context(state, player_id, 5);
    let item_registry = state.item_registry.clone();
    let mut rng = thread_rng();
    if let Some(core_race) = &mut state.core_race {
        let previous_event_count = state.events.len();
        let bonus_outcome = apply_bonus_key(
            &mut BonusFlowState {
                race: core_race,
                bonuses: &mut state.bonuses,
                bonus_attempts: &mut state.runtime.bonus_attempts,
                spent_bonus_gaps: &mut state.runtime.spent_bonus_gaps,
            },
            player_id,
            RacePlayerId(player_id.0),
            action,
            now,
            BonusClaimRoll {
                item_context,
                item_registry: &item_registry,
                rng: &mut rng,
            },
        );
        if bonus_outcome.handled {
            handle_browser_bonus_events(state, player_id, bonus_outcome.events, now);
            browser_sync_active_race_from_core(state);
            set_browser_race_input_event(state, &player_name, previous_event_count);
            set_connection.set(ConnectionState::Connected {
                message: format!("{player_name} typed"),
            });
            return;
        }
    }

    {
        let Some(core_race) = &mut state.core_race else {
            return;
        };
        core_race.apply_key_input(RacePlayerId(player_id.0), action, Instant::now());
        state.runtime
        .lifecycle
            .update(core_race, Instant::now(), BROWSER_HOST_POST_FIRST_FINISH_TIMEOUT);
    }
    browser_sync_active_race_from_core(state);
    if let Some(race) = &mut state.active_race {
        race.events = vec![format!("{player_name} typed")];
    }
    browser_update_race_status(state, Instant::now());
    set_connection.set(ConnectionState::Connected {
        message: format!("{player_name} typed"),
    });
}

fn browser_player_input_is_paused(
    state: &BrowserHostLobby,
    player_id: PlayerId,
    now: Instant,
) -> bool {
    state.core_race.as_ref().is_some_and(|race| {
        player_input_is_paused(
            race,
            &state.runtime.player_effects,
            RacePlayerId(player_id.0),
            now,
        )
    })
}

fn set_browser_race_input_event(
    state: &mut BrowserHostLobby,
    player_name: &str,
    previous_event_count: usize,
) {
    let event = state
        .events
        .get(previous_event_count..)
        .and_then(|events| events.last())
        .cloned()
        .unwrap_or_else(|| format!("{player_name} typed"));
    if let Some(race) = &mut state.active_race {
        race.events = vec![event];
    }
}

fn browser_sync_active_race_from_core(state: &mut BrowserHostLobby) {
    let (Some(snapshot), Some(core_race)) = (&mut state.active_race, &state.core_race) else {
        return;
    };
    browser_sync_snapshot_from_core(
        snapshot,
        core_race,
        &state.players,
        &state.runtime.player_effects,
    );
    snapshot.bonuses = build_bonus_snapshots(&state.bonuses, Instant::now());
    state.race_sequence += 1;
    snapshot.sequence = state.race_sequence;
}

fn handle_browser_bonus_events(
    state: &mut BrowserHostLobby,
    player_id: PlayerId,
    events: Vec<BonusFlowEvent>,
    now: Instant,
) {
    for event in events {
        if let BonusFlowEvent::ClaimResolved(outcome) = event {
            handle_browser_bonus_claim_outcome(state, player_id, outcome.pickup, now);
        }
    }
}

fn handle_browser_bonus_claim_outcome(
    state: &mut BrowserHostLobby,
    player_id: PlayerId,
    pickup: Option<ItemPickup>,
    now: Instant,
) {
    let name = state
        .players
        .iter()
        .find(|player| player.id == player_id)
        .map(|player| player.name.clone())
        .unwrap_or_else(|| format!("player {}", player_id.0));
    match pickup {
        Some(item) => {
            let item_name = browser_item_pickup_name(item);
            state.push_event(format!("{name} got {item_name}"));
            activate_browser_item_pickup(state, player_id, item, now);
        }
        None => state.push_event(format!("{name} missed the bonus")),
    }
}

fn activate_browser_item_pickup(
    state: &mut BrowserHostLobby,
    player_id: PlayerId,
    item: ItemPickup,
    now: Instant,
) {
    let Some(core_race) = &mut state.core_race else {
        return;
    };
    let ai_players = state
        .players
        .iter()
        .filter(|player| player.kind == PlayerKind::Bot)
        .map(|player| RacePlayerId(player.id.0))
        .collect::<HashSet<_>>();
    let report = activate_item_pickup(
        core_race,
        &mut state.runtime.player_effects,
        &ai_players,
        &state.item_registry,
        RacePlayerId(player_id.0),
        item,
        now,
    );

    for interrupted in report.interrupted_players {
        state.runtime.bonus_attempts.remove(&PlayerId(interrupted.0));
    }
    for ai_id in report.reset_ai_players {
        state
            .ai_char_budget
            .insert(PlayerId(ai_id.0), AiDriverState::default());
    }
    for event in report.events {
        state.push_event(event);
    }
}

fn browser_item_roll_context(
    state: &BrowserHostLobby,
    player_id: PlayerId,
    max_distance_words: usize,
) -> ItemRollContext {
    ItemRollContext {
        has_nearby_racer: browser_player_has_nearby_racer(state, player_id, max_distance_words),
        position: browser_position_band(state, player_id),
    }
}

fn browser_player_has_nearby_racer(
    state: &BrowserHostLobby,
    player_id: PlayerId,
    max_distance_words: usize,
) -> bool {
    let Some(core_race) = &state.core_race else {
        return false;
    };
    let Some(player) = core_race.player(RacePlayerId(player_id.0)) else {
        return false;
    };

    core_race.players.iter().any(|other| {
        other.id != player.id
            && other.connected
            && !other.state.is_finished()
            && player.state.word_index.abs_diff(other.state.word_index) <= max_distance_words
    })
}

fn browser_position_band(state: &BrowserHostLobby, player_id: PlayerId) -> RacePositionBand {
    let Some(core_race) = &state.core_race else {
        return RacePositionBand::Middle;
    };
    let active_racers = core_race
        .players
        .iter()
        .filter(|player| player.connected && !player.state.is_finished())
        .collect::<Vec<_>>();
    if active_racers.len() <= 1 {
        return RacePositionBand::Middle;
    }

    let Some(player) = active_racers
        .iter()
        .find(|player| player.id == RacePlayerId(player_id.0))
    else {
        return RacePositionBand::Middle;
    };
    let ahead = active_racers
        .iter()
        .filter(|other| other.state.word_index > player.state.word_index)
        .count();
    let behind = active_racers
        .iter()
        .filter(|other| other.state.word_index < player.state.word_index)
        .count();

    if ahead == 0 && behind > 0 {
        RacePositionBand::First
    } else if behind == 0 && ahead > 0 {
        RacePositionBand::Trailing
    } else {
        RacePositionBand::Middle
    }
}

fn browser_item_pickup_name(item: ItemPickup) -> &'static str {
    match item {
        ItemPickup::Held(held_item) => held_item.name(),
        ItemPickup::Shield => "Shield",
    }
}

fn apply_browser_host_ai_tick(state: &mut BrowserHostLobby, tick_ms: u32) -> bool {
    let elapsed_ms = browser_host_ai_elapsed_ms(state, tick_ms);
    if elapsed_ms <= 0.0 {
        return false;
    }
    browser_ensure_core_race(state);
    let mut changed = false;
    if let Some(core_race) = &mut state.core_race {
        for interrupted in advance_mushrooms(core_race, Instant::now()) {
            state.runtime.bonus_attempts.remove(&PlayerId(interrupted.0));
            changed = true;
        }
    }

    {
        let Some(snapshot) = &mut state.active_race else {
            return false;
        };
        if snapshot.phase != NetworkRacePhase::Racing {
            return false;
        }
        let Some(core_race) = &mut state.core_race else {
            return false;
        };
        for player in &mut core_race.players {
            player.state.expire_effects(Instant::now());
        }

        let ai_configs = state
            .players
            .iter()
            .filter(|player| player.kind == PlayerKind::Bot)
            .map(|player| {
                let base_wpm = player
                    .ai_wpm
                    .unwrap_or_else(|| browser_ai_wpm(AiDifficultySnapshot::Easy));
                (
                    player.id,
                    AiDriverConfig {
                        base_wpm: f64::from(base_wpm),
                        focus_boost_wpm: state.item_registry.focus_effect().ai_wpm_boost,
                        ink_multiplier_percent: state
                            .item_registry
                            .squid_ink_effect()
                            .ai_wpm_multiplier_percent,
                    },
                )
            })
            .collect::<Vec<_>>();

        for (player_id, config) in ai_configs {
            let race_player_id = RacePlayerId(player_id.0);
            if player_is_stunned(&state.runtime.player_effects, race_player_id, Instant::now()) {
                continue;
            }
            let driver = state.ai_char_budget.entry(player_id).or_default();
            let advance = advance_ai_driver(
                core_race,
                race_player_id,
                driver,
                config,
                Instant::now(),
                Duration::from_secs_f64(elapsed_ms / 1000.0),
            );
            if advance.changed() {
                changed = true;
            }
        }

        if changed {
            update_race_flow(
                &mut state.runtime.lifecycle,
                core_race,
                Instant::now(),
                BROWSER_HOST_POST_FIRST_FINISH_TIMEOUT,
            );
            browser_sync_snapshot_from_core(
                snapshot,
                core_race,
                &state.players,
                &state.runtime.player_effects,
            );
            state.race_sequence += 1;
            snapshot.sequence = state.race_sequence;
            snapshot.events = vec!["AI racers advanced".to_string()];
        }
    }

    let race_status_changed = browser_update_race_status(state, Instant::now());
    changed || race_status_changed
}

fn browser_update_race_status(state: &mut BrowserHostLobby, now: Instant) -> bool {
    if state.active_results.is_some() {
        return false;
    }
    let Some(core_race) = &state.core_race else {
        return false;
    };

    let outcome = advance_race_flow(
        &mut state.runtime.lifecycle,
        core_race,
        now,
        BROWSER_HOST_POST_FIRST_FINISH_TIMEOUT,
    );

    if outcome.finished.is_none() {
        return false;
    }

    browser_finish_race(state);
    true
}

fn browser_finish_race(state: &mut BrowserHostLobby) {
    let Some(core_race) = &state.core_race else {
        return;
    };
    let rows = build_shared_race_result_snapshots(
        core_race,
        &state.runtime.lifecycle.placements,
        Instant::now(),
    );
    if let Some(snapshot) = &mut state.active_race {
        snapshot.phase = NetworkRacePhase::Finished;
        snapshot.events = vec!["Race finished".to_string()];
    }
    state.active_results = Some(ResultsFrame {
        placements: build_shared_placement_snapshots(&state.runtime.lifecycle.placements),
        rows,
        events: vec!["Race finished".to_string()],
    });
    state.active_race = None;
    state.ai_char_budget.clear();
    state.ai_last_tick_ms = None;
}

fn browser_host_ai_elapsed_ms(state: &mut BrowserHostLobby, tick_ms: u32) -> f64 {
    let Some(last_tick_ms) = state.ai_last_tick_ms else {
        return f64::from(tick_ms);
    };

    let now_ms = browser_now_ms();
    let elapsed_ms = now_ms - last_tick_ms;
    let minimum_real_tick_ms = f64::from(tick_ms) * 0.5;
    if elapsed_ms < minimum_real_tick_ms {
        return 0.0;
    }

    state.ai_last_tick_ms = Some(now_ms);
    elapsed_ms.min(1000.0)
}

fn browser_sync_snapshot_from_core(
    snapshot: &mut RaceSnapshot,
    core_race: &RaceState,
    lobby_players: &[LobbyPlayer],
    player_effects: &HashMap<RacePlayerId, RaceItemEffectState>,
) {
    let now = Instant::now();
    snapshot.track_words = core_race.track.words.clone();
    snapshot.players = build_player_snapshots(core_race, player_effects, now, |player_id| {
        lobby_players
            .iter()
            .find(|lobby_player| lobby_player.id == player_id)
            .map(|lobby_player| lobby_player.kind)
            .unwrap_or(PlayerKind::Human)
    });
}

fn browser_protocol_key_to_action(key: ProtocolKey) -> KeyAction {
    match key {
        ProtocolKey::Char(ch) => KeyAction::Char(ch),
        ProtocolKey::Space => KeyAction::Space,
        ProtocolKey::Backspace => KeyAction::Backspace,
    }
}

fn browser_player_color_id(color: AssignedColor) -> PlayerColorId {
    player_color_id(color)
}

#[cfg(target_arch = "wasm32")]
fn browser_now_ms() -> f64 {
    js_sys::Date::now()
}

#[cfg(not(target_arch = "wasm32"))]
fn browser_now_ms() -> f64 {
    use std::sync::OnceLock;
    use std::time::Instant;

    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as f64
}

fn browser_generate_track_words() -> Vec<String> {
    Track::generate(
        &WordList::from_static(BROWSER_HOST_WORDS),
        BROWSER_HOST_TRACK_WORD_COUNT,
    )
    .map(|track| track.words)
    .unwrap_or_else(|_| browser_demo_track_words())
}

fn browser_generate_bonus_state(track_words: &[String]) -> BonusState {
    BonusState::generate(
        &Track::new(track_words.to_vec()),
        &WordList::from_static(BROWSER_HOST_WORDS),
    )
}

fn browser_demo_track_words() -> Vec<String> {
    [
        "spark", "river", "focus", "cyclone", "maple", "harbor", "pixel", "finish",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

const BROWSER_HOST_WORDS: &str = "\
spark
river
focus
cyclone
maple
harbor
pixel
finish
rocket
salad
tiger
ember
frost
shadow
quiet
water
lemon
grape
panda
racer
ultra
crisp
vivid
storm
marker
typing
boost
shield
banana
mushroom
";

async fn send_browser_host_direct(
    state: &BrowserHostLobby,
    writer: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    player_id: PlayerId,
    message: ServerMessage,
) -> Result<(), String> {
    let relay_message = RelayClientMessage::HostToClient {
        room: state.room.clone(),
        player_id,
        message: serde_json::to_value(message)
            .map_err(|error| format!("failed to encode host direct message: {error}"))?,
    };
    writer
        .send(Message::Text(
            serde_json::to_string(&relay_message)
                .map_err(|error| format!("failed to encode relay message: {error}"))?,
        ))
        .await
        .map_err(|error| format!("failed to send host direct message: {error:?}"))
}

async fn send_browser_host_broadcast(
    state: &BrowserHostLobby,
    writer: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    message: ServerMessage,
) -> Result<(), String> {
    let relay_message = RelayClientMessage::HostBroadcast {
        room: state.room.clone(),
        message: serde_json::to_value(message)
            .map_err(|error| format!("failed to encode host broadcast message: {error}"))?,
    };
    writer
        .send(Message::Text(
            serde_json::to_string(&relay_message)
                .map_err(|error| format!("failed to encode relay message: {error}"))?,
        ))
        .await
        .map_err(|error| format!("failed to send host broadcast message: {error:?}"))
}

fn add_browser_lobby_human(
    state: &mut BrowserHostLobby,
    relay_player_id: PlayerId,
    name: &str,
) -> LobbyPlayer {
    let name = unique_lobby_name(
        state.players.iter(),
        &lobby_name_or_default(name, "player"),
    );
    let player_id = browser_next_lobby_player_id(state);
    let player = new_human_lobby_player(player_id, name, color_for_lobby_slot(state.players.len()));
    state.players.push(player.clone());
    state.next_player_id = state.next_player_id.max(player_id.0 + 1);
    state.relay_players.insert(relay_player_id, player_id);
    player
}

fn browser_next_lobby_player_id(state: &mut BrowserHostLobby) -> PlayerId {
    let player_id = first_available_player_id(&state.players, state.next_player_id);
    state.next_player_id = player_id.0 + 1;
    player_id
}

fn browser_ai_wpm(difficulty: AiDifficultySnapshot) -> u32 {
    match difficulty {
        AiDifficultySnapshot::Easy => 45,
        AiDifficultySnapshot::Hard => 85,
    }
}

fn browser_ai_label(difficulty: AiDifficultySnapshot) -> &'static str {
    match difficulty {
        AiDifficultySnapshot::Easy => "easy",
        AiDifficultySnapshot::Hard => "hard",
    }
}

fn browser_default_mod_config() -> ModConfigSnapshot {
    ModConfigSnapshot {
        word_set_id: "classic".to_string(),
        word_set_name: "Classic".to_string(),
        word_set_hash: "0000000000000001".to_string(),
        item_pack_name: "classic".to_string(),
        item_registry_hash: "0000000000000002".to_string(),
        combined_hash: "a598dc2b".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use leptos::prelude::signal;
    use typekart::game::bonus::{BonusChoice, BonusPoint, BonusState};
    use typekart::game::items::{HeldItem, ItemPickup};
    use typekart::game::race::RacePlayerId;
    use typekart_protocol::RaceResultStatus;


#[test]
fn browser_host_lobby_starts_with_ready_host() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();
    let lobby = BrowserHostLobby::new(room, "web-host".to_string());

    assert_eq!(lobby.players.len(), 1);
    assert_eq!(lobby.players[0].id, PlayerId(1));
    assert_eq!(lobby.players[0].name, "web-host");
    assert_eq!(lobby.players[0].color, AssignedColor::Cyan);
    assert!(lobby.players[0].ready);
}

#[test]
fn browser_host_assigns_joiner_from_relay_pending_id() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();
    let mut lobby = BrowserHostLobby::new(room, "host".to_string());

    let player = add_browser_lobby_human(&mut lobby, PlayerId(4), "laura");

    assert_eq!(player.id, PlayerId(2));
    assert_eq!(player.name, "laura");
    assert_eq!(player.color, AssignedColor::Red);
    assert!(!player.ready);
    assert_eq!(lobby.next_player_id, 3);
    assert_eq!(
        lobby.game_player_id_for_relay(PlayerId(4)),
        Some(PlayerId(2))
    );
}

#[test]
fn browser_host_joiner_id_does_not_collide_with_existing_ai() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();
    let mut lobby = BrowserHostLobby::new(room, "host".to_string());
    lobby.players.push(LobbyPlayer {
        id: PlayerId(2),
        name: "ai-1".to_string(),
        kind: PlayerKind::Bot,
        color: AssignedColor::Red,
        ready: true,
        connected: true,
        ai_difficulty: Some(AiDifficultySnapshot::Easy),
        ai_wpm: Some(browser_ai_wpm(AiDifficultySnapshot::Easy)),
    });

    let player = add_browser_lobby_human(&mut lobby, PlayerId(2), "laura");

    assert_eq!(lobby.players[1].name, "ai-1");
    assert_eq!(player.id, PlayerId(3));
    assert_eq!(
        lobby.game_player_id_for_relay(PlayerId(2)),
        Some(PlayerId(3))
    );
}

#[test]
fn browser_lobby_names_are_deduped() {
    let existing = [
        LobbyPlayer {
            id: PlayerId(1),
            name: "tom".to_string(),
            kind: PlayerKind::Human,
            color: AssignedColor::Cyan,
            ready: true,
            connected: true,
            ai_difficulty: None,
            ai_wpm: None,
        },
        LobbyPlayer {
            id: PlayerId(2),
            name: "tom2".to_string(),
            kind: PlayerKind::Human,
            color: AssignedColor::Red,
            ready: false,
            connected: true,
            ai_difficulty: None,
            ai_wpm: None,
        },
    ];

    assert_eq!(unique_lobby_name(existing.iter(), "tom"), "tom3");
}

#[test]
fn browser_ai_wpm_tracks_difficulty() {
    assert!(
        browser_ai_wpm(AiDifficultySnapshot::Hard) > browser_ai_wpm(AiDifficultySnapshot::Easy)
    );
}

#[test]
fn browser_generated_track_uses_shared_track_length() {
    let words = browser_generate_track_words();

    assert_eq!(words.len(), BROWSER_HOST_TRACK_WORD_COUNT);
    assert!(words.iter().all(|word| !word.is_empty()));
}

#[test]
fn browser_generated_track_includes_bonus_snapshots() {
    let words = browser_generate_track_words();
    let bonuses = browser_generate_bonus_state(&words);
    let snapshots = build_bonus_snapshots(&bonuses, std::time::Instant::now());

    assert!(!snapshots.is_empty());
    assert!(snapshots.iter().all(|point| point.choices.len() == 3));
    assert!(
        snapshots
            .iter()
            .all(|point| point.after_word_index < words.len() - 1)
    );
}

#[test]
fn browser_host_race_snapshot_uses_lobby_racers() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();
    let mut lobby = BrowserHostLobby::new(room, "host".to_string());
    let joiner = add_browser_lobby_human(&mut lobby, PlayerId(4), "laura");
    let racers = vec![lobby.players[0].clone(), joiner];

    let snapshot = browser_host_race_snapshot(
        7,
        NetworkRacePhase::Countdown {
            remaining_seconds: 3,
        },
        &lobby.mod_config,
        &racers,
        vec!["countdown 3".to_string()],
    );

    assert_eq!(snapshot.sequence, 7);
    assert_eq!(
        snapshot.phase,
        NetworkRacePhase::Countdown {
            remaining_seconds: 3
        }
    );
    assert_eq!(snapshot.players.len(), 2);
    assert_eq!(snapshot.players[0].id, PlayerId(1));
    assert_eq!(snapshot.players[1].id, PlayerId(2));
    assert_eq!(
        snapshot.track_words.first().map(String::as_str),
        Some("spark")
    );
    assert!(snapshot.bonuses.is_empty());
}

#[test]
fn browser_host_race_key_input_advances_words() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();
    let mut lobby = BrowserHostLobby::new(room, "host".to_string());
    lobby
        .item_registry
        .items
        .retain(|item| item.id.as_str() == "banana");
    let racers = vec![lobby.players[0].clone()];
    lobby.active_race = Some(browser_host_race_snapshot(
        1,
        NetworkRacePhase::Racing,
        &lobby.mod_config,
        &racers,
        Vec::new(),
    ));
    let (_connection, set_connection) = signal(ConnectionState::Disconnected);

    for ch in "spark".chars() {
        apply_browser_host_race_key_input(
            &mut lobby,
            PlayerId(1),
            ProtocolKey::Char(ch),
            set_connection,
        );
    }
    apply_browser_host_race_key_input(
        &mut lobby,
        PlayerId(1),
        ProtocolKey::Space,
        set_connection,
    );

    let player = &lobby.active_race.as_ref().unwrap().players[0];
    assert_eq!(player.word_index, 1);
    assert_eq!(player.input, "");
    assert_eq!(player.typo_index, None);
}

#[test]
fn browser_host_race_key_input_finishes_final_word_without_space() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();
    let mut lobby = BrowserHostLobby::new(room, "host".to_string());
    let racers = vec![lobby.players[0].clone()];
    let mut snapshot = browser_host_race_snapshot(
        1,
        NetworkRacePhase::Racing,
        &lobby.mod_config,
        &racers,
        Vec::new(),
    );
    snapshot.track_words = vec!["go".to_string()];
    lobby.active_race = Some(snapshot);
    let (_connection, set_connection) = signal(ConnectionState::Disconnected);

    apply_browser_host_race_key_input(
        &mut lobby,
        PlayerId(1),
        ProtocolKey::Char('g'),
        set_connection,
    );
    apply_browser_host_race_key_input(
        &mut lobby,
        PlayerId(1),
        ProtocolKey::Char('o'),
        set_connection,
    );

    let results = lobby.active_results.as_ref().unwrap();
    assert_eq!(results.placements, vec![PlayerId(1)]);
    assert_eq!(results.rows[0].player_id, PlayerId(1));
    assert_eq!(results.rows[0].progress_words, 1);
}

#[test]
fn browser_host_race_results_rank_racers_by_finish_order() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();
    let mut lobby = BrowserHostLobby::new(room, "host".to_string());
    let joiner = add_browser_lobby_human(&mut lobby, PlayerId(4), "laura");
    let racers = vec![lobby.players[0].clone(), joiner];
    let mut snapshot = browser_host_race_snapshot(
        1,
        NetworkRacePhase::Racing,
        &lobby.mod_config,
        &racers,
        Vec::new(),
    );
    snapshot.track_words = vec!["go".to_string()];
    lobby.active_race = Some(snapshot);
    let (_connection, set_connection) = signal(ConnectionState::Disconnected);

    for player_id in [PlayerId(2), PlayerId(1)] {
        apply_browser_host_race_key_input(
            &mut lobby,
            player_id,
            ProtocolKey::Char('g'),
            set_connection,
        );
        apply_browser_host_race_key_input(
            &mut lobby,
            player_id,
            ProtocolKey::Char('o'),
            set_connection,
        );
    }

    let results = lobby.active_results.as_ref().unwrap();
    assert_eq!(results.placements, vec![PlayerId(2), PlayerId(1)]);
    assert_eq!(results.rows.len(), 2);
    assert_eq!(results.rows[0].player_id, PlayerId(2));
    assert_eq!(results.rows[0].placement, 1);
    assert_eq!(results.rows[0].progress_words, 1);
    assert_eq!(results.rows[1].player_id, PlayerId(1));
    assert_eq!(results.rows[1].placement, 2);
    assert!(lobby.active_race.is_none());
}

#[test]
fn browser_host_race_results_timeout_places_unfinished_racers_by_progress() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();
    let mut lobby = BrowserHostLobby::new(room, "host".to_string());
    let joiner = add_browser_lobby_human(&mut lobby, PlayerId(4), "laura");
    let racers = vec![lobby.players[0].clone(), joiner];
    let mut snapshot = browser_host_race_snapshot(
        1,
        NetworkRacePhase::Racing,
        &lobby.mod_config,
        &racers,
        Vec::new(),
    );
    snapshot.track_words = vec!["go".to_string(), "fast".to_string()];
    lobby.active_race = Some(snapshot);
    let (_connection, set_connection) = signal(ConnectionState::Disconnected);

    for ch in "gof".chars() {
        apply_browser_host_race_key_input(
            &mut lobby,
            PlayerId(2),
            ProtocolKey::Char(ch),
            set_connection,
        );
        if ch == 'o' {
            apply_browser_host_race_key_input(
                &mut lobby,
                PlayerId(2),
                ProtocolKey::Space,
                set_connection,
            );
        }
    }
    for ch in "go".chars() {
        apply_browser_host_race_key_input(
            &mut lobby,
            PlayerId(1),
            ProtocolKey::Char(ch),
            set_connection,
        );
    }
    apply_browser_host_race_key_input(
        &mut lobby,
        PlayerId(1),
        ProtocolKey::Space,
        set_connection,
    );
    for ch in "fast".chars() {
        apply_browser_host_race_key_input(
            &mut lobby,
            PlayerId(1),
            ProtocolKey::Char(ch),
            set_connection,
        );
    }

    let first_finished_at = lobby.runtime.lifecycle.first_finished_at.unwrap();
    assert!(browser_update_race_status(
        &mut lobby,
        first_finished_at + BROWSER_HOST_POST_FIRST_FINISH_TIMEOUT
    ));

    let results = lobby.active_results.as_ref().unwrap();
    assert_eq!(results.placements, vec![PlayerId(1), PlayerId(2)]);
    assert_eq!(results.rows[0].status, RaceResultStatus::Finished);
    assert_eq!(results.rows[1].status, RaceResultStatus::TimedOut);
    assert_eq!(results.rows[1].progress_words, 1);
}

#[test]
fn browser_host_restart_command_returns_results_to_lobby() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();
    let mut lobby = BrowserHostLobby::new(room, "host".to_string());
    lobby.active_results = Some(crate::fixtures::ResultsFrame {
        placements: vec![PlayerId(1)],
        rows: Vec::new(),
        events: vec!["Race finished".to_string()],
    });
    lobby.active_race = Some(browser_host_race_snapshot(
        1,
        NetworkRacePhase::Finished,
        &lobby.mod_config,
        &lobby.players.clone(),
        Vec::new(),
    ));
    lobby.core_race = Some(browser_host_core_race(
        &lobby.players.clone(),
        browser_demo_track_words(),
    ));
    lobby.runtime.lifecycle.placements = vec![RacePlayerId(1)];

    process_browser_host_client_message(
        &mut lobby,
        PlayerId(1),
        typekart_protocol::ClientMessage::RestartRace,
        signal(ConnectionState::Disconnected).1,
    );

    assert!(lobby.active_results.is_none());
    assert!(lobby.active_race.is_none());
    assert!(lobby.core_race.is_none());
    assert!(lobby.runtime.lifecycle.placements.is_empty());
    assert!(lobby.events.iter().any(|event| event == "Returned to lobby"));
}

#[test]
fn browser_host_race_key_input_marks_and_clears_typos() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();
    let mut lobby = BrowserHostLobby::new(room, "host".to_string());
    let racers = vec![lobby.players[0].clone()];
    lobby.active_race = Some(browser_host_race_snapshot(
        1,
        NetworkRacePhase::Racing,
        &lobby.mod_config,
        &racers,
        Vec::new(),
    ));
    let (_connection, set_connection) = signal(ConnectionState::Disconnected);

    apply_browser_host_race_key_input(
        &mut lobby,
        PlayerId(1),
        ProtocolKey::Char('x'),
        set_connection,
    );
    assert_eq!(
        lobby.active_race.as_ref().unwrap().players[0].typo_index,
        Some(0)
    );

    apply_browser_host_race_key_input(
        &mut lobby,
        PlayerId(1),
        ProtocolKey::Backspace,
        set_connection,
    );
    let player = &lobby.active_race.as_ref().unwrap().players[0];
    assert_eq!(player.input, "");
    assert_eq!(player.typo_index, None);
}

#[test]
fn browser_host_bonus_word_claims_choice_after_space() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();
    let mut lobby = BrowserHostLobby::new(room, "host".to_string());
    lobby
        .item_registry
        .items
        .retain(|item| item.id.as_str() == "shield");
    let racers = vec![lobby.players[0].clone()];
    let mut snapshot = browser_host_race_snapshot(
        1,
        NetworkRacePhase::Racing,
        &lobby.mod_config,
        &racers,
        Vec::new(),
    );
    snapshot.track_words = vec!["one".to_string(), "two".to_string()];
    snapshot.players[0].word_index = 1;
    lobby.active_race = Some(snapshot);
    lobby.bonuses = BonusState::with_points(
        vec![BonusPoint::new(
            0,
            [
                BonusChoice::available("dash"),
                BonusChoice::available("drift"),
                BonusChoice::available("turbo"),
            ],
        )],
        vec!["dash".to_string(), "drift".to_string(), "turbo".to_string()],
    );
    let (_connection, set_connection) = signal(ConnectionState::Disconnected);

    for ch in "dash".chars() {
        apply_browser_host_race_key_input(
            &mut lobby,
            PlayerId(1),
            ProtocolKey::Char(ch),
            set_connection,
        );
    }

    let player = &lobby.active_race.as_ref().unwrap().players[0];
    assert_eq!(player.word_index, 1);
    assert_eq!(player.input, "dash");
    assert!(lobby.runtime.bonus_attempts.contains_key(&PlayerId(1)));
    assert!(matches!(
        lobby.active_race.as_ref().unwrap().bonuses[0].choices[0].status,
        typekart_protocol::BonusChoiceSnapshotStatus::Available
    ));

    apply_browser_host_race_key_input(
        &mut lobby,
        PlayerId(1),
        ProtocolKey::Space,
        set_connection,
    );

    let player = &lobby.active_race.as_ref().unwrap().players[0];
    assert_eq!(player.word_index, 1);
    assert_eq!(player.input, "");
    assert_eq!(player.typo_index, None);
    assert!(!lobby.runtime.bonus_attempts.contains_key(&PlayerId(1)));
    assert_eq!(lobby.runtime.spent_bonus_gaps.get(&PlayerId(1)), Some(&0));
    assert!(matches!(
        lobby.active_race.as_ref().unwrap().bonuses[0].choices[0].status,
        typekart_protocol::BonusChoiceSnapshotStatus::Cooldown { .. }
    ));
    assert!(lobby.events.iter().any(|event| event.contains("got")));
    assert!(
        !lobby.active_race.as_ref().unwrap().events[0].contains("typed"),
        "bonus pickup or item event should be visible in the race feed"
    );
}

#[test]
fn browser_host_banana_activation_resets_target_and_renders_impact() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();
    let mut lobby = BrowserHostLobby::new(room, "host".to_string());
    let joiner = add_browser_lobby_human(&mut lobby, PlayerId(4), "laura");
    let racers = vec![lobby.players[0].clone(), joiner];
    let mut snapshot = browser_host_race_snapshot(
        1,
        NetworkRacePhase::Racing,
        &lobby.mod_config,
        &racers,
        Vec::new(),
    );
    snapshot.track_words = vec!["one".to_string(), "two".to_string()];
    snapshot.players[0].word_index = 0;
    snapshot.players[1].word_index = 1;
    snapshot.players[1].input = "twx".to_string();
    snapshot.players[1].typo_index = Some(2);
    lobby.active_race = Some(snapshot);
    browser_ensure_core_race(&mut lobby);

    activate_browser_item_pickup(
        &mut lobby,
        PlayerId(1),
        ItemPickup::Held(HeldItem::Banana),
        std::time::Instant::now(),
    );
    browser_sync_active_race_from_core(&mut lobby);

    let target = lobby
        .active_race
        .as_ref()
        .unwrap()
        .players
        .iter()
        .find(|player| player.id == PlayerId(2))
        .unwrap();
    assert_eq!(target.input, "");
    assert_eq!(target.typo_index, None);
    assert!(target.impact_cue.is_some());
    assert!(!target.stunned);
}

#[test]
fn browser_host_ai_tick_advances_bot_racers() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();
    let mut lobby = BrowserHostLobby::new(room, "host".to_string());
    process_browser_host_client_message(
        &mut lobby,
        PlayerId(1),
        typekart_protocol::ClientMessage::AddAi,
        signal(ConnectionState::Disconnected).1,
    );
    let racers = lobby.players.clone();
    lobby.active_race = Some(browser_host_race_snapshot(
        1,
        NetworkRacePhase::Racing,
        &lobby.mod_config,
        &racers,
        Vec::new(),
    ));

    assert!(apply_browser_host_ai_tick(&mut lobby, 1_000));

    let ai = lobby
        .active_race
        .as_ref()
        .unwrap()
        .players
        .iter()
        .find(|player| player.kind == PlayerKind::Bot)
        .unwrap();
    assert!(!ai.input.is_empty() || ai.word_index > 0);
}

#[test]
fn browser_host_ai_tick_finishes_bot_with_enough_budget() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();
    let mut lobby = BrowserHostLobby::new(room, "host".to_string());
    process_browser_host_client_message(
        &mut lobby,
        PlayerId(1),
        typekart_protocol::ClientMessage::AddAi,
        signal(ConnectionState::Disconnected).1,
    );
    let ai_id = lobby
        .players
        .iter()
        .find(|player| player.kind == PlayerKind::Bot)
        .unwrap()
        .id;
    lobby
        .players
        .iter_mut()
        .find(|player| player.id == ai_id)
        .unwrap()
        .ai_wpm = Some(1000);
    let racers = lobby.players.clone();
    lobby.active_race = Some(browser_host_race_snapshot(
        1,
        NetworkRacePhase::Racing,
        &lobby.mod_config,
        &racers,
        Vec::new(),
    ));

    assert!(apply_browser_host_ai_tick(&mut lobby, 60_000));

    let ai = lobby
        .active_race
        .as_ref()
        .unwrap()
        .players
        .iter()
        .find(|player| player.id == ai_id)
        .unwrap();
    assert!(ai.finished);
}

#[test]
fn browser_host_ai_tick_ignores_queued_countdown_ticks() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();
    let mut lobby = BrowserHostLobby::new(room, "host".to_string());
    process_browser_host_client_message(
        &mut lobby,
        PlayerId(1),
        typekart_protocol::ClientMessage::AddAi,
        signal(ConnectionState::Disconnected).1,
    );
    let racers = lobby.players.clone();
    lobby.active_race = Some(browser_host_race_snapshot(
        1,
        NetworkRacePhase::Racing,
        &lobby.mod_config,
        &racers,
        Vec::new(),
    ));
    lobby.ai_last_tick_ms = Some(browser_now_ms());

    assert!(!apply_browser_host_ai_tick(
        &mut lobby,
        BROWSER_HOST_AI_TICK_MS
    ));

    let ai = lobby
        .active_race
        .as_ref()
        .unwrap()
        .players
        .iter()
        .find(|player| player.kind == PlayerKind::Bot)
        .unwrap();
    assert_eq!(ai.word_index, 0);
    assert_eq!(ai.input, "");
}
}
