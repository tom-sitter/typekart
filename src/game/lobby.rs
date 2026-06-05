//! Shared lobby policy helpers.
//!
//! Network and browser hosts keep their own transport state, but the core
//! player roster rules should stay consistent across both adapters.

use std::{error::Error, fmt};

use typekart_protocol::{
    AiDifficultySnapshot, AssignedColor, LobbyPlayer, NetworkRacePhase, PlayerId, PlayerKind,
};

use super::{
    race::{RaceParticipant, RacePlayerId},
    snapshot::player_color_id,
};

pub const LOBBY_COLOR_ROTATION: [AssignedColor; 6] = [
    AssignedColor::Cyan,
    AssignedColor::Red,
    AssignedColor::Green,
    AssignedColor::Blue,
    AssignedColor::Yellow,
    AssignedColor::Magenta,
];

pub fn lobby_can_manage_roster(phase: NetworkRacePhase) -> bool {
    matches!(
        phase,
        NetworkRacePhase::Lobby | NetworkRacePhase::WaitingForHost
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LobbyPolicyError {
    RosterLocked,
    LobbyFull,
    HostCannotBeRemoved,
    PlayerMissing,
    NameEmpty,
    RenameUnavailable,
}

impl fmt::Display for LobbyPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::RosterLocked => "Lobby roster can only be changed in the lobby",
            Self::LobbyFull => "Lobby is full",
            Self::HostCannotBeRemoved => "Host cannot be removed",
            Self::PlayerMissing => "Selected racer is no longer in the lobby",
            Self::NameEmpty => "Name cannot be empty",
            Self::RenameUnavailable => "Renaming is only available in the lobby",
        };
        formatter.write_str(message)
    }
}

