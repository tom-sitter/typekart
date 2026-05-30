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
mod tests {
    use super::{
        AssignedColor, BonusChoiceSnapshot, BonusChoiceSnapshotStatus, BonusPointSnapshot,
        ClientMessage, ClientSequence, ImpactCueSnapshot, ImpactCueSnapshotKind,
        ItemCuePlacementSnapshot, ItemCueSnapshot, ItemCueSnapshotKind, LobbyPlayer,
        ModConfigSnapshot, NetworkRacePhase, PlayerId, PlayerKind, PlayerSnapshot, ProtocolKey,
        RaceDeltaSnapshot, RaceResultRow, RaceResultStatus, RaceSnapshot, RelayClientMessage,
        RelayServerMessage, RoomCode, ServerMessage, WordOverrideSnapshot, decode_client_message,
        decode_server_message, encode_client_message, encode_server_message,
        version_mismatch_message,
    };

    #[test]
    fn room_codes_normalize_display_form() {
        let code = RoomCode::parse("Rocket Salad TIGER").unwrap();

        assert_eq!(code.as_str(), "rocket-salad-tiger");
        assert_eq!(code.display(), "rocket-salad-tiger");
    }

    #[test]
    fn room_codes_reject_invalid_values() {
        assert!(RoomCode::parse("short").is_err());
        assert!(RoomCode::parse("rocket-salad").is_err());
        assert!(RoomCode::parse("rocket-salad-turnip").is_err());
    }

    #[test]
    fn version_mismatch_message_names_room_and_user_versions() {
        let message = version_mismatch_message("0.1.0", "0.2.0");

        assert!(message.contains("room is running TypeKart 0.1.0"));
        assert!(message.contains("you are running TypeKart 0.2.0"));
    }

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
                fogged: false,
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
                fogged: false,
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

    #[test]
    fn browser_client_key_input_fixture_matches_wire_shape() {
        let message = ClientMessage::KeyInput {
            sequence: ClientSequence(42),
            key: ProtocolKey::Char('x'),
        };

        let value = serde_json::to_value(&message).unwrap();

        assert_eq!(
            value,
            serde_json::json!({
                "type": "key_input",
                "sequence": 42,
                "key": { "char": "x" }
            })
        );
    }

    #[test]
    fn browser_server_lobby_snapshot_fixture_matches_wire_shape() {
        let message = ServerMessage::LobbySnapshot {
            host_id: PlayerId(1),
            mod_config: test_mod_config(),
            events: vec!["tom joined".to_string()],
            players: vec![LobbyPlayer {
                id: PlayerId(1),
                name: "tom".to_string(),
                kind: PlayerKind::Human,
                color: AssignedColor::Cyan,
                ready: true,
                connected: true,
                ai_difficulty: None,
                ai_wpm: None,
            }],
        };

        let value = serde_json::to_value(&message).unwrap();

        assert_eq!(
            value,
            serde_json::json!({
                "type": "lobby_snapshot",
                "players": [
                    {
                        "id": 1,
                        "name": "tom",
                        "kind": "human",
                        "color": "cyan",
                        "ready": true,
                        "connected": true,
                        "ai_difficulty": null,
                        "ai_wpm": null
                    }
                ],
                "host_id": 1,
                "mod_config": {
                    "word_set_id": "classic",
                    "word_set_name": "Classic",
                    "word_set_hash": "0000000000000001",
                    "item_pack_name": "classic",
                    "item_registry_hash": "0000000000000002",
                    "combined_hash": "0000000000000003"
                },
                "events": ["tom joined"]
            })
        );
    }

