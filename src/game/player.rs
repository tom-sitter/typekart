//! Player race state.
//!
//! For Milestone 1 there is only one local player, but this type is shaped so
//! it can later become the per-player state stored by a multiplayer server.

use std::{collections::HashMap, time::Instant};

use super::{effects::ActiveEffect, items::HeldItem, stats::TypingStats};

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
    pub held_item: Option<HeldItem>,
    pub active_effects: Vec<ActiveEffect>,
    pub word_overrides: HashMap<usize, String>,
    pub inked_word_index: Option<usize>,
    pub inked_until: Option<Instant>,
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
            held_item: None,
            active_effects: Vec::new(),
            word_overrides: HashMap::new(),
            inked_word_index: None,
            inked_until: None,
        }
    }

    pub fn is_finished(&self) -> bool {
        self.finished_at.is_some()
    }

    pub fn has_active_shield(&self, now: Instant) -> bool {
        self.active_effects
            .iter()
            .any(|effect| effect.is_shield_active_at(now))
    }

    pub fn has_active_focus(&self, now: Instant) -> bool {
        self.active_effects
            .iter()
            .any(|effect| effect.is_focus_active_at(now))
    }

    pub fn word_override(&self, word_index: usize) -> Option<&str> {
        self.word_overrides.get(&word_index).map(String::as_str)
    }

    pub fn is_inked_at(&self, now: Instant) -> bool {
        !self.is_finished() && self.inked_until.is_some_and(|until| until > now)
    }

    pub fn expire_effects(&mut self, now: Instant) -> usize {
        if self.is_finished() || self.inked_until.is_some_and(|until| until <= now) {
            self.inked_word_index = None;
            self.inked_until = None;
        }

        let before = self.active_effects.len();
        self.active_effects.retain(|effect| match effect {
            ActiveEffect::Shield { until } => *until > now,
            ActiveEffect::Focus { until } => *until > now,
            ActiveEffect::Mushroom {
                remaining_words, ..
            } => *remaining_words > 0,
        });
        before - self.active_effects.len()
    }
}
