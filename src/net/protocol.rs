//! JSON protocol messages for local-network multiplayer.
//!
//! The first network transport will send one JSON message per line over TCP.
//! Keeping the protocol explicit and text-readable makes early multiplayer bugs
//! much easier to diagnose from logs.

use serde::{Deserialize, Serialize};

use crate::game::{ai::AiDifficulty, mods::ActiveModConfig};

pub fn version_mismatch_message(host_version: &str, client_version: &str) -> String {
    format!(
        "Version mismatch: host is {host_version}, client is {client_version}. Install the same TypeKart version as the host."
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

impl From<AiDifficultySnapshot> for AiDifficulty {
    fn from(value: AiDifficultySnapshot) -> Self {
        match value {
            AiDifficultySnapshot::Easy => Self::Easy,
            AiDifficultySnapshot::Hard => Self::Hard,
        }
    }
}

impl From<AiDifficulty> for AiDifficultySnapshot {
    fn from(value: AiDifficulty) -> Self {
        match value {
            AiDifficulty::Easy => Self::Easy,
            AiDifficulty::Hard => Self::Hard,
        }
    }
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
    pub inked: bool,
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
    SquidInk,
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

impl From<&ActiveModConfig> for ModConfigSnapshot {
    fn from(config: &ActiveModConfig) -> Self {
        Self {
            word_set_id: config.word_set_id.clone(),
            word_set_name: config.word_set_name.clone(),
            word_set_hash: config.word_set_hash.hex(),
            item_pack_name: config.item_pack_name.clone(),
            item_registry_hash: config.item_registry_hash.hex(),
            combined_hash: config.combined_hash.hex(),
        }
    }
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
    SquidInk,
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
mod tests {
    use super::{
        AssignedColor, BonusChoiceSnapshot, BonusChoiceSnapshotStatus, BonusPointSnapshot,
        ClientMessage, ClientSequence, LobbyPlayer, ModConfigSnapshot, NetworkRacePhase, PlayerId,
        PlayerKind, PlayerSnapshot, ProtocolKey, RaceDeltaSnapshot, RaceResultRow,
        RaceResultStatus, RaceSnapshot, ServerMessage, WordOverrideSnapshot, decode_client_message,
        decode_server_message, encode_client_message, encode_server_message,
    };

    #[test]
    fn client_message_round_trips_key_input() {
        let message = ClientMessage::KeyInput {
            sequence: ClientSequence(42),
            key: ProtocolKey::Char('a'),
        };

        let encoded = encode_client_message(&message).unwrap();
        let decoded = decode_client_message(&encoded).unwrap();

        assert_eq!(decoded, message);
    }

    #[test]
    fn client_message_round_trips_lobby_commands() {
        let messages = [
            ClientMessage::Hello {
                name: "tom".to_string(),
                client_version: env!("CARGO_PKG_VERSION").to_string(),
            },
            ClientMessage::SetReady { ready: true },
            ClientMessage::Rename {
                name: "alex".to_string(),
            },
            ClientMessage::StartCountdown,
            ClientMessage::RestartRace,
            ClientMessage::Leave,
        ];

        for message in messages {
            let encoded = encode_client_message(&message).unwrap();
            let decoded = decode_client_message(&encoded).unwrap();
            assert_eq!(decoded, message);
        }
    }

    #[test]
    fn server_message_round_trips_lobby_snapshot() {
        let message = ServerMessage::LobbySnapshot {
            host_id: PlayerId(1),
            mod_config: test_mod_config(),
            events: vec!["host joined".to_string()],
            players: vec![LobbyPlayer {
                id: PlayerId(1),
                name: "tom".to_string(),
                kind: PlayerKind::Human,
                color: AssignedColor::Cyan,
                ready: false,
                connected: true,
                ai_difficulty: None,
                ai_wpm: None,
            }],
        };

        let encoded = encode_server_message(&message).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();

        assert_eq!(decoded, message);
    }

    #[test]
    fn server_message_round_trips_race_snapshot() {
        let message = ServerMessage::RaceSnapshot(RaceSnapshot {
            sequence: 7,
            phase: NetworkRacePhase::Racing,
            mod_config: test_mod_config(),
            track_words: vec!["one".to_string(), "two".to_string()],
            bonuses: vec![BonusPointSnapshot {
                after_word_index: 0,
                choices: vec![BonusChoiceSnapshot {
                    word: "boost".to_string(),
                    status: BonusChoiceSnapshotStatus::Available,
                }],
            }],
            players: vec![PlayerSnapshot {
                id: PlayerId(1),
                name: "tom".to_string(),
                kind: PlayerKind::Human,
                color: AssignedColor::Cyan,
                word_index: 0,
                input: "o".to_string(),
                typo_index: None,
                word_overrides: vec![WordOverrideSnapshot {
                    word_index: 1,
                    word: "owt".to_string(),
                }],
                finished: false,
                connected: true,
                shielded: false,
                focused: true,
                inked: false,
                boosted: false,
                stunned: false,
                impact_remaining_ms: 0,
                impact_cue: None,
                item_cue: None,
            }],
            events: vec!["Go".to_string()],
        });

        let encoded = encode_server_message(&message).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();

        assert_eq!(decoded, message);
    }

    #[test]
    fn server_message_round_trips_race_delta() {
        let message = ServerMessage::RaceDelta(RaceDeltaSnapshot {
            sequence: 8,
            phase: NetworkRacePhase::Racing,
            bonuses: Vec::new(),
            players: vec![PlayerSnapshot {
                id: PlayerId(1),
                name: "tom".to_string(),
                kind: PlayerKind::Human,
                color: AssignedColor::Cyan,
                word_index: 1,
                input: "t".to_string(),
                typo_index: None,
                word_overrides: Vec::new(),
                finished: false,
                connected: true,
                shielded: false,
                focused: false,
                inked: false,
                boosted: false,
                stunned: false,
                impact_remaining_ms: 0,
                impact_cue: None,
                item_cue: None,
            }],
            events: vec!["Go".to_string()],
        });

        let encoded = encode_server_message(&message).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();

        assert_eq!(decoded, message);
    }

    #[test]
    fn server_message_round_trips_race_results() {
        let message = ServerMessage::RaceResults {
            placements: vec![PlayerId(2), PlayerId(1)],
            rows: vec![
                RaceResultRow {
                    placement: 1,
                    player_id: PlayerId(2),
                    name: "alex".to_string(),
                    color: AssignedColor::Red,
                    status: RaceResultStatus::Finished,
                    progress_words: 20,
                    track_words: 20,
                    wpm: 72,
                    accuracy_percent: 98,
                    typo_chars: 1,
                    backspaces: 2,
                },
                RaceResultRow {
                    placement: 2,
                    player_id: PlayerId(1),
                    name: "tom".to_string(),
                    color: AssignedColor::Cyan,
                    status: RaceResultStatus::TimedOut,
                    progress_words: 17,
                    track_words: 20,
                    wpm: 54,
                    accuracy_percent: 95,
                    typo_chars: 3,
                    backspaces: 4,
                },
            ],
        };

        let encoded = encode_server_message(&message).unwrap();
        let decoded = decode_server_message(&encoded).unwrap();

        assert_eq!(decoded, message);
    }

    fn test_mod_config() -> ModConfigSnapshot {
        ModConfigSnapshot {
            word_set_id: "classic".to_string(),
            word_set_name: "Classic".to_string(),
            word_set_hash: "0000000000000001".to_string(),
            item_pack_name: "classic".to_string(),
            item_registry_hash: "0000000000000002".to_string(),
            combined_hash: "0000000000000003".to_string(),
        }
    }
}
