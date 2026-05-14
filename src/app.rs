//! Application-level coordination.
//!
//! This module wires together word loading, track generation, player state, and
//! the terminal session. It intentionally avoids owning the rules themselves.

use std::time::Instant;

use anyhow::{Context, Result};

use crate::game::{
    player::PlayerState,
    track::{Track, WordList},
};
use crate::ui::terminal::run_typing_session;

pub fn play(word_count: usize) -> Result<()> {
    let word_list = WordList::load("words_alpha.txt").context("failed to load word list")?;
    let track = Track::generate(&word_list, word_count).context("failed to generate track")?;
    let player = PlayerState::new(Instant::now());

    run_typing_session(track, player)
}
