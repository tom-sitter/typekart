//! Relay room and envelope protocol.
//!
//! The relay protocol wraps opaque TypeKart client/server payloads so a public
//! relay can route them without understanding race rules or game command
//! schemas.

use anyhow::{Result, bail};
use rand::{seq::SliceRandom, thread_rng};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::protocol::PlayerId;

const ROOM_CODE_WORDS: &[&str] = &[
    "apple", "beach", "brave", "candy", "cedar", "charm", "cloud", "coral", "crisp", "delta",
    "eagle", "ember", "fancy", "field", "flame", "frost", "giant", "glide", "grape", "happy",
    "harbor", "honey", "jolly", "laser", "lemon", "lucky", "maple", "melon", "mint", "music",
    "noble", "ocean", "olive", "orbit", "panda", "pearl", "pilot", "pixel", "quiet", "racer",
    "river", "rocket", "salad", "shadow", "spark", "sunny", "tango", "tiger", "ultra", "vivid",
    "water", "whale", "wonder", "yellow", "zebra",
];
const ROOM_CODE_WORD_COUNT: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RoomCode(String);

impl RoomCode {
    pub fn generate() -> Self {
        let mut rng = thread_rng();
        let words = ROOM_CODE_WORDS
            .choose_multiple(&mut rng, ROOM_CODE_WORD_COUNT)
            .copied()
            .collect::<Vec<_>>();
        Self(words.join("-"))
    }

    #[allow(dead_code)]
    pub fn parse(value: impl AsRef<str>) -> Result<Self> {
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
            bail!("room code must be three words separated by hyphens");
        }
        if !words.iter().all(|word| ROOM_CODE_WORDS.contains(word)) {
            bail!("room code contains an unknown word");
        }
        Ok(Self(words.join("-")))
    }

    #[allow(dead_code)]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[allow(dead_code)]
    pub fn display(&self) -> String {
        self.0.clone()
    }
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

#[cfg(test)]
mod tests {
    use super::{RelayClientMessage, RelayServerMessage, RoomCode};
    use crate::net::protocol::PlayerId;

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
    fn generated_room_codes_are_valid() {
        let code = RoomCode::generate();

        assert!(RoomCode::parse(code.as_str()).is_ok());
    }

    #[test]
    fn client_relay_envelopes_round_trip() {
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
    fn server_relay_envelopes_round_trip() {
        let message = RelayServerMessage::HostToClient {
            room: RoomCode::parse("rocket-salad-tiger").unwrap(),
            player_id: PlayerId(2),
            message: serde_json::json!({
                "type": "future_server_command",
                "payload": { "anything": true }
            }),
        };

        let encoded = serde_json::to_string(&message).unwrap();
        let decoded = serde_json::from_str::<RelayServerMessage>(&encoded).unwrap();

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
    }
}
