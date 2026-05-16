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
    words::{WordSetSelection, DEFAULT_WORD_SET_ID},
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
        /// Load a directory of .txt word sets and choose one at random.
        #[arg(long)]
        word_set_dir: Option<PathBuf>,
        /// Load an item pack from a JSON file.
        #[arg(long)]
        item_pack_file: Option<PathBuf>,
        /// Number of local AI racers to include, capped at 6.
        #[arg(long, default_value_t = 0)]
        ai_racers: usize,
        /// Difficulty used by all local AI racers.
        #[arg(long, value_enum, default_value_t = CliAiDifficulty::Easy)]
        ai_difficulty: CliAiDifficulty,
        /// Use ASCII-safe markers instead of Unicode item icons.
        #[arg(long = "ascii")]
        ascii_icons: bool,
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
        /// Load a directory of .txt word sets and choose one at random.
        #[arg(long)]
        word_set_dir: Option<PathBuf>,
        /// Load an item pack from a JSON file.
        #[arg(long)]
        item_pack_file: Option<PathBuf>,
        /// Address and port to listen on.
        #[arg(long, default_value = "127.0.0.1:4000")]
        bind: SocketAddr,
        /// Maximum total players, including the host.
        #[arg(long, default_value_t = 6)]
        max_players: usize,
        /// Number of server-owned AI racers to include, capped by max players.
        #[arg(long, default_value_t = 0)]
        ai_racers: usize,
        /// Difficulty used by all network AI racers.
        #[arg(long, value_enum, default_value_t = CliAiDifficulty::Easy)]
        ai_difficulty: CliAiDifficulty,
        /// Write detailed network diagnostics to this file after the session exits.
        #[arg(long)]
        debug_log: Option<PathBuf>,
        /// Use ASCII-safe markers instead of Unicode item icons.
        #[arg(long = "ascii")]
        ascii_icons: bool,
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
        /// Use ASCII-safe markers instead of Unicode item icons.
        #[arg(long = "ascii")]
        ascii_icons: bool,
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
            word_set_dir,
            item_pack_file,
            ai_racers,
            ai_difficulty,
            ascii_icons,
            debug_log,
        } => app::play(
            words,
            word_set_selection(word_set, word_set_file, word_set_dir)?,
            item_pack_file,
            ai_racers,
            ai_difficulty.into(),
            if ascii_icons {
                IconMode::Ascii
            } else {
                IconMode::Unicode
            },
            debug_log,
        ),
        Command::Host {
            name,
            words,
            word_set,
            word_set_file,
            word_set_dir,
            item_pack_file,
            bind,
            max_players,
            ai_racers,
            ai_difficulty,
            debug_log,
            ascii_icons,
        } => app::host(
            bind,
            name,
            words,
            word_set_selection(word_set, word_set_file, word_set_dir)?,
            item_pack_file,
            max_players,
            ai_racers,
            ai_difficulty.into(),
            if ascii_icons {
                IconMode::Ascii
            } else {
                IconMode::Unicode
            },
            debug_log,
        ),
        Command::Join {
            name,
            server,
            debug_log,
            ascii_icons,
        } => app::join(
            server,
            name,
            if ascii_icons {
                IconMode::Ascii
            } else {
                IconMode::Unicode
            },
            debug_log,
        ),
    }
}

fn word_set_selection(
    word_set: String,
    word_set_file: Option<PathBuf>,
    word_set_dir: Option<PathBuf>,
) -> Result<WordSetSelection> {
    match (word_set_file, word_set_dir) {
        (Some(_), Some(_)) => anyhow::bail!("use only one of --word-set-file or --word-set-dir"),
        (Some(path), None) => {
            if word_set != DEFAULT_WORD_SET_ID {
                anyhow::bail!("use either --word-set or --word-set-file, not both");
            }
            Ok(WordSetSelection::File(path))
        }
        (None, Some(path)) => {
            if word_set != DEFAULT_WORD_SET_ID {
                anyhow::bail!("use either --word-set or --word-set-dir, not both");
            }
            Ok(WordSetSelection::Directory(path))
        }
        (None, None) => Ok(WordSetSelection::BuiltIn(word_set)),
    }
}
