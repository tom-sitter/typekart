//! Local AI racer configuration.
//!
//! AI racers use simple WPM-based behavior across local, network-hosted, and
//! browser-hosted races.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiDifficulty {
    Easy,
    Hard,
}

impl AiDifficulty {
    pub fn wpm_range(self) -> std::ops::RangeInclusive<f64> {
        match self {
            Self::Easy => 20.0..=50.0,
            Self::Hard => 55.0..=105.0,
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
mod tests;
