use std::{
    collections::HashMap,
    io::BufReader,
    net::{SocketAddr, TcpStream},
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

use super::{
    HostBroadcastDedupe, HostRelayDelivery, OnlineHostBridgeConfig, OnlineJoinProxyConfig,
    handle_host_relay_message, host_relay_delivery, join_version, run_online_host_bridge,
    run_online_join_proxy,
};
use crate::{
    game::{
        ai::AiDifficulty, items::ItemRegistry, mods::ActiveModConfig, track::Track,
        words::WordSetDefinition,
    },
    net::{
        protocol::{ClientMessage, PlayerId, ServerMessage},
        relay::{RelayServerMessage, RoomCode},
        relay_server::{RelayConfig, run_relay},
        server::{HostConfig, run_host},
        transport::{read_client_message, read_server_message, write_client_message},
    },
};

#[test]
fn join_version_uses_local_client_hello_version() {
    let message = ClientMessage::Hello {
        name: "alex".to_string(),
        client_version: "9.9.9".to_string(),
    };

    assert_eq!(join_version(&message), Some("9.9.9"));
}

#[test]
fn relay_join_url_adds_room_query_with_valid_path() {
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();

    assert_eq!(
        super::relay_join_url("ws://127.0.0.1:8080", &room),
        "ws://127.0.0.1:8080/?typekart_room=rocket-salad-tiger"
    );
    assert_eq!(
        super::relay_join_url("wss://relay.example.com/ws?debug=true", &room),
        "wss://relay.example.com/ws?debug=true&typekart_room=rocket-salad-tiger"
    );
}

#[test]
fn online_bridge_allows_joiner_to_receive_host_welcome_through_relay() {
    let relay_addr = spawn_test_relay();
    let local_host = spawn_test_host();
    let (room_tx, room_rx) = mpsc::channel();
    let relay_url = format!("ws://{relay_addr}");

    {
        let relay_url = relay_url.clone();
        thread::spawn(move || {
            let _ = run_online_host_bridge(OnlineHostBridgeConfig {
                relay: relay_url,
                local_server: local_host,
                ready_signal: Some(room_tx),
            });
        });
    }
    let room = room_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("online host bridge should create a room");

    let (proxy_tx, proxy_rx) = mpsc::channel();
    {
        let relay_url = relay_url.clone();
        thread::spawn(move || {
            let _ = run_online_join_proxy(OnlineJoinProxyConfig {
                relay: relay_url,
                room,
                name: "alex".to_string(),
                ready_signal: proxy_tx,
            });
        });
    }
    let proxy_addr = proxy_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("online join proxy should start");

    let mut client =
        TcpStream::connect(proxy_addr).expect("test client should connect to join proxy");
    client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("read timeout should be configurable");
    write_client_message(
        &mut client,
        &ClientMessage::Hello {
            name: "alex".to_string(),
            client_version: env!("CARGO_PKG_VERSION").to_string(),
        },
    )
    .expect("hello should be forwarded to relay");

    let mut reader = std::io::BufReader::new(client.try_clone().unwrap());
    let welcome = read_server_message(&mut reader)
        .expect("welcome should decode")
        .expect("welcome should arrive");
    assert!(matches!(welcome, ServerMessage::Welcome { .. }));
    let lobby = read_server_message(&mut reader)
        .expect("initial lobby should decode")
        .expect("initial lobby should arrive");
    assert!(
        matches!(lobby, ServerMessage::LobbySnapshot { ref players, .. } if players.iter().any(|player| player.name == "alex"))
    );

    write_client_message(&mut client, &ClientMessage::Leave).unwrap();
}

#[test]
fn online_bridge_forwards_countdown_snapshot_to_joiner() {
    let relay_addr = spawn_test_relay();
    let local_host = spawn_test_host();
    let relay_url = format!("ws://{relay_addr}");

    let mut host_client =
        TcpStream::connect(local_host).expect("host client should connect to local host");
    host_client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("host read timeout should be configurable");
    write_client_message(
        &mut host_client,
        &ClientMessage::Hello {
            name: "host".to_string(),
            client_version: env!("CARGO_PKG_VERSION").to_string(),
        },
    )
    .expect("host hello should be accepted");
    let mut host_reader = BufReader::new(host_client.try_clone().unwrap());
    assert!(matches!(
        read_server_message(&mut host_reader)
            .expect("host welcome should decode")
            .expect("host welcome should arrive"),
        ServerMessage::Welcome { .. }
    ));

    let (room_tx, room_rx) = mpsc::channel();
    {
        let relay_url = relay_url.clone();
        thread::spawn(move || {
            let _ = run_online_host_bridge(OnlineHostBridgeConfig {
                relay: relay_url,
                local_server: local_host,
                ready_signal: Some(room_tx),
            });
        });
    }
    let room = room_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("online host bridge should create a room");

    let (proxy_tx, proxy_rx) = mpsc::channel();
    {
        let relay_url = relay_url.clone();
        thread::spawn(move || {
            let _ = run_online_join_proxy(OnlineJoinProxyConfig {
                relay: relay_url,
                room,
                name: "joiner".to_string(),
                ready_signal: proxy_tx,
            });
        });
    }
    let proxy_addr = proxy_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("online join proxy should start");

    let mut join_client =
        TcpStream::connect(proxy_addr).expect("join client should connect to proxy");
    join_client
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("join read timeout should be configurable");
    write_client_message(
        &mut join_client,
        &ClientMessage::Hello {
            name: "joiner".to_string(),
            client_version: env!("CARGO_PKG_VERSION").to_string(),
        },
    )
    .expect("join hello should be accepted");
    let mut join_reader = BufReader::new(join_client.try_clone().unwrap());
    assert!(matches!(
        read_server_message(&mut join_reader)
            .expect("join welcome should decode")
            .expect("join welcome should arrive"),
        ServerMessage::Welcome { .. }
    ));
    assert!(matches!(
        read_server_message(&mut join_reader)
            .expect("join lobby should decode")
            .expect("join lobby should arrive"),
        ServerMessage::LobbySnapshot { ref players, .. }
            if players.iter().any(|player| player.name == "joiner")
    ));

    write_client_message(
        &mut join_client,
        &ClientMessage::Rename {
            name: "speedy".to_string(),
        },
    )
    .expect("joiner rename should be forwarded");
    let renamed_lobby = read_server_message(&mut join_reader)
        .expect("renamed lobby should decode")
        .expect("renamed lobby should arrive");
    assert!(matches!(
        renamed_lobby,
        ServerMessage::LobbySnapshot { ref players, .. }
            if players.iter().any(|player| player.name == "speedy")
    ));

    write_client_message(&mut join_client, &ClientMessage::SetReady { ready: true })
        .expect("joiner ready should be forwarded");
    let ready_lobby = read_server_message(&mut join_reader)
        .expect("ready lobby should decode")
        .expect("ready lobby should arrive");
    assert!(matches!(ready_lobby, ServerMessage::LobbySnapshot { .. }));

    write_client_message(&mut host_client, &ClientMessage::StartCountdown)
        .expect("host start should be accepted");
    let countdown = read_server_message(&mut join_reader)
        .expect("countdown should decode")
        .expect("countdown should arrive");
    assert!(matches!(
        countdown,
        ServerMessage::RaceSnapshot(snapshot)
            if matches!(snapshot.phase, crate::net::protocol::NetworkRacePhase::Countdown { remaining_seconds: 3 })
    ));

    write_client_message(&mut host_client, &ClientMessage::RestartRace)
        .expect("host cancel should be accepted");
    let cancelled = read_server_message(&mut join_reader)
        .expect("cancel snapshot should decode")
        .expect("cancel snapshot should arrive");
    assert!(matches!(
        cancelled,
        ServerMessage::RaceSnapshot(snapshot)
            if snapshot.phase == crate::net::protocol::NetworkRacePhase::WaitingForHost
                && snapshot.events.iter().any(|event| event == "Race cancelled")
    ));

    write_client_message(&mut join_client, &ClientMessage::Leave).unwrap();
    write_client_message(&mut host_client, &ClientMessage::Leave).unwrap();
}

#[test]
fn host_bridge_turns_relay_participant_disconnect_into_local_leave() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let participant_stream = TcpStream::connect(address).unwrap();
    let (host_side_stream, _) = listener.accept().unwrap();
    let room = RoomCode::parse("rocket-salad-tiger").unwrap();
    let player_id = PlayerId(2);
    let (outbound_tx, _outbound_rx) = mpsc::channel();
    let mut participants = HashMap::from([(player_id, participant_stream)]);
    let broadcast_dedupe = Arc::new(Mutex::new(HostBroadcastDedupe::default()));

    handle_host_relay_message(
        RelayServerMessage::ParticipantDisconnected {
            room: room.clone(),
            player_id,
        },
        &room,
        address,
        &outbound_tx,
        &mut participants,
        &broadcast_dedupe,
    )
    .unwrap();

    let mut reader = BufReader::new(host_side_stream);
    assert_eq!(
        read_client_message(&mut reader).unwrap(),
        Some(ClientMessage::Leave)
    );
    assert!(!participants.contains_key(&player_id));
}

