//! Player race state.
//!
//! For Milestone 1 there is only one local player, but this type is shaped so
//! it can later become the per-player state stored by a multiplayer server.

use std::time::Instant;

use super::stats::TypingStats;

#[derive(Debug, Clone)]
pub struct PlayerState {
    /// Index of the current target word in the race track.
    pub word_index: usize,
    /// Raw input for the current word, including typo characters.
    pub input: String,
    /// Character index of the first typo in `input`, if a typo is active.
    pub typo_index: Option<usize>,
    pub started_at: Instant,
    pub finished_at: Option<Instant>,
    pub stats: TypingStats,
}

impl PlayerState {
    pub fn new(started_at: Instant) -> Self {
        Self {
            word_index: 0,
            input: String::new(),
            typo_index: None,
            started_at,
            finished_at: None,
            stats: TypingStats::default(),
        }
    }

    pub fn is_finished(&self) -> bool {
        self.finished_at.is_some()
    }
}
