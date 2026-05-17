//! Relay room and envelope protocol.
//!
//! The relay protocol wraps the existing TypeKart client/server messages so a
//! public relay can route them without understanding race rules.

use anyhow::{bail, Result};
use rand::{distributions::Alphanumeric, thread_rng, Rng};
use serde::{Deserialize, Serialize};

use super::protocol::{ClientMessage, PlayerId, ServerMessage};

const ROOM_CODE_LEN: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RoomCode(String);

impl RoomCode {
    pub fn generate() -> Self {
        let mut rng = thread_rng();
        let code = (0..ROOM_CODE_LEN)
            .map(|_| char::from(rng.sample(Alphanumeric)).to_ascii_uppercase())
            .collect();
        Self(code)
    }

    #[allow(dead_code)]
    pub fn parse(value: impl AsRef<str>) -> Result<Self> {
        let normalized = value
            .as_ref()
            .chars()
            .filter(|ch| *ch != '-')
            .map(|ch| ch.to_ascii_uppercase())
            .collect::<String>();
        if normalized.len() != ROOM_CODE_LEN {
            bail!("room code must be {ROOM_CODE_LEN} characters");
        }
        if !normalized
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
        {
            bail!("room code must contain only ASCII letters and digits");
        }
        Ok(Self(normalized))
    }

    #[allow(dead_code)]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[allow(dead_code)]
    pub fn display(&self) -> String {
        format!("{}-{}", &self.0[..4], &self.0[4..])
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
        message: ClientMessage,
    },
    HostToClient {
        room: RoomCode,
        player_id: PlayerId,
        message: ServerMessage,
    },
    HostBroadcast {
        room: RoomCode,
        message: ServerMessage,
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
        message: ClientMessage,
    },
    HostToClient {
        room: RoomCode,
        player_id: PlayerId,
        message: ServerMessage,
    },
    HostBroadcast {
        room: RoomCode,
        message: ServerMessage,
    },
    Error {
        message: String,
    },
    RoomClosed {
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::{RelayClientMessage, RelayServerMessage, RoomCode};
    use crate::net::protocol::{
        AssignedColor, ClientMessage, ClientSequence, PlayerId, ProtocolKey, ServerMessage,
    };

    #[test]
    fn room_codes_normalize_display_form() {
        let code = RoomCode::parse("ab12-cd34").unwrap();

        assert_eq!(code.as_str(), "AB12CD34");
        assert_eq!(code.display(), "AB12-CD34");
    }

    #[test]
    fn room_codes_reject_invalid_values() {
        assert!(RoomCode::parse("short").is_err());
        assert!(RoomCode::parse("ABCD_123").is_err());
    }

    #[test]
    fn generated_room_codes_are_valid() {
        let code = RoomCode::generate();

        assert!(RoomCode::parse(code.as_str()).is_ok());
    }

    #[test]
    fn client_relay_envelopes_round_trip() {
        let message = RelayClientMessage::ClientToHost {
            room: RoomCode::parse("ABCD-1234").unwrap(),
            player_id: PlayerId(2),
            message: ClientMessage::KeyInput {
                sequence: ClientSequence(9),
                key: ProtocolKey::Space,
            },
        };

        let encoded = serde_json::to_string(&message).unwrap();
        let decoded = serde_json::from_str::<RelayClientMessage>(&encoded).unwrap();

        assert_eq!(decoded, message);
    }

    #[test]
    fn server_relay_envelopes_round_trip() {
        let message = RelayServerMessage::HostToClient {
            room: RoomCode::parse("ABCD-1234").unwrap(),
            player_id: PlayerId(2),
            message: ServerMessage::Welcome {
                player_id: PlayerId(2),
                assigned_color: AssignedColor::Red,
            },
        };

        let encoded = serde_json::to_string(&message).unwrap();
        let decoded = serde_json::from_str::<RelayServerMessage>(&encoded).unwrap();

        assert_eq!(decoded, message);
    }
}
