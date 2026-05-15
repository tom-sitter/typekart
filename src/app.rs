//! Application-level coordination.
//!
//! This module wires together word loading, track generation, player state, and
//! the terminal session. It intentionally avoids owning the rules themselves.

use std::{path::PathBuf, time::Instant};

use anyhow::{Context, Result};

use crate::game::{
    ai::AiDifficulty,
    player::PlayerState,
    track::{Track, WordList},
};
use crate::ui::{render::IconMode, terminal::run_typing_session};

pub fn play(
    word_count: usize,
    ai_racer_count: usize,
    ai_difficulty: AiDifficulty,
    icon_mode: IconMode,
    debug_log: Option<PathBuf>,
) -> Result<()> {
    let word_list = WordList::load("words_alpha.txt").context("failed to load word list")?;
    let track = Track::generate(&word_list, word_count).context("failed to generate track")?;
    let player = PlayerState::new(Instant::now());

    run_typing_session(
        track,
        player,
        word_list,
        ai_racer_count,
        ai_difficulty,
        icon_mode,
        debug_log,
    )
}
