//! Network-host state container and initial state construction.

use std::{collections::HashMap, net::TcpStream, time::Instant};

use crate::{
    game::{
        ai::AiDifficulty,
        bonus::BonusState,
        bonus_flow::BonusAttempt,
        items::ItemRegistry,
        lobby::LOBBY_COLOR_ROTATION,
        mods::ActiveModConfig,
        race::{PlayerColorId, RacePlayerId, RaceRuntimeState, RaceState},
        track::WordList,
    },
    net::{
        log::SharedNetworkLog,
        protocol::{LobbyPlayer, NetworkRacePhase, PlayerId, PlayerKind},
    },
};

use super::{HostConfig, host_ai::NetworkAiRacer};

pub(super) struct ConnectedClient {
    pub(super) player_id: PlayerId,
    pub(super) stream: TcpStream,
}

pub(super) struct HostState {
    pub(super) players: Vec<LobbyPlayer>,
    pub(super) clients: Vec<ConnectedClient>,
    pub(super) race: RaceState,
    pub(super) ai_racers: HashMap<PlayerId, NetworkAiRacer>,
    pub(super) word_list: WordList,
    pub(super) bonuses: BonusState,
    pub(super) item_registry: ItemRegistry,
    pub(super) active_mod_config: ActiveModConfig,
    pub(super) max_players: usize,
    pub(super) ai_difficulty: AiDifficulty,
    pub(super) runtime: RaceRuntimeState<PlayerId, BonusAttempt>,
    pub(super) phase: NetworkRacePhase,
    pub(super) snapshot_sequence: u64,
    pub(super) events: Vec<String>,
    pub(super) debug_log: Option<SharedNetworkLog>,
    pub(super) race_results_sent: bool,
}

pub(super) struct InitialHostState {
    pub(super) state: HostState,
    pub(super) next_player_id: u64,
}

pub(super) fn build_initial_host_state(config: HostConfig) -> InitialHostState {
    let bonuses = BonusState::generate(&config.track, &config.word_list);
    let mut race = RaceState::new(config.track.clone());
    let mut players = Vec::new();
    let mut next_player_id = 1;
    if let Some(host_name) = config.host_name {
        race.add_player(
            RacePlayerId(1),
            host_name.clone(),
            PlayerColorId::Cyan,
            Instant::now(),
        );
        players.push(LobbyPlayer {
            id: PlayerId(1),
            name: host_name,
            kind: PlayerKind::Human,
            color: LOBBY_COLOR_ROTATION[0],
            ready: true,
            connected: true,
            ai_difficulty: None,
            ai_wpm: None,
        });
        next_player_id = 2;
    }
    let ai_racers = super::host_ai::add_network_ai_racers(
        &mut race,
        &mut players,
        config.ai_racer_count,
        config.ai_difficulty,
        Instant::now(),
    );

    InitialHostState {
        state: HostState {
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
            runtime: RaceRuntimeState::new(),
            phase: NetworkRacePhase::WaitingForHost,
            snapshot_sequence: 0,
            events: Vec::new(),
            debug_log: config.debug_log,
            race_results_sent: false,
        },
        next_player_id,
    }
}
