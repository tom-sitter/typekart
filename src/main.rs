//! Command-line entry point for TypeKart.
//!
//! Keep this file small: it should parse CLI arguments and delegate to `app`.
//! The actual game rules live under `game`, and terminal code lives under `ui`.

mod app;
mod game;
mod net;
mod ui;

use std::{net::SocketAddr, path::PathBuf};

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};

use crate::game::{
    ai::AiDifficulty,
    words::{DEFAULT_WORD_SET_ID, WordSetSelection},
};
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
        /// Built-in word set id to use.
        #[arg(long, default_value = DEFAULT_WORD_SET_ID)]
        word_set: String,
        /// Load a custom word set from a newline-delimited text file.
        #[arg(long)]
        word_set_file: Option<PathBuf>,
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
    /// Host a local-network multiplayer lobby.
    Host {
        /// Display name for the host player.
        #[arg(long)]
        name: String,
        /// Number of words in the generated track.
        #[arg(short, long, default_value_t = 40)]
        words: usize,
        /// Built-in word set id to use.
        #[arg(long, default_value = DEFAULT_WORD_SET_ID)]
        word_set: String,
        /// Load a custom word set from a newline-delimited text file.
        #[arg(long)]
        word_set_file: Option<PathBuf>,
        /// Address and port to listen on.
        #[arg(long, default_value = "127.0.0.1:4000")]
        bind: SocketAddr,
        /// Maximum total players, including the host.
        #[arg(long, default_value_t = 6)]
        max_players: usize,
        /// Write detailed network diagnostics to this file after the session exits.
        #[arg(long)]
        debug_log: Option<PathBuf>,
        /// Use Unicode item icons instead of ASCII-safe markers.
        #[arg(long)]
        unicode_icons: bool,
    },
    /// Join a local-network multiplayer lobby.
    Join {
        /// Display name for this player.
        #[arg(long)]
        name: String,
        /// Host address and port to connect to.
        #[arg(long)]
        server: SocketAddr,
        /// Write detailed network diagnostics to this file after the session exits.
        #[arg(long)]
        debug_log: Option<PathBuf>,
        /// Use Unicode item icons instead of ASCII-safe markers.
        #[arg(long)]
        unicode_icons: bool,
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
            word_set,
            word_set_file,
            ai_racers,
            ai_difficulty,
            unicode_icons,
            debug_log,
        } => app::play(
            words,
            word_set_selection(word_set, word_set_file)?,
            ai_racers,
            ai_difficulty.into(),
            if unicode_icons {
                IconMode::Unicode
            } else {
                IconMode::Ascii
            },
            debug_log,
        ),
        Command::Host {
            name,
            words,
            word_set,
            word_set_file,
            bind,
            max_players,
            debug_log,
            unicode_icons,
        } => app::host(
            bind,
            name,
            words,
            word_set_selection(word_set, word_set_file)?,
            max_players,
            if unicode_icons {
                IconMode::Unicode
            } else {
                IconMode::Ascii
            },
            debug_log,
        ),
        Command::Join {
            name,
            server,
            debug_log,
            unicode_icons,
        } => app::join(
            server,
            name,
            if unicode_icons {
                IconMode::Unicode
            } else {
                IconMode::Ascii
            },
            debug_log,
        ),
    }
}

fn word_set_selection(
    word_set: String,
    word_set_file: Option<PathBuf>,
) -> Result<WordSetSelection> {
    if let Some(path) = word_set_file {
        if word_set != DEFAULT_WORD_SET_ID {
            anyhow::bail!("use either --word-set or --word-set-file, not both");
        }
        Ok(WordSetSelection::File(path))
    } else {
        Ok(WordSetSelection::BuiltIn(word_set))
    }
}
