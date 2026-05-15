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
    /// Host a local-network multiplayer lobby.
    Host {
        /// Display name for the host player.
        #[arg(long)]
        name: String,
        /// Number of words in the generated track.
        #[arg(short, long, default_value_t = 40)]
        words: usize,
        /// Address and port to listen on.
        #[arg(long, default_value = "127.0.0.1:4000")]
        bind: SocketAddr,
        /// Maximum total players, including the host.
        #[arg(long, default_value_t = 6)]
        max_players: usize,
    },
    /// Join a local-network multiplayer lobby.
    Join {
        /// Display name for this player.
        #[arg(long)]
        name: String,
        /// Host address and port to connect to.
        #[arg(long)]
        server: SocketAddr,
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
        Command::Host {
            name,
            words,
            bind,
            max_players,
        } => app::host(bind, name, words, max_players),
        Command::Join { name, server } => app::join(server, name),
    }
}
