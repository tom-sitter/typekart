//! Command-line entry point for TypeKart.
//!
//! Keep this file small: it should parse CLI arguments and delegate to `app`.
//! The actual game rules live under `game`, and terminal code lives under `ui`.

mod app;
mod game;
mod ui;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};

use crate::game::ai::AiDifficulty;
use crate::ui::render::IconMode;

#[derive(Debug, Parser)]
#[command(name = "typekart")]
#[command(about = "A terminal typing racer")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Start a local typing race.
    Play {
        /// Number of words in the generated track.
        #[arg(short, long, default_value_t = 40)]
        words: usize,
        /// Number of local AI racers to include, capped at 6.
        #[arg(long, default_value_t = 0)]
        ai_racers: usize,
        /// Difficulty used by all local AI racers.
        #[arg(long, value_enum, default_value_t = CliAiDifficulty::Easy)]
        ai_difficulty: CliAiDifficulty,
        /// Use Unicode item icons instead of ASCII-safe markers.
        #[arg(long)]
        unicode_icons: bool,
        /// Write detailed run diagnostics to this file after the race session exits.
        #[arg(long)]
        debug_log: Option<PathBuf>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliAiDifficulty {
    Easy,
    Hard,
}

impl From<CliAiDifficulty> for AiDifficulty {
    fn from(value: CliAiDifficulty) -> Self {
        match value {
            CliAiDifficulty::Easy => Self::Easy,
            CliAiDifficulty::Hard => Self::Hard,
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Play {
            words,
            ai_racers,
            ai_difficulty,
            unicode_icons,
            debug_log,
        } => app::play(
            words,
            ai_racers,
            ai_difficulty.into(),
            if unicode_icons {
                IconMode::Unicode
            } else {
                IconMode::Ascii
            },
            debug_log,
        ),
    }
}
