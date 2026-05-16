//! JSON protocol messages for local-network multiplayer.
//!
//! The first network transport will send one JSON message per line over TCP.
//! Keeping the protocol explicit and text-readable makes early multiplayer bugs
//! much easier to diagnose from logs.

use serde::{Deserialize, Serialize};

use crate::game::mods::ActiveModConfig;

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
    StartCountdown,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LobbyPlayer {
    pub id: PlayerId,
    pub name: String,
    pub color: AssignedColor,
    pub ready: bool,
    pub connected: bool,
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
    pub color: AssignedColor,
    pub word_index: usize,
    pub input: String,
    pub typo_index: Option<usize>,
    pub finished: bool,
    pub connected: bool,
    pub shielded: bool,
    pub boosted: bool,
    pub stunned: bool,
    pub impact_remaining_ms: u64,
    pub item_cue: Option<ItemCueSnapshot>,
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
    pub remaining_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ItemCueSnapshotKind {
    Banana { direction: AttackDirectionSnapshot },
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
    },
    RaceSnapshot(RaceSnapshot),
    RaceEvent {
        message: String,
    },
    RaceResults {
        placements: Vec<PlayerId>,
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
        decode_client_message, decode_server_message, encode_client_message, encode_server_message,
        AssignedColor, BonusChoiceSnapshot, BonusChoiceSnapshotStatus, BonusPointSnapshot,
        ClientMessage, ClientSequence, LobbyPlayer, ModConfigSnapshot, NetworkRacePhase, PlayerId,
        PlayerSnapshot, ProtocolKey, RaceSnapshot, ServerMessage,
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
            players: vec![LobbyPlayer {
                id: PlayerId(1),
                name: "tom".to_string(),
                color: AssignedColor::Cyan,
                ready: false,
                connected: true,
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
                color: AssignedColor::Cyan,
                word_index: 0,
                input: "o".to_string(),
                typo_index: None,
                finished: false,
                connected: true,
                shielded: false,
                boosted: false,
                stunned: false,
                impact_remaining_ms: 0,
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
