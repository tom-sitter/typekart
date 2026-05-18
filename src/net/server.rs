//! Authoritative TCP host for multiplayer races.

use std::{
    collections::HashMap,
    io::{self, BufRead, BufReader},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use rand::{Rng, thread_rng};

use crate::game::{
    ai::AiDifficulty,
    bonus::{BonusChoiceStatus, BonusState, claim_bonus_choice},
    effects::ActiveEffect,
    items::{
        BananaDisplayConfig, HeldItem, ItemPickup, ItemRegistry, ItemRollContext, RacePositionBand,
        RacerPosition, select_nearest_banana_target,
    },
    mods::ActiveModConfig,
    race::{PlayerColorId, RacePlayerId, RaceState},
    track::{Track, WordList},
    typing::{KeyAction, TypingEvent, first_typo_index},
};

use super::log::{SharedNetworkLog, push_network_log};
use super::protocol::{
    AssignedColor, AttackDirectionSnapshot, BonusChoiceSnapshot, BonusChoiceSnapshotStatus,
    BonusPointSnapshot, ClientMessage, ImpactCueSnapshot, ImpactCueSnapshotKind,
    ItemCuePlacementSnapshot, ItemCueSnapshot, ItemCueSnapshotKind, LobbyPlayer, NetworkRacePhase,
    PlayerId, PlayerKind, PlayerSnapshot, ProtocolKey, RaceDeltaSnapshot, RaceResultRow,
    RaceResultStatus, RaceSnapshot, ServerMessage, WordOverrideSnapshot, version_mismatch_message,
};
use super::transport::{read_client_message, write_server_message as write_framed_server_message};

const COLOR_ROTATION: [AssignedColor; 6] = [
    AssignedColor::Cyan,
    AssignedColor::Red,
    AssignedColor::Green,
    AssignedColor::Blue,
    AssignedColor::Yellow,
    AssignedColor::Magenta,
];
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

macro_rules! server_eprintln {
    ($($arg:tt)*) => {
        if SERVER_CONSOLE_LOGGING.load(std::sync::atomic::Ordering::Relaxed) {
            eprintln!($($arg)*);
        }
    };
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

struct ConnectedClient {
    player_id: PlayerId,
    stream: TcpStream,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct JoinHello {
    name: String,
    client_version: String,
}

struct HostState {
    players: Vec<LobbyPlayer>,
    clients: Vec<ConnectedClient>,
    race: RaceState,
    ai_racers: HashMap<PlayerId, NetworkAiRacer>,
    word_list: WordList,
    bonuses: BonusState,
    item_registry: ItemRegistry,
    active_mod_config: ActiveModConfig,
    max_players: usize,
    ai_difficulty: AiDifficulty,
    bonus_attempts: HashMap<PlayerId, NetworkBonusAttempt>,
    spent_bonus_gaps: HashMap<PlayerId, usize>,
    player_effects: HashMap<PlayerId, NetworkPlayerEffects>,
    phase: NetworkRacePhase,
    snapshot_sequence: u64,
    events: Vec<String>,
    placements: Vec<PlayerId>,
    first_finished_at: Option<Instant>,
    debug_log: Option<SharedNetworkLog>,
    race_results_sent: bool,
}

#[derive(Debug, Clone)]
struct NetworkAiRacer {
    difficulty: AiDifficulty,
    words_per_minute: f64,
    char_budget: f64,
    last_update: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NetworkBonusAttempt {
    point_index: usize,
    choice_index: usize,
}

#[derive(Debug, Clone, Default)]
struct NetworkPlayerEffects {
    stunned_until: Option<Instant>,
    impact_cue: Option<NetworkImpactCue>,
    item_cue: Option<NetworkItemCue>,
}

#[derive(Debug, Clone, Copy)]
struct NetworkImpactCue {
    kind: ImpactCueSnapshotKind,
    until: Instant,
}

#[derive(Debug, Clone)]
struct NetworkItemCue {
    kind: NetworkItemCueKind,
    ascii_label: String,
    unicode_label: String,
    placement: ItemCuePlacementSnapshot,
    until: Instant,
}

#[derive(Debug, Clone, Copy)]
enum NetworkItemCueKind {
    Banana { direction: AttackDirectionSnapshot },
    BlueShell { direction: AttackDirectionSnapshot },
    SquidInk,
}

pub fn run_host(config: HostConfig) -> Result<()> {
    SERVER_CONSOLE_LOGGING.store(config.console_logging, Ordering::Relaxed);

    validate_host_capacity(config.max_players, config.ai_racer_count)?;

    let listener = TcpListener::bind(config.bind)
        .with_context(|| format!("failed to bind host socket at {}", config.bind))?;
    let local_addr = listener
        .local_addr()
        .context("failed to read host address")?;
    if let Some(ready_signal) = config.ready_signal {
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

    let bonuses = BonusState::generate(&config.track, &config.word_list);
    let mut race = RaceState::new(config.track.clone());
    let mut players = Vec::new();
    let mut next_player_id = 1;
    if let Some(host_name) = config.host_name {
        race.add_player(
            RacePlayerId(1),
            host_name.clone(),
            PlayerColorId::Cyan,
            std::time::Instant::now(),
        );
        players.push(LobbyPlayer {
            id: PlayerId(1),
            name: host_name,
            kind: PlayerKind::Human,
            color: COLOR_ROTATION[0],
            ready: true,
            connected: true,
            ai_difficulty: None,
            ai_wpm: None,
        });
        next_player_id = 2;
    }
    let ai_racers = add_network_ai_racers(
        &mut race,
        &mut players,
        config.ai_racer_count,
        config.ai_difficulty,
        std::time::Instant::now(),
    );

    let state = Arc::new(Mutex::new(HostState {
        players,
        clients: Vec::new(),
        race,
        ai_racers,
        word_list: config.word_list,
        bonuses,
        item_registry: config.item_registry,
        active_mod_config: config.active_mod_config,
        max_players: config.max_players,
        ai_difficulty: config.ai_difficulty,
        bonus_attempts: HashMap::new(),
        spent_bonus_gaps: HashMap::new(),
        player_effects: HashMap::new(),
        phase: NetworkRacePhase::WaitingForHost,
        snapshot_sequence: 0,
        events: Vec::new(),
        placements: Vec::new(),
        first_finished_at: None,
        debug_log: config.debug_log,
        race_results_sent: false,
    }));

    server_println!("TypeKart host listening on {local_addr}");
    if has_embedded_host_player(&state) {
        server_println!("Host lobby commands: start, lobby, ready, unready");
        spawn_host_command_loop(Arc::clone(&state));
    }
    server_println!("Waiting for joiners. Press Ctrl-C to stop.");

    for stream in listener.incoming() {
        let stream = stream.context("failed to accept client connection")?;
        let peer = stream.peer_addr().ok();

        let join_hello = match read_join_hello(&stream) {
            Ok(join_hello) => join_hello,
            Err(error) => {
                server_eprintln!("Rejected connection: {error:#}");
                continue;
            }
        };
        let requested_player_name = join_hello.name;

        let (player_id, assigned_color, player_name) = {
            let mut state = state.lock().expect("host state poisoned");
            if join_hello.client_version != env!("CARGO_PKG_VERSION") {
                send_server_message(
                    stream,
                    &ServerMessage::Error {
                        message: version_mismatch_message(
                            env!("CARGO_PKG_VERSION"),
                            &join_hello.client_version,
                        ),
                    },
                )?;
                push_network_log(
                    &state.debug_log,
                    format!(
                        "join rejected: version mismatch name={} host_version={} client_version={}",
                        requested_player_name,
                        env!("CARGO_PKG_VERSION"),
                        join_hello.client_version
                    ),
                );
                continue;
            }

            let connected_players = connected_player_count(&state.players);
            if connected_players >= config.max_players {
                send_server_message(
                    stream,
                    &ServerMessage::Error {
                        message: format!(
                            "Lobby is full: {connected_players}/{} connected players",
                            config.max_players
                        ),
                    },
                )?;
                push_network_log(
                    &state.debug_log,
                    format!(
                        "join rejected: lobby full {connected_players}/{}",
                        config.max_players
                    ),
                );
                continue;
            }

            let player_name = unique_player_name(&requested_player_name, &state.players);
            let player_id = first_available_player_id(&state.players, next_player_id);
            let assigned_color = first_available_color(&state.players);
            let write_stream = match welcome_joiner(&stream, player_id, assigned_color) {
                Ok(write_stream) => write_stream,
                Err(error) => {
                    server_eprintln!("Rejected connection: {error:#}");
                    continue;
                }
            };

            state.clients.push(ConnectedClient {
                player_id,
                stream: write_stream,
            });
            state.players.push(new_human_lobby_player(
                player_id,
                player_name.clone(),
                assigned_color,
            ));
            if matches!(
                state.phase,
                NetworkRacePhase::Lobby | NetworkRacePhase::WaitingForHost
            ) {
                state.race.add_player(
                    RacePlayerId(player_id.0),
                    player_name.clone(),
                    assigned_color.into(),
                    std::time::Instant::now(),
                );
            }
            push_event(&mut state, format!("{player_name} joined"));
            push_network_log(
                &state.debug_log,
                format!(
                    "{player_name} joined player={} color={assigned_color:?}",
                    player_id.0
                ),
            );
            print_lobby_snapshot(&state.players);
            broadcast_lobby_snapshot(&mut state)?;

            (player_id, assigned_color, player_name)
        };

        server_println!(
            "{} joined as player {} ({assigned_color:?}){}",
            player_name,
            player_id.0,
            peer.map(|addr| format!(" from {addr}")).unwrap_or_default()
        );

        let state_for_client = Arc::clone(&state);
        thread::spawn(move || handle_client_messages(player_id, stream, state_for_client));
        next_player_id = next_player_id.max(player_id.0 + 1);
    }

    Ok(())
}

fn add_network_ai_racers(
    race: &mut RaceState,
    players: &mut Vec<LobbyPlayer>,
    ai_racer_count: usize,
    ai_difficulty: AiDifficulty,
    now: Instant,
) -> HashMap<PlayerId, NetworkAiRacer> {
    let mut ai_racers = HashMap::new();
    let mut rng = thread_rng();
    for index in 0..ai_racer_count {
        let player_id = PlayerId(index as u64 + 2);
        let name = format!("ai-{}", index + 1);
        let color = COLOR_ROTATION[index + 1];
        race.add_player(RacePlayerId(player_id.0), name.clone(), color.into(), now);
        ai_racers.insert(
            player_id,
            NetworkAiRacer {
                words_per_minute: rng.gen_range(ai_difficulty.wpm_range()),
                difficulty: ai_difficulty,
                char_budget: 0.0,
                last_update: now,
            },
        );
        players.push(LobbyPlayer {
            id: player_id,
            name,
            kind: PlayerKind::Bot,
            color,
            ready: true,
            connected: true,
            ai_difficulty: Some(ai_difficulty.into()),
            ai_wpm: ai_racers
                .get(&player_id)
                .map(|racer| racer.words_per_minute.round() as u32),
        });
    }
    ai_racers
}

fn validate_host_capacity(max_players: usize, ai_racer_count: usize) -> Result<()> {
    if max_players == 0 || max_players > COLOR_ROTATION.len() {
        bail!("max players must be between 1 and {}", COLOR_ROTATION.len());
    }
    if ai_racer_count >= max_players {
        bail!("ai racers must be less than max players so the host has a slot");
    }
    Ok(())
}

fn spawn_host_command_loop(state: Arc<Mutex<HostState>>) {
    thread::spawn(move || {
        for line in io::stdin().lock().lines() {
            let Ok(command) = line else {
                break;
            };

            match command.trim() {
                "ready" => update_host_ready(&state, true),
                "unready" => update_host_ready(&state, false),
                "lobby" => {
                    let state = state.lock().expect("host state poisoned");
                    print_lobby_snapshot(&state.players);
                }
                "start" => start_countdown(Arc::clone(&state)),
                "" if command == " " => start_countdown(Arc::clone(&state)),
                "" => {}
                other => {
                    if current_phase(&state) == NetworkRacePhase::Racing {
                        apply_line_input(&state, PlayerId(1), other);
                    } else {
                        server_println!("Unknown host command: {other}");
                    }
                }
            }
        }
    });
}

fn has_embedded_host_player(state: &Arc<Mutex<HostState>>) -> bool {
    state
        .lock()
        .expect("host state poisoned")
        .players
        .iter()
        .any(|player| player.id == PlayerId(1))
}

fn update_host_ready(state: &Arc<Mutex<HostState>>, ready: bool) {
    let mut state = state.lock().expect("host state poisoned");
    if let Some(host) = state
        .players
        .iter_mut()
        .find(|player| player.id == PlayerId(1))
    {
        host.ready = ready;
        server_println!(
            "{} is {}",
            host.name,
            if ready { "ready" } else { "not ready" }
        );
    }
    print_lobby_snapshot(&state.players);
    if let Err(error) = broadcast_lobby_snapshot(&mut state) {
        server_eprintln!("Failed to broadcast lobby snapshot: {error:#}");
    }
}

fn lobby_can_manage_roster(state: &HostState) -> bool {
    matches!(
        state.phase,
        NetworkRacePhase::Lobby | NetworkRacePhase::WaitingForHost
    )
}

fn add_lobby_ai_racer(state: &mut HostState) -> Result<()> {
    if !lobby_can_manage_roster(state) {
        bail!("AI racers can only be changed in the lobby");
    }
    if connected_player_count(&state.players) >= state.max_players {
        bail!("Lobby is full");
    }

    let player_id = first_available_player_id(&state.players, 2);
    let color = first_available_color(&state.players);
    let name = next_ai_name(&state.players);
    let now = Instant::now();
    let mut rng = thread_rng();
    let words_per_minute = rng.gen_range(state.ai_difficulty.wpm_range());

    state
        .race
        .add_player(RacePlayerId(player_id.0), name.clone(), color.into(), now);
    state.ai_racers.insert(
        player_id,
        NetworkAiRacer {
            difficulty: state.ai_difficulty,
            words_per_minute,
            char_budget: 0.0,
            last_update: now,
        },
    );
    state.players.push(LobbyPlayer {
        id: player_id,
        name: name.clone(),
        kind: PlayerKind::Bot,
        color,
        ready: true,
        connected: true,
        ai_difficulty: Some(state.ai_difficulty.into()),
        ai_wpm: Some(words_per_minute.round() as u32),
    });
    push_event(state, format!("{name} added"));
    push_network_log(
        &state.debug_log,
        format!(
            "ai added player={} difficulty={} wpm={:.0}",
            player_id.0,
            state.ai_difficulty.name(),
            words_per_minute
        ),
    );

    Ok(())
}

fn remove_lobby_player(state: &mut HostState, player_id: PlayerId) -> Result<()> {
    if !lobby_can_manage_roster(state) {
        bail!("Racers can only be removed in the lobby");
    }
    if player_id == PlayerId(1) {
        bail!("Host cannot be removed");
    }

    let Some(player) = state.players.iter().find(|player| player.id == player_id) else {
        bail!("Selected racer is no longer in the lobby");
    };
    let name = player.name.clone();
    let kind = player.kind;

    if kind == PlayerKind::Human {
        for client in state
            .clients
            .iter_mut()
            .filter(|client| client.player_id == player_id)
        {
            let _ = write_server_message(
                &mut client.stream,
                &ServerMessage::Error {
                    message: "Kicked by host".to_string(),
                },
            );
            let _ = client.stream.shutdown(Shutdown::Both);
        }
        state.clients.retain(|client| client.player_id != player_id);
    }

    state.players.retain(|player| player.id != player_id);
    state
        .race
        .players
        .retain(|player| player.id != RacePlayerId(player_id.0));
    state.ai_racers.remove(&player_id);
    state.bonus_attempts.remove(&player_id);
    state.spent_bonus_gaps.remove(&player_id);
    state.player_effects.remove(&player_id);
    push_event(
        state,
        match kind {
            PlayerKind::Human => format!("{name} kicked"),
            PlayerKind::Bot => format!("{name} removed"),
        },
    );
    push_network_log(
        &state.debug_log,
        format!(
            "lobby removed player={} name={name} kind={kind:?}",
            player_id.0
        ),
    );

    Ok(())
}

fn set_lobby_ai_difficulty(
    state: &mut HostState,
    player_id: Option<PlayerId>,
    difficulty: AiDifficulty,
) -> Result<()> {
    if !lobby_can_manage_roster(state) {
        bail!("AI difficulty can only be changed in the lobby");
    }

    state.ai_difficulty = difficulty;
    let Some(player_id) = player_id else {
        push_event(
            state,
            format!("New AI difficulty set to {}", difficulty.name()),
        );
        push_network_log(
            &state.debug_log,
            format!("default ai difficulty={}", difficulty.name()),
        );
        return Ok(());
    };

    let Some(ai) = state.ai_racers.get_mut(&player_id) else {
        push_event(
            state,
            format!("New AI difficulty set to {}", difficulty.name()),
        );
        push_network_log(
            &state.debug_log,
            format!("default ai difficulty={}", difficulty.name()),
        );
        return Ok(());
    };

    let mut rng = thread_rng();
    ai.difficulty = difficulty;
    ai.words_per_minute = rng.gen_range(difficulty.wpm_range());
    let words_per_minute = ai.words_per_minute;
    ai.char_budget = 0.0;
    ai.last_update = Instant::now();
    let _ = ai;
    if let Some(player) = state
        .players
        .iter_mut()
        .find(|player| player.id == player_id)
    {
        player.ai_difficulty = Some(difficulty.into());
        player.ai_wpm = Some(words_per_minute.round() as u32);
        let name = player.name.clone();
        let _ = player;
        push_event(state, format!("{name} set to {}", difficulty.name()));
        push_network_log(
            &state.debug_log,
            format!(
                "ai difficulty player={} difficulty={} wpm={:.0}",
                player_id.0,
                difficulty.name(),
                words_per_minute
            ),
        );
    }

    Ok(())
}

fn next_ai_name(players: &[LobbyPlayer]) -> String {
    let mut index = 1;
    loop {
        let name = format!("ai-{index}");
        if !players
            .iter()
            .any(|player| player.name.eq_ignore_ascii_case(&name))
        {
            return name;
        }
        index += 1;
    }
}

fn unique_player_name(requested_name: &str, players: &[LobbyPlayer]) -> String {
    let base_name = requested_name.trim();
    if !connected_name_exists(base_name, players) {
        return base_name.to_string();
    }

    let mut suffix = 2;
    loop {
        let candidate = format!("{base_name}{suffix}");
        if !connected_name_exists(&candidate, players) {
            return candidate;
        }
        suffix += 1;
    }
}

fn connected_name_exists(name: &str, players: &[LobbyPlayer]) -> bool {
    players
        .iter()
        .any(|player| player.connected && player.name.eq_ignore_ascii_case(name))
}

fn new_human_lobby_player(
    id: PlayerId,
    name: impl Into<String>,
    color: AssignedColor,
) -> LobbyPlayer {
    LobbyPlayer {
        id,
        name: name.into(),
        kind: PlayerKind::Human,
        color,
        ready: id == PlayerId(1),
        connected: true,
        ai_difficulty: None,
        ai_wpm: None,
    }
}

fn read_join_hello(stream: &TcpStream) -> Result<JoinHello> {
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .context("failed to clone client stream for reading")?,
    );
    let Some(message) = read_client_message(&mut reader).context("failed to read client hello")?
    else {
        bail!("client disconnected before hello");
    };
    let ClientMessage::Hello {
        name,
        client_version,
    } = message
    else {
        send_server_message(
            stream
                .try_clone()
                .context("failed to clone client stream for error response")?,
            &ServerMessage::Error {
                message: "Expected hello message".to_string(),
            },
        )?;
        bail!("client sent non-hello first message");
    };

    if name.trim().is_empty() {
        send_server_message(
            stream
                .try_clone()
                .context("failed to clone client stream for error response")?,
            &ServerMessage::Error {
                message: "Name cannot be empty".to_string(),
            },
        )?;
        bail!("client sent empty name");
    }

    if client_version.trim().is_empty() {
        send_server_message(
            stream
                .try_clone()
                .context("failed to clone client stream for error response")?,
            &ServerMessage::Error {
                message: "Client version cannot be empty".to_string(),
            },
        )?;
        bail!("client sent empty version");
    }

    Ok(JoinHello {
        name: name.trim().to_string(),
        client_version: client_version.trim().to_string(),
    })
}

fn welcome_joiner(
    stream: &TcpStream,
    player_id: PlayerId,
    assigned_color: AssignedColor,
) -> Result<TcpStream> {
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

    Ok(write_stream)
}

fn connected_player_count(players: &[LobbyPlayer]) -> usize {
    players.iter().filter(|player| player.connected).count()
}

fn current_race_connected_player_count(state: &HostState) -> usize {
    state
        .race
        .players
        .iter()
        .filter(|player| player.connected)
        .count()
}

fn first_available_player_id(players: &[LobbyPlayer], start_at: u64) -> PlayerId {
    let mut id = start_at;
    while players.iter().any(|player| player.id == PlayerId(id)) {
        id += 1;
    }
    PlayerId(id)
}

fn first_available_color(players: &[LobbyPlayer]) -> AssignedColor {
    COLOR_ROTATION
        .iter()
        .copied()
        .find(|color| {
            !players
                .iter()
                .any(|player| player.connected && player.color == *color)
        })
        .expect("color rotation covers the configured player limit")
}

fn handle_client_messages(player_id: PlayerId, stream: TcpStream, state: Arc<Mutex<HostState>>) {
    let mut reader = BufReader::new(stream);
    loop {
        let message = match read_client_message(&mut reader) {
            Ok(Some(message)) => message,
            Ok(None) => break,
            Err(_) => continue,
        };

        match message {
            ClientMessage::Rename { name } => {
                let mut state = state.lock().expect("host state poisoned");
                if let Err(error) = rename_lobby_player(&mut state, player_id, &name) {
                    push_event(&mut state, error.to_string());
                }
                print_lobby_snapshot(&state.players);
                if let Err(error) = broadcast_lobby_snapshot(&mut state) {
                    server_eprintln!("Failed to broadcast lobby snapshot: {error:#}");
                }
            }
            ClientMessage::SetReady { ready } => {
                let mut state = state.lock().expect("host state poisoned");
                if let Some(player) = state
                    .players
                    .iter_mut()
                    .find(|player| player.id == player_id)
                {
                    let name = player.name.clone();
                    player.ready = ready;
                    server_println!("{} is {}", name, if ready { "ready" } else { "not ready" });
                    push_event(
                        &mut state,
                        format!("{} {}", name, if ready { "ready" } else { "not ready" }),
                    );
                    push_network_log(&state.debug_log, format!("{name} ready={ready}"));
                }
                print_lobby_snapshot(&state.players);
                if let Err(error) = broadcast_lobby_snapshot(&mut state) {
                    server_eprintln!("Failed to broadcast lobby snapshot: {error:#}");
                }
            }
            ClientMessage::StartCountdown => {
                if player_id == PlayerId(1) {
                    {
                        let state = state.lock().expect("host state poisoned");
                        push_network_log(&state.debug_log, "host requested countdown");
                    }
                    start_countdown(Arc::clone(&state));
                } else {
                    server_println!(
                        "Ignoring start request from non-host player {}",
                        player_id.0
                    );
                }
            }
            ClientMessage::AddAi if player_id == PlayerId(1) => {
                let mut state = state.lock().expect("host state poisoned");
                if let Err(error) = add_lobby_ai_racer(&mut state) {
                    push_event(&mut state, error.to_string());
                }
                print_lobby_snapshot(&state.players);
                if let Err(error) = broadcast_lobby_snapshot(&mut state) {
                    server_eprintln!("Failed to broadcast lobby snapshot: {error:#}");
                }
            }
            ClientMessage::RemoveLobbyPlayer { player_id: target } if player_id == PlayerId(1) => {
                let mut state = state.lock().expect("host state poisoned");
                if let Err(error) = remove_lobby_player(&mut state, target) {
                    push_event(&mut state, error.to_string());
                }
                print_lobby_snapshot(&state.players);
                if let Err(error) = broadcast_lobby_snapshot(&mut state) {
                    server_eprintln!("Failed to broadcast lobby snapshot: {error:#}");
                }
            }
            ClientMessage::SetAiDifficulty {
                player_id: target,
                difficulty,
            } if player_id == PlayerId(1) => {
                let mut state = state.lock().expect("host state poisoned");
                if let Err(error) = set_lobby_ai_difficulty(&mut state, target, difficulty.into()) {
                    push_event(&mut state, error.to_string());
                }
                print_lobby_snapshot(&state.players);
                if let Err(error) = broadcast_lobby_snapshot(&mut state) {
                    server_eprintln!("Failed to broadcast lobby snapshot: {error:#}");
                }
            }
            ClientMessage::RestartRace => {
                if player_id == PlayerId(1) {
                    let mut state = state.lock().expect("host state poisoned");
                    if let Err(error) = return_to_lobby(&mut state) {
                        server_eprintln!("Failed to return to lobby: {error:#}");
                        push_network_log(
                            &state.debug_log,
                            format!("failed to return to lobby: {error:#}"),
                        );
                    }
                } else {
                    server_println!(
                        "Ignoring rematch request from non-host player {}",
                        player_id.0
                    );
                }
            }
            ClientMessage::KeyInput { key, .. } => {
                let now = std::time::Instant::now();
                let mut state = state.lock().expect("host state poisoned");
                if state.phase != NetworkRacePhase::Racing {
                    continue;
                }
                let action = protocol_key_to_action(key);
                push_network_log(
                    &state.debug_log,
                    format!("input player={} key={key:?}", player_id.0),
                );
                if apply_network_key_input(&mut state, player_id, action, now) {
                    update_race_status(&mut state, now);
                    if state.phase == NetworkRacePhase::Finished {
                        if let Err(error) = broadcast_race_snapshot(&mut state) {
                            server_eprintln!("Failed to broadcast race snapshot: {error:#}");
                        }
                        server_println!("Race finished");
                        if let Err(error) = broadcast_race_results_once(&mut state) {
                            server_eprintln!("Failed to broadcast race results: {error:#}");
                        }
                    } else if let Err(error) = broadcast_race_delta(&mut state) {
                        server_eprintln!("Failed to broadcast race delta: {error:#}");
                    }
                }
            }
            ClientMessage::Leave => break,
            _ => {}
        }
    }

    let mut state = state.lock().expect("host state poisoned");
    let was_race_screen_phase = matches!(
        state.phase,
        NetworkRacePhase::Countdown { .. } | NetworkRacePhase::Racing | NetworkRacePhase::Finished
    );
    if handle_player_disconnect(&mut state, player_id, std::time::Instant::now()) {
        return;
    }
    print_lobby_snapshot(&state.players);
    if let Err(error) = broadcast_lobby_snapshot(&mut state) {
        server_eprintln!("Failed to broadcast lobby snapshot: {error:#}");
    }
    if was_race_screen_phase && let Err(error) = broadcast_race_snapshot(&mut state) {
        server_eprintln!("Failed to broadcast race snapshot: {error:#}");
    }
    if state.phase == NetworkRacePhase::Finished
        && let Err(error) = broadcast_race_results_once(&mut state)
    {
        server_eprintln!("Failed to broadcast race results: {error:#}");
    }
}

fn rename_lobby_player(
    state: &mut HostState,
    player_id: PlayerId,
    requested_name: &str,
) -> Result<()> {
    if !matches!(
        state.phase,
        NetworkRacePhase::Lobby | NetworkRacePhase::WaitingForHost
    ) {
        bail!("Renaming is only available in the lobby");
    }

    let requested_name = requested_name.trim();
    if requested_name.is_empty() {
        bail!("Name cannot be empty");
    }

    let existing_name = state
        .players
        .iter()
        .find(|player| {
            player.id == player_id && player.connected && player.kind == PlayerKind::Human
        })
        .map(|player| player.name.clone())
        .context("Player is no longer in the lobby")?;
    let other_players = state
        .players
        .iter()
        .filter(|player| player.id != player_id)
        .cloned()
        .collect::<Vec<_>>();
    let new_name = unique_player_name(requested_name, &other_players);

    if let Some(player) = state
        .players
        .iter_mut()
        .find(|player| player.id == player_id)
    {
        player.name = new_name.clone();
    }
    if let Some(racer) = state
        .race
        .players
        .iter_mut()
        .find(|racer| racer.id == RacePlayerId(player_id.0))
    {
        racer.name = new_name.clone();
    }
    push_event(state, format!("{existing_name} renamed to {new_name}"));
    push_network_log(
        &state.debug_log,
        format!(
            "player={} renamed {existing_name} -> {new_name}",
            player_id.0
        ),
    );

    Ok(())
}

fn handle_player_disconnect(state: &mut HostState, player_id: PlayerId, now: Instant) -> bool {
    if let Some(player) = state
        .players
        .iter_mut()
        .find(|player| player.id == player_id)
    {
        let name = player.name.clone();
        player.connected = false;
        player.ready = false;
        server_println!("{name} disconnected");
        push_event(state, format!("{name} disconnected"));
        push_network_log(&state.debug_log, format!("{name} disconnected"));
    }
    if let Some(player) = state
        .race
        .players
        .iter_mut()
        .find(|player| player.id == RacePlayerId(player_id.0))
    {
        player.connected = false;
    }
    state.bonus_attempts.remove(&player_id);
    state.spent_bonus_gaps.remove(&player_id);
    state.player_effects.remove(&player_id);
    state.clients.retain(|client| client.player_id != player_id);

    if player_id == PlayerId(1) {
        close_game_for_joiners(state, "Game closed: host left");
        return true;
    }

    reconcile_phase_after_disconnect(state, now);
    cleanup_disconnected_waiting_players(state);
    false
}

fn close_game_for_joiners(state: &mut HostState, message: &str) {
    push_event(state, message.to_string());
    push_network_log(&state.debug_log, message);

    let message = ServerMessage::Error {
        message: message.to_string(),
    };
    for client in &mut state.clients {
        if let Err(error) = write_server_message(&mut client.stream, &message) {
            server_eprintln!(
                "Failed to send close message to player {}: {error:#}",
                client.player_id.0
            );
        }
        let _ = client.stream.shutdown(Shutdown::Both);
    }
    state.clients.clear();

    for player in &mut state.players {
        if player.kind == PlayerKind::Human {
            player.connected = false;
            player.ready = false;
        }
    }
    for player in &mut state.race.players {
        player.connected = false;
    }
}

fn print_lobby_snapshot(players: &[LobbyPlayer]) {
    server_println!("Lobby:");
    for player in players {
        server_println!(
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
            if player.id == PlayerId(1) {
                " host"
            } else {
                ""
            }
        );
    }
}

fn broadcast_lobby_snapshot(state: &mut HostState) -> Result<()> {
    let snapshot = ServerMessage::LobbySnapshot {
        players: state.players.clone(),
        host_id: PlayerId(1),
        mod_config: (&state.active_mod_config).into(),
        events: state.events.clone(),
    };

    let mut failed_clients = Vec::new();
    for client in state.clients.iter_mut() {
        if let Err(error) = write_server_message(&mut client.stream, &snapshot) {
            server_eprintln!(
                "Failed to send lobby snapshot to player {}: {error:#}",
                client.player_id.0
            );
            failed_clients.push(client.player_id);
        }
    }

    state
        .clients
        .retain(|client| !failed_clients.contains(&client.player_id));
    Ok(())
}

fn start_countdown(state: Arc<Mutex<HostState>>) {
    let should_start = {
        let mut state = state.lock().expect("host state poisoned");
        match state.phase {
            NetworkRacePhase::WaitingForHost
            | NetworkRacePhase::Lobby
            | NetworkRacePhase::Finished => {
                if let Err(error) = reset_race_from_lobby(&mut state) {
                    server_eprintln!("Failed to prepare race: {error:#}");
                    push_network_log(
                        &state.debug_log,
                        format!("failed to prepare race: {error:#}"),
                    );
                    return;
                }
            }
            NetworkRacePhase::Countdown { .. } | NetworkRacePhase::Racing => {
                server_println!("Race has already started");
                return;
            }
        }

        if current_race_connected_player_count(&state) < 2 {
            server_println!("Cannot start: at least two ready connected racers are required");
            return;
        }

        state.phase = NetworkRacePhase::Countdown {
            remaining_seconds: 3,
        };
        push_event(&mut state, "Countdown started".to_string());
        push_network_log(&state.debug_log, "countdown started remaining=3");
        server_println!("Countdown: 3");
        if let Err(error) = broadcast_race_snapshot(&mut state) {
            server_eprintln!("Failed to broadcast race snapshot: {error:#}");
        }
        true
    };

    if should_start {
        thread::spawn(move || run_countdown(state));
    }
}

fn reset_race_from_lobby(state: &mut HostState) -> Result<()> {
    cleanup_disconnected_waiting_players(state);

    let word_count = state.race.track.len();
    let track = Track::generate(&state.word_list, word_count)
        .context("failed to generate rematch track")?;
    state.race = RaceState::new(track);
    let now = Instant::now();
    for player in state
        .players
        .iter()
        .filter(|player| player.connected && player.ready)
    {
        state.race.add_player(
            RacePlayerId(player.id.0),
            player.name.clone(),
            player.color.into(),
            now,
        );
    }
    reset_network_ai_timing(state, now);

    state.bonuses = BonusState::generate(&state.race.track, &state.word_list);
    state.bonus_attempts.clear();
    state.spent_bonus_gaps.clear();
    state.player_effects.clear();
    state.placements.clear();
    state.first_finished_at = None;
    state.race_results_sent = false;
    state.events.clear();
    state.phase = NetworkRacePhase::WaitingForHost;
    push_network_log(
        &state.debug_log,
        format!("prepared rematch racers={}", state.race.players.len()),
    );

    Ok(())
}

fn return_to_lobby(state: &mut HostState) -> Result<()> {
    let event = match state.phase {
        NetworkRacePhase::Countdown { .. } | NetworkRacePhase::Racing => "Race cancelled",
        NetworkRacePhase::Finished => "Returned to lobby",
        NetworkRacePhase::Lobby | NetworkRacePhase::WaitingForHost => return Ok(()),
    };
    reset_race_from_lobby(state)?;
    push_event(state, event.to_string());
    push_network_log(&state.debug_log, event.to_ascii_lowercase());
    if let Err(error) = broadcast_race_snapshot(state) {
        server_eprintln!("Failed to broadcast lobby race snapshot: {error:#}");
    }
    if let Err(error) = broadcast_lobby_snapshot(state) {
        server_eprintln!("Failed to broadcast lobby snapshot: {error:#}");
    }

    Ok(())
}

fn current_phase(state: &Arc<Mutex<HostState>>) -> NetworkRacePhase {
    state.lock().expect("host state poisoned").phase
}

fn apply_line_input(state: &Arc<Mutex<HostState>>, player_id: PlayerId, line: &str) {
    let now = std::time::Instant::now();
    let mut state = state.lock().expect("host state poisoned");
    if state.phase != NetworkRacePhase::Racing {
        return;
    }

    for ch in line.chars() {
        let action = if ch == ' ' {
            KeyAction::Space
        } else {
            KeyAction::Char(ch)
        };
        apply_network_key_input(&mut state, player_id, action, now);
    }
    apply_network_key_input(&mut state, player_id, KeyAction::Space, now);

    update_race_status(&mut state, now);
    if state.phase == NetworkRacePhase::Finished {
        if let Err(error) = broadcast_race_snapshot(&mut state) {
            server_eprintln!("Failed to broadcast race snapshot: {error:#}");
        }
        server_println!("Race finished");
        push_network_log(&state.debug_log, "race finished after host line input");
        if let Err(error) = broadcast_race_results_once(&mut state) {
            server_eprintln!("Failed to broadcast race results: {error:#}");
        }
    } else if let Err(error) = broadcast_race_delta(&mut state) {
        server_eprintln!("Failed to broadcast race delta: {error:#}");
    }
}

fn apply_network_key_input(
    state: &mut HostState,
    player_id: PlayerId,
    action: KeyAction,
    now: Instant,
) -> bool {
    if player_input_is_paused(state, player_id, now) {
        return true;
    }

    if state.bonus_attempts.contains_key(&player_id) {
        apply_network_bonus_typing_action(state, player_id, action, now);
        return true;
    }

    if let KeyAction::Char(ch) = action
        && let Some(attempt) = network_bonus_start(state, player_id, ch, now)
    {
        state.bonus_attempts.insert(player_id, attempt);
        apply_network_bonus_char(state, player_id, ch);
        return true;
    }

    state
        .race
        .apply_key_input(RacePlayerId(player_id.0), action, now)
        .is_some()
}

fn apply_network_track_key_input(
    state: &mut HostState,
    player_id: PlayerId,
    action: KeyAction,
    now: Instant,
) -> Option<Vec<TypingEvent>> {
    if player_input_is_paused(state, player_id, now) {
        return Some(Vec::new());
    }

    state
        .race
        .apply_key_input(RacePlayerId(player_id.0), action, now)
}

fn apply_network_bonus_typing_action(
    state: &mut HostState,
    player_id: PlayerId,
    action: KeyAction,
    now: Instant,
) {
    match action {
        KeyAction::Char(ch) => apply_network_bonus_char(state, player_id, ch),
        KeyAction::Backspace => {
            let Some(player) = state
                .race
                .players
                .iter_mut()
                .find(|player| player.id == RacePlayerId(player_id.0))
            else {
                state.bonus_attempts.remove(&player_id);
                return;
            };

            let previous_typo = player.state.typo_index;
            if player.state.input.pop().is_some() {
                player.state.stats.backspaces += 1;
            }

            let input_is_empty = player.state.input.is_empty();
            let input = player.state.input.clone();
            let _ = player;

            recalculate_network_bonus_typo(state, player_id, &input);

            let typo_cleared = previous_typo.is_some()
                && state
                    .race
                    .player(RacePlayerId(player_id.0))
                    .is_some_and(|player| player.state.typo_index.is_none());
            if typo_cleared {
                push_network_log(
                    &state.debug_log,
                    format!("{} bonus typo cleared", player_label(state, player_id)),
                );
            }
            if input_is_empty {
                state.bonus_attempts.remove(&player_id);
                push_network_log(
                    &state.debug_log,
                    format!("{} bonus attempt cancelled", player_label(state, player_id)),
                );
            }
        }
        KeyAction::Space => {
            if network_bonus_completed_without_typo(state, player_id) {
                claim_network_bonus(state, player_id, now);
            } else {
                apply_network_bonus_char(state, player_id, ' ');
            }
        }
    }
}

fn network_bonus_start(
    state: &HostState,
    player_id: PlayerId,
    ch: char,
    now: Instant,
) -> Option<NetworkBonusAttempt> {
    let player = state.race.player(RacePlayerId(player_id.0))?;
    if player.state.held_item.is_some()
        || player.state.has_active_shield(now)
        || player.state.has_active_star(now)
        || player.state.typo_index.is_some()
        || !player.state.input.is_empty()
        || player.state.is_finished()
    {
        return None;
    }

    let (point_index, point) = state.bonuses.point_for_gap(player.state.word_index)?;
    if state
        .spent_bonus_gaps
        .get(&player_id)
        .is_some_and(|after_word_index| *after_word_index == point.after_word_index)
    {
        return None;
    }

    point
        .available_choice_starting_with(ch, now)
        .map(|(choice_index, _)| NetworkBonusAttempt {
            point_index,
            choice_index,
        })
}

fn apply_network_bonus_char(state: &mut HostState, player_id: PlayerId, ch: char) {
    let Some(attempt) = state.bonus_attempts.get(&player_id).copied() else {
        return;
    };
    let Some(target) = network_bonus_target(state, attempt).map(str::to_owned) else {
        state.bonus_attempts.remove(&player_id);
        return;
    };
    let Some(player) = state
        .race
        .players
        .iter_mut()
        .find(|player| player.id == RacePlayerId(player_id.0))
    else {
        state.bonus_attempts.remove(&player_id);
        return;
    };

    let previous_typo = player.state.typo_index;
    let input_index = player.state.input.chars().count();
    let is_correct = previous_typo.is_none() && target.chars().nth(input_index) == Some(ch);

    player.state.stats.typed_chars += 1;
    if is_correct {
        player.state.stats.correct_chars += 1;
    } else {
        player.state.stats.typo_chars += 1;
    }

    player.state.input.push(ch);
    player.state.typo_index = first_typo_index(&player.state.input, &target);
    if previous_typo.is_none() && player.state.typo_index.is_some() {
        push_network_log(
            &state.debug_log,
            format!("{} bonus typo started", player_label(state, player_id)),
        );
    }
}

fn recalculate_network_bonus_typo(state: &mut HostState, player_id: PlayerId, input: &str) {
    let Some(attempt) = state.bonus_attempts.get(&player_id).copied() else {
        return;
    };
    let target = network_bonus_target(state, attempt).map(str::to_owned);
    let Some(player) = state
        .race
        .players
        .iter_mut()
        .find(|player| player.id == RacePlayerId(player_id.0))
    else {
        state.bonus_attempts.remove(&player_id);
        return;
    };

    player.state.typo_index = target
        .as_deref()
        .and_then(|target| first_typo_index(input, target));
}

fn network_bonus_completed_without_typo(state: &HostState, player_id: PlayerId) -> bool {
    let Some(attempt) = state.bonus_attempts.get(&player_id).copied() else {
        return false;
    };
    let Some(target) = network_bonus_target(state, attempt) else {
        return false;
    };
    let Some(player) = state.race.player(RacePlayerId(player_id.0)) else {
        return false;
    };

    player.state.typo_index.is_none() && player.state.input == target
}

fn claim_network_bonus(state: &mut HostState, player_id: PlayerId, now: Instant) {
    let Some(attempt) = state.bonus_attempts.remove(&player_id) else {
        return;
    };

    let after_word_index = state
        .bonuses
        .points
        .get(attempt.point_index)
        .map(|point| point.after_word_index);
    let item_context = network_item_roll_context(state, player_id, 5);
    let item_registry = state.item_registry.clone();
    let mut rng = thread_rng();
    let pickup = claim_bonus_choice(
        &mut state.bonuses,
        attempt.point_index,
        attempt.choice_index,
        now,
        item_context,
        &item_registry,
        &mut rng,
    );

    if let Some(player) = state
        .race
        .players
        .iter_mut()
        .find(|player| player.id == RacePlayerId(player_id.0))
    {
        player.state.input.clear();
        player.state.typo_index = None;
    }

    if let Some(after_word_index) = after_word_index {
        state.spent_bonus_gaps.insert(player_id, after_word_index);
    }

    let name = player_name(state, player_id).unwrap_or_else(|| format!("player {}", player_id.0));
    match pickup {
        Some(item) => {
            let item_name = item_pickup_name(item);
            push_event(state, format!("{name} got {item_name}"));
            push_network_log(
                &state.debug_log,
                format!("{name} picked up {item_name} from network bonus"),
            );
            activate_network_pickup(state, player_id, item, now);
        }
        None => {
            push_event(state, format!("{name} missed the bonus"));
            push_network_log(
                &state.debug_log,
                format!("{name} missed network bonus; choice was unavailable"),
            );
        }
    }
}

fn network_bonus_target(state: &HostState, attempt: NetworkBonusAttempt) -> Option<&str> {
    state
        .bonuses
        .points
        .get(attempt.point_index)?
        .choices
        .get(attempt.choice_index)
        .map(|choice| choice.word.as_str())
}

fn player_has_nearby_racer(
    state: &HostState,
    player_id: PlayerId,
    max_distance_words: usize,
) -> bool {
    let Some(player) = state.race.player(RacePlayerId(player_id.0)) else {
        return false;
    };

    state.race.players.iter().any(|other| {
        other.id != player.id
            && other.connected
            && !other.state.is_finished()
            && player.state.word_index.abs_diff(other.state.word_index) <= max_distance_words
    })
}

fn network_item_roll_context(
    state: &HostState,
    player_id: PlayerId,
    max_distance_words: usize,
) -> ItemRollContext {
    ItemRollContext {
        has_nearby_racer: player_has_nearby_racer(state, player_id, max_distance_words),
        position: network_position_band(state, player_id),
    }
}

fn network_position_band(state: &HostState, player_id: PlayerId) -> RacePositionBand {
    let active_racers = state
        .race
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

fn player_input_is_paused(state: &HostState, player_id: PlayerId, now: Instant) -> bool {
    if state
        .player_effects
        .get(&player_id)
        .and_then(|effects| effects.stunned_until)
        .is_some_and(|until| until > now)
    {
        return true;
    }

    state
        .race
        .player(RacePlayerId(player_id.0))
        .is_some_and(|player| player_has_active_mushroom_effect(player, now))
}

fn activate_network_pickup(
    state: &mut HostState,
    player_id: PlayerId,
    item: ItemPickup,
    now: Instant,
) {
    match item {
        ItemPickup::Held(HeldItem::Mushroom) => activate_network_mushroom(state, player_id, now),
        ItemPickup::Held(HeldItem::Banana) => activate_network_banana(state, player_id, now),
        ItemPickup::Held(HeldItem::Star) => activate_network_star(state, player_id, now),
        ItemPickup::Held(HeldItem::BlueShell) => activate_network_blue_shell(state, player_id, now),
        ItemPickup::Held(HeldItem::SquidInk) => activate_network_squid_ink(state, player_id, now),
        ItemPickup::Shield => activate_network_shield(state, player_id, now),
    }
}

fn activate_network_shield(state: &mut HostState, player_id: PlayerId, now: Instant) {
    let Some(player) = state
        .race
        .players
        .iter_mut()
        .find(|player| player.id == RacePlayerId(player_id.0))
    else {
        return;
    };

    player.state.active_effects.push(ActiveEffect::Shield {
        until: now + Duration::from_millis(state.item_registry.shield_effect().duration_ms),
    });
}

fn activate_network_mushroom(state: &mut HostState, player_id: PlayerId, now: Instant) {
    state.bonus_attempts.remove(&player_id);
    let Some(player) = state
        .race
        .players
        .iter_mut()
        .find(|player| player.id == RacePlayerId(player_id.0))
    else {
        return;
    };

    player.state.input.clear();
    player.state.typo_index = None;
    let mushroom = state.item_registry.mushroom_effect();
    player.state.active_effects.push(ActiveEffect::Mushroom {
        remaining_words: mushroom.boost_words,
        next_step_at: now,
        step_interval: mushroom_step_interval(mushroom.wpm),
    });
    advance_network_mushrooms(state, now);
}

fn activate_network_star(state: &mut HostState, player_id: PlayerId, now: Instant) {
    let Some(player) = state
        .race
        .players
        .iter_mut()
        .find(|player| player.id == RacePlayerId(player_id.0))
    else {
        return;
    };

    player.state.active_effects.push(ActiveEffect::Star {
        until: now + Duration::from_millis(state.item_registry.star_effect().duration_ms),
    });
}

fn activate_network_blue_shell(state: &mut HostState, player_id: PlayerId, now: Instant) {
    let attacker_name = player_label(state, player_id);
    let Some(target_id) = first_place_network_target(state, Some(player_id)) else {
        push_event(state, format!("{attacker_name} missed Blue Shell"));
        return;
    };

    let attacker_word_index = state
        .race
        .player(RacePlayerId(player_id.0))
        .map(|player| player.state.word_index)
        .unwrap_or_default();
    let target_word_index = state
        .race
        .player(RacePlayerId(target_id.0))
        .map(|player| player.state.word_index)
        .unwrap_or_default();
    let direction = attack_direction(attacker_word_index, target_word_index);
    state.player_effects.entry(player_id).or_default().item_cue = Some(NetworkItemCue {
        kind: NetworkItemCueKind::BlueShell { direction },
        ascii_label: network_blue_shell_cue_label(direction, false),
        unicode_label: network_blue_shell_cue_label(direction, true),
        placement: network_item_cue_placement(direction),
        until: now + Duration::from_millis(1_500),
    });

    let target_name = player_label(state, target_id);
    if apply_network_blue_shell_to_player(state, target_id, now) {
        push_event(
            state,
            format!("{attacker_name} hit {target_name} with Blue Shell"),
        );
    }
}

fn activate_network_banana(state: &mut HostState, player_id: PlayerId, now: Instant) {
    let Some(attacker) = state.race.player(RacePlayerId(player_id.0)) else {
        return;
    };
    let attacker_word_index = attacker.state.word_index;
    let attacker_name = attacker.name.clone();
    let candidates = state
        .race
        .players
        .iter()
        .filter(|player| player.id != attacker.id)
        .filter(|player| player.connected)
        .filter(|player| !player.state.is_finished())
        .filter(|player| !player_is_stunned(state, PlayerId(player.id.0), now))
        .map(|player| RacerPosition {
            id: player.id.0 as usize,
            word_index: player.state.word_index,
        })
        .collect::<Vec<_>>();

    push_network_log(
        &state.debug_log,
        format!(
            "{attacker_name} banana fired from word={attacker_word_index}; candidates={}",
            network_racer_positions_summary(state, &candidates, now)
        ),
    );

    let banana = state.item_registry.banana_effect();
    let Some(target) =
        select_nearest_banana_target(attacker_word_index, &candidates, banana.range_words)
    else {
        push_event(state, format!("{attacker_name} missed Banana"));
        push_network_log(&state.debug_log, format!("{attacker_name} banana missed"));
        return;
    };

    let target_id = PlayerId(target.id as u64);
    let direction = attack_direction(attacker_word_index, target.word_index);
    let banana_display = state.item_registry.banana_display();
    let (ascii_label, unicode_label) = network_banana_cue_labels(direction, banana_display);
    let distance = attacker_word_index.abs_diff(target.word_index);
    push_network_log(
        &state.debug_log,
        format!(
            "{attacker_name} banana target={} target_word={} direction={direction:?} distance_words={distance}",
            player_name(state, target_id).unwrap_or_else(|| format!("player {}", target_id.0)),
            target.word_index
        ),
    );
    state.player_effects.entry(player_id).or_default().item_cue = Some(NetworkItemCue {
        kind: NetworkItemCueKind::Banana { direction },
        ascii_label,
        unicode_label,
        placement: network_item_cue_placement(direction),
        until: now + Duration::from_millis(banana.cue_ms),
    });

    match apply_network_banana_to_player(state, target_id, now) {
        Some(BananaResolution::SpunOut) => {
            let target_name =
                player_name(state, target_id).unwrap_or_else(|| format!("player {}", target_id.0));
            push_event(state, format!("{attacker_name} hit {target_name}"));
        }
        Some(BananaResolution::Blocked) | None => {}
    }
}

fn activate_network_squid_ink(state: &mut HostState, player_id: PlayerId, now: Instant) {
    let Some(attacker) = state.race.player(RacePlayerId(player_id.0)) else {
        return;
    };
    let attacker_word_index = attacker.state.word_index;
    let attacker_name = attacker.name.clone();
    let squid_ink = state.item_registry.squid_ink_effect();
    let targets = state
        .race
        .players
        .iter()
        .filter(|player| player.id != attacker.id)
        .filter(|player| player.connected)
        .filter(|player| !player.state.is_finished())
        .filter(|player| {
            attacker_word_index.abs_diff(player.state.word_index) <= squid_ink.range_words
        })
        .map(|player| RacerPosition {
            id: player.id.0 as usize,
            word_index: player.state.word_index,
        })
        .collect::<Vec<_>>();

    push_network_log(
        &state.debug_log,
        format!(
            "{attacker_name} squid ink fired from word={attacker_word_index}; candidates={}",
            network_racer_positions_summary(state, &targets, now)
        ),
    );

    state.player_effects.entry(player_id).or_default().item_cue = Some(NetworkItemCue {
        kind: NetworkItemCueKind::SquidInk,
        ascii_label: " ink ".to_string(),
        unicode_label: " 🦑 ".to_string(),
        placement: ItemCuePlacementSnapshot::After,
        until: now + Duration::from_millis(squid_ink.cue_ms),
    });

    let mut hit_count = 0;
    for target in targets {
        if apply_network_squid_ink_to_player(state, PlayerId(target.id as u64), now) {
            hit_count += 1;
        }
    }

    if hit_count == 0 {
        push_event(state, format!("{attacker_name} missed Squid Ink"));
        push_network_log(
            &state.debug_log,
            format!("{attacker_name} squid ink missed"),
        );
    } else {
        push_event(state, format!("{attacker_name} inked {hit_count} racer(s)"));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BananaResolution {
    SpunOut,
    Blocked,
}

fn apply_network_banana_to_player(
    state: &mut HostState,
    target_id: PlayerId,
    now: Instant,
) -> Option<BananaResolution> {
    let target_index = state
        .race
        .players
        .iter()
        .position(|player| player.id == RacePlayerId(target_id.0))?;
    let target_name = state.race.players[target_index].name.clone();
    let word_index = state.race.players[target_index].state.word_index;

    if state.race.players[target_index]
        .state
        .has_active_shield(now)
    {
        state.race.players[target_index]
            .state
            .active_effects
            .retain(|effect| !matches!(effect, ActiveEffect::Shield { .. }));
        state
            .player_effects
            .entry(target_id)
            .or_default()
            .impact_cue = Some(NetworkImpactCue {
            kind: ImpactCueSnapshotKind::ShieldBlock,
            until: now + Duration::from_millis(700),
        });
        push_event(state, format!("{target_name} blocked Banana"));
        push_network_log(
            &state.debug_log,
            format!("{target_name} blocked Banana at word={word_index}; shield consumed"),
        );
        return Some(BananaResolution::Blocked);
    }

    state.bonus_attempts.remove(&target_id);
    let target = &mut state.race.players[target_index];
    target.state.input.clear();
    target.state.typo_index = None;
    if let Some(ai) = state.ai_racers.get_mut(&target_id) {
        ai.char_budget = 0.0;
    }
    let effects = state.player_effects.entry(target_id).or_default();
    let banana = state.item_registry.banana_effect();
    effects.stunned_until = Some(now + Duration::from_millis(banana.stun_ms));
    effects.impact_cue = Some(NetworkImpactCue {
        kind: ImpactCueSnapshotKind::Banana,
        until: now + Duration::from_millis(banana.impact_blink_ms),
    });
    push_network_log(
        &state.debug_log,
        format!(
            "{target_name} spun out at word={word_index}; stun_ms={} impact_blink_ms={}",
            banana.stun_ms, banana.impact_blink_ms
        ),
    );
    Some(BananaResolution::SpunOut)
}

fn first_place_network_target(state: &HostState, exclude: Option<PlayerId>) -> Option<PlayerId> {
    state
        .race
        .players
        .iter()
        .filter(|player| Some(PlayerId(player.id.0)) != exclude)
        .filter(|player| player.connected)
        .filter(|player| !player.state.is_finished())
        .max_by_key(|player| (player.state.word_index, player.state.input.chars().count()))
        .map(|player| PlayerId(player.id.0))
}

fn apply_network_blue_shell_to_player(
    state: &mut HostState,
    target_id: PlayerId,
    now: Instant,
) -> bool {
    let Some(target_index) = state
        .race
        .players
        .iter()
        .position(|player| player.id == RacePlayerId(target_id.0))
    else {
        return false;
    };
    let target_name = state.race.players[target_index].name.clone();

    if state.race.players[target_index]
        .state
        .has_active_shield(now)
    {
        state.race.players[target_index]
            .state
            .active_effects
            .retain(|effect| !matches!(effect, ActiveEffect::Shield { .. }));
        state
            .player_effects
            .entry(target_id)
            .or_default()
            .impact_cue = Some(NetworkImpactCue {
            kind: ImpactCueSnapshotKind::ShieldBlock,
            until: now + Duration::from_millis(700),
        });
        push_event(state, format!("{target_name} blocked Blue Shell"));
        push_network_log(
            &state.debug_log,
            format!("{target_name} blocked Blue Shell; shield consumed"),
        );
        return false;
    }

    let affected_words = state.item_registry.blue_shell_effect().affected_words;
    let target = &mut state.race.players[target_index].state;
    let mut applied = false;
    for word_index in target.word_index..target.word_index.saturating_add(affected_words) {
        let Some(word) = state.race.track.current_word(word_index) else {
            break;
        };
        target
            .word_overrides
            .insert(word_index, word.chars().rev().collect());
        applied = true;
    }
    if applied {
        target.input.clear();
        target.typo_index = None;
        state.bonus_attempts.remove(&target_id);
        state
            .player_effects
            .entry(target_id)
            .or_default()
            .impact_cue = Some(NetworkImpactCue {
            kind: ImpactCueSnapshotKind::BlueShell,
            until: now + Duration::from_millis(1_200),
        });
    }
    applied
}

fn apply_network_squid_ink_to_player(
    state: &mut HostState,
    target_id: PlayerId,
    now: Instant,
) -> bool {
    let Some(target_index) = state
        .race
        .players
        .iter()
        .position(|player| player.id == RacePlayerId(target_id.0))
    else {
        return false;
    };
    let target_name = state.race.players[target_index].name.clone();

    if state.race.players[target_index]
        .state
        .has_active_shield(now)
    {
        state.race.players[target_index]
            .state
            .active_effects
            .retain(|effect| !matches!(effect, ActiveEffect::Shield { .. }));
        state
            .player_effects
            .entry(target_id)
            .or_default()
            .impact_cue = Some(NetworkImpactCue {
            kind: ImpactCueSnapshotKind::ShieldBlock,
            until: now + Duration::from_millis(700),
        });
        push_event(state, format!("{target_name} blocked Squid Ink"));
        push_network_log(
            &state.debug_log,
            format!("{target_name} blocked Squid Ink; shield consumed"),
        );
        return false;
    }

    let target = &mut state.race.players[target_index].state;
    target.inked_word_index = Some(target.word_index);
    state
        .player_effects
        .entry(target_id)
        .or_default()
        .impact_cue = Some(NetworkImpactCue {
        kind: ImpactCueSnapshotKind::SquidInk,
        until: now + Duration::from_millis(state.item_registry.squid_ink_effect().impact_blink_ms),
    });
    true
}

fn advance_network_mushrooms(state: &mut HostState, now: Instant) {
    let player_ids = state
        .race
        .players
        .iter()
        .map(|player| PlayerId(player.id.0))
        .collect::<Vec<_>>();

    for player_id in player_ids {
        loop {
            if !advance_network_mushroom_one_word(state, player_id, now) {
                break;
            }
            if state
                .race
                .player(RacePlayerId(player_id.0))
                .is_some_and(|player| player.state.is_finished())
            {
                break;
            }
        }
    }
}

fn advance_network_mushroom_one_word(
    state: &mut HostState,
    player_id: PlayerId,
    now: Instant,
) -> bool {
    let Some(player_index) = state
        .race
        .players
        .iter()
        .position(|player| player.id == RacePlayerId(player_id.0))
    else {
        return false;
    };
    let Some(effect_index) = state.race.players[player_index]
        .state
        .active_effects
        .iter()
        .position(|effect| {
            matches!(
                effect,
                ActiveEffect::Mushroom {
                    remaining_words,
                    next_step_at,
                    ..
                } if *remaining_words > 0 && *next_step_at <= now
            )
        })
    else {
        return false;
    };

    let remaining = state
        .race
        .track
        .len()
        .saturating_sub(state.race.players[player_index].state.word_index);
    if remaining == 0 {
        state.race.players[player_index]
            .state
            .active_effects
            .remove(effect_index);
        return false;
    }

    let player = &mut state.race.players[player_index];
    player.state.word_index += 1;
    player.state.stats.completed_words += 1;
    player.state.input.clear();
    player.state.typo_index = None;
    state.bonus_attempts.remove(&player_id);

    if player.state.word_index >= state.race.track.len() {
        player.state.finished_at = Some(now);
        player.state.active_effects.remove(effect_index);
        return false;
    }

    if let Some(ActiveEffect::Mushroom {
        remaining_words,
        next_step_at,
        step_interval,
    }) = player.state.active_effects.get_mut(effect_index)
    {
        *remaining_words -= 1;
        if *remaining_words == 0 {
            player.state.active_effects.remove(effect_index);
        } else {
            *next_step_at += *step_interval;
        }
    }

    true
}

fn advance_network_ai_racers(state: &mut HostState, now: Instant) {
    if state.phase != NetworkRacePhase::Racing {
        reset_network_ai_timing(state, now);
        return;
    }

    let player_ids = state.ai_racers.keys().copied().collect::<Vec<_>>();
    for player_id in player_ids {
        network_ai_try_claim_bonus(state, player_id, now);
        advance_network_ai_typing(state, player_id, now);
    }
}

fn reset_network_ai_timing(state: &mut HostState, now: Instant) {
    for ai in state.ai_racers.values_mut() {
        ai.char_budget = 0.0;
        ai.last_update = now;
    }
}

fn network_ai_try_claim_bonus(state: &mut HostState, player_id: PlayerId, now: Instant) {
    let Some(player) = state.race.player(RacePlayerId(player_id.0)) else {
        return;
    };
    if player.state.held_item.is_some()
        || player.state.has_active_shield(now)
        || player.state.has_active_star(now)
        || player_has_active_mushroom_effect(player, now)
        || player_is_stunned(state, player_id, now)
        || state
            .player_effects
            .get(&player_id)
            .and_then(|effects| effects.item_cue.as_ref())
            .is_some_and(|cue| cue.until > now)
        || player.state.typo_index.is_some()
        || !player.state.input.is_empty()
        || player.state.is_finished()
        || state.bonus_attempts.contains_key(&player_id)
    {
        return;
    }

    let Some((point_index, point)) = state.bonuses.point_for_gap(player.state.word_index) else {
        return;
    };
    if state
        .spent_bonus_gaps
        .get(&player_id)
        .is_some_and(|after_word_index| *after_word_index == point.after_word_index)
    {
        return;
    }

    let available_choices = point
        .choices
        .iter()
        .enumerate()
        .filter(|(_, choice)| choice.is_available(now))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    if available_choices.is_empty() {
        return;
    }

    let mut rng = thread_rng();
    let choice_index = available_choices[rng.gen_range(0..available_choices.len())];
    state.bonus_attempts.insert(
        player_id,
        NetworkBonusAttempt {
            point_index,
            choice_index,
        },
    );
    claim_network_bonus(state, player_id, now);
}

fn advance_network_ai_typing(state: &mut HostState, player_id: PlayerId, now: Instant) {
    let paused = state
        .race
        .player(RacePlayerId(player_id.0))
        .is_none_or(|player| player.state.is_finished())
        || player_input_is_paused(state, player_id, now);

    let Some(ai) = state.ai_racers.get_mut(&player_id) else {
        return;
    };
    let elapsed = now.saturating_duration_since(ai.last_update);
    ai.last_update = now;

    if paused {
        return;
    }

    ai.char_budget += elapsed.as_secs_f64() * ai_chars_per_second(ai.words_per_minute);

    while state
        .ai_racers
        .get(&player_id)
        .is_some_and(|ai| ai.char_budget >= 1.0)
    {
        let Some(action) = next_network_ai_key(state, player_id) else {
            break;
        };
        let events =
            apply_network_track_key_input(state, player_id, action, now).unwrap_or_default();
        if let Some(ai) = state.ai_racers.get_mut(&player_id) {
            ai.char_budget -= 1.0;
        }

        if events
            .iter()
            .any(|event| matches!(event, TypingEvent::RaceFinished))
        {
            push_event(
                state,
                format!("{} finished", player_label(state, player_id)),
            );
            break;
        }
    }
}

fn next_network_ai_key(state: &HostState, player_id: PlayerId) -> Option<KeyAction> {
    let player = state.race.player(RacePlayerId(player_id.0))?;
    let target = player
        .state
        .word_override(player.state.word_index)
        .or_else(|| state.race.track.current_word(player.state.word_index))?;
    if player.state.input == target {
        return Some(KeyAction::Space);
    }

    target
        .chars()
        .nth(player.state.input.chars().count())
        .map(KeyAction::Char)
}

fn ai_chars_per_second(words_per_minute: f64) -> f64 {
    words_per_minute * 5.0 / 60.0
}

fn player_has_active_mushroom_effect(
    player: &crate::game::race::RacePlayer,
    _now: Instant,
) -> bool {
    player.state.active_effects.iter().any(|effect| {
        matches!(
            effect,
            ActiveEffect::Mushroom {
                remaining_words,
                ..
            } if *remaining_words > 0
        )
    })
}

fn player_is_stunned(state: &HostState, player_id: PlayerId, now: Instant) -> bool {
    state
        .player_effects
        .get(&player_id)
        .and_then(|effects| effects.stunned_until)
        .is_some_and(|until| until > now)
}

fn attack_direction(
    attacker_word_index: usize,
    target_word_index: usize,
) -> AttackDirectionSnapshot {
    match target_word_index.cmp(&attacker_word_index) {
        std::cmp::Ordering::Greater => AttackDirectionSnapshot::Ahead,
        std::cmp::Ordering::Less => AttackDirectionSnapshot::Behind,
        std::cmp::Ordering::Equal => AttackDirectionSnapshot::Overlap,
    }
}

fn mushroom_step_interval(wpm: u32) -> Duration {
    Duration::from_secs_f64(60.0 / f64::from(wpm))
}

fn network_banana_cue_labels(
    direction: AttackDirectionSnapshot,
    display: BananaDisplayConfig,
) -> (String, String) {
    match direction {
        AttackDirectionSnapshot::Ahead => (display.ascii_ahead, display.unicode_ahead),
        AttackDirectionSnapshot::Behind => (display.ascii_behind, display.unicode_behind),
        AttackDirectionSnapshot::Overlap => (display.ascii_overlap, display.unicode_overlap),
    }
}

fn network_blue_shell_cue_label(direction: AttackDirectionSnapshot, unicode: bool) -> String {
    match (direction, unicode) {
        (AttackDirectionSnapshot::Ahead, false) => " sh>>".to_string(),
        (AttackDirectionSnapshot::Behind, false) => "<<sh ".to_string(),
        (AttackDirectionSnapshot::Overlap, false) => " sh<>".to_string(),
        (AttackDirectionSnapshot::Ahead, true) => " 🐢 >>".to_string(),
        (AttackDirectionSnapshot::Behind, true) => "<< 🐢 ".to_string(),
        (AttackDirectionSnapshot::Overlap, true) => " 🐢 <>".to_string(),
    }
}

fn network_item_cue_placement(direction: AttackDirectionSnapshot) -> ItemCuePlacementSnapshot {
    match direction {
        AttackDirectionSnapshot::Ahead | AttackDirectionSnapshot::Overlap => {
            ItemCuePlacementSnapshot::After
        }
        AttackDirectionSnapshot::Behind => ItemCuePlacementSnapshot::Before,
    }
}

fn network_racer_positions_summary(
    state: &HostState,
    racers: &[RacerPosition],
    now: Instant,
) -> String {
    if racers.is_empty() {
        return "none".to_string();
    }

    racers
        .iter()
        .map(|racer| {
            let player_id = PlayerId(racer.id as u64);
            let shield = state
                .race
                .player(RacePlayerId(player_id.0))
                .is_some_and(|player| player.state.has_active_shield(now));
            let stunned = player_is_stunned(state, player_id, now);
            let finished = state
                .race
                .player(RacePlayerId(player_id.0))
                .is_some_and(|player| player.state.is_finished());
            format!(
                "{}@{} shield={shield} stunned={stunned} finished={finished}",
                player_name(state, player_id).unwrap_or_else(|| format!("player {}", player_id.0)),
                racer.word_index
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn item_pickup_name(item: ItemPickup) -> &'static str {
    match item {
        ItemPickup::Held(held_item) => held_item.name(),
        ItemPickup::Shield => "Shield",
    }
}

fn player_label(state: &HostState, player_id: PlayerId) -> String {
    player_name(state, player_id).unwrap_or_else(|| format!("player {}", player_id.0))
}

fn run_countdown(state: Arc<Mutex<HostState>>) {
    for remaining_seconds in [2, 1] {
        thread::sleep(Duration::from_secs(1));
        let mut guard = state.lock().expect("host state poisoned");
        if !matches!(guard.phase, NetworkRacePhase::Countdown { .. }) {
            push_network_log(&guard.debug_log, "countdown stopped before next tick");
            return;
        }
        if !countdown_has_enough_connected_racers(&guard) {
            cancel_countdown(&mut guard);
            if let Err(error) = broadcast_race_snapshot(&mut guard) {
                server_eprintln!("Failed to broadcast race snapshot: {error:#}");
            }
            if let Err(error) = broadcast_lobby_snapshot(&mut guard) {
                server_eprintln!("Failed to broadcast lobby snapshot: {error:#}");
            }
            return;
        }

        guard.phase = NetworkRacePhase::Countdown { remaining_seconds };
        push_network_log(
            &guard.debug_log,
            format!("countdown tick remaining={remaining_seconds}"),
        );
        server_println!("Countdown: {remaining_seconds}");
        if let Err(error) = broadcast_race_snapshot(&mut guard) {
            server_eprintln!("Failed to broadcast race snapshot: {error:#}");
        }
    }

    thread::sleep(Duration::from_secs(1));
    let mut guard = state.lock().expect("host state poisoned");
    if !matches!(guard.phase, NetworkRacePhase::Countdown { .. }) {
        push_network_log(&guard.debug_log, "countdown stopped before race start");
        return;
    }
    if !countdown_has_enough_connected_racers(&guard) {
        cancel_countdown(&mut guard);
        if let Err(error) = broadcast_race_snapshot(&mut guard) {
            server_eprintln!("Failed to broadcast race snapshot: {error:#}");
        }
        if let Err(error) = broadcast_lobby_snapshot(&mut guard) {
            server_eprintln!("Failed to broadcast lobby snapshot: {error:#}");
        }
        return;
    }

    guard.phase = NetworkRacePhase::Racing;
    reset_network_ai_timing(&mut guard, Instant::now());
    push_event(&mut guard, "Race started".to_string());
    push_network_log(&guard.debug_log, "race started");
    server_println!("Race started");
    if let Err(error) = broadcast_race_snapshot(&mut guard) {
        server_eprintln!("Failed to broadcast race snapshot: {error:#}");
    }
    drop(guard);
    spawn_race_snapshot_loop(state);
}

fn spawn_race_snapshot_loop(state: Arc<Mutex<HostState>>) {
    thread::spawn(move || {
        loop {
            thread::sleep(RACE_SNAPSHOT_INTERVAL);
            let mut state = state.lock().expect("host state poisoned");
            if state.phase != NetworkRacePhase::Racing {
                break;
            }

            let now = Instant::now();
            advance_network_mushrooms(&mut state, now);
            advance_network_ai_racers(&mut state, now);
            update_race_status(&mut state, now);
            let expired_choices = expire_bonus_cooldowns(&mut state, now);
            if expired_choices > 0 {
                push_network_log(
                    &state.debug_log,
                    format!("bonus refreshed choices={expired_choices}"),
                );
            }

            if state.phase == NetworkRacePhase::Finished {
                if let Err(error) = broadcast_race_snapshot(&mut state) {
                    server_eprintln!("Failed to broadcast race snapshot: {error:#}");
                }
                server_println!("Race finished");
                push_network_log(&state.debug_log, "race finished on snapshot tick");
                if let Err(error) = broadcast_race_results_once(&mut state) {
                    server_eprintln!("Failed to broadcast race results: {error:#}");
                }
                break;
            } else if let Err(error) = broadcast_race_delta(&mut state) {
                server_eprintln!("Failed to broadcast race delta: {error:#}");
            }
        }
    });
}

fn reconcile_phase_after_disconnect(state: &mut HostState, now: Instant) {
    match state.phase {
        NetworkRacePhase::Countdown { .. } => {
            if !countdown_has_enough_connected_racers(state) {
                cancel_countdown(state);
            }
        }
        NetworkRacePhase::Racing => update_race_status(state, now),
        NetworkRacePhase::Finished => {}
        NetworkRacePhase::Lobby | NetworkRacePhase::WaitingForHost => {}
    }
}

fn countdown_has_enough_connected_racers(state: &HostState) -> bool {
    state
        .race
        .players
        .iter()
        .filter(|player| player.connected && !player.state.is_finished())
        .count()
        >= 2
}

fn cancel_countdown(state: &mut HostState) {
    state.phase = NetworkRacePhase::WaitingForHost;
    push_event(state, "Countdown cancelled".to_string());
    push_network_log(
        &state.debug_log,
        "countdown cancelled fewer than two connected racers",
    );
    server_println!("Countdown cancelled");
}

fn cleanup_disconnected_waiting_players(state: &mut HostState) {
    if !matches!(
        state.phase,
        NetworkRacePhase::Lobby | NetworkRacePhase::WaitingForHost | NetworkRacePhase::Finished
    ) {
        return;
    }

    let disconnected_ids = state
        .players
        .iter()
        .filter(|player| !player.connected)
        .map(|player| player.id)
        .collect::<Vec<_>>();
    if disconnected_ids.is_empty() {
        return;
    }

    state
        .players
        .retain(|player| !disconnected_ids.contains(&player.id));
    state
        .race
        .players
        .retain(|player| !disconnected_ids.contains(&PlayerId(player.id.0)));
    push_network_log(
        &state.debug_log,
        format!("cleaned up disconnected waiting players={disconnected_ids:?}"),
    );
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

fn broadcast_race_snapshot(state: &mut HostState) -> Result<()> {
    let snapshot = ServerMessage::RaceSnapshot(build_race_snapshot(state));
    log_race_snapshot(state);
    broadcast_server_message_to_clients(state, &snapshot)
}

fn broadcast_race_delta(state: &mut HostState) -> Result<()> {
    let delta = ServerMessage::RaceDelta(build_race_delta_snapshot(state));
    log_race_delta(state);
    broadcast_server_message_to_clients(state, &delta)
}

fn broadcast_server_message_to_clients(
    state: &mut HostState,
    message: &ServerMessage,
) -> Result<()> {
    let mut failed_clients = Vec::new();
    for client in state.clients.iter_mut() {
        if let Err(error) = write_server_message(&mut client.stream, message) {
            server_eprintln!(
                "Failed to send server message to player {}: {error:#}",
                client.player_id.0
            );
            failed_clients.push(client.player_id);
        }
    }

    state
        .clients
        .retain(|client| !failed_clients.contains(&client.player_id));
    Ok(())
}

fn log_race_snapshot(state: &HostState) {
    match state.phase {
        NetworkRacePhase::Countdown { remaining_seconds } => push_network_log(
            &state.debug_log,
            format!(
                "broadcast snapshot seq={} phase=countdown remaining={remaining_seconds}",
                state.snapshot_sequence
            ),
        ),
        NetworkRacePhase::Racing if state.snapshot_sequence.is_multiple_of(20) => push_network_log(
            &state.debug_log,
            format!(
                "broadcast snapshot seq={} phase=racing",
                state.snapshot_sequence
            ),
        ),
        NetworkRacePhase::Finished => push_network_log(
            &state.debug_log,
            format!(
                "broadcast snapshot seq={} phase=finished",
                state.snapshot_sequence
            ),
        ),
        _ => {}
    }
}

fn log_race_delta(state: &HostState) {
    match state.phase {
        NetworkRacePhase::Racing if state.snapshot_sequence.is_multiple_of(20) => push_network_log(
            &state.debug_log,
            format!(
                "broadcast delta seq={} phase=racing",
                state.snapshot_sequence
            ),
        ),
        NetworkRacePhase::Finished => push_network_log(
            &state.debug_log,
            format!(
                "broadcast delta seq={} phase=finished",
                state.snapshot_sequence
            ),
        ),
        _ => {}
    }
}

fn broadcast_race_results(state: &mut HostState) -> Result<()> {
    let rows = build_race_result_rows(state, Instant::now());
    let row_count = rows.len();
    let results = ServerMessage::RaceResults {
        placements: state.placements.clone(),
        rows,
    };
    push_network_log(
        &state.debug_log,
        format!(
            "broadcast race results placements={:?} rows={}",
            state.placements, row_count
        ),
    );

    let mut failed_clients = Vec::new();
    for client in state.clients.iter_mut() {
        if let Err(error) = write_server_message(&mut client.stream, &results) {
            server_eprintln!(
                "Failed to send race results to player {}: {error:#}",
                client.player_id.0
            );
            failed_clients.push(client.player_id);
        }
    }

    state
        .clients
        .retain(|client| !failed_clients.contains(&client.player_id));
    Ok(())
}

#[cfg(test)]
fn client_is_in_current_race(race: &RaceState, player_id: PlayerId) -> bool {
    race.players
        .iter()
        .any(|player| player.id == RacePlayerId(player_id.0))
}

fn build_race_result_rows(state: &HostState, now: Instant) -> Vec<RaceResultRow> {
    let mut ordered_ids = state.placements.clone();
    let mut remaining = state
        .race
        .players
        .iter()
        .map(|player| {
            (
                PlayerId(player.id.0),
                player.connected,
                player.state.word_index,
                player.state.input.chars().count(),
            )
        })
        .filter(|(id, _, _, _)| !ordered_ids.contains(id))
        .collect::<Vec<_>>();

    remaining.sort_by_key(|(_, connected, word_index, input_len)| {
        (
            // Active racers should appear before disconnected racers if the
            // server ever needs to synthesize rows before placement completion.
            !*connected,
            std::cmp::Reverse(*word_index),
            std::cmp::Reverse(*input_len),
        )
    });
    ordered_ids.extend(remaining.into_iter().map(|(id, _, _, _)| id));

    let track_words = state.race.track.len();
    ordered_ids
        .into_iter()
        .enumerate()
        .filter_map(|(index, id)| {
            let player = state
                .race
                .players
                .iter()
                .find(|player| player.id == RacePlayerId(id.0))?;
            let finished = player.state.is_finished();
            let status = if finished {
                RaceResultStatus::Finished
            } else if player.connected {
                RaceResultStatus::TimedOut
            } else {
                RaceResultStatus::Disconnected
            };
            let stats_until = player.state.finished_at.unwrap_or(now);
            let wpm = player
                .state
                .stats
                .words_per_minute(player.state.started_at, stats_until)
                .round()
                .clamp(0.0, u32::MAX as f64) as u32;
            let accuracy_percent = player.state.stats.accuracy().round().clamp(0.0, 100.0) as u32;
            let progress_words = if finished {
                track_words
            } else {
                player.state.word_index.min(track_words)
            };

            Some(RaceResultRow {
                placement: index + 1,
                player_id: id,
                name: player.name.clone(),
                color: player.color.into(),
                status,
                progress_words,
                track_words,
                wpm,
                accuracy_percent,
                typo_chars: player.state.stats.typo_chars,
                backspaces: player.state.stats.backspaces,
            })
        })
        .collect()
}

fn broadcast_race_results_once(state: &mut HostState) -> Result<()> {
    if state.race_results_sent {
        push_network_log(&state.debug_log, "skipped duplicate race results broadcast");
        return Ok(());
    }

    broadcast_race_results(state)?;
    state.race_results_sent = true;
    Ok(())
}

fn update_race_status(state: &mut HostState, now: Instant) {
    if state.phase != NetworkRacePhase::Racing {
        return;
    }

    let finished_ids = state
        .race
        .players
        .iter()
        .filter(|player| player.connected && player.state.is_finished())
        .map(|player| PlayerId(player.id.0))
        .collect::<Vec<_>>();

    for id in finished_ids {
        if !state.placements.contains(&id) {
            state.placements.push(id);
            let name = player_name(state, id).unwrap_or_else(|| format!("player {}", id.0));
            push_event(
                state,
                format!("{}. {name} finished", state.placements.len()),
            );
            push_network_log(
                &state.debug_log,
                format!("{}. {name} finished", state.placements.len()),
            );
        }
    }

    if state.first_finished_at.is_none() && !state.placements.is_empty() {
        state.first_finished_at = Some(now);
    }

    let connected_racers = state
        .race
        .players
        .iter()
        .filter(|player| player.connected)
        .count();
    let connected_finished = state
        .race
        .players
        .iter()
        .filter(|player| player.connected && player.state.is_finished())
        .count();
    let all_connected_finished = connected_racers > 0 && connected_finished == connected_racers;
    let all_connected_disconnected = connected_racers == 0;
    let timeout_expired = state.first_finished_at.is_some_and(|first_finished_at| {
        now.duration_since(first_finished_at) >= POST_FIRST_FINISH_TIMEOUT
    });

    if all_connected_finished || all_connected_disconnected || timeout_expired {
        append_unfinished_connected_placements(state);
        state.phase = NetworkRacePhase::Finished;
        push_event(state, "Race finished".to_string());
        push_network_log(
            &state.debug_log,
            format!(
                "race finished all_connected_finished={all_connected_finished} all_connected_disconnected={all_connected_disconnected} timeout_expired={timeout_expired}"
            ),
        );
    }
}

fn append_unfinished_connected_placements(state: &mut HostState) {
    let mut remaining = state
        .race
        .players
        .iter()
        .filter(|player| player.connected)
        .map(|player| {
            (
                PlayerId(player.id.0),
                player.state.word_index,
                player.state.input.chars().count(),
            )
        })
        .filter(|(id, _, _)| !state.placements.contains(id))
        .collect::<Vec<_>>();

    remaining.sort_by_key(|(_, word_index, input_len)| {
        (
            std::cmp::Reverse(*word_index),
            std::cmp::Reverse(*input_len),
        )
    });

    state
        .placements
        .extend(remaining.into_iter().map(|(id, _, _)| id));
}

fn player_name(state: &HostState, id: PlayerId) -> Option<String> {
    state
        .race
        .players
        .iter()
        .find(|player| player.id == RacePlayerId(id.0))
        .map(|player| player.name.clone())
}

fn build_race_snapshot(state: &mut HostState) -> RaceSnapshot {
    let now = Instant::now();
    expire_bonus_cooldowns(state, now);

    state.snapshot_sequence += 1;
    RaceSnapshot {
        sequence: state.snapshot_sequence,
        phase: state.phase,
        mod_config: (&state.active_mod_config).into(),
        track_words: state.race.track.words.clone(),
        bonuses: build_bonus_snapshots(&state.bonuses, now),
        players: build_player_snapshots(state, now),
        events: state.events.clone(),
    }
}

fn build_race_delta_snapshot(state: &mut HostState) -> RaceDeltaSnapshot {
    let now = Instant::now();
    expire_bonus_cooldowns(state, now);

    state.snapshot_sequence += 1;
    RaceDeltaSnapshot {
        sequence: state.snapshot_sequence,
        phase: state.phase,
        bonuses: build_bonus_snapshots(&state.bonuses, now),
        players: build_player_snapshots(state, now),
        events: state.events.clone(),
    }
}

fn build_player_snapshots(state: &HostState, now: Instant) -> Vec<PlayerSnapshot> {
    state
        .race
        .players
        .iter()
        .map(|player| {
            let player_id = PlayerId(player.id.0);
            let effects = state
                .player_effects
                .get(&player_id)
                .cloned()
                .unwrap_or_default();
            PlayerSnapshot {
                id: player_id,
                name: player.name.clone(),
                kind: player_kind(state, player_id),
                color: player.color.into(),
                word_index: player.state.word_index,
                input: player.state.input.clone(),
                typo_index: player.state.typo_index,
                word_overrides: player
                    .state
                    .word_overrides
                    .iter()
                    .map(|(word_index, word)| WordOverrideSnapshot {
                        word_index: *word_index,
                        word: word.clone(),
                    })
                    .collect(),
                finished: player.state.is_finished(),
                connected: player.connected,
                shielded: player.state.has_active_shield(now),
                starred: player.state.has_active_star(now),
                inked: player.state.is_inked(),
                boosted: player_has_active_mushroom_effect(player, now),
                stunned: effects.stunned_until.is_some_and(|until| until > now),
                impact_remaining_ms: remaining_ms(effects.impact_cue.map(|cue| cue.until), now),
                impact_cue: build_impact_cue_snapshot(effects.impact_cue, now),
                item_cue: build_item_cue_snapshot(effects.item_cue.clone(), now),
            }
        })
        .collect()
}

fn remaining_ms(until: Option<Instant>, now: Instant) -> u64 {
    until
        .filter(|until| *until > now)
        .map(|until| until.saturating_duration_since(now).as_millis() as u64)
        .unwrap_or(0)
}

fn build_item_cue_snapshot(cue: Option<NetworkItemCue>, now: Instant) -> Option<ItemCueSnapshot> {
    let cue = cue.filter(|cue| cue.until > now)?;
    Some(ItemCueSnapshot {
        kind: match cue.kind {
            NetworkItemCueKind::Banana { direction } => ItemCueSnapshotKind::Banana { direction },
            NetworkItemCueKind::BlueShell { direction } => {
                ItemCueSnapshotKind::BlueShell { direction }
            }
            NetworkItemCueKind::SquidInk => ItemCueSnapshotKind::SquidInk,
        },
        ascii_label: cue.ascii_label,
        unicode_label: cue.unicode_label,
        placement: cue.placement,
        remaining_ms: cue.until.saturating_duration_since(now).as_millis() as u64,
    })
}

fn build_impact_cue_snapshot(
    cue: Option<NetworkImpactCue>,
    now: Instant,
) -> Option<ImpactCueSnapshot> {
    let cue = cue.filter(|cue| cue.until > now)?;
    Some(ImpactCueSnapshot {
        kind: cue.kind,
        remaining_ms: cue.until.saturating_duration_since(now).as_millis() as u64,
    })
}

fn player_kind(state: &HostState, player_id: PlayerId) -> PlayerKind {
    state
        .players
        .iter()
        .find(|player| player.id == player_id)
        .map(|player| player.kind)
        .unwrap_or(PlayerKind::Human)
}

fn build_bonus_snapshots(bonuses: &BonusState, now: Instant) -> Vec<BonusPointSnapshot> {
    bonuses
        .points
        .iter()
        .map(|point| BonusPointSnapshot {
            after_word_index: point.after_word_index,
            choices: point
                .choices
                .iter()
                .map(|choice| BonusChoiceSnapshot {
                    word: choice.word.clone(),
                    status: match choice.status {
                        BonusChoiceStatus::Available => BonusChoiceSnapshotStatus::Available,
                        BonusChoiceStatus::Cooldown { until } if until <= now => {
                            BonusChoiceSnapshotStatus::Available
                        }
                        BonusChoiceStatus::Cooldown { until } => {
                            BonusChoiceSnapshotStatus::Cooldown {
                                remaining_ms: until.saturating_duration_since(now).as_millis()
                                    as u64,
                            }
                        }
                    },
                })
                .collect(),
        })
        .collect()
}

fn protocol_key_to_action(key: ProtocolKey) -> KeyAction {
    match key {
        ProtocolKey::Char(' ') | ProtocolKey::Space => KeyAction::Space,
        ProtocolKey::Char(ch) => KeyAction::Char(ch),
        ProtocolKey::Backspace => KeyAction::Backspace,
    }
}

impl From<AssignedColor> for PlayerColorId {
    fn from(value: AssignedColor) -> Self {
        match value {
            AssignedColor::Cyan => Self::Cyan,
            AssignedColor::Red => Self::Red,
            AssignedColor::Green => Self::Green,
            AssignedColor::Blue => Self::Blue,
            AssignedColor::Yellow => Self::Yellow,
            AssignedColor::Magenta => Self::Magenta,
        }
    }
}

impl From<PlayerColorId> for AssignedColor {
    fn from(value: PlayerColorId) -> Self {
        match value {
            PlayerColorId::Cyan => Self::Cyan,
            PlayerColorId::Red => Self::Red,
            PlayerColorId::Green => Self::Green,
            PlayerColorId::Blue => Self::Blue,
            PlayerColorId::Yellow => Self::Yellow,
            PlayerColorId::Magenta => Self::Magenta,
        }
    }
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
        add_network_ai_racers, advance_network_ai_racers, apply_network_banana_to_player,
        apply_network_key_input, broadcast_lobby_snapshot, broadcast_race_results_once,
        broadcast_race_snapshot, build_race_result_rows, build_race_snapshot,
        cleanup_disconnected_waiting_players, client_is_in_current_race, connected_player_count,
        first_available_color, handle_client_messages, handle_player_disconnect,
        new_human_lobby_player, push_event, read_join_hello, reconcile_phase_after_disconnect,
        remove_lobby_player, rename_lobby_player, reset_race_from_lobby, return_to_lobby,
        set_lobby_ai_difficulty, unique_player_name, update_host_ready, update_race_status,
        validate_host_capacity, welcome_joiner,
    };
    use crate::game::{
        ai::AiDifficulty,
        bonus::{BonusChoice, BonusChoiceStatus, BonusPoint, BonusState},
        effects::ActiveEffect,
        items::{HeldItem, ItemActivation, ItemDefinition, ItemPickup, ItemRegistry},
        mods::{ActiveModConfig, ContentMetadata},
        race::{PlayerColorId, RacePlayerId, RaceState},
        stats::TypingStats,
        track::{Track, WordList},
        typing::KeyAction,
        words::WordSetDefinition,
    };
    use crate::net::protocol::{
        ClientMessage, ImpactCueSnapshotKind, LobbyPlayer, PlayerKind, RaceResultStatus,
        ServerMessage, decode_server_message, encode_client_message,
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
        let players = vec![
            lobby_player(PlayerId(1), "tom", PlayerKind::Human, true),
            lobby_player(PlayerId(2), "Tom2", PlayerKind::Human, true),
            lobby_player(PlayerId(3), "tom3", PlayerKind::Human, false),
        ];

        assert_eq!(unique_player_name("tom", &players), "tom3");
        assert_eq!(unique_player_name("alex", &players), "alex");
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
                bonus_attempts: HashMap::new(),
                spent_bonus_gaps: HashMap::new(),
                player_effects: HashMap::new(),
                phase: NetworkRacePhase::WaitingForHost,
                snapshot_sequence: 0,
                events: Vec::new(),
                placements: Vec::new(),
                first_finished_at: None,
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
                bonus_attempts: HashMap::new(),
                spent_bonus_gaps: HashMap::new(),
                player_effects: HashMap::new(),
                phase: NetworkRacePhase::WaitingForHost,
                snapshot_sequence: 0,
                events: Vec::new(),
                placements: Vec::new(),
                first_finished_at: None,
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
        assert_eq!(first_available_color(&players), AssignedColor::Red);
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
        state.player_effects.insert(
            PlayerId(2),
            super::NetworkPlayerEffects {
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
        assert_eq!(state.spent_bonus_gaps.get(&PlayerId(2)), Some(&0));
        assert!(state.events.iter().any(|event| event == "alex got Shield"));
    }

    #[test]
    fn network_ai_racer_can_claim_bonus_and_hit_human_with_banana() {
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
                .player_effects
                .get(&PlayerId(1))
                .and_then(|effects| effects.stunned_until)
                .is_some_and(|until| until > now)
        );
        assert!(
            state
                .player_effects
                .get(&PlayerId(2))
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

        let result = apply_network_banana_to_player(&mut state, PlayerId(2), now);

        assert_eq!(result, Some(super::BananaResolution::SpunOut));
        assert_eq!(state.ai_racers.get(&PlayerId(2)).unwrap().char_budget, 0.0);
        assert!(
            state
                .player_effects
                .get(&PlayerId(2))
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
        state.placements = vec![PlayerId(2), PlayerId(1)];
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
        assert!(state.placements.is_empty());
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
        state.placements = vec![PlayerId(2), PlayerId(1)];
        state.race_results_sent = true;

        return_to_lobby(&mut state).unwrap();

        assert_eq!(state.phase, NetworkRacePhase::WaitingForHost);
        assert!(state.placements.is_empty());
        assert!(!state.race_results_sent);
        assert_eq!(state.race.players.len(), 2);
    }

    #[test]
    fn return_to_lobby_cancels_active_race() {
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.snapshot_sequence = 7;
        state.placements = vec![PlayerId(1)];
        state.player_effects.insert(PlayerId(1), Default::default());

        return_to_lobby(&mut state).unwrap();

        assert_eq!(state.snapshot_sequence, 8);
        assert_eq!(state.phase, NetworkRacePhase::WaitingForHost);
        assert!(state.placements.is_empty());
        assert!(state.player_effects.is_empty());
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
            bonus_attempts: HashMap::new(),
            spent_bonus_gaps: HashMap::new(),
            player_effects: HashMap::new(),
            phase: NetworkRacePhase::WaitingForHost,
            snapshot_sequence: 0,
            events: Vec::new(),
            placements: Vec::new(),
            first_finished_at: None,
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
            bonus_attempts: HashMap::new(),
            spent_bonus_gaps: HashMap::new(),
            player_effects: HashMap::new(),
            phase: NetworkRacePhase::Countdown {
                remaining_seconds: 3,
            },
            snapshot_sequence: 0,
            events: Vec::new(),
            placements: Vec::new(),
            first_finished_at: None,
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
            bonus_attempts: HashMap::new(),
            spent_bonus_gaps: HashMap::new(),
            player_effects: HashMap::new(),
            phase: NetworkRacePhase::Racing,
            snapshot_sequence: 0,
            events: Vec::new(),
            placements: Vec::new(),
            first_finished_at: None,
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
        assert!(state.bonus_attempts.contains_key(&PlayerId(2)));
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
        assert!(!state.bonus_attempts.contains_key(&PlayerId(2)));
        assert_eq!(state.spent_bonus_gaps.get(&PlayerId(2)), Some(&0));
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
        assert_eq!(state.spent_bonus_gaps.get(&PlayerId(2)), Some(&0));

        apply_network_key_input(&mut state, PlayerId(2), KeyAction::Char('s'), now);

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(alex.state.input, "s");
        assert_eq!(alex.state.typo_index, Some(0));
        assert!(!state.bonus_attempts.contains_key(&PlayerId(2)));
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
    fn network_banana_stuns_nearest_target_and_clears_input() {
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
        assert!(
            state
                .player_effects
                .get(&PlayerId(2))
                .and_then(|effects| effects.stunned_until)
                .is_some_and(|until| until > now)
        );
        assert!(
            state
                .player_effects
                .get(&PlayerId(1))
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
        let result = apply_network_banana_to_player(&mut state, PlayerId(2), now);

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(result, Some(super::BananaResolution::Blocked));
        assert!(!alex.state.has_active_shield(now));
        assert_eq!(
            state
                .player_effects
                .get(&PlayerId(2))
                .and_then(|effects| effects.impact_cue)
                .map(|cue| cue.kind),
            Some(ImpactCueSnapshotKind::ShieldBlock)
        );
    }

    #[test]
    fn network_star_pickup_marks_snapshot_as_starred() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);

        activate_network_pickup(
            &mut state,
            PlayerId(1),
            ItemPickup::Held(HeldItem::Star),
            now,
        );
        let snapshot = build_race_snapshot(&mut state);

        assert!(snapshot.players[0].starred);
    }

    #[test]
    fn network_blue_shell_reverses_first_place_target_word() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.race.players[1].state.word_index = 1;

        activate_network_pickup(
            &mut state,
            PlayerId(1),
            ItemPickup::Held(HeldItem::BlueShell),
            now,
        );

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(alex.state.word_override(1), Some("owt"));
        assert!(
            state
                .events
                .iter()
                .any(|event| event == "host hit alex with Blue Shell")
        );
    }

    #[test]
    fn network_blue_shell_is_blocked_by_shield_and_consumes_shield() {
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
            ItemPickup::Held(HeldItem::BlueShell),
            now,
        );

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert_eq!(alex.state.word_override(1), None);
        assert!(!alex.state.has_active_shield(now));
        assert!(
            state
                .events
                .iter()
                .any(|event| event == "alex blocked Blue Shell")
        );
    }

    #[test]
    fn network_squid_ink_marks_all_targets_in_range() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Racing);
        state.race.players[0].state.word_index = 1;
        state.race.players[1].state.word_index = 3;

        activate_network_pickup(
            &mut state,
            PlayerId(1),
            ItemPickup::Held(HeldItem::SquidInk),
            now,
        );

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert!(alex.state.is_inked());
        assert_eq!(
            state
                .player_effects
                .get(&PlayerId(2))
                .and_then(|effects| effects.impact_cue)
                .map(|cue| cue.kind),
            Some(ImpactCueSnapshotKind::SquidInk)
        );

        let snapshot = build_race_snapshot(&mut state);
        assert!(snapshot.players[1].inked);
    }

    #[test]
    fn network_squid_ink_is_blocked_by_shield() {
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
            ItemPickup::Held(HeldItem::SquidInk),
            now,
        );

        let alex = state.race.player(RacePlayerId(2)).unwrap();
        assert!(!alex.state.is_inked());
        assert!(!alex.state.has_active_shield(now));
        assert!(
            state
                .events
                .iter()
                .any(|event| event == "alex blocked Squid Ink")
        );
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
            bonus_attempts: HashMap::new(),
            spent_bonus_gaps: HashMap::new(),
            player_effects: HashMap::new(),
            phase: NetworkRacePhase::Racing,
            snapshot_sequence: 0,
            events: Vec::new(),
            placements: Vec::new(),
            first_finished_at: None,
            debug_log: None,
            race_results_sent: false,
        };

        finish_player(&mut state, RacePlayerId(2), now);
        update_race_status(&mut state, now);

        assert_eq!(state.phase, NetworkRacePhase::Racing);
        assert_eq!(state.placements, vec![PlayerId(2)]);

        finish_player(&mut state, RacePlayerId(1), now);
        update_race_status(&mut state, now);

        assert_eq!(state.phase, NetworkRacePhase::Finished);
        assert_eq!(state.placements, vec![PlayerId(2), PlayerId(1)]);
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
            bonus_attempts: HashMap::new(),
            spent_bonus_gaps: HashMap::new(),
            player_effects: HashMap::new(),
            phase: NetworkRacePhase::Racing,
            snapshot_sequence: 0,
            events: Vec::new(),
            placements: Vec::new(),
            first_finished_at: None,
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
        assert_eq!(state.placements, vec![PlayerId(2), PlayerId(1)]);
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
        assert!(state.placements.is_empty());
        assert!(state.events.iter().any(|event| event == "Race finished"));
    }

    #[test]
    fn countdown_cancels_when_fewer_than_two_connected_racers_remain() {
        let now = std::time::Instant::now();
        let mut state = test_host_state(NetworkRacePhase::Countdown {
            remaining_seconds: 2,
        });
        state.players[1].connected = false;
        state.race.players[1].connected = false;

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
    fn countdown_continues_when_at_least_two_racers_remain_connected() {
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
            bonus_attempts: HashMap::new(),
            spent_bonus_gaps: HashMap::new(),
            player_effects: HashMap::new(),
            phase: NetworkRacePhase::Finished,
            snapshot_sequence: 0,
            events: Vec::new(),
            placements: vec![PlayerId(2), PlayerId(1)],
            first_finished_at: None,
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
        state.placements = vec![PlayerId(2)];

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
            ItemPickup::Held(HeldItem::Star) => ("star", "Star Power", ItemActivation::Held),
            ItemPickup::Held(HeldItem::BlueShell) => {
                ("blue_shell", "Blue Shell", ItemActivation::Held)
            }
            ItemPickup::Held(HeldItem::SquidInk) => {
                ("squid_ink", "Squid Ink", ItemActivation::Held)
            }
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
            bonus_attempts: HashMap::new(),
            spent_bonus_gaps: HashMap::new(),
            player_effects: HashMap::new(),
            phase,
            snapshot_sequence: 0,
            events: Vec::new(),
            placements: Vec::new(),
            first_finished_at: None,
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
