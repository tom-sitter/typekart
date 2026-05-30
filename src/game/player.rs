//! Player race state.
//!
//! This is the per-racer typing/effect state used by local, network, and
//! browser-hosted races.

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
    pub fogged_word_index: Option<usize>,
    pub fogged_until: Option<Instant>,
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
            fogged_word_index: None,
            fogged_until: None,
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

    pub fn is_fogged_at(&self, now: Instant) -> bool {
        !self.is_finished() && self.fogged_until.is_some_and(|until| until > now)
    }

    pub fn expire_effects(&mut self, now: Instant) -> usize {
        if self.is_finished() || self.fogged_until.is_some_and(|until| until <= now) {
            self.fogged_word_index = None;
            self.fogged_until = None;
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