impl Error for LobbyPolicyError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LobbyReadyOutcome {
    pub player_id: PlayerId,
    pub name: String,
    pub ready: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LobbyRenameOutcome {
    pub player_id: PlayerId,
    pub previous_name: String,
    pub new_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LobbyAiAdded {
    pub player: LobbyPlayer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LobbyPlayerRemoved {
    pub player: LobbyPlayer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LobbyAiDifficultyOutcome {
    DefaultChanged {
        difficulty: AiDifficultySnapshot,
    },
    PlayerChanged {
        player_id: PlayerId,
        name: String,
        difficulty: AiDifficultySnapshot,
        words_per_minute: u32,
    },
}

pub fn set_lobby_ready(
    players: &mut [LobbyPlayer],
    player_id: PlayerId,
    ready: bool,
) -> Result<LobbyReadyOutcome, LobbyPolicyError> {
    let player = players
        .iter_mut()
        .find(|player| {
            player.id == player_id && player.connected && player.kind == PlayerKind::Human
        })
        .ok_or(LobbyPolicyError::PlayerMissing)?;
    player.ready = ready;

    Ok(LobbyReadyOutcome {
        player_id,
        name: player.name.clone(),
        ready,
    })
}

pub fn rename_lobby_player(
    players: &mut [LobbyPlayer],
    phase: NetworkRacePhase,
    player_id: PlayerId,
    requested_name: &str,
) -> Result<LobbyRenameOutcome, LobbyPolicyError> {
    if !lobby_can_manage_roster(phase) {
        return Err(LobbyPolicyError::RenameUnavailable);
    }
    let requested_name = requested_name.trim();
    if requested_name.is_empty() {
        return Err(LobbyPolicyError::NameEmpty);
    }

    let index = players
        .iter()
        .position(|player| {
            player.id == player_id && player.connected && player.kind == PlayerKind::Human
        })
        .ok_or(LobbyPolicyError::PlayerMissing)?;
    let previous_name = players[index].name.clone();
    let new_name = unique_lobby_name(
        players
            .iter()
            .enumerate()
            .filter_map(|(candidate_index, player)| (candidate_index != index).then_some(player)),
        requested_name,
    );
    players[index].name = new_name.clone();

    Ok(LobbyRenameOutcome {
        player_id,
        previous_name,
        new_name,
    })
}

pub fn add_ai_lobby_player(
    players: &mut Vec<LobbyPlayer>,
    phase: NetworkRacePhase,
    max_players: usize,
    difficulty: AiDifficultySnapshot,
    words_per_minute: u32,
) -> Result<LobbyAiAdded, LobbyPolicyError> {
    if !lobby_can_manage_roster(phase) {
        return Err(LobbyPolicyError::RosterLocked);
    }
    if connected_player_count(players) >= max_players {
        return Err(LobbyPolicyError::LobbyFull);
    }
    let player_id = first_available_player_id(players, 2);
    let color = first_available_color(players).ok_or(LobbyPolicyError::LobbyFull)?;
    let name = next_ai_name(players);
    let player = new_ai_lobby_player(player_id, name, color, difficulty, words_per_minute);
    players.push(player.clone());

    Ok(LobbyAiAdded { player })
}

pub fn remove_lobby_player(
    players: &mut Vec<LobbyPlayer>,
    phase: NetworkRacePhase,
    player_id: PlayerId,
) -> Result<LobbyPlayerRemoved, LobbyPolicyError> {
    if !lobby_can_manage_roster(phase) {
        return Err(LobbyPolicyError::RosterLocked);
    }
    if player_id == PlayerId(1) {
        return Err(LobbyPolicyError::HostCannotBeRemoved);
    }
    let index = players
        .iter()
        .position(|player| player.id == player_id)
        .ok_or(LobbyPolicyError::PlayerMissing)?;
    let player = players.remove(index);

    Ok(LobbyPlayerRemoved { player })
}

pub fn set_lobby_ai_difficulty(
    players: &mut [LobbyPlayer],
    phase: NetworkRacePhase,
    player_id: Option<PlayerId>,
    difficulty: AiDifficultySnapshot,
    words_per_minute: u32,
) -> Result<LobbyAiDifficultyOutcome, LobbyPolicyError> {
    if !lobby_can_manage_roster(phase) {
        return Err(LobbyPolicyError::RosterLocked);
    }
    let Some(player_id) = player_id else {
        return Ok(LobbyAiDifficultyOutcome::DefaultChanged { difficulty });
    };
    let Some(player) = players
        .iter_mut()
        .find(|player| player.id == player_id && player.kind == PlayerKind::Bot)
    else {
        return Ok(LobbyAiDifficultyOutcome::DefaultChanged { difficulty });
    };
    player.ai_difficulty = Some(difficulty);
    player.ai_wpm = Some(words_per_minute);

    Ok(LobbyAiDifficultyOutcome::PlayerChanged {
        player_id,
        name: player.name.clone(),
        difficulty,
        words_per_minute,
    })
}

pub fn connected_player_count(players: &[LobbyPlayer]) -> usize {
    players.iter().filter(|player| player.connected).count()
}

pub fn first_available_player_id(players: &[LobbyPlayer], start_at: u64) -> PlayerId {
    let mut id = start_at;
    while players.iter().any(|player| player.id == PlayerId(id)) {
        id += 1;
    }
    PlayerId(id)
}

pub fn first_available_color(players: &[LobbyPlayer]) -> Option<AssignedColor> {
    LOBBY_COLOR_ROTATION.iter().copied().find(|color| {
        !players
            .iter()
            .any(|player| player.connected && player.color == *color)
    })
}

pub fn color_for_lobby_slot(slot: usize) -> AssignedColor {
    LOBBY_COLOR_ROTATION[slot % LOBBY_COLOR_ROTATION.len()]
}

pub fn lobby_name_or_default(name: &str, fallback: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn unique_lobby_name<'a>(
    players: impl Iterator<Item = &'a LobbyPlayer>,
    requested_name: &str,
) -> String {
    let base_name = lobby_name_or_default(requested_name, "player");
    let players = players.collect::<Vec<_>>();
    if !connected_name_exists(&base_name, &players) {
        return base_name;
    }

    let mut suffix = 2;
    loop {
        let candidate = format!("{base_name}{suffix}");
        if !connected_name_exists(&candidate, &players) {
            return candidate;
        }
        suffix += 1;
    }
}

pub fn next_ai_name(players: &[LobbyPlayer]) -> String {
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

pub fn new_human_lobby_player(
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

pub fn new_ai_lobby_player(
    id: PlayerId,
    name: impl Into<String>,
    color: AssignedColor,
    difficulty: AiDifficultySnapshot,
    words_per_minute: u32,
) -> LobbyPlayer {
    LobbyPlayer {
        id,
        name: name.into(),
        kind: PlayerKind::Bot,
        color,
        ready: true,
        connected: true,
        ai_difficulty: Some(difficulty),
        ai_wpm: Some(words_per_minute),
    }
}

pub fn ready_connected_participants(players: &[LobbyPlayer]) -> Vec<RaceParticipant> {
    lobby_players_to_participants(
        &players
            .iter()
            .filter(|player| player.connected && player.ready)
            .cloned()
            .collect::<Vec<_>>(),
    )
}

pub fn lobby_players_to_participants(players: &[LobbyPlayer]) -> Vec<RaceParticipant> {
    players
        .iter()
        .map(|player| RaceParticipant {
            id: RacePlayerId(player.id.0),
            name: player.name.clone(),
            color: player_color_id(player.color),
            connected: player.connected,
        })
        .collect()
}

fn connected_name_exists(name: &str, players: &[&LobbyPlayer]) -> bool {
    players
        .iter()
        .any(|player| player.connected && player.name.eq_ignore_ascii_case(name))
}

#[cfg(test)]
mod tests;
