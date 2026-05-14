//! Command-line entry point for TypeKart.
//!
//! Keep this file small: it should parse CLI arguments and delegate to `app`.
//! The actual game rules live under `game`, and terminal code lives under `ui`.

mod app;
mod game;
mod ui;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "typekart")]
#[command(about = "A terminal typing racer")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Start a local single-player typing race.
    Play {
        /// Number of words in the generated track.
        #[arg(short, long, default_value_t = 40)]
        words: usize,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Play { words } => app::play(words),
    }
}
