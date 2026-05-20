//! TypeKart network protocol re-exports and game-specific conversions.
//!
//! The shared wire types live in `typekart-protocol` so the browser app can use
//! the same JSON contract without compiling the terminal client or native
//! networking stack.

#[path = "protocol_types.rs"]
mod protocol_types;

pub use protocol_types::*;

use crate::game::{ai::AiDifficulty, mods::ActiveModConfig};

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
