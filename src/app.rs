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
    items::ItemRegistry,
    mods::ActiveModConfig,
    player::PlayerState,
    track::Track,
    words::{WordSetRegistry, WordSetSelection},
};
use crate::net::{
    client::{JoinConfig, OnlineRoomInfo, run_join},
    log::{NetworkLog, write_network_log},
    online::{
        OnlineHostBridgeConfig, OnlineJoinProxyConfig, run_online_host_bridge,
        run_online_join_proxy,
    },
    relay::RoomCode,
    server::{HostConfig, run_host},
};
use crate::ui::{render::IconMode, terminal::run_typing_session};

const DEFAULT_PLAYER_NAME: &str = "anonymous";

pub fn play(
    word_count: usize,
    word_set: WordSetSelection,
    item_pack_file: Option<PathBuf>,
    ai_racer_count: usize,
    ai_difficulty: AiDifficulty,
    icon_mode: IconMode,
    debug_log: Option<PathBuf>,
) -> Result<()> {
    let word_set = WordSetRegistry::builtin()
        .load(&word_set)
        .context("failed to load selected word set")?;
    let item_pack_source = item_pack_file
        .as_ref()
        .map(|path| path.display().to_string());
    let item_registry = load_item_registry(item_pack_file)?;
    let active_mod_config = ActiveModConfig::new(&word_set, &item_registry, item_pack_source);
    let word_list = word_set.words;
    let track = Track::generate(&word_list, word_count).context("failed to generate track")?;
    let player = PlayerState::new(Instant::now());

    run_typing_session(
        track,
        player,
        word_list,
        ai_racer_count,
        ai_difficulty,
        item_registry,
        active_mod_config,
        icon_mode,
        debug_log,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn host(
    bind: SocketAddr,
    name: Option<String>,
    word_count: usize,
    word_set: WordSetSelection,
    item_pack_file: Option<PathBuf>,
    max_players: usize,
    ai_racer_count: usize,
    ai_difficulty: AiDifficulty,
    icon_mode: IconMode,
    debug_log: Option<PathBuf>,
) -> Result<()> {
    let word_set = WordSetRegistry::builtin()
        .load(&word_set)
        .context("failed to load selected word set")?;
    let item_pack_source = item_pack_file
        .as_ref()
        .map(|path| path.display().to_string());
    let item_registry = load_item_registry(item_pack_file)?;
    let active_mod_config = ActiveModConfig::new(&word_set, &item_registry, item_pack_source);
    let word_list = word_set.words;
    let track = Track::generate(&word_list, word_count).context("failed to generate track")?;
    let (ready_sender, ready_receiver) = mpsc::channel();
    let network_log = debug_log
        .as_ref()
        .map(|_| NetworkLog::shared(Instant::now(), 2_000));
    let server_log = network_log.clone();

    thread::spawn(move || {
        if let Err(error) = run_host(HostConfig {
            bind,
            host_name: None,
            track,
            word_list,
            item_registry,
            active_mod_config,
            max_players,
            ai_racer_count,
            ai_difficulty,
            ready_signal: Some(ready_sender),
            console_logging: false,
            debug_log: server_log,
        }) {
            eprintln!("Host server stopped: {error:#}");
        }
    });

    let server = ready_receiver
        .recv_timeout(Duration::from_secs(2))
        .context("host server did not start")?;
    let server = loopback_server_addr(server);

    let result = run_join(JoinConfig {
        server,
        name: player_name_or_default(name),
        icon_mode,
        online_room: None,
        debug_log: None,
        shared_log: network_log.clone(),
    });

    if let (Some(path), Some(log)) = (debug_log, network_log) {
        thread::sleep(Duration::from_millis(50));
        write_network_log(path, &log)?;
    }

    result
}

#[allow(clippy::too_many_arguments)]
pub fn host_online(
    relay: String,
    name: Option<String>,
    word_count: usize,
    word_set: WordSetSelection,
    item_pack_file: Option<PathBuf>,
    max_players: usize,
    ai_racer_count: usize,
    ai_difficulty: AiDifficulty,
    icon_mode: IconMode,
    debug_log: Option<PathBuf>,
) -> Result<()> {
    let word_set = WordSetRegistry::builtin()
        .load(&word_set)
        .context("failed to load selected word set")?;
    let item_pack_source = item_pack_file
        .as_ref()
        .map(|path| path.display().to_string());
    let item_registry = load_item_registry(item_pack_file)?;
    let active_mod_config = ActiveModConfig::new(&word_set, &item_registry, item_pack_source);
    let word_list = word_set.words;
    let track = Track::generate(&word_list, word_count).context("failed to generate track")?;
    let (ready_sender, ready_receiver) = mpsc::channel();
    let network_log = debug_log
        .as_ref()
        .map(|_| NetworkLog::shared(Instant::now(), 2_000));
    let server_log = network_log.clone();

    thread::spawn(move || {
        if let Err(error) = run_host(HostConfig {
            bind: SocketAddr::from(([127, 0, 0, 1], 0)),
            host_name: None,
            track,
            word_list,
            item_registry,
            active_mod_config,
            max_players,
            ai_racer_count,
            ai_difficulty,
            ready_signal: Some(ready_sender),
            console_logging: false,
            debug_log: server_log,
        }) {
            eprintln!("Host server stopped: {error:#}");
        }
    });

    let local_server = ready_receiver
        .recv_timeout(Duration::from_secs(2))
        .context("host server did not start")?;
    let (room_sender, room_receiver) = mpsc::channel();
    let bridge_relay = relay.clone();
    thread::spawn(move || {
        if let Err(error) = run_online_host_bridge(OnlineHostBridgeConfig {
            relay: bridge_relay,
            local_server,
            ready_signal: Some(room_sender),
        }) {
            eprintln!("Online host bridge stopped: {error:#}");
        }
    });
    let room = room_receiver
        .recv_timeout(Duration::from_secs(5))
        .context("online host bridge did not create a room")?;

    let result = run_join(JoinConfig {
        server: loopback_server_addr(local_server),
        name: player_name_or_default(name),
        icon_mode,
        online_room: Some(OnlineRoomInfo { relay, room }),
        debug_log: None,
        shared_log: network_log.clone(),
    });

    if let (Some(path), Some(log)) = (debug_log, network_log) {
        thread::sleep(Duration::from_millis(50));
        write_network_log(path, &log)?;
    }

    result
}

fn load_item_registry(item_pack_file: Option<PathBuf>) -> Result<ItemRegistry> {
    if let Some(path) = item_pack_file {
        ItemRegistry::load_json_file(&path)
            .with_context(|| format!("failed to load item pack file {}", path.display()))
    } else {
        Ok(ItemRegistry::builtin())
    }
}

fn loopback_server_addr(address: SocketAddr) -> SocketAddr {
    let ip = match address.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(Ipv6Addr::LOCALHOST),
        ip => ip,
    };

    SocketAddr::new(ip, address.port())
}

pub fn join(
    server: SocketAddr,
    name: Option<String>,
    icon_mode: IconMode,
    debug_log: Option<PathBuf>,
) -> Result<()> {
    run_join(JoinConfig {
        server,
        name: player_name_or_default(name),
        icon_mode,
        online_room: None,
        debug_log,
        shared_log: None,
    })
}

pub fn join_online(
    relay: String,
    room: RoomCode,
    name: Option<String>,
    icon_mode: IconMode,
    debug_log: Option<PathBuf>,
) -> Result<()> {
    let (ready_sender, ready_receiver) = mpsc::channel();
    let proxy_relay = relay.clone();
    let proxy_room = room.clone();
    let player_name = player_name_or_default(name);
    let proxy_name = player_name.clone();
    thread::spawn(move || {
        let result = run_online_join_proxy(OnlineJoinProxyConfig {
            relay: proxy_relay,
            room: proxy_room,
            name: proxy_name,
            ready_signal: ready_sender,
        });
        match result {
            Err(error) if !is_expected_online_join_rejection(&error) => {
                eprintln!("Online join proxy stopped: {error:#}");
            }
            _ => {}
        }
    });

    let local_server = ready_receiver
        .recv_timeout(Duration::from_secs(2))
        .context("online join proxy did not start")?;

    run_join(JoinConfig {
        server: local_server,
        name: player_name,
        icon_mode,
        online_room: Some(OnlineRoomInfo { relay, room }),
        debug_log,
        shared_log: None,
    })
}

fn player_name_or_default(name: Option<String>) -> String {
    name.as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(DEFAULT_PLAYER_NAME)
        .to_string()
}

fn is_expected_online_join_rejection(error: &anyhow::Error) -> bool {
    let message = format!("{error:#}");
    message.contains("relay rejected join") || message.contains("room closed while joining")
}