    #[test]
    fn browser_server_race_snapshot_fixture_matches_wire_shape() {
        let message = ServerMessage::RaceSnapshot(RaceSnapshot {
            sequence: 12,
            phase: NetworkRacePhase::Racing,
            mod_config: test_mod_config(),
            track_words: vec!["spark".to_string(), "river".to_string()],
            bonuses: vec![BonusPointSnapshot {
                after_word_index: 0,
                choices: vec![
                    BonusChoiceSnapshot {
                        word: "focus".to_string(),
                        status: BonusChoiceSnapshotStatus::Available,
                    },
                    BonusChoiceSnapshot {
                        word: "shield".to_string(),
                        status: BonusChoiceSnapshotStatus::Cooldown { remaining_ms: 800 },
                    },
                ],
            }],
            players: vec![PlayerSnapshot {
                id: PlayerId(2),
                name: "alex".to_string(),
                kind: PlayerKind::Human,
                color: AssignedColor::Red,
                word_index: 1,
                input: "r".to_string(),
                typo_index: None,
                word_overrides: vec![WordOverrideSnapshot {
                    word_index: 1,
                    word: "revir".to_string(),
                }],
                finished: false,
                connected: true,
                shielded: true,
                focused: false,
                fogged: false,
                boosted: false,
                stunned: true,
                impact_remaining_ms: 900,
                impact_cue: Some(ImpactCueSnapshot {
                    kind: ImpactCueSnapshotKind::Cyclone,
                    remaining_ms: 900,
                }),
                item_cue: Some(ItemCueSnapshot {
                    kind: ItemCueSnapshotKind::Cyclone {
                        direction: super::AttackDirectionSnapshot::Ahead,
                    },
                    ascii_label: "~~>>".to_string(),
                    unicode_label: "🌀 >>".to_string(),
                    placement: ItemCuePlacementSnapshot::After,
                    remaining_ms: 700,
                }),
            }],
            events: vec!["alex was hit by cyclone".to_string()],
        });

        let value = serde_json::to_value(&message).unwrap();

        assert_eq!(
            value,
            serde_json::json!({
                "type": "race_snapshot",
                "sequence": 12,
                "phase": "racing",
                "mod_config": {
                    "word_set_id": "classic",
                    "word_set_name": "Classic",
                    "word_set_hash": "0000000000000001",
                    "item_pack_name": "classic",
                    "item_registry_hash": "0000000000000002",
                    "combined_hash": "0000000000000003"
                },
                "track_words": ["spark", "river"],
                "bonuses": [
                    {
                        "after_word_index": 0,
                        "choices": [
                            {
                                "word": "focus",
                                "status": "available"
                            },
                            {
                                "word": "shield",
                                "status": {
                                    "cooldown": {
                                        "remaining_ms": 800
                                    }
                                }
                            }
                        ]
                    }
                ],
                "players": [
                    {
                        "id": 2,
                        "name": "alex",
                        "kind": "human",
                        "color": "red",
                        "word_index": 1,
                        "input": "r",
                        "typo_index": null,
                        "word_overrides": [
                            {
                                "word_index": 1,
                                "word": "revir"
                            }
                        ],
                        "finished": false,
                        "connected": true,
                        "shielded": true,
                        "focused": false,
                        "fogged": false,
                        "boosted": false,
                        "stunned": true,
                        "impact_remaining_ms": 900,
                        "impact_cue": {
                            "kind": "cyclone",
                            "remaining_ms": 900
                        },
                        "item_cue": {
                            "kind": {
                                "type": "cyclone",
                                "direction": "ahead"
                            },
                            "ascii_label": "~~>>",
                            "unicode_label": "🌀 >>",
                            "placement": "after",
                            "remaining_ms": 700
                        }
                    }
                ],
                "events": ["alex was hit by cyclone"]
            })
        );
    }

    #[test]
    fn relay_envelopes_round_trip() {
        let message = RelayClientMessage::ClientToHost {
            room: RoomCode::parse("rocket-salad-tiger").unwrap(),
            player_id: PlayerId(2),
            message: serde_json::json!({
                "type": "future_client_command",
                "payload": { "anything": true }
            }),
        };

        let encoded = serde_json::to_string(&message).unwrap();
        let decoded = serde_json::from_str::<RelayClientMessage>(&encoded).unwrap();

        assert_eq!(decoded, message);
    }

    #[test]
    fn browser_relay_host_broadcast_fixture_matches_wire_shape() {
        let message = RelayClientMessage::HostBroadcast {
            room: RoomCode::parse("rocket-salad-tiger").unwrap(),
            message: serde_json::json!({
                "type": "race_delta",
                "sequence": 12
            }),
        };

        let value = serde_json::to_value(&message).unwrap();

        assert_eq!(
            value,
            serde_json::json!({
                "type": "host_broadcast",
                "room": "rocket-salad-tiger",
                "message": {
                    "type": "race_delta",
                    "sequence": 12
                }
            })
        );

        let server_message = RelayServerMessage::HostBroadcast {
            room: RoomCode::parse("rocket-salad-tiger").unwrap(),
            message: serde_json::json!({ "type": "race_delta", "sequence": 12 }),
        };
        let encoded = serde_json::to_string(&server_message).unwrap();
        assert_eq!(
            serde_json::from_str::<RelayServerMessage>(&encoded).unwrap(),
            server_message
        );
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
