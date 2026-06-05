// Shared JSON protocol messages for TypeKart multiplayer.
//
// This module intentionally has no dependency on terminal UI, native networking,
// or game-engine modules so browser clients can compile the same wire contract.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{error::Error, fmt};

const ROOM_CODE_WORDS: &[&str] = &[
    "apple", "beach", "brave", "candy", "cedar", "charm", "cloud", "coral", "crisp", "delta",
    "eagle", "ember", "fancy", "field", "flame", "frost", "giant", "glide", "grape", "happy",
    "harbor", "honey", "jolly", "laser", "lemon", "lucky", "maple", "melon", "mint", "music",
    "noble", "ocean", "olive", "orbit", "panda", "pearl", "pilot", "pixel", "quiet", "racer",
    "river", "rocket", "salad", "shadow", "spark", "sunny", "tango", "tiger", "ultra", "vivid",
    "water", "whale", "wonder", "yellow", "zebra",
];
const ROOM_CODE_WORD_COUNT: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoomCodeParseError {
    WrongWordCount,
    UnknownWord,
}

impl fmt::Display for RoomCodeParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongWordCount => write!(
                formatter,
                "room code must be three words separated by hyphens"
            ),
            Self::UnknownWord => write!(formatter, "room code contains an unknown word"),
        }
    }
}

impl Error for RoomCodeParseError {}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RoomCode(pub(crate) String);

