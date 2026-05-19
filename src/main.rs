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
use crate::ui::{gallery::GalleryKind, render::IconMode};

const DEFAULT_PUBLIC_RELAY_URL: &str = "wss://typekart-relay.fly.dev";

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
    /// Host an online multiplayer lobby
    Host {
        /// Display name for the host player.
        #[arg(long)]
        name: Option<String>,
        /// WebSocket relay URL.
        #[arg(long, default_value = DEFAULT_PUBLIC_RELAY_URL)]
        relay: String,
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
    /// Host a local-network multiplayer lobby.
    #[command(name = "host-lan")]
    HostLan {
        /// Display name for the host player.
        #[arg(long)]
        name: Option<String>,
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
    /// Join an online multiplayer lobby
    Join {
        /// Display name for this player.
        #[arg(long)]
        name: Option<String>,
        /// WebSocket relay URL.
        #[arg(long, default_value = DEFAULT_PUBLIC_RELAY_URL)]
        relay: String,
        /// Relay room code shown by the host.
        #[arg(long)]
        room: String,
        /// Write detailed network diagnostics to this file after the session exits.
        #[arg(long)]
        debug_log: Option<PathBuf>,
        /// Use ASCII-safe markers instead of Unicode item icons.
        #[arg(long = "ascii")]
        ascii_icons: bool,
    },
    /// Join a local-network multiplayer lobby.
    #[command(name = "join-lan")]
    JoinLan {
        /// Display name for this player.
        #[arg(long)]
        name: Option<String>,
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
    /// Run a local development WebSocket relay.
    Relay {
        /// Address and port to listen on.
        #[arg(long, default_value = "127.0.0.1:8080")]
        bind: SocketAddr,
        /// Maximum active relay rooms.
        #[arg(long, default_value_t = 256)]
        max_rooms: usize,
        /// Maximum joiners per relay room, excluding the host.
        #[arg(long, default_value_t = 5)]
        max_participants_per_room: usize,
        /// Maximum concurrent relay WebSocket connections.
        #[arg(long, default_value_t = 1024)]
        max_connections: usize,
        /// Maximum concurrent relay WebSocket connections from one IP.
        #[arg(long, default_value_t = 64)]
        max_connections_per_ip: usize,
        /// Maximum WebSocket text message size in bytes.
        #[arg(long, default_value_t = 262_144)]
        max_message_bytes: usize,
        /// Maximum WebSocket text messages per second from one IP.
        #[arg(long, default_value_t = 120)]
        max_messages_per_second_per_ip: u32,
        /// Maximum room creates per minute from one IP.
        #[arg(long, default_value_t = 20)]
        max_room_creates_per_minute_per_ip: u32,
        /// Maximum room joins per minute from one IP.
        #[arg(long, default_value_t = 120)]
        max_room_joins_per_minute_per_ip: u32,
        /// Maximum queued outbound relay messages per connection.
        #[arg(long, default_value_t = 256)]
        outbound_queue_size: usize,
        /// Seconds a WebSocket may stay connected before creating or joining a room.
        #[arg(long, default_value_t = 5)]
        handshake_timeout_secs: u64,
        /// Close rooms after this many idle seconds.
        #[arg(long, default_value_t = 7200)]
        room_idle_timeout_secs: u64,
    },
    /// Preview renderer states for item and effect UI development.
    Gallery {
        #[command(subcommand)]
        kind: GalleryCommand,
    },
    /// Run a relay capacity load test.
    #[command(name = "relay-load-test", hide = true)]
    RelayLoadTest {
        /// WebSocket relay URL to test.
        #[arg(long, default_value = DEFAULT_PUBLIC_RELAY_URL)]
        relay: String,
        /// First concurrent game count to test.
        #[arg(long, default_value_t = 10)]
        start_games: usize,
        /// Maximum concurrent game count to test.
        #[arg(long, default_value_t = 100)]
        max_games: usize,
        /// Concurrent game increment for each test step.
        #[arg(long, default_value_t = 10)]
        step_games: usize,
        /// Joiners per game, excluding the host.
        #[arg(long, default_value_t = 5)]
        joiners_per_game: usize,
        /// Seconds to sustain each load-test step.
        #[arg(long, default_value_t = 30)]
        duration_secs: u64,
        /// Host broadcast interval in milliseconds.
        #[arg(long, default_value_t = 100)]
        snapshot_interval_ms: u64,
        /// Simulated joiner key input interval in milliseconds.
        #[arg(long, default_value_t = 125)]
        input_interval_ms: u64,
        /// Seconds to wait for rooms to be created before each step starts joiners.
        #[arg(long, default_value_t = 10)]
        settle_timeout_secs: u64,
    },
}

#[derive(Debug, Subcommand)]
enum GalleryCommand {
    /// Preview item pickup cues, active effects, and impact effects.
    Items {
        /// Start on a named scenario, such as multiplayer-pack or banana-hit-pack.
        #[arg(long)]
        scenario: Option<String>,
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
            relay,
            words,
            word_set,
            word_set_file,
            word_set_dir,
            item_pack_file,
            max_players,
            ai_racers,
            ai_difficulty,
            debug_log,
            ascii_icons,
        } => app::host_online(
            relay,
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
        Command::HostLan {
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
            relay,
            room,
            debug_log,
            ascii_icons,
        } => app::join_online(
            relay,
            crate::net::relay::RoomCode::parse(room)?,
            name,
            if ascii_icons {
                IconMode::Ascii
            } else {
                IconMode::Unicode
            },
            debug_log,
        ),
        Command::JoinLan {
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
        Command::Relay {
            bind,
            max_rooms,
            max_participants_per_room,
            max_connections,
            max_connections_per_ip,
            max_message_bytes,
            max_messages_per_second_per_ip,
            max_room_creates_per_minute_per_ip,
            max_room_joins_per_minute_per_ip,
            outbound_queue_size,
            handshake_timeout_secs,
            room_idle_timeout_secs,
        } => net::relay_server::run_relay(net::relay_server::RelayConfig {
            bind,
            ready_signal: None,
            limits: net::relay_server::RelayLimits {
                max_rooms,
                max_participants_per_room,
                max_connections,
                max_connections_per_ip,
                max_message_bytes,
                max_messages_per_second_per_ip,
                max_room_creates_per_minute_per_ip,
                max_room_joins_per_minute_per_ip,
                outbound_queue_size,
                handshake_timeout: std::time::Duration::from_secs(handshake_timeout_secs),
                room_idle_timeout: std::time::Duration::from_secs(room_idle_timeout_secs),
            },
        }),
        Command::Gallery { kind } => match kind {
            GalleryCommand::Items {
                scenario,
                ascii_icons,
            } => ui::gallery::run_gallery(
                GalleryKind::Items { scenario },
                if ascii_icons {
                    IconMode::Ascii
                } else {
                    IconMode::Unicode
                },
            ),
        },
        Command::RelayLoadTest {
            relay,
            start_games,
            max_games,
            step_games,
            joiners_per_game,
            duration_secs,
            snapshot_interval_ms,
            input_interval_ms,
            settle_timeout_secs,
        } => net::load_test::run_relay_load_test(net::load_test::RelayLoadTestConfig {
            relay,
            start_games,
            max_games,
            step_games,
            joiners_per_game,
            duration: std::time::Duration::from_secs(duration_secs),
            snapshot_interval: std::time::Duration::from_millis(snapshot_interval_ms),
            input_interval: std::time::Duration::from_millis(input_interval_ms),
            settle_timeout: std::time::Duration::from_secs(settle_timeout_secs),
        }),
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
