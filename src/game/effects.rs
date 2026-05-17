//! Timed effects and pending attacks.

use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveEffect {
    Shield {
        until: Instant,
    },
    Star {
        until: Instant,
    },
    Mushroom {
        remaining_words: usize,
        next_step_at: Instant,
        step_interval: std::time::Duration,
    },
}

impl ActiveEffect {
    pub fn is_shield_active_at(self, now: Instant) -> bool {
        match self {
            Self::Shield { until } => until > now,
            Self::Star { .. } | Self::Mushroom { .. } => false,
        }
    }

    pub fn is_star_active_at(self, now: Instant) -> bool {
        match self {
            Self::Star { until } => until > now,
            Self::Mushroom { .. } => false,
            Self::Shield { .. } => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum PendingAttack {
    BananaWordSwap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttackWarning {
    pub attack: PendingAttack,
    pub resolves_at: Instant,
}
