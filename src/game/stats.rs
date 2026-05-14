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
mod tests {
    use std::time::Duration;

    use super::{TypingStats, words_per_minute};

    #[test]
    fn accuracy_is_perfect_before_typing() {
        assert_eq!(TypingStats::default().accuracy(), 100.0);
    }

    #[test]
    fn accuracy_uses_correct_chars_over_typed_chars() {
        let stats = TypingStats {
            typed_chars: 10,
            correct_chars: 8,
            typo_chars: 2,
            backspaces: 1,
            completed_words: 1,
        };

        assert_eq!(stats.accuracy(), 80.0);
    }

    #[test]
    fn wpm_uses_standard_five_character_words() {
        let wpm = words_per_minute(25, Duration::from_secs(30));

        assert_eq!(wpm, 10.0);
    }
}