impl RoomCode {
    pub fn parse(value: impl AsRef<str>) -> Result<Self, RoomCodeParseError> {
        let normalized = value
            .as_ref()
            .trim()
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() {
                    ch.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect::<String>();
        let words = normalized
            .split('-')
            .filter(|word| !word.is_empty())
            .collect::<Vec<_>>();

        if words.len() != ROOM_CODE_WORD_COUNT {
            return Err(RoomCodeParseError::WrongWordCount);
        }
        if !words.iter().all(|word| ROOM_CODE_WORDS.contains(word)) {
            return Err(RoomCodeParseError::UnknownWord);
        }
        Ok(Self(words.join("-")))
    }

    pub fn from_normalized_words(words: impl Into<String>) -> Self {
        Self(words.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn display(&self) -> String {
        self.0.clone()
    }
}

pub fn version_mismatch_message(room_version: &str, user_version: &str) -> String {
    format!(
        "Version mismatch: this room is running TypeKart {room_version}, but you are running TypeKart {user_version}. Install or launch the same TypeKart version as the room host."
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PlayerId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClientSequence(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolKey {
    Char(char),
    Space,
    Backspace,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Hello {
        name: String,
        client_version: String,
    },
    SetReady {
        ready: bool,
    },
    Rename {
        name: String,
    },
    StartCountdown,
    AddAi,
    RemoveLobbyPlayer {
        player_id: PlayerId,
    },
    SetAiDifficulty {
        player_id: Option<PlayerId>,
        difficulty: AiDifficultySnapshot,
    },
    KeyInput {
        sequence: ClientSequence,
        key: ProtocolKey,
    },
    RestartRace,
    Leave,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssignedColor {
    Cyan,
    Red,
    Green,
    Blue,
    Yellow,
    Magenta,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerKind {
    Human,
    Bot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiDifficultySnapshot {
    Easy,
    Hard,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LobbyPlayer {
    pub id: PlayerId,
    pub name: String,
    pub kind: PlayerKind,
    pub color: AssignedColor,
    pub ready: bool,
    pub connected: bool,
    pub ai_difficulty: Option<AiDifficultySnapshot>,
    pub ai_wpm: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkRacePhase {
    Lobby,
    WaitingForHost,
    Countdown { remaining_seconds: u8 },
    Racing,
    Finished,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlayerSnapshot {
    pub id: PlayerId,
    pub name: String,
    pub kind: PlayerKind,
    pub color: AssignedColor,
    pub word_index: usize,
    pub input: String,
    pub typo_index: Option<usize>,
    pub word_overrides: Vec<WordOverrideSnapshot>,
    pub finished: bool,
    pub connected: bool,
    pub shielded: bool,
    pub focused: bool,
    pub fogged: bool,
    pub boosted: bool,
    pub stunned: bool,
    pub impact_remaining_ms: u64,
    pub impact_cue: Option<ImpactCueSnapshot>,
    pub item_cue: Option<ItemCueSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WordOverrideSnapshot {
    pub word_index: usize,
    pub word: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpactCueSnapshot {
    pub kind: ImpactCueSnapshotKind,
    pub remaining_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImpactCueSnapshotKind {
    Banana,
    Cyclone,
    Fog,
    ShieldBlock,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RaceSnapshot {
    pub sequence: u64,
    pub phase: NetworkRacePhase,
    pub mod_config: ModConfigSnapshot,
    pub track_words: Vec<String>,
    pub bonuses: Vec<BonusPointSnapshot>,
    pub players: Vec<PlayerSnapshot>,
    pub events: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RaceDeltaSnapshot {
    pub sequence: u64,
    pub phase: NetworkRacePhase,
    pub bonuses: Vec<BonusPointSnapshot>,
    pub players: Vec<PlayerSnapshot>,
    pub events: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RaceResultRow {
    pub placement: usize,
    pub player_id: PlayerId,
    pub name: String,
    pub color: AssignedColor,
    pub status: RaceResultStatus,
    pub progress_words: usize,
    pub track_words: usize,
    pub wpm: u32,
    pub accuracy_percent: u32,
    pub typo_chars: usize,
    pub backspaces: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RaceResultStatus {
    Finished,
    TimedOut,
    Disconnected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModConfigSnapshot {
    pub word_set_id: String,
    pub word_set_name: String,
    pub word_set_hash: String,
    pub item_pack_name: String,
    pub item_registry_hash: String,
    pub combined_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BonusPointSnapshot {
    pub after_word_index: usize,
    pub choices: Vec<BonusChoiceSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BonusChoiceSnapshot {
    pub word: String,
    pub status: BonusChoiceSnapshotStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BonusChoiceSnapshotStatus {
    Available,
    Cooldown { remaining_ms: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemCueSnapshot {
    pub kind: ItemCueSnapshotKind,
    pub ascii_label: String,
    pub unicode_label: String,
    pub placement: ItemCuePlacementSnapshot,
    pub remaining_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemCuePlacementSnapshot {
    Before,
    After,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ItemCueSnapshotKind {
    Banana { direction: AttackDirectionSnapshot },
    Cyclone { direction: AttackDirectionSnapshot },
    Fog,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttackDirectionSnapshot {
    Ahead,
    Behind,
    Overlap,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Welcome {
        player_id: PlayerId,
        assigned_color: AssignedColor,
    },
    LobbySnapshot {
        players: Vec<LobbyPlayer>,
        host_id: PlayerId,
        mod_config: ModConfigSnapshot,
        events: Vec<String>,
    },
    RaceSnapshot(RaceSnapshot),
    RaceDelta(RaceDeltaSnapshot),
    RaceEvent {
        message: String,
    },
    RaceResults {
        placements: Vec<PlayerId>,
        rows: Vec<RaceResultRow>,
    },
    Error {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RelayClientMessage {
    CreateRoom {
        host_version: String,
    },
    JoinRoom {
        room: RoomCode,
        name: String,
        client_version: String,
    },
    ClientToHost {
        room: RoomCode,
        player_id: PlayerId,
        message: Value,
    },
    HostToClient {
        room: RoomCode,
        player_id: PlayerId,
        message: Value,
    },
    HostBroadcast {
        room: RoomCode,
        message: Value,
    },
    LeaveRoom {
        room: RoomCode,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RelayServerMessage {
    RoomCreated {
        room: RoomCode,
    },
    JoinForwarded {
        room: RoomCode,
        pending_player_id: PlayerId,
        name: String,
        client_version: String,
    },
    ClientToHost {
        room: RoomCode,
        player_id: PlayerId,
        message: Value,
    },
    HostToClient {
        room: RoomCode,
        player_id: PlayerId,
        message: Value,
    },
    HostBroadcast {
        room: RoomCode,
        message: Value,
    },
    Error {
        message: String,
    },
    RoomClosed {
        reason: String,
    },
    ParticipantDisconnected {
        room: RoomCode,
        player_id: PlayerId,
    },
}

pub fn encode_client_message(message: &ClientMessage) -> serde_json::Result<String> {
    serde_json::to_string(message)
}

pub fn decode_client_message(line: &str) -> serde_json::Result<ClientMessage> {
    serde_json::from_str(line)
}

pub fn encode_server_message(message: &ServerMessage) -> serde_json::Result<String> {
    serde_json::to_string(message)
}

pub fn decode_server_message(line: &str) -> serde_json::Result<ServerMessage> {
    serde_json::from_str(line)
}

#[cfg(test)]
mod tests;
