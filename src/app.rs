//! Application-level coordination.
//!
//! This module wires together word loading, track generation, player state, and
//! the terminal session. It intentionally avoids owning the rules themselves.

use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::PathBuf,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};

use crate::game::{
    ai::AiDifficulty,
    player::PlayerState,
    track::{Track, WordList},
};
use crate::net::{
    client::{JoinConfig, run_join},
    server::{HostConfig, run_host},
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

pub fn host(bind: SocketAddr, name: String, word_count: usize, max_players: usize) -> Result<()> {
    let word_list = WordList::load("words_alpha.txt").context("failed to load word list")?;
    let track = Track::generate(&word_list, word_count).context("failed to generate track")?;
    let (ready_sender, ready_receiver) = mpsc::channel();

    thread::spawn(move || {
        if let Err(error) = run_host(HostConfig {
            bind,
            host_name: None,
            track,
            max_players,
            ready_signal: Some(ready_sender),
            console_logging: false,
        }) {
            eprintln!("Host server stopped: {error:#}");
        }
    });

    let server = ready_receiver
        .recv_timeout(Duration::from_secs(2))
        .context("host server did not start")?;
    let server = loopback_server_addr(server);

    run_join(JoinConfig { server, name })
}

fn loopback_server_addr(address: SocketAddr) -> SocketAddr {
    let ip = match address.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };

    SocketAddr::new(ip, address.port())
}

pub fn join(server: SocketAddr, name: String) -> Result<()> {
    run_join(JoinConfig { server, name })
}