#[test]
fn host_bridge_broadcast_dedupe_skips_duplicate_shared_race_update() {
    let dedupe = Arc::new(Mutex::new(HostBroadcastDedupe::default()));
    let message = ServerMessage::RaceDelta(crate::net::protocol::RaceDeltaSnapshot {
        sequence: 9,
        phase: crate::net::protocol::NetworkRacePhase::Racing,
        bonuses: Vec::new(),
        players: Vec::new(),
        events: Vec::new(),
    });

    assert_eq!(
        host_relay_delivery(&message, &dedupe),
        HostRelayDelivery::Broadcast
    );
    assert_eq!(
        host_relay_delivery(&message, &dedupe),
        HostRelayDelivery::Skip
    );
}

#[test]
fn online_bridge_treats_common_websocket_disconnects_as_normal_close() {
    assert!(super::websocket_disconnect_error(
        &tungstenite::Error::ConnectionClosed
    ));
    assert!(super::websocket_disconnect_error(
        &tungstenite::Error::Protocol(
            tungstenite::error::ProtocolError::ResetWithoutClosingHandshake
        )
    ));
    assert!(super::websocket_disconnect_error(&tungstenite::Error::Io(
        std::io::Error::new(std::io::ErrorKind::ConnectionReset, "reset")
    )));
}

fn spawn_test_relay() -> SocketAddr {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = run_relay(RelayConfig {
            bind: SocketAddr::from(([127, 0, 0, 1], 0)),
            ready_signal: Some(tx),
            limits: Default::default(),
            redis_routing: None,
        });
    });
    rx.recv_timeout(Duration::from_secs(2))
        .expect("relay should start")
}

fn spawn_test_host() -> SocketAddr {
    let word_set = WordSetDefinition::load_builtin_default().unwrap();
    let item_registry = ItemRegistry::builtin();
    let active_mod_config = ActiveModConfig::new(&word_set, &item_registry, None);
    let track = Track::generate(&word_set.words, 6).unwrap();
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        let _ = run_host(HostConfig {
            bind: SocketAddr::from(([127, 0, 0, 1], 0)),
            host_name: None,
            track,
            word_list: word_set.words,
            item_registry,
            active_mod_config,
            max_players: 6,
            ai_racer_count: 0,
            ai_difficulty: AiDifficulty::Easy,
            ready_signal: Some(tx),
            console_logging: false,
            debug_log: None,
        });
    });

    rx.recv_timeout(Duration::from_secs(2))
        .expect("host should start")
}
