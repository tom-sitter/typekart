//! Relay room and envelope protocol.
//!
//! The serializable relay envelope types live in the shared protocol module so
//! browser clients and terminal clients use the same wire contract. This module
//! keeps terminal/relay-server room generation helpers.

use rand::{seq::SliceRandom, thread_rng};

pub use super::protocol::{RelayClientMessage, RelayServerMessage, RoomCode};

const ROOM_CODE_WORDS: &[&str] = &[
    "apple", "beach", "brave", "candy", "cedar", "charm", "cloud", "coral", "crisp", "delta",
    "eagle", "ember", "fancy", "field", "flame", "frost", "giant", "glide", "grape", "happy",
    "harbor", "honey", "jolly", "laser", "lemon", "lucky", "maple", "melon", "mint", "music",
    "noble", "ocean", "olive", "orbit", "panda", "pearl", "pilot", "pixel", "quiet", "racer",
    "river", "rocket", "salad", "shadow", "spark", "sunny", "tango", "tiger", "ultra", "vivid",
    "water", "whale", "wonder", "yellow", "zebra",
];
const ROOM_CODE_WORD_COUNT: usize = 3;

pub fn generate_room_code() -> RoomCode {
    let mut rng = thread_rng();
    let words = ROOM_CODE_WORDS
        .choose_multiple(&mut rng, ROOM_CODE_WORD_COUNT)
        .copied()
        .collect::<Vec<_>>();
    RoomCode::from_normalized_words(words.join("-"))
}

#[cfg(test)]
mod tests {
    use super::{RoomCode, generate_room_code};

    #[test]
    fn generated_room_codes_are_valid() {
        let code = generate_room_code();

        assert!(RoomCode::parse(code.as_str()).is_ok());
    }
}
