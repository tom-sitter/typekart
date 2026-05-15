//! Local AI racer configuration.
//!
//! The AI racers are a temporary stand-in for remote multiplayer clients. They
//! deliberately use simple WPM-based behavior so the UI and item systems can be
//! exercised without adding networking yet.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiDifficulty {
    Easy,
    Hard,
}

impl AiDifficulty {
    pub fn wpm_range(self) -> std::ops::RangeInclusive<f64> {
        match self {
            Self::Easy => 28.0..=42.0,
            Self::Hard => 65.0..=85.0,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Easy => "easy",
            Self::Hard => "hard",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AiDifficulty;

    #[test]
    fn easy_and_hard_wpm_ranges_are_distinct() {
        let easy = AiDifficulty::Easy.wpm_range();
        let hard = AiDifficulty::Hard.wpm_range();

        assert!(easy.end() < hard.start());
    }
}
