//! Raw typing statistics.
//!
//! These stats describe what the player actually typed. Later item effects may
//! change progress, but they should not erase raw typing behavior.

use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TypingStats {
    pub typed_chars: usize,
    pub correct_chars: usize,
    pub typo_chars: usize,
    pub backspaces: usize,
    pub completed_words: usize,
}

impl TypingStats {
    pub fn accuracy(&self) -> f64 {
        if self.typed_chars == 0 {
            return 100.0;
        }

        (self.correct_chars as f64 / self.typed_chars as f64) * 100.0
    }

    pub fn words_per_minute(&self, started_at: Instant, finished_at: Instant) -> f64 {
        let elapsed = finished_at.saturating_duration_since(started_at);
        words_per_minute(self.correct_chars, elapsed)
    }
}

pub fn words_per_minute(correct_chars: usize, elapsed: Duration) -> f64 {
    let minutes = elapsed.as_secs_f64() / 60.0;
    if minutes <= f64::EPSILON {
        return 0.0;
    }

    (correct_chars as f64 / 5.0) / minutes
}

#[cfg(test)]
mod tests;
